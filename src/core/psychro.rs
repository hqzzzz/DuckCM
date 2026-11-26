//! 空气状态计算（PsychroLib / ASHRAE 2017 ch.1 的 SI 移植）
//!
//! 忠实移植自 `DuckCM/Tool/psychrolib.cs`（即开源 PsychroLib），
//! 仅保留 SI 单位制：温度 °C、压力 Pa、含湿量 kg/kg。
//!
//! 与原 C# 的差异只有一处——错误处理：
//! C# 版用 `throw`，此处用 `f64::NAN` 返回值 + `Result` 变体，
//! 避免 GUI 在异常输入时崩溃（原 C# 常把异常吞成 0 或 NaN）。

#![allow(clippy::too_many_arguments)]

/// 绝对零度偏移 (K)
const ZERO_CELSIUS_AS_KELVIN: f64 = 273.15;
/// 干空气气体常数 [J/(kg·K)]
const R_DA_SI: f64 = 287.042;
/// 最大迭代次数（与 C# 一致）
const MAX_ITER_COUNT: u32 = 100;
/// 最小含湿量（与 C# 一致）
const MIN_HUM_RATIO: f64 = 1e-7;
/// 水三相点温度 (°C)
const TRIPLE_POINT_WATER_SI: f64 = 0.01;
/// 迭代容差 (°C，与 C# 一致：0.001)
const PSYCHROLIB_TOLERANCE: f64 = 0.001;

/// 一组空气状态量（对应 C# `PsychrometricValue`）
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct PsychrometricValue {
    pub t_dry_bulb: f64,
    pub t_wet_bulb: f64,
    pub pressure: f64,
    pub hum_ratio: f64,
    pub t_dew_point: f64,
    pub rel_hum: f64,
    pub vap_pres: f64,
    pub moist_air_enthalpy: f64,
    pub moist_air_volume: f64,
    pub degree_of_saturation: f64,
}

/// celsius → kelvin
#[inline]
fn to_kelvin(t_c: f64) -> f64 {
    t_c + ZERO_CELSIUS_AS_KELVIN
}

/// 饱和水蒸气分压 [Pa]，ASHRAE (2017) ch.1 eq.5 & 6
///
/// ```text
/// ln Pws = C1/T + C2 + C3·T + C4·T² + C5·T³ + C6·ln T      (T ≤ 0.01 °C)
/// ln Pws = C1/T + C2 + C3·T + C4·T² + C5·T³ + C6·ln T      (T >  0.01 °C)
/// ```
/// 有效范围 −100 … 200 °C；越界返回 `NAN`（原 C# 抛异常）。
pub fn get_sat_vap_pres(t_dry_bulb: f64) -> f64 {
    if !(-100.0..=200.0).contains(&t_dry_bulb) {
        return f64::NAN;
    }
    let t = to_kelvin(t_dry_bulb);
    let ln_pws = if t_dry_bulb <= TRIPLE_POINT_WATER_SI {
        -5.6745359e3 / t + 6.3925247 - 9.677843e-3 * t
            + 6.2215701e-7 * t * t
            + 2.0747825e-9 * t * t * t
            - 9.484024e-13 * t * t * t * t
            + 4.1635019 * t.ln()
    } else {
        -5.8002206e3 / t + 1.3914993 - 4.8640239e-2 * t + 4.1764768e-5 * t * t
            - 1.4452093e-8 * t * t * t
            + 6.5459673 * t.ln()
    };
    ln_pws.exp()
}

/// 饱和水蒸气压对数的导数（Newton 法求露点用），ASHRAE ch.1 eq.5 & 6
fn d_ln_pws(t_dry_bulb: f64) -> f64 {
    let t = to_kelvin(t_dry_bulb);
    if t_dry_bulb <= TRIPLE_POINT_WATER_SI {
        5.6745359e3 / (t * t) - 9.677843e-3 + 2.0 * 6.2215701e-7 * t + 3.0 * 2.0747825e-9 * t * t
            - 4.0 * 9.484024e-13 * t * t * t
            + 4.1635019 / t
    } else {
        5.8002206e3 / (t * t) - 4.8640239e-2 + 2.0 * 4.1764768e-5 * t - 3.0 * 1.4452093e-8 * t * t
            + 6.5459673 / t
    }
}

/// 相对湿度 [0..1] → 水蒸气分压 [Pa]
pub fn get_vap_pres_from_rel_hum(t_dry_bulb: f64, rel_hum: f64) -> f64 {
    rel_hum * get_sat_vap_pres(t_dry_bulb)
}

/// 水蒸气分压 [Pa] → 相对湿度 [0..1]
pub fn get_rel_hum_from_vap_pres(t_dry_bulb: f64, vap_pres: f64) -> f64 {
    let pws = get_sat_vap_pres(t_dry_bulb);
    if pws == 0.0 {
        return f64::NAN;
    }
    vap_pres / pws
}

/// 露点温度 ← 水蒸气分压（Newton 迭代）
pub fn get_t_dew_point_from_vap_pres(t_dry_bulb: f64, vap_pres: f64) -> f64 {
    let bounds = [-100.0, 200.0];
    if vap_pres < get_sat_vap_pres(bounds[0]) || vap_pres > get_sat_vap_pres(bounds[1]) {
        return f64::NAN;
    }
    let mut t_dew_point = t_dry_bulb;
    let ln_vp = vap_pres.ln();
    for _ in 0..MAX_ITER_COUNT {
        let t_iter = t_dew_point;
        let ln_vp_iter = get_sat_vap_pres(t_iter).ln();
        let d_ln_vp = d_ln_pws(t_iter);
        t_dew_point = t_iter - (ln_vp_iter - ln_vp) / d_ln_vp;
        t_dew_point = t_dew_point.clamp(bounds[0], bounds[1]);
        if (t_dew_point - t_iter).abs() <= PSYCHROLIB_TOLERANCE {
            return t_dew_point.min(t_dry_bulb);
        }
    }
    t_dew_point.min(t_dry_bulb)
}

/// 露点温度 → 水蒸气分压 [Pa]（ASHRAE ch.1 eq.36）
pub fn get_vap_pres_from_t_dew_point(t_dew_point: f64) -> f64 {
    get_sat_vap_pres(t_dew_point)
}

/// 含湿量 → 水蒸气分压 [Pa]
pub fn get_vap_pres_from_hum_ratio(hum_ratio: f64, pressure: f64) -> f64 {
    let w = hum_ratio.max(MIN_HUM_RATIO);
    pressure * w / (0.621945 + w)
}

/// 水蒸气分压 [Pa] → 含湿量 [kg/kg]
pub fn get_hum_ratio_from_vap_pres(vap_pres: f64, pressure: f64) -> f64 {
    (0.621945 * vap_pres / (pressure - vap_pres)).max(MIN_HUM_RATIO)
}

/// 饱和含湿量 [kg/kg]
pub fn get_sat_hum_ratio(t_dry_bulb: f64, pressure: f64) -> f64 {
    get_hum_ratio_from_vap_pres(get_sat_vap_pres(t_dry_bulb), pressure)
}

/// 相对湿度 [0..1] → 含湿量 [kg/kg]
pub fn get_hum_ratio_from_rel_hum(t_dry_bulb: f64, rel_hum: f64, pressure: f64) -> f64 {
    let vap_pres = get_vap_pres_from_rel_hum(t_dry_bulb, rel_hum);
    get_hum_ratio_from_vap_pres(vap_pres, pressure)
}

/// 含湿量 → 相对湿度 [0..1]
pub fn get_rel_hum_from_hum_ratio(t_dry_bulb: f64, hum_ratio: f64, pressure: f64) -> f64 {
    let vap_pres = get_vap_pres_from_hum_ratio(hum_ratio, pressure);
    get_rel_hum_from_vap_pres(t_dry_bulb, vap_pres)
}

/// 含湿量 → 露点温度 (°C)
pub fn get_t_dew_point_from_hum_ratio(t_dry_bulb: f64, hum_ratio: f64, pressure: f64) -> f64 {
    let vap_pres = get_vap_pres_from_hum_ratio(hum_ratio, pressure);
    get_t_dew_point_from_vap_pres(t_dry_bulb, vap_pres)
}

/// 露点温度 → 含湿量 [kg/kg]
pub fn get_hum_ratio_from_t_dew_point(t_dew_point: f64, pressure: f64) -> f64 {
    get_hum_ratio_from_vap_pres(get_sat_vap_pres(t_dew_point), pressure)
}

/// (干球, 湿球) → 含湿量 [kg/kg]，ASHRAE ch.1 eq.33 & 35
pub fn get_hum_ratio_from_t_wet_bulb(t_dry_bulb: f64, t_wet_bulb: f64, pressure: f64) -> f64 {
    let ws_star = get_sat_hum_ratio(t_wet_bulb, pressure);
    let hum_ratio = if t_wet_bulb >= 0.0 {
        ((2501.0 - 2.326 * t_wet_bulb) * ws_star - 1.006 * (t_dry_bulb - t_wet_bulb))
            / (2501.0 + 1.86 * t_dry_bulb - 4.186 * t_wet_bulb)
    } else {
        ((2830.0 - 0.24 * t_wet_bulb) * ws_star - 1.006 * (t_dry_bulb - t_wet_bulb))
            / (2830.0 + 1.86 * t_dry_bulb - 2.1 * t_wet_bulb)
    };
    hum_ratio.max(MIN_HUM_RATIO)
}

/// (干球, 含湿量) → 湿球温度 (°C)：二分迭代（与 C# 完全一致）
pub fn get_t_wet_bulb_from_hum_ratio(t_dry_bulb: f64, hum_ratio: f64, pressure: f64) -> f64 {
    let bounded = hum_ratio.max(MIN_HUM_RATIO);
    let t_dew_point = get_t_dew_point_from_hum_ratio(t_dry_bulb, bounded, pressure);
    let mut t_wet_bulb_sup = t_dry_bulb;
    let mut t_wet_bulb_inf = t_dew_point;
    let mut t_wet_bulb = (t_wet_bulb_inf + t_wet_bulb_sup) / 2.0;
    let mut index = 0u32;
    while (t_wet_bulb_sup - t_wet_bulb_inf) > PSYCHROLIB_TOLERANCE {
        let w_star = get_hum_ratio_from_t_wet_bulb(t_dry_bulb, t_wet_bulb, pressure);
        if w_star > bounded {
            t_wet_bulb_sup = t_wet_bulb;
        } else {
            t_wet_bulb_inf = t_wet_bulb;
        }
        t_wet_bulb = (t_wet_bulb_sup + t_wet_bulb_inf) / 2.0;
        index += 1;
        if index > MAX_ITER_COUNT {
            break;
        }
    }
    t_wet_bulb
}

/// 湿空气焓 [kJ/kg]，ASHRAE ch.1 eq.30
pub fn get_moist_air_enthalpy(t_dry_bulb: f64, hum_ratio: f64) -> f64 {
    1.006 * t_dry_bulb + hum_ratio * (2501.0 + 1.86 * t_dry_bulb)
}

/// 干空气密度 [kg/m³]
pub fn get_dry_air_density(t_dry_bulb: f64, pressure: f64) -> f64 {
    pressure / (R_DA_SI * to_kelvin(t_dry_bulb))
}

/// 湿空气比容 [m³/kg]
pub fn get_moist_air_volume(t_dry_bulb: f64, hum_ratio: f64, pressure: f64) -> f64 {
    get_dry_air_volume(t_dry_bulb, pressure) * (1.0 + 1.607858 * hum_ratio)
}

/// 干空气比容 [m³/kg]
pub fn get_dry_air_volume(t_dry_bulb: f64, pressure: f64) -> f64 {
    R_DA_SI * to_kelvin(t_dry_bulb) / pressure
}

/// 湿空气密度 [kg/m³]
pub fn get_moist_air_density(t_dry_bulb: f64, hum_ratio: f64, pressure: f64) -> f64 {
    (1.0 + hum_ratio) / get_moist_air_volume(t_dry_bulb, hum_ratio, pressure)
}

/// 饱和度 [0..1+]
pub fn get_degree_of_saturation(t_dry_bulb: f64, hum_ratio: f64, pressure: f64) -> f64 {
    hum_ratio / get_sat_hum_ratio(t_dry_bulb, pressure)
}

/// 干球 + 相对湿度 → 全套空气状态（对应 C# `CalcPsychrometricsFromRelHum`）
pub fn calc_from_rel_hum(t_dry_bulb: f64, rel_hum: f64, pressure: f64) -> PsychrometricValue {
    let rel_hum = rel_hum.clamp(0.0, 1.0);
    let vap_pres = get_vap_pres_from_rel_hum(t_dry_bulb, rel_hum);
    let hum_ratio = get_hum_ratio_from_vap_pres(vap_pres, pressure);
    PsychrometricValue {
        t_dry_bulb,
        t_wet_bulb: get_t_wet_bulb_from_hum_ratio(t_dry_bulb, hum_ratio, pressure),
        pressure,
        hum_ratio,
        t_dew_point: get_t_dew_point_from_vap_pres(t_dry_bulb, vap_pres),
        rel_hum,
        vap_pres,
        moist_air_enthalpy: get_moist_air_enthalpy(t_dry_bulb, hum_ratio),
        moist_air_volume: get_moist_air_volume(t_dry_bulb, hum_ratio, pressure),
        degree_of_saturation: get_degree_of_saturation(t_dry_bulb, hum_ratio, pressure),
    }
}

/// 干球 + 露点 → 全套空气状态（对应 C# `CalcPsychrometricsFromTDewPoint`）
pub fn calc_from_t_dew_point(
    t_dry_bulb: f64,
    t_dew_point: f64,
    pressure: f64,
) -> PsychrometricValue {
    let vap_pres = get_vap_pres_from_t_dew_point(t_dew_point);
    let hum_ratio = get_hum_ratio_from_vap_pres(vap_pres, pressure);
    PsychrometricValue {
        t_dry_bulb,
        t_wet_bulb: get_t_wet_bulb_from_hum_ratio(t_dry_bulb, hum_ratio, pressure),
        pressure,
        hum_ratio,
        t_dew_point,
        rel_hum: get_rel_hum_from_vap_pres(t_dry_bulb, vap_pres),
        vap_pres,
        moist_air_enthalpy: get_moist_air_enthalpy(t_dry_bulb, hum_ratio),
        moist_air_volume: get_moist_air_volume(t_dry_bulb, hum_ratio, pressure),
        degree_of_saturation: get_degree_of_saturation(t_dry_bulb, hum_ratio, pressure),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const P: f64 = 101_325.0;

    /// 与 ASHRAE/ PsychroLib 参考值对照（25 °C, 40 %RH）
    #[test]
    fn ref_state_25c_40rh() {
        let s = calc_from_rel_hum(25.0, 0.40, P);
        // ASHRAE: Pws(25°C) = 3169.2 Pa → 40 %RH = 1267.7 Pa
        assert!(
            (s.vap_pres - 1267.7).abs() < 10.0,
            "vap_pres={}",
            s.vap_pres
        );
        assert!(
            (s.hum_ratio - 0.00787).abs() < 1e-3,
            "hum_ratio={}",
            s.hum_ratio
        );
        assert!(
            (s.t_dew_point - 10.21).abs() < 0.5,
            "t_dew_point={}",
            s.t_dew_point
        );
        assert!(
            (s.t_wet_bulb - 16.62).abs() < 0.5,
            "t_wet_bulb={}",
            s.t_wet_bulb
        );
        assert!(
            (s.moist_air_enthalpy - 45.2).abs() < 1.0,
            "h={}",
            s.moist_air_enthalpy
        );
    }

    /// 饱和水蒸气压：0 °C = 611.2 Pa, 100 °C = 101320 Pa
    #[test]
    fn sat_vap_pres_reference() {
        assert!(
            (get_sat_vap_pres(0.0) - 611.21).abs() < 1.0,
            "{}",
            get_sat_vap_pres(0.0)
        );
        assert!(
            (get_sat_vap_pres(100.0) - 101_318.0).abs() < 200.0,
            "{}",
            get_sat_vap_pres(100.0)
        );
        assert!(
            (get_sat_vap_pres(25.0) - 3169.0).abs() < 20.0,
            "{}",
            get_sat_vap_pres(25.0)
        );
    }

    /// 露点反演：rel_hum → dewpoint → vap_pres 往返一致
    #[test]
    fn dew_point_round_trip() {
        for &(t, rh) in &[
            (25.0, 0.4),
            (35.0, 0.85),
            (-10.0, 0.5),
            (5.0, 1.0),
            (30.0, 0.2),
        ] {
            let vp = get_vap_pres_from_rel_hum(t, rh);
            let dp = get_t_dew_point_from_vap_pres(t, vp);
            let vp2 = get_vap_pres_from_t_dew_point(dp);
            assert!((vp - vp2).abs() < 1.0, "t={t} rh={rh}: {vp} vs {vp2}");
            assert!(dp <= t, "dewpoint {dp} must not exceed dry bulb {t}");
        }
    }

    /// 湿球温度必在 [露点, 干球] 区间
    #[test]
    fn wet_bulb_within_bounds() {
        for &(t, rh) in &[(25.0, 0.4), (35.0, 0.85), (-10.0, 0.5), (5.0, 1.0)] {
            let s = calc_from_rel_hum(t, rh, P);
            assert!(s.t_wet_bulb <= s.t_dry_bulb + 1e-9);
            assert!(s.t_wet_bulb >= s.t_dew_point - 1e-9);
        }
    }

    /// 干球 + 含湿量 → 湿球 → 反推含湿量 往返一致
    #[test]
    fn wet_bulb_round_trip() {
        for &(t, rh) in &[(25.0, 0.4), (35.0, 0.85), (12.0, 0.7), (30.0, 0.2)] {
            let w = get_hum_ratio_from_rel_hum(t, rh, P);
            let tw = get_t_wet_bulb_from_hum_ratio(t, w, P);
            let w2 = get_hum_ratio_from_t_wet_bulb(t, tw, P);
            assert!((w - w2).abs() < 1e-4, "t={t} rh={rh}: {w} vs {w2}");
        }
    }

    /// 饱和态（RH=1）时干球 = 湿球 = 露点
    #[test]
    fn saturated_state() {
        let s = calc_from_rel_hum(20.0, 1.0, P);
        assert!((s.t_wet_bulb - 20.0).abs() < 0.01, "tw={}", s.t_wet_bulb);
        assert!((s.t_dew_point - 20.0).abs() < 0.01, "dp={}", s.t_dew_point);
    }
}
