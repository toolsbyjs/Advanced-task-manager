//! The modern dashboard and user-built custom dashboards.

use super::widgets::*;
use super::{App, Page};
use crate::collector::State;
use crate::config::{Dashboard, Widget};
use crate::intel;
use crate::util::{self, fmt_bytes};
use egui::{RichText, Ui};

#[derive(Default)]
pub struct DashUi {
    pub selected: usize,
    pub editing: bool,
}

pub const SECTIONS: &[&str] = &["Summary tiles", "Health and bottlenecks", "Live graphs", "Top processes", "Events and drives"];

pub fn overview(app: &mut App, ui: &mut Ui) {
    let shared = app.shared.clone();
    let s = shared.lock().unwrap();
    let hidden = app.prefs.overview_hidden.clone();
    let show = |n: &str| !hidden.iter().any(|h| h == n);
    egui::ScrollArea::vertical().id_salt("dash").auto_shrink([false, false]).show(ui, |ui| {
        ui.horizontal(|ui| {
            ui.label(RichText::new("Dashboard").size(22.0).strong());
            ui.label(RichText::new(format!("{}  •  up {}  •  {} logical processors", s.cpu_name, util::fmt_dur(s.sys.uptime), s.ncpu)).color(c_dim()));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                ui.menu_button("⚙ Customize", |ui| {
                    ui.label(RichText::new("Show on this dashboard").strong());
                    for sec in SECTIONS {
                        let mut on = !app.prefs.overview_hidden.iter().any(|x| x == sec);
                        if ui.checkbox(&mut on, *sec).changed() {
                            if on {
                                app.prefs.overview_hidden.retain(|x| x != sec);
                            } else {
                                app.prefs.overview_hidden.push(sec.to_string());
                            }
                        }
                    }
                    ui.separator();
                    if ui.button("Build my own dashboard ➡").clicked() {
                        app.page = Page::CustomDash;
                        ui.close_menu();
                    }
                });
            });
        });
        ui.add_space(6.0);
        let mut goto = None;
        if show("Summary tiles") {
        ui.columns(5, |cols| {
            let used = s.mem.st.total.saturating_sub(s.mem.st.avail);
            let disk_sub = s.disks.iter().map(|d| format!("{} {:.0}%", d.name.split(' ').last().unwrap_or(""), d.active)).collect::<Vec<_>>().join("  ");
            let net: Vec<f32> = s.hist.net_rx.to_vec(60).iter().zip(s.hist.net_tx.to_vec(60).iter()).map(|(a, b)| a + b).collect();
            if tile(&mut cols[0], "CPU", &format!("{:.0}%", s.cpu_total), &format!("{} processes", s.procs.len()), CYAN, &s.hist.cpu.to_vec(60), Some(100.0), false).clicked() {
                goto = Some(super::perf::PerfTab::Cpu);
            }
            if tile(&mut cols[1], "Memory", &format!("{:.0}%", s.mem_used_pct()), &format!("{} of {}", fmt_bytes(used as f64), fmt_bytes(s.mem.st.total as f64)), PURPLE, &s.hist.mem.to_vec(60), Some(100.0), false).clicked() {
                goto = Some(super::perf::PerfTab::Memory);
            }
            if tile(&mut cols[2], "Disk", &format!("{:.0}%", s.disk_active_max()), &disk_sub, GREEN, &s.hist.disk.to_vec(60), Some(100.0), false).clicked() {
                goto = Some(super::perf::PerfTab::Disk);
            }
            if tile(&mut cols[3], "Network", &net_rate((s.net_rx() + s.net_tx()) as f64), &format!("⬇ {}  ⬆ {}", util::fmt_rate(s.net_rx() as f64), util::fmt_rate(s.net_tx() as f64)), ORANGE, &net, None, false).clicked() {
                goto = Some(super::perf::PerfTab::Network);
            }
            if tile(&mut cols[4], "GPU", &format!("{:.0}%", s.gpu.total), &format!("{} dedicated", fmt_bytes(s.gpu.dedicated_used as f64)), PINK, &s.hist.gpu.to_vec(60), Some(100.0), false).clicked() {
                goto = Some(super::perf::PerfTab::Gpu);
            }
        });
        ui.add_space(8.0);
        }
        if let Some(t) = goto {
            app.perf.tab = t;
            app.page = Page::Performance;
        }
        if show("Health and bottlenecks") {
        ui.columns(3, |cols| {
            widget(app, &mut cols[0], &s, Widget::Health);
            widget(app, &mut cols[1], &s, Widget::SystemSummary);
            bottleneck_card(app, &mut cols[2], &s);
        });
        ui.add_space(8.0);
        }
        if show("Live graphs") {
        ui.columns(2, |cols| {
            card(&mut cols[0], "CPU and memory — last 2 minutes", |ui| {
                chart(
                    ui,
                    &[Series { data: s.hist.cpu.to_vec(120), color: CYAN, label: "CPU" }, Series { data: s.hist.mem.to_vec(120), color: PURPLE, label: "Memory" }],
                    170.0,
                    Some(100.0),
                    120,
                    &pct,
                );
            });
            card(&mut cols[1], "Disk and network — last 2 minutes", |ui| {
                chart(
                    ui,
                    &[
                        Series { data: s.hist.disk_read.to_vec(120), color: TEAL, label: "Disk read" },
                        Series { data: s.hist.disk_write.to_vec(120), color: GREEN, label: "Disk write" },
                        Series { data: s.hist.net_rx.to_vec(120), color: ORANGE, label: "Net down" },
                    ],
                    170.0,
                    None,
                    120,
                    &rate,
                );
            });
        });
        ui.add_space(8.0);
        }
        if show("Top processes") {
        ui.columns(3, |cols| {
            widget(app, &mut cols[0], &s, Widget::TopCpu);
            widget(app, &mut cols[1], &s, Widget::TopMemory);
            widget(app, &mut cols[2], &s, Widget::TopDisk);
        });
        ui.add_space(8.0);
        }
        if show("Events and drives") {
        ui.columns(2, |cols| {
            widget(app, &mut cols[0], &s, Widget::Timeline);
            widget(app, &mut cols[1], &s, Widget::Volumes);
        });
        }
    });
}

fn bottleneck_card(app: &mut App, ui: &mut Ui, s: &State) {
    let found = intel::bottlenecks(s);
    card(ui, "What's slowing things down", |ui| {
        ui.set_min_height(150.0);
        if found.is_empty() {
            ui.label(RichText::new("✔ No bottlenecks right now").color(GREEN).size(15.0));
            ui.label(RichText::new("CPU, memory, disks, network and temperature are within healthy limits.").color(c_dim()));
        }
        for f in found.iter().take(4) {
            ui.horizontal(|ui| {
                pill(ui, sev_label(f.severity), sev_color(f.severity));
                ui.label(RichText::new(&f.title).strong());
            });
            if !f.detail.is_empty() {
                ui.label(RichText::new(&f.detail).color(c_dim()).size(12.0));
            }
        }
        if ui.link("Open the bottleneck analysis ➡").clicked() {
            app.page = Page::Bottlenecks;
        }
    });
}

pub fn widget(app: &mut App, ui: &mut Ui, s: &State, w: Widget) {
    match w {
        Widget::CpuChart => card(ui, "CPU", |ui| chart(ui, &[Series { data: s.hist.cpu.to_vec(120), color: CYAN, label: "CPU" }], 140.0, Some(100.0), 120, &pct)),
        Widget::MemoryChart => card(ui, "Memory", |ui| chart(ui, &[Series { data: s.hist.mem.to_vec(120), color: PURPLE, label: "In use" }], 140.0, Some(100.0), 120, &pct)),
        Widget::DiskChart => card(ui, "Disk", |ui| chart(ui, &[Series { data: s.hist.disk.to_vec(120), color: GREEN, label: "Busiest disk" }], 140.0, Some(100.0), 120, &pct)),
        Widget::NetworkChart => card(ui, "Network", |ui| {
            chart(ui, &[Series { data: s.hist.net_rx.to_vec(120), color: ORANGE, label: "Down" }, Series { data: s.hist.net_tx.to_vec(120), color: CYAN, label: "Up" }], 140.0, None, 120, &rate)
        }),
        Widget::GpuChart => card(ui, "GPU", |ui| chart(ui, &[Series { data: s.hist.gpu.to_vec(120), color: PINK, label: "GPU" }], 140.0, Some(100.0), 120, &pct)),
        Widget::CoreGrid => super::perf::core_grid(ui, s),
        Widget::TopCpu | Widget::TopMemory | Widget::TopDisk => {
            let (title, color) = match w {
                Widget::TopCpu => ("Top CPU users", CYAN),
                Widget::TopMemory => ("Top memory users", PURPLE),
                _ => ("Top disk users", GREEN),
            };
            let mut ps: Vec<_> = s.procs.values().filter(|p| p.pid != 0).collect();
            match w {
                Widget::TopCpu => ps.sort_by(|a, b| b.cpu.total_cmp(&a.cpu)),
                Widget::TopMemory => ps.sort_by(|a, b| b.private_bytes.cmp(&a.private_bytes)),
                _ => ps.sort_by(|a, b| (b.io_read_rate + b.io_write_rate).total_cmp(&(a.io_read_rate + a.io_write_rate))),
            }
            let mut open = None;
            card(ui, title, |ui| {
                for p in ps.iter().take(7) {
                    let (val, frac) = match w {
                        Widget::TopCpu => (format!("{:.1}%", p.cpu), p.cpu / 100.0),
                        Widget::TopMemory => (fmt_bytes(p.private_bytes as f64), p.private_bytes as f32 / s.mem.st.total.max(1) as f32 * 4.0),
                        _ => (util::fmt_rate((p.io_read_rate + p.io_write_rate) as f64), (p.io_read_rate + p.io_write_rate) / 50e6),
                    };
                    ui.horizontal(|ui| {
                        bar(ui, frac, color, &val, 86.0);
                        if ui.add(egui::Label::new(&p.name).sense(egui::Sense::click()).truncate()).on_hover_text(&p.st.path).clicked() {
                            open = Some(p.pid);
                        }
                    });
                }
            });
            if let Some(pid) = open {
                app.procs.selected = Some(pid);
                app.page = Page::Processes;
            }
        }
        Widget::Alerts => card(ui, "Recent alerts", |ui| {
            if s.alerts.is_empty() {
                ui.label(RichText::new("No alerts. Rules watch your system in the background.").color(c_dim()));
            }
            for a in s.alerts.iter().rev().take(6) {
                ui.horizontal(|ui| {
                    ui.label(RichText::new("⏺").color(sev_color(a.severity)));
                    ui.label(RichText::new(util::fmt_clock(a.time)).color(c_dim()).monospace());
                    ui.label(&a.title);
                });
            }
        }),
        Widget::Health => {
            let h = intel::health(s);
            let color = if h.score >= 80 { GREEN } else if h.score >= 60 { YELLOW } else { RED };
            let mut open = false;
            card(ui, "Health score", |ui| {
                ui.set_min_height(150.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{}", h.score)).size(46.0).strong().color(color));
                    ui.vertical(|ui| {
                        ui.label(RichText::new("/ 100").color(c_dim()));
                        ui.label(RichText::new(if h.score >= 80 { "Healthy" } else if h.score >= 60 { "Needs attention" } else { "Struggling" }).color(color).strong());
                    });
                });
                for (n, sc, _) in h.parts.iter().take(5) {
                    ui.horizontal(|ui| {
                        bar(ui, *sc as f32 / 100.0, if *sc >= 80 { GREEN } else if *sc >= 60 { YELLOW } else { RED }, &format!("{sc}"), 60.0);
                        ui.label(RichText::new(*n).size(12.0));
                    });
                }
                if ui.link("Full health report ➡").clicked() {
                    open = true;
                }
            });
            if open {
                app.page = Page::Health;
            }
        }
        Widget::Volumes => super::perf::volumes_card(ui, s),
        Widget::SystemSummary => card(ui, "System", |ui| {
            ui.set_min_height(150.0);
            kv(
                ui,
                "sys_sum",
                &[
                    ("Processor", s.cpu_name.clone()),
                    ("Memory", fmt_bytes(s.mem.st.total as f64)),
                    ("Processes", format!("{} ({} threads)", s.procs.len(), s.mem.st.threads)),
                    ("Handles", util::fmt_num(s.mem.st.handles as u64)),
                    ("Up time", util::fmt_dur(s.sys.uptime)),
                    ("Context switches/s", util::fmt_num(s.sys.ctx_switches as u64)),
                    ("Temperature", if s.temps.is_empty() { "No sensor".into() } else { format!("{:.0} °C", s.max_temp()) }),
                    ("View", if app.admin() { "Admin".into() } else { "Standard".into() }),
                ],
            );
        }),
        Widget::Timeline => card(ui, "Latest events", |ui| {
            egui::ScrollArea::vertical().id_salt("tl_w").max_height(200.0).show(ui, |ui| {
                for e in s.timeline.iter().rev().take(40) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new("⏺").color(sev_color(e.severity)));
                        ui.label(RichText::new(util::fmt_clock(e.time)).color(c_dim()).monospace());
                        ui.label(RichText::new(e.category).color(CYAN).size(12.0));
                        ui.add(egui::Label::new(RichText::new(&e.text).size(12.5)).truncate());
                    });
                }
            });
        }),
        Widget::Sensors => card(ui, "Temperatures", |ui| {
            if s.temps.is_empty() {
                ui.label(RichText::new("Windows reports no thermal zones on this PC.").color(c_dim()));
            }
            for (n, t) in &s.temps {
                ui.horizontal(|ui| {
                    bar(ui, *t / 105.0, load_color(*t), &format!("{t:.1} °C"), 90.0);
                    ui.label(n.split('\\').last().unwrap_or(n));
                });
            }
        }),
    }
}

pub fn custom(app: &mut App, ui: &mut Ui) {
    let shared = app.shared.clone();
    ui.horizontal(|ui| {
        ui.label(RichText::new("My dashboards").size(22.0).strong());
        let s = shared.lock().unwrap();
        let names: Vec<String> = s.config.dashboards.iter().map(|d| d.name.clone()).collect();
        drop(s);
        for (i, n) in names.iter().enumerate() {
            ui.selectable_value(&mut app.dash.selected, i, n);
        }
        if ui.button("✚ New dashboard").clicked() {
            let mut s = shared.lock().unwrap();
            let id = s.config.new_id();
            let n = s.config.dashboards.len() + 1;
            s.config.dashboards.push(Dashboard { id, name: format!("Dashboard {n}"), columns: 2, widgets: vec![Widget::CpuChart, Widget::MemoryChart] });
            s.config_dirty = true;
            app.dash.selected = s.config.dashboards.len() - 1;
            app.dash.editing = true;
        }
        ui.toggle_value(&mut app.dash.editing, "✏ Edit");
    });
    let mut s = shared.lock().unwrap();
    if s.config.dashboards.is_empty() {
        ui.label("No dashboards yet. Press New dashboard.");
        return;
    }
    let idx = app.dash.selected.min(s.config.dashboards.len() - 1);
    app.dash.selected = idx;
    if app.dash.editing {
        let mut remove_dash = false;
        let mut changed = false;
        card(ui, "Edit dashboard", |ui| {
            let d = &mut s.config.dashboards[idx];
            ui.horizontal(|ui| {
                ui.label("Name");
                changed |= ui.text_edit_singleline(&mut d.name).changed();
                ui.label("Columns");
                changed |= ui.add(egui::Slider::new(&mut d.columns, 1..=4)).changed();
                if ui.button(RichText::new("Delete dashboard").color(RED)).clicked() {
                    remove_dash = true;
                }
            });
            ui.label(RichText::new("Add widgets:").color(c_dim()));
            ui.horizontal_wrapped(|ui| {
                for w in Widget::ALL {
                    if ui.button(format!("✚ {}", w.label())).clicked() {
                        d.widgets.push(w);
                        changed = true;
                    }
                }
            });
            ui.label(RichText::new("On this dashboard (use the arrows to reorder):").color(c_dim()));
            let mut action: Option<(usize, i32)> = None;
            ui.horizontal_wrapped(|ui| {
                for (i, w) in d.widgets.iter().enumerate() {
                    egui::Frame::none().fill(c_card_hi()).rounding(6.0).inner_margin(egui::Margin::symmetric(6.0, 2.0)).show(ui, |ui| {
                        ui.horizontal(|ui| {
                            if ui.small_button("◀").clicked() {
                                action = Some((i, -1));
                            }
                            ui.label(w.label());
                            if ui.small_button("▶").clicked() {
                                action = Some((i, 1));
                            }
                            if ui.small_button("✖").clicked() {
                                action = Some((i, 0));
                            }
                        });
                    });
                }
            });
            if let Some((i, dir)) = action {
                changed = true;
                if dir == 0 {
                    d.widgets.remove(i);
                } else {
                    let j = i as i32 + dir;
                    if j >= 0 && (j as usize) < d.widgets.len() {
                        d.widgets.swap(i, j as usize);
                    }
                }
            }
        });
        if changed {
            s.config_dirty = true;
        }
        if remove_dash {
            s.config.dashboards.remove(idx);
            s.config_dirty = true;
            app.dash.selected = 0;
            return;
        }
        ui.add_space(8.0);
    }
    let d = s.config.dashboards[idx].clone();
    let st: &State = &s;
    egui::ScrollArea::vertical().id_salt("custom_dash").auto_shrink([false, false]).show(ui, |ui| {
        let ncol = d.columns.max(1) as usize;
        for chunk in d.widgets.chunks(ncol) {
            ui.columns(ncol, |cols| {
                for (i, w) in chunk.iter().enumerate() {
                    widget(app, &mut cols[i], st, *w);
                }
            });
            ui.add_space(8.0);
        }
    });
}
