//! DuckCM 核心计算库（C# 版的 Rust 重写）
//!
//! 组织结构：
//! - [`psychro`] —— 空气状态（ASHRAE PsychroLib，SI）
//! - [`data`]    —— 工程输入/结果模型
//! - [`house`]   —— 房屋几何与围护热工
//! - [`cool`]    —— 湿帘蒸发降温
//! - [`vent`]    —— 小窗/风机选型
//! - [`heat`]    —— 供暖曲线、末端与水温-室温耦合
//! - [`bio`]     —— 养殖呼吸、CO₂ 与饮水
//!
//! [`compute_all`] 对应 C# 主窗体 `Calculate_refresh()` + 各 `TextChanged`
//! 事件链：一次性把输入 → 结果算完。

pub mod bio;
pub mod cool;
pub mod data;
pub mod docx;
pub mod enlex;
pub mod generate;
pub mod heat;
pub mod house;
pub mod material;
pub mod psychro;
pub mod terminal;
pub mod vent;

use data::{CoolResult, HeatResult, HouseResult, ProjectState, VentResult};

/// 计算过程中的可恢复问题（结果仍可给出，但界面应提示）
#[derive(Debug, Clone, PartialEq)]
pub enum CalcWarning {
    /// 供暖散热曲线未设置
    NoHeatingCurve,
    /// 湿帘降温无解
    CoolNoSolution,
    /// 水温低于需求温度，末端无法放热
    WaterTooCold,
    /// 其他
    Message(String),
    /// 双文本违规：brief 供界面状态栏，full 供计算书「配置违规」完整原因链
    Detail {
        brief: String,
        full: String,
        en: String,
    },
}

impl CalcWarning {
    /// 计算书用完整文本
    pub fn full(&self) -> String {
        match self {
            CalcWarning::Detail { full, .. } => full.clone(),
            other => other.to_string(),
        }
    }
    /// 英文报告用文本（固定违规有定译；动态详情带 en 字段）
    pub fn en_full(&self) -> String {
        match self {
            CalcWarning::Detail { en, .. } => en.clone(),
            CalcWarning::NoHeatingCurve => {
                "Heating dissipation curve not set; unit count cannot be computed".into()
            }
            CalcWarning::CoolNoSolution => {
                "Pad cooling has no solution (duty beyond pad capability)".into()
            }
            CalcWarning::WaterTooCold => {
                "Supply/return water temperature is not above the target room temperature; \
terminals deliver nothing"
                    .into()
            }
            CalcWarning::Message(m) => m.clone(),
        }
    }
}

impl core::fmt::Display for CalcWarning {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            CalcWarning::NoHeatingCurve => write!(f, "供暖散热曲线未设置，无法计算机组数"),
            CalcWarning::CoolNoSolution => write!(f, "湿帘降温无解（工况超出湿帘能力）"),
            CalcWarning::WaterTooCold => write!(f, "供回水温度不高于需求温度，末端无输出"),
            CalcWarning::Message(m) => write!(f, "{m}"),
            CalcWarning::Detail { brief, .. } => write!(f, "{brief}"),
        }
    }
}

/// 全量计算结果
#[derive(Debug, Clone, Default)]
pub struct CalcOutcome {
    pub house: HouseResult,
    pub heat: HeatResult,
    pub cool: CoolResult,
    pub vent: VentResult,
    pub warnings: Vec<CalcWarning>,
}

/// 一次性计算全部工程量（对应 C# `Calculate_refresh`）
///
/// 计算顺序与原程序一致：
/// 1. 房屋几何 → 围护热工系数；
/// 2. 热负荷（含风机/小窗开口修正）；
/// 3. 末端选型与可达温度；
/// 4. 冷负荷（房屋 + 生物 + 设备）；
/// 5. 湿帘降温；
/// 6. 小窗/风机选型。
///
/// 气候工况来自 [`data::ClimateInput`]：外墙表面温度按**综合温度**
/// `T = T外 + ρ·I/αe`（I 由日辐射 kWh/(m²·d) 折算为 Rz·133 W/m²）求得。
pub fn compute_all(state: &ProjectState, curve: Option<&heat::HeatingCurve>) -> CalcOutcome {
    let mut warnings = Vec::new();
    let vent_in = &state.vent;
    let climate = &state.climate;

    // 墙面温度：综合温度公式自动计算（环境温度 + 太阳辐射折算），无需输入
    let wall_surface_temp = heat::wall_surface_temp(
        climate.outdoor_temp,
        climate.solar_radiation,
        climate.absorptivity,
        climate.alpha_e,
    );
    // 屋面综合温度：按屋面材料吸收系数（与墙面可不同）
    let roof_surface_temp = heat::wall_surface_temp(
        climate.outdoor_temp,
        climate.solar_radiation,
        climate.roof_absorptivity,
        climate.alpha_e,
    );
    let outdoor_cool_temp = climate.outdoor_temp;

    // 1. 几何
    let house_res = house::geometry(&state.house).unwrap_or_default();

    // 2. 热负荷（修正口径：含开口）
    let heat_load = house::heat_load_kw(
        &state.house,
        &house_res,
        vent_in,
        state.heat.min_design_temp,
        state.heat.target_room_temp,
    );

    // 3. 末端选型
    let terminal = heat::terminal_quantity(
        heat_load,
        state.heat.terminal_unit_conductance,
        state.heat.water_temp,
        state.heat.target_room_temp,
        state.heat.terminal_efficiency,
    );
    let (terminal_power_kw, terminal_count_required) = match terminal {
        Ok(t) => (t.unit_power_kw, t.count),
        Err(_) => {
            warnings.push(CalcWarning::WaterTooCold);
            (0.0, 0.0)
        }
    };
    let terminal_total_kw =
        state.heat.terminal_count * state.heat.terminal_unit_conductance / 1000.0;

    // 3b. 可达温度（热源充足时的末端-围护平衡上限；运行舍温见热网曲线）
    let (wk_wall_t, wk_air_t) = (
        house::wall_total_conductance(&house_res, vent_in),
        house::air_total_conductance(&house_res, vent_in),
    );
    let reachable_pt = heat::heat_network_point(
        state.heat.min_design_temp,
        f64::MAX,
        terminal_total_kw,
        state.house.roof_height,
        house_res.wk_floor,
        wk_wall_t,
        house_res.wk_roof,
        wk_air_t,
        state.heat.water_temp,
    );
    let reachable = reachable_pt.room_temp;

    // 3c. 供暖机组数（需散热曲线）：需求台数与「热源可供」校核式同式逆运算
    // ——单台有效能力 = P(t) × 拟合系数，再 ÷0.92 热网系数
    // （units_factor 是实配台数乘数，属于供给侧，绝不能进需求公式）
    // 燃气热风机（直吹式）无末端水路：热网系数取 1.0
    let no_net = heat::heater_no_terminal(&state.heat.heater_model_name);
    let net_f = if no_net { 1.0 } else { 0.92 };
    let (heater_capacity, heater_units) = match curve {
        Some(c) if !c.coeff.is_empty() => {
            let cap = heat::poly_eval(&c.coeff, state.heat.min_design_temp).max(0.0);
            // heater_units 内部固定 ÷0.92；直吹式把基准抬高 1/0.92，
            // 等效消除热网损失：n = load/(cap×fit)
            let cap_eff = cap * heat::fit_factor(c) * net_f / 0.92;
            let n = heat::heater_units(cap_eff, heat_load).unwrap_or(0.0);
            (cap, n)
        }
        _ => {
            warnings.push(CalcWarning::NoHeatingCurve);
            (0.0, 0.0)
        }
    };

    // 3d. 热网运行点（内核）：实配机组可供热下的舍温/水温平衡——
    // 热源不足时台数↑ → 舍温、运行供水温度、平均水温、末端散热同升；
    // 充足后饱和于末端-围护平衡
    let (units_n, fit_n) = match curve {
        Some(c) => (heat::units_factor(c), heat::fit_factor(c)),
        None => (1.0, 1.0),
    };
    let q_src = heater_capacity * units_n * fit_n * net_f;
    let net_pt = heat::heat_network_point(
        state.heat.min_design_temp,
        q_src,
        terminal_total_kw,
        state.house.roof_height,
        house_res.wk_floor,
        wk_wall_t,
        house_res.wk_roof,
        wk_air_t,
        state.heat.water_temp,
    );
    if net_pt.heat_limited && heater_capacity > 0.0 {
        // 完整受限链：单台能力 → 实配可供热 → 平衡需求 → 缺口 → 后果 → 处方
        let w_sum = house_res.wk_floor + wk_wall_t + house_res.wk_roof + wk_air_t;
        let q_need = (state.heat.target_room_temp - state.heat.min_design_temp) * w_sum / 1000.0;
        let per_unit = heater_capacity * fit_n * net_f;
        let need_units = if per_unit > 0.0 {
            (q_need / per_unit).max(0.1)
        } else {
            0.0
        };
        // 容差：供热缺口在平衡需求 3% 以内不报警（整数台数的固有舍入，
        // 舍温影响 <0.5 ℃，工程上视为满足）
        if q_src < q_need * 0.97 {
            let net_txt = if no_net { "（直吹无热网）" } else { "× 热网系数 0.92" };
            let full = format!(
            "热源受限：设计温度 {} ℃ 下单台制热 {:.1} kW，实配 {:.0} 台 × 台数修正 {:.2} {} \
＝ 可供热 {:.0} kW；维持养殖需求温度 {:.1} ℃ 需散热 {:.0} kW\
（＝（{:.1}−({:.1}))×Σ(A·K) {:.0} W/K），供热缺口 {:.0} kW，\
热网平衡舍温只能达 {:.1} ℃（比需求低 {:.1} ℃）——\
需增配至 {:.0}÷{:.1}＝{:.1} 台方可满足",
            state.heat.min_design_temp,
            heater_capacity,
            units_n,
            fit_n,
            net_txt,
            q_src,
            state.heat.target_room_temp,
            q_need,
            state.heat.target_room_temp,
            state.heat.min_design_temp,
            w_sum,
            (q_need - q_src).max(0.0),
            net_pt.room_temp,
            state.heat.target_room_temp - net_pt.room_temp,
            q_need,
            per_unit,
            need_units
        );
            warnings.push(CalcWarning::Detail {
                brief: format!(
                    "热源受限：可供热 {:.0} kW < 需求 {:.0} kW，舍温 {:.1} ℃（需 {:.1} ℃），约需增配至 {:.1} 台",
                    q_src,
                    q_need,
                    net_pt.room_temp,
                    state.heat.target_room_temp,
                    need_units
                ),
                en: format!(
                "Heat source limited: at the design temperature of {} °C one unit delivers {:.1} kW; \
{} units × unit factor {:.2} {} = {:.0} kW available; holding the breeding target \
of {:.1} °C requires {:.0} kW of emission (= ({:.1}−({:.1})) × Σ(A·K) {:.0} W/K), a shortfall of \
{:.0} kW; the heat-network balance caps the house temperature at {:.1} °C \
({:.1} °C below target) — install {:.0}÷{:.1} = {:.1} units to satisfy the demand",
                state.heat.min_design_temp,
                heater_capacity,
                units_n,
                fit_n,
                if no_net {
                    "(direct-fire, no network loss)"
                } else {
                    "× network factor 0.92"
                },
                q_src,
                state.heat.target_room_temp,
                q_need,
                state.heat.target_room_temp,
                state.heat.min_design_temp,
                w_sum,
                (q_need - q_src).max(0.0),
                net_pt.room_temp,
                state.heat.target_room_temp - net_pt.room_temp,
                q_need,
                per_unit,
                need_units
            ),
            full,
        });
        }
    }

    let heat_res = HeatResult {
        house_heat_load_kw: heat_load,
        terminal_unit_power_kw: terminal_power_kw,
        terminal_total_power_kw: terminal_total_kw,
        terminal_count_required,
        reachable_temp: reachable,
        heating_capacity_kw: heater_capacity,
        heater_units_required: heater_units,
        network_room_temp: net_pt.room_temp,
        network_supply_temp: net_pt.supply_temp,
        network_avg_water: net_pt.avg_water,
        network_q_src_kw: q_src,
        network_heat_limited: net_pt.heat_limited,
    };

    // 4. 冷负荷
    let house_cool = house::cool_load_kw(
        &house_res,
        vent_in,
        wall_surface_temp,
        roof_surface_temp,
        outdoor_cool_temp,
        25.0,
    );
    // 生物冷负荷：原 JCRL 的 SWLF 项 = 呼吸 ATP 热（YangZCon rad_HX_ATP，
    // CO2RL(总呼吸量, 0.6)，总呼吸量 = 呼吸量·体重·只数）
    let bio_cool = bio::co2_respiration_heat(state.breed.minimum_ventilation, 0.6).unwrap_or(0.0);
    let total_cool = house_cool + bio_cool + vent_in.equipment_cool_load_kw;

    let vent_res = VentResult {
        house_cool_load_kw: house_cool,
        bio_cool_load_kw: bio_cool,
        total_cool_load_kw: total_cool,
        ..Default::default()
    };

    // 5. 湿帘（物理重构模型：显热平衡 + 水分平衡，审查 R1/R2/R4/Y5）
    let pad_thickness_m = vent_in.pad_thickness_mm / 1000.0;
    let w_bio = bio::barn_moisture_kg_h(state.breed.breed_type, state.breed.breed_vol);
    let cool_res = match cool::sl_ventilation(
        total_cool,
        outdoor_cool_temp,
        pad_thickness_m,
        vent_in,
        climate.humidity_pct / 100.0,
        w_bio,
    ) {
        Ok(sol) => {
            // 迭代收敛信息不再作为警告显示（湿帘参数单已有两行读数）
            // 穿帘温差校准 Δt_eff = clamp(t_o − t_sup, 0.9, 1.2)：
            // 穿帘干球温降（= ε×室外湿球温差），随设计日湿度真实响应——
            // 高湿帘失效 → 钳 0.9 放大面积；干燥天 → 钳 1.2 封顶
            let dt_eff = (outdoor_cool_temp - sol.supply_temperature).clamp(0.9, 1.2);
            // 山墙容量兜底：Δt 已上浮到 0.5 K 仍装不下 → 警告
            if vent_in.max_installable_fans > 0.0
                && sol.fans_required > vent_in.max_installable_fans
            {
                warnings.push(CalcWarning::Message(format!(
                    "山墙可装容量 {:.0} 台：Δt 已自动上浮至 {:.2} K 压缩风量，仍需 {:.0} 台装不下——请增大单台风机风量或调整山墙布置",
                    vent_in.max_installable_fans, sol.dt_used, sol.fans_required
                )));
            }
            CoolResult {
                air_volume: sol.air_volume,
                pad_wind_speed: cool::optimal_pad_speed(pad_thickness_m),
                // GB 规范湿帘面积（Δt 湿球温差校准）：装机控制值见平衡法面积
                pad_area_gb: cool::pad_area_gb_cal(total_cool, pad_thickness_m, dt_eff)
                    .unwrap_or(0.0),
                pad_area_balance: sol.pad_area,
                water_use: sol.water_use,
                cooling_power_kw: sol.cooling_power_kw,
                fans_required: sol.fans_required,
                apparent_temperature: sol.apparent_temperature,
                room_temperature: sol.room_temperature,
                solved_humidity: Some(sol.solved_humidity),
                supply_temperature: sol.supply_temperature,
                target_adjusted: sol.target_adjusted,
                iters: sol.iters,
                pad_target: sol.pad_target,
                s_tf: sol.s_tf,
                dt_used: sol.dt_used,
                saturation_pct: sol.saturation_pct,
                pressure_drop_pa: sol.pressure_drop_pa,
                evap_capacity_kg_h: sol.evap_capacity_kg_h,
            }
        }
        Err(_) => {
            warnings.push(CalcWarning::CoolNoSolution);
            CoolResult {
                pad_wind_speed: cool::optimal_pad_speed(pad_thickness_m),
                pad_area_gb: cool::pad_area_gb(total_cool, pad_thickness_m).unwrap_or(0.0),
                ..Default::default()
            }
        }
    };

    // 6. 小窗/风机（最优小窗用 2000PPM 风量对齐风机档位；总数受间距 2 m 下限约束）
    let sizing = vent::size_ventilation(
        vent_in,
        state.breed.minimum_ventilation,
        state.breed.co2_2000ppm_vol,
        state.breed.co2_2000ppm_vol,
        total_cool,
        state.house.length,
        vent_in.window_width_m,
    );
    // 6b. 工作点衰减校核（有气动曲线机型）：湿帘降温与最小通风**互斥**，
    // 降温工况主导工作点——系统静压 ΔP_sys **直接取帘润湿压降**：由实配
    // 运行台数总风量对系统曲线 dp(Q)=Δp_帘设计×(Q_tot/L_v)² 定点迭代反校；
    // 小窗工况阻力 0.5·ρ·v² 量级 <3 Pa，不参与工作点。
    // 风机效率变为**可变值** —— η_op = Q(ΔP_sys)/Q(0 Pa 自由空气量基准)，
    // 风机数按工作点真实风量计算；无气动曲线的机型透传固定效率口径
    let hood_f = if vent_in.fan_light_hood { 0.91 } else { 1.0 };
    // 曲线匹配按「分类 + 型号」全称（两段分开存储）
    let fan_full = format!("{} {}", vent_in.fan_category, vent_in.fan_model_name);
    let aero = vent::aero_curve_for(&fan_full);
    let lv = cool_res.air_volume.max(1.0);
    let (eta_op, q_op, dp_sys, dp_pad_op, fans_final) = match &aero {
        Some(c) => {
            // 手动修改「−20Pa校核点通风量」= **定义新风机**：气动曲线形状
            // 沿用当前机型，按新 −20 Pa 基准线性缩放（效率特性 = 曲线形状
            // 之比，不随基准平移），风机数/湿帘随之重算
            let q20_ref = c.q_at(20.0);
            let scale = if q20_ref > 0.0 {
                vent_in.fan_unit_air_volume / q20_ref
            } else {
                1.0
            };
            let cq = |dp: f64| c.q_at(dp) * scale;
            // 定点迭代：单台有效风量 ⇄ 系统曲线压降（下降风机曲线 ×
            // 上升系统曲线交点唯一，≤40 轮收敛到 1 m³/h）
            let mut q_u = cq(cool_res.pressure_drop_pa.max(20.0));
            let mut dp_pad_op = cool_res.pressure_drop_pa;
            for _ in 0..40 {
                let n_run = (lv / (q_u * hood_f).max(1.0)).ceil().max(1.0);
                let q_tot = n_run * q_u * hood_f;
                let dp_try = cool_res.pressure_drop_pa * (q_tot / lv).powi(2);
                let q_try = cq(dp_try);
                let done = (q_try - q_u).abs() < 1.0;
                q_u = q_try;
                dp_pad_op = dp_try;
                if done {
                    break;
                }
            }
            let dp = dp_pad_op;
            let q_op = cq(dp) * hood_f;
            let n = if q_op > 0.0 {
                (lv / q_op).ceil() + vent_in.fans_spare
            } else {
                cool_res.fans_required
            };
            let eta = if cq(0.0) > 0.0 { q_op / cq(0.0) } else { 1.0 };
            (eta, q_op, dp, dp_pad_op, n)
        }
        None => {
            let dp = cool_res.pressure_drop_pa;
            let eff = vent_in.fan_unit_air_volume * vent_in.fan_efficiency * hood_f;
            let n = if eff > 0.0 {
                (lv / eff).ceil() + vent_in.fans_spare
            } else {
                cool_res.fans_required
            };
            (
                vent_in.fan_efficiency * hood_f,
                vent_in.fan_unit_air_volume,
                dp,
                cool_res.pressure_drop_pa,
                n,
            )
        }
    };
    if aero.is_some() && fans_final > vent_in.max_installable_fans {
        warnings.push(CalcWarning::Message(format!(
            "工作点校核：系统静压 {:.0} Pa 下单台实际风量 {:.0} m³/h（可变效率 {:.2}），\
需 {:.0} 台超过山墙可装容量 {:.0} 台——请增大风机规格或降低系统阻力",
            dp_sys,
            q_op,
            eta_op,
            fans_final,
            vent_in.max_installable_fans
        )));
    }
    let vent_res = VentResult {
        windows_min_count: sizing.as_ref().map(|s| s.windows_min).unwrap_or(0.0),
        windows_amended_count: sizing.as_ref().map(|s| s.windows_amended).unwrap_or(0.0),
        windows_best_count: sizing.as_ref().map(|s| s.windows_best).unwrap_or(0.0),
        windows_total_count: sizing.as_ref().map(|s| s.windows_total).unwrap_or(0.0),
        fans_required: fans_final,
        fan_eta_operating: eta_op,
        fan_q_operating: q_op,
        dp_system_pa: dp_sys,
        dp_pad_operating: dp_pad_op,
        aero_based: aero.is_some(),
        ..vent_res
    };

    CalcOutcome {
        house: house_res,
        heat: heat_res,
        cool: cool_res,
        vent: vent_res,
        warnings,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use data::ProjectState;

    
    /// 工作点衰减校核原型：1.5Hp 三点曲线机型 → 可变效率生效、
    /// 风机数按工作点真实风量；普通机型透传固定效率
    /// 燃气热风机（直吹式 G75）：无热网系数——需求台数 = load/(P×fit)，
    /// 与带末端热源（÷0.92）相比需求少 ~8%
    #[test]
    fn gas_air_heater_no_network_factor() {
        let mut st = data::ProjectState::default();
        let with_net = compute_all(&st, None);
        let load = with_net.heat.house_heat_load_kw;
        st.heat.heater_model_name = "G75".into();
        // 曲线换成 G75 常数 75 kW、fit 1.0（模拟选型动作）
        let m = heat::find_heater_model("G75").unwrap();
        assert!(!m.has_terminal);
        let curve = heat::HeatingCurve {
            coeff: m.poly_coeffs(),
            fit: 1.0,
            water_temp: st.heat.water_temp,
            units: 1.0,
            terminal_count: 0.0,
            unit_energy_w_per_k: 0.0,
        };
        let direct = compute_all(&st, Some(&curve));
        // 单台 75 kW 无热网：需求 = load/75
        let want = load / 75.0;
        assert!(
            (direct.heat.heater_units_required - want).abs() / want < 0.01,
            "req {} want {}",
            direct.heat.heater_units_required,
            want
        );
        assert!(heat::heater_no_terminal("G75"));
        assert!(heat::heater_no_terminal("DH60"));
        assert!(heat::heater_no_terminal("Helios 170"));
        assert!(!heat::heater_no_terminal("ZGH-170IADG2"));
        assert!(!heat::heater_no_terminal("锅炉"));
    }

    /// 手动改「−20Pa校核点通风量」= 自定义风机：曲线按新基准线性缩放，
    /// 效率特性不变、工作点风量同比、风机数联动
    #[test]
    fn custom_fan_airflow_scales_curve() {
        let mut st = data::ProjectState::default();
        let base = compute_all(&st, None);
        assert!(base.vent.aero_based);
        st.vent.fan_unit_air_volume = 40_000.0;
        let cust = compute_all(&st, None);
        // 效率特性（曲线形状之比）近似不变
        assert!(
            (cust.vent.fan_eta_operating - base.vent.fan_eta_operating).abs() < 0.02,
            "eta {} vs {}",
            cust.vent.fan_eta_operating,
            base.vent.fan_eta_operating
        );
        // 工作点单台风量近似按基准比例放大
        let r = cust.vent.fan_q_operating / base.vent.fan_q_operating;
        let want = 40_000.0 / 36_847.0;
        assert!((r - want).abs() / want < 0.02, "ratio = {r}, want ≈ {want}");
        // 风机数联动下降
        assert!(cust.vent.fans_required <= base.vent.fans_required);
    }

    #[test]
    fn aero_operating_point_variable_efficiency() {
        let mut st = data::ProjectState::default();
        // 无曲线机型（试验名称）：固定效率口径透传
        st.vent.fan_model_name = "试验定频风机 X-9".into();
        let base = compute_all(&st, None);
        assert!(!base.vent.aero_based);
        assert!((base.vent.fan_eta_operating - st.vent.fan_efficiency).abs() < 1e-9);
        // 默认推拉风机 1380-1.1KW：九点样本曲线 → 工作点可变效率
        let mut st2 = data::ProjectState::default();
        let out = compute_all(&st2, None);
        assert!(out.vent.aero_based);
        assert!(out.vent.dp_system_pa >= out.cool.pressure_drop_pa - 1e-9);
        let eta = out.vent.fan_eta_operating;
        assert!(eta > 0.9 && eta < 1.05, "eta = {eta}");
        let expect =
            (out.cool.air_volume / out.vent.fan_q_operating).ceil() + st2.vent.fans_spare;
        assert!((out.vent.fans_required - expect).abs() < 1e-9);
        assert!(out.vent.fans_required.is_finite() && out.vent.fans_required > 0.0);
        // EC50 机型同样命中
        st2.vent.fan_model_name = "推拉风机 EC50-1.5HP".into();
        let out2 = compute_all(&st2, None);
        assert!(out2.vent.aero_based);
    }

    /// 遮光罩 9% 风阻：单台有效风量下降、风机数不减（反校系统曲线后
    /// 阻力部分补偿，比值介于 0.91×裸曲线衰减之间）
    #[test]
    fn fan_light_hood_nine_percent_resistance() {
        let mut st = data::ProjectState::default();
        st.vent.fan_light_hood = false;
        let no_hood = compute_all(&st, None);
        st.vent.fan_light_hood = true;
        let hood = compute_all(&st, None);
        // 工作点风量：加罩后严格下降、且不低于裸曲线值的 0.91 倍再多
        assert!(hood.vent.fan_q_operating < no_hood.vent.fan_q_operating);
        let ratio = hood.vent.fan_q_operating / no_hood.vent.fan_q_operating;
        assert!(ratio > 0.85 && ratio < 1.0, "ratio = {ratio}");
        // 效率同向下降
        assert!(hood.vent.fan_eta_operating < no_hood.vent.fan_eta_operating);
        // 风机数不减
        assert!(hood.vent.fans_required >= no_hood.vent.fans_required);
        // 非曲线机型：效率口径精确 ×0.91
        st.vent.fan_model_name = "试验定频风机 X-9".into();
        st.vent.fan_light_hood = false;
        let plain_no = compute_all(&st, None);
        let plain_hood = {
            let mut s2 = st.clone();
            s2.vent.fan_light_hood = true;
            compute_all(&s2, None)
        };
        let pr = plain_hood.vent.fan_eta_operating / plain_no.vent.fan_eta_operating;
        assert!((pr - 0.91).abs() < 1e-9, "plain ratio = {pr}");
        assert!(plain_hood.vent.fans_required >= plain_no.vent.fans_required);
    }

    #[test]
    fn compute_all_no_nan_anywhere() {
        let state = ProjectState::default();
        let out = compute_all(&state, None);
        let all = [
            out.house.area_zq,
            out.house.area_sq,
            out.house.volume,
            out.house.wk_floor,
            out.house.wk_wall_total,
            out.house.wk_roof,
            out.house.wk_air,
            out.heat.house_heat_load_kw,
            out.heat.reachable_temp,
            out.heat.terminal_unit_power_kw,
            out.cool.air_volume,
            out.cool.pad_area_gb,
            out.vent.windows_total_count,
        ];
        for v in all {
            assert!(v.is_finite(), "出现 NaN/Inf: {all:?}");
        }
    }

    #[test]
    fn compute_all_with_curve_computes_units() {
        let state = ProjectState::default();
        let curve = heat::HeatingCurve {
            coeff: vec![100.0], // 恒定 100 kW/台
            terminal_count: 82.0,
            unit_energy_w_per_k: 300.0,
            water_temp: 50.0,
            units: 1.0,
            fit: 1.0,
        };
        let out = compute_all(&state, Some(&curve));
        assert!(out.heat.heater_units_required > 0.0);
        assert!(!out
            .warnings
            .iter()
            .any(|w| matches!(w, CalcWarning::NoHeatingCurve)));
    }
}
