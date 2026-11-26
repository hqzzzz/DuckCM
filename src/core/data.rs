//! 工程输入与结果数据模型
//!
//! 对应 C# 的 `HousingData` / `FWData` / `SLData` / `TFVentilation` / `hEnddata` 继承链。
//! 原 C# 用一个 5 层深的可变继承链 + 反射绑定 UI，导致字段语义混乱；
//! 这里改为「输入 / 结果」两个扁平结构，可序列化，便于测试与 UI 绑定。

use serde::{Deserialize, Serialize};

/// 地面传热系数表：4 个分层，对应原 C# `dm4m` / `dm4ms` 数组
/// （0.87·0.30 + 0.47·0.26 + 0.35·0.23 + 0.29·0.21 = 0.5246 W/(m²·K)）
pub const GROUND_LAYERS_M: [f64; 4] = [0.30, 0.26, 0.23, 0.21];
pub const GROUND_LAYERS_MS: [f64; 4] = [0.87, 0.47, 0.35, 0.29];

/// 养殖种类（细分肉禽 / 蛋禽）
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BreedType {
    /// 蛋鸡（旧工程 "Chicken" 自动归入）
    #[serde(alias = "Chicken")]
    LayerChicken,
    /// 肉鸡
    BroilerChicken,
    /// 蛋鸭（旧工程 "Duck" 自动归入）
    #[serde(alias = "Duck")]
    LayerDuck,
    /// 肉鸭
    BroilerDuck,
    /// 鹅
    Goose,
    /// 其他
    Other,
}

impl Default for BreedType {
    fn default() -> Self {
        BreedType::LayerChicken
    }
}

/// 养殖参数
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct BreedData {
    /// 养殖种类
    pub breed_type: BreedType,
    /// 养殖只数 [只]
    pub breed_vol: f64,
    /// 养殖体重 [kg]
    pub breed_weight: f64,
    /// 二氧化碳呼吸量 [m³/h]（原 C02Rvol）
    pub co2_respiration_vol: f64,
    /// 2000PPM 呼吸交换量（原 C2000PPMRvol）
    pub co2_2000ppm_vol: f64,
    /// 1000PPM 呼吸交换量 [m³/h]（C# `C2000PPMRvol` 字段实际承载值 = YangZCon 界面风量，用于最优小窗）
    #[serde(default)]
    pub co2_1000ppm_vol: f64,
    /// 需水量 [kg/h]（原 BreedNeedWater）
    pub breed_need_water: f64,
    /// 最小通风量 [m³/h]（原 MinimumVentilation）
    pub minimum_ventilation: f64,
}

impl Default for BreedData {
    fn default() -> Self {
        Self {
            breed_type: BreedType::BroilerDuck,
            breed_vol: 35600.0,
            breed_weight: 3.2,
            // 最低呼吸量 [m³/h·kg]（原 C02Rvol，鸭 CO2TF5000PPM(2.8,18,1.0)）
            co2_respiration_vol: 0.41,
            co2_2000ppm_vol: 0.0,
            co2_1000ppm_vol: 0.0,
            breed_need_water: 0.0,
            minimum_ventilation: 0.0,
        }
    }
}

/// 房屋几何与围护结构输入
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HouseInput {
    /// 舍长 L [m]
    pub length: f64,
    /// 舍宽 B [m]
    pub width: f64,
    /// 檐口高度 [m]
    pub eave_height: f64,
    /// 维护高度 [mm]（原 Lg_wh，注意单位是毫米，C# 中除以 1000 转 m）
    pub wall_height_mm: f64,
    /// 吊顶高度 [m]（原 Lg_dd）
    pub ceiling_height: f64,
    /// 屋顶高度 [m]（原 Lg_wd）
    pub roof_height: f64,
    /// 屋面传热系数 [W/(m²·K)]（原 Xs_WM）
    pub roof_u: f64,
    /// 墙面传热系数 [W/(m²·K)]（原 Xs_QM）
    pub wall_u: f64,
    /// 是否有吊顶
    pub has_ceiling: bool,
}

impl Default for HouseInput {
    fn default() -> Self {
        Self {
            length: 92.0,
            width: 16.5,
            eave_height: 5.2,
            wall_height_mm: 300.0,
            ceiling_height: 4.93, // 吊顶低于屋顶(6.375→坡度限 5.28) 至少 350 mm
            roof_height: 6.375,
            // 默认保温构造：屋面 100mm 玻璃丝棉 → U≈0.534；墙面 75mm → U = 1/(0.075/0.058+0.15) ≈ 0.693
            roof_u: 0.534,
            wall_u: 0.693,
            has_ceiling: false,
        }
    }
}

/// 房屋几何与热损失结果（对应原 `FW_VsLayout` 表 + WK_* 系数）
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HouseResult {
    /// 纵墙面积 [m²]
    pub area_zq: f64,
    /// 山墙面积 [m²]
    pub area_sq: f64,
    /// 屋面面积（有吊顶）[m²]
    pub area_roof_ceiling: f64,
    /// 屋面面积（无吊顶）[m²]
    pub area_roof_no_ceiling: f64,
    /// 实际采用的屋面面积（随吊顶形式切换，审计 M2）
    pub area_roof: f64,
    /// 外墙展开面积 [m²]
    pub area_wall_expanded: f64,
    /// 地面面积 [m²]
    pub area_floor: f64,
    /// 房间容积 [m³]
    pub volume: f64,
    /// 地面热损系数 [W/K]
    pub wk_floor: f64,
    /// 墙面热损系数（含门窗/设备渗透）[W/K]
    pub wk_wall_total: f64,
    /// 屋面热损系数 [W/K]
    pub wk_roof: f64,
    /// 冷渗热损系数 [W/K]
    pub wk_air: f64,
    /// 墙体基础热损（不含开口）[W/K]
    pub wk_wall_base: f64,
    /// 冷风渗透基础系数 [W/K]
    pub wk_infiltration: f64,
    /// 屋顶坡面斜长（无吊顶）[m]
    pub slope_length: f64,
}

/// 供暖设计输入
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HeatInput {
    /// 最低养殖环境温度 [°C]（原 rad_Min_WD）
    pub min_design_temp: f64,
    /// 养殖需求舍内温度 [°C]（原 rad_YZXQ_WD）
    pub target_room_temp: f64,
    /// 供回水平均温度 [°C]（原 textBox_MaxSW）
    pub water_temp: f64,
    /// 末端单体散热能力 [W/K]（原 textBox_MDNL）
    pub terminal_unit_conductance: f64,
    /// 供暖机组单台能力 [kW]（原 radButton_GNPZ 返回 textBox23）
    pub heater_unit_capacity_kw: f64,
    /// 末端数量（实际）
    pub terminal_count: f64,
    /// 末端效率系数（原 qw=0.92）
    pub terminal_efficiency: f64,
    /// 供暖方式（末端形式名称，取自末端库 RG.json，如「20mm地暖管」）
    #[serde(default)]
    pub terminal_type_name: String,
    /// 所选热源机组型号（如「ZGH-170IADG2」，赤焰系列）
    #[serde(default)]
    pub heater_model_name: String,
    /// 实际应用手动改过机组台数（true 时不再自动配置）
    #[serde(default)]
    pub heater_units_manual: bool,
}

impl Default for HeatInput {
    fn default() -> Self {
        Self {
            min_design_temp: -10.0,
            target_room_temp: 35.0,
            water_temp: 50.0,
            // 默认末端形式：20mm地暖管（理论散热系数 = 0.8038 × 1）
            terminal_unit_conductance: 0.8038,
            heater_unit_capacity_kw: 0.0,
            terminal_count: 19965.0,
            terminal_efficiency: 0.92,
            terminal_type_name: "20mm地暖管".to_string(),
            // 默认热源机组：赤焰 ZGH-170IADG2（额定制热 170 kW，A7/W45）
            heater_model_name: "ZGH-170IADG2".to_string(),
            heater_units_manual: false,
        }
    }
}

/// 供暖结果
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct HeatResult {
    /// 房屋热负荷 [kW]（原 FwHeatload）
    pub house_heat_load_kw: f64,
    /// 末端单体输出功率 [kW]
    pub terminal_unit_power_kw: f64,
    /// 末端总能力 [kW]
    pub terminal_total_power_kw: f64,
    /// 所需末端数量 [台]
    pub terminal_count_required: f64,
    /// 可达温度 [°C]
    pub reachable_temp: f64,
    /// 供暖能力 @设计温度 [kW]
    pub heating_capacity_kw: f64,
    /// 所需机组数 [台]
    pub heater_units_required: f64,
    /// 热网运行点：舍温 [°C]（热源不足时被压低，充足＝可达温度）
    #[serde(default)]
    pub network_room_temp: f64,
    /// 热网运行点：运行供水温度 [°C]（热源不足时低于设定）
    #[serde(default)]
    pub network_supply_temp: f64,
    /// 热网运行点：平均水温 [°C]
    #[serde(default)]
    pub network_avg_water: f64,
    /// 热网运行点：可供热 Q_src [kW]（实配台数×单台能力×fit×0.92）
    #[serde(default)]
    pub network_q_src_kw: f64,
    /// 热网运行点：true = 热源受限（不足达末端-围护平衡上限）
    #[serde(default)]
    pub network_heat_limited: bool,
}

/// 气候输入（对应 C# `UIQH` 气候设置对话框）
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ClimateInput {
    /// 环境温度（室外设计温度）[°C]（原 UIQH.Htemp / radTextBox2）
    pub outdoor_temp: f64,
    /// 太阳辐射 [W/m²]（原 UIQH.QH_GZ / radTextBox3）
    pub solar_radiation: f64,
    /// 太阳辐射吸收系数 ρ（原 p，默认 0.84）
    pub absorptivity: f64,
    /// 外表面换热系数 αe [W/(m²·K)]（原 a0，默认 20）
    pub alpha_e: f64,
    /// 相对湿度 [%]
    pub humidity_pct: f64,
    /// 墙面温度 [°C]（原 UIQH.QMtemp / radQR，直接输入，默认 60；
    /// 0 = 按太阳辐射公式计算）
    #[serde(default = "default_wall_temp")]
    pub wall_temp: f64,
    /// 墙面表层材料（决定太阳辐射吸收系数 → 墙面温度自动计算）
    #[serde(default = "default_wall_surface")]
    pub wall_surface: String,
    /// 屋面表层材料（决定屋面太阳辐射吸收系数 → 屋面 sol-air 温度）
    #[serde(default = "default_wall_surface")]
    pub roof_surface: String,
    /// 屋面太阳辐射吸收系数 α（随屋面材料联动）
    #[serde(default = "default_wall_alpha")]
    pub roof_absorptivity: f64,
}

fn default_wall_alpha() -> f64 {
    0.35
}

fn default_wall_temp() -> f64 {
    60.0
}

impl Default for ClimateInput {
    fn default() -> Self {
        Self {
            outdoor_temp: 35.0,
            // 典型日总辐射（与原 C# 气候设置默认值一致）
            solar_radiation: 7.2,
            absorptivity: 0.35, // 随默认墙面材料「白色彩钢」
            alpha_e: 20.0,
            humidity_pct: 60.0,
            wall_temp: 60.0,
            wall_surface: default_wall_surface(),
            roof_surface: default_wall_surface(),
            roof_absorptivity: 0.35,
        }
    }
}

/// 实际应用湿帘装机面积默认（≈ 75 玻璃丝棉工况的平衡法计算值）
fn default_wall_surface() -> String {
    "白色彩钢".to_string()
}

/// 墙体表面材料 → 太阳辐射吸收系数 α
pub const WALL_SURFACES: [(&str, f64); 4] = [
    ("白色彩钢", 0.35),
    ("新水泥抹面", 0.45),
    ("银灰色彩钢", 0.45),
    ("浅灰混凝土", 0.55),
];

/// 按名称查吸收系数
pub fn wall_surface_alpha(name: &str) -> Option<f64> {
    WALL_SURFACES
        .iter()
        .find(|(n, _)| *n == name)
        .map(|(_, a)| *a)
}

/// 弹簧进风窗默认型号（通风面积 0.11 m²）
fn default_fan_category() -> String {
    "推拉风机".to_string()
}

fn default_window_model() -> String {
    "560×270".to_string()
}

fn default_window_width() -> f64 {
    0.56
}

fn default_pad_area() -> f64 {
    109.8
}

fn default_true() -> bool {
    true
}

fn default_pad_height() -> f64 {
    1.5
}

fn default_pad_gable_h() -> f64 {
    2.5
}

fn default_pad_layers1() -> f64 {
    1.0
}

fn default_pad_walls() -> f64 {
    3.0 // 默认侧墙 + 山墙三面
}

/// 通风输入
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VentInput {
    /// 小窗单个面积 [m²]（原 textBox_XCSize）
    pub window_area: f64,
    /// 所选窗型洞口宽 [m]（间距校核 / 最少布窗数）
    #[serde(default = "default_window_width")]
    pub window_width_m: f64,
    /// 小窗风速 [m/s]（原 textBox_Fs，0.8~3）
    pub window_speed: f64,
    /// 单台风机风量 [m³/h]（原 oFanAirVol）
    pub fan_unit_air_volume: f64,
    /// 风机效率（原 FanEFF）
    pub fan_efficiency: f64,
    /// 最低通风量 [m³/h]（原 MinimumVentilation）
    pub min_ventilation: f64,
    /// 设备冷负荷 [kW]（原 textBox_SBLF）
    pub equipment_cool_load_kw: f64,
    /// 湿帘厚度 [mm]（原 textBox_SLHd）
    pub pad_thickness_mm: f64,
    /// 风机数量（实际）（原 FansVolTrue）
    pub fans_actual: f64,
    /// 小窗数量（实际）（原 WindowsVolTrue）
    pub windows_actual: f64,
    /// 备用风机数（原 FanQuantity 的 yu 参数）
    pub fans_spare: f64,
    /// 实际应用：湿帘装机面积 [m²]（默认跟随计算值，可改）
    #[serde(default = "default_pad_area")]
    pub pad_area_actual: f64,
    /// 实际应用是否自动跟随计算值（手动改任一实际值后置 false）
    #[serde(default = "default_true")]
    pub actual_follow: bool,
    /// 山墙风机可装容量 [台]（recalc 每轮按布置口径刷新：洞口1.4/间距0.3/留边0.5；
    /// 0 = 不限，仅内部迭代用）
    #[serde(default)]
    pub max_installable_fans: f64,
    /// 侧墙湿帘高度 [m]（布置图用：长墙每道帘高，默认 1.5）
    #[serde(default = "default_pad_height", alias = "pad_layout_height")]
    pub pad_side_height: f64,
    /// 山墙湿帘高度 [m]（默认 2.5）
    #[serde(default = "default_pad_gable_h")]
    pub pad_gable_height: f64,
    /// 山墙湿帘层数 [道]（<1 时 recalc 自动写回 ⌊檐口/3.5⌋，界面永不为 0）
    #[serde(default)]
    pub pad_layers: f64,
    /// 侧墙湿帘层数 [道]（垂直堆叠，默认 1）
    #[serde(default = "default_pad_layers1")]
    pub pad_side_layers: f64,
    /// 湿帘布置墙面数 [面]（2=两侧长墙，4=长墙+山墙，默认 2）
    #[serde(default = "default_pad_walls")]
    pub pad_layout_walls: f64,
    /// 所选风机**分类**（「推拉风机」/「拢风筒风机」——与型号分段存储）
    #[serde(default = "default_fan_category")]
    pub fan_category: String,
    /// 所选风机**型号**段（如「1380-1.1KW」「EC50-1.5HP」；曲线与书内
    /// 显示按「分类 + 型号」全称匹配）
    #[serde(default)]
    pub fan_model_name: String,
    /// 已选进风小窗型号（弹簧窗选型库）
    #[serde(default = "default_window_model")]
    pub window_model_name: String,
    /// 所选风机功率 [kW]
    #[serde(default)]
    pub fan_power_kw: f64,
    /// 风机是否加**遮光罩**（进/排风口遮光百叶罩：加 9% 风阻，
    /// 有效风量 ×0.91；工作点可变效率自动包含该项）
    #[serde(default)]
    pub fan_light_hood: bool,
}

impl Default for VentInput {
    fn default() -> Self {
        Self {
            window_area: 0.1512,
            // 所选窗型洞口宽 [m]（间距校核/最少布窗数用）
            window_width_m: default_window_width(),
            // 进风洞口风速（首次启动即 2.0+ 湍流口径下限，recalc 自动解）
            window_speed: 2.2,
            // 默认风机：推拉风机 1380-1.1KW（样本 −20 Pa 插值）
            fan_unit_air_volume: 36847.0,
            fan_efficiency: 0.8,
            max_installable_fans: 0.0,
            min_ventilation: 0.0,
            equipment_cool_load_kw: 20.0,
            pad_thickness_mm: 150.0,
            fans_actual: 20.0,
            windows_actual: 94.0,
            fans_spare: 2.0,
            pad_area_actual: default_pad_area(),
            actual_follow: true,
            pad_side_height: default_pad_height(),
            pad_gable_height: default_pad_gable_h(),
            pad_layers: 0.0,
            pad_side_layers: default_pad_layers1(),
            pad_layout_walls: default_pad_walls(),
            fan_category: default_fan_category(),
            fan_model_name: "1380-1.1KW".to_string(),
            window_model_name: default_window_model(),
            fan_power_kw: 1.1,
            fan_light_hood: false,
        }
    }
}

/// 湿帘计算结果
#[derive(Debug, Clone, Default, PartialEq, Serialize, Deserialize)]
pub struct CoolResult {
    /// 计算通风量 [m³/h]
    pub air_volume: f64,
    /// 湿帘风速（过帘风速）[m/s]
    pub pad_wind_speed: f64,
    /// GB 规范面积 [m²]
    pub pad_area_gb: f64,
    /// 平衡法面积 [m²]
    pub pad_area_balance: f64,
    /// 蒸发量 [m³/h]
    pub water_use: f64,
    /// 蒸发吸热 [kW]
    pub cooling_power_kw: f64,
    /// 计算风机数 [台]
    pub fans_required: f64,
    /// 体感温度 [°C]
    pub apparent_temperature: f64,
    /// 舍内可达温度 [°C]
    pub room_temperature: f64,
    /// 室内相对湿度 [%]
    pub solved_humidity: Option<f64>,
    /// 湿帘降温极限（室外湿球）[°C]
    pub supply_temperature: f64,
    /// 目标温度是否被自动上浮
    pub target_adjusted: bool,
    /// 达标迭代轮数（原程序 SLVentilation 递归：湿度−0.01 / 目标+0.1）
    #[serde(default)]
    pub iters: u32,
    /// 收敛后的湿帘出风目标温度 [°C]
    #[serde(default)]
    pub pad_target: f64,
    /// 收敛时的吸热系数 s_tf [kW per m³/h]
    #[serde(default)]
    pub s_tf: f64,
    /// 实际采用的舍内-送风显热温差 Δt [K]（山墙装不下时自 0.4 上浮至 0.5）
    #[serde(default)]
    pub dt_used: f64,
    /// 7090 厂商曲线饱和度 ε [%]（设计面风速 1.6 m/s）
    #[serde(default)]
    pub saturation_pct: f64,
    /// 7090 厂商曲线空气侧压降 [Pa]（风机静压校核）
    #[serde(default)]
    pub pressure_drop_pa: f64,
    /// 7090 传质法总蒸发能力 [kg/h]（湿帘耗水口径，供水量/管径/水泵依据）
    #[serde(default)]
    pub evap_capacity_kg_h: f64,
}

/// 通风参数结果
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct VentResult {
    /// 最低小窗数量 [个]
    pub windows_min_count: f64,
    /// 空间修正小窗数 [个]
    pub windows_amended_count: f64,
    /// 最优小窗数 [个]（C# WindowZYQuantity / textBox_GXC）
    #[serde(default)]
    pub windows_best_count: f64,
    /// 计算小窗总数 [个]
    pub windows_total_count: f64,
    /// 计算风机数 [台]
    pub fans_required: f64,
    /// 房屋冷负荷 [kW]
    pub house_cool_load_kw: f64,
    /// 生物冷负荷 [kW]
    pub bio_cool_load_kw: f64,
    /// 基础总冷负荷 [kW]
    pub total_cool_load_kw: f64,
    /// 工作点可变效率 [—]（有气动曲线机型 = 系统阻力下 Q_op/Q_rated；
    /// 无曲线 = 输入固定效率透传）
    #[serde(default)]
    pub fan_eta_operating: f64,
    /// 工作点单台实际风量 [m³/h]（系统总静压下的 Q(ΔP_sys)）
    #[serde(default)]
    pub fan_q_operating: f64,
    /// 系统总静压 [Pa]（工作点反校：帘/窗互斥工况最不利值）
    #[serde(default)]
    pub dp_system_pa: f64,
    /// 帘工况反校压降 [Pa]（= Δp_帘设计 × (Q_tot/L_v)²，工作点自洽值；
    /// 参数单「帘压降」行显示本值，与系统静压同步）
    #[serde(default)]
    pub dp_pad_operating: f64,
    /// 效率是否为气动工作点法（false = 固定效率口径）
    #[serde(default)]
    pub aero_based: bool,
}

/// 完整工程状态：输入 + 计算结果
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ProjectState {
    pub name: String,
    pub breed: BreedData,
    pub house: HouseInput,
    pub heat: HeatInput,
    pub vent: VentInput,
    pub climate: ClimateInput,

    #[serde(default)]
    pub house_result: HouseResult,
    #[serde(default)]
    pub heat_result: HeatResult,
    #[serde(default)]
    pub cool_result: CoolResult,
    #[serde(default)]
    pub vent_result: VentResult,
}

impl Default for ProjectState {
    fn default() -> Self {
        Self {
            name: "未命名工程".to_string(),
            breed: BreedData::default(),
            house: HouseInput::default(),
            heat: HeatInput::default(),
            vent: VentInput::default(),
            climate: ClimateInput::default(),
            house_result: HouseResult::default(),
            heat_result: HeatResult::default(),
            cool_result: CoolResult::default(),
            vent_result: VentResult::default(),
        }
    }
}
