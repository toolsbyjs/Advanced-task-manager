//! Settings: appearance, layout & navigation, process table, monitoring, shortcuts, data.

use super::widgets::*;
use super::{page_key, App, NAV};
use crate::config::UiPrefs;
use crate::util;
use egui::{Color32, RichText, Ui, Vec2};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum SetTab {
    #[default]
    Appearance,
    Layout,
    Processes,
    Monitoring,
    Shortcuts,
    Data,
}

fn row(ui: &mut Ui, label: &str, hint: &str, add: impl FnOnce(&mut Ui)) {
    ui.horizontal(|ui| {
        ui.allocate_ui_with_layout(Vec2::new(200.0, 20.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
            ui.set_min_width(200.0);
            ui.label(RichText::new(label).strong());
        });
        add(ui);
    });
    if !hint.is_empty() {
        ui.horizontal(|ui| {
            ui.add_space(208.0);
            ui.label(RichText::new(hint).color(c_dim()).size(11.5));
        });
    }
    ui.add_space(4.0);
}

fn swatch(ui: &mut Ui, color: Color32, selected: bool, tip: &str) -> bool {
    let (rect, resp) = ui.allocate_exact_size(Vec2::splat(26.0), egui::Sense::click());
    ui.painter().circle_filled(rect.center(), 11.0, color);
    if selected {
        ui.painter().circle_stroke(rect.center(), 13.0, egui::Stroke::new(2.0, c_text()));
    } else if resp.hovered() {
        ui.painter().circle_stroke(rect.center(), 13.0, egui::Stroke::new(1.0, c_dim()));
    }
    resp.on_hover_text(tip).clicked()
}

fn theme_preview(ui: &mut Ui, name: &str, selected: bool) -> bool {
    let pal = preview_colors(name);
    let (rect, resp) = ui.allocate_exact_size(Vec2::new(120.0, 76.0), egui::Sense::click());
    let p = ui.painter();
    p.rect_filled(rect, 8.0, pal[0]);
    let side = egui::Rect::from_min_size(rect.min, Vec2::new(30.0, rect.height()));
    p.rect_filled(side, egui::Rounding { nw: 8.0, sw: 8.0, ne: 0.0, se: 0.0 }, pal[1]);
    let card = egui::Rect::from_min_size(rect.min + Vec2::new(38.0, 10.0), Vec2::new(74.0, 34.0));
    p.rect_filled(card, 4.0, pal[2]);
    p.line_segment([card.left_bottom() + Vec2::new(6.0, -8.0), card.right_top() + Vec2::new(-6.0, 10.0)], egui::Stroke::new(2.0, accent()));
    p.text(rect.left_bottom() + Vec2::new(38.0, -8.0), egui::Align2::LEFT_BOTTOM, name, egui::FontId::proportional(11.5), pal[3]);
    let stroke = if selected { egui::Stroke::new(2.0, accent()) } else if resp.hovered() { egui::Stroke::new(1.0, c_dim()) } else { egui::Stroke::new(1.0, c_border()) };
    p.rect_stroke(rect, 8.0, stroke);
    resp.clicked()
}

fn preview_colors(name: &str) -> [Color32; 4] {
    let c = Color32::from_rgb;
    match name {
        "Graphite" => [c(18, 18, 20), c(26, 26, 29), c(33, 33, 37), c(230, 230, 232)],
        "Ocean" => [c(7, 18, 28), c(11, 27, 41), c(15, 35, 53), c(214, 232, 245)],
        "Forest" => [c(10, 20, 15), c(15, 28, 21), c(20, 37, 28), c(220, 236, 226)],
        "Light" => [c(234, 238, 244), c(247, 248, 251), c(255, 255, 255), c(26, 32, 40)],
        "High contrast" => [c(0, 0, 0), c(8, 8, 8), c(14, 14, 14), c(255, 255, 255)],
        _ => [c(13, 17, 23), c(22, 27, 34), c(28, 34, 43), c(220, 226, 235)],
    }
}

pub fn page(app: &mut App, ui: &mut Ui) {
    ui.horizontal(|ui| {
        ui.label(RichText::new("Settings").size(22.0).strong());
        ui.add_space(12.0);
        for (t, l) in [
            (SetTab::Appearance, "🎨 Appearance"),
            (SetTab::Layout, "☰ Layout & navigation"),
            (SetTab::Processes, "📋 Process table"),
            (SetTab::Monitoring, "📈 Monitoring"),
            (SetTab::Shortcuts, "⌨ Shortcuts"),
            (SetTab::Data, "📂 Your data"),
        ] {
            ui.selectable_value(&mut app.settings_tab, t, l);
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("Reset look to defaults").on_hover_text("Theme, colours, sizes and layout go back to how they were at first start").clicked() {
                let keep_favs = app.prefs.favorites.clone();
                app.prefs = UiPrefs { favorites: keep_favs, ..Default::default() };
                app.notify("Appearance reset", false);
            }
        });
    });
    ui.add_space(6.0);
    egui::ScrollArea::vertical().id_salt("settings").auto_shrink([false, false]).show(ui, |ui| match app.settings_tab {
        SetTab::Appearance => appearance(app, ui),
        SetTab::Layout => layout(app, ui),
        SetTab::Processes => processes(app, ui),
        SetTab::Monitoring => monitoring(app, ui),
        SetTab::Shortcuts => shortcuts(ui),
        SetTab::Data => data(app, ui),
    });
}

fn appearance(app: &mut App, ui: &mut Ui) {
    let p = &mut app.prefs;
    card(ui, "Theme", |ui| {
        ui.horizontal_wrapped(|ui| {
            for t in THEMES {
                if theme_preview(ui, t, p.theme == t) {
                    p.theme = t.to_string();
                }
            }
        });
    });
    ui.add_space(8.0);
    card(ui, "Accent colour", |ui| {
        ui.horizontal(|ui| {
            for (name, rgb) in ACCENTS {
                if swatch(ui, Color32::from_rgb(rgb[0], rgb[1], rgb[2]), p.accent == rgb, name) {
                    p.accent = rgb;
                }
            }
            ui.add_space(10.0);
            ui.label("Custom");
            let mut col = Color32::from_rgb(p.accent[0], p.accent[1], p.accent[2]);
            if ui.color_edit_button_srgba(&mut col).changed() {
                p.accent = [col.r(), col.g(), col.b()];
            }
        });
        ui.label(RichText::new("Used for selections, the active page, links and highlights.").color(c_dim()).size(11.5));
    });
    ui.add_space(8.0);
    card(ui, "Size and text", |ui| {
        row(ui, "Zoom", "Ctrl + and Ctrl − also work anywhere; Ctrl 0 resets.", |ui| {
            let mut z = p.zoom * 100.0;
            if ui.add(egui::Slider::new(&mut z, 60.0..=200.0).suffix(" %").step_by(5.0)).changed() {
                p.zoom = z / 100.0;
            }
            for (l, v) in [("90%", 0.9), ("100%", 1.0), ("125%", 1.25), ("150%", 1.5)] {
                if ui.small_button(l).clicked() {
                    p.zoom = v;
                }
            }
        });
        row(ui, "Text size", "", |ui| {
            ui.add(egui::Slider::new(&mut p.font_size, 11.0..=18.0).suffix(" pt").step_by(0.5));
        });
        row(ui, "Density", "Compact fits more rows on screen; spacious is easier to read and click.", |ui| {
            for d in DENSITIES {
                ui.selectable_value(&mut p.density, d.to_string(), d);
            }
        });
        row(ui, "Corner roundness", "", |ui| {
            ui.add(egui::Slider::new(&mut p.rounding, 0.0..=16.0).suffix(" px"));
        });
        row(ui, "Animations", "", |ui| {
            ui.checkbox(&mut p.animations, "Smooth transitions");
        });
    });
    ui.add_space(8.0);
    card(ui, "Graphs", |ui| {
        row(ui, "Style", "", |ui| {
            ui.checkbox(&mut p.chart_fill, "Filled area");
            ui.checkbox(&mut p.chart_grid, "Grid lines");
            ui.checkbox(&mut p.chart_smooth, "Smooth lines");
        });
        row(ui, "Line thickness", "", |ui| {
            ui.add(egui::Slider::new(&mut p.line_width, 0.8..=3.5).step_by(0.1));
        });
        row(ui, "Network speeds in", "", |ui| {
            ui.selectable_value(&mut p.net_bits, true, "bits per second (Mbps)");
            ui.selectable_value(&mut p.net_bits, false, "bytes per second (MB/s)");
        });
        let demo: Vec<f32> = (0..80).map(|i| 45.0 + 30.0 * ((i as f32) / 7.0).sin() + ((i * 37 % 11) as f32) * 2.0).collect();
        ui.label(RichText::new("Preview").color(c_dim()));
        chart(ui, &[Series { data: demo, color: accent(), label: "Example" }], 110.0, Some(100.0), 80, &pct);
    });
}

fn layout(app: &mut App, ui: &mut Ui) {
    card(ui, "Sidebar", |ui| {
        row(ui, "Style", "Icons only gives the pages more room (Ctrl+B switches).", |ui| {
            ui.selectable_value(&mut app.prefs.nav_compact, false, "Icons and names");
            ui.selectable_value(&mut app.prefs.nav_compact, true, "Icons only");
        });
        row(ui, "Status bar", "The thin strip at the bottom with live state and monitor cost.", |ui| {
            ui.checkbox(&mut app.prefs.status_bar, "Show");
        });
        row(ui, "Start-up page", "", |ui| {
            let cur = super::page_by_key(&app.prefs.start_page).map(|p| super::page_info(p).label).unwrap_or("Dashboard");
            egui::ComboBox::from_id_salt("startpage").selected_text(cur).width(220.0).show_ui(ui, |ui| {
                for (_, items) in NAV {
                    for i in items.iter() {
                        ui.selectable_value(&mut app.prefs.start_page, page_key(i.page), format!("{} {}", i.icon, i.label));
                    }
                }
            });
        });
        row(ui, "Notifications stay for", "", |ui| {
            ui.add(egui::Slider::new(&mut app.prefs.toast_secs, 2..=20).suffix(" s"));
        });
    });
    ui.add_space(8.0);
    card(ui, "Pages in the sidebar", |ui| {
        ui.label(RichText::new("★ pins a page to Favorites (reach the first nine with Ctrl+1…9). Untick to hide a page you never use; it stays reachable from Ctrl+K.").color(c_dim()).size(12.0));
        ui.add_space(4.0);
        let mut move_fav: Option<(usize, i32)> = None;
        if !app.prefs.favorites.is_empty() {
            ui.label(RichText::new("Favorites order").strong());
            ui.horizontal_wrapped(|ui| {
                for (i, k) in app.prefs.favorites.iter().enumerate() {
                    let label = super::page_by_key(k).map(|p| super::page_info(p).label).unwrap_or("?");
                    egui::Frame::none().fill(c_card_hi()).rounding(6.0).inner_margin(egui::Margin::symmetric(6.0, 2.0)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui.small_button("◀").clicked() {
                                move_fav = Some((i, -1));
                            }
                            ui.label(format!("{}. {label}", i + 1));
                            if ui.small_button("▶").clicked() {
                                move_fav = Some((i, 1));
                            }
                        });
                    });
                }
            });
            ui.add_space(6.0);
        }
        if let Some((i, d)) = move_fav {
            let j = i as i32 + d;
            if j >= 0 && (j as usize) < app.prefs.favorites.len() {
                app.prefs.favorites.swap(i, j as usize);
            }
        }
        ui.columns(3, |cols| {
            let mut ci = 0;
            for (group, items) in NAV {
                let ui = &mut cols[ci % 3];
                ci += 1;
                if !group.is_empty() {
                    ui.label(RichText::new(*group).color(c_dim()).size(11.0).strong());
                }
                for i in items.iter() {
                    let k = page_key(i.page);
                    ui.horizontal(|ui| {
                        let fav = app.prefs.favorites.contains(&k);
                        if ui.add(egui::Button::new(RichText::new(if fav { "★" } else { "☆" }).color(if fav { YELLOW } else { c_dim() })).frame(false)).on_hover_text("Favorite").clicked() {
                            if fav {
                                app.prefs.favorites.retain(|x| *x != k);
                            } else {
                                app.prefs.favorites.push(k.clone());
                            }
                        }
                        let mut shown = !app.prefs.hidden_pages.contains(&k);
                        if i.page == super::Page::Settings {
                            ui.add_enabled(false, egui::Checkbox::new(&mut shown, format!("{} {}", i.icon, i.label)));
                        } else if ui.checkbox(&mut shown, format!("{} {}", i.icon, i.label)).changed() {
                            if shown {
                                app.prefs.hidden_pages.retain(|x| *x != k);
                            } else {
                                app.prefs.hidden_pages.push(k.clone());
                            }
                        }
                    });
                }
                ui.add_space(6.0);
            }
        });
    });
    ui.add_space(8.0);
    card(ui, "Dashboard sections", |ui| {
        ui.label(RichText::new("Choose what the main Dashboard shows. For fully free layouts use My dashboards.").color(c_dim()).size(12.0));
        ui.horizontal_wrapped(|ui| {
            for s in super::dashboard::SECTIONS {
                let mut on = !app.prefs.overview_hidden.iter().any(|x| x == s);
                if ui.checkbox(&mut on, *s).changed() {
                    if on {
                        app.prefs.overview_hidden.retain(|x| x != s);
                    } else {
                        app.prefs.overview_hidden.push(s.to_string());
                    }
                }
            }
        });
    });
}

fn processes(app: &mut App, ui: &mut Ui) {
    card(ui, "Process table", |ui| {
        row(ui, "Colour hints", "Tints busy CPU, memory, disk and GPU cells from yellow to red.", |ui| {
            ui.checkbox(&mut app.prefs.heat_colors, "Heat colours");
        });
        row(ui, "Name colours", "Orange = running as administrator, blue = Windows service, grey = suspended.", |ui| {
            ui.checkbox(&mut app.prefs.row_colors, "Colour process names");
        });
        row(ui, "Details panel height", "You can also drag its top edge.", |ui| {
            ui.add(egui::Slider::new(&mut app.prefs.detail_height, 160.0..=700.0).suffix(" px"));
        });
    });
    ui.add_space(8.0);
    let shared = app.shared.clone();
    let mut s = shared.lock().unwrap();
    card(ui, "Columns", |ui| {
        ui.label(RichText::new("Pick a ready-made set, then fine-tune on the Processes page with ☰ Columns (drag order with the arrows there).").color(c_dim()).size(12.0));
        ui.horizontal_wrapped(|ui| {
            for (name, cols) in super::processes::PRESETS {
                if ui.button(*name).clicked() {
                    s.config.columns = cols.iter().map(|c| c.to_string()).collect();
                    s.config_dirty = true;
                }
            }
        });
        let mut confirm = s.config.confirm_actions;
        if ui.checkbox(&mut confirm, "Ask before ending or suspending a process").changed() {
            s.config.confirm_actions = confirm;
            s.config_dirty = true;
        }
    });
}

fn monitoring(app: &mut App, ui: &mut Ui) {
    let shared = app.shared.clone();
    let mut s = shared.lock().unwrap();
    card(ui, "Sampling", |ui| {
        row(ui, "Update every", "Faster gives smoother graphs; slower uses less CPU.", |ui| {
            let mut ms = s.config.sample_ms;
            if ui.add(egui::Slider::new(&mut ms, 250..=5000).suffix(" ms")).changed() {
                s.config.sample_ms = ms;
                s.config_dirty = true;
            }
            for (l, v) in [("0.5 s", 500u64), ("1 s", 1000), ("2 s", 2000), ("5 s", 5000)] {
                if ui.small_button(l).clicked() {
                    s.config.sample_ms = v;
                    s.config_dirty = true;
                }
            }
        });
        ui.label(RichText::new(format!("Each sample currently costs {:.1} ms of CPU time on one core.", s.sample_cost_ms)).color(c_dim()));
        row(ui, "Live updates", "", |ui| {
            let mut paused = s.paused;
            if ui.checkbox(&mut paused, "Paused").changed() {
                s.paused = paused;
            }
        });
    });
    ui.add_space(8.0);
    card(ui, "View", |ui| {
        ui.label(format!("Current view: {}", if app.admin() { "Admin" } else { "Standard" }));
        ui.label(RichText::new("Standard view keeps things simple. Admin view adds 30+ process columns, threads, modules, memory maps, handles, security tokens, kernel objects, live system events and full control over services, drivers and adapters.").color(c_dim()));
    });
}

fn shortcuts(ui: &mut Ui) {
    card(ui, "Keyboard shortcuts", |ui| {
        egui::Grid::new("set_keys").striped(true).spacing([28.0, 8.0]).show(ui, |ui| {
            for (k, d) in super::shell::SHORTCUTS {
                ui.label(RichText::new(*k).monospace().color(accent()));
                ui.label(*d);
                ui.end_row();
            }
        });
    });
    ui.add_space(8.0);
    card(ui, "Mouse", |ui| {
        ui.label("• Right-click a process for every action (end, priority, efficiency, affinity, dump, properties).");
        ui.label("• Right-click a page in the sidebar to pin it, open it at start-up or hide it.");
        ui.label("• Click a column title to sort; click again to reverse. Drag column edges to resize.");
        ui.label("• Hover any graph to read exact values.");
    });
}

fn data(app: &mut App, ui: &mut Ui) {
    card(ui, "Your data", |ui| {
        ui.label(format!("Settings, snapshots, recordings, reports and dumps are kept in {}", util::data_dir().display()));
        ui.horizontal(|ui| {
            if ui.button("📂 Open folder").clicked() {
                util::open_path(&util::data_dir());
            }
            if ui.button("💾 Save settings now").clicked() {
                let s = app.shared.lock().unwrap();
                s.config.save();
                drop(s);
                app.notify("Settings saved", false);
            }
            if ui.button("📋 Back up settings").on_hover_text("Saves a dated copy next to the settings file").clicked() {
                let src = crate::config::Config::path();
                let dst = util::data_dir().join(format!("config-backup-{}.json", util::stamp()));
                let r = std::fs::copy(&src, &dst).map(|_| format!("Backup saved as {}", dst.file_name().unwrap_or_default().to_string_lossy())).map_err(|e| e.to_string());
                app.result(r);
            }
        });
    });
}
