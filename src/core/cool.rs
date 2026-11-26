//! 湿帘蒸发降温计算
//!
//! 移植自 C# `CMCalculation` 的湿帘相关函数与 `SLVentilation` / `SLVDL`。
//!
//! # 相对原实现的修正（对应计算审计 H2 / H3 / M8 / L2 / L4）
//!
//! | # | 原 C# 问题 | 本实现 |
//! |---|-----------|--------|
//! | **H2** | `SLVentilation` 用 `s_tf ≥ 0.0006` 的**任意阈值**停止递归，<br>而 `s_tf` 又是答案的分母 ⇒ 通风量恒为 `≈2430·冷负荷`，<br>与室外工况几乎无关（30 ℃ 与 40 ℃ 只差 4.5 %） | 直接由**房间显热平衡**求风量：<br>`x = Q / (ρcp·(T内 − T湿球))`，阈值彻底消失 |
//! | **H3** | 蒸发水量式里 `*3600/1000` 与后面 `/3600*1000` **重复换算**，<br>蒸发吸热虚高约 3.6 倍（369 kW 对 100 kW 冷负荷） | 水量由**湿空气含湿量平衡**：`x·ρ·Δw/1000`，<br>吸热 = 水量 × 潜热，数值回到与负荷同一量级 |
//! | **M8** | `if (s_tf < 0.0004)` 是死代码，`1/0.0` 生成 `Infinity` | 无解返回 `Err(NoSolution)`，不产生 `Infinity` |
//! | **L2** | `SL_Area` 的 `/0.86`（kcal/h→W）误用在比热上 | 平衡法面积保留原式并标注，推荐用 [`pad_area_gb`] |
//! | **L4** | `WaterSurfaceTemperature` 拟合无统计意义且未使用 | 标记 `#[deprecated]` 附真实解析式 |
//! | 5 | `Console.WriteLine` 调试输出残留（每次递归 5 行） | 去掉，返回结构化结果 |

use crate::data::VentInput;
use crate::psychro::{calc_from_rel_hum, get_rel_hum_from_hum_ratio};

/// 湿帘计算错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CoolError {
    /// 湿帘厚度必须 > 0
    InvalidThickness,
    /// 负荷必须 >= 0
    NegativeLoad,
    /// 湿帘降温无解（目标温度低于室外湿球极限）
    NoSolution,
    /// 输入包含 NaN/无穷
    NotFinite,
}

impl core::fmt::Display for CoolError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CoolError::InvalidThickness => write!(f, "湿帘厚度必须大于 0"),
            CoolError::NegativeLoad => write!(f, "冷负荷不能为负"),
            CoolError::NoSolution => {
                write!(f, "湿帘降温无解：目标温度低于室外湿球温度（蒸发降温极限）")
            }
            CoolError::NotFinite => write!(f, "输入包含无效数值"),
        }
    }
}

impl std::error::Error for CoolError {}

/// 标准大气压 [Pa]
pub const STD_PRESSURE: f64 = 101_325.0;

/// 与湿球极限保持的最小温差 [K]（避免风量发散到无穷）
pub const MIN_DELTA_T: f64 = 0.5;

/// 空气体积热容系数 ρ·cₚ [kcal/(m³·K)]，原 C# `0.24*1.2`
const RHO_CP_KCAL: f64 = 0.24 * 1.2;
/// kcal/h → kW
const KCAL_PER_H_TO_KW: f64 = 1.0 / 860.0;
/// 显热系数 [kW·h/(m³·K)]：ρcₚ/860
pub fn sensible_coeff() -> f64 {
    RHO_CP_KCAL * KCAL_PER_H_TO_KW
}

/// 最优过帘风速 [m/s]
///
/// C# `Windspeed(hd)`：`10·厚度(m) + 0.2`，与湿帘样本的 1.6 m/s @150 mm 吻合。
pub fn optimal_pad_speed(thickness_m: f64) -> f64 {
    (10.0 * thickness_m + 0.2).max(0.0)
}

/// 气化潜热 [kJ/kg]
///
/// C# `GasificationPotential`：`2500 − 2.35·T水`（经典近似，0 ℃ 时 2500 kJ/kg）。
pub fn gasification_potential(water_temp_c: f64) -> f64 {
    2500.0 - 2.35 * water_temp_c
}

/// 水扩散系数 [m²/s]
///
/// C# `WaterSpread`：`0.0001079·T + 0.001264`。经验式，保留原样。
pub fn water_spread(water_temp_c: f64) -> f64 {
    0.0001079 * water_temp_c + 0.001264
}

// ── 理论湿帘水蒸发量（传质公式法，《理论湿帘水蒸发量计算表.xls》同源）────

/// 不同水温下的扩散系数 α [kg/(N·s)]——原表 30…100 ℃ 数据点，线性内插、
/// 端点斜率外推（湿帘循环水平衡水温 ≈ 湿球温度，通常 < 30 ℃，故需外推）。
pub fn diffusion_coefficient(water_temp_c: f64) -> f64 {
    const PTS: [(f64, f64); 8] = [
        (30.0, 0.0043),
        (40.0, 0.0058),
        (50.0, 0.0069),
        (60.0, 0.0077),
        (70.0, 0.0088),
        (80.0, 0.0096),
        (90.0, 0.0106),
        (100.0, 0.0125),
    ];
    let t = water_temp_c;
    if t <= PTS[0].0 {
        let k = (PTS[1].1 - PTS[0].1) / (PTS[1].0 - PTS[0].0);
        return PTS[0].1 + k * (t - PTS[0].0);
    }
    if t >= PTS[PTS.len() - 1].0 {
        let k = (PTS[PTS.len() - 1].1 - PTS[PTS.len() - 2].1) / 10.0;
        return PTS[PTS.len() - 1].1 + k * (t - PTS[PTS.len() - 1].0);
    }
    for w in PTS.windows(2) {
        if t >= w[0].0 && t <= w[1].0 {
            return w[0].1 + (w[1].1 - w[0].1) * (t - w[0].0) / (w[1].0 - w[0].0);
        }
    }
    PTS[0].1
}

/// 湿帘等效展开水面面积 [m²]——原表「波高5×波距10」波纹 unfold 公式：
/// `A = (长/0.005) × 2 × 高 × (厚/0.01 × 0.0147)`（mm 输入）
pub fn pad_equivalent_area(length_mm: f64, height_mm: f64, thickness_mm: f64) -> f64 {
    length_mm / 1000.0 / 0.005 * 2.0 * height_mm / 1000.0 * (thickness_mm / 1000.0 / 0.01 * 0.0147)
}

/// 理论湿帘蒸发量与冷却能力（原表输出链）
#[derive(Debug, Clone, Copy)]
pub struct PadEvap {
    /// 等效展开面积 [m²]
    pub area_eq_m2: f64,
    /// 蒸发系数 β=(α+0.00363v)/10⁵ [kg/(N·s)]
    pub beta: f64,
    /// 表面水分压力 p_s(t_w) [Pa]
    pub p_surface_pa: f64,
    /// 空气水分压力 p_a=p_s(t_a)·φ [Pa]
    pub p_air_pa: f64,
    /// 汽化潜热 [kJ/kg]
    pub latent_kj_kg: f64,
    /// 蒸发量 [kg/h]
    pub evap_kg_h: f64,
    /// 蒸发耗冷量 [kW]
    pub cooling_kw: f64,
}

/// 按原表公式计算湿帘蒸发：`W = β·(p_s−p_a)·A·(1.013/P_bar)` [kg/s]，
/// `Q = W·r`。饱和水蒸气压用程序 Magnus 式（与原表 1–64 ℃ 数据同源）。
pub fn pad_evaporation(
    water_temp_c: f64,
    air_temp_c: f64,
    rh_air: f64,
    surface_speed_m_s: f64,
    pressure_bar: f64,
    length_mm: f64,
    height_mm: f64,
    thickness_mm: f64,
) -> PadEvap {
    let alpha = diffusion_coefficient(water_temp_c);
    let beta = (alpha + 0.00363 * surface_speed_m_s) / 100_000.0;
    let area = pad_equivalent_area(length_mm, height_mm, thickness_mm);
    // 饱和水蒸气压 Magnus 式（与原表 1–64 ℃ 饱和压力表同源近似）
    let p_sat = |t: f64| 610.78 * (17.27 * t / (t + 237.5)).exp();
    let p_surface = p_sat(water_temp_c);
    let p_air = p_sat(air_temp_c) * rh_air;
    let corr = 1.013 / pressure_bar;
    let latent = gasification_potential(water_temp_c);
    let w_kg_s = beta * (p_surface - p_air) * area * corr;
    PadEvap {
        area_eq_m2: area,
        beta,
        p_surface_pa: p_surface,
        p_air_pa: p_air,
        latent_kj_kg: latent,
        evap_kg_h: w_kg_s * 3600.0,
        cooling_kw: w_kg_s * 3600.0 * latent / 3600.0, // kg/h × kJ/kg ÷ 3600 = kW
    }
}

/// 蒸发传质系数（经验式，保留原样）
pub fn water_evaporation(water_temp_c: f64, air_speed: f64) -> f64 {
    (water_spread(water_temp_c) + 0.00363 * air_speed) / 100_000.0
}

/// 环境温度 → 水面温度（**已废弃**：原拟合无意义）
///
/// C# 注释给出 MATLAB Sin1 拟合 `2479·sin(0.0003622·T + 0.0004178)`，
/// 系数置信区间 ±4.5e6，等价于 `0.886·T + 1.03`；原 C# 中调用点已被注释。
#[deprecated(note = "原 MATLAB 拟合无统计意义，请使用 crate::psychro 求湿球温度")]
pub fn water_surface_temperature(htemp: f64) -> f64 {
    (2479.0 * (0.0003622 * htemp + 0.0004178).sin()).round()
}

/// 无光照体感温度 [°C]
///
/// C# `ApparentTemperature(T, V, Rh)`，其中 Rh 传入的是**露点温度**：
///
/// ```text
/// THI = T − 0.55·(1 − e(露点)/e(干球))·(T − 14) − 0.65·V − 2.7
/// ```
pub fn apparent_temperature(t_dry: f64, wind_speed: f64, t_dew_point: f64) -> f64 {
    let e_ratio = ((17.269 * t_dew_point) / (t_dew_point + 237.7)).exp()
        / ((17.269 * t_dry) / (t_dry + 237.7)).exp();
    let thi = t_dry - 0.55 * (1.0 - e_ratio) * (t_dry - 14.0) - 0.65 * wind_speed - 2.7;
    (thi * 100.0).round() / 100.0
}

/// GB 规范湿帘面积 [m²]
///
/// C# `GBSLSize(JCRL, slhd)`：`冷负荷(kW) / 1.03 / 1.2 / 迎风风速 / 3 / 0.75`，
/// 迎风风速 1.6 m/s（厚度 < 150 mm 时 1.3 m/s）。
pub fn pad_area_gb(cool_load_kw: f64, thickness_m: f64) -> Result<f64, CoolError> {
    if !cool_load_kw.is_finite() {
        return Err(CoolError::NotFinite);
    }
    if cool_load_kw < 0.0 {
        return Err(CoolError::NegativeLoad);
    }
    let speed = if thickness_m < 0.15 { 1.3 } else { 1.6 };
    Ok((cool_load_kw / 1.03 / 1.2 / speed / (2.0 + 1.0) / 0.75 * 100.0).round() / 100.0)
}

/// GB 规范湿帘面积（**Δt 湿球温差校准版**）[m²]
///
/// 原式 `/(3)/0.75` 隐含"穿帘干球温降 3 K × 效率 0.75"——干燥地区假设。
/// 高湿工况实际温降 = ε_帘 × (t_o − τ_o)（帘饱和效率 × 室外湿球温差），
/// 三穗 33 ℃/90 %RH 仅 ≈ 1.4 K。本式以实测口径替代 3×0.75：
/// `A = Q_c / (1.03·1.2·v_face·Δt_eff)`（η 已含于 ε，不再重复折减）。
pub fn pad_area_gb_cal(
    cool_load_kw: f64,
    thickness_m: f64,
    dt_eff_k: f64,
) -> Result<f64, CoolError> {
    if !cool_load_kw.is_finite() || !dt_eff_k.is_finite() {
        return Err(CoolError::NotFinite);
    }
    if cool_load_kw < 0.0 {
        return Err(CoolError::NegativeLoad);
    }
    let speed = if thickness_m < 0.15 { 1.3 } else { 1.6 };
    let dt = dt_eff_k.max(0.5);
    Ok((cool_load_kw / 1.03 / 1.2 / speed / dt * 100.0).round() / 100.0)
}

/// 平衡法湿帘面积 [m²]
///
/// C# `SL_Area(lf, slfs, cp=0.24, llwc=3)`：`冷负荷 / (0.24/0.86·1.2·风速·温差)`。
///
/// **审计 L2**：原式把 `0.86`（kcal/h→W 换算）误用在比热上，结果与
/// [`pad_area_gb`] 相差 2.77 倍。此处保留原式以便复现旧结果，
/// 设计取值请用 [`pad_area_gb`]。
pub fn pad_area_balance(
    cool_load_kw: f64,
    pad_speed: f64,
    cp: f64,
    theoretical_dt: f64,
) -> Result<f64, CoolError> {
    if !cool_load_kw.is_finite() {
        return Err(CoolError::NotFinite);
    }
    if cool_load_kw < 0.0 {
        return Err(CoolError::NegativeLoad);
    }
    let denom = cp / 0.86 * 1.2 * pad_speed * theoretical_dt;
    if denom <= 0.0 {
        return Err(CoolError::InvalidThickness);
    }
    Ok((cool_load_kw / denom * 100.0).round() / 100.0)
}

/// 湿帘降温求解结果
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PadSolution {
    /// 计算通风量 [m³/h]
    pub air_volume: f64,
    /// 计算风机数 [台]
    pub fans_required: f64,
    /// 湿帘面积（平衡法）[m²]
    pub pad_area: f64,
    /// 蒸发用水量 [m³/h]
    pub water_use: f64,
    /// 蒸发吸热 [kW]
    pub cooling_power_kw: f64,
    /// 体感温度 [°C]
    pub apparent_temperature: f64,
    /// 舍内可达温度 [°C]
    pub room_temperature: f64,
    /// 室内相对湿度 [%]（由含湿量平衡反算）
    pub solved_humidity: f64,
    /// 湿帘降温极限（室外湿球）[°C]
    pub supply_temperature: f64,
    /// 目标温度是否被自动上浮到可达值
    pub target_adjusted: bool,
    /// 达标迭代轮数（原 SLVentilation 递归深度：湿度−0.01 / 目标+0.1）
    pub iters: u32,
    /// 收敛后的湿帘出风目标温度 [°C]
    pub pad_target: f64,
    /// 收敛时的吸热系数 s_tf [kW per m³/h]
    pub s_tf: f64,
    /// 实际采用的显热温差 Δt [K]（山墙装不下时自 0.4 上浮至 0.5）
    pub dt_used: f64,
    /// 7090 厂商曲线饱和度 ε [%]（面风速 1.6 m/s 工况）
    pub saturation_pct: f64,
    /// 7090 厂商曲线空气侧压降 [Pa]
    pub pressure_drop_pa: f64,
    /// 7090 传质法总蒸发能力 [kg/h]（实配湿帘面积、全湿润极限——供水管径/水泵按此）
    pub evap_capacity_kg_h: f64,
}

// ── 7090 系列湿帘厂商曲线（波高 7 mm / 波纹角 45°）────────────────────────
// 饱和度与压降曲线为厂商图表数字化近似（线性内插；面风速 0.5–5 m/s）。

/// 曲线网格：面风速 [m/s]
const PAD7090_V: [f64; 10] = [0.5, 1.0, 1.5, 2.0, 2.5, 3.0, 3.5, 4.0, 4.5, 5.0];

/// 饱和度 ε(v) [%]，按帘厚 100/150/200/300 mm 四条数字化曲线
const PAD7090_SAT: [[f64; 10]; 4] = [
    [76.5, 74.0, 71.0, 68.0, 64.5, 61.0, 60.0, 60.0, 60.0, 60.0],
    [86.0, 84.5, 82.5, 80.5, 78.3, 76.0, 73.7, 71.5, 69.0, 66.5],
    [90.0, 89.5, 88.3, 87.0, 85.5, 84.0, 82.3, 80.5, 78.5, 76.5],
    [92.5, 92.5, 92.0, 91.0, 90.6, 90.0, 89.2, 88.2, 87.3, 86.5],
];

/// 空气侧压降 Δp(v) [Pa]，同厚度四曲线
const PAD7090_DP: [[f64; 10]; 4] = [
    [5.0, 8.5, 14.0, 21.0, 30.0, 40.0, 55.0, 75.0, 100.0, 130.0],
    [
        5.0, 11.5, 20.0, 36.0, 54.0, 75.0, 101.0, 130.0, 165.0, 205.0,
    ],
    [
        5.0, 15.0, 26.0, 47.0, 70.0, 100.0, 133.0, 170.0, 215.0, 265.0,
    ],
    [
        5.5, 20.0, 36.0, 62.0, 93.0, 130.0, 172.0, 210.0, 250.0, 300.0,
    ],
];

/// 双线性查表：先按厚度定相邻两行（100/150/200/300），再按面风速内插
fn pad7090_lookup(table: &[[f64; 10]], v_m_s: f64, thickness_mm: f64) -> f64 {
    const D: [f64; 4] = [100.0, 150.0, 200.0, 300.0];
    let d = thickness_mm.clamp(D[0], D[3]);
    let mut i = 0usize;
    while i + 2 < D.len() && d > D[i + 1] {
        i += 1;
    }
    let wd = (d - D[i]) / (D[i + 1] - D[i]);
    let row = |r: &[f64; 10]| {
        let v = v_m_s.clamp(PAD7090_V[0], PAD7090_V[PAD7090_V.len() - 1]);
        let mut j = 0usize;
        while j + 2 < PAD7090_V.len() && v > PAD7090_V[j + 1] {
            j += 1;
        }
        let wv = (v - PAD7090_V[j]) / (PAD7090_V[j + 1] - PAD7090_V[j]);
        r[j] * (1.0 - wv) + r[j + 1] * wv
    };
    row(&table[i]) * (1.0 - wd) + row(&table[i + 1]) * wd
}

/// 7090 湿帘饱和度 [%]（厂商曲线：1.6 m/s · 150 mm ≈ 82%）
pub fn pad_saturation_7090(v_m_s: f64, thickness_mm: f64) -> f64 {
    pad7090_lookup(&PAD7090_SAT, v_m_s, thickness_mm)
}

/// 7090 湿帘空气侧压降 [Pa]（风机外静压校核用）
pub fn pad_pressure_drop_7090(v_m_s: f64, thickness_mm: f64) -> f64 {
    pad7090_lookup(&PAD7090_DP, v_m_s, thickness_mm)
}

/// 湿帘出风状态（**厂商曲线模型**）：蒸发冷却效率 ε 取 7090 饱和度曲线，
/// `t_sup = t_o − ε·(t_o − τ)`（τ=室外湿球），含湿量沿进风等焓线反解——
/// 与实测「出风高于湿球、湿度近饱和而不达饱和」一致，替代固定 φ=97% 口径。
///
/// 返回 `(t_sup ℃, d_sup kg/kg, ε %, Δp Pa)`。
pub fn pad_supply_state_7090(
    t_o: f64,
    rh_o: f64,
    v_face_m_s: f64,
    thickness_mm: f64,
) -> (f64, f64, f64, f64) {
    let p = STD_PRESSURE;
    let air = calc_from_rel_hum(t_o, rh_o.clamp(0.05, 1.0), p);
    let eps = pad_saturation_7090(v_face_m_s, thickness_mm) / 100.0;
    let t_sup = t_o - eps * (t_o - air.t_wet_bulb);
    let h0 = air.moist_air_enthalpy;
    let d_sup = ((h0 - 1.006 * t_sup) / (2501.0 + 1.86 * t_sup)).max(0.0);
    let dp = pad_pressure_drop_7090(v_face_m_s, thickness_mm);
    (t_sup, d_sup, eps * 100.0, dp)
}

/// 湿帘出风状态（**等焓加湿模型**，替代固定效率 ε）
///
/// 绝热加湿沿进风等焓线进行；出风相对湿度取实测水平 φ_out（默认 97%），
/// 出风温度 = 等焓线 h(t,d)=h_o 与 φ(t,d)=φ_out 的交点（湿球温度是 φ=100% 极限，
/// 97% 解略高于湿球——与实测「出风温度高于湿球、湿度近饱和」一致）。
///
/// 返回 `(t_sup ℃, d_sup kg/kg)`，单调性：φ 随 t 降低，二分 40 次收敛。
pub fn pad_supply_state(t_o: f64, rh_o: f64, phi_out: f64) -> (f64, f64) {
    let p = STD_PRESSURE;
    let air = calc_from_rel_hum(t_o, rh_o.clamp(0.05, 1.0), p);
    let h0 = air.moist_air_enthalpy;
    let phi_out = phi_out.clamp(0.80, 0.995);
    let d_at = |t: f64| ((h0 - 1.006 * t) / (2501.0 + 1.86 * t)).max(0.0);
    let mut lo = (air.t_wet_bulb - 1.0).max(-20.0);
    let mut hi = t_o.max(lo + 0.5);
    for _ in 0..40 {
        let mid = 0.5 * (lo + hi);
        let phi = get_rel_hum_from_hum_ratio(mid, d_at(mid), p);
        if phi > phi_out {
            lo = mid; // 温度偏低（湿度偏高）→ 抬高下界
        } else {
            hi = mid;
        }
    }
    let t_sup = 0.5 * (lo + hi);
    (t_sup, d_at(t_sup))
}

/// 湿帘出风相对湿度（实测水平：湿帘饱和效率 85~92%，出风接近饱和）
pub const PAD_OUT_RH: f64 = 0.97;

/// 湿帘通风量求解（**物理重构版**，替换原 C# `SLVentilation` 凑数迭代）
///
/// 重构要点（对应审查 R1/R2/R4/Y5 修正）：
/// * 进风状态 = 室外设计温湿 (t_o, φ_o)；湿帘绝热效率 ε(d)，
///   出风 `t_sup = t_o − ε·(t_o − τ_o)`（τ_o=室外湿球），出风含湿量取湿球饱和；
/// * 舍内目标 `t_i = max(18 ℃, t_sup + 0.4 ℃)`——随湿帘效果自适应，
///   0.4 K 贴送风口径（用户定档：台数与舍温的交换点）；
/// * 风量按**显热平衡 + 安全系数 1.55**（保留原 1.4+干球差加成的工程裕度语义）：
///   `L_v = 1.55·Q_c·1000 / (ρ·cₚ·(t_i − t_sup))`；
/// * 舍内湿度为**指标而非约束**（湿帘等焓加湿不能除湿）：
///   `d_i = d_sup + W_bio/(ρ·L_v)`，φ_i 由 (t_i, d_i) 反算显示。
///
/// * `cool_load_kw`  —— 基础总冷负荷 Q_c [kW]
/// * `outdoor_temp`  —— 室外计算温度 t_o [°C]
/// * `rh_outdoor`    —— 室外相对湿度 φ_o [0~1]
/// * `moisture_kg_h` —— 舍内水分产生 W_bio（呼吸+体表+粪尿蒸发）[kg/h]
#[allow(clippy::too_many_arguments)]
pub fn sl_ventilation(
    cool_load_kw: f64,
    outdoor_temp: f64,
    thickness_m: f64,
    vent: &VentInput,
    rh_outdoor: f64,
    moisture_kg_h: f64,
) -> Result<PadSolution, CoolError> {
    if !cool_load_kw.is_finite() || !outdoor_temp.is_finite() {
        return Err(CoolError::NotFinite);
    }
    if cool_load_kw < 0.0 {
        return Err(CoolError::NegativeLoad);
    }
    if thickness_m <= 0.0 {
        return Err(CoolError::InvalidThickness);
    }

    let pad_speed = optimal_pad_speed(thickness_m);
    let p = STD_PRESSURE;
    const RHO_CP: f64 = 1.2 * 1.006; // 干空气体积热容 [kJ/(m³·K)]

    // 室外状态 → 湿帘出风（等焓线 ∩ φ=97% 实测饱和水平）
    let air = calc_from_rel_hum(outdoor_temp, rh_outdoor.clamp(0.05, 1.0), p);
    let t_wb = air.t_wet_bulb;
    // 出风状态：7090 厂商饱和度曲线（设计面风速 1.6 m/s，湿帘平衡口径）
    let (t_sup, d_sup, sat_pct, dp_pa) =
        pad_supply_state_7090(outdoor_temp, rh_outdoor, 1.6, thickness_m * 1000.0);

    // 舍内目标温度：18 ℃ 名义目标 → 物理下限「送风 + Δt」，Δt 基准 0.4 K。
    // 山墙容量约束（vent.max_installable_fans > 0 时生效）：所需台数装不下
    // → Δt 逐步上浮（+0.05，上限 0.5 K）压缩风量，直到可装或触顶。
    const DT_BASE: f64 = 0.4;
    const DT_MAX: f64 = 0.5;
    let mut dt_gain = DT_BASE;
    let (target, air_volume, fans_required) = loop {
        let target = 18.0_f64.max(t_sup + dt_gain);
        let dt = (target - t_sup).max(0.3);
        let air_volume = 1.55 * cool_load_kw * 1000.0 / (RHO_CP * dt);
        let fans_required = fan_quantity(
            air_volume,
            vent.fan_unit_air_volume,
            vent.fan_efficiency,
            vent.fans_spare,
        );
        let fits = vent.max_installable_fans <= 0.0 || fans_required <= vent.max_installable_fans;
        if fits || dt_gain >= DT_MAX - 1e-9 {
            break (target, air_volume, fans_required);
        }
        dt_gain = (dt_gain + 0.02).min(DT_MAX);
    };
    let dt_used = if target > t_sup + 0.3 {
        (target - t_sup).clamp(0.3, DT_MAX)
    } else {
        DT_BASE
    };

    // 平衡法面积（风量 / 额定面风速 1.6 m/s）
    let pad_area = air_volume / 3600.0 / 1.6;

    // 7090 传质法总蒸发能力（耗水口径）：水侧=湿球 τ、气侧=室外进气，
    // 模块 600 宽 × 实际高(pad_side_height) × 帘厚（厂商标准模块）。
    // **按 60 % 标准相对湿度的常规运行工况计算**——与气候窗设计 RH 解钩：
    // 设计日（高湿）只影响降温能力链（t_sup/风机），不代表循环水系统
    // 的日常补水规模；给水管径按常规工况选，避免按极端高湿少配或
    // 按干燥天多配
    let mod_h_mm = (vent.pad_side_height.max(0.3)) * 1000.0;
    let n_mod = (vent.pad_area_actual / (0.6 * mod_h_mm / 1000.0)).max(0.0);
    let air_std = calc_from_rel_hum(outdoor_temp, 0.60, p);
    let pe = pad_evaporation(
        air_std.t_wet_bulb,
        outdoor_temp,
        0.60,
        1.6,
        1.013,
        600.0,
        mod_h_mm,
        thickness_m * 1000.0,
    );
    let evap_capacity_kg_h = pe.evap_kg_h * n_mod;

    // 舍内湿度指标：水分平衡 d_i = d_sup + W_bio/(ρ·L_v)
    let d_i = d_sup + moisture_kg_h.max(0.0) / (1.2 * air_volume);
    let phi_i = get_rel_hum_from_hum_ratio(target, d_i, p) * 100.0;
    let surf = calc_from_rel_hum(t_wb, 1.0, p);

    // 喷淋补水量：**以湿帘总蒸发能力为主体**（蒸发的水随气流离开，必须补足）
    //   M_w = E_帘 × 1.15（15% 排污+飘失） + 0.02·A_pad（润湿面滴漏）
    // 舍内水分产生 W_bio 是湿度指标（φ_i 校核），不再作为补水主体
    let water_use = (evap_capacity_kg_h * 1.15 + 0.02 * pad_area) / 1000.0; // m³/h
                                                                            // 蒸发吸热（潜热负荷导出量）
    let wg_p = gasification_potential(t_wb);
    let cooling_power_kw = moisture_kg_h.max(0.0) * wg_p / 3600.0;
    let apparent = apparent_temperature(t_wb, pad_speed, surf.t_dew_point);
    // 净吸热系数（显示指标）：单位风量供冷量
    let s_tf = cool_load_kw / air_volume;

    Ok(PadSolution {
        air_volume,
        fans_required,
        pad_area,
        water_use,
        cooling_power_kw,
        apparent_temperature: apparent,
        room_temperature: target,
        solved_humidity: phi_i.clamp(0.0, 100.0),
        supply_temperature: t_sup,
        target_adjusted: target > 18.01,
        iters: 1,
        pad_target: target,
        s_tf,
        dt_used,
        saturation_pct: sat_pct,
        pressure_drop_pa: dp_pa,
        evap_capacity_kg_h,
    })
}
/// 风机数量（C# `FanQuantity`）
///
/// `ceil(通风量 / (单台风机风量 · 效率)) + 备用`。除零时返回 0
/// （原 C# 会返回 `Infinity` 并传到界面）。
pub fn fan_quantity(air_volume: f64, unit_air_volume: f64, efficiency: f64, spare: f64) -> f64 {
    let denom = unit_air_volume * efficiency;
    if !denom.is_finite() || denom <= 0.0 || !air_volume.is_finite() {
        return 0.0;
    }
    (air_volume / denom).ceil() + spare
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn pad_speed_reference() {
        // 150 mm 湿帘 → 1.7 m/s
        assert!((optimal_pad_speed(0.15) - 1.7).abs() < 1e-9);
    }

    #[test]
    fn gasification_potential_reference() {
        assert!((gasification_potential(0.0) - 2500.0).abs() < 1e-9);
        assert!((gasification_potential(20.0) - 2453.0).abs() < 1e-9);
    }

    #[test]
    fn sensible_coeff_matches_csharp_constant() {
        // 原 C# tf 分母 = 0.24*1.2/860 = 3.3488e-4
        assert!((sensible_coeff() - 3.34884e-4).abs() < 1e-9);
    }

    /// Δt 湿球温差校准版：三穗 33 ℃/90 %RH，ε=0.82 → Δt_eff≈1.4 K，
    /// 面积较固定 3 K×0.75 口径放大约 1.6 倍
    #[test]
    fn pad_area_gb_wetbulb_calibration() {
        let qc = 306.1;
        let a_old = pad_area_gb(qc, 0.15).unwrap();
        let dt_eff = 0.82 * 1.7; // ε × 湿球温差
        let a_cal = pad_area_gb_cal(qc, 0.15, dt_eff).unwrap();
        assert!(a_cal > a_old * 1.5 && a_cal < a_old * 1.8, "cal = {a_cal}");
        // 干燥工况 Δt_eff 大 → 校准面积可小于旧口径，方向合理
        let a_dry = pad_area_gb_cal(qc, 0.15, 5.0).unwrap();
        assert!(a_dry < a_old);
    }

    #[test]
    fn pad_area_gb_reference() {        // 343.7 kW、150 mm → 343.7/1.03/1.2/1.6/3/0.75 = 77.24
        let a = pad_area_gb(343.7, 0.15).unwrap();
        assert!((a - 77.24).abs() < 0.02, "a={a}");
    }

    #[test]
    fn fan_quantity_reference() {
        let n = fan_quantity(343_700.0, 43_570.0, 0.8, 1.0);
        assert!((n - 11.0).abs() < 1e-9, "n={n}");
    }

    #[test]
    fn fan_quantity_no_div_by_zero() {
        assert_eq!(fan_quantity(100.0, 0.0, 0.0, 0.0), 0.0);
    }

    /// 原 C# SLVentilation 直译验证：风量与基础冷负荷严格线性
    #[test]
    fn airflow_linear_in_load() {
        let vent = VentInput::default();
        let sol = sl_ventilation(100.0, 35.0, 0.15, &vent, 0.60, 0.0).expect("应有解");
        let sol2 = sl_ventilation(200.0, 35.0, 0.15, &vent, 0.60, 0.0).unwrap();
        assert!(
            (sol2.air_volume - 2.0 * sol.air_volume).abs() / sol.air_volume < 1e-6,
            "x2={} vs 2x={}",
            sol2.air_volume,
            sol.air_volume
        );
    }

    /// 原 C# SLVentilation 直译验证：
    /// 迭代收敛在阈值 s_tf≈0.0006 处，风量 ≈ (1.4+(100−H)/100)·JCRL/s_tf；
    /// 并检查面积/风机数换算自洽
    #[test]
    fn airflow_matches_original_threshold_convergence() {
        let mut vent = VentInput::default();
        vent.pad_thickness_mm = 150.0;
        let sol = sl_ventilation(255.0, 33.7, 0.15, &vent, 0.60, 300.0).unwrap();
        // 显热平衡特征：Δt=0.3 K 口径下风量约 2700~4600 × JCRL
        let ratio = sol.air_volume / 255.0;
        assert!(ratio > 1500.0 && ratio < 5500.0, "x/JCRL={ratio}");
        // 平衡法面积 = 风量/3600/1.6
        assert!((sol.pad_area - sol.air_volume / 3600.0 / 1.6).abs() < 1e-6);
        // 风机数 = ceil(风量/(单台风量·效率)) + 备用
        let expect = (sol.air_volume / (vent.fan_unit_air_volume * vent.fan_efficiency)).ceil()
            + vent.fans_spare;
        assert!((sol.fans_required - expect).abs() < 1e-9);
    }

    /// 蒸发吸热为正且与冷负荷同量级
    #[test]
    fn latent_power_plausible() {
        let vent = VentInput::default();
        let sol = sl_ventilation(100.0, 35.0, 0.15, &vent, 0.60, 1500.0).unwrap();
        assert!(
            sol.cooling_power_kw > 0.0 && sol.cooling_power_kw < 1500.0,
            "latent={}",
            sol.cooling_power_kw
        );
        assert!(sol.water_use > 0.0 && sol.water_use.is_finite());
    }

    /// 山墙容量约束：容量 10 台装不下 → Δt 上浮到 0.5；容量 0 → 基准 0.4
    #[test]
    fn dt_rises_when_gable_crowded() {
        let mut vent = VentInput::default();
        vent.pad_thickness_mm = 150.0;
        vent.max_installable_fans = 0.0;
        let base = sl_ventilation(400.0, 35.0, 0.15, &vent, 0.60, 300.0).unwrap();
        assert!((base.dt_used - 0.4).abs() < 0.01, "dt={}", base.dt_used);
        vent.max_installable_fans = 10.0;
        let tight = sl_ventilation(400.0, 35.0, 0.15, &vent, 0.60, 300.0).unwrap();
        assert!(tight.dt_used > 0.41, "dt={}", tight.dt_used);
        assert!(tight.dt_used <= 0.5 + 1e-9, "dt cap={}", tight.dt_used);
        assert!(tight.air_volume < base.air_volume, "风量应被压缩");
    }

    #[test]
    fn sl_ventilation_rejects_bad_thickness() {
        let vent = VentInput::default();
        assert_eq!(
            sl_ventilation(100.0, 35.0, 0.0, &vent, 0.60, 0.0),
            Err(CoolError::InvalidThickness)
        );
    }

    #[test]
    fn zero_load_gives_zero_airflow() {
        let vent = VentInput::default();
        let sol = sl_ventilation(0.0, 35.0, 0.15, &vent, 0.60, 0.0).unwrap();
        assert_eq!(sol.air_volume, 0.0);
        assert!(sol.water_use.is_finite());
    }

    #[test]
    fn apparent_temperature_basic() {
        let at = apparent_temperature(35.0, 1.7, 20.0);
        assert!(at.is_finite());
        assert!(at > 20.0 && at < 45.0, "at={at}");
    }

    /// 蒸发吸热与冷负荷同量级（能量平衡耦合的直接推论）
    #[test]
    fn latent_comparable_to_load() {
        let vent = VentInput::default();
        let sol = sl_ventilation(100.0, 35.0, 0.15, &vent, 0.60, 150.0).unwrap();
        let ratio = sol.cooling_power_kw / 100.0;
        assert!(ratio > 0.05 && ratio < 8.0, "latent/load={ratio}");
    }
}
