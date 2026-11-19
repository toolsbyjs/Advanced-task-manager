//! Intelligence pages: health report & recommendations, bottlenecks, behaviour,
//! leak candidates, startup analysis and correlated events.

use super::widgets::*;
use super::{App, Page};
use crate::intel::{self, Finding};
use crate::queries::{self, Rows};
use crate::util::{self, fmt_bytes, jf64, jstr, Job};
use egui::{RichText, Ui};

#[derive(Default)]
pub struct InsightsUi {
    pub boot: Job<Rows>,
    pub beh_filter: String,
    pub plan_name: Job<String>,
}

fn findings(ui: &mut Ui, list: &[Finding]) {
    for f in list {
        egui::Frame::none()
            .fill(c_card())
            .rounding(8.0)
            .stroke(egui::Stroke::new(1.0, sev_color(f.severity).gamma_multiply(0.6)))
            .inner_margin(egui::Margin::same(10.0))
            .show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    pill(ui, sev_label(f.severity), sev_color(f.severity));
                    pill(ui, f.area, BLUE);
                    ui.label(RichText::new(&f.title).strong().size(14.5));
                });
                if !f.detail.is_empty() {
                    ui.label(RichText::new(&f.detail).color(c_dim()));
                }
                if !f.advice.is_empty() {
                    ui.label(RichText::new(format!("➡ {}", f.advice)).color(GREEN));
                }
            });
        ui.add_space(4.0);
    }
}

pub fn write_report(app: &mut App, open: bool) {
    let startup_n = app.manage.startup.take_ok().map(|r| r.iter().filter(|x| jstr(x, "State") != "Disabled").count());
    let plan = app.insights.plan_name.take_ok();
    let html = {
        let s = app.shared.lock().unwrap();
        let recs = intel::recommendations(&s, startup_n, plan.as_deref());
        intel::report_html(&s, &recs)
    };
    let path = util::sub_dir("reports").join(format!("health-report-{}.html", util::stamp()));
    match std::fs::write(&path, html) {
        Ok(()) => {
            app.shared.lock().unwrap().log("Report", crate::collector::Severity::Info, format!("Health report saved to {}", path.display()), None);
            if open {
                crate::sys::shell_open("open", &path.to_string_lossy(), "");
            }
            app.notify("Health report saved", false);
        }
        Err(e) => app.notify(format!("Could not save the report: {e}"), true),
    }
}

pub fn health(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    page_header(ui, "System health report", "One score for the whole PC, what drags it down and what to do about it.");
    app.insights.plan_name.ensure(ctx, || {
        let o = util::run_hidden("powercfg", &["/getactivescheme"])?;
        Ok(o.split('(').nth(1).map(|x| x.trim_end().trim_end_matches(')').to_string()).unwrap_or_default())
    });
    let startup_n = app.manage.startup.take_ok().map(|r| r.iter().filter(|x| jstr(x, "State") != "Disabled").count());
    app.manage.startup.ensure(ctx, queries::startup);
    let plan = app.insights.plan_name.take_ok();
    let (h, recs) = {
        let s = app.shared.lock().unwrap();
        (intel::health(&s), intel::recommendations(&s, startup_n, plan.as_deref()))
    };
    ui.horizontal(|ui| {
        if ui.button("💾 Save report as a web page and open it").clicked() {
            write_report(app, true);
        }
        if ui.button("📂 Reports folder").clicked() {
            util::open_path(&util::sub_dir("reports"));
        }
    });
    ui.add_space(6.0);
    egui::ScrollArea::vertical().id_salt("health").auto_shrink([false, false]).show(ui, |ui| {
        ui.columns(2, |cols| {
            card(&mut cols[0], "Score", |ui| {
                let color = if h.score >= 80 { GREEN } else if h.score >= 60 { YELLOW } else { RED };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(h.score.to_string()).size(60.0).strong().color(color));
                    ui.label(RichText::new("/ 100").size(18.0).color(c_dim()));
                });
                for (n, sc, d) in &h.parts {
                    ui.horizontal(|ui| {
                        bar(ui, *sc as f32 / 100.0, if *sc >= 80 { GREEN } else if *sc >= 60 { YELLOW } else { RED }, &sc.to_string(), 70.0);
                        ui.label(RichText::new(*n).strong());
                        ui.label(RichText::new(d).color(c_dim()).size(12.0));
                    });
                }
            });
            card(&mut cols[1], "Diagnostic recommendations", |ui| {
                findings(ui, &recs);
            });
        });
    });
}

pub fn bottlenecks(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Bottleneck detection", "Checks CPU, run queue, driver time, memory, commit, paging, each disk, free space, network links, GPU, heat and throttling.");
    let s = app.shared.lock().unwrap();
    let list = intel::bottlenecks(&s);
    let n = 120;
    egui::ScrollArea::vertical().id_salt("bn").auto_shrink([false, false]).show(ui, |ui| {
        if list.is_empty() {
            notice(ui, "✔ No bottleneck right now. Everything has headroom.", GREEN);
        }
        findings(ui, &list);
        ui.add_space(6.0);
        ui.columns(4, |cols| {
            let items: [(&str, Vec<f32>, egui::Color32); 4] = [("CPU", s.hist.cpu.to_vec(n), CYAN), ("Memory", s.hist.mem.to_vec(n), PURPLE), ("Disk", s.hist.disk.to_vec(n), GREEN), ("GPU", s.hist.gpu.to_vec(n), PINK)];
            for (i, (l, d, c)) in items.into_iter().enumerate() {
                card(&mut cols[i], l, |ui| {
                    let avg = d.iter().sum::<f32>() / d.len().max(1) as f32;
                    let mx = d.iter().cloned().fold(0.0, f32::max);
                    ui.label(format!("avg {avg:.0}%  •  peak {mx:.0}%"));
                    sparkline(ui, &d, c, egui::vec2(ui.available_width(), 50.0), Some(100.0));
                });
            }
        });
        ui.add_space(6.0);
        card(ui, "Key signals", |ui| {
            kv(
                ui,
                "sig",
                &[
                    ("Processor queue", format!("{:.0} threads waiting ({} logical processors)", s.sys.proc_queue, s.ncpu)),
                    ("Driver time (DPC + interrupts)", format!("{:.2}%", s.cpu_dpc + s.cpu_interrupt)),
                    ("Hard paging", format!("{:.0} pages/s", s.mem.pages_sec)),
                    ("Commit", format!("{} of {}", fmt_bytes(s.mem.st.commit_total as f64), fmt_bytes(s.mem.st.commit_limit as f64))),
                    ("Slowest disk response", s.disks.iter().map(|d| format!("{} {:.1} ms", d.name, d.resp_ms)).collect::<Vec<_>>().join(", ")),
                    ("CPU speed vs base", format!("{:.0}%", s.sys.perf_pct)),
                    ("Hottest sensor", if s.temps.is_empty() { "-".into() } else { format!("{:.0} °C", s.max_temp()) }),
                ],
            );
        });
    });
}

pub fn behaviour(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Process behaviour analysis", "Each process classified from its recent history: CPU-bound, bursty, disk-heavy, growing memory/handles/threads, network-chatty, idle…");
    search_field(ui, &mut app.insights.beh_filter, "Filter by name or trait…", 280.0);
    let s = app.shared.lock().unwrap();
    let list = intel::behaviour(&s);
    drop(s);
    let f = app.insights.beh_filter.to_lowercase();
    let list: Vec<_> = list.into_iter().filter(|b| f.is_empty() || b.name.to_lowercase().contains(&f) || b.traits.iter().any(|t| t.to_lowercase().contains(&f))).collect();
    if list.is_empty() && f.is_empty() {
        notice(ui, "Collecting history… each process is classified after about 10 samples (10 seconds at the default speed).", BLUE);
    }
    let mut open = None;
    egui_extras::TableBuilder::new(ui)
        .id_salt("beh")
        .striped(true)
        .resizable(true)
        .auto_shrink([false, false])
        .column(egui_extras::Column::initial(180.0))
        .column(egui_extras::Column::initial(60.0))
        .column(egui_extras::Column::initial(300.0))
        .column(egui_extras::Column::initial(80.0))
        .column(egui_extras::Column::initial(110.0))
        .column(egui_extras::Column::initial(100.0))
        .column(egui_extras::Column::remainder())
        .header(20.0, |mut h| {
            for t in ["Process", "PID", "Traits", "Avg CPU", "Memory trend", "Handles/min", "Summary"] {
                h.col(|ui| {
                    ui.add(egui::Label::new(RichText::new(t).strong()).truncate());
                });
            }
        })
        .body(|body| {
            body.rows(row_h(), list.len(), |mut row| {
                let b = &list[row.index()];
                row.col(|ui| {
                    if ui.link(&b.name).clicked() {
                        open = Some(b.pid);
                    }
                });
                row.col(|ui| {
                    ui.label(b.pid.to_string());
                });
                row.col(|ui| {
                    ui.horizontal(|ui| {
                        for t in &b.traits {
                            let c = match *t {
                                "CPU-bound" | "Memory growing" | "Handles growing" | "Threads growing" => ORANGE,
                                "Bursty" | "Disk-heavy" | "Page-fault heavy" => YELLOW,
                                "Idle" | "Steady" => c_dim(),
                                "Suspended" => PURPLE,
                                _ => BLUE,
                            };
                            pill(ui, t, c);
                        }
                    });
                });
                row.col(|ui| {
                    ui.label(format!("{:.1}%", b.cpu_avg));
                });
                row.col(|ui| {
                    ui.label(RichText::new(format!("{:+.2} MB/min", b.mem_trend_mb_min)).color(if b.mem_trend_mb_min > 2.0 { ORANGE } else { c_text() }));
                });
                row.col(|ui| {
                    ui.label(format!("{:+.1}", b.handle_trend));
                });
                row.col(|ui| {
                    ui.label(RichText::new(&b.summary).color(c_dim()).size(12.0));
                });
            });
        });
    if let Some(pid) = open {
        app.procs.selected = Some(pid);
        app.procs.tab = super::processes::Tab::Performance;
        app.page = Page::Processes;
    }
}

pub fn leaks(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Memory leak candidates", "Processes whose private memory, handles or threads keep rising steadily (straight-line fit over the last few minutes).");
    let s = app.shared.lock().unwrap();
    let list = intel::leak_candidates(&s);
    let samples = s.hist.cpu.len();
    ui.label(RichText::new(format!("Watching {} processes; analysis needs at least 30 samples per process ({samples} collected so far). Longer running time gives better answers.", s.procs.len())).color(c_dim()));
    let mut open = None;
    egui::ScrollArea::vertical().id_salt("leaks").auto_shrink([false, false]).show(ui, |ui| {
        if list.is_empty() {
            notice(ui, "✔ Nothing is growing steadily right now.", GREEN);
        }
        for l in &list {
            let hist: Vec<f32> = s.procs.get(&l.pid).map(|p| match l.kind {
                "Handles" => p.hist_handles.to_vec(300),
                "Threads" => p.hist_threads.to_vec(300),
                _ => p.hist_mem.to_vec(300),
            }).unwrap_or_default();
            card(ui, "", |ui| {
                ui.horizontal(|ui| {
                    pill(ui, l.kind, ORANGE);
                    if ui.link(RichText::new(format!("{} (PID {})", l.name, l.pid)).strong().size(15.0)).clicked() {
                        open = Some(l.pid);
                    }
                    let (rate, now, grown) = if l.kind == "Private memory" {
                        (format!("{:+.2} MB/min", l.growth_per_min / 1048576.0), fmt_bytes(l.now), fmt_bytes(l.grown))
                    } else {
                        (format!("{:+.1}/min", l.growth_per_min), format!("{:.0}", l.now), format!("{:.0}", l.grown))
                    };
                    ui.label(format!("grows {rate}  •  now {now}  •  +{grown} over {} samples  •  fit {:.0}%", l.samples, l.r2 * 100.0));
                });
                sparkline(ui, &hist, ORANGE, egui::vec2(ui.available_width(), 50.0), None);
            });
            ui.add_space(4.0);
        }
    });
    drop(s);
    if let Some(pid) = open {
        app.procs.selected = Some(pid);
        app.procs.tab = super::processes::Tab::History;
        app.page = Page::Processes;
    }
}

pub fn startup_analysis(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    page_header(ui, "Startup analysis", "How much starts with Windows, its estimated cost, and how long recent boots took.");
    app.manage.startup.ensure(ctx, queries::startup);
    app.insights.boot.ensure(ctx, queries::boot_performance);
    if !job_status(ui, &app.manage.startup) {
        return;
    }
    let rows = app.manage.startup.take_ok().unwrap_or_default();
    let s = app.shared.lock().unwrap();
    let mut scored: Vec<(String, String, String, &'static str, String)> = vec![];
    for r in &rows {
        if jstr(r, "State") == "Disabled" {
            continue;
        }
        let exe = jstr(r, "Executable").to_lowercase();
        let file = exe.rsplit('\\').next().unwrap_or("").to_string();
        let running = s.procs.values().find(|p| !file.is_empty() && p.name.to_lowercase() == file);
        let size = jf64(r, "FileSize");
        let mut score = 0.0;
        if let Some(p) = running {
            score += p.private_bytes as f64 / 1048576.0 / 50.0;
            score += p.cpu_time / 30.0;
        }
        score += size / 1048576.0 / 20.0;
        if jstr(r, "Kind") == "Auto-start service" {
            score += 1.0;
        }
        let impact = if score > 4.0 { "High" } else if score > 1.5 { "Medium" } else { "Low" };
        let now = running.map(|p| format!("running: {} memory, {:.0}s CPU so far", fmt_bytes(p.private_bytes as f64), p.cpu_time)).unwrap_or_else(|| "not running now".into());
        scored.push((jstr(r, "Name"), jstr(r, "Kind"), jstr(r, "Publisher"), impact, now));
    }
    drop(s);
    let high = scored.iter().filter(|x| x.3 == "High").count();
    let med = scored.iter().filter(|x| x.3 == "Medium").count();
    ui.horizontal(|ui| {
        pill(ui, &format!("{} enabled entries", scored.len()), BLUE);
        pill(ui, &format!("{high} high impact"), if high > 0 { RED } else { GREEN });
        pill(ui, &format!("{med} medium"), YELLOW);
        if ui.link("Manage startup apps ➡").clicked() {
            app.page = Page::Startup;
        }
    });
    ui.label(RichText::new("Impact is estimated from file size and, if it's running now, its memory and CPU use since boot.").color(c_dim()).size(12.0));
    ui.columns(2, |cols| {
        card(&mut cols[0], "Startup entries by estimated impact", |ui| {
            let mut sc = scored.clone();
            sc.sort_by_key(|x| match x.3 {
                "High" => 0,
                "Medium" => 1,
                _ => 2,
            });
            egui::ScrollArea::vertical().id_salt("sa").max_height(500.0).show(ui, |ui| {
                for (n, k, p, imp, now) in sc {
                    ui.horizontal(|ui| {
                        pill(ui, imp, match imp {
                            "High" => RED,
                            "Medium" => YELLOW,
                            _ => GREEN,
                        });
                        ui.label(RichText::new(n).strong());
                        ui.label(RichText::new(format!("{k} • {p}")).color(c_dim()).size(11.5));
                    });
                    ui.label(RichText::new(now).color(c_dim()).size(11.5));
                }
            });
        });
        card(&mut cols[1], "Recent boots (Windows boot performance log)", |ui| {
            let ok = job_status(ui, &app.insights.boot);
            if ok {
                let b = app.insights.boot.take_ok().unwrap_or_default();
                if b.is_empty() {
                    ui.label(RichText::new("No boot measurements available (this log needs the Admin view).").color(c_dim()));
                }
                for e in b {
                    let total = jf64(&e, "BootMs") / 1000.0;
                    let main = jf64(&e, "MainPathMs") / 1000.0;
                    let post = jf64(&e, "PostBootMs") / 1000.0;
                    ui.horizontal(|ui| {
                        ui.label(jstr(&e, "Time"));
                        bar(ui, (total / 120.0) as f32, load_color((total / 120.0 * 100.0) as f32), &format!("{total:.0} s"), 120.0);
                        ui.label(RichText::new(format!("until desktop {main:.0}s, settling {post:.0}s")).color(c_dim()).size(11.5));
                    });
                }
            }
        });
    });
}

pub fn correlations(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Correlated event detection", "Spikes in CPU, memory, disk or GPU matched with what happened within ±5 seconds (process starts, exits, automation…).");
    let s = app.shared.lock().unwrap();
    let list = intel::correlations(&s);
    drop(s);
    egui::ScrollArea::vertical().id_salt("corr").auto_shrink([false, false]).show(ui, |ui| {
        if list.is_empty() {
            notice(ui, "No unusual spikes yet. Leave the app running; spikes are found against the recent average.", BLUE);
        }
        for c in &list {
            card(ui, "", |ui| {
                ui.horizontal(|ui| {
                    pill(ui, c.metric, ORANGE);
                    ui.label(RichText::new(format!("{} spike to {:.0}% at {}", c.metric, c.value, util::fmt_time(c.time))).strong());
                });
                if c.related.is_empty() {
                    ui.label(RichText::new("Nothing else happened at the same moment — likely work inside an already running app.").color(c_dim()));
                } else {
                    for r in &c.related {
                        ui.label(RichText::new(format!("⏵ {r}")).color(CYAN));
                    }
                }
            });
            ui.add_space(4.0);
        }
    });
}
