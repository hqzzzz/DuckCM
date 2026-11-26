//! 主界面
//!
//! 布局（简洁、对齐、可缩放）：
//!
//! ```text
//! ┌─ 顶栏：工程名 · 新建/打开/保存 · 重算 · 版本 ───────────────────────┐
//! ├─ 左侧参数 (380px, 滚动) ──┬─ 右侧结果 (滚动) ──────────────────────┤
//! │  ▪ 房屋参数                │  ┌ KPI：热负荷 / 总冷负荷 / 通风 / 湿帘 ┐│
//! │  ▪ 养殖参数                │  ├ 房屋 · 供暖 · 湿帘 · 通风 结果表 ────┤│
//! │  ▪ 供暖参数                │  ├ 房间可达温度曲线 ────────────────────┤│
//! │  ▪ 通风/湿帘               │  └────────────────────────────────────┘│
//! ├─ 状态栏：警告 · 计算耗时 ───────────────────────────────────────────┤
//! ```
//!
//! 任何输入变化立即重算（纯函数计算 < 1 ms），无「刷新」按钮负担。

use std::time::Instant;

use duckcm_core::data::{BreedType, ProjectState};
use duckcm_core::heat::{units_factor, HeatingCurve};
use duckcm_core::{compute_all, CalcOutcome};
use egui::{Align2, Color32, ComboBox, DragValue, FontId, Frame, RichText, Slider, Stroke, Ui};

use crate::material_ui::LayerEditor;
use crate::section::{draw_portal_frame, SectionGeometry};
use crate::theme::{self, text};

/// 五个设置画面（底部配置按钮；供暖与末端已合并）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ConfigScreen {
    /// 房屋配置（剖面图 / 围护 / 保温材料）
    House,
    /// 供暖配置（热源机组 + 末端库 / 散热能力，合并页）
    Heat,
    /// 养殖配置（只数 / 体重 / CO₂ / 饮水）
    Breed,
    /// 气候设置（温度 / 太阳辐射 / 湿度）
    Climate,
    /// 养殖设备（小窗 / 风机）
    Equip,
}

impl ConfigScreen {
    pub const ALL: [ConfigScreen; 5] = [
        ConfigScreen::House,
        ConfigScreen::Breed,
        ConfigScreen::Equip,
        ConfigScreen::Heat,
        ConfigScreen::Climate,
    ];

    pub fn label(&self) -> &'static str {
        match self {
            ConfigScreen::House => "房屋配置",
            ConfigScreen::Heat => "供暖·末端配置",
            ConfigScreen::Breed => "养殖配置",
            ConfigScreen::Climate => "气候及墙面",
            ConfigScreen::Equip => "通风设备",
        }
    }
}

/// 房屋配置画面内的子视图（就地切换，不弹窗）
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HouseView {
    /// 配置单
    Config,
    /// 墙板保温构造
    WallMaterial,
    /// 屋面保温构造
    RoofMaterial,
}

pub struct DuckApp {
    state: ProjectState,
    curve: HeatingCurve,
    outcome: CalcOutcome,
    project_path: String,
    status: String,
    calc_micros: u128,
    show_about: bool,
    /// GitHub 在线更新（CDN 检查/下载/重启应用）
    upd: crate::updater::Updater,
    /// 启动后自动检查一次（静默，结果只反映在顶栏按钮）
    upd_booted: bool,
    /// 当前打开的配置窗口（None = 全部关闭）
    screen: Option<ConfigScreen>,
    /// 房屋画面内的子视图
    house_view: HouseView,
    /// 剖面图是否放大显示
    section_large: bool,
    /// 墙板保温构造
    wall_editor: LayerEditor,
    /// 屋面保温构造
    roof_editor: LayerEditor,
    /// 启动画面计时（半透明 Splash，约 2.4 s 淡出）
    splash_start: Instant,
    /// Splash 是否已计时（首帧才启动，避免等待 GL 初始化耗尽时长）
    splash_begun: bool,
    /// Splash logo 纹理（懒加载）
    splash_tex: Option<egui::TextureHandle>,
    /// 手动保存基线（上次 save/载入的 json 本体，autoSave 已剥离；Null=尚无）
    baseline: serde_json::Value,
    /// 载入时发现的 autoSave 快照（与本体不同才弹窗询问）
    autosave_prompt: Option<serde_json::Value>,
    /// 输入变更防抖（1.5 s 静默自动保存到 json.autoSave，不碰本体）
    last_change: Option<Instant>,
    /// 窗口尺寸变更后的连续重绘剩余帧数（最大化/还原黑块修复）
    repaint_after_resize: u8,
    /// 上一帧窗口内容尺寸（变化即 resize）
    last_win_size: Option<egui::Vec2>,
    /// 上次触发湿帘补偿的风机实配台数（风机变更 → 湿帘面积自动跟算）
    pad_fan_prev: f64,
    /// 小窗均布净间距 [m]（中心距 − 窗宽，校核 ≥1.2）
    window_spacing: f64,
    /// 每侧长墙最大可布小窗数（净间距 1.2 m 模数）
    window_max_per_side: f64,
    /// 每侧长墙最小应布小窗数（净间距 ≤2.0 m 模数）
    window_min_per_side: f64,
}

impl DuckApp {
    pub fn new() -> Self {
        let state = ProjectState::default();
        // 默认热源：ZGH-170IADG2 机组（三次能力曲线 + 空气能拟合系数 0.93）
        let coeff = duckcm_core::heat::find_heater_model(&state.heat.heater_model_name)
            .map(|m| m.poly_coeffs())
            .unwrap_or_else(|| vec![120.0]);
        let curve = HeatingCurve {
            coeff,
            terminal_count: 82.0,
            unit_energy_w_per_k: 300.0,
            water_temp: 50.0,
            units: 1.0,
            fit: 0.93,
        };
        let fallback = default_project_path();
        // 记忆上次工程：同目录 exe 打开（双击）时自动载入最近使用的 json
        let project_path = remembered_project_path(&fallback);
        let mut app = Self {
            state,
            curve,
            outcome: CalcOutcome::default(),
            project_path,
            status: String::new(),
            calc_micros: 0,
            show_about: false,
            upd: crate::updater::Updater::new(),
            upd_booted: false,
            screen: None,
            house_view: HouseView::Config,
            section_large: false,
            wall_editor: LayerEditor::wall_default(),
            roof_editor: LayerEditor::roof_default(),
            splash_start: Instant::now(),
            splash_begun: false,
            splash_tex: None,
            baseline: serde_json::Value::Null,
            autosave_prompt: None,
            last_change: None,
            repaint_after_resize: 0,
            last_win_size: None,
            pad_fan_prev: f64::NAN,
            window_spacing: 0.0,
            window_max_per_side: 0.0,
            window_min_per_side: 0.0,
        };
        // 启动恢复：默认工程文件存在则直接载入（数据保持）
        app.restore_from_disk();
        // 载入后湿帘保持工程保存值：基准对齐现有风机台数，避免首帧补偿覆盖用户配置
        app.pad_fan_prev = app.state.vent.fans_actual;
        app.recalc();
        app
    }

    /// 风机变更后重设湿帘基准（载入/拖放工程后调用，防止跨工程补偿）
    fn sync_pad_fan_base(&mut self) {
        self.pad_fan_prev = self.state.vent.fans_actual;
    }

    /// 从 project_path 恢复 state/curve（文件不存在或损坏则保持默认）；
    /// 与 load() 同逻辑：autoSave 快照与本体不一致时置询问弹窗
    fn restore_from_disk(&mut self) -> bool {
        let Ok(s) = std::fs::read_to_string(&self.project_path) else {
            return false;
        };
        let Ok(v) = serde_json::from_str::<serde_json::Value>(&s) else {
            return false;
        };
        let mut base = v.clone();
        if let Some(obj) = base.as_object_mut() {
            obj.remove("autoSave");
        }
        self.baseline = base.clone();
        self.apply_payload(&base);
        self.autosave_prompt = v
            .get("autoSave")
            .filter(|a| a.get("state").map(|s| s != &base["state"]).unwrap_or(false))
            .cloned();
        true
    }

    /// 静默写盘（自动保存用，不写状态栏）
    /// 生成计算书：`as_pdf=false` 仅产出可编辑 DOC；
    /// `as_pdf=true` 继续 COM 导出 PDF 签署正本（页眉页脚/页码域 + SM3 sidecar），
    /// 成功后删除中间 DOC
    fn gen_reports(&mut self, as_pdf: bool) {
        let wdesc = Self::layers_desc(&self.wall_editor);
        let rdesc = Self::layers_desc(&self.roof_editor);
        match duckcm_core::generate::write_reports(
            &self.state,
            &self.outcome,
            self.curve.units,
            &self.project_path,
            &wdesc,
            &rdesc,
        ) {
            Ok((a, b)) => {
                if !as_pdf {
                    self.status = format!("已生成：{a} ｜ {b}");
                    return;
                }
                let mut pdfs = Vec::new();
                let mut fails = Vec::new();
                for d in [&a, &b] {
                    match duckcm_core::generate::export_pdf(std::path::Path::new(d)) {
                        Ok(p) => {
                            duckcm_core::generate::delete_with_retry(std::path::Path::new(d));
                            let _ = duckcm_core::generate::write_pdf_sm3(&p);
                            pdfs.push(
                                p.file_name()
                                    .map(|n| n.to_string_lossy().into_owned())
                                    .unwrap_or_default(),
                            );
                        }
                        Err(e) => fails.push(format!("{d}：{e}")),
                    }
                }
                if !pdfs.is_empty() {
                    self.status = format!(
                        "PDF 签署正本：{}{}",
                        pdfs.join(" ｜ "),
                        if fails.is_empty() {
                            String::new()
                        } else {
                            format!("；导出失败：{}", fails.join("；"))
                        }
                    );
                } else {
                    self.status = format!("已生成 DOC，但 PDF 导出失败：{}", fails.join("；"));
                }
            }
            Err(e) => self.status = format!("计算书生成失败: {e}"),
        }
    }

    /// 当前内存全量状态的 json 本体（version 3；不含 autoSave 字段）
    fn build_payload(&self) -> serde_json::Value {
        serde_json::json!({
            "version": 3,
            "state": self.state,
            "curve": {
                "coeff": self.curve.coeff,
                "terminal_count": self.curve.terminal_count,
                "unit_energy_w_per_k": self.curve.unit_energy_w_per_k,
                "water_temp": self.curve.water_temp,
                "units": self.curve.units,
                "fit": self.curve.fit,
            },
            "wall_layers": self.wall_editor.layers,
            "roof_layers": self.roof_editor.layers,
        })
    }

    /// 把 payload（本体或 autoSave 快照，同构）应用到内存
    fn apply_payload(&mut self, v: &serde_json::Value) {
        if let Ok(st) = serde_json::from_value::<ProjectState>(v["state"].clone()) {
            self.state = st;
        }
        // 风机型号「分类 + 型号」两段拆分/迁移（旧 json：全称连写或历史型号名）
        let raw = self.state.vent.fan_model_name.clone();
        let old_air = self.state.vent.fan_unit_air_volume;
        let (cat, model, air): (&str, &str, Option<f64>) =
            if let Some(rest) = raw.strip_prefix("推拉风机") {
                ("推拉风机", rest.trim_start(), None)
            } else if let Some(rest) = raw.strip_prefix("拢风筒风机") {
                ("拢风筒风机", rest.trim_start(), None)
            } else {
                match raw.as_str() {
                    "1.1KW 推拉风机" | "推拉风机1380-1.1KW" => (
                        "推拉风机",
                        "1380-1.1KW",
                        if (old_air - 36400.0).abs() < 1.0 { Some(36847.0) } else { None },
                    ),
                    "1.5KW 推拉风机" | "推拉风机1380-1.5KW" => (
                        "推拉风机",
                        "1380-1.5KW",
                        if (old_air - 42902.0).abs() < 1.0 { Some(42750.0) } else { None },
                    ),
                    "1.1KW 笼风筒" | "拢分筒风机 1380-1.1KW" => (
                        "拢风筒风机",
                        "1380-1.1KW",
                        if (old_air - 42100.0).abs() < 1.0 { Some(40500.0) } else { None },
                    ),
                    "拢分筒风机 1380-1.5KW" => ("拢风筒风机", "1380-1.5KW", None),
                    "1.5Hp 螺旋桨风机 50″" => ("推拉风机", "EC50-1.5HP", None),
                    _ => (self.state.vent.fan_category.as_str(), raw.as_str(), None),
                }
            };
        if cat != self.state.vent.fan_category || model != self.state.vent.fan_model_name {
            self.state.vent.fan_category = cat.to_string();
            self.state.vent.fan_model_name = model.to_string();
            if let Some(q) = air {
                self.state.vent.fan_unit_air_volume = q;
            }
        }
        if let Ok(c) = serde_json::from_value::<CurveSpec>(v["curve"].clone()) {
            self.curve = HeatingCurve {
                coeff: c.coeff,
                terminal_count: c.terminal_count,
                unit_energy_w_per_k: c.unit_energy_w_per_k,
                water_temp: c.water_temp,
                units: c.units.unwrap_or(1.0),
                fit: c.fit.unwrap_or(0.93),
            };
        }
        if let Ok(ls) =
            serde_json::from_value::<Vec<duckcm_core::material::Layer>>(v["wall_layers"].clone())
        {
            if !ls.is_empty() {
                self.wall_editor.layers = ls;
            }
        }
        if let Ok(ls) =
            serde_json::from_value::<Vec<duckcm_core::material::Layer>>(v["roof_layers"].clone())
        {
            if !ls.is_empty() {
                self.roof_editor.layers = ls;
            }
        }
    }

    /// 写盘：auto_save=true 时附带 autoSave 快照（本体保持基线不变）
    fn write_payload(&self, mut payload: serde_json::Value, auto_save: bool) -> bool {
        if auto_save {
            payload["autoSave"] = serde_json::json!({
                "at": duckcm_core::generate::now_stamp(),
                "state": self.state,
                "curve": {
                    "coeff": self.curve.coeff,
                    "terminal_count": self.curve.terminal_count,
                    "unit_energy_w_per_k": self.curve.unit_energy_w_per_k,
                    "water_temp": self.curve.water_temp,
                    "units": self.curve.units,
                    "fit": self.curve.fit,
                },
                "wall_layers": self.wall_editor.layers,
                "roof_layers": self.roof_editor.layers,
            });
        }
        serde_json::to_string_pretty(&payload)
            .map(|s| {
                let ok = std::fs::write(&self.project_path, s).is_ok();
                if ok {
                    // 记住最近工程：下次双击 exe 自动载入该文件
                    remember_project_path(&self.project_path);
                }
                ok
            })
            .unwrap_or(false)
    }

    /// 手动「保存」：本体更新为当前状态，autoSave 快照清除
    fn write_file(&mut self) -> bool {
        self.baseline = self.build_payload();
        self.write_payload(self.baseline.clone(), false)
    }

    /// 静默自动保存：写 json.autoSave（本体＝上次手动保存基线，不动）
    fn auto_tick(&mut self) {
        let base = if self.baseline.is_null() {
            self.build_payload()
        } else {
            self.baseline.clone()
        };
        self.write_payload(base, true);
    }

    /// 启动画面：程序生成的公司标识 + 系统名，白色半透明底，2.6 s 内淡出
    fn draw_splash(&mut self, ctx: &egui::Context) {
        if !self.splash_begun {
            // 首帧才起计时（GL 初始化可能耗时数秒）
            self.splash_begun = true;
            self.splash_start = Instant::now();
        }
        let t = self.splash_start.elapsed().as_secs_f32();
        if t > 2.6 {
            // 启动后前 4 s 强制连续重绘：规避 winit 遮挡节流导致 splash 不出画
            if self.splash_start.elapsed().as_secs_f32() < 4.0 {
                ctx.request_repaint();
            }
            return;
        }
        ctx.request_repaint();
        let screen = ctx.screen_rect();
        let painter = ctx.layer_painter(egui::LayerId::new(
            egui::Order::Foreground,
            egui::Id::new("splash"),
        ));
        let a = ((2.6 - t) / 0.7).min(1.0).max(0.0);
        let alpha = (a * 255.0) as u8;
        let fa = |c: Color32| Color32::from_rgba_unmultiplied(c.r(), c.g(), c.b(), alpha);
        // 白色底
        painter.rect_filled(screen, 0.0, fa(Color32::WHITE));

        let cx = screen.center().x;
        let cy = screen.center().y;
        // 自适应缩放（以 1200 宽为基准）
        let k = (screen.width() / 1200.0).clamp(0.7, 1.25);
        let fs = |v: f32| v * k;

        let ink = fa(Color32::from_rgb(24, 24, 26));

        // ── 集团标识：LOGO.png（白底与 splash 白底融合，程序内嵌）──
        if self.splash_tex.is_none() {
            let img = image::load_from_memory(include_bytes!("../assets/logo.png"))
                .map(|i| i.to_rgba8())
                .expect("logo.png 应可解码");
            let (w, h) = (img.width() as usize, img.height() as usize);
            let ci = egui::ColorImage::from_rgba_unmultiplied([w, h], img.as_raw());
            self.splash_tex =
                Some(ctx.load_texture("splash_logo", ci, egui::TextureOptions::LINEAR));
        }
        if let Some(tex) = &self.splash_tex {
            let sz = tex.size(); // [w, h] px
            let lw = fs(470.0);
            let lh = lw * (sz[1] as f32 / sz[0] as f32);
            let rect =
                egui::Rect::from_center_size(egui::pos2(cx, cy - fs(112.0)), egui::vec2(lw, lh));
            painter.image(
                tex.id(),
                rect,
                egui::Rect::from_min_max(egui::pos2(0.0, 0.0), egui::pos2(1.0, 1.0)),
                Color32::from_white_alpha(alpha),
            );
        }

        // ── 系统主标题（大字距）──
        painter.text(
            egui::pos2(cx, cy + fs(16.0)),
            Align2::CENTER_TOP,
            "养 殖 用 房 屋 设 备 自 动 化 计 算 系 统",
            FontId::proportional(fs(44.0_f32)),
            ink,
        );

        // ── 右下英文公司名（绿）+ 设计人邮箱 ──
        painter.text(
            egui::pos2(cx + fs(332.0), cy + fs(128.0)),
            Align2::RIGHT_TOP,
            "Qingdao Xinguangzheng Steel Structure Co., Ltd",
            FontId::monospace(fs(12.5_f32)),
            fa(Color32::from_rgb(46, 139, 87)),
        );
        painter.text(
            egui::pos2(cx + fs(332.0), cy + fs(148.0)),
            Align2::RIGHT_TOP,
            "yang.linhao@hotmail.com",
            FontId::monospace(fs(12.5_f32)),
            fa(Color32::from_rgb(46, 139, 87)),
        );
    }

    fn recalc(&mut self) {
        let t = Instant::now();
        // 风机实配变更 → 湿帘面积自动补偿：按**运行风机数**口径
        //（⌈实配 × 0.9⌉——10% 备用机不常开不计风量；÷1.6 m/s 面风速；模块上取整）
        if (self.state.vent.fans_actual - self.pad_fan_prev).abs() > 1e-9 {
            self.pad_fan_prev = self.state.vent.fans_actual;
            let v = &self.state.vent;
            let module = 0.6 * v.pad_side_height.max(0.3);
            let run_f = (v.fans_actual * 0.9).ceil();
            let need = run_f * v.fan_unit_air_volume / 3600.0 / 1.6;
            self.state.vent.pad_area_actual = (need / module).ceil() * module;
        }
        // 山墙风机可装容量（Δt 自适应约束：装不下时内核自 0.4 上浮 Δt 至 0.5）
        self.state.vent.max_installable_fans =
            duckcm_core::vent::gable_fan_capacity(&self.state.house);
        // 全链迭代收敛（≤5 轮）：养殖派生 → 几何/热/冷/湿帘/通风 → 机组台数
        // → 实际装机跟随 → 装机数反馈洞口/渗透 → 再算……直到稳定。
        // 房屋、养殖、气候、末端任何输入变更都会走完整链重算。
        let mut prev_sig = [f64::NAN; 7];
        // 屋面坡度限制：常用坡度 1/6 ~ 1/3 → 屋脊 ≤ 檐口 + 半跨/3（= 宽/6）
        {
            let h = &mut self.state.house;
            let ridge_max = h.eave_height + h.width / 6.0;
            if h.roof_height > ridge_max {
                h.roof_height = ridge_max;
            }
            // 吊顶高度必须低于屋顶高度至少 350 mm
            let ceil_max = h.roof_height - 0.35;
            if h.ceiling_height > ceil_max {
                h.ceiling_height = ceil_max;
            }
        }
        // 墙面吸收系数由「墙面材料」唯一决定（打开旧工程也自动对齐）
        {
            let c = &mut self.state.climate;
            if let Some(a) = duckcm_core::data::wall_surface_alpha(&c.wall_surface) {
                c.absorptivity = a;
            }
        }
        // 湿帘层数：为 0 时按檐口自动写回（界面不显示 0）；侧墙层数最少 1
        {
            let v = &mut self.state.vent;
            let eave = self.state.house.eave_height;
            if v.pad_layers < 1.0 {
                v.pad_layers = (eave / 3.5).floor().max(1.0);
            }
            if v.pad_side_layers < 1.0 {
                v.pad_side_layers = 1.0;
            }
            // 小窗面积/窗宽由选型库唯一决定（弧面窗有效面积 = 洞口×0.7）
            if let Some(m) = duckcm_core::vent::window_db()
                .iter()
                .find(|m| m.name == v.window_model_name)
            {
                v.window_area = m.vent_area;
                v.window_width_m = m.w_mm as f64 / 1000.0;
            }
        }
        for _ in 0..5 {
            // 小窗洞口风速自动取「舍中轴速 0.6」反解值（下限 2.2，不设上限）
            if let Some(v0) = duckcm_core::vent::reynolds_window_speed(
                self.state.vent.window_area,
                self.state.house.width / 2.0,
            ) {
                self.state.vent.window_speed = v0.max(2.2);
            }
            // 养殖派生量：最低维持风量 = 呼吸量 × 体重 × 只数（原 YangZCon 自动联动）
            {
                let b = &self.state.breed;
                let num = b.co2_respiration_vol * b.breed_weight * b.breed_vol;
                self.state.breed.minimum_ventilation = num;
                // 字段语义为 2000PPM 呼吸交换量（5000→2000 ppm 稀释系数 1.875）
                self.state.breed.co2_2000ppm_vol =
                    duckcm_core::bio::co2_ventilation_for_ppm(2000.0, num, 5000.0).unwrap_or(0.0);
                // 1000PPM 风量（C# 界面「风量」581653 一类值，用于最优小窗 WindowZYQuantity）
                self.state.breed.co2_1000ppm_vol =
                    duckcm_core::bio::co2_ventilation_for_ppm(1000.0, num, 5000.0).unwrap_or(0.0);
            }
            self.outcome = compute_all(&self.state, Some(&self.curve));
            // 热源机组台数按设计工况热负荷自动配置（热源选择界面只选设备，不手填台数）；
            // 实配 = ⌈需求台数(核) × 1.1⌉，与核心公式同链
            if !self.state.heat.heater_units_manual {
                let req = self.outcome.heat.heater_units_required;
                if req > 0.0 {
                    self.curve.units = (req * 1.1).ceil().max(1.0);
                }
            }
            // 实际应用自动跟随计算值（手动改过则停止跟随）
            // 收敛判据含通风量与基础总冷负荷：整链（洞口↔冷负荷↔湿帘风量）必须稳定
            let air_vol = self.outcome.cool.air_volume;
            let total_cool = self.outcome.vent.total_cool_load_kw;
            let sig: [f64; 7];
            if self.state.vent.actual_follow {
                // 实际应用 = 计算值 ×1.1 取整；
                // 湿帘按额定抽吸能力算出后，按单块 600 宽模块向上取整
                let w = self.outcome.vent.windows_total_count.ceil();
                // 实配 = 计算值（工作点反校口径 vent.fans_required）含
                // 10% 冗余向下取整，与「计算风机数」显示同源
                let f = (self.outcome.vent.fans_required * 1.1).floor().max(1.0);
                let side_h = self.state.vent.pad_side_height.max(0.3);
                let module_area = 0.6 * side_h; // 单块湿帘面积（600 宽 × 侧墙高）
                let p = {
                    // 湿帘按运行风机数（实配含 10% 备用，×0.9 取整台数）抽吸平衡
                    let run_f = (f * 0.9).ceil();
                    let need = run_f * self.state.vent.fan_unit_air_volume / 3600.0 / 1.6;
                    (need / module_area).ceil() * module_area
                };
                let t = (self.outcome.heat.terminal_count_required * 1.1).ceil();
                let v = &mut self.state.vent;
                v.windows_actual = w;
                v.fans_actual = f;
                v.pad_area_actual = p;
                self.state.heat.terminal_count = t;
                sig = [w, f, p, self.curve.units, air_vol, total_cool, t];
            } else {
                // 手动值自由输入（允许低于计算需求——不足时输入框转橙提示，
                // 不再自动抬升；风险由通风/热网警告与橙底共同提示）
                // 机组数仅保底 ≥1 台
                if self.curve.units < 1.0 {
                    self.curve.units = 1.0;
                }
                let v = &self.state.vent;
                sig = [
                    v.windows_actual,
                    v.fans_actual,
                    v.pad_area_actual,
                    self.curve.units,
                    air_vol,
                    total_cool,
                    self.state.heat.terminal_count,
                ];
            }
            // 与上一轮一致 → 已收敛（风量/冷负荷 0.5 容差，其余 0.01）
            let stable = sig
                .iter()
                .zip(prev_sig.iter())
                .enumerate()
                .all(|(i, (a, b))| (a - b).abs() < if i >= 4 && i <= 5 { 0.5 } else { 0.01 });
            prev_sig = sig;
            if stable {
                break;
            }
        }
        // 小窗间距校核：长墙两侧均布，**洞口净间距** = 中心距 − 窗宽 ≥ 1.2 m
        {
            let per_side = (self.state.vent.windows_actual / 2.0).ceil().max(1.0);
            let pitch = self.state.house.length / per_side;
            // 窗宽从型号库查（"560×270" → 0.56 m）
            let win_w = duckcm_core::vent::window_db()
                .iter()
                .find(|m| m.name == self.state.vent.window_model_name)
                .map(|m| m.w_mm as f64 / 1000.0)
                .unwrap_or(self.state.vent.window_width_m.max(0.56));
            let clear = pitch - win_w;
            self.window_spacing = clear;
            // 每侧可布数区间（净间距 1.2 ~ 2.0 m：中心距模数 = 窗宽 + 净距）
            let max_per_side = (self.state.house.length / (win_w + 1.2)).floor().max(0.0);
            let min_per_side = (self.state.house.length / (win_w + 2.0)).ceil().max(1.0);
            self.window_max_per_side = max_per_side;
            self.window_min_per_side = min_per_side;
            if clear < 1.2 {
                self.outcome.warnings.push(duckcm_core::CalcWarning::Message(format!(
                    "小窗净间距 {clear:.2} m < 1.2 m（中心距 {pitch:.2} − 窗宽 {win_w:.2}，每侧 {per_side:.0} 个），每侧最多 {max_per_side:.0} 个（共 {:.0}），请减数或改大洞口型号",
                    max_per_side * 2.0
                )));
            } else if clear > 2.0 {
                self.outcome.warnings.push(duckcm_core::CalcWarning::Message(format!(
                    "小窗净间距 {clear:.2} m > 2.0 m（每侧仅 {per_side:.0} 个，布窗过稀），每侧至少 {min_per_side:.0} 个（共 {:.0}）",
                    min_per_side * 2.0
                )));
            }
        }
        self.calc_micros = t.elapsed().as_micros();
        // 审查 G2：洞口风速超 4.5 m/s 噪声/贼风风险，提示加大洞口型号（不钳制取值）
        if self.state.vent.window_speed > 4.5 {
            self.outcome.warnings.push(duckcm_core::CalcWarning::Message(format!(
                "洞口风速 {:.2} m/s > 4.5 m/s（射流保底所需），洞口噪声与贼风风险，建议改用更大洞口型号",
                self.state.vent.window_speed
            )));
        }
        // 控制台计算摘要（debug 运行 `cargo run` 时随每次重算刷新，逐参数一行）
        #[cfg(debug_assertions)]
        {
            let v = &self.state.vent;
            let c = &self.outcome.cool;
            let vt = &self.outcome.vent;
            let hh = &self.outcome.heat;
            let bd = &self.state.breed;
            let daily_feed = breed_feed(bd.breed_type) * bd.breed_vol as f64; // kg/日
            println!("──────── 计算 {:>6} µs ────────", self.calc_micros);
            println!("通风量      {:>10.0} m³/h", c.air_volume);
            println!(
                "风机        {:>4.0} 计算 / {:<4.0} 实配 台（{}）",
                vt.fans_required, v.fans_actual, v.fan_model_name
            );
            // 山墙截面风速 = 实配风机额定总风量 ÷ 山墙面积（不乘效率系数）
            let hs = &self.state.house;
            let gable_area =
                hs.width * hs.eave_height + hs.width * (hs.roof_height - hs.eave_height) / 2.0;
            let gable_flow = v.fans_actual * v.fan_unit_air_volume / 3600.0;
            println!(
                "山墙截面风速 {:>8.2} m/s（实配 {:<3.0} 台 × {:.0} m³/h ÷ 山墙 {:.1} m²）",
                if gable_area > 0.0 {
                    gable_flow / gable_area
                } else {
                    0.0
                },
                v.fans_actual,
                v.fan_unit_air_volume,
                gable_area
            );
            println!(
                "小窗        {:>4.0} 计算 / {:<4.0} 实配 个（{}，洞口风速 {:.2} m/s）",
                vt.windows_total_count, v.windows_actual, v.window_model_name, v.window_speed
            );
            println!(
                "湿帘        {:>7.1} 平衡法 / {:<7.1} 实配 m²（厚 {:.0} mm，过帘 {:.2} m/s）",
                c.pad_area_balance, v.pad_area_actual, v.pad_thickness_mm, c.pad_wind_speed
            );
            println!(
                "供暖机组    {:>4.1} 计算 / {:<4.0} 实配 台",
                hh.heater_units_required, self.curve.units
            );
            println!(
                "热网运行点  舍温 {:>5.1} / 供水 {:>5.1} ℃（Q_src {:.0} kW，{}）",
                hh.network_room_temp,
                hh.network_supply_temp,
                hh.network_q_src_kw,
                if hh.network_heat_limited {
                    "热源受限"
                } else {
                    "充足→饱和于可达温度"
                }
            );
            println!(
                "末端数量    {:>8.0} 计算 / {:<8.0} 实配",
                hh.terminal_count_required, self.state.heat.terminal_count
            );
            println!("房屋热负荷  {:>10.1} kW", hh.house_heat_load_kw);
            println!(
                "冷负荷      房屋 {:>7.1} + 生物 {:>6.2} + 设备 {:>5.1} = {:>7.1} kW",
                vt.house_cool_load_kw,
                vt.bio_cool_load_kw,
                v.equipment_cool_load_kw,
                vt.total_cool_load_kw
            );
            println!(
                "湿帘模型    送风 {:.1} ℃ 目标 {:.1} ℃ Δt {:.1} K；舍内湿度 {:.1}%；净吸热 s {:.4} kW/(m³/h)",
                c.supply_temperature,
                c.pad_target,
                (c.pad_target - c.supply_temperature).max(0.3),
                c.solved_humidity.unwrap_or(0.0),
                c.s_tf
            );
            println!(
                "7090 曲线   饱和度 ε {:.1}% @1.6 m/s · {:.0} mm；压降 {:.0} Pa",
                c.saturation_pct, self.state.vent.pad_thickness_mm, c.pressure_drop_pa
            );
            println!(
                "呼吸 ATP    {:>10.2} kW（CO2RL：最低通风 {:.0}×0.03%×2870×0.6÷22.4÷3600）",
                vt.bio_cool_load_kw, bd.minimum_ventilation
            );
            println!(
                "日采食      {:>8.0} kg/日（{:.3} kg/只 × {:.0} 只）",
                daily_feed,
                breed_feed(bd.breed_type),
                bd.breed_vol
            );
            println!(
                "突发用水    {:>8.2} m³/h（日采食 ÷3 × 水料比 2.2）",
                daily_feed / 3.0 * 2.2 / 1000.0
            );
            println!("最低供水量  {:>8.2} m³/h", bd.breed_need_water);
            println!("蒸发需水量  {:>8.2} m³/h", c.evap_capacity_kg_h / 1000.0);
            println!(
                "警告        {:>4} 条{}",
                self.outcome.warnings.len(),
                if self.outcome.warnings.is_empty() {
                    String::new()
                } else {
                    self.outcome
                        .warnings
                        .iter()
                        .map(|w| format!("\n  ⚠ {w}"))
                        .collect::<String>()
                }
            );
        }
    }

    // ── 文件操作 ────────────────────────────────────────────────
    fn save(&mut self) {
        // 统一走 write_file（payload 与自动保存一致，含构造层），成功即更新状态栏
        if self.write_file() {
            self.status = format!("已保存到 {}", self.project_path);
        } else {
            self.status = "保存失败".to_string();
        }
    }

    fn load(&mut self) {
        match std::fs::read_to_string(&self.project_path) {
            Ok(s) => match serde_json::from_str::<serde_json::Value>(&s) {
                Ok(v) => {
                    // 基线＝去掉 autoSave 的本体；应用本体
                    let mut base = v.clone();
                    if let Some(obj) = base.as_object_mut() {
                        obj.remove("autoSave");
                    }
                    self.baseline = base.clone();
                    self.apply_payload(&base);
                    // autoSave 快照与本体不一致 → 载入后弹窗询问采用哪份
                    self.autosave_prompt = v
                        .get("autoSave")
                        .filter(|a| a.get("state").map(|s| s != &base["state"]).unwrap_or(false))
                        .cloned();
                    self.sync_pad_fan_base();
                    self.recalc();
                    remember_project_path(&self.project_path);
                    self.status = format!("已载入 {}", self.project_path);
                }
                Err(e) => self.status = format!("工程文件损坏: {e}"),
            },
            Err(e) => self.status = format!("读取失败: {e}"),
        }
    }

    fn reset(&mut self) {
        self.state = ProjectState::default();
        self.baseline = serde_json::Value::Null;
        self.autosave_prompt = None;
        self.recalc();
        self.status = "已新建工程".to_string();
    }
}

#[derive(serde::Deserialize)]
struct CurveSpec {
    coeff: Vec<f64>,
    terminal_count: f64,
    unit_energy_w_per_k: f64,
    water_temp: f64,
    #[serde(default)]
    units: Option<f64>,
    #[serde(default)]
    fit: Option<f64>,
}

fn default_project_path() -> String {
    let dir = dirs::document_dir()
        .or_else(dirs::home_dir)
        .unwrap_or_else(|| std::path::PathBuf::from("."));
    let dir = dir.join("DuckCM");
    let _ = std::fs::create_dir_all(&dir);
    dir.join("未命名工程.json").to_string_lossy().into_owned()
}

/// 同目录「最近工程」记忆文件（.last_project）：双击 exe 打开时优先载入
/// 上次使用过的工程 json；文件缺失/无效则回默认路径
fn remembered_project_path(fallback: &str) -> String {
    let mem = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".last_project");
    if let Ok(s) = std::fs::read_to_string(&mem) {
        let p = s.trim().to_string();
        if !p.is_empty() && std::path::Path::new(&p).is_file() {
            return p;
        }
    }
    fallback.to_string()
}

fn remember_project_path(path: &str) {
    let mem = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join(".last_project");
    let _ = std::fs::write(mem, path);
}

impl eframe::App for DuckApp {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        let mut changed = false;

        // 更新器：收割后台消息；进行中保持重绘节拍
        if self.upd.poll() {
            ctx.request_repaint_after(std::time::Duration::from_millis(200));
        }
        // 启动后静默自动检查一次（Splash 淡出完成后发起）
        if !self.upd_booted && self.splash_start.elapsed().as_secs_f32() > 2.8 {
            self.upd_booted = true;
            self.upd.check(ctx);
        }

        // 标题栏跟随当前工程名（未命名时显示默认系统名）
        let name = self.state.name.trim().to_string();
        let want = if name.is_empty() {
            "DuckCM 2 · 养殖用房屋设备自动化计算系统".to_string()
        } else {
            format!("DuckCM 2 · {name}")
        };
        ctx.send_viewport_cmd(egui::ViewportCommand::Title(want));

        // 最大化/还原/拖拽改变尺寸后连续强制重绘数帧——
        // 规避 winit 偶发不派发 resize 重绘帧导致的左侧黑块与旧布局残留
        let win_size = ctx.input(|i| i.screen_rect().size());
        if self.last_win_size != Some(win_size) {
            self.last_win_size = Some(win_size);
            self.repaint_after_resize = 6;
        }
        if self.repaint_after_resize > 0 {
            self.repaint_after_resize -= 1;
            ctx.request_repaint();
        }
        // 全局兜底自绘：10 Hz 低频轮询。遮挡/失焦（模态文件对话框关闭、
        // 切窗口回来等）后 winit 偶发不派发重绘帧 → 半屏黑块；自绘保证
        // 最迟 ~100 ms 画面必然刷新（egui 空闲帧开销可忽略）
        ctx.request_repaint_after(std::time::Duration::from_millis(100));

        // ── 拖放工程 json → 直接载入；拖放 .docx/.doc 计算书 → 正文验签；.pdf 正本 → 文件指纹验签 ──
        let dropped_all: Vec<std::path::PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        let jsons: Vec<_> = dropped_all
            .iter()
            .filter(|p| p.extension().map(|e| e == "json").unwrap_or(false))
            .cloned()
            .collect();
        let docs: Vec<_> = dropped_all
            .iter()
            .filter(|p| {
                p.extension()
                    .map(|e| e == "doc" || e == "docx")
                    .unwrap_or(false)
            })
            .cloned()
            .collect();
        let pdfs: Vec<_> = dropped_all
            .iter()
            .filter(|p| p.extension().map(|e| e == "pdf").unwrap_or(false))
            .cloned()
            .collect();
        if let Some(p) = jsons.first() {
            self.project_path = p.to_string_lossy().to_string();
            remember_project_path(&self.project_path);
            self.load();
            changed = true;
        }
        for p in &pdfs {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            self.status = match duckcm_core::generate::verify_pdf_sm3(p) {
                Ok(true) => format!("✅ PDF 正本验真：文件 SM3 与指纹清单一致（{name}）"),
                Ok(false) => format!("❌ PDF 已篡改：文件 SM3 与清单不符（{name}）"),
                Err(e) => format!("⚠ PDF 验真失败（{name}）：{e}"),
            };
            ctx.request_repaint();
        }
        for p in &docs {
            let name = p
                .file_name()
                .map(|n| n.to_string_lossy().into_owned())
                .unwrap_or_default();
            let verdict = (|| -> Result<(), String> {
                // Word 二进制 .doc（docx zip）：抽 word/source.html 重算；
                // 旧版 HTML .doc 直接读文本
                let bytes = std::fs::read(p).map_err(|e| format!("读取失败：{e}"))?;
                let html = if bytes.starts_with(b"PK") {
                    duckcm_core::docx::extract_source_docx(&bytes)
                        .ok_or_else(|| "docx 内未找到验签源（word/source.html）".to_string())?
                } else {
                    String::from_utf8_lossy(&bytes).into_owned()
                };
                match duckcm_core::generate::verify_doc_fingerprint(&html) {
                    Ok(true) => Ok(()),
                    Ok(false) => Err("指纹失配——正文与验签页不一致（可能被改动）".to_string()),
                    Err(e) => Err(e),
                }
            })();
            self.status = match verdict {
                Ok(()) => format!("✅ 验签通过：SM3 指纹与正文一致（{name}）"),
                Err(e) => format!("❌ 验签失败（{name}）：{e}"),
            };
            ctx.request_repaint();
        }

        // ── 顶栏 ───────────────────────────────────────────────
        egui::TopBottomPanel::top("toolbar").show(ctx, |ui| {
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(
                    RichText::new("DuckCM 2")
                        .strong()
                        .size(18.0)
                        .color(theme::ACCENT),
                );
                ui.separator();
                ui.label(text::caption("工程名称"));
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.state.name)
                        .desired_width(180.0)
                        .hint_text("未命名工程"),
                );
                changed |= r.changed();

                ui.separator();
                if ui.button(text::button("新建")).clicked() {
                    self.reset();
                }
                if ui.button(text::button("打开")).clicked() {
                    // 弹窗选择工程文件
                    if let Some(p) = rfd::FileDialog::new()
                        .add_filter("DuckCM 工程", &["json"])
                        .set_directory(std::path::PathBuf::from(
                            self.project_path
                                .rsplit_once(['/', '\\'])
                                .map(|(d, _)| d)
                                .unwrap_or("."),
                        ))
                        .pick_file()
                    {
                        self.project_path = p.to_string_lossy().into_owned();
                        self.load();
                    }
                }
                if ui.button(text::button("保存")).clicked() {
                    // 弹窗选择保存位置（默认当前文件名）
                    let name = self
                        .project_path
                        .rsplit(['/', '\\'])
                        .next()
                        .unwrap_or("未命名工程.json")
                        .to_string();
                    match rfd::FileDialog::new()
                        .add_filter("DuckCM 工程", &["json"])
                        .set_file_name(&name)
                        .save_file()
                    {
                        Some(p) => {
                            self.project_path = p.to_string_lossy().into_owned();
                            self.save();
                        }
                        None => {}
                    }
                }
                ui.separator();
                // 路径框占满剩余宽度（长 UNC 路径也能看全），悬停显示完整路径；
                // 右侧预留「关于 + 更新按钮 + 版本号」整块宽度，避免重叠
                let avail = ui.available_width() - 250.0;
                let r = ui.add(
                    egui::TextEdit::singleline(&mut self.project_path)
                        .desired_width(avail.max(160.0))
                        .hint_text("工程文件路径 (.json)"),
                );
                let r_changed = r.changed();
                let _ = r.on_hover_text(self.project_path.clone());
                changed |= r_changed;

                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button(text::button("关于")).clicked() {
                        self.show_about = true;
                    }
                    // ── 在线更新按钮（GitHub 源 + CDN 加速，状态机驱动）──
                    let (txt, col, enabled, act, tip) = match &self.upd.state {
                        crate::updater::UpdState::Idle => (
                            "检查更新".to_string(),
                            theme::TEXT,
                            true,
                            0u8,
                            "从 GitHub（jsDelivr / gh-proxy CDN 回退）检查新版本".to_string(),
                        ),
                        crate::updater::UpdState::Checking => (
                            "检查中…".to_string(),
                            theme::TEXT_MUTED,
                            false,
                            1,
                            "正在连接更新源".to_string(),
                        ),
                        crate::updater::UpdState::Latest => (
                            "已是最新".to_string(),
                            theme::OK,
                            true,
                            0,
                            "当前已是最新版本，点击可重新检查".to_string(),
                        ),
                        crate::updater::UpdState::Avail { ver, notes } => (
                            format!("可更新 v{ver}"),
                            egui::Color32::from_rgb(255, 176, 80),
                            true,
                            2,
                            if notes.is_empty() {
                                "点击下载新版（CDN 加速）".to_string()
                            } else {
                                notes.clone()
                            },
                        ),
                        crate::updater::UpdState::Downloading(p) => (
                            format!("下载中 {:.0}%", p * 100.0),
                            theme::TEXT_MUTED,
                            false,
                            1,
                            "正在经 CDN 下载更新包".to_string(),
                        ),
                        crate::updater::UpdState::Ready { ver, .. } => (
                            format!("重启更新 v{ver}"),
                            theme::OK,
                            true,
                            3,
                            "下载完成：保存工程后点击，自动替换并重启".to_string(),
                        ),
                        crate::updater::UpdState::Failed(e) => (
                            "检查失败".to_string(),
                            theme::ERR,
                            true,
                            0,
                            format!("{e}——点击重试"),
                        ),
                    };
                    let b = ui.add_enabled(
                        enabled,
                        egui::Button::new(RichText::new(txt).size(11.5).color(col)),
                    );
                    if b.clicked() {
                        match act {
                            0 => self.upd.check(ctx),
                            2 => self.upd.download(ctx),
                            3 => {
                                let _ = self.upd.apply();
                            }
                            _ => {}
                        }
                    }
                    let _ = b.on_hover_text(tip);
                    ui.label(
                        RichText::new(format!("v{}", crate::updater::VERSION))
                            .size(11.0)
                            .color(theme::TEXT_MUTED),
                    );
                });
            });
            ui.add_space(2.0);
        });

        // ── 底部导航：6 个配置画面按钮 + 制热拟合 / 生成 ────────
        egui::TopBottomPanel::bottom("nav").show(ctx, |ui| {
            ui.add_space(6.0);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                for screen in ConfigScreen::ALL {
                    let active = self.screen == Some(screen);
                    let resp = nav_button(ui, screen, active);
                    if resp.clicked() {
                        // 再点一次收起；点其它则切换
                        self.screen = if active { None } else { Some(screen) };
                    }
                }
                // 气候之后：制热拟合 / 生成（原版右下角按钮）
                ui.add_space(10.0);
                if ui.button(text::button("最优应用")).clicked() {
                    // 计算最优值一键应用：机组台数、风机、小窗、湿帘面积全部跟随最优解
                    let w = self.outcome.vent.windows_total_count;
                    let f = self.outcome.vent.fans_required;
                    let t = self.outcome.heat.terminal_count_required;
                    self.state.vent.actual_follow = true;
                    self.state.heat.heater_units_manual = false;
                    self.state.vent.windows_actual = w;
                    self.state.vent.fans_actual = f;
                    self.state.heat.terminal_count = t;
                    self.recalc();
                    // 状态播报用跟随后的实际值（含 10% 冗余与额定湿帘面积）
                    self.status = format!(
                        "最优应用：机组 {:.0} 台 · 风机 {:.0} 台 · 小窗 {:.0} 个 · 末端 {:.0} · 湿帘 {:.1} m²",
                        self.curve.units,
                        self.state.vent.fans_actual,
                        self.state.vent.windows_actual,
                        self.state.heat.terminal_count,
                        self.state.vent.pad_area_actual
                    );
                }
                if ui.button(text::button("生成 DOC")).clicked() {
                    self.gen_reports(false);
                }
                if ui.button(text::button("生成 EN DOC")).clicked() {
                    match duckcm_core::generate::write_report_en(
                        &self.state,
                        &self.outcome,
                        self.curve.units,
                        &self.project_path,
                    ) {
                        Ok(p) => self.status = format!("已生成英文报告：{p}"),
                        Err(e) => self.status = format!("英文报告生成失败: {e}"),
                    }
                }
                if ui.button(text::button("生成 PDF")).clicked() {
                    self.gen_reports(true);
                }
            });
            // ── 独立状态行（与按钮行分行，杜绝重叠）────────────
            ui.add_space(2.0);
            ui.horizontal(|ui| {
                ui.add_space(6.0);
                // 左侧：警告链 → 状态播报 → 计算正常
                if let Some(w) = self.outcome.warnings.first() {
                    let mut label = format!("⚠ {w}");
                    if self.outcome.warnings.len() > 1 {
                        label += &format!(" 等 {} 条", self.outcome.warnings.len());
                    }
                    let avail = (ui.available_width() - 560.0).max(160.0);
                    ui.add_sized(
                        [avail, 16.0],
                        egui::Label::new(
                            RichText::new(label).size(10.5_f32).color(theme::ERR),
                        )
                        .selectable(false)
                        .truncate(),
                    );
                } else if !self.status.is_empty() {
                    let st = self.status.clone();
                    let avail = (ui.available_width() - 560.0).max(160.0);
                    ui.add_sized(
                        [avail, 16.0],
                        egui::Label::new(text::caption(&st)).selectable(false).truncate(),
                    );
                    if ui.small_button("关闭").clicked() {
                        self.status.clear();
                    }
                } else {
                    ui.colored_label(
                        theme::OK,
                        text::caption(&format!("计算正常 · {} µs", self.calc_micros)),
                    );
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    ui.label(text::caption("Version 2.2 · Rust + egui"));
                    ui.separator();
                    ui.add_sized(
                        [360.0, 16.0],
                        egui::Label::new(
                            RichText::new("青岛鑫光正钢结构股份有限公司 · yang.linhao@hotmail.com")
                                .size(10.5_f32)
                                .color(theme::ACCENT),
                        )
                        .selectable(false)
                        .truncate(),
                    );
                });
            });
            ui.add_space(6.0);
        });

        // ── 配置窗口（点底部按钮弹出，同一时刻一个）──────────
        if let Some(screen) = self.screen {
            let mut open = true;
            let mut inner_changed = false;
            let title = format!("{} · {}", screen.label(), screen_hint(screen));
            egui::Window::new(title)
                .open(&mut open)
                .default_width(match screen {
                    ConfigScreen::House => 620.0,
                    ConfigScreen::Climate => 640.0,
                    ConfigScreen::Equip => 500.0,
                    _ => 470.0,
                })
                .default_height(640.0)
                .resizable(true)
                .collapsible(false)
                .show(ctx, |ui| {
                    egui::ScrollArea::vertical()
                        .max_height(740.0)
                        .show(ui, |ui| {
                            ui.add_space(6.0);
                            inner_changed |= match screen {
                                ConfigScreen::House => self.house_panel(ui),
                                ConfigScreen::Heat => {
                                    let c1 = self.heat_panel(ui);
                                    let c2 = self.terminal_panel(ui);
                                    c1 || c2
                                }
                                ConfigScreen::Breed => self.breed_panel(ui),
                                ConfigScreen::Climate => self.climate_panel(ui),
                                ConfigScreen::Equip => self.equip_panel(ui),
                            };
                            ui.add_space(10.0);
                        });
                });
            if !open {
                self.screen = None;
            }
            changed |= inner_changed;
        }

        // ── 中央：仪表盘（复刻原程序布局）───────────────────
        egui::CentralPanel::default().show(ctx, |ui| {
            egui::ScrollArea::vertical().show(ui, |ui| {
                changed |= self.dashboard(ui);
            });
        });

        // ── 关于 ───────────────────────────────────────────────
        if self.show_about {
            egui::Window::new("关于 DuckCM 2")
                .collapsible(false)
                .resizable(false)
                .open(&mut self.show_about)
                .show(ctx, |ui| {
                    ui.label("DuckCM 2 —— 养殖用房屋设备自动化计算系统");
                    ui.add_space(6.0);
                    ui.label("计算内核：Rust（PsychroLib / ASHRAE 2017 空气状态）");
                    ui.label("界面：egui · 即时模式 GUI");
                    ui.add_space(6.0);
                    ui.label(text::caption("相对 C# 版的修正："));
                    ui.label(text::caption(
                        "· 递归搜索改为有界迭代，杜绝栈溢出与 Infinity",
                    ));
                    ui.label(text::caption("· 除零/NaN 一律返回明确错误，不再显示「∞」"));
                    ui.label(text::caption("· 热负荷改用含风机/小窗开口的完整围护系数"));
                    ui.label(text::caption("· 曲线拟合消除逐项舍入累积误差"));
                });
        }

        if changed {
            self.recalc();
            // 输入变更 → 1.5 s 防抖**静默自动保存**：只写 json 的 autoSave 字段，
            // 文件本体保持上次「保存」的内容（载入时可询问采用哪份）
            self.last_change = Some(Instant::now());
        }
        if let Some(t0) = self.last_change {
            if t0.elapsed().as_secs_f32() > 1.5 {
                self.auto_tick();
                self.last_change = None;
            } else {
                ctx.request_repaint_after(std::time::Duration::from_millis(300));
            }
        }

        // ── autoSave 询问弹窗（载入时发现快照与本体不一致）──
        if let Some(a) = self.autosave_prompt.clone() {
            egui::Window::new("发现自动保存快照")
                .id(egui::Id::new("autosave_prompt_win"))
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .show(ctx, |ui| {
                    let at = a["at"].as_str().unwrap_or("未知时间");
                    ui.label(format!(
                        "工程文件中存在 {at} 的自动保存快照，\n与文件保存的参数不一致。采用哪一份？"
                    ));
                    ui.add_space(6.0);
                    ui.horizontal(|ui| {
                        if ui
                            .add(egui::Button::new(
                                egui::RichText::new("加载自动保存").color(theme::OK),
                            ))
                            .clicked()
                        {
                            self.apply_payload(&a);
                            self.sync_pad_fan_base();
                            self.recalc();
                            self.autosave_prompt = None;
                            self.status = format!("已加载自动保存快照（{at}，尚未存为本体）");
                        }
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("用文件保存参数").clicked() {
                                self.autosave_prompt = None;
                                self.status = "已采用文件保存的参数".to_string();
                            }
                        });
                    });
                });
        }

        // ── 启动半透明画面（前景层 Splash，约 2.6 s 淡出）───
        // ── 启动半透明画面（前景层 Splash，约 2.6 s 淡出）───
        self.draw_splash(ctx);
    }
}

// ═══════════════════════════════════════════════════════════════
// 左侧参数面板
// ═══════════════════════════════════════════════════════════════

impl DuckApp {
    /// 卡片容器：白底 + 细边框 + 标题
    fn card(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui)) {
        Frame::new()
            .fill(theme::CARD)
            .stroke(Stroke::new(1.0_f32, theme::BORDER))
            .corner_radius(egui::CornerRadius::same(10))
            .inner_margin(egui::Margin::same(12))
            .show(ui, |ui| {
                ui.horizontal(|ui| {
                    // 标题左侧的主色小竖条
                    egui::Frame::new()
                        .fill(theme::ACCENT)
                        .corner_radius(egui::CornerRadius::same(2))
                        .inner_margin(egui::Margin {
                            left: 0,
                            right: 3,
                            top: 2,
                            bottom: 2,
                        })
                        .show(ui, |ui| {
                            ui.label(RichText::new(" ").size(12.0));
                        });
                    ui.label(text::group_header(title));
                });
                ui.add_space(6.0);
                add(ui);
            });
        ui.add_space(10.0);
    }

    fn house_panel(&mut self, ui: &mut Ui) -> bool {
        let mut changed = false;

        // ── 剖面图（就地放大/还原，不弹窗）───────────────────
        let geo = SectionGeometry {
            span_mm: self.state.house.width * 1000.0,
            length_mm: self.state.house.length * 1000.0,
            eave_mm: self.state.house.eave_height * 1000.0,
            ridge_mm: self.state.house.roof_height * 1000.0,
            ceiling_mm: self.state.house.ceiling_height * 1000.0,
            wall_band_mm: self.state.house.wall_height_mm,
            has_ceiling: self.state.house.has_ceiling,
        };
        let w = ui.available_width().min(540.0);
        let h = if self.section_large { 520.0 } else { 252.0 };
        draw_portal_frame(ui, egui::vec2(w, h), &geo);
        ui.add_space(6.0);

        // ── 子视图切换（就地显示，不弹窗）────────────────────
        ui.horizontal(|ui| {
            let label = if self.section_large {
                "还原剖面图"
            } else {
                "放大剖面图"
            };
            if ui
                .selectable_label(self.section_large, text::button(label))
                .clicked()
            {
                self.section_large = !self.section_large;
            }
            ui.separator();
            let r = ui.selectable_value(
                &mut self.house_view,
                HouseView::Config,
                text::button("配置单"),
            );
            changed |= r.changed();
            let r = ui.selectable_value(
                &mut self.house_view,
                HouseView::WallMaterial,
                text::button("墙板构造"),
            );
            changed |= r.changed();
            let r = ui.selectable_value(
                &mut self.house_view,
                HouseView::RoofMaterial,
                text::button("屋面构造"),
            );
            changed |= r.changed();
        });
        ui.add_space(8.0);

        match self.house_view {
            HouseView::Config => {
                changed |= self.house_config_grid(ui);
            }
            HouseView::WallMaterial => {
                // 构造即口径：U 有变化就同步（无需「应用」按钮；按值比较防抖）
                if let Some(u) = self.wall_editor.ui(ui) {
                    // 任何构造编辑都触发防抖快照（换材不换 U 也要进 autoSave）
                    changed = true;
                    if (self.state.house.wall_u - u).abs() > 1e-6 {
                        self.state.house.wall_u = u;
                        self.status = format!("墙面传热系数 → {u:.3} W/(m²·K)（构造自动应用）");
                    }
                }
            }
            HouseView::RoofMaterial => {
                if let Some(u) = self.roof_editor.ui(ui) {
                    changed = true;
                    if (self.state.house.roof_u - u).abs() > 1e-6 {
                        self.state.house.roof_u = u;
                        self.status = format!("屋面传热系数 → {u:.3} W/(m²·K)（构造自动应用）");
                    }
                }
            }
        }
        changed
    }

    /// 养殖配置画面（对照原 YangZCon）
    fn breed_panel(&mut self, ui: &mut Ui) -> bool {
        let mut changed = false;

        // ── 养殖参数（输入）────────────────────────────────────
        let mut win_pick: Option<(&'static str, f64, f64)> = None;
        Self::card(ui, "养殖参数", |ui| {
            let b = &mut self.state.breed;
            let before = b.breed_type;
            ui.horizontal(|ui| {
                ui.label(text::label("养殖类"));
                ComboBox::from_id_salt("breed_type")
                    .selected_text(breed_label(b.breed_type))
                    .show_ui(ui, |ui| {
                        for t in [
                            BreedType::LayerChicken,
                            BreedType::BroilerChicken,
                            BreedType::LayerDuck,
                            BreedType::BroilerDuck,
                            BreedType::Goose,
                            BreedType::Other,
                        ] {
                            changed |= ui
                                .selectable_value(&mut b.breed_type, t, breed_label(t))
                                .changed();
                        }
                    });
            });
            // 切换种类 → 默认成型体重 + 默认小窗（肉禽 560×270 弹簧窗 / 蛋禽 970×340 弧面窗）
            if before != b.breed_type {
                if let Some(w) = breed_default_weight(b.breed_type) {
                    b.breed_weight = w;
                }
                win_pick = Some(match b.breed_type {
                    // 蛋禽 970×340 弧面窗：有效通风面积 = 洞口 ×0.7
                    BreedType::LayerChicken | BreedType::LayerDuck => ("970×340", 0.1319, 0.97),
                    _ => ("560×270", 0.1512, 0.56),
                });
            }
            changed |= field(ui, "成型体重", "kg", &mut b.breed_weight, 0.0..=100.0, 0.1);
            changed |= field(ui, "养殖数量", "只", &mut b.breed_vol, 0.0..=1e7, 100.0);
            // 最低呼吸量 [m³/h·kg]（原 C02Rvol，按种类/体重自动算，可改）
            changed |= field(
                ui,
                "最低呼吸量",
                "m³/h·kg",
                &mut b.co2_respiration_vol,
                0.0..=1e4,
                0.01,
            );
            ui.label(text::caption("呼吸量由体重与 18°C 温度修正自动估算"));
        });
        // 种类切换的默认小窗写回（肉禽 560×270 / 蛋禽 970×340）
        if let Some((name, area, width)) = win_pick {
            let lbl = breed_label(self.state.breed.breed_type);
            self.state.vent.window_model_name = name.to_string();
            self.state.vent.window_area = area;
            self.state.vent.window_width_m = width;
            self.status = format!("养殖类 → {lbl}（小窗默认 {name}）");
        }

        // ── 派生结果（输入变化即重算，同原 TextChanged）────────
        let (cpc, hx, tz, zs) = {
            let b = &self.state.breed;
            (
                breed_cpc(b.breed_type),
                b.co2_respiration_vol,
                b.breed_weight,
                b.breed_vol,
            )
        };
        let num = hx * tz * zs; // 最低维持风量 [m³/h]
        let need_water = duckcm_core::bio::breed_need_water(zs, tz, cpc).unwrap_or(0.0);
        let atp_kw = duckcm_core::bio::co2_respiration_heat(num, 0.6).unwrap_or(0.0);
        let unit_vent =
            duckcm_core::bio::co2_ventilation_for_ppm(1000.0, hx, 5000.0).unwrap_or(0.0);
        let vent_1000 =
            duckcm_core::bio::co2_ventilation_for_ppm(1000.0, num, 5000.0).unwrap_or(0.0);
        let unit_vent_2000 =
            duckcm_core::bio::co2_ventilation_for_ppm(2000.0, hx, 5000.0).unwrap_or(0.0);
        let vent_2000 =
            duckcm_core::bio::co2_ventilation_for_ppm(2000.0, num, 5000.0).unwrap_or(0.0);
        let day_water = duckcm_core::bio::drinking_water(num).unwrap_or(0.0) * cpc;

        // 写回状态（供通风计算与工程保存使用；1000PPM 风量仅作参考显示）
        {
            let b = &mut self.state.breed;
            b.minimum_ventilation = num;
            b.breed_need_water = need_water;
        }

        Self::card(ui, "通风与饮水", |ui| {
            // 统一三列网格：标签右对齐 | 数值右对齐 | 单位
            let lw = 132.0_f32;
            let vw = 118.0_f32;
            let prow = |ui: &mut Ui, label: &str, val: String, unit: &str, col: Color32| {
                ui.horizontal(|ui| {
                    let (lr, _) =
                        ui.allocate_exact_size(egui::vec2(lw, 20.0), egui::Sense::hover());
                    ui.painter().text(
                        lr.right_center(),
                        Align2::RIGHT_CENTER,
                        label,
                        FontId::proportional(11.5_f32),
                        theme::TEXT_MUTED,
                    );
                    let (vr, _) =
                        ui.allocate_exact_size(egui::vec2(vw, 20.0), egui::Sense::hover());
                    ui.painter().text(
                        vr.right_center(),
                        Align2::RIGHT_CENTER,
                        val,
                        FontId::proportional(12.0_f32),
                        col,
                    );
                    ui.add_space(8.0);
                    ui.label(text::unit(unit));
                });
            };
            prow(
                ui,
                "最低维持风量",
                format!("{num:.2}"),
                "m³/h",
                theme::ACCENT,
            );
            prow(
                ui,
                "最低供水量",
                format!("{need_water:.2}"),
                "m³/h",
                theme::ACCENT,
            );
            prow(ui, "呼吸ATP热", format!("{atp_kw:.1}"), "kW", theme::ERR);
            // 2000PPM / 1000PPM 稀释风量组（原 CO2TFXPPM，5000→目标 ppm）
            ui.add_space(4.0);
            prow(
                ui,
                "2000PPM  风量",
                format!("{vent_2000:.2}"),
                "m³/h",
                theme::ACCENT,
            );
            prow(
                ui,
                "单位通风",
                format!("{unit_vent_2000:.2}"),
                "m³/h·kg",
                theme::ACCENT,
            );
            prow(
                ui,
                "1000PPM  风量",
                format!("{vent_1000:.2}"),
                "m³/h",
                theme::ACCENT,
            );
            prow(
                ui,
                "单位通风",
                format!("{unit_vent:.2}"),
                "m³/h·kg",
                theme::ACCENT,
            );
            prow(
                ui,
                "日供水量",
                format!("{day_water:.1}"),
                "m³/h",
                theme::ERR,
            );
        });
        changed
    }

    /// 供暖配置画面：热源选择（设计温度在仪表盘「供暖参数单」输入）
    fn heat_panel(&mut self, ui: &mut Ui) -> bool {
        let mut changed = false;

        Self::card(ui, "热源选择（机组 / 锅炉）", |ui| {
            let t_design = self.state.heat.min_design_temp;
            let cur_model = self.state.heat.heater_model_name.clone();
            let c = &mut self.curve;
            if c.coeff.is_empty() {
                c.coeff.push(120.0);
            }
            ui.label(
                RichText::new("锅炉拟合系数 0.80 · 空气能拟合系数 0.93（按所选热源自动应用）")
                    .size(10.5_f32)
                    .color(theme::ERR),
            );

            // 热源表：空气能机组（赤焰系列）+ 锅炉；kW 与大卡同时显示
            let mut picked_model: Option<&'static duckcm_core::heat::HeaterModel> = None;
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.add_sized(
                    [120.0, 20.0],
                    egui::Label::new(text::caption("热源")).selectable(false),
                );
                ui.add_sized(
                    [64.0, 20.0],
                    egui::Label::new(text::caption("kW")).selectable(false),
                );
                ui.add_sized(
                    [86.0, 20.0],
                    egui::Label::new(text::caption("大卡kcal/h")).selectable(false),
                );
                ui.add_sized(
                    [58.0, 20.0],
                    egui::Label::new(text::caption("功率kW")).selectable(false),
                );
                ui.add_sized(
                    [42.0, 20.0],
                    egui::Label::new(text::caption("COP")).selectable(false),
                );
                ui.add_sized(
                    [150.0, 20.0],
                    egui::Label::new(text::caption("能耗/h")).selectable(false),
                );
            });
            ui.separator();
            let mut last_hcat = "";
            for m in duckcm_core::heat::heater_db() {
                if m.category != last_hcat {
                    last_hcat = m.category;
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(m.category)
                            .size(10.5_f32)
                            .strong()
                            .color(theme::ACCENT),
                    );
                }
                let sel = m.name == cur_model;
                let power_s = if m.power_kw <= 0.0 {
                    "—".to_string()
                } else {
                    m.power_kw.to_string()
                };
                let cop_s = if m.cop <= 0.0 {
                    "—".to_string()
                } else {
                    m.cop.to_string()
                };
                ui.horizontal(|ui| {
                    if sel {
                        ui.colored_label(theme::ACCENT, egui::RichText::new("●").size(10.0_f32));
                    } else {
                        ui.label("  ");
                    }
                    ui.add_sized(
                        [104.0, 22.0],
                        egui::Label::new(egui::RichText::new(m.name).color(if sel {
                            theme::ACCENT
                        } else {
                            theme::TEXT
                        }))
                        .selectable(false),
                    );
                    ui.add_sized(
                        [64.0, 22.0],
                        egui::Label::new(text::caption(&format!("{}", m.heat_kw)))
                            .selectable(false),
                    );
                    ui.add_sized(
                        [86.0, 22.0],
                        egui::Label::new(text::caption(&format!("{:.0}", m.heat_kw * 860.0)))
                            .selectable(false),
                    );
                    ui.add_sized(
                        [58.0, 22.0],
                        egui::Label::new(text::caption(&power_s)).selectable(false),
                    );
                    ui.add_sized(
                        [42.0, 22.0],
                        egui::Label::new(text::caption(&cop_s)).selectable(false),
                    );
                    ui.add_sized(
                        [150.0, 22.0],
                        egui::Label::new(text::caption(m.energy_note)).selectable(false),
                    );
                    let btn = if sel { "使用中" } else { "选择" };
                    let b = ui.add(
                        egui::Button::new(egui::RichText::new(btn).size(11.0_f32).color(if sel {
                            theme::OK
                        } else {
                            theme::ACCENT
                        }))
                        .min_size(egui::vec2(52.0, 22.0)),
                    );
                    if b.clicked() && !sel {
                        picked_model = Some(m);
                    }
                });
            }
            if let Some(m) = picked_model {
                // 只选设备：单台能力曲线 = 机组三次曲线，拟合系数随类型；
                // 台数回到自动配置（按设计工况热负荷）
                self.curve.coeff = m.poly_coeffs();
                self.curve.fit = if m.name == "锅炉" {
                    0.80
                } else if !m.has_terminal {
                    1.0 // 燃气/燃油热风机：燃烧额定出力直吹，无拟合折扣
                } else {
                    0.93
                };
                self.state.heat.heater_model_name = m.name.to_string();
                self.state.heat.heater_units_manual = false;
                self.status = if m.has_terminal {
                    format!(
                        "热源 → {}（额定制热 {:.0} kW ≈ {:.0} 大卡/台）",
                        m.name,
                        m.heat_kw,
                        m.heat_kw * 860.0
                    )
                } else {
                    format!(
                        "热源 → {} {}（{:.0} kW ≈ {:.0} 大卡 · 直吹式：无末端、不计热网损失）",
                        m.category,
                        m.name,
                        m.heat_kw,
                        m.heat_kw * 860.0
                    )
                };
                changed = true;
            }

            // 当前热源输出（kW + 大卡）
            let single_now = match duckcm_core::heat::find_heater_model(&cur_model) {
                Some(m) => m.capacity_kw(t_design),
                None => duckcm_core::heat::poly_eval(&self.curve.coeff, t_design),
            };
            let fit_now = self.curve.fit;
            let units_now = units_factor(&self.curve);
            let total_now = single_now * units_now * fit_now;
            ui.add_space(4.0);
            ui.horizontal(|ui| {
                ui.label(text::label("单台输出"));
                ui.label(RichText::new(format!("{:.1} kW", single_now)).strong());
                ui.label(text::caption("·"));
                ui.label(RichText::new(format!("{:.0} 大卡", single_now * 860.0)).strong());
            });
            ui.horizontal(|ui| {
                ui.label(text::label("总能力"));
                ui.label(
                    RichText::new(format!("{:.1} kW", total_now))
                        .strong()
                        .color(theme::ACCENT),
                );
                ui.label(text::caption("·"));
                ui.label(
                    RichText::new(format!("{:.0} 大卡", total_now * 860.0))
                        .strong()
                        .color(theme::ACCENT),
                );
                ui.label(text::caption(&format!(
                    "（{t_design:.0}°C × {:.0} 台 × {:.2} 拟合）",
                    units_now, fit_now
                )));
            });
            // 入舍可供 = 总能力 × 热网系数 0.92——需求台数与热源校核按本值；
            // 燃气热风机直吹无热网，不显示本行（总能力即入舍热量）
            if !duckcm_core::heat::heater_no_terminal(&self.state.heat.heater_model_name) {
                ui.horizontal(|ui| {
                    ui.label(text::label("入舍可供"));
                    ui.label(
                        RichText::new(format!("{:.1} kW", total_now * 0.92))
                            .strong()
                            .color(theme::OK),
                    );
                    ui.label(text::caption(
                        "（×0.92 热网系数：机组→末端水路沿程散热，需求/校核同式）",
                    ));
                });
            }
        });

        // 供暖能力（按设计温度）——图表统一在右侧结果区显示，避免重复
        let design_t = self.state.heat.min_design_temp;
        Self::card(
            ui,
            &format!("供暖能力（设计最低工作温度 {design_t:.0}°C 工况）"),
            |ui| {
                let t = self.state.heat.min_design_temp;
                let (cap, cop) = match duckcm_core::heat::find_heater_model(
                    &self.state.heat.heater_model_name,
                ) {
                    Some(m) => (m.capacity_kw(t), m.cop_at(t)),
                    None => (duckcm_core::heat::poly_eval(&self.curve.coeff, t), 0.0),
                };
                row(ui, "单台能力", cap, "kW");
                if cop > 0.0 {
                    ui.horizontal(|ui| {
                        ui.label(text::label("COP（指示）"));
                        ui.label(
                            RichText::new(format!("{cop:.2}"))
                                .strong()
                                .color(theme::ACCENT),
                        );
                        ui.label(text::caption(
                            "＝ 制热量 ÷ 额定输入功率（低温除霜功耗未计入，实际略低）",
                        ));
                    });
                }
                if cap <= 0.0 {
                    ui.colored_label(theme::ERR, text::warn("散热曲线未设置或能力为 0"));
                }
            },
        );
        changed
    }

    /// 末端配置画面：末端散热 + 湿帘降温
    fn terminal_panel(&mut self, ui: &mut Ui) -> bool {
        let mut changed = false;
        // 供暖方式（原 EndCon + RGForm：选一行 → 理论散热系数 = 传热系数 × 面积系数）
        Self::card(ui, "供暖方式（末端库）", |ui| {
            ui.horizontal(|ui| {
                ui.add_sized([110.0, 20.0], egui::Label::new(text::caption("物料编号")));
                ui.add_sized([120.0, 20.0], egui::Label::new(text::caption("名称")));
                ui.add_sized(
                    [110.0, 20.0],
                    egui::Label::new(text::caption("标准散热量kW")),
                );
                ui.add_sized(
                    [130.0, 20.0],
                    egui::Label::new(text::caption("传热系数 W/m²·K")),
                );
                ui.add_sized(
                    [120.0, 20.0],
                    egui::Label::new(text::caption("单位换热面积系数")),
                );
            });
            ui.separator();
            let cur = self.state.heat.terminal_type_name.clone();
            let mut picked: Option<&'static duckcm_core::terminal::TerminalType> = None;
            for t in duckcm_core::terminal::terminal_db() {
                let sel = t.name == cur;
                ui.horizontal(|ui| {
                    if sel {
                        ui.colored_label(theme::ACCENT, egui::RichText::new("●").size(10.0_f32));
                    } else {
                        ui.label("  ");
                    }
                    ui.add_sized(
                        [70.0, 22.0],
                        egui::Label::new(text::caption(&format!("{}", t.id))).selectable(false),
                    );
                    ui.add_sized(
                        [104.0, 22.0],
                        egui::Label::new(egui::RichText::new(&t.name).color(if sel {
                            theme::ACCENT
                        } else {
                            theme::TEXT
                        }))
                        .selectable(false),
                    );
                    ui.add_sized(
                        [96.0, 22.0],
                        egui::Label::new(text::caption(&format!("{}", t.cc_kw))).selectable(false),
                    );
                    ui.add_sized(
                        [116.0, 22.0],
                        egui::Label::new(text::caption(&format!("{}", t.wk_m2))).selectable(false),
                    );
                    ui.add_sized(
                        [104.0, 22.0],
                        egui::Label::new(text::caption(&format!("{}", t.m2))).selectable(false),
                    );
                    let btn = if sel { "使用中" } else { "选择" };
                    let b = ui.add(
                        egui::Button::new(egui::RichText::new(btn).size(11.0_f32).color(if sel {
                            theme::OK
                        } else {
                            theme::ACCENT
                        }))
                        .min_size(egui::vec2(52.0, 22.0)),
                    );
                    if b.clicked() && !sel {
                        picked = Some(t);
                    }
                });
            }
            if let Some(t) = picked {
                let name = t.name.clone();
                let wk = t.conductance();
                self.state.heat.terminal_type_name = name;
                self.state.heat.terminal_unit_conductance = wk;
                self.status = format!("供暖方式 → {}（{:.1} W/K）", t.name, wk);
                changed = true;
            }
        });

        changed
    }

    /// 气候设置画面：气温/墙面材料/辐射/湿度
    fn climate_panel(&mut self, ui: &mut Ui) -> bool {
        let mut changed = false;
        // 墙面表层材料 → 太阳辐射吸收系数（墙面温度自动计算）
        const WALL_SURFACES: &[(&str, f64)] = &duckcm_core::data::WALL_SURFACES;
        Self::card(ui, "气候参数", |ui| {
            let c = &mut self.state.climate;
            changed |= field(ui, "环境温度", "°C", &mut c.outdoor_temp, -30.0..=55.0, 0.5);
            // 墙面材料下拉（决定吸收系数）
            ui.horizontal(|ui| {
                ui.add_sized([120.0, 24.0], egui::Label::new(text::label("墙面材料")));
                egui::ComboBox::from_id_salt("wall_surface_cb")
                    .selected_text(c.wall_surface.clone())
                    .show_ui(ui, |ui| {
                        for &(name, alpha) in WALL_SURFACES {
                            let sel = c.wall_surface == name;
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(format!("{name}（α={alpha:.2}）"))
                                            .size(11.5_f32)
                                            .color(if sel { theme::ACCENT } else { theme::TEXT }),
                                    )
                                    .min_size(egui::vec2(180.0, 20.0)),
                                )
                                .clicked()
                            {
                                c.wall_surface = name.to_string();
                                c.absorptivity = alpha;
                                changed = true;
                            }
                        }
                    });
            });
            // 屋面材料下拉（决定屋面吸收系数——屋面 sol-air 温度）
            ui.horizontal(|ui| {
                ui.add_sized([120.0, 24.0], egui::Label::new(text::label("屋面材料")));
                egui::ComboBox::from_id_salt("roof_surface_cb")
                    .selected_text(c.roof_surface.clone())
                    .show_ui(ui, |ui| {
                        for &(name, alpha) in WALL_SURFACES {
                            let sel = c.roof_surface == name;
                            if ui
                                .add(
                                    egui::Button::new(
                                        RichText::new(format!("{name}（α={alpha:.2}）"))
                                            .size(11.5_f32)
                                            .color(if sel { theme::ACCENT } else { theme::TEXT }),
                                    )
                                    .min_size(egui::vec2(180.0, 20.0)),
                                )
                                .clicked()
                            {
                                c.roof_surface = name.to_string();
                                c.roof_absorptivity = alpha;
                                changed = true;
                            }
                        }
                    });
            });
            changed |= field(
                ui,
                "太阳辐射",
                "kWh/(m²·d)",
                &mut c.solar_radiation,
                0.0..=12.0,
                0.1,
            );
            changed |= field(
                ui,
                "外表面换热系数",
                "W/(m²·K)",
                &mut c.alpha_e,
                5.0..=50.0,
                1.0,
            );
            changed |= field(ui, "相对湿度", "%", &mut c.humidity_pct, 60.0..=93.0, 1.0);
            // 墙面温度（只读自动值）
            let sol_air = duckcm_core::heat::wall_surface_temp(
                c.outdoor_temp,
                c.solar_radiation,
                c.absorptivity,
                c.alpha_e,
            );
            // 吸收系数（只读，随墙面材料自动）——与其他行同网格：label 120 + 框 110 + 单位
            ui.horizontal(|ui| {
                ui.add_sized([120.0, 24.0], egui::Label::new(text::label("吸收系数")));
                let mut a_shown = format!("{:.2}", c.absorptivity);
                ui.add(
                    egui::TextEdit::singleline(&mut a_shown)
                        .desired_width(110.0)
                        .horizontal_align(egui::Align::Center)
                        .text_color(theme::ACCENT),
                );
            });
            ui.horizontal(|ui| {
                ui.add_sized([120.0, 24.0], egui::Label::new(text::label("墙面温度")));
                let mut shown = format!("{sol_air:.1}");
                ui.add(
                    egui::TextEdit::singleline(&mut shown)
                        .desired_width(110.0)
                        .horizontal_align(egui::Align::Center)
                        .text_color(theme::ACCENT),
                );
                ui.label(text::unit("°C"));
            });
            let sol_air_roof = duckcm_core::heat::wall_surface_temp(
                c.outdoor_temp,
                c.solar_radiation,
                c.roof_absorptivity,
                c.alpha_e,
            );
            ui.horizontal(|ui| {
                ui.add_sized([120.0, 24.0], egui::Label::new(text::label("屋面温度")));
                let mut shown = format!("{sol_air_roof:.1}");
                ui.add(
                    egui::TextEdit::singleline(&mut shown)
                        .desired_width(110.0)
                        .horizontal_align(egui::Align::Center)
                        .text_color(theme::ACCENT),
                );
                ui.label(text::unit("°C"));
            });
            ui.label(text::caption(&format!(
                "综合温度 = {:.1} + α·{:.1}·133 / {:.0}（墙面 α={:.2} · 屋面 α={:.2}，随材料与辐射自动计算）",
                c.outdoor_temp, c.solar_radiation, c.alpha_e, c.absorptivity, c.roof_absorptivity
            )));
        });
        changed
    }

    /// 养殖设备画面：风机选型 / 小窗 / 风机（原在气候设置内，现独立成页）
    /// 通风设备配置窗口：风机/小窗选型（湿帘几何与参数在主界面）
    fn equip_panel(&mut self, ui: &mut Ui) -> bool {
        let mut changed = false;
        // 风机型号库：一键选型（风量自动写入单台风机风量）
        Self::card(ui, "风机型号（样本曲线）", |ui| {
            ui.horizontal(|ui| {
                // 表头行首补 ● 位空档，与数据行列对齐
                ui.add_sized([14.0, 20.0], egui::Label::new(" ").selectable(false));
                ui.add_sized(
                    [120.0, 20.0],
                    egui::Label::new(text::caption("型号")).selectable(false),
                );
                ui.add_sized(
                    [62.0, 20.0],
                    egui::Label::new(text::caption("功率kW")).selectable(false),
                );
                ui.add_sized(
                    [98.0, 20.0],
                    egui::Label::new(text::caption("0Pa 风量")).selectable(false),
                );
                ui.add_sized(
                    [104.0, 20.0],
                    egui::Label::new(text::caption("−20Pa 风量")).selectable(false),
                );
            });
            ui.separator();
            let mut picked: Option<&'static duckcm_core::vent::FanModel> = None;
            let mut last_cat: Option<&'static str> = None;
            for m in duckcm_core::vent::fan_db() {
                // 分类小标题；型号列只显示分类前缀之后的型号段
                if last_cat != Some(m.category) {
                    last_cat = Some(m.category);
                    ui.add_space(2.0);
                    ui.label(
                        egui::RichText::new(m.category)
                            .size(10.5_f32)
                            .strong()
                            .color(theme::ACCENT),
                    );
                }
                let short = m.model;
                let sel = m.category == self.state.vent.fan_category
                    && m.model == self.state.vent.fan_model_name;
                ui.horizontal(|ui| {
                    if sel {
                        ui.colored_label(theme::ACCENT, egui::RichText::new("●").size(10.0_f32));
                    } else {
                        ui.label("  ");
                    }
                    ui.add_sized(
                        [120.0, 22.0],
                        egui::Label::new(egui::RichText::new(short).color(if sel {
                            theme::ACCENT
                        } else {
                            theme::TEXT
                        }))
                        .selectable(false),
                    );
                    ui.add_sized(
                        [62.0, 22.0],
                        egui::Label::new(text::caption(&format!("{}", m.power_kw)))
                            .selectable(false),
                    );
                    // 0Pa 自由空气量：有气动曲线的机型按曲线取值
                    let q0 = duckcm_core::vent::aero_curve_for(&format!(
                        "{} {}",
                        m.category, m.model
                    ))
                    .map(|c| c.q_at(0.0))
                    .unwrap_or(m.air_volume_m3h);
                    ui.add_sized(
                        [98.0, 22.0],
                        egui::Label::new(text::caption(&format!("{:.0}", q0))).selectable(false),
                    );
                    ui.add_sized(
                        [104.0, 22.0],
                        egui::Label::new(text::caption(&format!("{}", m.air_volume_m3h)))
                            .selectable(false),
                    );
                    let btn = if sel { "使用中" } else { "选择" };
                    let b = ui.add(
                        egui::Button::new(egui::RichText::new(btn).size(11.0_f32).color(if sel {
                            theme::OK
                        } else {
                            theme::ACCENT
                        }))
                        .min_size(egui::vec2(52.0, 22.0)),
                    );
                    if b.clicked() && !sel {
                        picked = Some(m);
                    }
                });
            }
            if let Some(m) = picked {
                self.state.vent.fan_category = m.category.to_string();
                self.state.vent.fan_model_name = m.model.to_string();
                self.state.vent.fan_unit_air_volume = m.air_volume_m3h;
                self.state.vent.fan_power_kw = m.power_kw;
                self.status = format!(
                    "风机 → {} {}（{:.0} m³/h）",
                    m.category, m.model, m.air_volume_m3h
                );
                changed = true;
            }
            // 遮光罩选项：加 9% 风阻（有效风量 ×0.91，计入可变效率/风机数）
            ui.separator();
            let hood = &mut self.state.vent.fan_light_hood;
            ui.horizontal(|ui| {
                if ui.checkbox(hood, "加遮光罩（风阻 9%，有效风量 ×0.91）").changed() {
                    changed = true;
                    self.status = if *hood {
                        "风机已加遮光罩：风阻 9% 计入工作点".to_string()
                    } else {
                        "已取消遮光罩".to_string()
                    };
                }
            });
            // 计算口径备用台数：n = ⌈L_v ÷ 工作点风量⌉ + 备用
            ui.horizontal(|ui| {
                ui.set_width(300.0);
                let sp = &mut self.state.vent.fans_spare;
                if field(ui, "备用台数", "台", sp, 0.0..=6.0, 1.0) {
                    changed = true;
                    self.status =
                        format!("计算备用台数 → {:.0} 台（风机数 = 运行 + 备用）", *sp);
                }
            });
        });

        // 弹簧进风窗选型（PS 材质，通风面积自动写入小窗单个面积）
        Self::card(ui, "小窗型号（弹簧 / 弧面进风窗）", |ui| {
            ui.horizontal(|ui| {
                ui.add_sized([14.0, 20.0], egui::Label::new(" ").selectable(false));
                ui.add_sized(
                    [120.0, 20.0],
                    egui::Label::new(text::caption("项目")).selectable(false),
                );
                ui.add_sized(
                    [62.0, 20.0],
                    egui::Label::new(text::caption("材质")).selectable(false),
                );
                ui.add_sized(
                    [98.0, 20.0],
                    egui::Label::new(text::caption("开孔 mm")).selectable(false),
                );
                ui.add_sized(
                    [104.0, 20.0],
                    egui::Label::new(text::caption("通风面积m²")).selectable(false),
                );
            });
            ui.separator();
            let cur = self.state.vent.window_model_name.clone();
            let mut picked: Option<&'static duckcm_core::vent::WindowModel> = None;
            for m in duckcm_core::vent::window_db() {
                let sel = m.name == cur;
                let dims = format!("{}×{}", m.w_mm, m.h_mm);
                ui.horizontal(|ui| {
                    if sel {
                        ui.colored_label(theme::ACCENT, egui::RichText::new("●").size(10.0_f32));
                    } else {
                        ui.label("  ");
                    }
                    ui.add_sized(
                        [120.0, 22.0],
                        egui::Label::new(
                            egui::RichText::new(format!("{} {}", m.name, m.kind)).color(if sel {
                                theme::ACCENT
                            } else {
                                theme::TEXT
                            }),
                        )
                        .selectable(false),
                    );
                    ui.add_sized(
                        [62.0, 22.0],
                        egui::Label::new(text::caption(m.material)).selectable(false),
                    );
                    ui.add_sized(
                        [98.0, 22.0],
                        egui::Label::new(text::caption(&dims)).selectable(false),
                    );
                    ui.add_sized(
                        [104.0, 22.0],
                        egui::Label::new(text::caption(&format!("{}", m.vent_area)))
                            .selectable(false),
                    );
                    let btn = if sel { "使用中" } else { "选择" };
                    let b = ui.add(
                        egui::Button::new(egui::RichText::new(btn).size(11.0_f32).color(if sel {
                            theme::OK
                        } else {
                            theme::ACCENT
                        }))
                        .min_size(egui::vec2(52.0, 22.0)),
                    );
                    if b.clicked() && !sel {
                        picked = Some(m);
                    }
                });
            }
            if let Some(m) = picked {
                self.state.vent.window_model_name = m.name.to_string();
                self.state.vent.window_area = m.vent_area;
                self.state.vent.window_width_m = m.w_mm as f64 / 1000.0;
                self.status = format!(
                    "小窗型号 → {} {}（通风面积 {} m²）",
                    m.name, m.material, m.vent_area
                );
                changed = true;
            }
            // 小窗间距校核（洞口净间距 = 中心距 − 窗宽，最小 1.2 m）
            let sp = self.window_spacing;
            let per_side = (self.state.vent.windows_actual / 2.0).ceil().max(1.0);
            let pitch = self.state.house.length / per_side;
            ui.add_space(4.0);
            ui.label(
                RichText::new(format!(
                    "间距校核：中心距 {pitch:.2} − 窗宽 = 净间距 {sp:.2} m（1.2 ~ 2.0 m）· 每侧应布 {:.0} ~ {:.0} 个（共 {:.0} ~ {:.0}）",
                    self.window_min_per_side,
                    self.window_max_per_side,
                    self.window_min_per_side * 2.0,
                    self.window_max_per_side * 2.0
                ))
                .size(10.0_f32)
                .color(if sp >= 1.2 - 1e-6 && sp <= 2.0 + 1e-6 {
                    theme::OK
                } else {
                    theme::ERR
                }),
            );
            // 射流传导校核：洞口风速按「舍中轴线 0.6 m/s」反解
            let x_need = self.state.house.width / 2.0;
            if let (Some(v_auto), Some(v_center)) = (
                duckcm_core::vent::reynolds_window_speed(self.state.vent.window_area, x_need),
                duckcm_core::vent::jet_axis_velocity(
                    self.state.vent.window_area,
                    self.state.vent.window_speed,
                    x_need,
                ),
            ) {
                let ok = v_center >= 0.6 - 1e-3;
                ui.label(
                    RichText::new(format!(
                        "射流校核：舍中轴速 {v_center:.2} m/s（需 0.60，自动解洞口 {v_auto:.2}，现 {:.2}）",
                        self.state.vent.window_speed
                    ))
                    .size(10.0_f32)
                    .color(if ok { theme::OK } else { theme::ERR }),
                );
            }
        });
        changed
    }
    /// 风机 P-Q 曲线小图：厂商样本曲线、−20 Pa 额定基准点（绿）、
    /// 系统阻力工作点（红，含遮光罩衰减）——仅曲线机型显示
    fn fan_curve_mini(
        ui: &mut Ui,
        state: &ProjectState,
        out: &duckcm_core::CalcOutcome,
        _h: f32,
    ) {
        let full = format!("{} {}", state.vent.fan_category, state.vent.fan_model_name);
        let Some(curve) = duckcm_core::vent::aero_curve_for(&full) else {
            return;
        };
        let pts = curve.0;
        if pts.len() < 2 {
            return;
        }
        let vt = &out.vent;
        let hood_f = if state.vent.fan_light_hood { 0.91 } else { 1.0 };
        // 手动改「−20Pa校核点通风量」= 自定义风机：曲线按新基准线性缩放
        // （形状/效率特性不变），图示与引擎同口径
        let q20_ref = curve.q_at(20.0);
        let scale = if q20_ref > 0.0 {
            state.vent.fan_unit_air_volume / q20_ref
        } else {
            1.0
        };
        let dp_max = pts[pts.len() - 1].0.max(30.0);
        let q_max = pts.iter().map(|p| p.1).fold(1.0_f64, f64::max) * scale * 1.04;
        let w = ui.available_width().max(150.0);
        // 标题行（居中），图形整体在其下方
        ui.with_layout(egui::Layout::top_down(egui::Align::Center), |ui| {
            ui.label(
                RichText::new("P-Q 曲线 · 工作点（千m³/h — Pa）")
                    .size(10.0_f32)
                    .color(theme::TEXT_MUTED),
            );
        });
        // 用尽左列剩余高度（不留底部空白），边距收紧让图形向左下延展
        let h_plot = (ui.available_height() - 6.0).max(56.0);
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h_plot), egui::Sense::hover());
        let plot = egui::Rect::from_min_max(
            egui::pos2(rect.min.x + 8.0, rect.min.y + 6.0),
            egui::pos2(rect.max.x - 2.0, rect.max.y - 14.0),
        );
        let map = |dp: f64, q: f64| {
            egui::pos2(
                plot.min.x + (dp / dp_max) as f32 * plot.width(),
                plot.max.y - (q / q_max) as f32 * plot.height(),
            )
        };
        let p = ui.painter();
        // ── 网格 + 刻度（x: Pa，y: m³/h，nice-step）──
        let nice = |span: f64, want: f64| -> f64 {
            let raw = span / want;
            let mag = 10f64.powf(raw.log10().floor());
            [1.0, 2.0, 2.5, 5.0, 10.0]
                .into_iter()
                .map(|m| m * mag)
                .find(|s| *s >= raw)
                .unwrap_or(10.0 * mag)
        };
        let xs = nice(dp_max, 4.0);
        let mut dp = xs;
        while dp <= dp_max + 1e-6 {
            let gx = map(dp, 0.0).x;
            p.line_segment(
                [egui::pos2(gx, plot.min.y), egui::pos2(gx, plot.max.y)],
                Stroke::new(0.5_f32, theme::BORDER),
            );
            p.text(
                egui::pos2(gx, plot.max.y + 2.0),
                Align2::CENTER_TOP,
                format!("{:.0}", dp),
                FontId::proportional(8.5),
                theme::TEXT_MUTED,
            );
            dp += xs;
        }
        let ys = nice(q_max, 4.0);
        let mut qy = ys;
        while qy <= q_max + 1e-6 {
            let gy = map(0.0, qy).y;
            p.line_segment(
                [egui::pos2(plot.min.x, gy), egui::pos2(plot.max.x, gy)],
                Stroke::new(0.5_f32, theme::BORDER),
            );
            p.text(
                egui::pos2(plot.min.x + 2.0, gy - 9.0),
                Align2::LEFT_TOP,
                format!("{:.0}", qy / 1000.0),
                FontId::proportional(8.5),
                theme::TEXT_MUTED,
            );
            qy += ys;
        }
        p.text(
            plot.right_bottom() + egui::vec2(0.0, 10.0),
            Align2::RIGHT_TOP,
            "Pa",
            FontId::proportional(8.5),
            theme::TEXT_MUTED,
        );
        p.line_segment(
            [plot.left_bottom(), plot.right_bottom()],
            Stroke::new(1.0_f32, theme::BORDER),
        );
        p.line_segment(
            [plot.left_bottom(), plot.left_top()],
            Stroke::new(1.0_f32, theme::BORDER),
        );
        // Catmull–Rom 三次样条：曲线精确经过每个样本点、切向连续
        // （比单调限幅版更顺滑；P-Q 样本单调缓变，无过冲风险）
        let n = pts.len();
        let dk: Vec<f64> =
            (0..n - 1).map(|i| (pts[i + 1].1 - pts[i].1) / (pts[i + 1].0 - pts[i].0)).collect();
        let mut m = vec![0.0_f64; n];
        for i in 1..n - 1 {
            // 非均匀参数化中心差商（权重按邻段长度）
            let (h0, h1) = (pts[i].0 - pts[i - 1].0, pts[i + 1].0 - pts[i].0);
            m[i] = (dk[i - 1] * h1 + dk[i] * h0) / (h0 + h1);
        }
        m[0] = dk[0];
        m[n - 1] = dk[n - 2];
        let interp = |dp: f64| -> f64 {
            if dp <= pts[0].0 {
                return pts[0].1;
            }
            if dp >= pts[n - 1].0 {
                return pts[n - 1].1;
            }
            let seg = (1..n).find(|&i| dp <= pts[i].0).unwrap_or(n - 1);
            let (x0, y0) = pts[seg - 1];
            let (x1, y1) = pts[seg];
            let h = x1 - x0;
            let t = ((dp - x0) / h).clamp(0.0, 1.0);
            let t2 = t * t;
            let t3 = t2 * t;
            let y = (2.0 * t3 - 3.0 * t2 + 1.0) * y0
                + (t3 - 2.0 * t2 + t) * h * m[seg - 1]
                + (-2.0 * t3 + 3.0 * t2) * y1
                + (t3 - t2) * h * m[seg];
            y.max(0.0)
        };
        let seg_steps = 24usize;
        let mut prev = map(pts[0].0, pts[0].1 * scale);
        for sgm in 0..n - 1 {
            for i in 1..=seg_steps {
                let dp =
                    pts[sgm].0 + (pts[sgm + 1].0 - pts[sgm].0) * i as f64 / seg_steps as f64;
                let cur = map(dp, interp(dp) * scale);
                p.line_segment([prev, cur], Stroke::new(1.7_f32, theme::ACCENT));
                prev = cur;
            }
        }
        for &(dp, q) in pts {
            p.circle(map(dp, q * scale), 2.0, theme::ACCENT, Stroke::NONE);
        }
        // −20 Pa 额定基准点（绿，在曲线上）
        let m20 = map(20.0, interp(20.0) * scale);
        p.circle(m20, 3.2, theme::OK, Stroke::new(1.0_f32, theme::CARD));
        // 工作点（系统静压，红实心，在曲线上）；加遮光罩时下方空心点
        // = 有效风量 ×0.91，标签显示计入罩后的有效值
        let dps = vt.dp_system_pa.min(dp_max);
        let mop = map(dps, interp(dps) * scale);
        p.line_segment([mop, egui::pos2(mop.x, plot.max.y)], Stroke::new(1.0_f32, theme::BORDER));
        p.circle_filled(mop, 3.8, theme::ERR);
        if hood_f < 1.0 {
            let meff = map(dps, interp(dps) * scale * hood_f);
            p.line_segment([mop, meff], Stroke::new(1.0_f32, theme::ERR));
            p.circle(meff, 3.2, theme::CARD, Stroke::new(1.6_f32, theme::ERR));
            p.text(
                meff + egui::vec2(5.0, -7.0),
                Align2::LEFT_BOTTOM,
                format!("{:.0}（罩）", vt.fan_q_operating),
                FontId::proportional(9.5),
                theme::ERR,
            );
        } else {
            p.text(
                mop + egui::vec2(5.0, -8.0),
                Align2::LEFT_BOTTOM,
                format!("{:.0}", vt.fan_q_operating),
                FontId::proportional(9.5),
                theme::ERR,
            );
        }
    }

    /// 山墙风机布置立面：1380 规格风机 · 间距 300 · 距边 ≥750 · 对称居中
    /// 自动核算该面墙最多能装多少台，与需求台数比对
    fn pad_gable_view(ui: &mut Ui, state: &ProjectState, vh: f32) {
        let b = state.house.width.max(6.0) as f32; // 山墙宽 [m]
        let eave = state.house.eave_height.max(3.0) as f32;
        let ridge = (state.house.roof_height.max(state.house.eave_height + 0.3)) as f32;
        let wall_base = state.house.wall_height_mm.max(0.0) as f32 / 1000.0; // 墙下基础
        let fans_need = state.vent.fans_actual.round() as i32;
        // ── 自动满布山墙（移植 CPQ FanCableCalculator.computeAutoRows）──
        // 从维护高起逐排向上到屋脊；三角区按排顶标高用三角形相似求可用宽度，
        // 每排水平居中；左右留边 500，风机 1.4 m 留洞、间距 300。
        const FAN_MM: i64 = 1400;
        const GAP_MM: i64 = 300;
        const SIDE_MARGIN_MM: i64 = 500;
        let b_mm = (b as f64 * 1000.0).round() as i64;
        let eave_mm = (eave as f64 * 1000.0).round() as i64;
        let wall_mm = (wall_base as f64 * 1000.0).round() as i64;
        // 屋脊：坡度 1/6（坡高 = 宽/12），向上取一位小数（同 CPQ autoRoof）
        let ridge_mm = {
            let auto = ((eave_mm as f64 + b_mm as f64 / 12.0 - 1e-9) * 10.0).ceil() / 10.0 * 1000.0;
            ((ridge as f64 * 1000.0).round() as i64)
                .min(auto.round() as i64)
                .max(eave_mm + 1)
        };
        let pitch = FAN_MM + GAP_MM;
        let gable_h = ridge_mm - eave_mm;
        // 排顶标高 y 处山墙可用宽度（三角形相似收窄）
        let wall_w_at = |y: i64| -> i64 {
            if gable_h > 100 && y > eave_mm {
                b_mm * (ridge_mm - y).max(0) / gable_h
            } else {
                b_mm
            }
        };
        // 逐排计算列数（行顶不超屋脊，宽度不足即停）
        let mut row_cols: Vec<i64> = Vec::new();
        {
            let mut yb = wall_mm;
            loop {
                let yt = yb + FAN_MM;
                if yt >= ridge_mm {
                    break;
                }
                let cols = ((wall_w_at(yt) - 2 * SIDE_MARGIN_MM).max(0) + GAP_MM) / pitch;
                if cols <= 0 {
                    break;
                }
                row_cols.push(cols);
                yb += pitch;
                if row_cols.len() > 12 {
                    break;
                }
            }
        }
        let cap: i64 = row_cols.iter().sum();
        let fan = FAN_MM as f32 / 1000.0;

        let w = ui.available_width().max(110.0) - 12.0;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, vh), egui::Sense::hover());
        let p = ui.painter();
        let area = rect.shrink(10.0);
        // 以总高（檐口+屋顶三角）定标
        let total_h = ridge + 0.2_f32;
        let s = (area.width() / b).min(area.height() / total_h);
        let dw = b * s;
        let cx = area.center().x;
        let bottom = area.max.y - 12.0;
        let wall_top = bottom - eave * s;
        let ridge_y = bottom - ridge * s;

        // 屋面（双坡三角）
        p.add(egui::Shape::convex_polygon(
            vec![
                egui::pos2(cx - dw / 2.0 - 4.0, wall_top),
                egui::pos2(cx, ridge_y),
                egui::pos2(cx + dw / 2.0 + 4.0, wall_top),
            ],
            Color32::from_rgb(190, 200, 210),
            egui::Stroke::new(1.0_f32, theme::TEXT_MUTED),
        ));
        // 山墙墙面
        p.rect_filled(
            egui::Rect::from_min_max(
                egui::pos2(cx - dw / 2.0, wall_top),
                egui::pos2(cx + dw / 2.0, bottom),
            ),
            0.0,
            Color32::from_rgb(246, 249, 252),
        );
        p.rect_stroke(
            egui::Rect::from_min_max(
                egui::pos2(cx - dw / 2.0, wall_top),
                egui::pos2(cx + dw / 2.0, bottom),
            ),
            0.0,
            egui::Stroke::new(1.5_f32, theme::TEXT_MUTED),
            egui::StrokeKind::Middle,
        );
        // 基础（挡水墙）
        if wall_base > 0.05 {
            p.rect_filled(
                egui::Rect::from_min_max(
                    egui::pos2(cx - dw / 2.0, bottom - wall_base * s),
                    egui::pos2(cx + dw / 2.0, bottom),
                ),
                0.0,
                Color32::from_rgb(210, 214, 220),
            );
        }

        // 风机：逐排绘制，每排数量随三角区收窄、各自水平居中
        for (r, &n) in row_cols.iter().enumerate() {
            let total_w = n * pitch - GAP_MM; // mm
            let x0 = (b_mm - total_w) / 2; // 本排居中
            let top_m = (wall_mm + r as i64 * pitch + FAN_MM) as f32 / 1000.0;
            let fy = bottom - top_m * s;
            for c in 0..n {
                let fx = cx - dw / 2.0 + (x0 + c * pitch) as f32 / 1000.0 * s;
                p.rect_filled(
                    egui::Rect::from_min_size(egui::pos2(fx, fy), egui::vec2(fan * s, fan * s)),
                    2.0,
                    Color32::from_rgb(228, 232, 238),
                );
                p.rect_stroke(
                    egui::Rect::from_min_size(egui::pos2(fx, fy), egui::vec2(fan * s, fan * s)),
                    2.0,
                    egui::Stroke::new(1.0_f32, theme::ERR),
                    egui::StrokeKind::Middle,
                );
                p.circle_filled(
                    egui::pos2(fx + fan * s / 2.0, fy + fan * s / 2.0),
                    (fan * s * 0.28).max(1.5),
                    theme::ERR,
                );
            }
        }

        // 标注（两行，避免窄屏超框）
        let over = (fans_need as i64) > cap;
        let col = if over { theme::ERR } else { theme::OK };
        p.text(
            egui::pos2(rect.center().x, rect.max.y - 17.0),
            Align2::CENTER_BOTTOM,
            format!(
                "山墙 {b:.0} m × 檐口 {eave:.1} m · 屋脊 {ridge:.1} m · 风机 1.4 m · 间距 0.30 · 留边 0.50"
            ),
            FontId::proportional(10.5),
            theme::TEXT_MUTED,
        );
        p.text(
            egui::pos2(rect.center().x, rect.max.y - 4.0),
            Align2::CENTER_BOTTOM,
            format!("{} 排共 {cap} 台（需求 {fans_need} 台）", row_cols.len(),),
            FontId::proportional(10.5),
            col,
        );
    }

    /// 保温构造层摘要（计算书用）：「75mm 玻璃丝棉 + 50mm XPS」
    fn layers_desc(ed: &crate::material_ui::LayerEditor) -> String {
        ed.layers
            .iter()
            .map(|l| format!("{:.0}mm {}", l.thickness_mm, l.name))
            .collect::<Vec<_>>()
            .join(" + ")
    }

    /// 设备布置示意图：按当前装机数自动绘制的鸭舍俯视图
    fn pad_layout_view(ui: &mut Ui, state: &ProjectState, vh: f32) {
        let l = state.house.length.max(1.0) as f32;
        let b = state.house.width.max(1.0) as f32;
        let eave = state.house.eave_height.max(1.0);
        let pad_area = state.vent.pad_area_actual;
        let pad_h = state.vent.pad_side_height.max(0.3);
        let walls = state.vent.pad_layout_walls.round().clamp(2.0, 3.0) as i32;
        let fans = state.vent.fans_actual.round().max(0.0) as i32;
        let wins = state.vent.windows_actual.round().max(0.0) as i32;
        // 山墙湿帘规则：帘高 2.5 m、宽 = 山墙宽 × 0.8；檐口每满 3.5 m 加一道
        let gable_h = state.vent.pad_gable_height.max(0.3);
        let gable_tiers = if state.vent.pad_layers >= 1.0 {
            state.vent.pad_layers.floor()
        } else {
            (eave / 3.5).floor().max(1.0)
        };
        let gable_w = ((b as f64 * 0.8) / 0.6).ceil() * 0.6; // 山墙帘宽（600 模块取整）
        let gable_area = if walls >= 3 {
            gable_w * gable_h * gable_tiers
        } else {
            0.0
        };

        let w = ui.available_width().max(110.0) - 12.0;
        let h = vh;
        let (rect, _) = ui.allocate_exact_size(egui::vec2(w, h), egui::Sense::hover());
        let p = ui.painter();

        // 绘图区（留出标注空间；窄时收缩标注边距，防溢出卡片）
        let area = rect.shrink(((w * 0.10) as f32).clamp(12.0, 30.0));
        let s = (area.width() / l).min(area.height() / b);
        let dw = l * s;
        let dh = b * s;
        let c = area.center();
        let shed = egui::Rect::from_center_size(c, egui::vec2(dw, dh));

        // 鸭舍轮廓
        p.rect_filled(shed, 2.0, Color32::from_rgb(246, 249, 252));
        p.rect_stroke(
            shed,
            2.0,
            egui::Stroke::new(1.5_f32, theme::TEXT_MUTED),
            egui::StrokeKind::Middle,
        );
        // 中央尺寸标注：仅在两端文字放得下时绘制，避免与「前侧进风/后侧风机」叠字
        if dw > 250.0 {
            p.text(
                shed.center(),
                Align2::CENTER_CENTER,
                format!("{l:.1} × {b:.1} m"),
                FontId::proportional(11.0),
                theme::TEXT_MUTED,
            );
        }

        // 长墙湿帘（上下，前侧进风面）：面积扣除山墙后均分两长墙，
        // 每面 = 帘长 × 帘高 × 层数；帘长按单块 600 mm 向上取整
        let side_layers = state.vent.pad_side_layers.max(1.0);
        let long_area = (pad_area - gable_area).max(0.0);
        let per_len =
            ((((long_area / 2.0) / pad_h.max(0.3) / side_layers) / 0.6).ceil() * 0.6) as f32; // 每面长墙帘长 [m]（模块取整）
        let pad_t = 5.0_f32; // 图上帘厚（像素）
        let pad_col = Color32::from_rgba_premultiplied(56, 132, 255, 210);
        if walls >= 2 {
            let len = per_len.min(l - 2.0).max(0.0);
            for top in [false, true] {
                let x0 = shed.min.x + 8.0;
                let y = if top { shed.min.y - pad_t } else { shed.max.y };
                p.rect_filled(
                    egui::Rect::from_min_max(
                        egui::pos2(x0, y),
                        egui::pos2(x0 + len * s, y + pad_t),
                    ),
                    1.0,
                    pad_col,
                );
            }
        }
        // 前侧山墙湿帘：宽 = 山墙宽 × 0.8（居中），标注道数
        if walls >= 3 {
            let gw = b * 0.8;
            let y0 = shed.center().y - (gw * s) / 2.0;
            let x = shed.min.x - pad_t;
            p.rect_filled(
                egui::Rect::from_min_max(egui::pos2(x, y0), egui::pos2(x + pad_t, y0 + gw * s)),
                1.0,
                pad_col,
            );
        }
        let mut label = format!(
            "侧墙帘 {:.1} m × {:.1} m × 2 面 × {:.0} 层",
            per_len, pad_h, side_layers
        );
        if walls >= 3 {
            label += &format!(
                " ＋ 山墙帘 {:.1} m × {:.1} m × {:.0} 道",
                gable_w as f32, gable_h as f32, gable_tiers
            );
        }
        // 窄屏时湿帘标注拆两行（一行 = 汇总），避免超出卡片
        let label_total = format!("＝ {:.1} m²", pad_area);
        let split = label.chars().count() > 30 && w < 430.0;
        let (label_top, label_bot) = if split {
            match label.find(" ＋ 山墙帘") {
                Some(i) => (&label[..i], label[i..].to_string()),
                None => (label.as_str(), String::new()),
            }
        } else {
            (label.as_str(), String::new())
        };
        p.text(
            egui::pos2(shed.center().x, shed.max.y + 14.0),
            Align2::CENTER_TOP,
            format!("{label_top} {label_total}"),
            FontId::proportional(10.5),
            theme::ACCENT,
        );
        if !label_bot.is_empty() {
            p.text(
                egui::pos2(shed.center().x, shed.max.y + 25.0),
                Align2::CENTER_TOP,
                label_bot.trim_start(),
                FontId::proportional(10.5),
                theme::ACCENT,
            );
        }

        // 风机：全部集中于后侧山墙（推拉式），前侧进风——标注置于舍内两侧端头
        if fans > 0 {
            let x = shed.max.x;
            let n = fans as usize;
            for i in 0..n {
                let y = shed.min.y + (i as f32 + 1.0) / (n as f32 + 1.0) * dh;
                p.circle_filled(egui::pos2(x, y), 3.5, theme::ERR);
            }
            p.text(
                egui::pos2(shed.max.x - 8.0, shed.center().y),
                Align2::RIGHT_CENTER,
                format!("后侧·风机×{fans}"),
                FontId::proportional(10.0),
                theme::ERR,
            );
            p.text(
                egui::pos2(shed.min.x + 8.0, shed.center().y),
                Align2::LEFT_CENTER,
                "前侧·进风",
                FontId::proportional(10.0),
                theme::TEXT_MUTED,
            );
        }

        // 小窗：均分上下长墙
        let wu = (wins + 1) / 2;
        let wd = wins / 2;
        for (top, n) in [(true, wu), (false, wd)] {
            let y = if top { shed.min.y } else { shed.max.y };
            for i in 0..n {
                let x = shed.min.x + (i as f32 + 1.0) / (n as f32 + 1.0) * dw;
                p.rect_filled(
                    egui::Rect::from_center_size(egui::pos2(x, y), egui::vec2(6.0, 2.5)),
                    1.0,
                    theme::OK,
                );
            }
        }
        if wins > 0 {
            p.text(
                egui::pos2(shed.center().x, shed.min.y - 14.0),
                Align2::CENTER_BOTTOM,
                format!("小窗 {wins} 个（均布两侧长墙 · 绿）"),
                FontId::proportional(10.0),
                theme::OK,
            );
        }
    }

    /// 配置单网格（房屋几何 + 传热系数）
    fn house_config_grid(&mut self, ui: &mut Ui) -> bool {
        let mut changed = false;
        let h = &mut self.state.house;
        egui::Grid::new("house_config_grid")
            .num_columns(3)
            .spacing([12.0, 6.0])
            .show(ui, |ui| {
                // 几何成对排布：舍长·舍宽·檐口 → 屋顶·吊顶·围护 → 传热系数与吊顶
                changed |= grid_field(ui, "舍长", "m", &mut h.length, 0.0..=500.0, 0.5);
                changed |= grid_field(ui, "舍宽", "m", &mut h.width, 0.0..=100.0, 0.5);
                changed |= grid_field(ui, "檐口高度", "m", &mut h.eave_height, 0.0..=30.0, 0.1);
                ui.end_row();
                changed |= grid_field(ui, "屋顶高度", "m", &mut h.roof_height, 0.0..=30.0, 0.1);
                changed |= grid_field(ui, "吊顶高度", "m", &mut h.ceiling_height, 0.0..=30.0, 0.1);
                changed |= grid_field(
                    ui,
                    "围护高度",
                    "mm",
                    &mut h.wall_height_mm,
                    0.0..=20000.0,
                    50.0,
                );
                ui.end_row();
                changed |= grid_field(
                    ui,
                    "墙面传热系数",
                    "W/(m²·K)",
                    &mut h.wall_u,
                    0.0..=10.0,
                    0.01,
                );
                changed |= grid_field(
                    ui,
                    "屋面传热系数",
                    "W/(m²·K)",
                    &mut h.roof_u,
                    0.0..=10.0,
                    0.01,
                );
                ui.horizontal(|ui| {
                    ui.label(text::label("内部吊顶"));
                    let r = ui.selectable_value(&mut h.has_ceiling, false, "无");
                    changed |= r.changed();
                    let r = ui.selectable_value(&mut h.has_ceiling, true, "有");
                    changed |= r.changed();
                });
                ui.end_row();
            });
        changed
    }
}

// ═══════════════════════════════════════════════════════════════
// 右侧结果
// ═══════════════════════════════════════════════════════════════

impl DuckApp {
    /// 主仪表盘（复刻原程序布局）
    ///
    /// ```text
    /// 第一行：房屋参数单 | 房间温度曲线（制热拟合） | 湿帘参数单
    /// 第二行：供暖参数单 | 末端参数单 | 通风参数单 | 实际应用
    /// ```
    fn dashboard(&mut self, ui: &mut Ui) -> bool {
        let mut changed = false;
        ui.add_space(6.0);

        let series = crate::chart::compute_series(&self.state, &self.curve);

        // 行高按窗口高度自适应：第一行（含图表）约 58 %
        let avail = (ui.ctx().screen_rect().height() - 158.0).max(620.0);
        let row1 = (avail * 0.56).clamp(340.0, 540.0);
        let row2 = (avail - row1 - 10.0).clamp(280.0, 430.0);

        // 第一行：左窄卡（房屋）+ 中间图表 + 右卡（湿帘参数单）
        // 高屏（>930px）时右卡双列（主参数 | 降温指标）加宽；否则单列窄卡
        let show_readout = ui.ctx().screen_rect().height() > 930.0;
        ui.horizontal_top(|ui| {
            let full = ui.available_width();
            let side = 258.0_f32.min(full * 0.22);
            let side_r = if show_readout {
                540.0_f32.min(full * 0.38)
            } else {
                258.0_f32.min(full * 0.24)
            };
            let center = (full - side - side_r - 18.0).max(320.0);
            let layout = egui::Layout::top_down(egui::Align::Min);

            let (r1, _) = ui.allocate_exact_size(egui::vec2(side, row1), egui::Sense::hover());
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r1).layout(layout));
            changed |= self.house_table(&mut child, row1);

            let (r2, _) = ui.allocate_exact_size(egui::vec2(center, row1), egui::Sense::hover());
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r2).layout(layout));
            chart_card(&mut child, &series, row1);

            let (r3, _) = ui.allocate_exact_size(egui::vec2(side_r, row1), egui::Sense::hover());
            let mut child = ui.new_child(egui::UiBuilder::new().max_rect(r3).layout(layout));
            changed |= self.pad_table(&mut child, row1, show_readout);
        });

        // 第二行：卡片宽度按文字内容自动收紧（不必铺满）
        ui.add_space(10.0);
        ui.horizontal_top(|ui| {
            let layout = egui::Layout::top_down(egui::Align::Min);
            const HEAT_L: [&str; 7] = [
                "供暖能力(-10°C)",
                "所需机组",
                "最低养殖环境温度",
                "房屋热负",
                "运行舍温(热网平衡)",
                "可达温度(供水50°C)",
                "养殖需求舍内温度",
            ];
            const TERM_L: [&str; 3] = ["末端散热能力", "供回水平均温度", "末端总能力"];
            const VENT_L: [&str; 10] = [
                "最低通风量",
                "小窗面积",
                "最低小窗数量",
                "风速(0.8-3)",
                "空间修正小窗",
                "计算小窗总数",
                "房屋冷负荷",
                "生物冷负荷",
                "设备冷负荷",
                "基础总冷负",
            ];
            const ACT_L: [&str; 5] = ["供暖机组数", "小窗数量", "风机数量", "末端数量", "湿帘面积"];
            let cards: [(f32, fn(&mut DuckApp, &mut Ui, f32) -> bool); 3] = [
                (compact_w(&HEAT_L).max(compact_w(&TERM_L)), Self::heat_table),
                (compact_w(&VENT_L), Self::vent_table),
                (compact_w(&ACT_L) + 52.0, Self::actual_table),
            ];
            for (w, make) in cards {
                let (rc, _) = ui.allocate_exact_size(egui::vec2(w, row2), egui::Sense::hover());
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rc).layout(layout));
                changed |= make(self, &mut child, row2);
            }
            // 第五卡：布置图（占剩余宽度）
            let w_layout = ui.available_width() - 16.0;
            if w_layout > 210.0 {
                let (rc, _) =
                    ui.allocate_exact_size(egui::vec2(w_layout, row2), egui::Sense::hover());
                let mut child = ui.new_child(egui::UiBuilder::new().max_rect(rc).layout(layout));
                dash_card(
                    &mut child,
                    "湿帘 / 风机 / 小窗布置（俯视 · 山墙立面）",
                    row2,
                    |ui| {
                        // 布置墙面：在外层直接配置
                        ui.horizontal(|ui| {
                            ui.label(text::caption("布置墙面"));
                            let w2 = (self.state.vent.pad_layout_walls - 2.0).abs() < 0.01;
                            let w3 = (self.state.vent.pad_layout_walls - 3.0).abs() < 0.01;
                            if ui
                                .selectable_label(
                                    w2,
                                    egui::RichText::new("侧墙 · 2面").size(10.5_f32),
                                )
                                .clicked()
                            {
                                self.state.vent.pad_layout_walls = 2.0;
                            }
                            if ui
                                .selectable_label(
                                    w3,
                                    egui::RichText::new("侧墙+山墙 · 3面").size(10.5_f32),
                                )
                                .clicked()
                            {
                                self.state.vent.pad_layout_walls = 3.0;
                            }
                        });
                        let body_h = row2 - 84.0;
                        // 左右分离：左列竖排 4 个湿帘几何参数，右列俯视/立面绘制
                        ui.horizontal_top(|ui| {
                            ui.vertical(|ui| {
                                ui.set_width(178.0);
                                changed |= field(
                                    ui,
                                    "山墙湿帘高度",
                                    "m",
                                    &mut self.state.vent.pad_gable_height,
                                    0.5..=4.0,
                                    0.1,
                                );
                                changed |= field(
                                    ui,
                                    "山墙湿帘层数",
                                    "道",
                                    &mut self.state.vent.pad_layers,
                                    1.0..=6.0,
                                    1.0,
                                );
                                changed |= field(
                                    ui,
                                    "侧墙湿帘高度",
                                    "m",
                                    &mut self.state.vent.pad_side_height,
                                    0.5..=4.0,
                                    0.1,
                                );
                                changed |= field(
                                    ui,
                                    "侧墙湿帘层数",
                                    "道",
                                    &mut self.state.vent.pad_side_layers,
                                    1.0..=6.0,
                                    1.0,
                                );
                                // 风机 P-Q 曲线 · 工作点小图（曲线机型；高窗口
                                // 时挂左列下方，小窗口时移到右列整幅绘制）
                                if self.outcome.vent.aero_based && row2 >= 350.0 {
                                    ui.add_space(6.0);
                                    ui.separator();
                                    let h_curve =
                                        (body_h - 4.0 * 28.0 - 40.0).clamp(70.0, 210.0);
                                    Self::fan_curve_mini(
                                        ui,
                                        &self.state,
                                        &self.outcome,
                                        h_curve,
                                    );
                                }
                            });
                            ui.separator();
                            ui.vertical(|ui| {
                                if row2 >= 350.0 {
                                    // 高窗口：俯视 · 立面布置图
                                    Self::pad_layout_view(ui, &self.state, body_h * 0.42);
                                    Self::pad_gable_view(ui, &self.state, body_h * 0.58);
                                } else if self.outcome.vent.aero_based {
                                    // 小窗口：不绘制两图（太挤），右列给 P-Q 曲线
                                    let full = format!(
                                        "{} {}",
                                        self.state.vent.fan_category, self.state.vent.fan_model_name
                                    );
                                    if duckcm_core::vent::aero_curve_for(&full).is_some() {
                                        Self::fan_curve_mini(
                                            ui,
                                            &self.state,
                                            &self.outcome,
                                            (body_h - 8.0).max(90.0),
                                        );
                                    }
                                }
                            });
                        });
                        // 配置违规：卡片右侧空域显示大型三角警告符（悬停列出全部警告）
                        if !self.outcome.warnings.is_empty() {
                            let r = ui.max_rect();
                            let msgs: Vec<String> = self
                                .outcome
                                .warnings
                                .iter()
                                .map(|w| w.to_string())
                                .collect();
                            let wr = egui::Rect::from_min_size(
                                egui::pos2(r.max.x - 118.0, (r.min.y + r.max.y) / 2.0 - 64.0),
                                egui::vec2(104.0, 104.0),
                            );
                            let resp = ui.put(
                                wr,
                                egui::Label::new(
                                    RichText::new("⚠").size(88.0_f32).color(theme::ERR),
                                )
                                .selectable(false),
                            );
                            resp.on_hover_text(msgs.join("\n"));
                            ui.put(
                                egui::Rect::from_min_size(
                                    egui::pos2(r.max.x - 118.0, (r.min.y + r.max.y) / 2.0 + 44.0),
                                    egui::vec2(104.0, 30.0),
                                ),
                                egui::Label::new(
                                    RichText::new(format!("配置违规 {} 项", msgs.len()))
                                        .size(10.5_f32)
                                        .color(theme::ERR),
                                )
                                .selectable(false)
                                .wrap_mode(egui::TextWrapMode::Extend),
                            );
                        }
                    },
                );
            }
        });

        ui.add_space(10.0);
        changed
    }

    /// 房屋参数单（3 项输入 + 8 项计算）
    fn house_table(&mut self, ui: &mut Ui, h: f32) -> bool {
        let mut changed = false;
        dash_card(ui, "房屋参数单", h, |ui| {
            let h = &mut self.state.house;
            let r = &self.outcome.house;
            changed |= dash_row(ui, "舍长", theme::WARN, Dash::In(&mut h.length), "m", 0.5);
            changed |= dash_row(ui, "舍宽", theme::WARN, Dash::In(&mut h.width), "m", 0.5);
            changed |= dash_row(
                ui,
                "檐口高度",
                theme::WARN,
                Dash::In(&mut h.eave_height),
                "m",
                0.1,
            );
            dash_row(ui, "山墙面积", theme::OK, Dash::Out(r.area_sq), "m²", 0.0);
            dash_row(
                ui,
                "屋面面积",
                theme::OK,
                Dash::Out(r.area_roof_no_ceiling),
                "m²",
                0.0,
            );
            dash_row(ui, "纵墙面积", theme::OK, Dash::Out(r.area_zq), "m²", 0.0);
            dash_row(ui, "房间容积", theme::OK, Dash::Out(r.volume), "m³", 0.0);
            dash_row(ui, "地面热损", theme::OK, Dash::Out(r.wk_floor), "W/K", 0.0);
            dash_row(
                ui,
                "墙面热损",
                theme::OK,
                Dash::Out(r.wk_wall_total),
                "W/K",
                0.0,
            );
            dash_row(ui, "屋面热损", theme::OK, Dash::Out(r.wk_roof), "W/K", 0.0);
            dash_row(ui, "冷渗热损", theme::OK, Dash::Out(r.wk_air), "W/K", 0.0);
        });
        changed
    }

    /// 供暖参数单
    fn heat_table(&mut self, ui: &mut Ui, h: f32) -> bool {
        let mut changed = false;
        dash_card(ui, "供暖·末端参数单", h, |ui| {
            let hp = &mut self.state.heat;
            let r = &self.outcome.heat;
            dash_row(
                ui,
                "供暖能力(-10℃)",
                theme::OK,
                Dash::Out(r.heating_capacity_kw),
                "kW",
                0.0,
            );
            dash_row(
                ui,
                "所需机组",
                theme::OK,
                Dash::Out(r.heater_units_required),
                "台",
                0.0,
            );
            changed |= dash_row(
                ui,
                "最低养殖环境温度",
                theme::ERR,
                Dash::In(&mut hp.min_design_temp),
                "°C",
                0.5,
            );
            dash_row(
                ui,
                "房屋热负",
                theme::OK,
                Dash::Out(r.house_heat_load_kw),
                "kW",
                0.0,
            );
            dash_row(
                ui,
                "运行舍温(热网平衡)",
                if r.network_heat_limited {
                    theme::WARN
                } else {
                    theme::OK
                },
                Dash::Out(r.network_room_temp),
                "°C",
                0.0,
            );
            dash_row(
                ui,
                "可达温度(供水50°C)",
                theme::OK,
                Dash::Out(r.reachable_temp),
                "°C",
                0.0,
            );
            changed |= dash_row(
                ui,
                "养殖需求舍内温度",
                theme::ERR,
                Dash::In(&mut hp.target_room_temp),
                "°C",
                0.5,
            );
            // 末端组（原「末端参数单」合并入本卡）；燃气热风机直吹无末端
            ui.add_space(4.0);
            ui.separator();
            if duckcm_core::heat::heater_no_terminal(&hp.heater_model_name) {
                ui.label(
                    RichText::new("直吹式燃气热风机：无末端水路，散热能力/水温/末端数量不适用")
                        .size(10.5_f32)
                        .color(theme::TEXT_MUTED),
                );
            } else {
                changed |= dash_row(
                    ui,
                    "末端散热能力",
                    theme::ERR,
                    Dash::In(&mut hp.terminal_unit_conductance),
                    "W/K",
                    10.0,
                );
                changed |= dash_row(
                    ui,
                    "供回水平均温度",
                    theme::ERR,
                    Dash::In(&mut hp.water_temp),
                    "°C",
                    1.0,
                );
                dash_row(
                    ui,
                    "末端总能力",
                    theme::OK,
                    Dash::Out(r.terminal_total_power_kw),
                    "kW/K",
                    0.0,
                );
            }
        });
        changed
    }

    /// 湿帘参数单
    fn pad_table(&mut self, ui: &mut Ui, h: f32, show_readout: bool) -> bool {
        let mut changed = false;
        dash_card(ui, "湿帘参数单", h, |ui| {
            if show_readout {
                // 左列＝主参数区（一整列），竖分割线，右列＝降温指标区
                ui.horizontal_top(|ui| {
                    let col = (ui.available_width() * 0.52).max(260.0);
                    ui.vertical(|ui| {
                        ui.set_width(col);
                        changed |= self.pad_main_rows(ui);
                    });
                    ui.separator();
                    ui.vertical(|ui| {
                        let r = &self.outcome.cool;
                        let vt = &self.outcome.vent;
                        cooling_readout_rows_range(ui, r, vt, 0..6);
                    });
                });
            } else {
                changed |= self.pad_main_rows(ui);
            }
        });
        changed
    }

    /// 主参数区（单列）：风量/湿帘 → 风机工作点因果链 → 体感温度
    fn pad_main_rows(&mut self, ui: &mut Ui) -> bool {
        let mut changed = false;
        let v = &mut self.state.vent;
        let r = &self.outcome.cool;
        dash_row(ui, "计算通风量", theme::OK, Dash::Out(r.air_volume), "m³/h", 0.0);
        changed |= dash_row(
            ui,
            "湿帘厚度",
            theme::ERR,
            Dash::In(&mut v.pad_thickness_mm),
            "mm",
            10.0,
        );
        dash_row(ui, "过帘风速", theme::OK, Dash::Out(r.pad_wind_speed), "m/s", 0.0);
        dash_row(ui, "GB规范面积", theme::OK, Dash::Out(r.pad_area_gb), "m²", 0.0);
        dash_row(ui, "平衡法面积", theme::OK, Dash::Out(r.pad_area_balance), "m²", 0.0);
        dash_row(
            ui,
            "蒸发需水量60%RH",
            theme::OK,
            Dash::Out(r.evap_capacity_kg_h / 1000.0),
            "m³/h",
            0.0,
        );
        dash_row(ui, "蒸发吸热", theme::OK, Dash::Out(r.cooling_power_kw), "kW", 0.0);
        changed |= dash_row(
            ui,
            "−20Pa校核点通风量",
            theme::ERR,
            Dash::In(&mut v.fan_unit_air_volume),
            "m³/h",
            100.0,
        );
        // 工作点因果链：系统静压 → 工作点风量 → 可变效率 → 计算风机数
        {
            let vt = &self.outcome.vent;
            if vt.aero_based {
                dash_row(ui, "系统静压", theme::OK, Dash::Out(vt.dp_system_pa), "Pa", 0.0);
                dash_row(
                    ui,
                    "工作点风量/台",
                    theme::OK,
                    Dash::Out(vt.fan_q_operating),
                    "m³/h",
                    0.0,
                );
                dash_row(
                    ui,
                    "可变风机效率",
                    theme::OK,
                    Dash::Out(vt.fan_eta_operating * 100.0),
                    "%",
                    0.0,
                );
            }
            // 计算风机数：统一 vent 域最终口径（曲线机型=工作点风量；
            // 非曲线机型=额定×固定效率折算，均含备用）
            let n_show = vt.fans_required;
            dash_row(ui, "计算风机数", theme::OK, Dash::Out(n_show), "台", 0.0);
        }
        dash_row(ui, "体感温度", theme::OK, Dash::Out(r.apparent_temperature), "°C", 0.0);
        changed
    }

    /// 通风参数单
    fn vent_table(&mut self, ui: &mut Ui, h: f32) -> bool {
        let mut changed = false;
        dash_card(ui, "通风参数单", h, |ui| {
            let b = &mut self.state.breed;
            let v = &mut self.state.vent;
            let r = &self.outcome.vent;
            dash_row(
                ui,
                "最低通风量",
                theme::OK,
                Dash::Out(b.minimum_ventilation),
                "m³/h",
                0.0,
            );
            changed |= dash_row(
                ui,
                "小窗面积",
                theme::ERR,
                Dash::In(&mut v.window_area),
                "m²",
                0.001,
            );
            dash_row(
                ui,
                "最低小窗数量",
                theme::OK,
                Dash::Out(r.windows_min_count),
                "个",
                0.0,
            );
            let w_changed = dash_row(
                ui,
                "风速(湍流校核)",
                theme::ERR,
                Dash::In(&mut v.window_speed),
                "m/s",
                0.1,
            );
            if w_changed {
                // 洞口风速下限 2.2（不设上限）
                v.window_speed = v.window_speed.max(2.2);
            }
            changed |= w_changed;
            dash_row(
                ui,
                "空间修正小窗",
                theme::OK,
                Dash::Out(r.windows_amended_count),
                "个",
                0.0,
            );
            dash_row(
                ui,
                "最优小窗数",
                theme::OK,
                Dash::Out(r.windows_best_count),
                "个",
                0.0,
            );
            dash_row(
                ui,
                "计算小窗总数",
                theme::OK,
                Dash::Out(r.windows_total_count),
                "个",
                0.0,
            );
            dash_row(
                ui,
                "房屋冷负荷",
                theme::OK,
                Dash::Out(r.house_cool_load_kw),
                "kW",
                0.0,
            );
            dash_row(
                ui,
                "生物冷负荷",
                theme::OK,
                Dash::Out(r.bio_cool_load_kw),
                "kW",
                0.0,
            );
            changed |= dash_row(
                ui,
                "设备冷负荷",
                theme::ERR,
                Dash::In(&mut v.equipment_cool_load_kw),
                "kW",
                1.0,
            );
            dash_row(
                ui,
                "基础总冷负",
                theme::OK,
                Dash::Out(r.total_cool_load_kw),
                "kW",
                0.0,
            );
        });
        changed
    }

    /// 实际应用（装机数量：显示计算值 + 自动跟随，可手动覆盖）
    fn actual_table(&mut self, ui: &mut Ui, h: f32) -> bool {
        let mut changed = false;
        let tuni = terminal_unit(&self.state.heat.terminal_type_name);
        let follow_now = self.state.vent.actual_follow;
        let restored = dash_card_head(
            ui,
            "实际应用",
            h,
            if follow_now {
                None
            } else {
                Some("恢复跟随计算值")
            },
            |ui| {
                let r = &self.outcome.heat;
                let r2 = &self.outcome.vent;
                let v = &mut self.state.vent;
                // 机组推荐（灰）= 跟随公式 ⌈需求台数 × 1.1⌉
                let units_rec = if r.heater_units_required > 0.0 {
                    (r.heater_units_required * 1.1).ceil().max(1.0)
                } else {
                    r.heater_units_required
                };
                let hp = &mut self.state.heat;
                // 供暖机组数：实际装机可改（改后不再自动配置）
                let units_changed = dash_row_calc(
                    ui,
                    "供暖机组数",
                    theme::ERR,
                    Dash::In(&mut self.curve.units),
                    units_rec,
                    "台",
                    1.0,
                );
                if units_changed {
                    hp.heater_units_manual = true;
                }
                changed |= units_changed;
                let a = dash_row_calc(
                    ui,
                    "小窗数量",
                    theme::ERR,
                    Dash::In(&mut v.windows_actual),
                    r2.windows_total_count,
                    "个",
                    1.0,
                );
                let b = dash_row_calc(
                    ui,
                    "风机数量",
                    theme::ERR,
                    Dash::In(&mut v.fans_actual),
                    // 推荐含 10% 冗余：⌊计算值 × 1.1⌋（工作点反校口径）
                    (r2.fans_required * 1.1).floor(),
                    "台",
                    1.0,
                );
                let c = if duckcm_core::heat::heater_no_terminal(&hp.heater_model_name) {
                    false
                } else {
                    dash_row_calc(
                        ui,
                        "末端数量",
                        theme::ERR,
                        Dash::In(&mut hp.terminal_count),
                        // 灰值 = 跟随公式 ceil(需求 × 1.1)
                        (self.outcome.heat.terminal_count_required * 1.1).ceil(),
                        tuni,
                        1.0,
                    )
                };
                let pad_module = 0.6 * v.pad_side_height.max(0.3);
                // 湿帘推荐（灰）与跟随公式同源：推荐实配 ⌊计算×1.1⌋ 台、
                // 运行台数 ×0.9 抽吸平衡（额定 −20 Pa 口径），模块上取整
                let rec_actual = (r2.fans_required * 1.1).floor();
                let pad_need = (rec_actual * 0.9).ceil() * v.fan_unit_air_volume / 3600.0 / 1.6;
                let pad_calc = (pad_need / pad_module).ceil() * pad_module;
                let d = dash_row_calc(
                    ui,
                    "湿帘面积",
                    theme::ERR,
                    Dash::In(&mut v.pad_area_actual),
                    pad_calc,
                    "m²",
                    0.5,
                );
                if a || b || c || d {
                    v.actual_follow = false;
                }
                changed |= a || b || c || d;

                // 采食与突发用水（种类采食标准；水料比 2.2，不计入冷负荷链）——不依赖迭代显示
                {
                    let calc_w = 52.0_f32;
                    let box_w = 84.0_f32;
                    let unit_w = 40.0_f32;
                    let gap = ui.spacing().item_spacing.x;
                    let group_w = calc_w + box_w + unit_w + gap * 4.0;
                    let feed_std = breed_feed(self.state.breed.breed_type);
                    let daily_feed = feed_std * self.state.breed.breed_vol; // kg/日
                    let sudden = daily_feed / 3.0 * 2.2 / 1000.0; // m³/h
                                                                  // 蒸发需水 = 7090 传质法湿帘总蒸发能力（全湿润耗水，非湿度平衡小值）
                    let evap_cap = self.outcome.cool.evap_capacity_kg_h / 1000.0;
                    let (wq, wdn, wv) = duckcm_core::bio::water_supply_sizing(sudden, evap_cap);
                    ui.add_space(6.0);
                    ui.separator();
                    let feed_rows = [
                        ("日采食量", format!("{:.0}", daily_feed), "kg/日"),
                        ("突发用水量", format!("{:.2}", sudden), "m³/h"),
                        ("湿帘蒸发能力", format!("{:.2}", evap_cap), "m³/h"),
                        ("流量需求", format!("{:.1}", wq), "m³/h"),
                        ("推荐水管管径", format!("DN{:.0}·{:.1}m/s", wdn, wv), "—"),
                    ];
                    for (label, val, unit) in feed_rows {
                        let label_w = (ui.available_width() - group_w).max(60.0);
                        ui.horizontal(|ui| {
                            let (rect, _) = ui.allocate_exact_size(
                                egui::vec2(label_w, 19.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().text(
                                rect.right_center(),
                                Align2::RIGHT_CENTER,
                                label,
                                FontId::proportional(10.5_f32),
                                theme::OK,
                            );
                            let (crect, _) = ui.allocate_exact_size(
                                egui::vec2(calc_w + gap + box_w, 19.0),
                                egui::Sense::hover(),
                            );
                            ui.painter().text(
                                crect.right_center(),
                                Align2::RIGHT_CENTER,
                                &val,
                                FontId::proportional(11.5_f32),
                                theme::ACCENT,
                            );
                            ui.add_sized(
                                [unit_w, 19.0],
                                egui::Label::new(
                                    RichText::new(unit).size(10.0_f32).color(theme::TEXT_MUTED),
                                )
                                .selectable(false),
                            );
                        });
                    }
                }
            },
        );
        if restored {
            self.state.vent.actual_follow = true;
            self.state.heat.heater_units_manual = false;
            changed = true;
        }
        changed
    }
}

// ═══════════════════════════════════════════════════════════════
// 小组件
// ═══════════════════════════════════════════════════════════════

/// 仪表盘单元格：可编辑输入 / 只读结果
enum Dash<'a> {
    /// 可编辑输入（原图绿色框）
    In(&'a mut f64),
    /// 计算结果（只读）
    Out(f64),
}

/// 按最长标签文字估算参数单卡片宽度（CJK ≈11.5px/字，西文 ≈7px）
fn compact_w(labels: &[&str]) -> f32 {
    let max_label = labels
        .iter()
        .map(|s| {
            s.chars()
                .map(|c| if (c as u32) > 0x2E7F { 11.5_f32 } else { 7.0 })
                .sum::<f32>()
        })
        .fold(0.0_f32, f32::max);
    (max_label + 84.0 + 44.0 + 24.0 + 30.0).max(230.0)
}

/// 参数单卡片（撑满所在列宽度，同排行等高）
fn dash_card(ui: &mut Ui, title: &str, min_h: f32, add: impl FnOnce(&mut Ui)) {
    dash_card_head(ui, title, min_h, None, add);
}

/// 降温/湿帘物理指标读数行段（湿帘参数单右列整列 0..6）；
/// 「帘压降」按工作点反校值显示（与系统静压同源自洽）
fn cooling_readout_rows_range(
    ui: &mut Ui,
    r3: &duckcm_core::data::CoolResult,
    vt: &duckcm_core::data::VentResult,
    range: std::ops::Range<usize>,
) {
    let Some(hum) = r3.solved_humidity else {
        return;
    };
    let pad_dp_show = if vt.dp_pad_operating > 0.0 {
        vt.dp_pad_operating
    } else {
        r3.pressure_drop_pa
    };
    let rows = [
        (
            "显热温差",
            format!("{:.1}", (r3.pad_target - r3.supply_temperature).max(0.3)),
            "K",
        ),
        ("舍内湿度", format!("{:.1}", hum), "%"),
        ("目标温度", format!("{:.1}", r3.pad_target), "°C"),
        ("净吸热系数", format!("{:.2}", r3.s_tf * 1000.0), "w/m³·h"),
        ("帘饱和度 ε", format!("{:.1}", r3.saturation_pct), "%"),
        ("帘压降", format!("{:.1}", pad_dp_show), "Pa"),
    ];
    for (label, val, unit) in rows.into_iter().take(range.end).skip(range.start) {
        ui.horizontal(|ui| {
            ui.add_sized(
                [96.0, 19.0],
                egui::Label::new(RichText::new(label).size(10.5_f32).color(theme::ERR))
                    .selectable(false),
            );
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.add_sized(
                    [40.0, 19.0],
                    egui::Label::new(RichText::new(unit).size(10.0_f32).color(theme::TEXT_MUTED))
                        .selectable(false),
                );
                ui.add_sized(
                    [110.0, 19.0],
                    egui::Label::new(RichText::new(val).size(11.5_f32).color(theme::ACCENT))
                        .selectable(false)
                        .halign(egui::Align::Center),
                );
            });
        });
    }
}

/// 带标题行插槽的仪表盘卡：`head_btn` 为 Some 时在标题右侧绘制小按钮，返回其是否被点击
fn dash_card_head(
    ui: &mut Ui,
    title: &str,
    min_h: f32,
    head_btn: Option<&str>,
    add: impl FnOnce(&mut Ui),
) -> bool {
    let mut head_clicked = false;
    Frame::new()
        .fill(theme::CARD)
        .stroke(Stroke::new(1.0_f32, theme::BORDER))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_min_width(ui.available_width());
            ui.set_min_height(min_h - 24.0);
            match head_btn {
                Some(btn_text) => {
                    ui.horizontal(|ui| {
                        ui.label(text::group_header(title));
                        if ui
                            .small_button(
                                egui::RichText::new(btn_text)
                                    .size(10.5_f32)
                                    .color(theme::ACCENT),
                            )
                            .clicked()
                        {
                            head_clicked = true;
                        }
                    });
                }
                None => {
                    ui.label(text::group_header(title));
                }
            }
            ui.separator();
            ui.add_space(2.0);
            // 内容超出卡高时卡内滚动，保持卡片等高（同行底边对齐）
            egui::ScrollArea::vertical()
                .id_salt(("dash_card_scroll", title))
                .auto_shrink([false, true])
                .max_height((min_h - 56.0).max(80.0))
                .show(ui, |ui| {
                    add(ui);
                });
        });
    head_clicked
}

/// 参数单一行：彩色标签（右对齐贴框）｜值框｜单位（右缘对齐）
///
/// 配色沿用原程序：红=关键输入、绿=计算结果、橙=一般输入
fn dash_row(
    ui: &mut Ui,
    label: &str,
    label_color: Color32,
    value: Dash,
    unit: &str,
    speed: f64,
) -> bool {
    let mut changed = false;
    let row_h = 24.0;
    let box_w = 84.0_f32;
    let unit_w = 40.0_f32;
    let gap = ui.spacing().item_spacing.x;
    let group_w = box_w + unit_w + gap * 3.0;
    let label_w = (ui.available_width() - group_w).max(70.0);

    ui.horizontal(|ui| {
        // 标签：在剩余宽度内右对齐，使所有卡片/行的值框左缘对齐
        let (rect, _) = ui.allocate_exact_size(egui::vec2(label_w, row_h), egui::Sense::hover());
        ui.painter().text(
            rect.right_center(),
            Align2::RIGHT_CENTER,
            label,
            FontId::proportional(11.5),
            label_color,
        );

        match value {
            Dash::In(v) => {
                // 输入框：整块绿底（同原程序），临时改 widget 配色、画完还原
                const GREEN: Color32 = Color32::from_rgb(152, 251, 152);
                let old = {
                    let vs = ui.visuals();
                    (
                        vs.widgets.inactive.bg_fill,
                        vs.widgets.inactive.weak_bg_fill,
                        vs.widgets.hovered.bg_fill,
                        vs.widgets.hovered.weak_bg_fill,
                        vs.widgets.active.bg_fill,
                        vs.widgets.active.weak_bg_fill,
                    )
                };
                {
                    let vs = ui.visuals_mut();
                    for w in [
                        &mut vs.widgets.inactive,
                        &mut vs.widgets.hovered,
                        &mut vs.widgets.active,
                    ] {
                        w.bg_fill = GREEN;
                        w.weak_bg_fill = GREEN;
                    }
                }
                // 稳定 id：按标签命名——上游条件行增删时自动 id 会漂移，
                // 打断进行中的拖拽（拖动偶发失灵根因），钉死不随布局变化
                let r = ui
                    .push_id(("dashrow", label), |ui| {
                        ui.add_sized(
                            [box_w, row_h - 4.0],
                            DragValue::new(v)
                                .speed(speed)
                                .max_decimals(2)
                                .range(-1e6..=1e6),
                        )
                    })
                    .inner;
                changed |= r.changed();
                {
                    let vs = ui.visuals_mut();
                    vs.widgets.inactive.bg_fill = old.0;
                    vs.widgets.inactive.weak_bg_fill = old.1;
                    vs.widgets.hovered.bg_fill = old.2;
                    vs.widgets.hovered.weak_bg_fill = old.3;
                    vs.widgets.active.bg_fill = old.4;
                    vs.widgets.active.weak_bg_fill = old.5;
                }
            }
            Dash::Out(v) => {
                // 结果框（灰底只读）
                egui::Frame::new()
                    .fill(theme::RESULT_BG)
                    .corner_radius(egui::CornerRadius::same(3))
                    .inner_margin(egui::Margin::symmetric(3, 2))
                    .show(ui, |ui| {
                        ui.add_sized(
                            [box_w - 6.0, row_h - 6.0],
                            egui::Label::new(
                                RichText::new(fmt_val(v)).size(11.5_f32).color(theme::TEXT),
                            ),
                        );
                    });
            }
        }
        ui.add_sized([unit_w, row_h], egui::Label::new(text::unit(unit)));
    });
    changed
}

/// 参数单一行（带计算值参考）：彩色标签 | 计算值（灰小字）| 输入框 | 单位
fn dash_row_calc(
    ui: &mut Ui,
    label: &str,
    label_color: Color32,
    value: Dash,
    calc: f64,
    unit: &str,
    speed: f64,
) -> bool {
    let mut changed = false;
    let row_h = 24.0;
    let calc_w = 52.0_f32;
    let box_w = 84.0_f32;
    let unit_w = 40.0_f32;
    let gap = ui.spacing().item_spacing.x;
    let group_w = calc_w + box_w + unit_w + gap * 4.0;
    let label_w = (ui.available_width() - group_w).max(60.0);

    ui.horizontal(|ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(label_w, row_h), egui::Sense::hover());
        ui.painter().text(
            rect.right_center(),
            Align2::RIGHT_CENTER,
            label,
            FontId::proportional(11.5),
            label_color,
        );
        // 计算值参考（灰色小字，右对齐）
        let (crect, _) = ui.allocate_exact_size(egui::vec2(calc_w, row_h), egui::Sense::hover());
        ui.painter().text(
            crect.right_center(),
            Align2::RIGHT_CENTER,
            fmt_val(calc),
            FontId::proportional(10.5),
            theme::TEXT_MUTED,
        );
        match value {
            Dash::In(v) => {
                const GREEN: Color32 = Color32::from_rgb(152, 251, 152);
                const ORANGE: Color32 = Color32::from_rgb(255, 196, 120);
                // 实配低于计算值 → 橙底警示（允许，不再强制抬升）
                let low = calc > 0.0 && *v < calc - 1e-9;
                let fill = if low { ORANGE } else { GREEN };
                let old = {
                    let vs = ui.visuals();
                    (
                        vs.widgets.inactive.bg_fill,
                        vs.widgets.inactive.weak_bg_fill,
                        vs.widgets.hovered.bg_fill,
                        vs.widgets.hovered.weak_bg_fill,
                        vs.widgets.active.bg_fill,
                        vs.widgets.active.weak_bg_fill,
                    )
                };
                {
                    let vs = ui.visuals_mut();
                    for w in [
                        &mut vs.widgets.inactive,
                        &mut vs.widgets.hovered,
                        &mut vs.widgets.active,
                    ] {
                        w.bg_fill = fill;
                        w.weak_bg_fill = fill;
                    }
                }
                // 稳定 id：按标签命名——上游条件行增删时自动 id 会漂移，
                // 打断进行中的拖拽（拖动偶发失灵根因），钉死不随布局变化
                let r = ui
                    .push_id(("dashrow", label), |ui| {
                        ui.add_sized(
                            [box_w, row_h - 4.0],
                            DragValue::new(v)
                                .speed(speed)
                                .max_decimals(2)
                                .range(-1e6..=1e6),
                        )
                    })
                    .inner;
                let r_changed = r.changed();
                if low {
                    let _ = r.on_hover_text(format!("低于计算值 {calc:.1}——配置不足，请核实设计"));
                }
                changed |= r_changed;
                {
                    let vs = ui.visuals_mut();
                    vs.widgets.inactive.bg_fill = old.0;
                    vs.widgets.inactive.weak_bg_fill = old.1;
                    vs.widgets.hovered.bg_fill = old.2;
                    vs.widgets.hovered.weak_bg_fill = old.3;
                    vs.widgets.active.bg_fill = old.4;
                    vs.widgets.active.weak_bg_fill = old.5;
                }
            }
            Dash::Out(v) => {
                egui::Frame::new()
                    .fill(theme::RESULT_BG)
                    .corner_radius(egui::CornerRadius::same(3))
                    .inner_margin(egui::Margin::symmetric(3, 2))
                    .show(ui, |ui| {
                        ui.add_sized(
                            [box_w - 6.0, row_h - 6.0],
                            egui::Label::new(
                                RichText::new(fmt_val(v)).size(11.5_f32).color(theme::TEXT),
                            ),
                        );
                    });
            }
        }
        ui.add_sized([unit_w, row_h], egui::Label::new(text::unit(unit)));
    });
    changed
}

/// 数值格式（无效值显示 —，尾零去除）
fn fmt_val(v: f64) -> String {
    if !v.is_finite() {
        return "—".to_string();
    }
    if v.abs() >= 100000.0 {
        return format!("{v:.0}");
    }
    let s = format!("{v:.2}");
    let s = s.trim_end_matches('0').trim_end_matches('.');
    s.to_string()
}

/// 图表卡片（第一行中列，与同行卡片等高）
fn chart_card(ui: &mut Ui, series: &crate::chart::ChartSeries, minh: f32) {
    Frame::new()
        .fill(theme::CARD)
        .stroke(Stroke::new(1.0_f32, theme::BORDER))
        .corner_radius(egui::CornerRadius::same(10))
        .inner_margin(egui::Margin::same(12))
        .show(ui, |ui| {
            ui.set_min_height(minh - 24.0);
            ui.label(text::group_header("房间温度曲线 · 制热拟合"));
            ui.add_space(4.0);
            crate::chart::draw_with_height(ui, series, "main", (minh - 52.0).max(240.0));
        });
}

/// 数值输入行：标签 | 输入框 | 单位
fn field(
    ui: &mut Ui,
    label: &str,
    unit: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    speed: f64,
) -> bool {
    let mut changed = false;
    ui.horizontal(|ui| {
        ui.add_sized([120.0, 24.0], egui::Label::new(text::label(label)));
        let r = ui.add_sized(
            [110.0, 24.0],
            DragValue::new(value).speed(speed).range(range),
        );
        changed |= r.changed();
        ui.label(text::unit(unit));
    });
    changed
}

/// 网格中的数值输入（配置单用）
fn grid_field(
    ui: &mut Ui,
    label: &str,
    unit: &str,
    value: &mut f64,
    range: std::ops::RangeInclusive<f64>,
    speed: f64,
) -> bool {
    ui.label(text::label(label));
    let r = ui.add(
        DragValue::new(value)
            .speed(speed)
            .range(range)
            .fixed_decimals(2),
    );
    ui.label(text::unit(unit));
    ui.end_row();
    r.changed()
}

/// 结果行：名称  数值 单位
fn row(ui: &mut Ui, label: &str, value: f64, unit: &str) {
    ui.horizontal(|ui| {
        ui.add_sized([110.0, 22.0], egui::Label::new(text::label(label)));
        let shown = if !value.is_finite() {
            "—".to_string()
        } else {
            format!("{value:.2}")
        };
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(text::unit(unit));
            ui.label(text::value(&shown));
        });
    });
}

// 未使用告警抑制（Slider 预留）
#[allow(dead_code)]
fn _unused(ui: &mut Ui, v: &mut f64) {
    ui.add(Slider::new(v, 0.0..=1.0));
}

/// 养殖种类中文名（原 YangZCon 下拉，细分肉禽/蛋禽）
/// 成年期头均日采食标准 [kg/只/日]（种类采食标准表）
fn breed_feed(t: BreedType) -> f64 {
    match t {
        BreedType::LayerChicken => 0.115,
        BreedType::BroilerChicken => 0.17,
        BreedType::LayerDuck => 0.13,
        BreedType::BroilerDuck => 0.19,
        BreedType::Goose => 0.22,
        BreedType::Other => 0.15,
    }
}

fn breed_label(t: BreedType) -> &'static str {
    match t {
        BreedType::LayerChicken => "蛋鸡",
        BreedType::BroilerChicken => "肉鸡",
        BreedType::LayerDuck => "蛋鸭",
        BreedType::BroilerDuck => "肉鸭",
        BreedType::Goose => "鹅",
        BreedType::Other => "其他",
    }
}

/// 种类修正系数 cpc（原 YangZCon：鸡 1.1 / 鸭 1.0 / 鹅 1.2 / 其他 1.3）
fn breed_cpc(t: BreedType) -> f64 {
    match t {
        BreedType::LayerChicken => 1.1,
        BreedType::BroilerChicken => 1.1,
        BreedType::LayerDuck => 1.0,
        BreedType::BroilerDuck => 1.0,
        BreedType::Goose => 1.2,
        BreedType::Other => 1.3,
    }
}

/// 种类默认成型体重 kg（按品种饲养手册成年/出栏体重修正：
/// 蛋鸡·海兰褐成年 ≈1.9；肉鸡·AA/罗斯308 42d ≈2.6；
/// 蛋鸭·金定/绍鸭成年 ≈1.75；肉鸭·樱桃谷 41d ≈3.2；鹅·朗德 ≈5.0）
fn breed_default_weight(t: BreedType) -> Option<f64> {
    match t {
        BreedType::LayerChicken => Some(1.9),
        BreedType::BroilerChicken => Some(2.6),
        BreedType::LayerDuck => Some(1.75),
        BreedType::BroilerDuck => Some(3.2),
        BreedType::Goose => Some(5.0),
        BreedType::Other => None,
    }
}

/// 末端计量单位：地暖管按米（m）计，散热器按台/组计
fn terminal_unit(type_name: &str) -> &'static str {
    if type_name.contains("地暖") {
        "m"
    } else {
        "台"
    }
}

/// 画面提示语
fn screen_hint(screen: ConfigScreen) -> &'static str {
    match screen {
        ConfigScreen::House => "剖面 / 围护 / 保温材料",
        ConfigScreen::Heat => "热源机组 / 末端库 / 能力预览",
        ConfigScreen::Breed => "养殖种类 / 只数 / 呼吸",
        ConfigScreen::Climate => "气温 / 辐射 / 湿度",
        ConfigScreen::Equip => "小窗 / 风机",
    }
}

/// 底部导航按钮：纯文字，激活态用主色填充
fn nav_button(ui: &mut Ui, screen: ConfigScreen, active: bool) -> egui::Response {
    let size = egui::vec2(104.0, 42.0);
    let (rect, resp) = ui.allocate_exact_size(size, egui::Sense::click());

    let p = ui.painter_at(rect);
    let bg = if active {
        theme::ACCENT
    } else if resp.hovered() {
        Color32::from_rgb(239, 246, 255)
    } else {
        theme::CARD
    };
    p.rect_filled(rect, egui::CornerRadius::same(8), bg);
    if !active {
        p.rect_stroke(
            rect,
            egui::CornerRadius::same(8),
            Stroke::new(1.0_f32, theme::BORDER),
            egui::StrokeKind::Inside,
        );
    }

    let text_color = if active { theme::CARD } else { theme::TEXT };
    p.text(
        rect.center(),
        Align2::CENTER_CENTER,
        screen.label(),
        FontId::proportional(13.5),
        text_color,
    );

    resp
}

/// 颜色工具（导出报告用）
#[allow(dead_code)]
pub fn hex(c: Color32) -> String {
    format!("#{:02X}{:02X}{:02X}", c.r(), c.g(), c.b())
}
