//! 通风设备选型：小窗、风机
//!
//! 移植自 C# `CMCalculation`（`WindowQuantity`、`WindowZYQuantity`、
//! `FanQuantity`、`QWindowSpeed`）。
//!
//! # 相对原实现的修正
//! 1. `WindowZYQuantity` 的 `Math.Ceiling(v)` 是**无效外层取整**
//!    （v 已被 `ceil(v/2)*2` 变成偶数整数），保留偶数取整语义但去掉冗余。
//! 2. 所有除法（`xcsize·qw=0`、`GZ=0`）改为 `Result`，不再产生 `Infinity`。
//! 3. `QWindowSpeed` 的 `Math.Pow(base, exp)` 在 base<0 时返回 NaN，
//!    增加域检查。

use crate::data::VentInput;
use crate::heat::HeatError;

/// 风机型号（选型库）：分类 + 型号两段存储
#[derive(Debug, Clone)]
pub struct FanModel {
    /// 分类（「推拉风机」/「拢风筒风机」，列表分组标题 + json 字段）
    pub category: &'static str,
    /// 型号（如「1380-1.1KW」——json 独立字段，UI 列表直接显示）
    pub model: &'static str,
    /// 电机功率 [kW]
    pub power_kw: f64,
    /// 额定风量 [m³/h]
    pub air_volume_m3h: f64,
    /// 额定静压 [Pa]（负压工况，通常 −20Pa）
    pub static_pa: i32,
}

/// 常用风机选型库（按产品样本，−20Pa 工况）
pub fn fan_db() -> &'static [FanModel] {
    static DB: &[FanModel] = &[
        // ── 推拉风机 ──
        FanModel {
            category: "推拉风机",
            model: "1380-1.1KW",
            power_kw: 1.1,
            // 样本 −20 Pa 插值（12Pa 38105 / 25Pa 36060）
            air_volume_m3h: 36847.0,
            static_pa: -20,
        },
        FanModel {
            category: "推拉风机",
            model: "1380-1.5KW",
            power_kw: 1.5,
            // 样本 −20 Pa 插值（12Pa 43903 / 25Pa 42029）
            air_volume_m3h: 42750.0,
            static_pa: -20,
        },
        // EuroCave® EC50 50″ 螺旋桨 1.5 HP：样本 0/25/50 Pa 三点曲线，
        // −20 Pa 插值 39,248 m³/h；型号含 "EC50"/"1.5HP" → 启用工作点可变效率
        FanModel {
            category: "推拉风机",
            model: "EC50-1.5HP",
            power_kw: 1.12,
            air_volume_m3h: 39248.0,
            static_pa: -20,
        },
        // ── 拢风筒风机 ──
        FanModel {
            category: "拢风筒风机",
            model: "1380-1.1KW",
            power_kw: 1.1,
            // 50″ 蝴蝶拢风筒样本 −20 Pa 插值（0Pa 43900 / 30Pa 38400）
            air_volume_m3h: 40500.0,
            static_pa: -20,
        },
        FanModel {
            category: "拢风筒风机",
            model: "1380-1.5KW",
            power_kw: 1.5,
            // 51″ 蝴蝶拢风筒样本 −20 Pa 插值（12Pa 47657 / 25Pa 45537）
            air_volume_m3h: 46352.0,
            static_pa: -20,
        },
    ];
    DB
}

/// 山墙风机可装容量（布置口径：洞口 1.4 m、间距 0.30、留边 0.50）
///
/// 与 UI 山墙立面布置一致的行×列估算（三角区按檐口矩形近似取整）：
/// - 每行台数 = ⌊(舍宽 − 2×留边 + 间距) / (洞口 + 间距)⌋
/// - 行数     = ⌊(檐口 + 间距) / (洞口 + 间距)⌋
pub fn gable_fan_capacity(house: &crate::data::HouseInput) -> f64 {
    const HOLE: f64 = 1.4;
    const GAP: f64 = 0.3;
    const EDGE: f64 = 0.5;
    let pitch = HOLE + GAP;
    let cols = ((house.width - 2.0 * EDGE + GAP) / pitch).floor().max(0.0);
    let rows = ((house.eave_height + GAP) / pitch).floor().max(0.0);
    cols * rows
}

/// 进风小窗型号（选型库）
#[derive(Debug, Clone)]
pub struct WindowModel {
    /// 名称
    pub name: &'static str,
    /// 类型（弹簧进风窗 / 弧面进风窗）
    pub kind: &'static str,
    /// 材质
    pub material: &'static str,
    /// 开孔宽 [mm]
    pub w_mm: i32,
    /// 开孔高 [mm]
    pub h_mm: i32,
    /// 通风面积 [m²]（有效通风面积）
    pub vent_area: f64,
}

/// 弹簧进风窗选型库（通风面积按开孔尺寸重算 = W × H）
pub fn window_db() -> &'static [WindowModel] {
    static DB: &[WindowModel] = &[
        WindowModel {
            name: "560×270",
            kind: "弹簧进风窗",
            material: "PS",
            w_mm: 560,
            h_mm: 270,
            vent_area: 0.1512, // 0.56 × 0.27
        },
        WindowModel {
            name: "840×270",
            kind: "弹簧进风窗",
            material: "PS",
            w_mm: 840,
            h_mm: 270,
            vent_area: 0.2268, // 0.84 × 0.27
        },
        WindowModel {
            name: "500×800",
            kind: "弧面进风窗",
            material: "PP+PVC",
            w_mm: 500,
            h_mm: 800,
            vent_area: 0.28, // 0.5 × 0.8 × 0.7（弧面百叶有效系数 0.7）
        },
        WindowModel {
            name: "970×340",
            kind: "弧面进风窗",
            material: "PP+PVC",
            w_mm: 970,
            h_mm: 340,
            vent_area: 0.231, // 0.97 × 0.34 × 0.7（有效通风面积，系数 0.7）
        },
    ];
    DB
}

/// 小窗数量 [个]（C# `WindowQuantity`）
///
/// `ceil(通风量(m³/h) / (单窗面积(m²) · 风速(m/s)) / 3600)`
pub fn window_quantity(
    air_volume_m3_per_h: f64,
    window_area_m2: f64,
    window_speed_m_per_s: f64,
) -> Result<f64, HeatError> {
    if !air_volume_m3_per_h.is_finite()
        || !window_area_m2.is_finite()
        || !window_speed_m_per_s.is_finite()
    {
        return Err(HeatError::NotFinite);
    }
    let denom = window_area_m2 * window_speed_m_per_s * 3600.0;
    if denom <= 0.0 {
        return Err(HeatError::NonPositiveCapacity);
    }
    if air_volume_m3_per_h < 0.0 {
        return Err(HeatError::NotFinite);
    }
    Ok((air_volume_m3_per_h / denom).ceil())
}

/// 最优小窗总数 [个]（C# `WindowZYQuantity`）
///
/// 1. 先把通风量向上取整到单台风机风量的整数倍（`FanEP`）；
/// 2. 再算小窗数 `通风量/(面积·风速)/3600`，与最小值比较；
/// 3. 向上取整到偶数（成对布置）。
///
/// 修正：原 C# 最后还有一层 `Math.Ceiling(v)`，但 v 已是偶数整数，属无效代码；
/// 且 `lp` 被就地改写（副作用），这里改为显式计算。
#[allow(clippy::too_many_arguments)]
pub fn window_optimal_quantity(
    min_count: f64,
    air_volume_m3_per_h: f64,
    window_area_m2: f64,
    window_speed_m_per_s: f64,
    fan_unit_volume: f64,
) -> Result<f64, HeatError> {
    if !min_count.is_finite()
        || !air_volume_m3_per_h.is_finite()
        || !window_area_m2.is_finite()
        || !window_speed_m_per_s.is_finite()
        || !fan_unit_volume.is_finite()
    {
        return Err(HeatError::NotFinite);
    }
    if window_area_m2 <= 0.0 || window_speed_m_per_s <= 0.0 || fan_unit_volume <= 0.0 {
        return Err(HeatError::NonPositiveCapacity);
    }

    // 通风量向上取整到风机风量整数倍
    let rounded_volume = (air_volume_m3_per_h / fan_unit_volume).ceil() * fan_unit_volume;

    let mut v = rounded_volume / (window_area_m2 * window_speed_m_per_s) / 3600.0;
    if v < min_count {
        v = min_count;
    }
    // 成对布置：向上取整到偶数
    Ok((v / 2.0).ceil() * 2.0)
}

/// 最优小窗风速 [m/s]（C# `QWindowSpeed`）
///
/// `(rch + 舍宽·rch/(舍宽+檐口高))^系数`，rch 默认 0.8。
/// 修正：`舍宽 + 檐口高 = 0` 或负底数时返回 `Err`，不再产生 NaN。
pub fn optimal_window_speed(
    width: f64,
    eave_height: f64,
    ratio: f64,
    exponent: f64,
) -> Result<f64, HeatError> {
    if !width.is_finite() || !eave_height.is_finite() || !ratio.is_finite() || !exponent.is_finite()
    {
        return Err(HeatError::NotFinite);
    }
    let denom = width + eave_height;
    if denom == 0.0 {
        return Err(HeatError::NonPositiveCapacity);
    }
    let base = ratio + width * ratio / denom;
    if base < 0.0 {
        return Err(HeatError::NotFinite);
    }
    let v = base.powf(exponent);
    if !v.is_finite() {
        return Err(HeatError::NotFinite);
    }
    Ok((v * 1000.0).round() / 1000.0)
}

/// 等温自由射流轴线速度衰减（ASHRAE 湍流射流，阿尔伯特模型）
///
/// `v(x) = v₀ · 0.48 / (α·x/d₀ + 0.147)`，d₀ = √(4A/π) 开孔当量直径，α = 0.076 湍流系数。
/// 用于校核负压进风小窗射流能否传导到舍宽中线。
pub fn jet_axis_velocity(window_area_m2: f64, v0_m_per_s: f64, x_m: f64) -> Option<f64> {
    if window_area_m2 <= 0.0 || v0_m_per_s <= 0.0 || x_m < 0.0 {
        return None;
    }
    let d0 = (4.0 * window_area_m2 / std::f64::consts::PI).sqrt();
    Some(v0_m_per_s * 0.48 / (0.076 * x_m / d0 + 0.147))
}

/// 射流满足「中线最低掺混风速 v_min」所需的最小洞口风速（反解上式）
pub fn required_jet_speed(window_area_m2: f64, x_target_m: f64, v_min: f64) -> Option<f64> {
    let d0 = (4.0 * window_area_m2 / std::f64::consts::PI).sqrt();
    if d0 <= 0.0 || v_min <= 0.0 {
        return None;
    }
    Some(v_min * (0.076 * x_target_m / d0 + 0.147) / 0.48)
}

/// 小窗洞口风速：保证射流抵达舍宽中线时轴线风速仍为 **0.6 m/s**
///
/// 由 `v₀ · k(x) = 0.6` 反解 `v₀ = 0.6 / k`，
/// 其中 `k(x) = 0.48/(0.076·x/d₀ + 0.147)`，d₀ = √(4A/π)。
pub fn reynolds_window_speed(window_area_m2: f64, x_mid_m: f64) -> Option<f64> {
    const V_CENTER: f64 = 0.6; // 舍中最低轴线风速 [m/s]
    let d0 = (4.0 * window_area_m2 / std::f64::consts::PI).sqrt();
    if d0 <= 0.0 || x_mid_m <= 0.0 {
        return None;
    }
    let k = 0.48 / (0.076 * x_mid_m / d0 + 0.147);
    if k <= 0.0 {
        return None;
    }
    Some(V_CENTER / k)
}

/// 风机空气动力学特性曲线（厂商 P-Q 样本点，静压 Pa 升序 → 风量 m³/h），
/// **分段线性插值**——用于工作点衰减校核：系统阻力下真实风量低于
/// 额定（−20 Pa）点，风机效率随之变为可变值。端点外取端值不外推。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct AeroCurve(pub &'static [(f64, f64)]);

impl AeroCurve {
    /// 静压 dp [Pa] 下的风量 [m³/h]（分段线性；越界取端值，截 ≥ 0）
    pub fn q_at(&self, dp: f64) -> f64 {
        let pts = self.0;
        if pts.is_empty() {
            return 0.0;
        }
        if dp <= pts[0].0 {
            return pts[0].1.max(0.0);
        }
        for w in pts.windows(2) {
            let [(x0, y0), (x1, y1)] = [w[0], w[1]];
            if dp <= x1 {
                let t = (dp - x0) / (x1 - x0);
                return (y0 + t * (y1 - y0)).max(0.0);
            }
        }
        pts[pts.len() - 1].1.max(0.0)
    }
    /// 可变效率：系统阻力工作点相对额定基准点（默认 −20 Pa）的风量衰减系数
    pub fn eta_at(&self, dp_sys: f64, dp_ref: f64) -> f64 {
        let qr = self.q_at(dp_ref);
        if qr > 0.0 {
            (self.q_at(dp_sys) / qr).clamp(0.3, 1.2)
        } else {
            1.0
        }
    }
}

/// 50″ 蝴蝶拢风筒 1.1 kW 样本（0–50 Pa 六点）
static CURVE_50_BFLY: &[(f64, f64)] = &[
    (0.0, 43900.0),
    (10.0, 42400.0),
    (20.0, 40500.0),
    (30.0, 38400.0),
    (40.0, 36400.0),
    (50.0, 35400.0),
];
/// 51″ 蝴蝶拢风筒 1.5 kW 样本（0–100 Pa 九点）
static CURVE_51_BFLY: &[(f64, f64)] = &[
    (0.0, 49519.0),
    (12.0, 47657.0),
    (25.0, 45537.0),
    (37.0, 43400.0),
    (50.0, 40843.0),
    (62.0, 36790.0),
    (75.0, 34039.0),
    (87.0, 29327.0),
    (100.0, 25708.0),
];
/// EuroCave® EC50 50″ 1.5 HP 样本（0/25/50 Pa 三点）
static CURVE_EC50: &[(f64, f64)] = &[(0.0, 42400.0), (25.0, 38400.0), (50.0, 33800.0)];
/// 推拉风机 1380-1.1KW（1270 叶片，1380×450×1380）样本（0–100 Pa 九点）
static CURVE_TT11: &[(f64, f64)] = &[
    (0.0, 39792.0),
    (12.0, 38105.0),
    (25.0, 36060.0),
    (37.0, 34134.0),
    (50.0, 32213.0),
    (62.0, 30033.0),
    (75.0, 27230.0),
    (87.0, 23354.0),
    (100.0, 19901.0),
];
/// 推拉风机 1380-1.5KW 样本（0–100 Pa 九点）
static CURVE_TT15: &[(f64, f64)] = &[
    (0.0, 45584.0),
    (12.0, 43903.0),
    (25.0, 42029.0),
    (37.0, 39894.0),
    (50.0, 36193.0),
    (62.0, 34509.0),
    (75.0, 31878.0),
    (87.0, 28901.0),
    (100.0, 24109.0),
];

/// 已知气动曲线的机型（型号子串匹配）；未命中 → 维持固定效率口径。
/// 匹配按行序取首个命中——拢风筒行在前，防止「1380-1.1KW」子串误伤。
pub fn aero_curve_for(model: &str) -> Option<AeroCurve> {
    const TAB: &[(&str, &[&str], AeroCurve)] = &[
        (
            "51蝴蝶拢风筒 1.5kW",
            &["51蝴蝶", "1290", "拢风筒风机 1380-1.5KW", "拢风筒 1380-1.5"],
            AeroCurve(CURVE_51_BFLY),
        ),
        (
            "拢风筒 50″ 蝴蝶 1.1kW",
            &["拢分筒", "拢风筒", "50蝴蝶"],
            AeroCurve(CURVE_50_BFLY),
        ),
        (
            "推拉风机 1380-1.1KW",
            &["推拉风机 1380-1.1KW", "推拉风机1380-1.1KW", "1380-1.1KW"],
            AeroCurve(CURVE_TT11),
        ),
        (
            "推拉风机 1380-1.5KW",
            &["推拉风机 1380-1.5KW", "推拉风机1380-1.5KW", "1380-1.5KW"],
            AeroCurve(CURVE_TT15),
        ),
        ("EC50 50″ 1.5HP", &["EC50", "1.5HP", "1.5Hp", "1.5 HP", "EuroCave"], AeroCurve(CURVE_EC50)),
    ];
    TAB.iter()
        .find(|(_, keys, _)| keys.iter().any(|k| model.contains(k)))
        .map(|(_, _, c)| *c)
}

/// 通风/小窗综合计算结果（对齐原程序 HXZDTF/textBox_Fs/textBox_GXC/CalAmend）
#[derive(Debug, Clone, Copy)]
pub struct VentSizing {
    /// 最低小窗数量 [个]（C# `MinWindowsVol`：WindowQuantity(最低通风量, 窗大小)，风速固定 0.8）
    pub windows_min: f64,
    /// 修正（CO₂ 2000PPM）小窗数量 [个]（C# `Amend`：WindowQuantity(2000PPM风量, 窗大小, 用户风速)）
    pub windows_amended: f64,
    /// 最优小窗数量 [个]（C# `WindowZYQuantity`：1000PPM 风量按单台风机风量取整对齐后折算，向下取偶）
    pub windows_best: f64,
    /// 小窗计算总数 [个]（= 最优小窗数，配置可在 ±10 浮动）
    pub windows_total: f64,
    /// 计算风机数 [台]
    pub fans_required: f64,
}

/// 通风设备综合计算
///
/// * `min_ventilation`    —— 最低通风量 [m³/h]
/// * `co2_flow_2000`      —— CO₂ 2000PPM 稀释通风量 [m³/h]（空间修正小窗用）
/// * `co2_flow_best`      —— 最优小窗用通风量 [m³/h]（当前同为 2000PPM 风量，对齐风机档位）
/// * `house_cool_load_kw` —— 房屋冷负荷 [kW]
pub fn size_ventilation(
    input: &VentInput,
    min_ventilation: f64,
    co2_flow_2000: f64,
    co2_flow_best: f64,
    house_cool_load_kw: f64,
    house_length_m: f64,
    window_width_m: f64,
) -> Result<VentSizing, HeatError> {
    // 最低小窗：夜间最小通风口径 1.0 m/s（审查 G1——3 m/s 口径恒为三数最小、
    // 无下限作用；1 m/s 令其为真正的小窗数量 floor）
    let windows_min = window_quantity(min_ventilation, input.window_area, 1.0)?;
    // C# 470: WindowQuantity(CO2TFXPPM(2000), WindowsSize, Fs) —— 用户风速
    let windows_amended = window_quantity(co2_flow_2000, input.window_area, input.window_speed)?;
    // 最优小窗：2000PPM 风量对齐风机档位（WindowZYQuantity 口径）
    let windows_best = window_optimal_quantity(
        windows_min,
        co2_flow_best,
        input.window_area,
        input.window_speed,
        input.fan_unit_air_volume * input.fan_efficiency,
    )?;
    // 计算小窗总数 = 空间修正×0.2 + 最优×0.9（权重和 1.1 自带 10% 冗余），向上取偶；
    // 且不得低于间距 2.0 m 上限对应的最少布窗数（长墙两侧均布）
    let weighted = windows_amended * 0.2 + windows_best * 0.9;
    let windows_total = {
        let by_weight = (weighted / 2.0).ceil() * 2.0;
        let by_spacing = (house_length_m / (window_width_m + 2.0)).ceil() * 2.0;
        by_weight.max(by_spacing)
    };

    let fans_required = crate::cool::fan_quantity(
        house_cool_load_kw,
        input.fan_unit_air_volume,
        input.fan_efficiency,
        input.fans_spare,
    );

    Ok(VentSizing {
        windows_min,
        windows_amended,
        windows_best,
        windows_total,
        fans_required,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 山墙容量：18 m 宽 × 5.2 m 檐口 → 10 列 × 3 行 = 30 台
    #[test]
    fn gable_capacity_reference() {
        let mut h = crate::data::HouseInput::default();
        h.width = 18.0;
        h.eave_height = 5.2;
        assert_eq!(gable_fan_capacity(&h), 30.0);
        h.width = 16.5;
        assert_eq!(gable_fan_capacity(&h), 27.0);
    }

    #[test]
    fn window_quantity_reference() {
        // 最低通风量 43570 m³/h、单窗 0.1512 m²、风速 1 m/s
        // 43570/(0.1512·1·3600) = 80.13 → 81
        let n = window_quantity(43_570.0, 0.1512, 1.0).unwrap();
        assert!((n - 81.0).abs() < 1e-9, "n={n}");
    }

    #[test]
    fn window_quantity_rejects_zero_area() {
        assert_eq!(
            window_quantity(100.0, 0.0, 1.0),
            Err(HeatError::NonPositiveCapacity)
        );
    }

    /// 工作点衰减校核：样本节点精确复现、额定 −20 Pa 插值与可变效率
    #[test]
    fn aero_curve_interpolates_sample_points() {
        let c = super::aero_curve_for("推拉风机 EC50-1.5HP").expect("curve");
        assert!((c.q_at(0.0) - 42400.0).abs() < 1.0);
        assert!((c.q_at(25.0) - 38400.0).abs() < 1.0);
        assert!((c.q_at(50.0) - 33800.0).abs() < 1.0);
        // −20 Pa 额定基准插值 ≈39,200 m³/h
        let q20 = c.q_at(20.0);
        assert!(q20 > 38900.0 && q20 < 39600.0, "q20 = {q20}");
        // 系统静压 24.5 Pa（湿帘 23 + 洞口动压 1.5）→ 可变效率略低于 1
        let eta = c.eta_at(24.5, 20.0);
        assert!(eta > 0.90 && eta < 1.0, "eta = {eta}");
        // 未知机型 → None（维持固定效率口径）
        assert!(super::aero_curve_for("试验定频风机 X-9").is_none());
    }

    /// 推拉风机九点样本：−20 Pa 插值 ≈36,847（1.1KW）/ 42,750（1.5KW），
    /// 拢风筒行优先命中防止 1380-1.1KW 子串误伤
    #[test]
    fn push_pull_fan_curves() {
        let c11 = super::aero_curve_for("推拉风机 1380-1.1KW").expect("tt11");
        assert!((c11.q_at(0.0) - 39792.0).abs() < 1.0);
        assert!((c11.q_at(100.0) - 19901.0).abs() < 1.0);
        let q20 = c11.q_at(20.0);
        assert!((q20 - 36847.0).abs() < 5.0, "q20 = {q20}");
        // 系统静压 26 Pa → η ≈ 0.97
        let eta = c11.eta_at(26.0, 20.0);
        assert!((eta - 0.973).abs() < 0.01, "eta = {eta}");
        let c15 = super::aero_curve_for("推拉风机 1380-1.5KW").expect("tt15");
        assert!((c15.q_at(20.0) - 42750.0).abs() < 5.0);
        // 拢分筒优先于推拉子串；1.5KW 拢风筒命中 51″ 曲线
        let cb = super::aero_curve_for("拢风筒风机 1380-1.1KW").expect("bell");
        assert!((cb.q_at(20.0) - 40500.0).abs() < 1.0);
        let cb15 = super::aero_curve_for("拢风筒风机 1380-1.5KW").expect("bell15");
        assert!((cb15.q_at(0.0) - 49519.0).abs() < 1.0);
        assert!((cb15.q_at(20.0) - 46352.0).abs() < 5.0);
        assert!((cb15.q_at(100.0) - 25708.0).abs() < 1.0);
    }

    /// 50″ 蝴蝶拢风筒六点样本：节点精确、−20 Pa=40500、系统 26 Pa 衰减 ≈3%
    #[test]
    fn bell_mouth_50_curve() {
        let c = super::aero_curve_for("拢风筒风机 1380-1.1KW").expect("curve");
        for (dp, q) in [(0.0, 43900.0), (10.0, 42400.0), (20.0, 40500.0), (30.0, 38400.0), (40.0, 36400.0), (50.0, 35400.0)] {
            assert!((c.q_at(dp) - q).abs() < 1.0, "dp={dp}");
        }
        // 系统静压 26 Pa（湿帘 23 + 动压 1.5）→ 39,240 m³/h、η ≈ 0.969
        let q26 = c.q_at(26.0);
        assert!((q26 - 39240.0).abs() < 5.0, "q26 = {q26}");
        let eta = c.eta_at(26.0, 20.0);
        assert!((eta - 0.969).abs() < 0.01, "eta = {eta}");
        // 端点外取端值（55 Pa → 35400，不外推负斜率）
        assert!((c.q_at(55.0) - 35400.0).abs() < 1.0);
    }

    #[test]
    fn window_optimal_quantity_is_even() {
        let n = window_optimal_quantity(126.0, 236.0 * 43_570.0 * 0.8, 0.1512, 1.0, 43_570.0 * 0.8)
            .unwrap();
        assert_eq!(n % 2.0, 0.0, "应为偶数: {n}");
        assert!(n >= 126.0, "n={n}");
    }

    #[test]
    fn window_optimal_quantity_floor_at_min() {
        // 单窗 10 m² 时计算值极小 → 取最小值 42（偶数向上取整不变）
        let n = window_optimal_quantity(42.0, 1.0, 10.0, 1.0, 43_570.0).unwrap();
        assert_eq!(n, 42.0, "n={n}");
    }

    #[test]
    fn optimal_window_speed_reference() {
        // 舍宽 18、檐口 7.5、rch 0.8、指数 1 → 0.8 + 18·0.8/25.5 = 1.3647 → 1.365
        let v = optimal_window_speed(18.0, 7.5, 0.8, 1.0).unwrap();
        assert!((v - 1.365).abs() < 1e-3, "v={v}");
        // 原 C# 输出 1.365（截图显示 1.365）
    }

    #[test]
    fn optimal_window_speed_zero_denom() {
        assert_eq!(
            optimal_window_speed(0.0, 0.0, 0.8, 1.0),
            Err(HeatError::NonPositiveCapacity)
        );
    }

    #[test]
    fn jet_speed_round_trip() {
        // 反解所需最小风速代回正向公式，恰得最低掺混风速 0.25
        let v_req = required_jet_speed(0.1512, 9.0, 0.25).unwrap();
        let back = jet_axis_velocity(0.1512, v_req, 9.0).unwrap();
        assert!((back - 0.25).abs() < 1e-6, "back={back}");
        // 雷诺射流区间 2.2~3 时中线轴速必须高于 0.25
        for v0 in [2.2, 2.5, 3.0] {
            assert!(jet_axis_velocity(0.1512, v0, 9.0).unwrap() > 0.25);
        }
    }

    #[test]
    fn reynolds_speed_rule() {
        // 洞口风速按「舍中轴线 0.6 m/s」反解：代回衰减公式中线轴速恒为 0.6
        for (area, half_w) in [(0.1512_f64, 9.0_f64), (0.11, 7.0), (0.16, 12.0)] {
            let v0 = reynolds_window_speed(area, half_w).unwrap();
            let v_axis = jet_axis_velocity(area, v0, half_w).unwrap();
            assert!((v_axis - 0.6).abs() < 1e-9, "v0={v0} axis={v_axis}");
            assert!(v0 > 1.0 && v0 < 4.0, "v0={v0}");
        }
    }

    #[test]
    fn size_ventilation_smoke() {
        let input = VentInput::default();
        // 长 92 m、560 窗：间距 2.0 m 最少数 = ⌈92/2.56⌉×2 = 74
        let s = size_ventilation(&input, 43_570.0, 87_140.0, 87_140.0, 400.0, 92.0, 0.56).unwrap();
        // 最低小窗：3 m/s 折算 → ceil(43570/(0.1512·3·3600)) = 27
        assert_eq!(s.windows_min, 81.0); // 1 m/s 夜间最小口径（原 3 m/s → 27，审查 G1）
                                         // 空间修正：默认洞口风速 2.2 → ceil(87140/(0.1512·2.2·3600)) = 73
        assert_eq!(s.windows_amended, 73.0);
        // 总数 = max(修正×0.2+最优×0.9 取偶, 间距最少数)：max(88, 间距 90) = 90
        assert!(
            (s.windows_total - 90.0).abs() < 1e-9,
            "total={} am={} best={} min={}",
            s.windows_total,
            s.windows_amended,
            s.windows_best,
            s.windows_min
        );
        assert!(s.windows_total >= s.windows_amended);
        // 最优：2000PPM 按 29120 档对齐 ⌈87140/29120⌉=3 档 = 87360 → /(0.1512·2.2·3600)=72.9
        // → 取偶 74，但低于最低口径 min=81 → 抬到 81 再取偶 = 82
        assert_eq!(s.windows_best, 82.0);
        assert!(s.fans_required >= 1.0);
        // 小通风需求（蛋鸭例，弧面窗有效面积旧值 0.1319）：
        // 修正 ⌈51086/(0.1319·2.2·3600)⌉=49；最优 ⌈51086/29120⌉=2 档=58240 → 55.9 → 56；
        // 加权 0.2×49+0.9×56=60.2 → 62；间距 ⌈92/2.97⌉×2=64 的 ⌈⌋口径实得 60 → 62
        let mut lay = VentInput::default();
        lay.window_area = 0.1319;
        let s2 = size_ventilation(&lay, 25_543.0, 51_086.0, 51_086.0, 130.0, 92.0, 0.97).unwrap();
        assert!(
            (s2.windows_total - 62.0).abs() < 1e-9,
            "s2 total={} am={} best={}",
            s2.windows_total,
            s2.windows_amended,
            s2.windows_best
        );
        assert!(s2.windows_total >= s2.windows_amended);
    }
}
