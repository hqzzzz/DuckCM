//! 材料库与围护结构传热系数（导热系数计算）
//!
//! 对应 C# `SBMater` / `MaterialForm` / `Mater`：
//! 传热系数 `U = 1 / (Σ dᵢ/λᵢ + Rsi + Rse)`
//!
//! # 相对原实现的修正
//! 1. 原式 `cw = 1/(m1/1000/r1 + … + 0.15)` 中 λ 解析失败**静默取 1**，
//!    且 λ=0 时产生 `Infinity`。此处返回 `Result`，非法材料层直接报错。
//! 2. 原式只支持固定 4 层；此处支持任意层数，0 厚层自动忽略。
//! 3. 表面换热阻 0.15 m²·K/W = Rsi 0.11 + Rse 0.04（ISO 6946 墙体/屋面），
//!    改为具名常量并可按内表面/外表面工况调整。
//! 4. 内置完整 816 条材料库（原 `Resources/MATERIAL.json`），
//!    原程序每次弹窗都重新解析 JSON，这里只解析一次。

use serde::{Deserialize, Serialize};
use std::sync::OnceLock;

/// 内表面换热阻 Rsi [m²·K/W]（ISO 6946，水平热流/屋面 0.10，墙体 0.11）
pub const RSI_WALL: f64 = 0.11;
/// 外表面换热阻 Rse [m²·K/W]
pub const RSE: f64 = 0.04;
/// 原 C# 使用的合并值（0.15），保留兼容
pub const RSI_RSE_LEGACY: f64 = 0.15;

/// 材料库条目（对应 C# `Mater`）
#[allow(non_snake_case)] // 字段名须与 MATERIAL.json 一致
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Material {
    pub MATERIAL_ID: i64,
    #[serde(default)]
    pub GROUP_ID: String,
    #[serde(default)]
    pub CNAME: String,
    /// 导热系数 λ [W/(m·K)]
    #[serde(default)]
    pub CONDUCTIVITY: f64,
    /// 密度 [kg/m³]
    #[serde(default)]
    pub DENSITY: f64,
    /// 比热容 [J/(kg·K)]
    #[serde(default)]
    pub SPECIFIC_HEAT: f64,
    /// 蓄热系数（原表 S 列）
    #[serde(default)]
    pub S: f64,
    /// 蒸汽渗透系数
    #[serde(default)]
    pub STEAM: f64,
    #[serde(default)]
    pub PATTERN: String,
}

/// 构造层
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Layer {
    /// 材料名（选自材料库或自定义）
    pub name: String,
    /// 导热系数 λ [W/(m·K)]
    pub lambda: f64,
    /// 厚度 [mm]
    pub thickness_mm: f64,
}

impl Layer {
    /// 热阻 [m²·K/W]：d/λ
    pub fn resistance(&self) -> f64 {
        if self.lambda <= 0.0 || self.thickness_mm <= 0.0 {
            0.0
        } else {
            self.thickness_mm / 1000.0 / self.lambda
        }
    }
}

/// 传热系数计算错误
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MaterialError {
    /// 材料导热系数必须 > 0
    InvalidLambda,
    /// 总热阻为 0
    ZeroResistance,
    /// 输入包含 NaN/无穷
    NotFinite,
}

impl core::fmt::Display for MaterialError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            MaterialError::InvalidLambda => write!(f, "材料导热系数必须大于 0"),
            MaterialError::ZeroResistance => write!(f, "围护结构总热阻为 0，无法求传热系数"),
            MaterialError::NotFinite => write!(f, "输入包含无效数值"),
        }
    }
}

impl std::error::Error for MaterialError {}

/// 传热系数 U [W/(m²·K)]（对应 C# `SBMater.RadButton5_Click` 的 cw）
///
/// `U = 1 / (Σ dᵢ/λᵢ + Rsi + Rse)`
///
/// * `rsi` / `rse` —— 表面换热阻；传 `RSI_RSE_LEGACY` 可复现原 C# 的 0.15 合并值。
pub fn u_value(layers: &[Layer], rsi: f64, rse: f64) -> Result<f64, MaterialError> {
    if !rsi.is_finite() || !rse.is_finite() {
        return Err(MaterialError::NotFinite);
    }
    let mut r = rsi + rse;
    for l in layers {
        if !l.lambda.is_finite() || !l.thickness_mm.is_finite() {
            return Err(MaterialError::NotFinite);
        }
        if l.thickness_mm < 0.0 {
            return Err(MaterialError::NotFinite);
        }
        if l.thickness_mm == 0.0 {
            continue; // 空层忽略（原 C# 里 m=0 贡献为 0）
        }
        if l.lambda <= 0.0 {
            return Err(MaterialError::InvalidLambda);
        }
        r += l.thickness_mm / 1000.0 / l.lambda;
    }
    if r <= 0.0 {
        return Err(MaterialError::ZeroResistance);
    }
    Ok((1.0 / r * 1000.0).round() / 1000.0)
}

/// 复现原 C# 公式（λ 解析失败取 1、最多 4 层、固定 0.15）
#[doc(hidden)]
pub fn u_value_legacy(thickness_mm: [f64; 4], lambda: [f64; 4]) -> f64 {
    let lam = |v: f64| if v.is_finite() && v != 0.0 { v } else { 1.0 };
    let sum = (0..4)
        .map(|i| thickness_mm[i] / 1000.0 / lam(lambda[i]))
        .sum::<f64>();
    1.0 / (sum + 0.15)
}

/// 材料库（懒加载，全表 816 条）
pub fn material_db() -> &'static [Material] {
    static DB: OnceLock<Vec<Material>> = OnceLock::new();
    DB.get_or_init(|| {
        let raw = include_str!("../../data/MATERIAL.json");
        serde_json::from_str::<Vec<Material>>(raw).unwrap_or_default()
    })
}

/// 按分组索引材料库（GROUP_ID 去重）
pub fn material_groups() -> Vec<String> {
    let mut seen = Vec::new();
    for m in material_db() {
        if !m.GROUP_ID.is_empty() && !seen.contains(&m.GROUP_ID) {
            seen.push(m.GROUP_ID.clone());
        }
    }
    seen
}

/// 组内材料
pub fn materials_in_group(group: &str) -> Vec<&'static Material> {
    material_db()
        .iter()
        .filter(|m| m.GROUP_ID == group)
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn layer(name: &str, lambda: f64, mm: f64) -> Layer {
        Layer {
            name: name.to_string(),
            lambda,
            thickness_mm: mm,
        }
    }

    #[test]
    fn u_value_matches_csharp_formula() {
        // C# 例：100 mm 岩棉 λ=0.05 + 50 mm 混凝土 λ=1.74
        // 1/(0.1/0.05 + 0.05/1.74 + 0.15) = 1/(2 + 0.0287 + 0.15) = 0.4588
        let layers = vec![layer("岩棉", 0.05, 100.0), layer("混凝土", 1.74, 50.0)];
        let u = u_value(&layers, RSI_RSE_LEGACY, 0.0).unwrap();
        let expect = 1.0 / (0.1 / 0.05 + 0.05 / 1.74 + 0.15);
        assert!((u - expect).abs() < 0.002, "u={u} expect={expect}");

        let legacy = u_value_legacy([100.0, 50.0, 0.0, 0.0], [0.05, 1.74, 1.0, 1.0]);
        assert!((legacy - expect).abs() < 1e-9);
    }

    #[test]
    fn u_value_iso_surface_resistance() {
        // 200 mm 聚苯板 λ=0.04：U = 1/(5 + 0.11 + 0.04) = 0.1946
        let layers = vec![layer("EPS", 0.04, 200.0)];
        let u = u_value(&layers, RSI_WALL, RSE).unwrap();
        assert!((u - 0.1946).abs() < 0.001, "u={u}");
    }

    #[test]
    fn thicker_insulation_lowers_u() {
        let thin = vec![layer("EPS", 0.04, 50.0)];
        let thick = vec![layer("EPS", 0.04, 150.0)];
        assert!(u_value(&thick, RSI_WALL, RSE).unwrap() < u_value(&thin, RSI_WALL, RSE).unwrap());
    }

    #[test]
    fn rejects_zero_lambda() {
        let layers = vec![layer("坏材料", 0.0, 100.0)];
        assert_eq!(
            u_value(&layers, RSI_WALL, RSE),
            Err(MaterialError::InvalidLambda)
        );
    }

    #[test]
    fn empty_layer_is_ignored() {
        let layers = vec![layer("空", 0.05, 0.0)];
        let u = u_value(&layers, RSI_WALL, RSE).unwrap();
        assert!((u - 1.0 / 0.15).abs() < 0.01);
    }

    #[test]
    fn more_layers_supported() {
        // 5 层（原 C# 仅支持 4 层）
        let layers: Vec<Layer> = (1..=5)
            .map(|i| layer("多层", 0.5, 10.0 * i as f64))
            .collect();
        let u = u_value(&layers, RSI_WALL, RSE).unwrap();
        let sum: f64 = layers.iter().map(|l| l.resistance()).sum();
        assert!((u - 1.0 / (sum + RSI_WALL + RSE)).abs() < 0.002);
    }

    #[test]
    fn material_db_loads() {
        let db = material_db();
        assert!(db.len() > 800, "材料库条目数: {}", db.len());
        // 抽查：钢筋混凝土 λ=1.547
        let rc = db
            .iter()
            .find(|m| m.CNAME == "钢筋混凝土")
            .expect("应有钢筋混凝土");
        assert!((rc.CONDUCTIVITY - 1.547).abs() < 1e-6);
        assert!(!material_groups().is_empty());
        assert!(!materials_in_group("混凝土").is_empty());
    }
}
