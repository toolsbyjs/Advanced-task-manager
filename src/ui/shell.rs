//! The window shell: top bar, sidebar (favorites, groups, compact mode), status bar,
//! command palette, keyboard shortcuts and live preference syncing.

use super::widgets::*;
use super::{page_by_key, page_key, App, Confirm, Mode, Page, PageInfo, NAV};
use crate::util;
use egui::{Align2, FontId, Key, Modifiers, RichText, Sense, Stroke, Vec2};

pub const SHORTCUTS: &[(&str, &str)] = &[
    ("Ctrl+K", "Command palette: jump to any page, process or action"),
    ("Ctrl+F", "Search processes"),
    ("Ctrl+P", "Pause or resume live updates"),
    ("Ctrl+B", "Collapse or expand the sidebar"),
    ("Ctrl+1 … Ctrl+9", "Open your favorite pages in order"),
    ("Ctrl + / Ctrl − / Ctrl 0", "Zoom in, out, reset"),
    ("Delete", "End the selected process (asks first)"),
    ("Esc", "Close dialogs and process details"),
    ("F1", "Show this list"),
];

enum PalItem {
    Page(Page),
    Proc(u32, String),
    Action(&'static str, &'static str),
}

const ACTIONS: &[(&str, &str)] = &[
    ("pause", "Pause / resume live updates"),
    ("record", "Start / stop performance recording"),
    ("snapshot", "Take a configuration snapshot"),
    ("report", "Save a health report"),
    ("standard", "Switch to Standard view"),
    ("admin", "Switch to Admin view"),
    ("sidebar", "Collapse / expand sidebar"),
    ("theme", "Next colour theme"),
    ("density", "Next layout density"),
    ("tree", "Toggle process tree view"),
    ("ack", "Acknowledge all alerts"),
    ("help", "Show keyboard shortcuts"),
    ("settings", "Open settings"),
];

impl App {
    /// Apply preference edits (from Settings, palette or shortcuts) and keep zoom in sync.
    pub fn sync_prefs(&mut self, ctx: &egui::Context) {
        if self.shot.is_none() {
            let z = ctx.zoom_factor();
            if (z - self.prefs.zoom).abs() > 0.001 && self.applied.as_ref().map(|a| (a.zoom - self.prefs.zoom).abs() < 0.001).unwrap_or(true) {
                self.prefs.zoom = z;
            }
        }
        if self.applied.as_ref() != Some(&self.prefs) {
            apply_prefs(ctx, &self.prefs);
            self.applied = Some(self.prefs.clone());
            let mut s = self.shared.lock().unwrap();
            s.config.ui = self.prefs.clone();
            s.config_dirty = true;
        }
    }

    pub fn favorites(&self) -> Vec<Page> {
        self.prefs.favorites.iter().filter_map(|k| page_by_key(k)).collect()
    }

    pub fn toggle_favorite(&mut self, p: Page) {
        let k = page_key(p);
        if let Some(i) = self.prefs.favorites.iter().position(|x| *x == k) {
            self.prefs.favorites.remove(i);
        } else {
            self.prefs.favorites.push(k);
        }
    }

    pub fn shortcuts(&mut self, ctx: &egui::Context) {
        let typing = ctx.memory(|m| m.focused().is_some());
        let (pal, find, pause, side, help, esc, del, nums) = ctx.input_mut(|i| {
            let mut nums = None;
            for (n, k) in [Key::Num1, Key::Num2, Key::Num3, Key::Num4, Key::Num5, Key::Num6, Key::Num7, Key::Num8, Key::Num9].iter().enumerate() {
                if i.consume_key(Modifiers::COMMAND, *k) {
                    nums = Some(n);
                }
            }
            (
                i.consume_key(Modifiers::COMMAND, Key::K),
                i.consume_key(Modifiers::COMMAND, Key::F),
                i.consume_key(Modifiers::COMMAND, Key::P),
                i.consume_key(Modifiers::COMMAND, Key::B),
                i.consume_key(Modifiers::NONE, Key::F1),
                i.key_pressed(Key::Escape),
                i.key_pressed(Key::Delete),
                nums,
            )
        });
        if pal {
            self.palette.open = !self.palette.open;
            self.palette.query.clear();
            self.palette.sel = 0;
            self.palette.just_opened = true;
        }
        if find {
            self.page = Page::Processes;
            self.procs.focus_search = true;
        }
        if pause {
            self.run_action("pause", ctx);
        }
        if side {
            self.prefs.nav_compact = !self.prefs.nav_compact;
        }
        if help {
            self.show_help = !self.show_help;
        }
        if esc {
            if self.palette.open {
                self.palette.open = false;
            } else if self.show_help {
                self.show_help = false;
            } else if self.confirm.is_some() {
                self.confirm = None;
            } else if self.page == Page::Processes {
                self.procs.selected = None;
            }
        }
        if let Some(n) = nums {
            if let Some(p) = self.favorites().get(n) {
                self.page = *p;
            }
        }
        if del && !typing && self.page == Page::Processes && !self.palette.open {
            if let Some(pid) = self.procs.selected {
                let name = self.shared.lock().unwrap().procs.get(&pid).map(|p| p.name.clone());
                if let Some(n) = name {
                    self.confirm = Some(Confirm::Kill(pid, n));
                }
            }
        }
    }

    pub fn run_action(&mut self, a: &str, ctx: &egui::Context) {
        match a {
            "pause" => {
                let mut s = self.shared.lock().unwrap();
                s.paused = !s.paused;
                let p = s.paused;
                drop(s);
                self.notify(if p { "Live updates paused" } else { "Live updates resumed" }, false);
            }
            "record" => {
                let mut s = self.shared.lock().unwrap();
                if s.recorder.is_some() {
                    s.stop_recording();
                    drop(s);
                    self.notify("Recording saved", false);
                } else {
                    let r = s.start_recording(None);
                    drop(s);
                    self.result(r.map(|p| format!("Recording to {}", p.display())));
                }
            }
            "snapshot" => super::manage::take_snapshot(self, ctx),
            "report" => super::insights::write_report(self, true),
            "standard" => self.mode = Mode::Standard,
            "admin" => self.request_admin(),
            "sidebar" => self.prefs.nav_compact = !self.prefs.nav_compact,
            "theme" => {
                let i = THEMES.iter().position(|t| *t == self.prefs.theme).unwrap_or(0);
                self.prefs.theme = THEMES[(i + 1) % THEMES.len()].to_string();
                self.notify(format!("Theme: {}", self.prefs.theme), false);
            }
            "density" => {
                let i = DENSITIES.iter().position(|t| *t == self.prefs.density).unwrap_or(1);
                self.prefs.density = DENSITIES[(i + 1) % DENSITIES.len()].to_string();
                self.notify(format!("Density: {}", self.prefs.density), false);
            }
            "tree" => {
                let mut s = self.shared.lock().unwrap();
                s.config.tree_view = !s.config.tree_view;
                s.config_dirty = true;
                self.page = Page::Processes;
            }
            "ack" => {
                let mut s = self.shared.lock().unwrap();
                for al in s.alerts.iter_mut() {
                    al.acknowledged = true;
                }
            }
            "help" => self.show_help = true,
            "settings" => self.page = Page::Settings,
            _ => {}
        }
    }

    pub fn top_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::top("top")
            .frame(egui::Frame::none().fill(c_panel()).inner_margin(egui::Margin::symmetric(12.0, 7.0)).stroke(Stroke::new(1.0, c_border())))
            .show(ctx, |ui| {
                ui.horizontal(|ui| {
                    if ui.add(egui::Button::new(RichText::new("☰").size(16.0)).frame(false)).on_hover_text("Collapse or expand the sidebar (Ctrl+B)").clicked() {
                        self.prefs.nav_compact = !self.prefs.nav_compact;
                    }
                    ui.label(RichText::new("💠").size(20.0).color(accent()));
                    ui.vertical(|ui| {
                        ui.label(RichText::new("Advanced TaskManager").size(15.5).strong());
                        let info = super::page_info(self.page);
                        ui.label(RichText::new(format!("{} {}", info.icon, info.label)).size(11.0).color(c_dim()));
                    });
                    ui.add_space(14.0);
                    // view mode switch
                    egui::Frame::none().fill(c_bg()).rounding(8.0).inner_margin(egui::Margin::same(3.0)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            let std_sel = self.mode == Mode::Standard;
                            if ui.add(egui::SelectableLabel::new(std_sel, RichText::new("👤 Standard"))).on_hover_text("Simple view for everyday use").clicked() {
                                self.mode = Mode::Standard;
                            }
                            let adm = self.mode == Mode::Admin;
                            let lbl = if self.elevated { "🛡 Admin" } else { "🛡 Admin (restart)" };
                            if ui
                                .add(egui::SelectableLabel::new(adm, RichText::new(lbl).color(if adm { PURPLE } else { c_text() })))
                                .on_hover_text("Every detail: threads, handles, memory maps, live system events and full control. Needs administrator rights.")
                                .clicked()
                            {
                                self.request_admin();
                            }
                        });
                    });
                    ui.add_space(10.0);
                    // command palette launcher
                    let w = (ui.available_width() - 470.0).clamp(160.0, 380.0);
                    let (rect, resp) = ui.allocate_exact_size(Vec2::new(w, 26.0), Sense::click());
                    let hov = resp.hovered();
                    ui.painter().rect(rect, 7.0, c_bg(), Stroke::new(1.0, if hov { accent() } else { c_border() }));
                    ui.painter().text(rect.left_center() + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, "🔍 Search pages, processes, actions…", FontId::proportional(12.5), c_dim());
                    ui.painter().text(rect.right_center() - Vec2::new(8.0, 0.0), Align2::RIGHT_CENTER, "Ctrl+K", FontId::monospace(11.0), c_dim());
                    if resp.on_hover_cursor(egui::CursorIcon::PointingHand).clicked() {
                        self.palette.open = true;
                        self.palette.just_opened = true;
                        self.palette.query.clear();
                        self.palette.sel = 0;
                    }
                    let s = self.shared.lock().unwrap();
                    let cpu = s.cpu_total;
                    let mem = s.mem_used_pct();
                    let gpu = s.gpu.total;
                    let alerts = s.unacked();
                    let paused = s.paused;
                    let recording = s.recorder.is_some();
                    drop(s);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("⚙").on_hover_text("Settings").clicked() {
                            self.page = Page::Settings;
                        }
                        let bell = if alerts > 0 { format!("🔔 {alerts}") } else { "🔔".into() };
                        if ui.add(egui::Button::new(RichText::new(bell).color(if alerts > 0 { YELLOW } else { c_dim() }))).on_hover_text("Alerts").clicked() {
                            self.page = Page::Alerts;
                        }
                        let ptxt = if paused { "▶" } else { "⏸" };
                        if ui.button(ptxt).on_hover_text(if paused { "Resume live updates (Ctrl+P)" } else { "Pause live updates (Ctrl+P)" }).clicked() {
                            self.run_action("pause", ctx);
                        }
                        if recording {
                            pill(ui, "⏺ REC", RED);
                        }
                        mini_meter(ui, "GPU", gpu, PINK);
                        mini_meter(ui, "MEM", mem, PURPLE);
                        mini_meter(ui, "CPU", cpu, CYAN);
                    });
                });
            });
    }

    pub fn status_bar(&mut self, ctx: &egui::Context) {
        egui::TopBottomPanel::bottom("status")
            .exact_height(24.0)
            .frame(egui::Frame::none().fill(c_panel()).inner_margin(egui::Margin::symmetric(12.0, 3.0)).stroke(Stroke::new(1.0, c_border())))
            .show(ctx, |ui| {
                let s = self.shared.lock().unwrap();
                let live = !s.paused;
                let items = [
                    format!("{} {}", if live { "⏺" } else { "⏸" }, if live { "Live" } else { "Paused" }),
                    format!("every {} ms", s.config.sample_ms),
                    format!("last update {}", util::fmt_clock(s.last_sample)),
                    format!("{} processes • {} threads", s.procs.len(), util::fmt_num(s.mem.st.threads as u64)),
                    format!("monitor cost {:.0} ms/sample", s.sample_cost_ms),
                    format!("up {}", util::fmt_dur(s.sys.uptime)),
                ];
                let rec = s.recorder.as_ref().map(|r| format!("⏺ recording {} rows", r.rows));
                drop(s);
                ui.horizontal_centered(|ui| {
                    for (i, t) in items.iter().enumerate() {
                        let c = if i == 0 { if live { GREEN } else { YELLOW } } else { c_dim() };
                        ui.label(RichText::new(t).size(11.0).color(c));
                        ui.label(RichText::new("|").size(11.0).color(c_border()));
                    }
                    if let Some(r) = rec {
                        ui.label(RichText::new(r).size(11.0).color(RED));
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.add(egui::Label::new(RichText::new("F1 shortcuts").size(11.0).color(c_dim())).sense(Sense::click())).clicked() {
                            self.show_help = true;
                        }
                        ui.label(RichText::new(format!("{} view • {} • {:.0}%", if self.admin() { "Admin" } else { "Standard" }, self.prefs.theme, self.prefs.zoom * 100.0)).size(11.0).color(c_dim()));
                    });
                });
            });
    }

    pub fn side_nav(&mut self, ctx: &egui::Context) {
        let compact = self.prefs.nav_compact;
        egui::SidePanel::left("nav")
            .resizable(false)
            .exact_width(if compact { 54.0 } else { 220.0 })
            .frame(egui::Frame::none().fill(c_panel()).inner_margin(egui::Margin::symmetric(6.0, 8.0)).stroke(Stroke::new(1.0, c_border())))
            .show(ctx, |ui| {
                if !compact {
                    search_field(ui, &mut self.nav_filter, "🔍 Find a page…", ui.available_width());
                    ui.add_space(4.0);
                }
                egui::ScrollArea::vertical().auto_shrink([false, false]).show(ui, |ui| {
                    let f = self.nav_filter.to_lowercase();
                    let favs = self.favorites();
                    if !favs.is_empty() && f.is_empty() {
                        if !compact {
                            group_header(ui, "★ FAVORITES", false);
                        }
                        for (i, p) in favs.iter().enumerate() {
                            let info = super::page_info(*p);
                            self.nav_row(ui, info, compact, Some(i + 1));
                        }
                        ui.add_space(4.0);
                        ui.separator();
                    }
                    for (group, items) in NAV {
                        let visible: Vec<&PageInfo> = items
                            .iter()
                            .filter(|i| !self.prefs.hidden_pages.contains(&page_key(i.page)) || i.page == Page::Settings)
                            .filter(|i| f.is_empty() || i.label.to_lowercase().contains(&f))
                            .collect();
                        if visible.is_empty() {
                            continue;
                        }
                        let collapsed = !group.is_empty() && self.prefs.collapsed_groups.iter().any(|g| g == group) && f.is_empty();
                        if !group.is_empty() {
                            if compact {
                                ui.add_space(3.0);
                                ui.separator();
                            } else if group_header(ui, group, collapsed).clicked() {
                                if collapsed {
                                    self.prefs.collapsed_groups.retain(|g| g != group);
                                } else {
                                    self.prefs.collapsed_groups.push(group.to_string());
                                }
                            }
                        } else {
                            ui.separator();
                        }
                        if collapsed && !compact {
                            continue;
                        }
                        for item in visible {
                            self.nav_row(ui, item, compact, None);
                        }
                    }
                });
            });
    }

    fn nav_row(&mut self, ui: &mut egui::Ui, item: &PageInfo, compact: bool, fav_index: Option<usize>) {
        let sel = self.page == item.page;
        let locked = item.admin_only && !self.admin();
        let h = (row_h() + 5.0).max(24.0);
        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), h), Sense::click());
        let hov = resp.hovered();
        let p = ui.painter();
        if sel {
            p.rect_filled(rect, 6.0, accent().gamma_multiply(0.22));
            p.rect_filled(egui::Rect::from_min_size(rect.left_top() + Vec2::new(0.0, 4.0), Vec2::new(3.0, h - 8.0)), 2.0, accent());
        } else if hov {
            p.rect_filled(rect, 6.0, c_card_hi());
        }
        let color = if sel { accent() } else if locked { c_dim() } else { c_text() };
        let fs = ui.style().text_styles.get(&egui::TextStyle::Body).map(|f| f.size).unwrap_or(13.5);
        if compact {
            p.text(rect.center(), Align2::CENTER_CENTER, item.icon, FontId::proportional(fs + 3.0), color);
        } else {
            p.text(rect.left_center() + Vec2::new(12.0, 0.0), Align2::LEFT_CENTER, item.icon, FontId::proportional(fs + 1.0), color);
            p.text(rect.left_center() + Vec2::new(36.0, 0.0), Align2::LEFT_CENTER, item.label, FontId::proportional(fs), color);
            let mut right = rect.right() - 8.0;
            if let Some(n) = fav_index.filter(|n| *n <= 9) {
                p.text(egui::pos2(right, rect.center().y), Align2::RIGHT_CENTER, format!("^{n}"), FontId::monospace(10.0), c_dim());
                right -= 22.0;
            }
            if locked {
                p.text(egui::pos2(right, rect.center().y), Align2::RIGHT_CENTER, "🛡", FontId::proportional(11.0), PURPLE);
            }
        }
        let tip = format!(
            "{}{}\nRight-click to {} favorites",
            item.label,
            if locked { " (Admin view)" } else { "" },
            if self.prefs.favorites.contains(&page_key(item.page)) { "remove from" } else { "add to" }
        );
        let resp = resp.on_hover_text(tip);
        if resp.clicked() {
            self.page = item.page;
        }
        let page = item.page;
        resp.context_menu(|ui| {
            let is_fav = self.prefs.favorites.contains(&page_key(page));
            if ui.button(if is_fav { "☆ Remove from favorites" } else { "★ Add to favorites" }).clicked() {
                self.toggle_favorite(page);
                ui.close_menu();
            }
            if ui.button("🏠 Open this page at start-up").clicked() {
                self.prefs.start_page = page_key(page);
                ui.close_menu();
            }
            if page != Page::Settings && ui.button("Hide from sidebar").clicked() {
                self.prefs.hidden_pages.push(page_key(page));
                ui.close_menu();
            }
        });
    }

    pub fn command_palette(&mut self, ctx: &egui::Context) {
        if !self.palette.open {
            return;
        }
        let q = self.palette.query.to_lowercase();
        let mut items: Vec<(PalItem, String, String)> = vec![];
        for (_, list) in NAV {
            for i in list.iter() {
                if q.is_empty() || i.label.to_lowercase().contains(&q) {
                    items.push((PalItem::Page(i.page), format!("{}  {}", i.icon, i.label), "Page".into()));
                }
            }
        }
        for (k, l) in ACTIONS {
            if q.is_empty() || l.to_lowercase().contains(&q) {
                items.push((PalItem::Action(k, l), format!("⚡  {l}"), "Action".into()));
            }
        }
        if q.len() >= 2 {
            let s = self.shared.lock().unwrap();
            let mut ps: Vec<_> = s.procs.values().filter(|p| p.name.to_lowercase().contains(&q) || p.pid.to_string() == q).collect();
            ps.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
            for p in ps.iter().take(12) {
                items.push((PalItem::Proc(p.pid, p.name.clone()), format!("☰  {}  (PID {})", p.name, p.pid), format!("{:.1}% CPU • {}", p.cpu, util::fmt_bytes(p.private_bytes as f64))));
            }
        }
        let n = items.len();
        let (up, down, enter) = ctx.input_mut(|i| (i.consume_key(Modifiers::NONE, Key::ArrowUp), i.consume_key(Modifiers::NONE, Key::ArrowDown), i.consume_key(Modifiers::NONE, Key::Enter)));
        if down && n > 0 {
            self.palette.sel = (self.palette.sel + 1) % n;
        }
        if up && n > 0 {
            self.palette.sel = (self.palette.sel + n - 1) % n;
        }
        if self.palette.sel >= n {
            self.palette.sel = 0;
        }
        let mut chosen: Option<usize> = if enter && n > 0 { Some(self.palette.sel) } else { None };
        let mut close = false;
        egui::Area::new(egui::Id::new("palette_dim")).order(egui::Order::Foreground).fixed_pos(egui::pos2(0.0, 0.0)).show(ctx, |ui| {
            let r = ctx.screen_rect();
            let resp = ui.allocate_rect(r, Sense::click());
            ui.painter().rect_filled(r, 0.0, egui::Color32::from_black_alpha(110));
            if resp.clicked() {
                close = true;
            }
        });
        egui::Window::new("palette")
            .title_bar(false)
            .resizable(false)
            .order(egui::Order::Foreground)
            .fixed_size([560.0, 420.0])
            .anchor(Align2::CENTER_TOP, [0.0, 90.0])
            .frame(egui::Frame::window(&ctx.style()).fill(c_panel()).stroke(Stroke::new(1.0, accent().gamma_multiply(0.6))).inner_margin(egui::Margin::same(12.0)))
            .show(ctx, |ui| {
                let r = ui.add(egui::TextEdit::singleline(&mut self.palette.query).hint_text("Type a page, a process name or PID, or an action…").desired_width(f32::INFINITY).font(FontId::proportional(16.0)));
                if self.palette.just_opened {
                    r.request_focus();
                    self.palette.just_opened = false;
                }
                if r.changed() {
                    self.palette.sel = 0;
                }
                ui.add_space(6.0);
                egui::ScrollArea::vertical().max_height(360.0).show(ui, |ui| {
                    for (i, (_, title, sub)) in items.iter().enumerate() {
                        let sel = i == self.palette.sel;
                        let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 28.0), Sense::click());
                        if sel || resp.hovered() {
                            ui.painter().rect_filled(rect, 6.0, if sel { accent().gamma_multiply(0.25) } else { c_card_hi() });
                        }
                        ui.painter().text(rect.left_center() + Vec2::new(10.0, 0.0), Align2::LEFT_CENTER, title, FontId::proportional(14.0), if sel { accent() } else { c_text() });
                        ui.painter().text(rect.right_center() - Vec2::new(10.0, 0.0), Align2::RIGHT_CENTER, sub, FontId::proportional(11.5), c_dim());
                        if sel && (up || down) {
                            resp.scroll_to_me(None);
                        }
                        if resp.clicked() {
                            chosen = Some(i);
                        }
                    }
                    if items.is_empty() {
                        ui.label(RichText::new("Nothing matches.").color(c_dim()));
                    }
                });
                ui.label(RichText::new("Enter open   ⬆⬇ move   Esc close").size(11.0).color(c_dim()));
            });
        if let Some(i) = chosen {
            if let Some((item, _, _)) = items.into_iter().nth(i) {
                self.palette.open = false;
                match item {
                    PalItem::Page(p) => self.page = p,
                    PalItem::Proc(pid, _) => {
                        self.procs.selected = Some(pid);
                        self.page = Page::Processes;
                    }
                    PalItem::Action(k, _) => self.run_action(k, ctx),
                }
            }
        }
        if close {
            self.palette.open = false;
        }
    }

    pub fn help_window(&mut self, ctx: &egui::Context) {
        if !self.show_help {
            return;
        }
        let mut open = true;
        egui::Window::new("Keyboard shortcuts").collapsible(false).resizable(false).anchor(Align2::CENTER_CENTER, [0.0, 0.0]).open(&mut open).show(ctx, |ui| {
            egui::Grid::new("keys").striped(true).spacing([24.0, 6.0]).show(ui, |ui| {
                for (k, d) in SHORTCUTS {
                    ui.label(RichText::new(*k).monospace().color(accent()));
                    ui.label(*d);
                    ui.end_row();
                }
            });
            ui.add_space(6.0);
            ui.label(RichText::new("Tip: right-click any page in the sidebar to pin it to favorites, open it at start-up or hide it.").color(c_dim()));
        });
        if !open {
            self.show_help = false;
        }
    }
}

fn group_header(ui: &mut egui::Ui, text: &str, collapsed: bool) -> egui::Response {
    ui.add_space(5.0);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 16.0), Sense::click());
    let c = if resp.hovered() { c_text() } else { c_dim() };
    ui.painter().text(rect.left_center() + Vec2::new(6.0, 0.0), Align2::LEFT_CENTER, text, FontId::proportional(10.5), c);
    if !text.starts_with('★') {
        ui.painter().text(rect.right_center() - Vec2::new(6.0, 0.0), Align2::RIGHT_CENTER, if collapsed { "⏵" } else { "⏷" }, FontId::proportional(10.0), c);
    }
    resp.on_hover_text(if collapsed { "Expand this group" } else { "Collapse this group" })
}

fn mini_meter(ui: &mut egui::Ui, label: &str, v: f32, color: egui::Color32) {
    let (rect, _) = ui.allocate_exact_size(Vec2::new(78.0, 26.0), Sense::hover());
    let p = ui.painter();
    p.text(rect.left_top() + Vec2::new(0.0, 1.0), Align2::LEFT_TOP, label, FontId::proportional(10.0), c_dim());
    p.text(rect.right_top() + Vec2::new(0.0, 0.0), Align2::RIGHT_TOP, format!("{v:.0}%"), FontId::proportional(12.0), load_color(v));
    let bar = egui::Rect::from_min_size(rect.left_bottom() - Vec2::new(0.0, 6.0), Vec2::new(rect.width(), 4.0));
    p.rect_filled(bar, 2.0, c_bg());
    let mut f = bar;
    f.set_width(bar.width() * (v / 100.0).clamp(0.0, 1.0));
    p.rect_filled(f, 2.0, color);
}
