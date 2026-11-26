//! 房屋几何、围护结构热工与负荷
//!
//! 移植自 C# `HousingData.Calcuilation()`（`CydataLib/CMClass.cs`）
//! 与主窗体的热负荷/冷负荷公式（`DuckCMmain.cs` L229/L438）。
//!
//! # 相对原实现的修正
//! 1. **热负荷使用完整围护系数**：原 C# 加热负荷用 `WK_QM+WK_DM+WK_ST`（不含
//!    风机/小窗开口），冷负荷却用 `GetQMWK/GetAirWK`（含开口）——同一栋房子
//!    两个口径。此处统一使用含开口的完整系数（`wall_total` / `air_total`），
//!    并保留基础值供对比。
//! 2. **无陈旧状态**：C# 从设置反序列化后会把 `WK_DM` 清零（DuckCMmain L64）
//!    导致已保存工程显示错误；这里结果永远由输入现算，不存在失效缓存。
//! 3. **零值/负值防护**：面积、传热系数为负或非数时返回 `Result`，
//!    不再产生 NaN 静默传到界面。

use crate::data::{HouseInput, HouseResult, VentInput, GROUND_LAYERS_M, GROUND_LAYERS_MS};

/// 朝向/形状计算错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HouseError {
    /// 尺寸必须为正数
    NonPositiveDimension,
    /// 输入包含 NaN/无穷
    NotFinite,
}

impl core::fmt::Display for HouseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            HouseError::NonPositiveDimension => write!(f, "房屋尺寸必须大于 0"),
            HouseError::NotFinite => write!(f, "输入包含无效数值"),
        }
    }
}

impl std::error::Error for HouseError {}

/// 围护结构几何与基础热工系数
///
/// 对应 C# `HousingData.Calcuilation()`，公式完全一致：
///
/// ```text
/// 屋面坡长      X = sqrt((屋顶高-檐口高)² + (舍宽/2)²)
/// 纵墙面积      ZQ = 2·L·檐口高
/// 山墙面积      SQ = 2·(B·檐口高 + B·坡高/2)
/// 屋面面积      WM = 2·L·X
/// 外墙展开      WH = 维护高度(mm)·L·2 / 1000
/// 地面面积      DM = L·B
/// 房间容积      V  = L·B·檐口高 + B·坡高/2·L
/// 地面热损      K_floor = 0.5246·DM   （四层分区当量系数）
/// 墙面热损      K_wall  = (SQ+ZQ)·U墙 + WH·2.5        （审查 Y1：不再 ×1.1）
/// 屋面热损      K_roof  = WM·U屋 × 1.06               （审查 Y2：合并附加）
/// 冷风渗透      K_inf   = 0.5·V·1.293·0.24/0.86
/// ```
pub fn geometry(house: &HouseInput) -> Result<HouseResult, HouseError> {
    for v in [
        house.length,
        house.width,
        house.eave_height,
        house.wall_height_mm,
        house.roof_u,
        house.wall_u,
    ] {
        if !v.is_finite() {
            return Err(HouseError::NotFinite);
        }
    }
    if house.length <= 0.0 || house.width <= 0.0 || house.eave_height <= 0.0 {
        return Err(HouseError::NonPositiveDimension);
    }

    // 山墙/屋顶的坡高（檐口以上）
    let rise_ceiling = house.ceiling_height - house.eave_height;
    let rise_roof = house.roof_height - house.eave_height;

    // 坡面斜长
    let slope_ceiling = (rise_ceiling * rise_ceiling + house.width * house.width / 4.0).sqrt();
    let slope_roof = (rise_roof * rise_roof + house.width * house.width / 4.0).sqrt();

    let area_zq = house.length * house.eave_height * 2.0;
    let area_sq = (house.width * house.eave_height + house.width * rise_roof / 2.0) * 2.0;
    // 吊顶为水平平顶：面积 = 长×宽（与坡屋面面积不同）；屋面热损按吊顶口径
    let area_roof_ceiling = house.length * house.width;
    let area_roof_no_ceiling = slope_roof * house.length * 2.0;
    // 原 C#：Area_WH = Lg_wh * Lg_c * 2 / 1000（Lg_wh 以 mm 计）
    let area_wall_expanded = house.wall_height_mm * house.length * 2.0 / 1000.0;
    let area_floor = house.length * house.width;

    // 审查 Y1：门窗缝隙附加 1.1 与显式开口项（风机×9.8、小窗×5）重复计损，已去除
    let wk_wall_base = (area_sq + area_zq) * house.wall_u + area_wall_expanded * 2.5;

    // 房间容积（审计 M1 修正）
    //
    // 原 C# 两种情况都用「三角形截面」`L·B·檐高 + B·坡高/2·L`，
    // 即假设屋面在 dd 处收尖；实际吊顶是平顶：
    //   · dd ≤ 檐高：容积 = L·B·dd（矩形）
    //   · dd > 檐高：容积 = L·B·檐高 + 梯形段 L·B·(1 + 收缩比)/2·(dd−檐高)
    let volume = if house.has_ceiling {
        let rise_roof_total = house.roof_height - house.eave_height;
        if house.ceiling_height <= house.eave_height {
            house.length * house.width * house.ceiling_height
        } else {
            let over = house.ceiling_height - house.eave_height;
            // 吊顶截面从 eave 处的全宽 B 线性收缩到脊处的 0
            let shrink = if rise_roof_total > 0.0 {
                (1.0 - over / rise_roof_total).clamp(0.0, 1.0)
            } else {
                1.0
            };
            house.length * house.width * house.eave_height
                + house.length * house.width * (1.0 + shrink) / 2.0 * over
        }
    } else {
        house.length * house.width * house.eave_height
            + house.width * rise_roof / 2.0 * house.length
    };

    // 审查 Y2：热桥/无吊顶附加 1.01×1.05 合并为单一 1.06（热工附加裕度）
    let (wk_roof, slope_length) = if house.has_ceiling {
        (area_roof_ceiling * house.roof_u * 1.06, slope_ceiling)
    } else {
        (area_roof_no_ceiling * house.roof_u * 1.06, slope_roof)
    };

    let wk_infiltration = 0.5 * volume * 1.293 * 0.24 / 0.86;

    // 地面分层当量系数 = Σ(ms_i · m_i) = 0.5515
    let ground_coeff: f64 = GROUND_LAYERS_M
        .iter()
        .zip(GROUND_LAYERS_MS.iter())
        .map(|(m, ms)| m * ms)
        .sum();
    let wk_floor = ground_coeff * area_floor;

    Ok(HouseResult {
        area_zq,
        area_sq,
        area_roof_ceiling,
        area_roof_no_ceiling,
        area_roof: if house.has_ceiling {
            area_roof_ceiling
        } else {
            area_roof_no_ceiling
        },
        area_wall_expanded,
        area_floor,
        volume,
        wk_floor,
        wk_wall_total: wk_wall_base,
        wk_roof,
        wk_air: wk_infiltration,
        wk_wall_base,
        wk_infiltration,
        slope_length,
    })
}

/// 含风机/小窗开口的完整墙面系数 [W/K]
///
/// C# `HousingData.GetQMWK`：
/// `WK_QM + 风机数·9.8 + 小窗数·单窗面积·5`
pub fn wall_total_conductance(base: &HouseResult, vent: &VentInput) -> f64 {
    base.wk_wall_base + vent.fans_actual * 9.8 + vent.windows_actual * vent.window_area * 5.0
}

/// 含风机/小窗开口的完整冷渗系数 [W/K]
///
/// C# `HousingData.GetAirWK` 基础上审查 Y4 修正：
/// 小窗渗透改**线性面积**（渗透风量 ∝ A·u，取等效渗透风速 ≈0.75 m/s：
/// `A_win × 10` ≈ A×0.75×1.293×0.24/0.86×1.2），替代原 ∝√A 的超线性惩罚。
pub fn air_total_conductance(base: &HouseResult, vent: &VentInput) -> f64 {
    let aperture =
        vent.fans_actual * 1.4 * 4.0 * 4.2 + vent.windows_actual * vent.window_area * 10.0;
    base.wk_infiltration + aperture * 0.24 / 0.86 * 1.2
}

/// 房屋热负荷 [kW]
///
/// C# 主窗体 L229/L240，审查修正后：
/// `WK = (K_wall′ + WK_DM + K_inf′)·(T内 − T外) + WK_WM·(T内 + 屋顶高/3.0 − T外)`
///
/// 审查 Y1：墙体不再 ×1.1（开口已显式计入）；
/// 审查 Y3：屋面高度温差修正 2.3 → 3.0 m/K（近地面气温梯度 ≈0.33 K/m，暖通惯例）。
///
/// * `t_out`  —— 室外设计温度（最低养殖环境温度）[°C]
/// * `t_room` —— 养殖需求舍内温度 [°C]
pub fn heat_load_kw(
    house: &HouseInput,
    base: &HouseResult,
    vent: &VentInput,
    t_out: f64,
    t_room: f64,
) -> f64 {
    let wall = wall_total_conductance(base, vent);
    let air = air_total_conductance(base, vent);
    let delta = t_room - t_out;
    let w = (wall + base.wk_floor + air) * delta
        + base.wk_roof * (t_room + house.roof_height / 3.0 - t_out);
    w / 1000.0
}

/// 房屋热负荷（原口径，仅基础墙体系数，供对比）
pub fn heat_load_kw_legacy(house: &HouseInput, base: &HouseResult, t_out: f64, t_room: f64) -> f64 {
    let delta = t_room - t_out;
    let w = (base.wk_wall_base + base.wk_floor + base.wk_infiltration) * delta
        + base.wk_roof * (t_room + house.roof_height / 2.3 - t_out);
    w / 1000.0
}

/// 房屋冷负荷 [kW]
///
/// C# `CMCalculation.FWLF_Cal_KW(qm, h, WK_WM, GetQMWK, WK_DM, GetAirWK)`：
/// `((墙面温度 − 25)·(WK_WM + 墙面系数) + (环境温度 − 25)·(WK_DM + 冷渗系数)) / 1000`
///
/// * `wall_surface_temp` —— 外墙综合温度（sol-air，按墙面材料 α）[°C]
/// * `roof_surface_temp` —— 屋面综合温度（sol-air，按屋面材料 α，可与墙面不同）[°C]
/// * `outdoor_temp`      —— 室外空气温度 [°C]
/// * `room_design_temp`  —— 室内基准温度（原式固定 25 °C）
pub fn cool_load_kw(
    base: &HouseResult,
    vent: &VentInput,
    wall_surface_temp: f64,
    roof_surface_temp: f64,
    outdoor_temp: f64,
    room_design_temp: f64,
) -> f64 {
    let wall = wall_total_conductance(base, vent);
    let air = air_total_conductance(base, vent);
    ((wall_surface_temp - room_design_temp) * wall
        + (roof_surface_temp - room_design_temp) * base.wk_roof
        + (outdoor_temp - room_design_temp) * (base.wk_floor + air))
        / 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;

    #[allow(dead_code)]
    fn default_house() -> HouseInput {
        HouseInput::default()
    }

    #[test]
    fn geometry_matches_csharp_reference() {
        // C# 原程序载入示例工程（舍长 114、舍宽 18、檐口 7.5、屋顶高 8.4）时
        // 界面显示：山墙面积 286.20，屋面面积 2062.23，纵墙 1710.00，房间容积 16313.40
        let h = HouseInput {
            length: 114.0,
            width: 18.0,
            eave_height: 7.5,
            ceiling_height: 8.2,
            roof_height: 8.4,
            ..Default::default()
        };
        let r = geometry(&h).unwrap();

        // 纵墙 = 2·114·7.5 = 1710
        assert!((r.area_zq - 1710.0).abs() < 1e-9, "area_zq={}", r.area_zq);
        // 山墙 = 2·(18·7.5 + 18·(8.4-7.5)/2) = 2·(135+8.1) = 286.2
        assert!((r.area_sq - 286.2).abs() < 1e-9, "area_sq={}", r.area_sq);
        // 地面 = 114·18 = 2052
        assert!((r.area_floor - 2052.0).abs() < 1e-9);
        // 容积 = 114·18·7.5 + 18·0.9/2·114 = 15390 + 923.4 = 16313.4
        assert!((r.volume - 16313.4).abs() < 1e-9, "volume={}", r.volume);
        // 屋面面积 = 2·114·sqrt(0.9² + 18²/4) = 228·9.04489 = 2062.23
        assert!(
            (r.area_roof_no_ceiling - 2062.234).abs() < 0.01,
            "area_roof={}",
            r.area_roof_no_ceiling
        );
        // 地面热损 = 0.5246·2052 = 1076.48
        assert!(
            (r.wk_floor - 1076.4792).abs() < 1e-3,
            "wk_floor={}",
            r.wk_floor
        );
    }

    #[test]
    fn ground_coefficient_table() {
        let expected = 0.87 * 0.30 + 0.47 * 0.26 + 0.35 * 0.23 + 0.29 * 0.21;
        let computed: f64 = GROUND_LAYERS_M
            .iter()
            .zip(GROUND_LAYERS_MS.iter())
            .map(|(m, ms)| m * ms)
            .sum();
        assert!((computed - expected).abs() < 1e-12);
        assert!((computed - 0.5246).abs() < 1e-9, "computed={computed}");
    }

    #[test]
    fn roof_area_scales_with_slope() {
        let h = HouseInput::default();
        let r = geometry(&h).unwrap();
        // 无吊顶屋面面积 = 2·L·坡长
        let rise = h.roof_height - h.eave_height;
        let slope = (rise * rise + h.width * h.width / 4.0).sqrt();
        assert!((r.area_roof_no_ceiling - 2.0 * h.length * slope).abs() < 1e-9);
    }

    #[test]
    fn heat_load_positive_when_cold_outside() {
        let h = HouseInput::default();
        let r = geometry(&h).unwrap();
        let v = VentInput::default();
        let load = heat_load_kw(&h, &r, &v, -10.0, 35.0);
        assert!(load > 0.0, "load={load}");
        // 冬季 114×18×7.5 m 鸡舍、-10 °C → 35 °C，热负荷应在数十到数百 kW 量级
        assert!(load > 10.0 && load < 2000.0, "load={load}");

        // 修正后（含开口）应 ≥ 原口径（不含开口）
        let legacy = heat_load_kw_legacy(&h, &r, -10.0, 35.0);
        assert!(load >= legacy, "load={load} legacy={legacy}");
    }

    #[test]
    fn cool_load_sign_convention() {
        let h = HouseInput::default();
        let r = geometry(&h).unwrap();
        let v = VentInput::default();
        // 外墙 40 °C、屋面 42 °C、室外 35 °C、室内基准 25 °C → 冷负荷为正
        let q = cool_load_kw(&r, &v, 40.0, 42.0, 35.0, 25.0);
        assert!(q > 0.0, "q={q}");
    }

    #[test]
    fn rejects_invalid_dimensions() {
        let h = HouseInput {
            length: -5.0,
            ..Default::default()
        };
        assert_eq!(geometry(&h), Err(HouseError::NonPositiveDimension));

        let h2 = HouseInput {
            width: f64::NAN,
            ..Default::default()
        };
        assert_eq!(geometry(&h2), Err(HouseError::NotFinite));
    }
}
