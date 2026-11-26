//! 末端形式库（原 RGForm / RG.json）
//!
//! 每一行是一种末端（散热器 / 地暖管）：
//! - `cc_kw`  标准散热量 [kW]（样本工况测试值，仅展示）
//! - `wk_m2`  传热系数 [W/(m²·K)]
//! - `m2`     单位换热面积系数
//!
//! 选中一行的换算规则与原程序一致（RGForm.cs）：
//! 理论散热系数 [W/K] = `wk_m2 × m2`，写入 HeatInput.terminal_unit_conductance，
//! 名称写入 terminal_type_name（供暖方式）。

use serde::Deserialize;

/// 末端形式（RG.json 一行）
#[derive(Debug, Clone, Deserialize)]
pub struct TerminalType {
    /// 物料编号
    #[serde(rename = "ID")]
    pub id: u32,
    /// 名称
    #[serde(rename = "CNAME")]
    pub name: String,
    /// 标准散热量 [kW]
    #[serde(rename = "CCKW")]
    pub cc_kw: f64,
    /// 传热系数 [W/(m²·K)]
    #[serde(rename = "WKM2")]
    pub wk_m2: f64,
    /// 单位换热面积系数
    #[serde(rename = "M2")]
    pub m2: f64,
}

impl TerminalType {
    /// 理论散热系数 [W/K] = 传热系数 × 单位换热面积系数（原 MA_WK = wk * m2）
    pub fn conductance(&self) -> f64 {
        self.wk_m2 * self.m2
    }
}

/// 末端形式库（进程内加载一次）
pub fn terminal_db() -> &'static [TerminalType] {
    static DB: std::sync::OnceLock<Vec<TerminalType>> = std::sync::OnceLock::new();
    DB.get_or_init(|| {
        let raw = include_str!("../../data/RG.json");
        serde_json::from_str(raw).unwrap_or_default()
    })
    .as_slice()
}

/// 按名称查找末端形式
pub fn find_terminal(name: &str) -> Option<&'static TerminalType> {
    terminal_db().iter().find(|t| t.name == name)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn db_has_original_rows() {
        let db = terminal_db();
        assert_eq!(db.len(), 4);
        let names: Vec<&str> = db.iter().map(|t| t.name.as_str()).collect();
        assert!(names.contains(&"全铝双排"));
        assert!(names.contains(&"铜管四排"));
        assert!(names.contains(&"20mm地暖管"));
        assert!(names.contains(&"30mm地暖管"));
    }

    #[test]
    fn conductance_matches_rgform() {
        // 原 RGForm：MA_WK = WKM2 * M2，格式 "0.0"
        assert!((find_terminal("20mm地暖管").unwrap().conductance() - 0.8038).abs() < 1e-9);
        assert!((find_terminal("30mm地暖管").unwrap().conductance() - 1.138).abs() < 1e-9);
        assert!((find_terminal("全铝双排").unwrap().conductance() - 304.0).abs() < 1e-9);
        assert!((find_terminal("铜管四排").unwrap().conductance() - 644.0).abs() < 1e-9);
    }
}
