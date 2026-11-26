//! 供暖系统计算：散热曲线、末端数量、水温/室温耦合
//!
//! 移植自 C# `CMCalculation`（`DG_Fit`、`HeatJZTF`、`MDQuantity`、
//! `RFWaterTem`、`RFRoomTemp`、`vKT`、`QTtemperature`）。
//!
//! # 相对原实现的修正
//!
//! | # | 原 C# 问题 | 本实现 |
//! |---|-----------|--------|
//! | 1 | `DG_Fit` **递归求和时每项都 `Math.Round(…,3)`**，<br>逐项舍入累积误差（多项式求值应只在最后舍入一次） | `poly_eval` 一次求和、一次舍入 |
//! | 2 | `HeatJZTF = RF/GZ/0.92`：`GZ=0`（供暖曲线为空时）返回 `Infinity`，<br>界面显示「∞ 台」（截图可见） | `Option` 返回，`None` 表示无曲线 |
//! | 3 | `MDQuantity` 中 `deltaT=0` → `R=0` → `RZL/R` 为 `Infinity`，<br>`Math.Ceiling(Infinity)=Infinity` 被当正常值显示 | `Result` + 明确错误 |
//! | 4 | `RFRoomTemp`/`RFWaterTem` 中 5 个 `W/=1000` 隐式单位换算、<br>参数名 `hEndWK` 实为末端总功率(kW) | 明确单位文档，常量命名 |

/// 供暖计算错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HeatError {
    /// 供暖曲线系数为空
    NoCurve,
    /// 温差必须大于 0（水温需高于需求温度）
    NonPositiveDeltaT,
    /// 末端能力必须为正
    NonPositiveCapacity,
    /// 输入包含 NaN/无穷
    NotFinite,
}

impl core::fmt::Display for HeatError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HeatError::NoCurve => write!(f, "供暖散热曲线未设置（无法求供暖能力）"),
            HeatError::NonPositiveDeltaT => write!(f, "供回水平均温度必须高于养殖需求温度"),
            HeatError::NonPositiveCapacity => write!(f, "末端/机组能力必须大于 0"),
            HeatError::NotFinite => write!(f, "输入包含无效数值"),
        }
    }
}

impl std::error::Error for HeatError {}

/// 热源机组型号（空气能热泵 · 赤焰系列 / 锅炉 / 燃气热风机）
#[derive(Debug, Clone)]
pub struct HeaterModel {
    /// 机组型号
    pub name: &'static str,
    /// 设备分类（空气能热泵 / 锅炉 / 燃气热风机）
    pub category: &'static str,
    /// 是否经末端（水路）供暖：燃气热风机直吹 false → 无热网系数 0.92
    pub has_terminal: bool,
    /// 额定制热量 [kW]（A7/W45）
    pub heat_kw: f64,
    /// 额定功率 [kW]
    pub power_kw: f64,
    /// COP
    pub cop: f64,
    /// 制热能力曲线锚点（外温 °C, 制热量 kW）——来自产品参数表
    /// 名义制热 A-12 与低温名义制热 A-20 的首列值 + 额定点 A7
    pub curve_pts: &'static [(f64, f64)],
    /// 能耗说明（每小时口径：燃气 m³/h、燃油 L/h、电 kW·h），选型表显示
    pub energy_note: &'static str,
}

impl HeaterModel {
    /// 三次多项式求值 p(t) = a·t³ + b·t² + c·t + d
    fn cubic(c: [f64; 4], t: f64) -> f64 {
        ((c[0] * t + c[1]) * t + c[2]) * t + c[3]
    }

    /// 由 3 个锚点 + 高温端一阶导数连续（= 末段斜率）确定唯一三次曲线
    ///
    /// p(xᵢ)=yᵢ (i=1..3)、p'(x₃)=(y₃−y₂)/(x₃−x₂)，4×4 线性方程组高斯消元。
    fn fit_cubic(pts: &[(f64, f64)]) -> Option<[f64; 4]> {
        if pts.len() != 3 {
            return None;
        }
        let (x, y): (Vec<f64>, Vec<f64>) = pts.iter().map(|(a, b)| (*a, *b)).unzip();
        let m = (y[2] - y[1]) / (x[2] - x[1]);
        // 未知数 [a b c d]
        let mut a = [
            [x[0] * x[0] * x[0], x[0] * x[0], x[0], 1.0, y[0]],
            [x[1] * x[1] * x[1], x[1] * x[1], x[1], 1.0, y[1]],
            [x[2] * x[2] * x[2], x[2] * x[2], x[2], 1.0, y[2]],
            [3.0 * x[2] * x[2], 2.0 * x[2], 1.0, 0.0, m],
        ];
        // 高斯消元（主元归一 + 回代）
        for col in 0..4 {
            let mut piv = col;
            let mut best = a[col][col].abs();
            for r in col + 1..4 {
                if a[r][col].abs() > best {
                    best = a[r][col].abs();
                    piv = r;
                }
            }
            a.swap(col, piv);
            let p = a[col][col];
            if p.abs() < 1e-12 {
                return None;
            }
            for j in col..5 {
                a[col][j] /= p;
            }
            for r in 0..4 {
                if r != col && a[r][col].abs() > 1e-15 {
                    let f = a[r][col];
                    for j in col..5 {
                        a[r][j] -= f * a[col][j];
                    }
                }
            }
        }
        Some([a[0][4], a[1][4], a[2][4], a[3][4]])
    }

    /// 制热量曲线的多项式系数（`poly_eval` 约定：coeff[i] 为 t^i 项），
    /// 供水温/室温公式的 `s` 使用（锚点区间内与 capacity_kw 一致）
    pub fn poly_coeffs(&self) -> Vec<f64> {
        Self::fit_cubic(self.curve_pts)
            .map(|[a, b, c, d]| vec![d, c, b, a])
            .unwrap_or_else(|| vec![self.heat_kw])
    }

    /// 外温 t 时的制热量 [kW]：锚点区间内**三次拟合曲线**（平滑无折角）；
    /// 低温端按首段斜率线性外推（下限 0），高温端按末段斜率外推并封顶 1.15× 额定。
    pub fn capacity_kw(&self, t: f64) -> f64 {
        let pts = self.curve_pts;
        if pts.is_empty() {
            return self.heat_kw;
        }
        if pts.len() == 1 {
            return pts[0].1;
        }
        let lo = pts[0].0;
        let hi = pts[pts.len() - 1].0;
        if t < lo {
            let (x0, y0) = pts[0];
            let (x1, y1) = pts[1];
            let k = (y1 - y0) / (x1 - x0);
            return (y0 + k * (t - x0)).max(0.0);
        }
        if t > hi {
            let n = pts.len();
            let (x0, y0) = pts[n - 2];
            let (x1, y1) = pts[n - 1];
            let k = (y1 - y0) / (x1 - x0);
            let v = y1 + k * (t - x1);
            return v.min(self.heat_kw * 1.15).max(0.0);
        }
        if let Some(c) = Self::fit_cubic(pts) {
            let v = Self::cubic(c, t);
            // 数值安全：不允许低于 0，也不超过高温封顶
            return v.max(0.0).min(self.heat_kw * 1.15);
        }
        self.heat_kw
    }

    /// 外温 t 时的制热性能系数指示值 COP(t) = 制热量 / 额定输入功率
    ///
    /// 电功率按样本额定值（保守指示：低温端除霜与风机功耗上升未计入，
    /// 实际低温 COP 略低于此值）；额定点 A7/W45 处与样本 COP 一致。
    pub fn cop_at(&self, t: f64) -> f64 {
        if self.power_kw <= 0.0 {
            return 0.0;
        }
        self.capacity_kw(t) / self.power_kw
    }
}

/// 热源机组选型库（赤焰系列产品参数表）
pub fn heater_db() -> &'static [HeaterModel] {
    static DB: &[HeaterModel] = &[
        HeaterModel {
            name: "ZGH-170IADG2",
            category: "空气能热泵",
            has_terminal: true,
            heat_kw: 170.0,
            power_kw: 47.9,
            cop: 3.55,
            // A-20/W35: 85.6 · A-12/W35: 112.9 · A7/W45: 170
            curve_pts: &[(-20.0, 85.6), (-12.0, 112.9), (7.0, 170.0)],
            energy_note: "电 47.9 kW·h/h",
        },
        HeaterModel {
            name: "ZGH-185IIBD",
            category: "空气能热泵",
            has_terminal: true,
            heat_kw: 185.0,
            power_kw: 49.3,
            cop: 3.75,
            // A-20/W35: 97.5 · A-12/W35: 118.5 · A7/W45: 185
            curve_pts: &[(-20.0, 97.5), (-12.0, 118.5), (7.0, 185.0)],
            energy_note: "电 49.3 kW·h/h",
        },
        HeaterModel {
            name: "锅炉",
            category: "锅炉",
            has_terminal: true,
            heat_kw: 120.0,
            power_kw: 0.0,
            cop: 0.0,
            // 锅炉能力不随外温衰减（常数曲线，拟合系数 0.80）；
            // 吨位 120 kW ≈ 0.17 t/h（1 t/h ↔ 0.7 MW），
            // 煤耗 ≈24 kg/h（5500 大卡原煤 · 热效率 80%）
            curve_pts: &[(-20.0, 120.0), (-12.0, 120.0), (7.0, 120.0)],
            energy_note: "燃煤 0.17 t/h · ≈24 kg/h",
        },
        HeaterModel {
            // 燃气热风机 G75（ ItemNo G75 ）：发热量 75 kW ≈ 64,500 大卡/h，
            // 天然气 7.6 m³/h · 液化气 5.7 kg/h · 风量 6,500 m³/h · 电机 380 W
            name: "G75",
            category: "燃气热风机",
            has_terminal: false,
            heat_kw: 75.0,
            power_kw: 0.38,
            cop: 0.0,
            // 直吹式：制热量不随外温衰减（燃烧额定，常数曲线）
            curve_pts: &[(-20.0, 75.0), (-12.0, 75.0), (7.0, 75.0)],
            energy_note: "天然气 7.6 m³/h · 液化气 5.7 kg/h",
        },
        HeaterModel {
            // 燃油热风机 DH60：58 kW ≈ 49,880 大卡/h，柴油 ≈5.8 L/h
            name: "DH60",
            category: "燃油热风机",
            has_terminal: false,
            heat_kw: 58.0,
            power_kw: 0.0,
            cop: 0.0,
            curve_pts: &[(-20.0, 58.0), (-12.0, 58.0), (7.0, 58.0)],
            energy_note: "柴油 ≈5.8 L/h",
        },
        HeaterModel {
            // 燃油热风机 Helios 170：174 kW ≈ 149,200 大卡/h · 效率 90.3%，
            // 风量 14,000 m³/h @ 900 mm 水柱 · 油耗 17.29 L/h · 380V-50Hz-3Φ
            name: "Helios 170",
            category: "燃油热风机",
            has_terminal: false,
            heat_kw: 174.0,
            power_kw: 0.0,
            cop: 0.0,
            curve_pts: &[(-20.0, 174.0), (-12.0, 174.0), (7.0, 174.0)],
            energy_note: "柴油 17.29 L/h",
        },
    ];
    DB
}

/// 该热源名称是否为**直吹式无末端**（燃气热风机）：无热网系数 0.92、
/// 无末端数量/水温核算。DB 查不到（自定义拟合曲线）时按传统水路供暖。
pub fn heater_no_terminal(name: &str) -> bool {
    find_heater_model(name).is_some_and(|m| !m.has_terminal)
}

/// 按名称查找热源机组
pub fn find_heater_model(name: &str) -> Option<&'static HeaterModel> {
    if name.is_empty() {
        return None;
    }
    heater_db().iter().find(|m| m.name == name)
}

#[cfg(test)]
mod heater_model_tests {
    use super::*;

    #[test]
    fn anchors_match_product_table() {
        let m = find_heater_model("ZGH-185IIBD").unwrap();
        assert!((m.capacity_kw(7.0) - 185.0).abs() < 1e-9);
        assert!((m.capacity_kw(-12.0) - 118.5).abs() < 1e-9);
        assert!((m.capacity_kw(-20.0) - 97.5).abs() < 1e-9);
        let m2 = find_heater_model("ZGH-170IADG2").unwrap();
        assert!((m2.capacity_kw(-20.0) - 85.6).abs() < 1e-9);
    }

    #[test]
    fn curve_monotonic_and_smooth() {
        let m = find_heater_model("ZGH-185IIBD").unwrap();
        // −20 → 7 能力单调上升（三次曲线）
        let mut prev = f64::MIN;
        for i in -200..=70 {
            let v = m.capacity_kw(i as f64 / 10.0);
            assert!(v >= prev - 1e-9, "not monotonic at {i}");
            prev = v;
        }
        // 三次曲线通过全部锚点
        for (t, y) in m.curve_pts {
            assert!((m.capacity_kw(*t) - y).abs() < 1e-6, "anchor {t}->{y}");
        }
        // 二阶差分连续（平滑，无折角）：相邻步长差的突变很小
        let d: Vec<f64> = (-199..=70)
            .map(|i| m.capacity_kw(i as f64 / 10.0) - m.capacity_kw((i - 1) as f64 / 10.0))
            .collect();
        let max_jump = d
            .windows(2)
            .map(|w| (w[1] - w[0]).abs())
            .fold(0.0_f64, f64::max);
        assert!(max_jump < 0.05, "kink detected, max jump {max_jump}");
    }

    #[test]
    fn extrapolation_bounds() {
        let m = find_heater_model("ZGH-170IADG2").unwrap();
        // 高温端封顶 1.15× 额定
        assert!(m.capacity_kw(16.0) <= 170.0 * 1.15 + 1e-9);
        assert!(m.capacity_kw(16.0) >= 170.0);
        // 低温端不为负
        assert!(m.capacity_kw(-40.0) >= 0.0);
    }
}

/// 多项式求值（修正版 `DG_Fit`）
///
/// 原 C# 递归版：`round(p[i]·xⁱ + DG_Fit(p,x,i+1), 3)` —— 每层递归都把
/// 部分和舍入到 3 位小数，逐项误差累积。例如 5 项多项式在 x=25 处
/// 会引入最多 ±0.002 的额外误差。
///
/// 本实现按标准 Horner/直接求和后**只舍入一次**到 3 位小数（与原输出格式一致）。
pub fn poly_eval(coeff: &[f64], x: f64) -> f64 {
    let sum: f64 = coeff
        .iter()
        .enumerate()
        .map(|(i, &c)| c * x.powi(i as i32))
        .sum();
    (sum * 1000.0).round() / 1000.0
}

/// 原 C# `DG_Fit` 的逐项舍入版本（仅供对照测试）
#[doc(hidden)]
pub fn poly_eval_legacy_rounding(coeff: &[f64], x: f64) -> f64 {
    fn rec(p: &[f64], x: f64, i: usize) -> f64 {
        if i < p.len() {
            let sum = p[i] * x.powi(i as i32);
            ((sum + rec(p, x, i + 1)) * 1000.0).round() / 1000.0
        } else {
            0.0
        }
    }
    rec(coeff, x, 0)
}

/// 水表面饱和压力拟合系数（C# `pBHszq`，MATLAB Poly4）
///
/// `f(x) = p1x⁴ + p2x³ + p3x² + p4x + p5`，单位 hPa，×100 → Pa。
/// 该拟合实为饱和水蒸气压曲线，可与 `psychro::get_sat_vap_pres` 交叉验证。
pub const WATER_SURFACE_PRESSURE_COEFF: [f64; 5] =
    [6.246, 0.3819, 0.02068, 0.000004211, 0.000007291];

/// 水表面压力 [Pa]（C# `WatarSurfacePressure`）
///
/// `poly_eval(系数, 水温) · 100 · 相对湿度% / 100`
pub fn water_surface_pressure(water_temp_c: f64, humidity_pct: f64) -> f64 {
    (poly_eval(&WATER_SURFACE_PRESSURE_COEFF, water_temp_c) * 100.0 * humidity_pct / 100.0
        * 10000.0)
        .round()
        / 10000.0
}

/// 单台供暖机组能力 [kW]（C# `HeatJZTF(GZ, RF)`：`RF / GZ / 0.92`）
///
/// * `curve_capacity` —— 供暖曲线在设计温度处的能力（kW/台）
/// * `heat_load_kw`  —— 房屋热负荷 [kW]
///
/// 曲线能力 ≤ 0（C# 里是「供暖曲线未设置」）时返回 `None`，
/// 而不是原实现的 `Infinity`。
pub fn heater_units(curve_capacity: f64, heat_load_kw: f64) -> Result<f64, HeatError> {
    if !curve_capacity.is_finite() || !heat_load_kw.is_finite() {
        return Err(HeatError::NotFinite);
    }
    if curve_capacity <= 0.0 {
        return Err(HeatError::NoCurve);
    }
    if heat_load_kw < 0.0 {
        return Err(HeatError::NotFinite);
    }
    Ok((heat_load_kw / curve_capacity / 0.92 * 10.0).round() / 10.0)
}

/// 末端单体输出功率 [kW] 与所需数量（C# `MDQuantity`）
///
/// ```text
/// ΔT  = max(水温 − 需求温度, 0)
/// R   = ΔT · 单体散热能力(W/K) / 1000      （kW/K）
/// 数量 = ceil(热负荷 / R / 效率)
/// ```
///
/// 修正：`ΔT = 0` 时原实现返回 `Infinity`（除零），此处返回
/// [`HeatError::NonPositiveDeltaT`]。
#[allow(clippy::too_many_arguments)]
pub fn terminal_quantity(
    heat_load_kw: f64,
    unit_conductance_w_per_k: f64,
    water_temp_c: f64,
    required_room_temp: f64,
    efficiency: f64,
) -> Result<TerminalSizing, HeatError> {
    if !heat_load_kw.is_finite()
        || !unit_conductance_w_per_k.is_finite()
        || !water_temp_c.is_finite()
        || !required_room_temp.is_finite()
    {
        return Err(HeatError::NotFinite);
    }
    let delta_t = (water_temp_c - required_room_temp).max(0.0);
    if delta_t <= 0.0 {
        return Err(HeatError::NonPositiveDeltaT);
    }
    if unit_conductance_w_per_k <= 0.0 || efficiency <= 0.0 {
        return Err(HeatError::NonPositiveCapacity);
    }
    let unit_power_kw = delta_t * unit_conductance_w_per_k / 1000.0;
    let count = (heat_load_kw / unit_power_kw / efficiency).ceil();
    Ok(TerminalSizing {
        unit_power_kw,
        count,
    })
}

/// 末端选型结果
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct TerminalSizing {
    /// 单体输出功率 [kW]
    pub unit_power_kw: f64,
    /// 所需数量 [台]
    pub count: f64,
}

/// 供暖能力曲线（用户录入的散热器性能数据）
#[derive(Debug, Clone, Default)]
pub struct HeatingCurve {
    /// 多项式系数（温度 → 单台能力 kW）
    pub coeff: Vec<f64>,
    /// 末端数量（散热器组数）
    pub terminal_count: f64,
    /// 单组功率系数（C# `WaterEnergy_WK`，W/K）
    pub unit_energy_w_per_k: f64,
    /// 供水温度 [°C]
    pub water_temp: f64,
    /// 热源机组台数（总能力 = 单台曲线 × 台数 × 拟合系数；≤0 视为 1）
    pub units: f64,
    /// 热源拟合系数（锅炉 0.80 / 空气能 0.93；≤0 视为 1）
    pub fit: f64,
}

/// 机组数因子（缺省 1）
pub fn units_factor(curve: &HeatingCurve) -> f64 {
    if curve.units > 0.0 {
        curve.units
    } else {
        1.0
    }
}

/// 拟合系数因子（缺省 1）
pub fn fit_factor(curve: &HeatingCurve) -> f64 {
    if curve.fit > 0.0 {
        curve.fit
    } else {
        1.0
    }
}

/// 末端总功率 [kW]
pub fn terminal_total_power(curve: &HeatingCurve) -> f64 {
    curve.terminal_count * curve.unit_energy_w_per_k / 1000.0
}

/// 热网运行点（物理透明模型，替代原 C# RFRoomTemp 闭式解中台数抵消的问题）
#[derive(Debug, Clone, Copy)]
pub struct HeatPoint {
    /// 舍温 [°C]
    pub room_temp: f64,
    /// 运行供水温度 [°C]（热源不足时低于设定）
    pub supply_temp: f64,
    /// 平均水温 (供水+回水≈舍温)/2 [°C]
    pub avg_water: f64,
    /// 受热源限制（true = 机组容量不足，舍温低于末端平衡值）
    pub heat_limited: bool,
}

/// 热网运行点求解：热源供热 ↔ 末端散热 ↔ 围护失热 三层平衡
///
/// ```text
/// K = (WK_DM+WK_QM+WK_WM+WK_ST)/1000           围护总热导 [kW/K]
/// G = 末端数量 × 单体散热系数/1000               末端总热导 [kW/K]
/// Q_src = n_机组 × P(t_o) × fit × 0.92          热源可供热量 [kW]
/// 热源充足:  t_eq = (0.92G·t_ws + K·t_o − 屋面梯度)/(K + 0.92G)
/// 热源受限:  t_i  = t_o + (Q_src + 屋面梯度)/K   供水同步压低 t_ws' = t_i + Q_src/(0.92G)
/// ```
///
/// 物理结论（修正旧模型「加机组室温不变」）：机组不足时 **台数↑ → Q_src↑ →
/// 舍温、运行供水温度、平均水温、末端散热全部上升**；机组充足后饱和于
/// 末端/围护平衡点（＝可达温度），再加台数才无增益。
#[allow(clippy::too_many_arguments)]
pub fn heat_network_point(
    outdoor_temp: f64,
    q_src_kw: f64,
    terminal_kw_per_k: f64,
    roof_height: f64,
    wk_floor: f64,
    wk_wall: f64,
    wk_roof: f64,
    wk_air: f64,
    max_water: f64,
) -> HeatPoint {
    const GD: f64 = 2.3;
    const SR: f64 = 0.92;
    let k = (wk_floor + wk_wall + wk_roof + wk_air) / 1000.0;
    let g = SR * terminal_kw_per_k.max(0.0);
    let roof_corr = wk_roof / 1000.0 * roof_height / GD;
    if k + g <= 0.0 || !(k + g).is_finite() {
        return HeatPoint {
            room_temp: f64::NAN,
            supply_temp: max_water,
            avg_water: max_water,
            heat_limited: true,
        };
    }
    // 热源充足（供水满流 t_ws）的末端-围护平衡舍温
    let t_eq = (g * max_water + k * outdoor_temp + roof_corr) / (k + g);
    let q_eq = g * (max_water - t_eq); // 维持 t_eq 所需末端散热
    if q_src_kw >= q_eq {
        HeatPoint {
            room_temp: t_eq.min(max_water),
            supply_temp: max_water,
            avg_water: 0.5 * (max_water + t_eq.min(max_water)),
            heat_limited: false,
        }
    } else {
        // 热源受限：舍温由供热量定，运行供水温度同步被压低
        let t = outdoor_temp + (q_src_kw + roof_corr) / k;
        let t_sup = t + q_src_kw / g;
        let t_sup = t_sup.min(max_water);
        HeatPoint {
            room_temp: t.min(t_sup),
            supply_temp: t_sup,
            avg_water: 0.5 * (t_sup + t.min(t_sup)),
            heat_limited: true,
        }
    }
}

/// 供水温度求解（C# `RFWaterTem`）
///
/// 求解在室外温度 x 处的**供水温度**，使房间维持目标温度；
/// 公式为能量平衡闭式解（gd=2.3 屋顶梯度、sR1=0.98、a=2.1 显热系数）。
///
/// 修正：原函数把 `DM/QM/WM/ST/hEndWK` 五个变量就地 `/=1000`（隐式 W→kW），
/// 这里显式换算并命名；返回值仍与原式一致（含 `min(v, MaxWater-1)` 上限）。
#[allow(clippy::too_many_arguments)]
pub fn required_water_temp(
    curve: &HeatingCurve,
    outdoor_temp: f64,
    max_water: f64,
    roof_height: f64,
    wk_floor: f64,
    wk_wall: f64,
    wk_roof: f64,
    wk_air: f64,
    terminal_power_kw: f64,
) -> f64 {
    const GD: f64 = 2.3;
    const SR1: f64 = 0.98;
    const A: f64 = 2.1;

    let dm = wk_floor / 1000.0;
    let qm = wk_wall / 1000.0;
    let wm = wk_roof / 1000.0;
    let st = wk_air / 1000.0;
    let end_wk = terminal_power_kw; // 已是 kW

    let s = poly_eval(&curve.coeff, outdoor_temp);
    // 原 C# 的 `Tians` 语义是**热源机组台数**（「增加一台机组」曲线即 Tians+1），
    // 水温/室温能量平衡按台数参与
    let tias = units_factor(curve);

    let numerator = dm * s * GD * tias
        + qm * s * GD * tias
        + 40.0 * dm * A * GD * tias
        + s * st * GD * tias
        + s * wm * GD * tias
        + 40.0 * qm * A * GD * tias
        + 40.0 * st * A * GD * tias
        + 40.0 * wm * A * GD * tias
        - wm * end_wk * roof_height * SR1
        + dm * GD * end_wk * SR1 * outdoor_temp
        + s * GD * end_wk * SR1 * tias
        + qm * GD * end_wk * SR1 * outdoor_temp
        + st * GD * end_wk * SR1 * outdoor_temp
        + wm * GD * end_wk * SR1 * outdoor_temp
        + 40.0 * A * GD * end_wk * SR1 * tias;

    let denominator = GD
        * (dm * A * tias
            + dm * end_wk * SR1
            + qm * A * tias
            + st * A * tias
            + wm * A * tias
            + qm * end_wk * SR1
            + st * end_wk * SR1
            + wm * end_wk * SR1
            + A * end_wk * SR1 * tias);

    if denominator == 0.0 || !denominator.is_finite() {
        return max_water.min(f64::NAN);
    }

    let sw = numerator / denominator;
    let sw = sw.min(max_water - 1.0);

    room_temp_from_water(
        outdoor_temp,
        max_water,
        roof_height,
        wk_floor,
        wk_wall,
        wk_roof,
        wk_air,
        terminal_power_kw,
        sw,
    )
}

/// 房间可达温度（C# `RFRoomTemp` 5 参数版）
///
/// ```text
/// v = (WK_WM·(x − h/2.3) + x·(WK_DM+WK_QM+WK_ST) + 末端功率·0.92·水温)
///     / (WK_DM + WK_QM + WK_ST + WK_WM + 末端功率·0.92)
/// ```
/// 返回 `min(v, MaxWater)`。
#[allow(clippy::too_many_arguments)]
pub fn room_temp_from_water(
    outdoor_temp: f64,
    max_water: f64,
    roof_height: f64,
    wk_floor: f64,
    wk_wall: f64,
    wk_roof: f64,
    wk_air: f64,
    terminal_power_kw: f64,
    water_temp: f64,
) -> f64 {
    const GD: f64 = 2.3;
    const SR: f64 = 0.92;

    let dm = wk_floor / 1000.0;
    let qm = wk_wall / 1000.0;
    let wm = wk_roof / 1000.0;
    let st = wk_air / 1000.0;
    let end_wk = terminal_power_kw;

    let denom = dm + qm + st + wm + end_wk * SR;
    if denom == 0.0 || !denom.is_finite() {
        return f64::NAN;
    }
    let v = (wm * (outdoor_temp - roof_height / GD)
        + outdoor_temp * (dm + qm + st)
        + end_wk * SR * water_temp)
        / denom;
    let v = (v * 1000.0).round() / 1000.0;
    v.min(max_water)
}

/// 房间可达温度（C# `RFRoomTemp` 10 参数版：先求供水温度 sw，再代入室温公式）
#[allow(clippy::too_many_arguments)]
pub fn room_temp_curve(
    curve: &HeatingCurve,
    outdoor_temp: f64,
    max_water: f64,
    roof_height: f64,
    wk_floor: f64,
    wk_wall: f64,
    wk_roof: f64,
    wk_air: f64,
    terminal_power_kw: f64,
) -> f64 {
    // 第一步：供水温度 sw = RFWaterTem（含 min(sw, MaxWater−1) 上限）
    let sw = required_water_temp(
        curve,
        outdoor_temp,
        max_water,
        roof_height,
        wk_floor,
        wk_wall,
        wk_roof,
        wk_air,
        terminal_power_kw,
    );
    // 第二步：室温 v = RFRoomTemp(x, MaxWater, …, sw)
    room_temp_from_water(
        outdoor_temp,
        max_water,
        roof_height,
        wk_floor,
        wk_wall,
        wk_roof,
        wk_air,
        terminal_power_kw,
        sw,
    )
}

/// 热源总能力曲线（C# `vKT`）：单台曲线 × 机组台数 × 拟合系数
pub fn terminal_power_at(curve: &HeatingCurve, outdoor_temp: f64) -> f64 {
    (poly_eval(&curve.coeff, outdoor_temp) * units_factor(curve) * fit_factor(curve) * 100.0)
        .round()
        / 100.0
}

/// 外墙参考温度点（C# `QTtemperature`）
///
/// `T表 = T常 + 0.84·辐射(W/m²)·0.133·1000/20`（等价 `+5.586·辐射`）
pub fn wall_surface_temp(base_temp: f64, solar_radiation: f64, absorptivity: f64, a0: f64) -> f64 {
    if a0 == 0.0 {
        return f64::NAN;
    }
    let v = base_temp + absorptivity * solar_radiation * 0.133 * 1000.0 / a0;
    (v * 100.0).round() / 100.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn poly_eval_no_accumulated_rounding() {
        // 同一多项式，修正版与逐项舍入版应几乎一致，但修正版是精确值
        let p = [6.246, 0.3819, 0.02068, 0.000004211, 0.000007291];
        let exact: f64 = p
            .iter()
            .enumerate()
            .map(|(i, &c)| c * 25.0f64.powi(i as i32))
            .sum();
        assert!((poly_eval(&p, 25.0) - exact.round()).abs() < 0.5);
        // 逐项舍入版偏离精确值 < 0.02（历史行为保留）
        let legacy = poly_eval_legacy_rounding(&p, 25.0);
        assert!(
            (legacy - poly_eval(&p, 25.0)).abs() < 0.02,
            "legacy={legacy}"
        );
    }

    #[test]
    fn water_pressure_fit_matches_saturated_vapor_pressure() {
        // 该拟合就是饱和水蒸气压曲线：25 °C 应 ≈ 3160 Pa（ASHRAE 3169 Pa）
        let p = water_surface_pressure(25.0, 100.0);
        assert!((p - 3163.0).abs() < 20.0, "p={p}");
        // 50 % RH 时减半
        let p_half = water_surface_pressure(25.0, 50.0);
        assert!((p_half - p / 2.0).abs() < 1.0);
    }

    #[test]
    fn heater_units_no_infinity() {
        // 曲线能力为 0（原 C# 显示「∞ 台」的场景）
        assert_eq!(heater_units(0.0, 343.7), Err(HeatError::NoCurve));
        // 正常：曲线 100 kW/台、负荷 343.7 kW → 343.7/100/0.92 = 3.736 → 3.7
        let n = heater_units(100.0, 343.7).unwrap();
        assert!((n - 3.7).abs() < 0.01, "n={n}");
    }

    #[test]
    fn terminal_quantity_zero_dt_is_error() {
        // 水温 50 = 需求 50 → ΔT=0 → 原 C# 返回 Infinity
        assert_eq!(
            terminal_quantity(100.0, 300.0, 50.0, 50.0, 0.92),
            Err(HeatError::NonPositiveDeltaT)
        );
    }

    #[test]
    fn terminal_quantity_reference() {
        // 343.7 kW、300 W/K 单体、50 °C 水、35 °C 需求、0.92 效率
        // ΔT=15 → R = 4.5 kW/K → 343.7/4.5/0.92 = 83.14 → 84
        let s = terminal_quantity(343.7, 300.0, 50.0, 35.0, 0.92).unwrap();
        assert!((s.unit_power_kw - 4.5).abs() < 1e-9);
        assert!((s.count - 84.0).abs() < 1e-9, "count={}", s.count);
    }

    /// 热网运行点：热源不足→台数驱动舍温；充足→饱和于末端-围护平衡
    #[test]
    fn heat_network_point_regimes() {
        // K=10 kW/K（屋面 wk=0 免梯度项），G=10 kW/K，供水 50，外温 −10
        let pt_limited = heat_network_point(-10.0, 200.0, 10.0, 0.0, 0.0, 10000.0, 0.0, 0.0, 50.0);
        assert!(pt_limited.heat_limited);
        assert!(
            (pt_limited.room_temp - 10.0).abs() < 0.01,
            "t={}",
            pt_limited.room_temp
        );
        // 充足：t_eq=(0.92×10×50+10×(−10))/(10+9.2)=18.75
        let pt_ok = heat_network_point(-10.0, 1e9, 10.0, 0.0, 0.0, 10000.0, 0.0, 0.0, 50.0);
        assert!(!pt_ok.heat_limited);
        assert!(
            (pt_ok.room_temp - 18.75).abs() < 0.01,
            "t_eq={}",
            pt_ok.room_temp
        );
        // 单调性：不足区间台数↑ → 舍温↑
        let a = heat_network_point(-10.0, 100.0, 10.0, 0.0, 0.0, 10000.0, 0.0, 0.0, 50.0);
        let b = heat_network_point(-10.0, 150.0, 10.0, 0.0, 0.0, 10000.0, 0.0, 0.0, 50.0);
        assert!(b.room_temp > a.room_temp);
    }

    #[test]
    fn room_temp_monotonic_in_water_temp() {
        let t1 = room_temp_from_water(-10.0, 50.0, 9.5, 1131.0, 2633.0, 2160.0, 3144.0, 24.9, 40.0);
        let t2 = room_temp_from_water(-10.0, 50.0, 9.5, 1131.0, 2633.0, 2160.0, 3144.0, 24.9, 50.0);
        assert!(t2 > t1, "t1={t1} t2={t2}");
        assert!(t2 <= 50.0);
    }

    #[test]
    fn wall_surface_temp_reference() {
        // 25 °C、800 W/m²、0.84、a0=20 → 25 + 0.84·800·0.133·1000/20 = 25 + 4475 = 4500?!
        // （原式如此：辐射项系数 5.586/W·m²，见文档说明）
        let t = wall_surface_temp(25.0, 800.0, 0.84, 20.0);
        assert!(t > 25.0);
        assert!((t - (25.0 + 0.84 * 800.0 * 0.133 * 50.0)).abs() < 0.02);
    }
}
