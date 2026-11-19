//! Reusable drawing pieces: theme, cards, charts, sparklines, bars and tables.

use crate::collector::Severity;
use crate::util::{self, json_str, Ring};
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Rounding, Sense, Shape, Stroke, Ui, Vec2};
use serde_json::Value;

pub const CYAN: Color32 = Color32::from_rgb(56, 189, 248);
pub const BLUE: Color32 = Color32::from_rgb(96, 140, 255);
pub const PURPLE: Color32 = Color32::from_rgb(168, 120, 255);
pub const GREEN: Color32 = Color32::from_rgb(74, 222, 128);
pub const ORANGE: Color32 = Color32::from_rgb(251, 146, 60);
pub const YELLOW: Color32 = Color32::from_rgb(250, 204, 21);
pub const RED: Color32 = Color32::from_rgb(248, 113, 113);
pub const PINK: Color32 = Color32::from_rgb(244, 114, 182);
pub const TEAL: Color32 = Color32::from_rgb(45, 212, 191);

// ------------------------------------------------------------ live theme

/// The look currently in use (colours, chart style, density); changed from Settings.
#[derive(Clone, Copy, Debug)]
pub struct Look {
    pub bg: Color32,
    pub panel: Color32,
    pub card: Color32,
    pub card_hi: Color32,
    pub border: Color32,
    pub text: Color32,
    pub dim: Color32,
    pub accent: Color32,
    pub light: bool,
    pub fill: bool,
    pub grid: bool,
    pub smooth: bool,
    pub line: f32,
    pub row_h: f32,
    pub rounding: f32,
    pub heat: bool,
    pub row_colors: bool,
    pub net_bits: bool,
}

impl Look {
    pub const DEFAULT: Look = Look {
        bg: Color32::from_rgb(13, 17, 23),
        panel: Color32::from_rgb(22, 27, 34),
        card: Color32::from_rgb(28, 34, 43),
        card_hi: Color32::from_rgb(36, 44, 56),
        border: Color32::from_rgb(48, 56, 68),
        text: Color32::from_rgb(220, 226, 235),
        dim: Color32::from_rgb(139, 150, 165),
        accent: Color32::from_rgb(56, 189, 248),
        light: false,
        fill: true,
        grid: true,
        smooth: false,
        line: 1.6,
        row_h: 21.0,
        rounding: 10.0,
        heat: true,
        row_colors: true,
        net_bits: true,
    };
}

static LOOK: std::sync::RwLock<Look> = std::sync::RwLock::new(Look::DEFAULT);

pub fn look() -> Look {
    *LOOK.read().unwrap()
}
pub fn c_bg() -> Color32 {
    look().bg
}
pub fn c_panel() -> Color32 {
    look().panel
}
pub fn c_card() -> Color32 {
    look().card
}
pub fn c_card_hi() -> Color32 {
    look().card_hi
}
pub fn c_border() -> Color32 {
    look().border
}
pub fn c_text() -> Color32 {
    look().text
}
pub fn c_dim() -> Color32 {
    look().dim
}
pub fn accent() -> Color32 {
    look().accent
}
pub fn row_h() -> f32 {
    look().row_h
}

pub const THEMES: [&str; 6] = ["Midnight", "Graphite", "Ocean", "Forest", "Light", "High contrast"];
pub const ACCENTS: [(&str, [u8; 3]); 8] = [
    ("Sky", [56, 189, 248]),
    ("Violet", [168, 120, 255]),
    ("Emerald", [52, 211, 153]),
    ("Amber", [251, 191, 36]),
    ("Rose", [251, 113, 133]),
    ("Orange", [251, 146, 60]),
    ("Indigo", [129, 140, 248]),
    ("Teal", [45, 212, 191]),
];
pub const DENSITIES: [&str; 3] = ["Compact", "Comfortable", "Spacious"];

fn palette(theme: &str) -> [Color32; 7] {
    let c = Color32::from_rgb;
    match theme {
        "Graphite" => [c(18, 18, 20), c(26, 26, 29), c(33, 33, 37), c(44, 44, 49), c(58, 58, 64), c(230, 230, 232), c(150, 150, 158)],
        "Ocean" => [c(7, 18, 28), c(11, 27, 41), c(15, 35, 53), c(22, 48, 70), c(34, 66, 92), c(214, 232, 245), c(128, 160, 184)],
        "Forest" => [c(10, 20, 15), c(15, 28, 21), c(20, 37, 28), c(28, 50, 38), c(42, 70, 54), c(220, 236, 226), c(138, 168, 150)],
        "Light" => [c(234, 238, 244), c(247, 248, 251), c(255, 255, 255), c(229, 236, 246), c(208, 214, 224), c(26, 32, 40), c(96, 106, 122)],
        "High contrast" => [c(0, 0, 0), c(8, 8, 8), c(14, 14, 14), c(40, 40, 40), c(210, 210, 210), c(255, 255, 255), c(215, 215, 215)],
        _ => [c(13, 17, 23), c(22, 27, 34), c(28, 34, 43), c(36, 44, 56), c(48, 56, 68), c(220, 226, 235), c(139, 150, 165)],
    }
}

/// Apply the person's appearance preferences to egui and to the live look.
pub fn apply_prefs(ctx: &egui::Context, p: &crate::config::UiPrefs) {
    let [bg, panel, card, card_hi, border, text, dim] = palette(&p.theme);
    let light = p.theme == "Light";
    let accent = Color32::from_rgb(p.accent[0], p.accent[1], p.accent[2]);
    let (row_h, spacing_y, pad) = match p.density.as_str() {
        "Compact" => (18.0, 3.0, Vec2::new(7.0, 2.0)),
        "Spacious" => (26.0, 9.0, Vec2::new(12.0, 6.0)),
        _ => (21.0, 6.0, Vec2::new(10.0, 4.0)),
    };
    *LOOK.write().unwrap() = Look {
        bg,
        panel,
        card,
        card_hi,
        border,
        text,
        dim,
        accent,
        light,
        fill: p.chart_fill,
        grid: p.chart_grid,
        smooth: p.chart_smooth,
        line: p.line_width.clamp(0.5, 4.0),
        row_h,
        rounding: p.rounding.clamp(0.0, 16.0),
        heat: p.heat_colors,
        row_colors: p.row_colors,
        net_bits: p.net_bits,
    };
    ctx.set_theme(if light { egui::Theme::Light } else { egui::Theme::Dark });
    let mut v = if light { egui::Visuals::light() } else { egui::Visuals::dark() };
    let strong = if light { Color32::BLACK } else { Color32::WHITE };
    let r = Rounding::same((p.rounding * 0.6).clamp(0.0, 10.0));
    v.panel_fill = panel;
    v.window_fill = panel;
    v.extreme_bg_color = bg;
    v.faint_bg_color = card_hi.gamma_multiply(if light { 0.6 } else { 0.5 });
    v.override_text_color = None;
    v.widgets.noninteractive.fg_stroke = Stroke::new(1.0, text);
    v.widgets.inactive.fg_stroke = Stroke::new(1.0, text);
    v.widgets.hovered.fg_stroke = Stroke::new(1.5, strong);
    v.widgets.active.fg_stroke = Stroke::new(2.0, strong);
    v.widgets.open.fg_stroke = Stroke::new(1.0, strong);
    v.selection.bg_fill = accent.gamma_multiply(if light { 0.35 } else { 0.45 });
    v.selection.stroke = Stroke::new(1.0, accent);
    v.hyperlink_color = accent;
    v.window_rounding = Rounding::same(p.rounding.clamp(0.0, 16.0));
    v.widgets.noninteractive.bg_fill = card;
    v.widgets.noninteractive.bg_stroke = Stroke::new(1.0, border);
    v.widgets.inactive.bg_fill = card_hi;
    v.widgets.inactive.weak_bg_fill = card_hi;
    v.widgets.inactive.rounding = r;
    v.widgets.hovered.bg_fill = card_hi.lerp_to_gamma(accent, 0.25);
    v.widgets.hovered.weak_bg_fill = card_hi.lerp_to_gamma(accent, 0.25);
    v.widgets.hovered.bg_stroke = Stroke::new(1.0, accent.gamma_multiply(0.6));
    v.widgets.hovered.rounding = r;
    v.widgets.active.bg_fill = accent.gamma_multiply(0.6);
    v.widgets.active.weak_bg_fill = accent.gamma_multiply(0.6);
    v.widgets.active.rounding = r;
    v.widgets.open.rounding = r;
    v.window_shadow.color = Color32::from_black_alpha(if light { 40 } else { 120 });
    ctx.set_visuals(v);
    let mut style = (*ctx.style()).clone();
    let fs = p.font_size.clamp(10.0, 20.0);
    style.spacing.item_spacing = Vec2::new(8.0, spacing_y);
    style.spacing.button_padding = pad;
    style.spacing.interact_size.y = (row_h + 1.0).max(18.0);
    style.animation_time = if p.animations { 0.12 } else { 0.0 };
    style.text_styles.insert(egui::TextStyle::Body, FontId::proportional(fs));
    style.text_styles.insert(egui::TextStyle::Button, FontId::proportional(fs));
    style.text_styles.insert(egui::TextStyle::Small, FontId::proportional((fs - 2.5).max(9.0)));
    style.text_styles.insert(egui::TextStyle::Heading, FontId::proportional(fs + 6.5));
    style.text_styles.insert(egui::TextStyle::Monospace, FontId::monospace(fs - 1.0));
    ctx.set_style(style);
    if (ctx.zoom_factor() - p.zoom).abs() > 0.001 {
        ctx.set_zoom_factor(p.zoom.clamp(0.6, 2.5));
    }
}

/// Network speed in the unit the person picked (bits or bytes per second).
pub fn net_rate(bytes_per_sec: f64) -> String {
    if look().net_bits {
        util::fmt_bits(bytes_per_sec)
    } else {
        util::fmt_rate(bytes_per_sec)
    }
}
pub fn sev_color(s: Severity) -> Color32 {
    match s {
        Severity::Info => BLUE,
        Severity::Warning => YELLOW,
        Severity::Critical => RED,
    }
}

pub fn sev_label(s: Severity) -> &'static str {
    match s {
        Severity::Info => "Info",
        Severity::Warning => "Warning",
        Severity::Critical => "Critical",
    }
}

pub fn load_color(pct: f32) -> Color32 {
    if pct >= 90.0 {
        RED
    } else if pct >= 70.0 {
        ORANGE
    } else if pct >= 40.0 {
        YELLOW
    } else {
        GREEN
    }
}

/// Heat tint for table cells (like Task Manager's yellow ➡ orange ➡ red).
pub fn heat(pct: f32) -> Color32 {
    let p = (pct / 100.0).clamp(0.0, 1.0);
    if p < 0.04 {
        return Color32::TRANSPARENT;
    }
    let a = (20.0 + p * 150.0) as u8;
    if p < 0.5 {
        Color32::from_rgba_unmultiplied(250, 204, 21, a)
    } else if p < 0.8 {
        Color32::from_rgba_unmultiplied(251, 146, 60, a)
    } else {
        Color32::from_rgba_unmultiplied(248, 113, 113, a)
    }
}

pub fn card<R>(ui: &mut Ui, title: &str, add: impl FnOnce(&mut Ui) -> R) -> R {
    egui::Frame::none()
        .fill(c_card())
        .rounding(look().rounding)
        .stroke(Stroke::new(1.0, c_border()))
        .inner_margin(egui::Margin::same(12.0))
        .show(ui, |ui| {
            if !title.is_empty() {
                ui.label(RichText::new(title).strong().size(14.0).color(c_text()));
                ui.add_space(4.0);
            }
            add(ui)
        })
        .inner
}

pub fn page_header(ui: &mut Ui, title: &str, subtitle: &str) {
    ui.horizontal(|ui| {
        ui.label(RichText::new(title).size(22.0).strong());
    });
    if !subtitle.is_empty() {
        ui.label(RichText::new(subtitle).color(c_dim()));
    }
    ui.add_space(6.0);
}

pub fn pill(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(color.gamma_multiply(0.18))
        .rounding(9.0)
        .inner_margin(egui::Margin::symmetric(8.0, 2.0))
        .show(ui, |ui| {
            ui.label(RichText::new(text).color(color).size(11.5).strong());
        });
}

pub fn notice(ui: &mut Ui, text: &str, color: Color32) {
    egui::Frame::none()
        .fill(color.gamma_multiply(0.12))
        .stroke(Stroke::new(1.0, color.gamma_multiply(0.5)))
        .rounding(8.0)
        .inner_margin(egui::Margin::symmetric(10.0, 6.0))
        .show(ui, |ui| {
            ui.set_width(ui.available_width());
            ui.label(RichText::new(text).color(color));
        });
}

/// Horizontal usage bar with text on top.
pub fn bar(ui: &mut Ui, frac: f32, color: Color32, text: &str, width: f32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(width, 16.0), Sense::hover());
    let p = ui.painter();
    p.rect_filled(rect, 4.0, c_bg());
    let mut fill = rect;
    fill.set_width(rect.width() * frac.clamp(0.0, 1.0));
    p.rect_filled(fill, 4.0, color.gamma_multiply(0.85));
    p.text(rect.center(), Align2::CENTER_CENTER, text, FontId::proportional(11.0), c_text());
}

pub fn sparkline(ui: &mut Ui, data: &[f32], color: Color32, size: Vec2, max: Option<f32>) -> egui::Response {
    let (rect, resp) = ui.allocate_exact_size(size, Sense::hover());
    draw_series(ui.painter(), rect, data, color, max.unwrap_or_else(|| data.iter().cloned().fold(1e-6, f32::max)), true, data.len().max(2));
    resp
}

fn draw_series(p: &egui::Painter, rect: Rect, data: &[f32], color: Color32, max: f32, fill: bool, points: usize) {
    if data.len() < 2 {
        return;
    }
    let max = max.max(1e-6);
    let step = rect.width() / (points.max(2) - 1) as f32;
    let offset = points.saturating_sub(data.len());
    let pts: Vec<Pos2> = data
        .iter()
        .enumerate()
        .map(|(i, &v)| {
            let x = rect.left() + (i + offset) as f32 * step;
            let y = rect.bottom() - (v / max).clamp(0.0, 1.0) * rect.height();
            Pos2::new(x, y)
        })
        .collect();
    let pts = if look().smooth { smooth(&pts) } else { pts };
    if fill && look().fill {
        let mut mesh = egui::Mesh::default();
        let top = color.gamma_multiply(0.35);
        let bot = color.gamma_multiply(0.04);
        for w in pts.windows(2) {
            let i = mesh.vertices.len() as u32;
            mesh.colored_vertex(w[0], top);
            mesh.colored_vertex(w[1], top);
            mesh.colored_vertex(Pos2::new(w[1].x, rect.bottom()), bot);
            mesh.colored_vertex(Pos2::new(w[0].x, rect.bottom()), bot);
            mesh.add_triangle(i, i + 1, i + 2);
            mesh.add_triangle(i, i + 2, i + 3);
        }
        p.add(Shape::mesh(mesh));
    }
    p.add(Shape::line(pts, Stroke::new(look().line, color)));
}

pub struct Series<'a> {
    pub data: Vec<f32>,
    pub color: Color32,
    pub label: &'a str,
}

/// Line chart with grid, legend and hover read-out.
pub fn chart(ui: &mut Ui, series: &[Series], height: f32, fixed_max: Option<f32>, points: usize, fmt: &dyn Fn(f32) -> String) {
    let width = ui.available_width().max(100.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(width, height), Sense::hover());
    let p = ui.painter_at(rect.expand(2.0));
    p.rect_filled(rect, 6.0, c_bg());
    let max = fixed_max.unwrap_or_else(|| {
        let m = series.iter().flat_map(|s| s.data.iter().cloned()).fold(0.0, f32::max);
        nice_max(m)
    });
    let plot = Rect::from_min_max(Pos2::new(rect.left() + 4.0, rect.top() + 6.0), Pos2::new(rect.right() - 4.0, rect.bottom() - 4.0));
    for i in (1..4).filter(|_| look().grid) {
        let y = plot.top() + plot.height() * i as f32 / 4.0;
        p.line_segment([Pos2::new(plot.left(), y), Pos2::new(plot.right(), y)], Stroke::new(1.0, c_border().gamma_multiply(0.45)));
    }
    for i in (1..12).filter(|_| look().grid) {
        let x = plot.left() + plot.width() * i as f32 / 12.0;
        p.line_segment([Pos2::new(x, plot.top()), Pos2::new(x, plot.bottom())], Stroke::new(1.0, c_border().gamma_multiply(0.25)));
    }
    for s in series {
        let d: Vec<f32> = if s.data.len() > points { s.data[s.data.len() - points..].to_vec() } else { s.data.clone() };
        draw_series(&p, plot, &d, s.color, max, true, points);
    }
    p.text(Pos2::new(plot.right() - 4.0, plot.top() + 2.0), Align2::RIGHT_TOP, fmt(max), FontId::proportional(10.5), c_dim());
    // legend
    let mut x = plot.left() + 6.0;
    for s in series {
        if s.label.is_empty() {
            continue;
        }
        let last = s.data.last().copied().unwrap_or(0.0);
        let txt = format!("{} {}", s.label, fmt(last));
        p.circle_filled(Pos2::new(x + 4.0, plot.top() + 9.0), 4.0, s.color);
        let r = p.text(Pos2::new(x + 12.0, plot.top() + 2.0), Align2::LEFT_TOP, txt, FontId::proportional(11.0), c_text());
        x = r.right() + 14.0;
    }
    if let Some(pos) = resp.hover_pos() {
        let step = plot.width() / (points.max(2) - 1) as f32;
        let idx_from_right = ((plot.right() - pos.x) / step).round() as usize;
        p.line_segment([Pos2::new(pos.x, plot.top()), Pos2::new(pos.x, plot.bottom())], Stroke::new(1.0, c_dim()));
        let mut lines = vec![];
        for s in series {
            if idx_from_right < s.data.len() {
                let v = s.data[s.data.len() - 1 - idx_from_right];
                lines.push(format!("{}: {}", if s.label.is_empty() { "value" } else { s.label }, fmt(v)));
            }
        }
        lines.push(format!("{} samples ago", idx_from_right));
        egui::show_tooltip_at_pointer(ui.ctx(), ui.layer_id(), egui::Id::new("chart_tip").with(rect.min.x as i32).with(rect.min.y as i32), |ui| {
            for l in lines {
                ui.label(l);
            }
        });
    }
}

/// Light smoothing (3-point average) for chart lines.
fn smooth(pts: &[Pos2]) -> Vec<Pos2> {
    if pts.len() < 3 {
        return pts.to_vec();
    }
    let mut out = Vec::with_capacity(pts.len());
    out.push(pts[0]);
    for w in pts.windows(3) {
        out.push(Pos2::new(w[1].x, (w[0].y + w[1].y * 2.0 + w[2].y) / 4.0));
    }
    out.push(pts[pts.len() - 1]);
    out
}

pub fn nice_max(m: f32) -> f32 {
    if m <= 0.0 {
        return 1.0;
    }
    let exp = m.log10().floor();
    let base = 10f32.powf(exp);
    let n = m / base;
    let nice = if n <= 1.0 {
        1.0
    } else if n <= 2.0 {
        2.0
    } else if n <= 5.0 {
        5.0
    } else {
        10.0
    };
    nice * base
}

pub fn pct(v: f32) -> String {
    format!("{v:.0}%")
}

pub fn rate(v: f32) -> String {
    util::fmt_rate(v as f64)
}

pub fn bytes(v: f32) -> String {
    util::fmt_bytes(v as f64)
}

pub fn ring_vec(r: &Ring, n: usize) -> Vec<f32> {
    r.to_vec(n)
}

/// Big tile with a value, a caption and a sparkline.
pub fn tile(ui: &mut Ui, title: &str, value: &str, sub: &str, color: Color32, data: &[f32], max: Option<f32>, selected: bool) -> egui::Response {
    let w = ui.available_width();
    let fill = if selected { c_card_hi() } else { c_card() };
    let r = egui::Frame::none()
        .fill(fill)
        .rounding(10.0)
        .stroke(Stroke::new(if selected { 1.5 } else { 1.0 }, if selected { color } else { c_border() }))
        .inner_margin(egui::Margin::same(10.0))
        .show(ui, |ui| {
            ui.set_width(w - 22.0);
            ui.horizontal(|ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(title).color(c_dim()).size(12.0));
                    ui.label(RichText::new(value).color(color).size(22.0).strong());
                    ui.label(RichText::new(sub).color(c_dim()).size(11.0));
                });
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    let sw = (ui.available_width() - 4.0).clamp(40.0, 220.0);
                    sparkline(ui, data, color, Vec2::new(sw, 48.0), max);
                });
            });
        });
    r.response.interact(Sense::click())
}

pub fn kv(ui: &mut Ui, id: &str, rows: &[(&str, String)]) {
    egui::Grid::new(id).num_columns(2).spacing([16.0, 4.0]).striped(true).show(ui, |ui| {
        for (k, v) in rows {
            ui.label(RichText::new(*k).color(c_dim()));
            ui.add(egui::Label::new(v.as_str()).wrap());
            ui.end_row();
        }
    });
}

pub fn search_field(ui: &mut Ui, text: &mut String, hint: &str, width: f32) -> egui::Response {
    ui.add(egui::TextEdit::singleline(text).hint_text(hint).desired_width(width))
}

// ------------------------------------------------------------ JSON tables

#[derive(Default)]
pub struct TableState {
    pub filter: String,
    pub sort_col: Option<usize>,
    pub asc: bool,
    pub selected: Option<usize>,
}

pub struct Col {
    pub key: std::borrow::Cow<'static, str>,
    pub label: std::borrow::Cow<'static, str>,
    pub width: f32,
}

pub fn col(key: &'static str, label: &'static str, width: f32) -> Col {
    Col { key: key.into(), label: label.into(), width }
}

fn cmp_json(a: &Value, b: &Value) -> std::cmp::Ordering {
    match (a.as_f64(), b.as_f64()) {
        (Some(x), Some(y)) => x.total_cmp(&y),
        _ => json_str(a).to_lowercase().cmp(&json_str(b).to_lowercase()),
    }
}

/// Filterable, sortable table of JSON objects. Returns the index (into `rows`) of the selected row.
pub fn json_table(
    ui: &mut Ui,
    id: &str,
    rows: &[Value],
    cols: &[Col],
    st: &mut TableState,
    tint: &dyn Fn(&Value) -> Option<Color32>,
    height: f32,
) -> Option<usize> {
    let f = st.filter.to_lowercase();
    let mut idx: Vec<usize> = (0..rows.len())
        .filter(|&i| {
            f.is_empty()
                || cols.iter().any(|c| json_str(util::jget(&rows[i], &c.key)).to_lowercase().contains(&f))
        })
        .collect();
    if let Some(sc) = st.sort_col {
        if let Some(c) = cols.get(sc) {
            idx.sort_by(|&a, &b| {
                let o = cmp_json(util::jget(&rows[a], &c.key), util::jget(&rows[b], &c.key));
                if st.asc {
                    o
                } else {
                    o.reverse()
                }
            });
        }
    }
    ui.label(RichText::new(format!("{} of {} items", idx.len(), rows.len())).color(c_dim()).size(11.0));
    let mut table = egui_extras::TableBuilder::new(ui)
        .id_salt(id)
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .max_scroll_height(height)
        .min_scrolled_height(height.min(200.0))
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
    for (i, c) in cols.iter().enumerate() {
        table = if i + 1 == cols.len() {
            table.column(egui_extras::Column::remainder().at_least(c.width.min(120.0)).clip(true))
        } else {
            table.column(egui_extras::Column::initial(c.width).at_least(30.0).clip(true))
        };
    }
    let mut clicked_header = None;
    let mut new_sel = st.selected;
    table
        .header(22.0, |mut h| {
            for (i, c) in cols.iter().enumerate() {
                h.col(|ui| {
                    let arrow = if st.sort_col == Some(i) { if st.asc { " 🔼" } else { " 🔽" } } else { "" };
                    if ui.add(egui::Label::new(RichText::new(format!("{}{arrow}", c.label)).strong()).sense(Sense::click())).clicked() {
                        clicked_header = Some(i);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(row_h(), idx.len(), |mut row| {
                let ri = idx[row.index()];
                let r = &rows[ri];
                row.set_selected(st.selected == Some(ri));
                let tc = tint(r);
                for c in cols {
                    row.col(|ui| {
                        let s = json_str(util::jget(r, &c.key));
                        let t = RichText::new(s.replace('\n', " ")).size(12.5);
                        let t = if let Some(tc) = tc { t.color(tc) } else { t };
                        ui.add(egui::Label::new(t).truncate());
                    });
                }
                if row.response().clicked() {
                    new_sel = Some(ri);
                }
            });
        });
    if let Some(i) = clicked_header {
        if st.sort_col == Some(i) {
            st.asc = !st.asc;
        } else {
            st.sort_col = Some(i);
            st.asc = true;
        }
    }
    st.selected = new_sel;
    st.selected.filter(|&i| i < rows.len())
}

/// Spinner or error text for a job; returns true when the data is ready.
pub fn job_status<T>(ui: &mut Ui, job: &util::Job<T>) -> bool {
    job.with(|r| match r {
        None => {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new("Reading from Windows…").color(c_dim()));
            });
            false
        }
        Some(Err(e)) => {
            notice(ui, &format!("Could not read this: {e}"), RED);
            false
        }
        Some(Ok(_)) => true,
    })
}

pub fn value_rows(v: &Value) -> Vec<(String, String)> {
    match v {
        Value::Object(o) => o.iter().map(|(k, v)| (k.clone(), json_str(v))).collect(),
        _ => vec![],
    }
}

pub fn value_list(v: &Value) -> Vec<Value> {
    match v {
        Value::Array(a) => a.clone(),
        Value::Null => vec![],
        o => vec![o.clone()],
    }
}

/// Table of all keys found in a list of objects (for ad-hoc data).
pub fn auto_table(ui: &mut Ui, id: &str, rows: &[Value], st: &mut TableState, height: f32) -> Option<usize> {
    let mut keys: Vec<String> = vec![];
    for r in rows.iter().take(50) {
        if let Value::Object(o) = r {
            for k in o.keys() {
                if !keys.iter().any(|x| x == k) {
                    keys.push(k.clone());
                }
            }
        }
    }
    let cols: Vec<Col> = keys.into_iter().map(|k| Col { key: k.clone().into(), label: k.into(), width: 140.0 }).collect();
    json_table(ui, id, rows, &cols, st, &|_| None, height)
}
