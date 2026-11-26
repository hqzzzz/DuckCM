//! 门式刚架（门钢）截面参数化绘图
//!
//! 替代原 C# 的 CAD 方案：原程序用 netDxf 解析 `AutoCad2013.dxf` 再用
//! CSharpGL 渲染固定剖面图（依赖 3 个大型 DLL），且图形不随参数变化。
//! 这里改为**纯参数化绘制**——改一个尺寸，图立即跟着变，零 CAD 依赖。
//!
//! 绘制内容（对应原截图）：
//! 双坡门式刚架（柱 + 斜梁 + 梁柱加腋 + 脊拼接）、隅撑/檩条刻度、墙梁刻度、
//! 柱脚底板、地面线与阴影线、吊顶线、围护高度带、左侧高度标注链、
//! 底部跨度标注（±0.00 / 300 / 7500 / 8300 / 8400 / 18000）。

use egui::{Align2, Color32, FontId, Pos2, Rect, Response, Sense, Stroke, Ui, Vec2};

/// 剖面几何参数（单位 mm）
#[derive(Debug, Clone, Copy)]
pub struct SectionGeometry {
    /// 舍宽（跨度）
    pub span_mm: f64,
    /// 舍长
    pub length_mm: f64,
    /// 檐口高度
    pub eave_mm: f64,
    /// 屋顶高度（脊高）
    pub ridge_mm: f64,
    /// 吊顶高度
    pub ceiling_mm: f64,
    /// 围护高度（墙脚带）
    pub wall_band_mm: f64,
    /// 是否有吊顶
    pub has_ceiling: bool,
}

impl Default for SectionGeometry {
    fn default() -> Self {
        Self {
            span_mm: 18_000.0,
            length_mm: 114_000.0,
            eave_mm: 7_500.0,
            ridge_mm: 8_400.0,
            ceiling_mm: 8_300.0,
            wall_band_mm: 300.0,
            has_ceiling: false,
        }
    }
}

// ── 配色（与主题一致）───────────────────────────────────────────
const STEEL: Color32 = Color32::from_rgb(51, 65, 85); // 主刚架
const SECONDARY: Color32 = Color32::from_rgb(148, 163, 184); // 檩条/墙梁
const GROUND: Color32 = Color32::from_rgb(71, 85, 105);
const DIM: Color32 = Color32::from_rgb(22, 163, 74); // 标注（沿用原图绿色）
const DIM_LINE: Color32 = Color32::from_rgb(134, 239, 172);
const CEILING: Color32 = Color32::from_rgb(37, 99, 235);
const BAND: Color32 = Color32::from_rgb(217, 119, 6);
const GRID: Color32 = Color32::from_rgb(226, 232, 240);

/// 世界坐标 → 屏幕坐标变换
struct Transform {
    scale: f64,
    origin: Pos2,
}

impl Transform {
    fn new(rect: Rect, bounds: (f64, f64, f64, f64)) -> Self {
        // bounds = (x_min, x_max, y_min, y_max) in world mm
        let (x0, x1, y0, y1) = bounds;
        let w = (x1 - x0).max(1.0);
        let h = (y1 - y0).max(1.0);
        let scale = ((rect.width() as f64) / w).min((rect.height() as f64) / h);
        let origin = Pos2::new(
            rect.center().x - (scale * (x0 + x1) / 2.0) as f32,
            rect.center().y + (scale * (y0 + y1) / 2.0) as f32,
        );
        Self { scale, origin }
    }

    fn p(&self, x: f64, y: f64) -> Pos2 {
        Pos2::new(
            self.origin.x + (x * self.scale) as f32,
            self.origin.y - (y * self.scale) as f32,
        )
    }

    fn len(&self, mm: f64) -> f32 {
        (mm * self.scale) as f32
    }
}

/// 绘制门式刚架剖面，返回控件响应
pub fn draw_portal_frame(ui: &mut Ui, desired: Vec2, geo: &SectionGeometry) -> Response {
    let (rect, response) = ui.allocate_exact_size(desired, Sense::hover());
    let painter = ui.painter_at(rect);

    // 背景
    painter.rect_filled(
        rect,
        egui::CornerRadius::same(8),
        Color32::from_rgb(250, 251, 253),
    );
    painter.rect_stroke(
        rect,
        egui::CornerRadius::same(8),
        Stroke::new(1.0_f32, GRID),
        egui::StrokeKind::Inside,
    );

    let span = geo.span_mm.max(1_000.0);
    let eave = geo.eave_mm.max(500.0);
    let ridge = geo.ridge_mm.max(eave + 100.0);

    // 世界边界：左侧留标高尺寸链，底部留跨度标注
    let bounds = (-3_100.0, span + 700.0, -1_900.0, ridge + 1_100.0);
    let t = Transform::new(rect.shrink(12.0), bounds);

    draw_ground(&painter, &t, span);
    draw_frame(&painter, &t, span, eave, ridge);
    if geo.has_ceiling && geo.ceiling_mm > 0.0 {
        draw_ceiling(&painter, &t, span, geo.ceiling_mm.min(ridge));
    }
    draw_wall_band(&painter, &t, span, geo.wall_band_mm.min(eave));
    draw_elevations(&painter, &t, geo, span, eave, ridge);
    draw_span_dimension(&painter, &t, span);
    draw_length_note(&painter, &t, geo.length_mm, ridge);

    response
}

/// 地面线 + 剖面阴影
fn draw_ground(p: &egui::Painter, t: &Transform, span: f64) {
    let y = t.p(0.0, 0.0).y;
    let x0 = t.p(-1_500.0, 0.0).x;
    let x1 = t.p(span + 500.0, 0.0).x;
    p.line_segment(
        [Pos2::new(x0, y), Pos2::new(x1, y)],
        Stroke::new(2.0_f32, GROUND),
    );
    // 斜线阴影（地面）
    let mut x = x0;
    while x < x1 {
        p.line_segment(
            [
                Pos2::new(x, y),
                Pos2::new(x + t.len(260.0), y + t.len(260.0).min(10.0)),
            ],
            Stroke::new(1.0_f32, SECONDARY),
        );
        x += t.len(600.0);
    }
}

/// 刚架主体：柱、斜梁、加腋、脊拼接、檩条/墙梁刻度、柱脚
fn draw_frame(p: &egui::Painter, t: &Transform, span: f64, eave: f64, ridge: f64) {
    let half = span / 2.0;
    let steel = Stroke::new(2.2_f32, STEEL);
    let thin = Stroke::new(1.2_f32, SECONDARY);

    // 柱（H 型钢：翼缘双线）
    let flange = 120.0; // 翼缘半宽 [mm]
    for &cx in &[-flange, flange, span - flange, span + flange] {
        p.line_segment([t.p(cx, 0.0), t.p(cx, eave)], steel);
    }
    // 柱腹板
    for &cx in &[0.0, span] {
        p.line_segment(
            [t.p(cx, 0.0), t.p(cx, eave)],
            Stroke::new(1.0_f32, SECONDARY),
        );
    }

    // 斜梁（双线，梁高 180 mm，垂直偏移）
    let girder_half = 90.0;
    draw_sloped_girder(p, t, 0.0, eave, half, ridge, girder_half, steel);
    draw_sloped_girder(p, t, half, ridge, span, eave, girder_half, steel);

    // 梁柱加腋（门刚特征：檐口膝节点处加深，沿斜梁**向跨内**渐变到梁高）
    draw_haunch(p, t, 0.0, eave, half, ridge, steel);
    draw_haunch(p, t, span, eave, half, ridge, steel);

    // 脊拼接板
    let splice_h = 420.0;
    p.line_segment(
        [
            t.p(half - 90.0, ridge - splice_h),
            t.p(half - 90.0, ridge + 90.0),
        ],
        thin,
    );
    p.line_segment(
        [
            t.p(half + 90.0, ridge - splice_h),
            t.p(half + 90.0, ridge + 90.0),
        ],
        thin,
    );

    // 檩条刻度（沿斜梁每 1500 mm）
    draw_slope_ticks(p, t, 0.0, eave, half, ridge, 1_500.0, thin);
    draw_slope_ticks(p, t, half, ridge, span, eave, 1_500.0, thin);

    // 墙梁刻度（沿柱每 1800 mm）
    let mut y = 1_800.0;
    while y < eave - 200.0 {
        p.line_segment([t.p(-flange, y), t.p(flash(p, t, flange), y)], thin);
        p.line_segment([t.p(span - flange, y), t.p(span + flange, y)], thin);
        y += 1_800.0;
    }

    // 柱脚底板
    for &cx in &[0.0, span] {
        let a = t.p(cx - 320.0, 0.0);
        let b = t.p(cx + 320.0, -90.0);
        p.rect_filled(Rect::from_two_pos(a, b), egui::CornerRadius::ZERO, STEEL);
    }
}

fn flash(_p: &egui::Painter, t: &Transform, v: f64) -> f64 {
    let _ = t;
    v
}

/// 斜梁双线
fn draw_sloped_girder(
    p: &egui::Painter,
    t: &Transform,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    half_depth: f64,
    stroke: Stroke,
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    // 单位法向
    let nx = -dy / len;
    let ny = dx / len;
    p.line_segment(
        [
            t.p(x0 + nx * half_depth, y0 + ny * half_depth),
            t.p(x1 + nx * half_depth, y1 + ny * half_depth),
        ],
        stroke,
    );
    p.line_segment(
        [
            t.p(x0 - nx * half_depth, y0 - ny * half_depth),
            t.p(x1 - nx * half_depth, y1 - ny * half_depth),
        ],
        stroke,
    );
}

/// 檐口加腋（梁柱节点处加深，沿斜梁向跨内渐变到标准梁高）
///
/// 修正：原实现把楔形画到**跨外**且坡度取 0.5（与实际屋面坡度 0.1 不符），
/// 在檐口外侧留下一个突兀的"角"。现按斜梁真实走向、法向朝下绘制。
fn draw_haunch(
    p: &egui::Painter,
    t: &Transform,
    cx: f64,
    eave: f64,
    half_span: f64,
    ridge: f64,
    stroke: Stroke,
) {
    // 斜梁走向：从柱顶指向屋脊（跨内）
    let (x1, y1) = (half_span, ridge);
    let dx = x1 - cx;
    let dy = y1 - eave;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / len;
    let uy = dy / len;
    // 法向：取指向"下方"的一侧（ny > 0）
    let (mut nx, mut ny) = (-uy, ux);
    if ny < 0.0 {
        nx = -nx;
        ny = -ny;
    }

    // 加腋长度约为斜梁的 28 %，且不超过斜梁
    let haunch_len = (len * 0.28).min(len - 300.0);
    // 节点处梁深 → 跨中梁深
    let joint_depth = 420.0;
    let girder_depth = 180.0;

    let tip_x = cx + ux * haunch_len;
    let tip_y = eave + uy * haunch_len;

    // 下缘：从节点深度线性收缩到梁高（上缘与斜梁上翼缘重合，不再另画）
    p.line_segment(
        [
            t.p(cx + nx * joint_depth / 2.0, eave + ny * joint_depth / 2.0),
            t.p(
                tip_x + nx * girder_depth / 2.0,
                tip_y + ny * girder_depth / 2.0,
            ),
        ],
        stroke,
    );
    // 腋口端部斜封板
    p.line_segment(
        [
            t.p(
                tip_x + nx * girder_depth / 2.0,
                tip_y + ny * girder_depth / 2.0,
            ),
            t.p(
                tip_x - nx * girder_depth / 2.0,
                tip_y - ny * girder_depth / 2.0,
            ),
        ],
        stroke,
    );
    // 柱顶封板（节点处下缘到柱顶）
    p.line_segment(
        [
            t.p(cx + nx * joint_depth / 2.0, eave + ny * joint_depth / 2.0),
            t.p(cx, eave),
        ],
        stroke,
    );
}

/// 沿斜梁的檩条刻度
fn draw_slope_ticks(
    p: &egui::Painter,
    t: &Transform,
    x0: f64,
    y0: f64,
    x1: f64,
    y1: f64,
    spacing: f64,
    stroke: Stroke,
) {
    let dx = x1 - x0;
    let dy = y1 - y0;
    let len = (dx * dx + dy * dy).sqrt().max(1.0);
    let ux = dx / len;
    let uy = dy / len;
    let nx = -uy;
    let ny = ux;
    let mut s = spacing * 0.5;
    while s < len - 300.0 {
        let px = x0 + ux * s;
        let py = y0 + uy * s;
        p.line_segment(
            [
                t.p(px + nx * 130.0, py + ny * 130.0),
                t.p(px - nx * 130.0, py - ny * 130.0),
            ],
            stroke,
        );
        s += spacing;
    }
}

/// 吊顶线
fn draw_ceiling(p: &egui::Painter, t: &Transform, span: f64, ceiling: f64) {
    p.line_segment(
        [t.p(0.0, ceiling), t.p(span, ceiling)],
        Stroke::new(1.8_f32, CEILING),
    );
    // 吊顶示意：短竖线
    let mut x = 1_200.0;
    while x < span - 600.0 {
        p.line_segment(
            [t.p(x, ceiling), t.p(x, ceiling - 220.0)],
            Stroke::new(1.0_f32, CEILING),
        );
        x += 2_400.0;
    }
}

/// 围护高度带（墙脚保温带）
fn draw_wall_band(p: &egui::Painter, t: &Transform, span: f64, band: f64) {
    if band <= 0.0 {
        return;
    }
    let rect = Rect::from_two_pos(t.p(-120.0, 0.0), t.p(span + 120.0, band));
    p.rect_filled(
        rect,
        egui::CornerRadius::ZERO,
        Color32::from_rgba_unmultiplied(217, 119, 6, 28),
    );
    p.line_segment(
        [t.p(-120.0, band), t.p(span + 120.0, band)],
        Stroke::new(1.2_f32, BAND),
    );
}

/// 左侧高度标注（竖向尺寸链 + 标高值）
///
/// 按建筑制图：**高度是竖向尺寸**——左侧一条竖向尺寸线，
/// 各标高处画 45° 刻度，数值写在刻度左侧；
/// 另以极细的水平线引到刚架（仅作标高参照，不做尺寸线）。
fn draw_elevations(
    p: &egui::Painter,
    t: &Transform,
    geo: &SectionGeometry,
    span: f64,
    eave: f64,
    ridge: f64,
) {
    // 竖向尺寸线位置
    let dim_x = -1_150.0;
    // 参照线右端（只引到刚架左柱，不再横贯全图）
    let ref_end = span * 0.14;

    let mut marks: Vec<(f64, String)> = vec![
        (0.0, "±0.00".to_string()),
        (eave, format!("{:.0}", eave)),
        (ridge, format!("{:.0}", ridge)),
    ];
    if geo.wall_band_mm > 0.0 {
        marks.push((geo.wall_band_mm, format!("{:.0}", geo.wall_band_mm)));
    }
    if geo.has_ceiling && geo.ceiling_mm > 0.0 {
        marks.push((geo.ceiling_mm, format!("{:.0}", geo.ceiling_mm)));
    }
    marks.sort_by(|a, b| a.0.partial_cmp(&b.0).unwrap_or(core::cmp::Ordering::Equal));

    let y_min = marks.first().map(|m| m.0).unwrap_or(0.0);
    let y_max = marks.last().map(|m| m.0).unwrap_or(ridge);

    // 竖向尺寸线（高度方向）
    p.line_segment(
        [t.p(dim_x, y_min), t.p(dim_x, y_max)],
        Stroke::new(1.2_f32, DIM),
    );

    for (y, label) in &marks {
        // 极细水平参照线：仅从尺寸线引到左柱附近
        p.line_segment(
            [t.p(dim_x, *y), t.p(ref_end, *y)],
            Stroke::new(1.0_f32, DIM_LINE),
        );
        // 45° 建筑刻度（在竖向尺寸线上）
        let a = t.p(dim_x - 90.0, *y - 90.0);
        let b = t.p(dim_x + 90.0, *y + 90.0);
        p.line_segment([a, b], Stroke::new(1.8_f32, DIM));
        // 数值写在刻度左侧
        p.text(
            t.p(dim_x - 150.0, *y),
            Align2::RIGHT_CENTER,
            label,
            FontId::proportional(13.0),
            DIM,
        );
    }
}

/// 底部跨度标注（18000）
fn draw_span_dimension(p: &egui::Painter, t: &Transform, span: f64) {
    let y = -1_150.0;
    p.line_segment([t.p(0.0, y), t.p(span, y)], Stroke::new(1.0_f32, DIM_LINE));
    // 端部引出线
    for x in [0.0, span] {
        p.line_segment(
            [t.p(x, y - 160.0), t.p(x, y + 160.0)],
            Stroke::new(1.4_f32, DIM),
        );
    }
    p.text(
        t.p(span / 2.0, y - 120.0),
        Align2::CENTER_BOTTOM,
        format!("{:.0}", span),
        FontId::proportional(13.0),
        DIM,
    );
}

/// 右上角舍长标注
fn draw_length_note(p: &egui::Painter, t: &Transform, length_mm: f64, ridge: f64) {
    p.text(
        t.p(1_400.0, ridge + 700.0),
        Align2::LEFT_TOP,
        format!("舍长 {:.0}", length_mm),
        FontId::proportional(12.0),
        Color32::from_rgb(100, 116, 139),
    );
}

#[cfg(test)]
mod tests {
    use super::*;

    /// 变换必须是 y 轴翻转、等比例
    #[test]
    fn transform_preserves_aspect() {
        let rect = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(800.0, 400.0));
        let t = Transform::new(rect, (0.0, 1000.0, 0.0, 500.0));
        // 世界 1000mm 宽 → 应小于等于 800px
        assert!(t.len(1000.0) <= 800.0);
        // 等比例：1000mm 宽与 500mm 高按同一 scale
        assert!((t.len(1000.0) - 2.0 * t.len(500.0)).abs() < 0.01);
    }

    /// y 轴翻转：世界 y 越大，屏幕 y 越小
    #[test]
    fn transform_flips_y() {
        let rect = Rect::from_min_size(Pos2::new(0.0, 0.0), Vec2::new(400.0, 400.0));
        let t = Transform::new(rect, (0.0, 100.0, 0.0, 100.0));
        assert!(t.p(0.0, 100.0).y < t.p(0.0, 0.0).y);
    }

    /// 默认几何必须是正数
    #[test]
    fn default_geometry_valid() {
        let g = SectionGeometry::default();
        assert!(g.ridge_mm > g.eave_mm);
        assert!(g.span_mm > 0.0);
    }
}
