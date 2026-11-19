//! Deep observability: process inspector, kernel objects, live ETW events,
//! Event Viewer, diagnostics timeline, recording, historical graphs, sensors, disk health.

use super::widgets::*;
use super::App;
use crate::collector::Severity;
use crate::etw;
use crate::nt;
use crate::queries::{self, Rows};
use crate::util::{self, fmt_bytes, fmt_num, jf64, jget, jstr, Job};
use egui::{RichText, Ui};
use serde_json::Value;

pub struct DeepUi {
    pub inspect_filter: String,
    pub objects: Job<(Vec<nt::ObjectType>, Vec<(u32, String, usize)>, usize)>,
    pub etw: Option<etw::Session>,
    pub etw_sel: Vec<bool>,
    pub etw_filter: String,
    pub etw_paused: bool,
    pub etw_selected: Option<usize>,
    pub etw_sessions: Job<String>,
    pub events: Job<Rows>,
    pub events_t: TableState,
    pub ev_log: String,
    pub ev_levels: [bool; 4],
    pub ev_hours: u32,
    pub logs: Job<Rows>,
    pub tl_filter: String,
    pub tl_cats: Vec<(&'static str, bool)>,
    pub tl_include_events: bool,
    pub graph_range: usize,
    pub loaded: Option<(String, Vec<String>, Vec<Vec<f32>>)>,
    pub rec_secs: u32,
    pub sensors: Job<Value>,
    pub disks: Job<Value>,
}

impl Default for DeepUi {
    fn default() -> Self {
        Self {
            inspect_filter: String::new(),
            objects: Job::default(),
            etw: None,
            etw_sel: etw::PROVIDERS.iter().enumerate().map(|(i, _)| i == 0 || i == 2 || i == 3).collect(),
            etw_filter: String::new(),
            etw_paused: false,
            etw_selected: None,
            etw_sessions: Job::default(),
            events: Job::default(),
            events_t: TableState::default(),
            ev_log: "System".into(),
            ev_levels: [true, true, true, false],
            ev_hours: 24,
            logs: Job::default(),
            tl_filter: String::new(),
            tl_cats: vec![("Process", true), ("Alert", true), ("Spike", true), ("Automation", true), ("Schedule", true), ("Recording", true), ("Snapshot", true), ("App", true), ("Dump", true)],
            tl_include_events: true,
            graph_range: 600,
            loaded: None,
            rec_secs: 300,
            sensors: Job::default(),
            disks: Job::default(),
        }
    }
}

// ------------------------------------------------------------------ inspector

pub fn inspector(app: &mut App, ui: &mut Ui) {
    if app.locked(ui, "The inspector shows every thread, module, memory region, handle and security token of a process.") {
        return;
    }
    page_header(ui, "Process inspector", "Pick a process to see its threads, modules, memory map, handles, connections and token.");
    let shared = app.shared.clone();
    let list: Vec<(u32, String, f32)> = {
        let s = shared.lock().unwrap();
        let mut v: Vec<_> = s.procs.values().map(|p| (p.pid, p.name.clone(), p.cpu)).collect();
        v.sort_by(|a, b| a.1.to_lowercase().cmp(&b.1.to_lowercase()));
        v
    };
    egui::SidePanel::left("insp_list").resizable(true).default_width(240.0).frame(egui::Frame::none()).show_inside(ui, |ui| {
        search_field(ui, &mut app.deep.inspect_filter, "Find process…", ui.available_width());
        let f = app.deep.inspect_filter.to_lowercase();
        egui::ScrollArea::vertical().id_salt("insp_s").auto_shrink([false, false]).show(ui, |ui| {
            for (pid, name, cpu) in list.iter().filter(|x| f.is_empty() || x.1.to_lowercase().contains(&f) || x.0.to_string() == f) {
                let sel = app.procs.selected == Some(*pid);
                if ui.selectable_label(sel, format!("{name}  ({pid})  {cpu:.0}%")).clicked() {
                    app.procs.selected = Some(*pid);
                }
            }
        });
    });
    match app.procs.selected {
        Some(pid) => {
            egui::Frame::none().fill(c_panel()).rounding(8.0).inner_margin(egui::Margin::same(10.0)).show(ui, |ui| {
                ui.set_min_height(ui.available_height());
                super::processes::details(app, ui, pid);
            });
        }
        None => {
            ui.label("Select a process on the left.");
        }
    }
}

// ------------------------------------------------------------------ kernel objects

pub fn kernel_objects(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    if app.locked(ui, "Kernel objects lists every object type and every open handle in the system.") {
        return;
    }
    page_header(ui, "Kernel objects & handles", "System-wide totals per object type and the processes holding the most handles.");
    let names: std::collections::HashMap<u32, String> = app.shared.lock().unwrap().procs.values().map(|p| (p.pid, p.name.clone())).collect();
    app.deep.objects.ensure(ctx, move || {
        let types = nt::query_object_types();
        let handles = nt::query_handles(None);
        let mut per: std::collections::HashMap<u32, usize> = Default::default();
        for h in &handles {
            *per.entry(h.pid).or_default() += 1;
        }
        let mut top: Vec<(u32, String, usize)> = per.into_iter().map(|(pid, n)| (pid, names.get(&pid).cloned().unwrap_or_default(), n)).collect();
        top.sort_by(|a, b| b.2.cmp(&a.2));
        top.truncate(40);
        Ok((types, top, handles.len()))
    });
    if ui.button("⟳ Refresh").clicked() {
        app.deep.objects.started = false;
    }
    if !job_status(ui, &app.deep.objects) {
        return;
    }
    let Some((types, top, total)) = app.deep.objects.take_ok() else { return };
    ui.label(RichText::new(format!("{} open handles across {} object types", fmt_num(total as u64), types.len())).color(c_dim()));
    let mut open_pid = None;
    ui.columns(2, |cols| {
        card(&mut cols[0], "Object types", |ui| {
            let mut t = types.clone();
            t.sort_by(|a, b| b.handles.cmp(&a.handles));
            egui_extras::TableBuilder::new(ui)
                .id_salt("objtypes")
                .striped(true)
                .resizable(true)
                .auto_shrink([false, false])
                .column(egui_extras::Column::initial(50.0))
                .column(egui_extras::Column::initial(170.0))
                .column(egui_extras::Column::initial(90.0))
                .column(egui_extras::Column::initial(90.0))
                .column(egui_extras::Column::initial(90.0))
                .column(egui_extras::Column::remainder())
                .header(20.0, |mut h| {
                    for x in ["Index", "Type", "Objects", "Handles", "Peak objects", "Peak handles"] {
                        h.col(|ui| {
                            ui.add(egui::Label::new(RichText::new(x).strong()).truncate());
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_h(), t.len(), |mut row| {
                        let o = &t[row.index()];
                        for v in [o.index.to_string(), o.name.clone(), fmt_num(o.objects as u64), fmt_num(o.handles as u64), fmt_num(o.peak_objects as u64), fmt_num(o.peak_handles as u64)] {
                            row.col(|ui| {
                                ui.label(v);
                            });
                        }
                    });
                });
        });
        card(&mut cols[1], "Processes holding the most handles", |ui| {
            let maxn = top.first().map(|x| x.2).unwrap_or(1) as f32;
            egui::ScrollArea::vertical().id_salt("toph").auto_shrink([false, false]).show(ui, |ui| {
                for (pid, name, n) in &top {
                    ui.horizontal(|ui| {
                        bar(ui, *n as f32 / maxn, ORANGE, &fmt_num(*n as u64), 90.0);
                        if ui.link(format!("{name} ({pid})")).clicked() {
                            open_pid = Some(*pid);
                        }
                    });
                }
            });
        });
    });
    if let Some(pid) = open_pid {
        app.procs.selected = Some(pid);
        app.procs.tab = super::processes::Tab::Handles;
        app.page = super::Page::Inspector;
    }
}

// ------------------------------------------------------------------ ETW

pub fn etw(app: &mut App, ui: &mut Ui) {
    if app.locked(ui, "Live system events uses Event Tracing for Windows to show process starts, network and DNS activity, file creation and more as it happens.") {
        return;
    }
    page_header(ui, "Live system events (ETW)", "A private real-time trace session: pick sources and watch the kernel report what happens.");
    let ctx = ui.ctx().clone();
    let running = app.deep.etw.is_some();
    card(ui, "Sources", |ui| {
        ui.horizontal_wrapped(|ui| {
            for (i, p) in etw::PROVIDERS.iter().enumerate() {
                ui.add_enabled_ui(!running, |ui| {
                    ui.checkbox(&mut app.deep.etw_sel[i], p.name).on_hover_text(p.what);
                });
            }
        });
        ui.horizontal(|ui| {
            if !running {
                if ui.button(RichText::new("▶ Start live trace").color(GREEN)).clicked() {
                    let sel: Vec<usize> = app.deep.etw_sel.iter().enumerate().filter(|x| *x.1).map(|x| x.0).collect();
                    match etw::start(&sel) {
                        Ok(s) => {
                            let msg = format!("Live trace started: {}", s.providers.join(", "));
                            app.deep.etw = Some(s);
                            app.shared.lock().unwrap().log("ETW", Severity::Info, msg, None);
                        }
                        Err(e) => app.notify(e, true),
                    }
                }
            } else {
                if ui.button(RichText::new("■ Stop").color(RED)).clicked() {
                    if let Some(s) = app.deep.etw.take() {
                        let n = s.sink.lock().unwrap().total;
                        s.stop();
                        app.shared.lock().unwrap().log("ETW", Severity::Info, format!("Live trace stopped after {n} events"), None);
                    }
                }
                ui.toggle_value(&mut app.deep.etw_paused, "⏸ Freeze list");
                if ui.button("Clear").clicked() {
                    if let Some(s) = &app.deep.etw {
                        let mut k = s.sink.lock().unwrap();
                        k.events.clear();
                        k.counts.clear();
                    }
                }
            }
            if ui.button("Show all trace sessions on this PC").clicked() {
                app.deep.etw_sessions = Job::default();
                app.deep.etw_sessions.start(&ctx, queries::etw_sessions);
            }
        });
    });
    app.deep.etw_sessions.with(|r| {
        if let Some(r) = r {
            card(ui, "Active trace sessions", |ui| {
                egui::ScrollArea::vertical().id_salt("ets").max_height(150.0).show(ui, |ui| {
                    ui.label(RichText::new(match r {
                        Ok(s) => s.clone(),
                        Err(e) => e.clone(),
                    }).monospace().size(11.0));
                });
            });
        }
    });
    let Some(sess) = &app.deep.etw else {
        notice(ui, "Not running. Pick sources and press Start. Events are decoded field by field; nothing leaves your PC.", BLUE);
        return;
    };
    ctx.request_repaint_after(std::time::Duration::from_millis(250));
    let (events, counts, total) = {
        let k = sess.sink.lock().unwrap();
        let ev: Vec<etw::EtwEvent> = k.events.iter().rev().take(2000).cloned().collect();
        let mut c: Vec<(String, u64)> = k.counts.iter().map(|(a, b)| (a.clone(), *b)).collect();
        c.sort_by(|a, b| b.1.cmp(&a.1));
        (ev, c, k.total)
    };
    ui.horizontal(|ui| {
        pill(ui, &format!("{} events", fmt_num(total)), GREEN);
        ui.label(RichText::new(format!("since {}", util::fmt_clock(sess.started))).color(c_dim()));
        search_field(ui, &mut app.deep.etw_filter, "Filter by provider, event, PID or field value…", 340.0);
    });
    let names: std::collections::HashMap<u32, String> = app.shared.lock().unwrap().procs.values().map(|p| (p.pid, p.name.clone())).collect();
    ui.columns(2, |cols| {
        card(&mut cols[1], "Event counts", |ui| {
            egui::ScrollArea::vertical().id_salt("etwc").max_height(500.0).show(ui, |ui| {
                for (k, n) in counts.iter().take(40) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(fmt_num(*n)).color(CYAN).monospace());
                        ui.label(k);
                    });
                }
            });
        });
        let ui = &mut cols[0];
        let f = app.deep.etw_filter.to_lowercase();
        let list: Vec<&etw::EtwEvent> = events
            .iter()
            .filter(|e| {
                f.is_empty()
                    || e.provider.to_lowercase().contains(&f)
                    || e.name.to_lowercase().contains(&f)
                    || e.pid.to_string() == f
                    || e.props.iter().any(|p| p.1.to_lowercase().contains(&f))
            })
            .collect();
        egui::ScrollArea::vertical().id_salt("etwl").auto_shrink([false, false]).show(ui, |ui| {
            for (i, e) in list.iter().enumerate().take(600) {
                let pname = names.get(&e.pid).cloned().unwrap_or_default();
                let head = format!("{}  {}  {}  pid {} {}", util::fmt_clock(e.time), e.provider, e.name, e.pid, pname);
                let fields = e.props.iter().take(8).map(|(k, v)| format!("{k}={v}")).collect::<Vec<_>>().join("  ");
                let sel = app.deep.etw_selected == Some(i);
                let r = ui.add(egui::SelectableLabel::new(sel, RichText::new(&head).size(12.5).color(CYAN)));
                ui.label(RichText::new(&fields).size(11.5).color(c_dim()));
                if r.clicked() {
                    app.deep.etw_selected = if sel { None } else { Some(i) };
                }
                if sel {
                    egui::Frame::none().fill(c_card()).inner_margin(egui::Margin::same(6.0)).show(ui, |ui| {
                        for (k, v) in &e.props {
                            ui.label(format!("{k}: {v}"));
                        }
                        ui.label(format!("Event id {}  •  thread {}", e.id, e.tid));
                    });
                }
            }
        });
    });
}

// ------------------------------------------------------------------ Event Viewer

pub fn events(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    page_header(ui, "Event Viewer", "Errors and warnings Windows and your apps wrote to the event logs.");
    let d = &mut app.deep;
    ui.horizontal(|ui| {
        ui.label("Log");
        egui::ComboBox::from_id_salt("evlog").selected_text(&d.ev_log).width(260.0).show_ui(ui, |ui| {
            for l in ["System", "Application", "Security", "Setup", "Microsoft-Windows-Diagnostics-Performance/Operational", "Microsoft-Windows-WindowsUpdateClient/Operational", "Microsoft-Windows-Kernel-Power/Thermal-Operational"] {
                ui.selectable_value(&mut d.ev_log, l.to_string(), l);
            }
            d.logs.with(|r| {
                if let Some(Ok(rows)) = r {
                    ui.separator();
                    for row in rows {
                        let n = jstr(row, "LogName");
                        ui.selectable_value(&mut d.ev_log, n.clone(), format!("{n} ({})", jstr(row, "RecordCount")));
                    }
                }
            });
        });
        if ui.small_button("More logs…").clicked() {
            d.logs = Job::default();
            d.logs.start(ctx, queries::event_logs);
        }
        for (i, l) in ["Critical", "Error", "Warning", "Information"].iter().enumerate() {
            ui.checkbox(&mut d.ev_levels[i], *l);
        }
        ui.label("Last");
        egui::ComboBox::from_id_salt("evh").selected_text(format!("{} h", d.ev_hours)).show_ui(ui, |ui| {
            for h in [1, 6, 24, 72, 168, 720] {
                ui.selectable_value(&mut d.ev_hours, h, format!("{h} h"));
            }
        });
        if ui.button("Load").clicked() || !d.events.started {
            let log = d.ev_log.clone();
            let lv: Vec<u8> = d.ev_levels.iter().enumerate().filter(|x| *x.1).map(|x| x.0 as u8 + 1).flat_map(|l| if l == 4 { vec![0, 4] } else { vec![l] }).collect();
            let h = d.ev_hours;
            d.events = Job::default();
            d.events.start(ctx, move || queries::events(&log, &lv, h, 500));
        }
    });
    if app.deep.ev_log == "Security" {
        app.admin_hint(ui, "The Security log can only be read in the Admin view.");
    }
    search_field(ui, &mut app.deep.events_t.filter, "🔍 Filter events…", 300.0);
    if !job_status(ui, &app.deep.events) {
        return;
    }
    let rows = app.deep.events.take_ok().unwrap_or_default();
    if let Some(r) = app.deep.events_t.selected.and_then(|i| rows.get(i)) {
        card(ui, &format!("{} — event {}", jstr(r, "ProviderName"), jstr(r, "Id")), |ui| {
            ui.label(jstr(r, "Message"));
            ui.label(RichText::new(format!("{}  •  {}  •  {}", jstr(r, "Time"), jstr(r, "Level"), jstr(r, "LogName"))).color(c_dim()));
        });
    }
    let cols = [col("Time", "Time", 140.0), col("Level", "Level", 80.0), col("ProviderName", "Source", 220.0), col("Id", "Event", 60.0), col("Message", "Message", 600.0)];
    json_table(
        ui,
        "events",
        &rows,
        &cols,
        &mut app.deep.events_t,
        &|r| match jstr(r, "Level").as_str() {
            "Critical" | "Error" => Some(RED),
            "Warning" => Some(YELLOW),
            _ => None,
        },
        f32::INFINITY,
    );
}

// ------------------------------------------------------------------ timeline

pub fn timeline(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Diagnostics timeline", "Everything that happened, in order: process starts and exits, spikes, alerts, automation, snapshots and Windows errors.");
    ui.horizontal_wrapped(|ui| {
        search_field(ui, &mut app.deep.tl_filter, "🔍 Filter…", 260.0);
        for (c, on) in app.deep.tl_cats.iter_mut() {
            ui.checkbox(on, *c);
        }
        ui.checkbox(&mut app.deep.tl_include_events, "Windows errors (from Event Viewer page)");
    });
    let s = app.shared.lock().unwrap();
    let mut items: Vec<(i64, String, Severity, String)> = s
        .timeline
        .iter()
        .filter(|e| app.deep.tl_cats.iter().find(|c| c.0 == e.category).map(|c| c.1).unwrap_or(true))
        .map(|e| (e.time, e.category.to_string(), e.severity, e.text.clone()))
        .collect();
    let hist_times: Vec<i64> = s.hist.times.iter().copied().collect();
    let cpu: Vec<f32> = s.hist.cpu.data.iter().copied().collect();
    drop(s);
    if app.deep.tl_include_events {
        app.deep.events.with(|r| {
            if let Some(Ok(rows)) = r {
                for e in rows {
                    let lvl = jstr(e, "Level");
                    let sev = if lvl == "Error" || lvl == "Critical" { Severity::Critical } else if lvl == "Warning" { Severity::Warning } else { Severity::Info };
                    items.push((jf64(e, "Unix") as i64, "Windows log".into(), sev, format!("{} {}: {}", jstr(e, "ProviderName"), jstr(e, "Id"), jstr(e, "Message"))));
                }
            }
        });
    }
    let f = app.deep.tl_filter.to_lowercase();
    items.retain(|x| f.is_empty() || x.3.to_lowercase().contains(&f) || x.1.to_lowercase().contains(&f));
    items.sort_by(|a, b| b.0.cmp(&a.0));
    // density strip with CPU history behind it
    card(ui, "", |ui| {
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 70.0), egui::Sense::hover());
        let p = ui.painter();
        p.rect_filled(rect, 4.0, c_bg());
        if let (Some(&t0), Some(&t1)) = (hist_times.first(), hist_times.last()) {
            let span = (t1 - t0).max(1) as f32;
            let pts: Vec<egui::Pos2> = hist_times.iter().zip(cpu.iter()).map(|(&t, &c)| egui::pos2(rect.left() + (t - t0) as f32 / span * rect.width(), rect.bottom() - c / 100.0 * rect.height())).collect();
            p.add(egui::Shape::line(pts, egui::Stroke::new(1.0, CYAN.gamma_multiply(0.6))));
            for (t, _, sev, _) in &items {
                if *t >= t0 && *t <= t1 {
                    let x = rect.left() + (*t - t0) as f32 / span * rect.width();
                    p.line_segment([egui::pos2(x, rect.top()), egui::pos2(x, rect.top() + 14.0)], egui::Stroke::new(2.0, sev_color(*sev)));
                }
            }
            p.text(rect.left_bottom() + egui::vec2(4.0, -4.0), egui::Align2::LEFT_BOTTOM, util::fmt_clock(t0), egui::FontId::proportional(10.0), c_dim());
            p.text(rect.right_bottom() + egui::vec2(-4.0, -4.0), egui::Align2::RIGHT_BOTTOM, util::fmt_clock(t1), egui::FontId::proportional(10.0), c_dim());
        }
    });
    ui.label(RichText::new(format!("{} events", items.len())).color(c_dim()));
    egui::ScrollArea::vertical().id_salt("tl").auto_shrink([false, false]).show(ui, |ui| {
        for (t, cat, sev, text) in items.iter().take(1500) {
            ui.horizontal(|ui| {
                ui.label(RichText::new("⏺").color(sev_color(*sev)));
                ui.label(RichText::new(util::fmt_time(*t)).color(c_dim()).monospace().size(12.0));
                ui.add_sized([90.0, 18.0], egui::Label::new(RichText::new(cat).color(CYAN).size(12.0)));
                ui.add(egui::Label::new(RichText::new(text).size(12.5)).truncate());
            });
        }
    });
}

// ------------------------------------------------------------------ recording

fn recordings() -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(util::sub_dir("recordings"))
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|x| x == "csv").unwrap_or(false)).collect())
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

fn load_csv(p: &std::path::Path) -> Option<(String, Vec<String>, Vec<Vec<f32>>)> {
    let text = std::fs::read_to_string(p).ok()?;
    let mut lines = text.lines();
    let header: Vec<String> = lines.next()?.split(',').map(|s| s.to_string()).collect();
    let mut cols = vec![vec![]; header.len()];
    for l in lines {
        for (i, v) in l.split(',').enumerate() {
            if i < cols.len() {
                cols[i].push(v.parse::<f32>().unwrap_or(f32::NAN));
            }
        }
    }
    Some((p.file_name()?.to_string_lossy().to_string(), header, cols))
}

pub fn recording(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Performance recording", "Record every counter to a file (CSV) you can reopen here or in Excel.");
    let shared = app.shared.clone();
    {
        let mut s = shared.lock().unwrap();
        card(ui, "Recorder", |ui| {
            if let Some(r) = &s.recorder {
                ui.horizontal(|ui| {
                    pill(ui, "⏺ Recording", RED);
                    ui.label(format!("{} rows, {} elapsed", r.rows, util::fmt_dur(r.started.elapsed().as_secs())));
                    if let Some(u) = r.until {
                        ui.label(RichText::new(format!("stops in {}s", u.saturating_duration_since(std::time::Instant::now()).as_secs())).color(c_dim()));
                    }
                });
                ui.label(RichText::new(r.path.display().to_string()).color(c_dim()).size(11.5));
                if ui.button("■ Stop recording").clicked() {
                    s.stop_recording();
                }
            } else {
                ui.horizontal(|ui| {
                    ui.label("Length:");
                    egui::ComboBox::from_id_salt("rec_len").selected_text(if app.deep.rec_secs == 0 { "Until stopped".to_string() } else { util::fmt_dur(app.deep.rec_secs as u64) }).show_ui(ui, |ui| {
                        for x in [60, 300, 900, 3600, 0] {
                            ui.selectable_value(&mut app.deep.rec_secs, x, if x == 0 { "Until stopped".to_string() } else { util::fmt_dur(x as u64) });
                        }
                    });
                    if ui.button(RichText::new("⏺ Start recording").color(RED)).clicked() {
                        let secs = if app.deep.rec_secs == 0 { None } else { Some(app.deep.rec_secs) };
                        if let Err(e) = s.start_recording(secs) {
                            app.toasts.push(super::Toast { text: e, error: true, at: std::time::Instant::now() });
                        }
                    }
                });
                ui.label(RichText::new("Each row holds CPU, kernel time, memory, commit, disk, network, GPU, temperature, process/thread/handle counts and the top CPU and memory users.").color(c_dim()).size(12.0));
            }
        });
    }
    ui.add_space(6.0);
    let recs = recordings();
    card(ui, &format!("Saved recordings ({})", recs.len()), |ui| {
        if ui.button("📂 Open folder").clicked() {
            util::open_path(&util::sub_dir("recordings"));
        }
        egui::ScrollArea::vertical().id_salt("recs").max_height(260.0).show(ui, |ui| {
            for r in &recs {
                ui.horizontal(|ui| {
                    let size = std::fs::metadata(r).map(|m| m.len()).unwrap_or(0);
                    ui.label(r.file_name().unwrap_or_default().to_string_lossy());
                    ui.label(RichText::new(fmt_bytes(size as f64)).color(c_dim()));
                    if ui.button("Show in graphs").clicked() {
                        app.deep.loaded = load_csv(r);
                        app.page = super::Page::Graphs;
                    }
                });
            }
        });
    });
}

// ------------------------------------------------------------------ historical graphs

pub fn graphs(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Historical graphs", "Up to the last hour of every measure kept in memory, or a saved recording.");
    ui.horizontal(|ui| {
        ui.label("Range");
        for (n, l) in [(120, "2 min"), (600, "10 min"), (1800, "30 min"), (3600, "1 hour")] {
            ui.selectable_value(&mut app.deep.graph_range, n, l);
        }
        if let Some((name, _, _)) = &app.deep.loaded {
            pill(ui, &format!("Showing recording {name}"), ORANGE);
            if ui.button("Back to live history").clicked() {
                app.deep.loaded = None;
            }
        }
    });
    let n = app.deep.graph_range;
    egui::ScrollArea::vertical().id_salt("graphs").auto_shrink([false, false]).show(ui, |ui| {
        if let Some((_, header, cols)) = &app.deep.loaded {
            let get = |name: &str| header.iter().position(|h| h == name).map(|i| cols[i].iter().map(|v| if v.is_nan() { 0.0 } else { *v }).collect::<Vec<f32>>()).unwrap_or_default();
            let len = cols.first().map(|c| c.len()).unwrap_or(0).max(2);
            card(ui, "CPU and memory (%)", |ui| chart(ui, &[Series { data: get("cpu_pct"), color: CYAN, label: "CPU" }, Series { data: get("mem_used_pct"), color: PURPLE, label: "Memory" }], 180.0, Some(100.0), len, &pct));
            card(ui, "Disk", |ui| chart(ui, &[Series { data: get("disk_read_bps"), color: TEAL, label: "Read" }, Series { data: get("disk_write_bps"), color: GREEN, label: "Write" }], 150.0, None, len, &rate));
            card(ui, "Network", |ui| chart(ui, &[Series { data: get("net_rx_bps"), color: ORANGE, label: "Down" }, Series { data: get("net_tx_bps"), color: CYAN, label: "Up" }], 150.0, None, len, &rate));
            card(ui, "GPU (%)", |ui| chart(ui, &[Series { data: get("gpu_pct"), color: PINK, label: "GPU" }], 120.0, Some(100.0), len, &pct));
            return;
        }
        let s = app.shared.lock().unwrap();
        let h = &s.hist;
        ui.label(RichText::new(format!("{} samples kept (about {})", h.cpu.len(), util::fmt_dur((h.cpu.len() as u64 * s.config.sample_ms) / 1000))).color(c_dim()));
        card(ui, "CPU (%)", |ui| chart(ui, &[Series { data: h.cpu.to_vec(n), color: CYAN, label: "Total" }, Series { data: h.cpu_kernel.to_vec(n), color: BLUE, label: "Kernel" }], 170.0, Some(100.0), n, &pct));
        ui.add_space(6.0);
        card(ui, "Memory (%)", |ui| chart(ui, &[Series { data: h.mem.to_vec(n), color: PURPLE, label: "In use" }, Series { data: h.commit.to_vec(n), color: PINK, label: "Commit" }], 150.0, Some(100.0), n, &pct));
        ui.add_space(6.0);
        ui.columns(2, |cols| {
            card(&mut cols[0], "Disk throughput", |ui| chart(ui, &[Series { data: h.disk_read.to_vec(n), color: TEAL, label: "Read" }, Series { data: h.disk_write.to_vec(n), color: GREEN, label: "Write" }], 140.0, None, n, &rate));
            card(&mut cols[1], "Disk active (%)", |ui| chart(ui, &[Series { data: h.disk.to_vec(n), color: GREEN, label: "Busiest" }], 140.0, Some(100.0), n, &pct));
        });
        ui.add_space(6.0);
        ui.columns(2, |cols| {
            card(&mut cols[0], "Network", |ui| chart(ui, &[Series { data: h.net_rx.to_vec(n), color: ORANGE, label: "Down" }, Series { data: h.net_tx.to_vec(n), color: CYAN, label: "Up" }], 140.0, None, n, &rate));
            card(&mut cols[1], "GPU (%)", |ui| chart(ui, &[Series { data: h.gpu.to_vec(n), color: PINK, label: "GPU" }], 140.0, Some(100.0), n, &pct));
        });
        ui.add_space(6.0);
        ui.columns(2, |cols| {
            card(&mut cols[0], "Temperature (°C)", |ui| chart(ui, &[Series { data: h.temp.to_vec(n), color: RED, label: "Hottest" }], 120.0, None, n, &|v| format!("{v:.0} °C")));
            card(&mut cols[1], "Process count", |ui| chart(ui, &[Series { data: h.procs.to_vec(n), color: YELLOW, label: "Processes" }], 120.0, None, n, &|v| format!("{v:.0}")));
        });
    });
}

// ------------------------------------------------------------------ sensors

pub fn sensors(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    page_header(ui, "Hardware sensors", "Temperatures, processor clocks, fans and battery as Windows reports them.");
    {
        let s = app.shared.lock().unwrap();
        ui.columns(2, |cols| {
            card(&mut cols[0], "Thermal zones (live)", |ui| {
                if s.temps.is_empty() {
                    ui.label(RichText::new("This PC reports no thermal zones to Windows.").color(c_dim()));
                }
                for (n, t) in &s.temps {
                    ui.horizontal(|ui| {
                        bar(ui, *t / 105.0, load_color(*t), &format!("{t:.1} °C"), 110.0);
                        ui.label(n.split('\\').last().unwrap_or(n));
                    });
                }
                chart(ui, &[Series { data: s.hist.temp.to_vec(300), color: RED, label: "Hottest" }], 100.0, None, 300, &|v| format!("{v:.0} °C"));
            });
            card(&mut cols[1], "Processor clocks (live)", |ui| {
                ui.label(format!("Performance vs base speed: {:.0}%", s.sys.perf_pct));
                egui::Grid::new("clk").striped(true).show(ui, |ui| {
                    for (i, c) in s.cores.iter().enumerate() {
                        ui.label(format!("CPU {i}"));
                        ui.label(format!("{} MHz", c.mhz));
                        ui.label(RichText::new(format!("max {} MHz", c.max_mhz)).color(c_dim()));
                        if i % 2 == 1 {
                            ui.end_row();
                        }
                    }
                });
            });
        });
    }
    ui.add_space(6.0);
    app.manage_hint_sensors(ui);
    app.deep.sensors.ensure(ctx, queries::sensors);
    if ui.button("⟳ Refresh").clicked() {
        app.deep.sensors.started = false;
    }
    if !job_status(ui, &app.deep.sensors) {
        return;
    }
    let v = app.deep.sensors.take_ok().unwrap_or_default();
    ui.columns(2, |cols| {
        card(&mut cols[0], "ACPI thermal zones (detailed)", |ui| {
            let z = value_list(jget(&v, "ThermalZones"));
            if z.is_empty() {
                ui.label(RichText::new("Not available (needs Admin view, or not provided by this PC's firmware).").color(c_dim()));
            }
            for t in z {
                ui.label(format!("{}: {} °C (critical at {} °C)", jstr(&t, "Zone"), jstr(&t, "Celsius"), jstr(&t, "CriticalC")));
            }
            ui.separator();
            let fans = value_list(jget(&v, "Fans"));
            if fans.is_empty() {
                ui.label(RichText::new("No fan information exposed to Windows.").color(c_dim()));
            }
            for f in fans {
                ui.label(format!("{}: {} ({})", jstr(&f, "Name"), jstr(&f, "DesiredSpeed"), jstr(&f, "Status")));
            }
            let nv = jstr(&v, "NvidiaSmi");
            if !nv.is_empty() {
                ui.separator();
                ui.label(RichText::new("NVIDIA GPU (name, °C, fan, power, core clock, memory clock, use, memory)").strong());
                ui.label(RichText::new(nv).monospace());
            }
        });
        card(&mut cols[1], "Battery", |ui| {
            let bs = value_list(jget(&v, "BatteryStatus"));
            let full = value_list(jget(&v, "FullCapacity"));
            let design = value_list(jget(&v, "DesignCapacity"));
            if bs.is_empty() && design.is_empty() {
                ui.label(RichText::new("No battery data.").color(c_dim()));
            }
            for b in &bs {
                kv(
                    ui,
                    "bat",
                    &[
                        ("Remaining", format!("{} mWh", jstr(b, "RemainingCapacity"))),
                        ("Charge rate", format!("{} mW", jstr(b, "ChargeRate"))),
                        ("Discharge rate", format!("{} mW", jstr(b, "DischargeRate"))),
                        ("Voltage", format!("{} mV", jstr(b, "Voltage"))),
                        ("Plugged in", jstr(b, "PowerOnline")),
                    ],
                );
            }
            if let (Some(f), Some(d)) = (full.first(), design.first()) {
                let fc = jf64(f, "FullChargedCapacity");
                let dc = jf64(d, "DesignedCapacity");
                if dc > 0.0 {
                    let health = fc / dc * 100.0;
                    ui.label(RichText::new(format!("Battery health: {health:.0}% of design ({fc:.0} of {dc:.0} mWh)")).color(if health > 80.0 { GREEN } else if health > 60.0 { YELLOW } else { RED }).strong());
                }
            }
        });
    });
}

impl App {
    fn manage_hint_sensors(&mut self, ui: &mut Ui) {
        self.admin_hint(ui, "Detailed ACPI temperatures and battery wear need the Admin view.");
        ui.label(RichText::new("Fan speeds, voltages and GPU temperatures are only exposed by vendor tools on most PCs; where Windows has no reading, it is marked as not available.").color(c_dim()).size(11.5));
    }
}

// ------------------------------------------------------------------ disk health

pub fn disk_health(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    page_header(ui, "SSD and disk health", "Health status, wear, temperature, power-on hours and error counters from each drive.");
    app.admin_hint(ui, "Wear, temperature and error counters (reliability data) need the Admin view.");
    app.deep.disks.ensure(ctx, queries::disk_health);
    if ui.button("⟳ Refresh").clicked() {
        app.deep.disks.started = false;
    }
    if !job_status(ui, &app.deep.disks) {
        return;
    }
    let v = app.deep.disks.take_ok().unwrap_or_default();
    let disks = value_list(jget(&v, "Disks"));
    let rel = value_list(jget(&v, "Reliability"));
    let fp = value_list(jget(&v, "FailurePredict"));
    egui::ScrollArea::vertical().id_salt("dh").auto_shrink([false, false]).show(ui, |ui| {
        for d in &disks {
            let id = jstr(d, "DeviceId");
            let r = rel.iter().find(|x| jstr(x, "DeviceId") == id);
            let health = jstr(d, "Health");
            let hc = if health == "Healthy" { GREEN } else if health == "Warning" { YELLOW } else { RED };
            card(ui, &format!("{}  ({}, {})", jstr(d, "FriendlyName"), jstr(d, "MediaType"), jstr(d, "BusType")), |ui| {
                ui.horizontal(|ui| {
                    pill(ui, &format!("Health: {health}"), hc);
                    pill(ui, &jstr(d, "Operational"), BLUE);
                    ui.label(RichText::new(format!("{}  •  firmware {}", util::fmt_bytes(jf64(d, "Size")), jstr(d, "FirmwareVersion"))).color(c_dim()));
                });
                if let Some(r) = r {
                    let wear = jf64(r, "Wear");
                    ui.horizontal(|ui| {
                        ui.label("Wear (life used):");
                        bar(ui, (wear / 100.0) as f32, load_color(wear as f32), &format!("{wear:.0}%"), 200.0);
                    });
                    kv(
                        ui,
                        &format!("rel{id}"),
                        &[
                            ("Temperature", format!("{} °C (max {} °C)", jstr(r, "Temperature"), jstr(r, "TemperatureMax"))),
                            ("Power-on hours", format!("{} h", jstr(r, "PowerOnHours"))),
                            ("Read errors", format!("{} total, {} uncorrected", jstr(r, "ReadErrorsTotal"), jstr(r, "ReadErrorsUncorrected"))),
                            ("Write errors", format!("{} total, {} uncorrected", jstr(r, "WriteErrorsTotal"), jstr(r, "WriteErrorsUncorrected"))),
                            ("Worst read / write latency", format!("{} / {} ms", jstr(r, "ReadLatencyMax"), jstr(r, "WriteLatencyMax"))),
                            ("Start/stop cycles", jstr(r, "StartStopCycleCount")),
                        ],
                    );
                } else {
                    ui.label(RichText::new("Reliability counters unavailable (Admin view needed, or the drive does not report them).").color(c_dim()));
                }
            });
            ui.add_space(6.0);
        }
        if !fp.is_empty() {
            card(ui, "SMART failure prediction", |ui| {
                for f in &fp {
                    let bad = jget(f, "PredictFailure").as_bool() == Some(true);
                    ui.label(RichText::new(format!("{}: {}", jstr(f, "InstanceName"), if bad { "FAILURE PREDICTED — back up now" } else { "OK" })).color(if bad { RED } else { GREEN }));
                }
            });
            ui.add_space(6.0);
        }
        card(ui, "Volumes", |ui| {
            for vo in value_list(jget(&v, "Volumes")) {
                ui.label(format!(
                    "{}:  {}  {}  {}  {} free of {}",
                    jstr(&vo, "DriveLetter"),
                    jstr(&vo, "FileSystemLabel"),
                    jstr(&vo, "FileSystem"),
                    jstr(&vo, "Health"),
                    util::fmt_bytes(jf64(&vo, "SizeRemaining")),
                    util::fmt_bytes(jf64(&vo, "Size"))
                ));
            }
            let trim = jstr(&v, "Trim");
            if !trim.is_empty() {
                ui.label(RichText::new(format!("TRIM: {}", trim.trim())).color(c_dim()));
            }
        });
    });
}
