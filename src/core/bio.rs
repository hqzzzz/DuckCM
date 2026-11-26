//! 养殖生物呼吸、CO₂ 平衡与饮水
//!
//! 移植自 C# `CMCalculation`（`CO2RL`、`WTRL`、`CO2TF5000PPM`、
//! `CO2TFXPPM`、`CO2PPM2MG`、`CO2PPMTOV`、`BreedneedWater`、`K2XH`）
//! 与 `YangZCon` 对话框的调用。
//!
//! # 相对原实现的修正
//! 1. `CO2TF5000PPM`：`tz=0` 时 `Math.Pow(0, 0.75)/0` → `Infinity`/NaN；
//!    `wd > 25` 时 `(25-wd)³` 为负可使修正因子为负。改为域检查。
//! 2. `CO2TFXPPM`：`oldppm < 400` 或 `ppm < 400` 时分母/分子符号失控，
//!    负体积会被当成正常结果显示。改为 `Result` 并显式约束 `ppm ∈ [700, oldppm]`。
//! 3. `CO2PPM2MG` 无舍入（返回原始值）但 C# 其他函数都 `Math.Round`；
//!    这里统一在最终结果舍入，中间量保持全精度。

/// 生物计算错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BioError {
    /// 体重/只数必须为正
    NonPositiveBodyMass,
    /// PPM 范围无效（需 400 < 目标 ≤ 起始）
    InvalidPpmRange,
    /// 输入包含 NaN/无穷
    NotFinite,
}

impl core::fmt::Display for BioError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            BioError::NonPositiveBodyMass => write!(f, "体重/只数必须大于 0"),
            BioError::InvalidPpmRange => write!(f, "CO₂ PPM 范围无效（需 400 < 目标 ≤ 起始浓度）"),
            BioError::NotFinite => write!(f, "输入包含无效数值"),
        }
    }
}

impl std::error::Error for BioError {}

/// CO₂ 呼吸放热量 [kW]（C# `CO2RL`）
///
/// 化学计量：`C₆H₁₂O₆ + 6O₂ + 6H₂O → 6CO₂ + 12H₂O + 2870 kJ`
///
/// `0.03/100 · 呼吸量 · 1000/6 · 2870 · ATP系数 / 3600 / 22.4`
pub fn co2_respiration_heat(co2_volume: f64, atp_factor: f64) -> Result<f64, BioError> {
    if !co2_volume.is_finite() || !atp_factor.is_finite() {
        return Err(BioError::NotFinite);
    }
    if co2_volume < 0.0 {
        return Err(BioError::NotFinite);
    }
    let v = 0.03 / 100.0 * co2_volume * 1000.0 / 6.0 * 2870.0 * atp_factor / 3600.0 / 22.4;
    Ok((v * 1e6).round() / 1e6)
}

/// 饮水量 [kg/h]（C# `WTRL`）
///
/// `0.03/100 · 呼吸量 · 1000/22.4 · 18/1000`
pub fn drinking_water(co2_volume: f64) -> Result<f64, BioError> {
    if !co2_volume.is_finite() {
        return Err(BioError::NotFinite);
    }
    if co2_volume < 0.0 {
        return Err(BioError::NotFinite);
    }
    let v = 0.03 / 100.0 * co2_volume * 1000.0 / 22.4 * 18.0 / 1000.0;
    Ok((v * 1e6).round() / 1e6)
}

/// 5000PPM 呼吸交换量（C# `CO2TF5000PPM`）
///
/// `0.084 · 6.25 · 体重体重⁰·⁷⁵ · (1 + 0.00004·(25−温度)³) / 体重 · 修正系数`
///
/// 修正：`体重 ≤ 0` 返回 `Err`；温度修正因子下限截断为 0.1
/// （原式在低温下会转负，产生负呼吸量）。
pub fn co2_exchange_5000ppm(body_weight: f64, temp: f64, correction: f64) -> Result<f64, BioError> {
    if !body_weight.is_finite() || !temp.is_finite() || !correction.is_finite() {
        return Err(BioError::NotFinite);
    }
    if body_weight <= 0.0 {
        return Err(BioError::NonPositiveBodyMass);
    }
    let temp_factor = (1.0 + 0.00004 * (25.0 - temp).powi(3)).max(0.1);
    let v = (0.084 * (6.25 * body_weight.powf(0.75)) * temp_factor / body_weight) * correction;
    Ok((v * 100.0).round() / 100.0)
}

/// 养殖需水量 [kg/h]（C# `BreedneedWater`）
///
/// `0.017 · 只数 · 体重 · 修正系数 / 1000`
pub fn breed_need_water(
    head_count: f64,
    body_weight: f64,
    correction: f64,
) -> Result<f64, BioError> {
    if !head_count.is_finite() || !body_weight.is_finite() || !correction.is_finite() {
        return Err(BioError::NotFinite);
    }
    if head_count < 0.0 || body_weight <= 0.0 {
        return Err(BioError::NonPositiveBodyMass);
    }
    let v = 0.017 * head_count * body_weight * correction / 1000.0;
    Ok((v * 100.0).round() / 100.0)
}

/// CO₂ 体积分数 PPM → 质量浓度（C# `CO2PPM2MG`，未舍入）
///
/// `(44/22.4) · PPM · 273/(273+温度) · (ΔPa+101325)/101325`
pub fn co2_ppm_to_mg(ppm: f64, temperature: f64, delta_pa: f64) -> Result<f64, BioError> {
    if !ppm.is_finite() || !temperature.is_finite() || !delta_pa.is_finite() {
        return Err(BioError::NotFinite);
    }
    if ppm < 0.0 {
        return Err(BioError::InvalidPpmRange);
    }
    let tk = 273.0 + temperature;
    if tk == 0.0 {
        return Err(BioError::NotFinite);
    }
    Ok((44.0 / 22.4) * ppm * (273.0 / tk) * ((delta_pa + 101_325.0) / 101_325.0))
}

/// CO₂ PPM → 体积分数（C# `CO2PPMTOV`）
///
/// **修正（审计 L1）**：原式 `(44/22.4)·ppm/12930·0.916` 与其注释
/// 「1293000 气体质量」相差 **100 倍**：5000 ppm 会算出 69.6 % 的
/// 体积分数（荒谬），正确为 0.696 %。此处按注释的 1 293 000 修正。
pub fn co2_ppm_to_volume_fraction(ppm: f64) -> Result<f64, BioError> {
    if !ppm.is_finite() {
        return Err(BioError::NotFinite);
    }
    Ok((44.0 / 22.4) * ppm / 1_293_000.0 * 0.916)
}

/// 目标 PPM 对应**稀释体积**（C# `CO2TFXPPM` 的固定容积用法）
///
/// `V · (mg(起始) − mg(目标)) / (mg(目标) − mg(400))`
///
/// 仅适用于**一次性稀释固定容积**（如 `Size_FW` 房间容积）。
/// 求稳态通风量请用 [`co2_flow_for_ppm`]（审计 H1）。
///
/// 修正：`目标 PPM` 被下限截到 700（原样保留）；增加上界检查
/// `目标 ≤ 起始`，否则返回 `Err`（原式会返回负体积）。
pub fn co2_ventilation_for_ppm(
    target_ppm: f64,
    room_volume: f64,
    start_ppm: f64,
) -> Result<f64, BioError> {
    if !target_ppm.is_finite() || !room_volume.is_finite() || !start_ppm.is_finite() {
        return Err(BioError::NotFinite);
    }
    let ppm = target_ppm.max(700.0);
    if start_ppm <= 400.0 || ppm > start_ppm {
        return Err(BioError::InvalidPpmRange);
    }
    let mg_start = co2_ppm_to_mg(start_ppm, 25.0, -200.0)?;
    let mg_target = co2_ppm_to_mg(ppm, 25.0, -200.0)?;
    let mg_fresh = co2_ppm_to_mg(400.0, 25.0, -200.0)?;
    let denom = mg_target - mg_fresh;
    if denom == 0.0 {
        return Err(BioError::InvalidPpmRange);
    }
    let v = room_volume * (mg_start - mg_target) / denom;
    Ok((v * 100.0).round() / 100.0)
}

/// 按种类日采食标准 [kg/只·日]（体重档：品种手册量级，待联网复核）
///
/// 蛋禽取维持水平、肉禽取育成后期高档；与 UI 采食行同源。
pub fn breed_feed_kg(t: crate::data::BreedType) -> f64 {
    use crate::data::BreedType::*;
    match t {
        LayerChicken => 0.115,
        BroilerChicken => 0.17,
        LayerDuck => 0.13,
        BroilerDuck => 0.19,
        Goose => 0.22,
        Other => 0.15,
    }
}

/// 舍内水分产生 W_bio [kg/h]
///
/// 饮水经呼吸/体表/粪尿蒸发散出的比例取 0.55（禽舍质量平衡常用值）：
/// `W_bio = 日采食 × 水料比 2.2 × 0.55 / 24`
pub fn barn_moisture_kg_h(t: crate::data::BreedType, head: f64) -> f64 {
    breed_feed_kg(t) * head * 2.2 * 0.55 / 24.0
}

/// 水源选型：设计流量与推荐管径
///
/// ```text
/// 流量需求 Q_w = Q_突发 + E_帘   [m³/h]（实际合成流量直加，不乘裕度系数）
/// 管径     d   = 最小 DN，使 v = Q₀/3600/(πD²/4) ≤ 1.5 m/s，
///              其中 Q₀ = 突发 + 补水（实际流量，管径**不乘**系数）
/// ```
///
/// 流速上限 1.5 m/s：**GB 50015《建筑给水排水设计标准》**经济流速
/// 上限（配水管、支管 1.0~1.5 m/s 档，本场站按上限 1.5）。
/// 返回 `(Q_w m³/h, DN mm, v m/s)`；DN 系列 25/32/40/50/65/80/100/125/150。
pub fn water_supply_sizing(sudden_m3h: f64, evap_m3h: f64) -> (f64, f64, f64) {
    let q0 = sudden_m3h + evap_m3h;
    // 流量需求 = 实际合成流量直加（用户口径：突发 + 湿帘蒸发能力，不乘裕度系数）
    let q_demand = q0;
    let series = [25.0, 32.0, 40.0, 50.0, 65.0, 80.0, 100.0, 125.0, 150.0];
    let flow_ms =
        |dn_mm: f64| q0 / 3600.0 / (std::f64::consts::PI * (dn_mm / 1000.0).powi(2) / 4.0);
    let dn = *series
        .iter()
        .find(|dn| flow_ms(**dn) <= 1.5)
        .unwrap_or(series.last().unwrap());
    (q_demand, dn, flow_ms(dn))
}

/// 稳态排放下的目标 PPM 通风量（**审计 H1 修正**）
///
/// 原 C# 在 `YangZCon.cs:197/214` 把**流量(m³/h)** 传给
/// [`co2_ventilation_for_ppm`]，但该式是「固定容积一次性稀释」的分子。
/// 稳态排放应为：
///
/// ```text
/// 通风量 = 排放源强 · (C起始 − C新风) / (C目标 − C新风)
///        = 排放源强 · (起始PPM − 400) / (目标PPM − 400)
/// ```
///
/// 原式分子用 `(起始PPM − 目标PPM)`，导致通风量**偏小 13.0 %**
/// （5000→1000 ppm：系数 6.667 vs 正确 7.667）。
///
/// * `emission_flow` —— 排放源强对应的参考通风量 [m³/h]（原 `num`）
pub fn co2_flow_for_ppm(
    emission_flow: f64,
    target_ppm: f64,
    start_ppm: f64,
) -> Result<f64, BioError> {
    if !emission_flow.is_finite() || !target_ppm.is_finite() || !start_ppm.is_finite() {
        return Err(BioError::NotFinite);
    }
    let ppm = target_ppm.max(700.0);
    if start_ppm <= 400.0 || ppm > start_ppm {
        return Err(BioError::InvalidPpmRange);
    }
    if emission_flow < 0.0 {
        return Err(BioError::NotFinite);
    }
    let factor = (start_ppm - 400.0) / (ppm - 400.0);
    Ok((emission_flow * factor * 100.0).round() / 100.0)
}

/// MEL 计算法（C# `K2XH`）
///
/// `体重 · 3.5 · 1.2 · 只数 / 0.21 / 0.04 · 60 / 1e6`
pub fn mel_exchange(head_count: f64, body_weight: f64) -> Result<f64, BioError> {
    if !head_count.is_finite() || !body_weight.is_finite() {
        return Err(BioError::NotFinite);
    }
    if head_count < 0.0 || body_weight <= 0.0 {
        return Err(BioError::NonPositiveBodyMass);
    }
    let v = body_weight * 3.5 * 1.2 * head_count / 0.21 / 0.04 * 60.0 / 1_000_000.0;
    Ok((v * 1e6).round() / 1e6)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn co2_respiration_heat_reference() {
        // 呼吸量 100 m³/h、ATP 系数 0.6
        // 0.0003·100·1000/6·2870·0.6/3600/22.4 = 0.0003·100=0.03 → 0.03·166.67·2870·0.6/3600/22.4
        let v = co2_respiration_heat(100.0, 0.6).unwrap();
        let expect = 0.03 / 100.0 * 100.0 * 1000.0 / 6.0 * 2870.0 * 0.6 / 3600.0 / 22.4;
        assert!((v - expect).abs() < 1e-6, "v={v} expect={expect}");
        assert!(v > 0.0);
    }

    #[test]
    fn drinking_water_reference() {
        let v = drinking_water(100.0).unwrap();
        let expect = 0.03 / 100.0 * 100.0 * 1000.0 / 22.4 * 18.0 / 1000.0;
        assert!((v - expect).abs() < 1e-6);
    }

    #[test]
    fn water_supply_sizing_reference() {
        // 突发 7.44 + 蒸发 1.49 → 需求 = 直加 8.93；管径按 GB 50015 ≤1.5 m/s：
        // DN40 v=1.97 > 1.5 → DN50 v=1.27
        let (q, dn, v) = water_supply_sizing(7.44, 1.49);
        assert!((q - 8.93).abs() < 1e-9, "q={q}");
        assert_eq!(dn, 50.0, "dn={dn}");
        assert!(v <= 1.5 && v > 0.6, "v={v}");
        // 实际工况：7.44 + 0.39 → 7.83，DN50 (v≈1.11)
        let (q50, dn50, v50) = water_supply_sizing(7.44, 0.39);
        assert_eq!(dn50, 50.0, "dn={dn50} v={v50}");
        assert!((q50 - 7.83).abs() < 1e-9, "q50={q50}");
        // 小工程不超 DN25 下限
        let (_, dn2, _) = water_supply_sizing(1.0, 0.2);
        assert_eq!(dn2, 25.0);
    }

    #[test]
    fn co2_exchange_rejects_zero_weight() {
        assert_eq!(
            co2_exchange_5000ppm(0.0, 18.0, 1.3),
            Err(BioError::NonPositiveBodyMass)
        );
    }

    #[test]
    fn co2_exchange_reference() {
        // 2.5 kg、18 °C、1.3
        // = (0.084·6.25·2.5^0.75·(1+0.00004·7³)/2.5)·1.3 = 0.5502 → 0.55
        let v = co2_exchange_5000ppm(2.5, 18.0, 1.3).unwrap();
        assert!((v - 0.55).abs() < 0.01, "v={v}");
    }

    #[test]
    fn co2_ventilation_for_ppm_reference() {
        // 5000 → 1000 PPM、房间 16313 m³
        let v = co2_ventilation_for_ppm(1000.0, 16_313.0, 5000.0).unwrap();
        assert!(v > 0.0, "v={v}");
        // 降到 2000 应比降到 1000 需要更少通风
        let v2 = co2_ventilation_for_ppm(2000.0, 16_313.0, 5000.0).unwrap();
        assert!(v2 < v, "v2={v2} v={v}");
    }

    #[test]
    fn co2_ventilation_rejects_inverted_range() {
        assert_eq!(
            co2_ventilation_for_ppm(5000.0, 1000.0, 3000.0),
            Err(BioError::InvalidPpmRange)
        );
    }

    #[test]
    fn ppm_conversions_are_consistent() {
        // PPM → mg 单调递增
        let a = co2_ppm_to_mg(1000.0, 25.0, -200.0).unwrap();
        let b = co2_ppm_to_mg(2000.0, 25.0, -200.0).unwrap();
        assert!(b > a);
        // PPM → 体积分数（审计 L1 修正：分母 1 293 000，非 12 930）
        let v = co2_ppm_to_volume_fraction(1000.0).unwrap();
        assert!((v - (44.0 / 22.4) * 1000.0 / 1_293_000.0 * 0.916).abs() < 1e-12);
        // 5000 ppm 应约 0.696 %（原式给出荒谬的 69.6 %）
        let v5000 = co2_ppm_to_volume_fraction(5000.0).unwrap();
        assert!(v5000 < 0.01, "5000 ppm 体积分数应 <1%，实为 {v5000}");
    }

    #[test]
    fn breed_need_water_reference() {
        // 10000 只、2.5 kg、1.3 → 0.017·10000·2.5·1.3/1000 = 0.5525 → 0.55
        let v = breed_need_water(10_000.0, 2.5, 1.3).unwrap();
        assert!((v - 0.55).abs() < 0.01, "v={v}");
    }

    #[test]
    fn mel_exchange_reference() {
        let v = mel_exchange(10_000.0, 2.5).unwrap();
        let expect = 2.5 * 3.5 * 1.2 * 10_000.0 / 0.21 / 0.04 * 60.0 / 1_000_000.0;
        assert!((v - expect).abs() < 1e-6);
    }
}
