//! 保温构造编辑器：材料选择 + 厚度 + 实时传热系数
//!
//! 对应 C# `SBMater`（墙板/屋面保温设置）+ `MaterialForm`（材料查询）。
//! 原程序固定 4 层、λ 失败静默取 1；这里任意层数、错误显式提示。

use duckcm_core::material::{
    material_db, material_groups, materials_in_group, u_value, Layer, RSI_RSE_LEGACY,
};
use egui::{DragValue, RichText, Ui};

use crate::theme::{self, text};

/// 常用材料（名称, 导热系数 λ W/(m·K)）
///
/// 养殖舍围护最常用的几类：保温板材、纤维棉、混凝土、金属板。
/// 一键加入构造，无需在 816 条全库里翻找。
pub const COMMON_MATERIALS: &[(&str, f64)] = &[
    ("EPS 聚苯板", 0.047),
    ("PU 硬泡聚氨酯", 0.037),
    ("凌美板（聚氨酯夹芯）", 0.025),
    ("玻璃丝棉", 0.058),
    ("岩棉", 0.070),
    ("钢筋混凝土", 1.547),
    ("加气混凝土", 0.349),
    ("水泥砂浆", 0.930),
    ("彩钢板", 45.0),
    ("空气层", 0.190),
];

/// 保温构造编辑状态
pub struct LayerEditor {
    /// 构造层（外 → 内）
    pub layers: Vec<Layer>,
    /// 材料库条目选择（显示名）
    pub mat_sel: String,
    /// 材料名称过滤（子串匹配）
    pub filter: String,
    /// 当前编辑目标（"墙板" / "屋面"）
    pub title: String,
    /// 是否可见
    pub open: bool,
    /// 全部材料库窗口（表格浏览/搜索）是否打开
    pub lib_open: bool,
    /// 构造层拖拽排序：当前被拖行索引（手柄 ≡）
    pub drag_idx: Option<usize>,
    /// 材料库窗口中展开的类别
    pub open_groups: Vec<String>,
}

impl LayerEditor {
    pub fn wall_default() -> Self {
        Self {
            // 默认保温：75mm 玻璃丝棉（λ=0.058 → U≈0.693）
            layers: vec![Layer {
                name: "玻璃丝棉".into(),
                lambda: 0.058,
                thickness_mm: 75.0,
            }],
            mat_sel: String::new(),
            filter: String::new(),
            title: "墙板保温构造".into(),
            open: false,
            lib_open: false,
            drag_idx: None,
            open_groups: Vec::new(),
        }
    }

    pub fn roof_default() -> Self {
        Self {
            // 默认保温：75mm 玻璃丝棉
            layers: vec![Layer {
                name: "玻璃丝棉".into(),
                lambda: 0.058,
                thickness_mm: 75.0,
            }],
            mat_sel: String::new(),
            filter: String::new(),
            title: "屋面保温构造".into(),
            open: false,
            lib_open: false,
            drag_idx: None,
            open_groups: Vec::new(),
        }
    }

    /// 当前传热系数 [W/(m²·K)]
    pub fn u_value_now(&self) -> f64 {
        u_value(&self.layers, RSI_RSE_LEGACY, 0.0).unwrap_or(f64::NAN)
    }

    /// 绘制编辑器窗口内容
    ///
    /// **自动应用**：结构有任何编辑（加/删层、改 λ/厚度、恢复默认）且 U 有效时
    /// 返回 `Some(u)`，调用方直接同步 wall_u/roof_u——无需「应用」按钮。
    pub fn ui(&mut self, ui: &mut Ui) -> Option<f64> {
        ui.horizontal(|ui| {
            ui.label(text::group_header(&self.title));
            ui.label(text::caption(&format!("{} 层", self.layers.len())));
        });

        // 传热系数结果置顶常显（材料列表再长也看得到）；右侧「恢复默认」
        let u = self.u_value_now();
        let valid = u.is_finite() && u > 0.0;
        let total_r: f64 = self.layers.iter().map(|l| l.resistance()).sum();
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(text::label("传热系数 U"));
                ui.label(
                    RichText::new(if valid {
                        format!("{u:.3}")
                    } else {
                        "—".into()
                    })
                    .size(20.0_f32)
                    .strong()
                    .color(if valid { theme::ACCENT } else { theme::ERR }),
                );
                ui.label(text::unit("W/(m²·K)"));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(text::button("恢复默认")).clicked() {
                        if self.title.contains("屋面") {
                            *self = Self::roof_default();
                        } else {
                            *self = Self::wall_default();
                        }
                        self.open = true;
                    }
                });
            });
            ui.label(text::caption(&format!(
                "构造热阻 Σ(d/λ) = {total_r:.4} + 表面换热阻 0.15 = {:.4}",
                total_r + RSI_RSE_LEGACY
            )));
        });
        if !valid {
            ui.colored_label(theme::ERR, text::warn("请检查：材料 λ 必须 > 0，厚度 ≥ 0"));
        }
        ui.add_space(4.0);

        let mut changed = false;
        // ── 当前构造层（可编辑）：带框卡片，与下方材料库区分 ──────────
        egui::Frame::group(ui.style()).show(ui, |ui| {
            ui.label(text::caption("当前构造层（自外向内，可直接编辑）"));
            // 表头
            ui.horizontal(|ui| {
                ui.add_sized(
                    [16.0, 20.0],
                    egui::Label::new(text::caption("序")).selectable(false),
                );
                ui.add_sized([96.0, 20.0], egui::Label::new(text::caption("材料")));
                ui.add_sized([58.0, 20.0], egui::Label::new(text::caption("λ W/(m·K)")));
                ui.add_sized([64.0, 20.0], egui::Label::new(text::caption("厚度 mm")));
                ui.add_sized([52.0, 20.0], egui::Label::new(text::caption("热阻")));
                ui.add_sized([22.0, 20.0], egui::Label::new(text::caption("")));
            });
            ui.separator();

            let mut remove: Option<usize> = None;
            let mut swap: Option<(usize, usize)> = None;
            let mut set_drag: Option<Option<usize>> = None;
            let len = self.layers.len();
            for i in 0..len {
                let layer = &mut self.layers[i];
                ui.horizontal(|ui| {
                    // 拖拽手柄：按住 ⠿ 上下移动跨行即交换（自外向内排序）
                    let dragging = self.drag_idx == Some(i);
                    let handle = ui
                        .add(
                            egui::Label::new(egui::RichText::new("≡").color(if dragging {
                                theme::ACCENT
                            } else {
                                theme::TEXT_MUTED
                            }))
                            .selectable(false)
                            .sense(egui::Sense::drag()),
                        )
                        .on_hover_text("按住拖动排序");
                    if handle.drag_started() {
                        set_drag = Some(Some(i));
                    } else if dragging {
                        let pt = ui.input(|r| r.pointer.interact_pos());
                        if handle.dragged() {
                            if let Some(pt) = pt {
                                let r = handle.rect;
                                if pt.y < r.top() && i > 0 {
                                    swap = Some((i, i - 1));
                                    set_drag = Some(Some(i - 1));
                                } else if pt.y > r.bottom() && i + 1 < len {
                                    swap = Some((i, i + 1));
                                    set_drag = Some(Some(i + 1));
                                }
                            }
                        } else {
                            set_drag = Some(None); // 松手
                        }
                    }
                    let _ = ui.add_sized(
                        [96.0, 24.0],
                        egui::TextEdit::singleline(&mut layer.name).font(egui::TextStyle::Body),
                    );
                    let _ = ui.add_sized(
                        [58.0, 24.0],
                        DragValue::new(&mut layer.lambda)
                            .speed(0.001)
                            .fixed_decimals(3)
                            .range(0.001..=10.0),
                    );
                    let _ = ui.add_sized(
                        [64.0, 24.0],
                        DragValue::new(&mut layer.thickness_mm)
                            .speed(1.0)
                            .fixed_decimals(0)
                            .range(0.0..=2000.0),
                    );

                    ui.add_sized(
                        [52.0, 24.0],
                        egui::Label::new(text::caption(&format!("{:.4}", layer.resistance()))),
                    );

                    if ui.small_button("删").clicked() {
                        remove = Some(i);
                    }
                });
            }
            if let Some(i) = remove {
                self.layers.remove(i);
                self.drag_idx = None;
            }
            if let Some((a, b)) = swap {
                self.layers.swap(a, b);
            }
            if let Some(d) = set_drag {
                self.drag_idx = d;
            }
            if self.layers.is_empty() {
                ui.label(text::warn(
                    "构造为空——从下方常用材料点击加入，或添加自定义层",
                ));
            }
        });

        ui.add_space(6.0);

        // ── 常用材料：一键加入（自适应两列）────────────────────
        ui.label(text::label("常用材料（点击加入构造）"));
        ui.add_space(2.0);
        let n = COMMON_MATERIALS.len();
        let mut clicked: Option<usize> = None;
        ui.columns(2, |cols| {
            for (i, (name, lambda)) in COMMON_MATERIALS.iter().enumerate() {
                let col = &mut cols[i % 2];
                let resp = col.add_sized(
                    [col.available_width().max(120.0), 24.0],
                    egui::Button::new(
                        RichText::new(format!("{name} λ={lambda:.3}")).size(11.0_f32),
                    ),
                );
                if resp.clicked() {
                    clicked = Some(i);
                }
            }
        });
        if let Some(i) = clicked {
            let (name, lambda) = COMMON_MATERIALS[i];
            self.layers.push(Layer {
                name: name.to_string(),
                lambda,
                thickness_mm: 50.0,
            });
            self.mat_sel = name.to_string();
            changed = true;
        }
        let _ = n;

        // ── 完整材料库：独立窗口 + 表格浏览 ─────────────────────
        ui.add_space(6.0);
        if ui
            .button(
                RichText::new(format!(
                    "全部材料库（{} 种，可按分组/名称查找）",
                    material_db().len()
                ))
                .color(theme::ACCENT),
            )
            .clicked()
        {
            self.lib_open = true;
        }

        ui.horizontal(|ui| {
            if ui.button(text::button("添加自定义层")).clicked() {
                self.layers.push(Layer {
                    name: "新材料".into(),
                    lambda: 0.05,
                    thickness_mm: 50.0,
                });
                changed = true;
            }
        });

        // ── 全部材料库窗口：类别树（可折叠）+ 全参数表格 ──────────
        if self.lib_open {
            let mut add_layer: Option<Layer> = None;
            egui::Window::new(format!(
                "MATERIAL · 全部材料库（{} 种）",
                material_db().len()
            ))
            .id(egui::Id::new(("mat_lib", &self.title)))
            .open(&mut self.lib_open)
            .default_pos([200.0, 120.0])
            .default_size([880.0, 560.0])
            .resizable(true)
            .show(ui.ctx(), |ui| {
                ui.horizontal(|ui| {
                    ui.label(text::label("名称过滤"));
                    ui.add(
                        egui::TextEdit::singleline(&mut self.filter)
                            .desired_width(200.0)
                            .hint_text("如：玻璃棉 / 聚苯 / 混凝土"),
                    );
                    if ui.small_button("清空").clicked() {
                        self.filter.clear();
                    }
                    ui.separator();
                    ui.label(text::caption("点击「加入」放入当前构造（默认 50 mm）"));
                });
                ui.separator();
                let kw = self.filter.trim().to_string();
                let groups = material_groups();
                egui::ScrollArea::vertical()
                    .auto_shrink([false, false])
                    .show(ui, |ui| {
                        let mut toggled: Vec<(String, bool)> = Vec::new();
                        for g in &groups {
                            let mats: Vec<&duckcm_core::material::Material> = materials_in_group(g)
                                .iter()
                                .map(|m| *m)
                                .filter(|m| {
                                    kw.is_empty()
                                        || m.CNAME.contains(&kw)
                                        || m.GROUP_ID.contains(&kw)
                                        || m.PATTERN.contains(&kw)
                                        || g.contains(&kw)
                                })
                                .collect();
                            if !kw.is_empty() && mats.is_empty() {
                                continue; // 过滤时隐藏无匹配类别
                            }
                            let open_default = !kw.is_empty();
                            let is_open = open_default || self.open_groups.iter().any(|o| o == g);
                            let head = egui::CollapsingHeader::new(
                                RichText::new(format!("类别: {g}（{} 条）", mats.len()))
                                    .strong()
                                    .color(theme::ACCENT),
                            )
                            .id_salt(("matcat", g))
                            .default_open(is_open);
                            let resp = head.show(ui, |ui| {
                                egui::Grid::new(("matgrid", g))
                                    .num_columns(8)
                                    .striped(true)
                                    .spacing([10.0, 3.0])
                                    .min_col_width(46.0)
                                    .show(ui, |ui| {
                                        ui.label(text::caption("编号"));
                                        ui.label(text::caption("名称"));
                                        ui.label(text::caption("导热系数 W/(m·K)"));
                                        ui.label(text::caption("密度 kg/m³"));
                                        ui.label(text::caption("比热 J/(kg·K)"));
                                        ui.label(text::caption("蓄热系数"));
                                        ui.label(text::caption("注释"));
                                        ui.label(text::caption("操作"));
                                        ui.end_row();
                                        for m in &mats {
                                            ui.label(egui::RichText::new(format!(
                                                "{}",
                                                m.MATERIAL_ID
                                            )));
                                            ui.label(egui::RichText::new(if m.CNAME.is_empty() {
                                                "（未命名）".to_string()
                                            } else {
                                                m.CNAME.clone()
                                            }));
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "{:.3}",
                                                    m.CONDUCTIVITY
                                                ))
                                                .monospace(),
                                            );
                                            ui.label(
                                                egui::RichText::new(format!("{:.0}", m.DENSITY))
                                                    .monospace(),
                                            );
                                            ui.label(
                                                egui::RichText::new(format!(
                                                    "{:.0}",
                                                    m.SPECIFIC_HEAT
                                                ))
                                                .monospace(),
                                            );
                                            ui.label(
                                                egui::RichText::new(format!("{:.3}", m.S))
                                                    .monospace(),
                                            );
                                            ui.label(text::caption(&m.PATTERN));
                                            if ui.small_button("加入").clicked() {
                                                add_layer = Some(Layer {
                                                    name: m.CNAME.clone(),
                                                    lambda: m.CONDUCTIVITY,
                                                    thickness_mm: 50.0,
                                                });
                                            }
                                            ui.end_row();
                                        }
                                    });
                            });
                            {
                                let now_open = resp.openness > 0.5;
                                if now_open != is_open {
                                    toggled.push((g.clone(), now_open));
                                }
                            }
                        }
                        for (g, o) in toggled {
                            if o {
                                if !self.open_groups.contains(&g) {
                                    self.open_groups.push(g);
                                }
                            } else if let Some(p) = self.open_groups.iter().position(|x| *x == g) {
                                self.open_groups.remove(p);
                            }
                        }
                    });
            });
            if let Some(l) = add_layer {
                self.layers.push(l);
                changed = true;
            }
        }

        let _ = changed;
        if valid {
            Some(u)
        } else {
            None
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 常用材料必须全部有效（λ > 0 且有限）
    #[test]
    fn common_materials_are_valid() {
        assert!(!COMMON_MATERIALS.is_empty());
        for (name, lambda) in COMMON_MATERIALS {
            assert!(*lambda > 0.0 && lambda.is_finite(), "{name}: λ={lambda}");
        }
    }

    /// 常用材料名不得重复
    #[test]
    fn common_materials_unique() {
        for (i, (name, _)) in COMMON_MATERIALS.iter().enumerate() {
            for (j, (other, _)) in COMMON_MATERIALS.iter().enumerate() {
                if i != j {
                    assert_ne!(name, other, "重复材料名 {name}");
                }
            }
        }
    }

    /// 用户点名的几类必须在列
    #[test]
    fn common_materials_cover_requested() {
        let joined: String = COMMON_MATERIALS
            .iter()
            .map(|(n, _)| n.to_string())
            .collect::<Vec<_>>()
            .join("|");
        for key in ["EPS", "玻璃丝棉", "混凝土", "PU", "凌美板"] {
            assert!(joined.contains(key), "缺少常用材料: {key}");
        }
    }

    /// 默认构造求出的 U 应为正
    #[test]
    fn default_assembly_has_positive_u() {
        let w = LayerEditor::wall_default();
        let u = w.u_value_now();
        assert!(u > 0.0 && u.is_finite(), "u={u}");
    }
}
