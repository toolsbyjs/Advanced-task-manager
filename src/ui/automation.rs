//! Automation pages: rule engine, alerts, system and process profiles,
//! scheduled monitoring, script integrations and settings.

use super::widgets::*;
use super::App;
use crate::collector::{self, Severity};
use crate::config::{Action, Metric, Op, Priority, ProcProfile, Rule, Schedule, ScheduleKind, Script, SysProfile};
use crate::util;
use egui::{RichText, Ui};
use std::sync::{Arc, Mutex};

#[derive(Default)]
pub struct AutoUi {
    pub editing_rule: Option<Rule>,
    pub editing_proc: Option<ProcProfile>,
    pub editing_sys: Option<SysProfile>,
    pub editing_sched: Option<Schedule>,
    pub editing_script: Option<Script>,
    pub script_output: Arc<Mutex<Vec<(String, String)>>>,
    pub sev_filter: Option<Severity>,
}

fn metric_combo(ui: &mut Ui, m: &mut Metric) {
    egui::ComboBox::from_id_salt("metric").selected_text(m.label()).width(240.0).show_ui(ui, |ui| {
        for x in Metric::ALL {
            ui.selectable_value(m, x, x.label());
        }
    });
}

fn action_editor(ui: &mut Ui, a: &mut Action, scripts: &[Script], profiles: &[SysProfile]) {
    let kinds = ["Show an alert", "End the process", "Suspend the process", "Set priority", "Efficiency mode", "Run script", "Record performance", "Apply system profile"];
    let mut k = match a {
        Action::Alert => 0,
        Action::KillProcess => 1,
        Action::SuspendProcess => 2,
        Action::SetPriority(_) => 3,
        Action::EfficiencyMode => 4,
        Action::RunScript(_) => 5,
        Action::StartRecording(_) => 6,
        Action::ApplySystemProfile(_) => 7,
    };
    let before = k;
    egui::ComboBox::from_id_salt("act").selected_text(kinds[k]).width(200.0).show_ui(ui, |ui| {
        for (i, n) in kinds.iter().enumerate() {
            ui.selectable_value(&mut k, i, *n);
        }
    });
    if k != before {
        *a = match k {
            0 => Action::Alert,
            1 => Action::KillProcess,
            2 => Action::SuspendProcess,
            3 => Action::SetPriority(Priority::BelowNormal),
            4 => Action::EfficiencyMode,
            5 => Action::RunScript(scripts.first().map(|s| s.name.clone()).unwrap_or_default()),
            6 => Action::StartRecording(300),
            _ => Action::ApplySystemProfile(profiles.first().map(|p| p.name.clone()).unwrap_or_default()),
        };
    }
    match a {
        Action::SetPriority(p) => {
            egui::ComboBox::from_id_salt("act_pri").selected_text(p.label()).show_ui(ui, |ui| {
                for x in Priority::ALL {
                    ui.selectable_value(p, x, x.label());
                }
            });
        }
        Action::RunScript(s) => {
            egui::ComboBox::from_id_salt("act_script").selected_text(s.as_str()).show_ui(ui, |ui| {
                for sc in scripts {
                    ui.selectable_value(s, sc.name.clone(), &sc.name);
                }
            });
        }
        Action::StartRecording(secs) => {
            ui.add(egui::DragValue::new(secs).range(10..=86400).suffix(" s"));
        }
        Action::ApplySystemProfile(n) => {
            egui::ComboBox::from_id_salt("act_prof").selected_text(n.as_str()).show_ui(ui, |ui| {
                for p in profiles {
                    ui.selectable_value(n, p.name.clone(), &p.name);
                }
            });
        }
        _ => {}
    }
}

pub fn rules(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Rule engine", "When a condition holds for long enough, do something: alert, end, suspend, re-prioritise, run a script, record or apply a profile.");
    let shared = app.shared.clone();
    if ui.button("✚ New rule").clicked() {
        let mut s = shared.lock().unwrap();
        let id = s.config.new_id();
        app.auto.editing_rule = Some(Rule {
            id,
            name: "New rule".into(),
            enabled: true,
            metric: Metric::ProcessCpu,
            op: Op::Above,
            threshold: 50.0,
            duration_s: 10,
            target: "*".into(),
            action: Action::Alert,
            cooldown_s: 120,
            fired: 0,
        });
    }
    if !app.admin() {
        notice(ui, "🛡 Rules that end, suspend or re-prioritise other users' or system processes only succeed in the Admin view.", PURPLE);
    }
    let (scripts, profiles) = {
        let s = shared.lock().unwrap();
        (s.config.scripts.clone(), s.config.sys_profiles.clone())
    };
    let mut save: Option<Rule> = None;
    let mut cancel = false;
    if let Some(r) = app.auto.editing_rule.as_mut() {
        card(ui, "Edit rule", |ui| {
            egui::Grid::new("rule_ed").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut r.name);
                ui.end_row();
                ui.label("When");
                ui.horizontal(|ui| {
                    metric_combo(ui, &mut r.metric);
                    if !r.metric.is_event() {
                        egui::ComboBox::from_id_salt("op").selected_text(if r.op == Op::Above { "is above" } else { "is below" }).show_ui(ui, |ui| {
                            ui.selectable_value(&mut r.op, Op::Above, "is above");
                            ui.selectable_value(&mut r.op, Op::Below, "is below");
                        });
                        ui.add(egui::DragValue::new(&mut r.threshold).speed(1.0));
                    }
                });
                ui.end_row();
                if r.metric.is_process() {
                    ui.label("For processes named");
                    ui.horizontal(|ui| {
                        ui.text_edit_singleline(&mut r.target);
                        ui.label(RichText::new("use * as wildcard, e.g. chrome.exe or *.exe").color(c_dim()));
                    });
                    ui.end_row();
                }
                if !r.metric.is_event() {
                    ui.label("For at least");
                    ui.add(egui::DragValue::new(&mut r.duration_s).range(0..=86400).suffix(" seconds"));
                    ui.end_row();
                }
                ui.label("Then");
                ui.horizontal(|ui| action_editor(ui, &mut r.action, &scripts, &profiles));
                ui.end_row();
                ui.label("Don't repeat for");
                ui.add(egui::DragValue::new(&mut r.cooldown_s).range(0..=86400).suffix(" seconds"));
                ui.end_row();
                ui.label("Enabled");
                ui.checkbox(&mut r.enabled, "");
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Save rule").color(GREEN)).clicked() {
                    save = Some(r.clone());
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    }
    if let Some(r) = save {
        let mut s = shared.lock().unwrap();
        if let Some(x) = s.config.rules.iter_mut().find(|x| x.id == r.id) {
            *x = r;
        } else {
            s.config.rules.push(r);
        }
        s.config_dirty = true;
        app.auto.editing_rule = None;
    }
    if cancel {
        app.auto.editing_rule = None;
    }
    ui.add_space(6.0);
    let mut s = shared.lock().unwrap();
    let mut remove = None;
    let mut edit = None;
    let mut dirty = false;
    egui::ScrollArea::vertical().id_salt("rules").auto_shrink([false, false]).show(ui, |ui| {
        for (i, r) in s.config.rules.iter_mut().enumerate() {
            egui::Frame::none().fill(c_card()).rounding(8.0).inner_margin(egui::Margin::same(10.0)).stroke(egui::Stroke::new(1.0, c_border())).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    dirty |= ui.checkbox(&mut r.enabled, "").changed();
                    ui.label(RichText::new(&r.name).strong().size(14.5));
                    pill(ui, &format!("fired {} times", r.fired), if r.fired > 0 { YELLOW } else { c_dim() });
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("🗑").clicked() {
                            remove = Some(i);
                        }
                        if ui.button("✏ Edit").clicked() {
                            edit = Some(r.clone());
                        }
                    });
                });
                let cond = if r.metric.is_event() {
                    format!("When {} matches '{}'", r.metric.label(), r.target)
                } else {
                    format!(
                        "When {}{} is {} {} for {}s",
                        r.metric.label(),
                        if r.metric.is_process() { format!(" of '{}'", r.target) } else { String::new() },
                        if r.op == Op::Above { "above" } else { "below" },
                        r.threshold,
                        r.duration_s
                    )
                };
                ui.label(RichText::new(format!("{cond}  ➡  {}  (then wait {}s)", r.action.label(), r.cooldown_s)).color(c_dim()));
            });
            ui.add_space(4.0);
        }
    });
    if let Some(i) = remove {
        s.config.rules.remove(i);
        dirty = true;
    }
    if dirty {
        s.config_dirty = true;
    }
    drop(s);
    if let Some(r) = edit {
        app.auto.editing_rule = Some(r);
    }
}

pub fn alerts(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Alerts", "Raised by your rules. Acknowledge them once you've looked.");
    let shared = app.shared.clone();
    let mut s = shared.lock().unwrap();
    ui.horizontal(|ui| {
        if ui.button("✔ Acknowledge all").clicked() {
            for a in s.alerts.iter_mut() {
                a.acknowledged = true;
            }
        }
        if ui.button("🗑 Clear all").clicked() {
            s.alerts.clear();
        }
        ui.label("Show:");
        ui.selectable_value(&mut app.auto.sev_filter, None, "All");
        ui.selectable_value(&mut app.auto.sev_filter, Some(Severity::Critical), "Critical");
        ui.selectable_value(&mut app.auto.sev_filter, Some(Severity::Warning), "Warning");
        ui.selectable_value(&mut app.auto.sev_filter, Some(Severity::Info), "Info");
        if ui.button("Test alert").clicked() {
            s.alert(Severity::Info, "Test alert".into(), "Alerts are working.".into());
        }
    });
    let filter = app.auto.sev_filter;
    egui::ScrollArea::vertical().id_salt("alerts").auto_shrink([false, false]).show(ui, |ui| {
        if s.alerts.is_empty() {
            notice(ui, "No alerts. Rules on the Rules page watch the system and raise alerts here.", BLUE);
        }
        for a in s.alerts.iter_mut().rev().filter(|a| filter.map(|f| f == a.severity).unwrap_or(true)) {
            egui::Frame::none().fill(if a.acknowledged { c_card() } else { c_card_hi() }).rounding(8.0).inner_margin(egui::Margin::same(8.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    pill(ui, sev_label(a.severity), sev_color(a.severity));
                    ui.label(RichText::new(util::fmt_time(a.time)).color(c_dim()).monospace());
                    ui.label(RichText::new(&a.title).strong());
                    if !a.acknowledged {
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.small_button("Acknowledge").clicked() {
                                a.acknowledged = true;
                            }
                        });
                    }
                });
                ui.label(RichText::new(&a.detail).color(c_dim()));
            });
            ui.add_space(3.0);
        }
    });
}

pub fn apply_sys_profile_by_name(app: &mut App, name: &str) {
    let p = app.shared.lock().unwrap().config.sys_profiles.iter().find(|p| p.name == name).cloned();
    if let Some(p) = p {
        apply_sys_profile(app, &p);
    }
}

pub fn apply_sys_profile(app: &mut App, p: &SysProfile) {
    let mut log = vec![];
    if !p.power_plan.trim().is_empty() {
        match crate::queries::set_power_plan(p.power_plan.trim()) {
            Ok(_) => log.push("power plan set".to_string()),
            Err(e) => log.push(format!("power plan failed: {e}")),
        }
    }
    let procs: Vec<(u32, String)> = app.shared.lock().unwrap().procs.values().map(|x| (x.pid, x.name.clone())).collect();
    for (pid, name) in &procs {
        if p.end_processes.iter().any(|pat| util::wildcard_match(pat, name)) && crate::sys::terminate(*pid).is_ok() {
            log.push(format!("ended {name}"));
        }
        if p.lower_priority.iter().any(|pat| util::wildcard_match(pat, name)) && crate::sys::set_priority(*pid, Priority::BelowNormal.class()).is_ok() {
            log.push(format!("lowered {name}"));
        }
        if p.raise_priority.iter().any(|pat| util::wildcard_match(pat, name)) && crate::sys::set_priority(*pid, Priority::AboveNormal.class()).is_ok() {
            log.push(format!("raised {name}"));
        }
    }
    let mut s = app.shared.lock().unwrap();
    if p.sample_ms >= 250 {
        s.config.sample_ms = p.sample_ms;
        s.config_dirty = true;
    }
    let msg = format!("System profile '{}' applied: {}", p.name, if log.is_empty() { "nothing to change".into() } else { log.join(", ") });
    s.log("Automation", Severity::Info, msg.clone(), None);
    drop(s);
    app.notify(msg, false);
}

fn list_edit(ui: &mut Ui, label: &str, v: &mut Vec<String>) {
    let mut text = v.join(", ");
    ui.label(label);
    if ui.add(egui::TextEdit::singleline(&mut text).desired_width(420.0).hint_text("names, comma separated, * allowed")).changed() {
        *v = text.split(',').map(|s| s.trim().to_string()).filter(|s| !s.is_empty()).collect();
    }
    ui.end_row();
}

pub fn profiles(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Profiles", "System profiles switch many settings at once. Process profiles apply a priority, CPU set or efficiency mode every time an app starts.");
    let shared = app.shared.clone();
    egui::ScrollArea::vertical().id_salt("profiles").auto_shrink([false, false]).show(ui, |ui| {
        ui.columns(2, |cols| {
            // ---------------- system profiles
            let ui = &mut cols[0];
            ui.label(RichText::new("System profiles").size(16.0).strong());
            if ui.button("✚ New system profile").clicked() {
                let id = shared.lock().unwrap().config.new_id();
                app.auto.editing_sys = Some(SysProfile { id, name: "New profile".into(), description: String::new(), power_plan: String::new(), end_processes: vec![], lower_priority: vec![], raise_priority: vec![], sample_ms: 1000 });
            }
            let mut save = None;
            let mut cancel = false;
            if let Some(p) = app.auto.editing_sys.as_mut() {
                card(ui, "Edit system profile", |ui| {
                    egui::Grid::new("sysp").num_columns(2).show(ui, |ui| {
                        ui.label("Name");
                        ui.text_edit_singleline(&mut p.name);
                        ui.end_row();
                        ui.label("Description");
                        ui.text_edit_singleline(&mut p.description);
                        ui.end_row();
                        ui.label("Power plan id");
                        ui.text_edit_singleline(&mut p.power_plan);
                        ui.end_row();
                        list_edit(ui, "End these", &mut p.end_processes);
                        list_edit(ui, "Lower priority of", &mut p.lower_priority);
                        list_edit(ui, "Raise priority of", &mut p.raise_priority);
                        ui.label("Sampling every");
                        ui.add(egui::DragValue::new(&mut p.sample_ms).range(250..=10000).suffix(" ms"));
                        ui.end_row();
                    });
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Save").color(GREEN)).clicked() {
                            save = Some(p.clone());
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            }
            if let Some(p) = save {
                let mut s = shared.lock().unwrap();
                if let Some(x) = s.config.sys_profiles.iter_mut().find(|x| x.id == p.id) {
                    *x = p;
                } else {
                    s.config.sys_profiles.push(p);
                }
                s.config_dirty = true;
                app.auto.editing_sys = None;
            }
            if cancel {
                app.auto.editing_sys = None;
            }
            let list = shared.lock().unwrap().config.sys_profiles.clone();
            let mut apply = None;
            let mut remove = None;
            for p in &list {
                card(ui, &p.name, |ui| {
                    ui.label(RichText::new(&p.description).color(c_dim()));
                    let mut parts = vec![];
                    if !p.power_plan.is_empty() {
                        parts.push(format!("power plan {}", &p.power_plan[..8.min(p.power_plan.len())]));
                    }
                    if !p.end_processes.is_empty() {
                        parts.push(format!("ends {}", p.end_processes.join(", ")));
                    }
                    if !p.lower_priority.is_empty() {
                        parts.push(format!("lowers {}", p.lower_priority.join(", ")));
                    }
                    if !p.raise_priority.is_empty() {
                        parts.push(format!("raises {}", p.raise_priority.join(", ")));
                    }
                    parts.push(format!("samples every {} ms", p.sample_ms));
                    ui.label(RichText::new(parts.join(" • ")).size(12.0));
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("▶ Apply now").color(GREEN)).clicked() {
                            apply = Some(p.clone());
                        }
                        if ui.button("✏").clicked() {
                            app.auto.editing_sys = Some(p.clone());
                        }
                        if ui.button("🗑").clicked() {
                            remove = Some(p.id);
                        }
                    });
                });
                ui.add_space(4.0);
            }
            if let Some(p) = apply {
                apply_sys_profile(app, &p);
            }
            if let Some(id) = remove {
                let mut s = shared.lock().unwrap();
                s.config.sys_profiles.retain(|x| x.id != id);
                s.config_dirty = true;
            }
            // ---------------- process profiles
            let ui = &mut cols[1];
            ui.label(RichText::new("Process-specific profiles").size(16.0).strong());
            if ui.button("✚ New process profile").clicked() {
                let id = shared.lock().unwrap().config.new_id();
                app.auto.editing_proc = Some(ProcProfile { id, pattern: "example.exe".into(), enabled: true, priority: Some(Priority::BelowNormal), affinity: None, efficiency: None, applied: 0 });
            }
            let mut save = None;
            let mut cancel = false;
            let ncpu = shared.lock().unwrap().ncpu.min(64);
            if let Some(p) = app.auto.editing_proc.as_mut() {
                card(ui, "Edit process profile", |ui| {
                    egui::Grid::new("procp").num_columns(2).show(ui, |ui| {
                        ui.label("Process name");
                        ui.text_edit_singleline(&mut p.pattern);
                        ui.end_row();
                        ui.label("Priority");
                        ui.horizontal(|ui| {
                            let mut on = p.priority.is_some();
                            if ui.checkbox(&mut on, "").changed() {
                                p.priority = if on { Some(Priority::BelowNormal) } else { None };
                            }
                            if let Some(pr) = p.priority.as_mut() {
                                egui::ComboBox::from_id_salt("pp_pri").selected_text(pr.label()).show_ui(ui, |ui| {
                                    for x in Priority::ALL {
                                        ui.selectable_value(pr, x, x.label());
                                    }
                                });
                            }
                        });
                        ui.end_row();
                        ui.label("Efficiency mode");
                        ui.horizontal(|ui| {
                            let mut on = p.efficiency.is_some();
                            if ui.checkbox(&mut on, "").changed() {
                                p.efficiency = if on { Some(true) } else { None };
                            }
                            if let Some(e) = p.efficiency.as_mut() {
                                ui.checkbox(e, "turn on");
                            }
                        });
                        ui.end_row();
                        ui.label("CPU affinity");
                        ui.vertical(|ui| {
                            let mut on = p.affinity.is_some();
                            if ui.checkbox(&mut on, "limit to these processors").changed() {
                                p.affinity = if on { Some((1u64 << ncpu.min(63)) - 1) } else { None };
                            }
                            if let Some(m) = p.affinity.as_mut() {
                                ui.horizontal_wrapped(|ui| {
                                    for i in 0..ncpu {
                                        let mut b = *m & (1 << i) != 0;
                                        if ui.checkbox(&mut b, i.to_string()).changed() {
                                            if b {
                                                *m |= 1 << i;
                                            } else {
                                                *m &= !(1 << i);
                                            }
                                        }
                                    }
                                });
                            }
                        });
                        ui.end_row();
                        ui.label("Enabled");
                        ui.checkbox(&mut p.enabled, "");
                        ui.end_row();
                    });
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("Save").color(GREEN)).clicked() {
                            save = Some(p.clone());
                        }
                        if ui.button("Cancel").clicked() {
                            cancel = true;
                        }
                    });
                });
            }
            if let Some(p) = save {
                let mut s = shared.lock().unwrap();
                if let Some(x) = s.config.proc_profiles.iter_mut().find(|x| x.id == p.id) {
                    *x = p;
                } else {
                    s.config.proc_profiles.push(p);
                }
                s.config_dirty = true;
                app.auto.editing_proc = None;
            }
            if cancel {
                app.auto.editing_proc = None;
            }
            let list = shared.lock().unwrap().config.proc_profiles.clone();
            if list.is_empty() {
                notice(ui, "No process profiles yet. Example: make a game always start with High priority, or a backup tool always in efficiency mode.", BLUE);
            }
            let mut remove = None;
            for p in &list {
                card(ui, "", |ui| {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&p.pattern).strong());
                        pill(ui, if p.enabled { "On" } else { "Off" }, if p.enabled { GREEN } else { c_dim() });
                        pill(ui, &format!("applied {}x", p.applied), BLUE);
                        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                            if ui.button("🗑").clicked() {
                                remove = Some(p.id);
                            }
                            if ui.button("✏").clicked() {
                                app.auto.editing_proc = Some(p.clone());
                            }
                        });
                    });
                    let mut parts = vec![];
                    if let Some(pr) = p.priority {
                        parts.push(format!("priority {}", pr.label()));
                    }
                    if let Some(e) = p.efficiency {
                        parts.push(format!("efficiency {}", if e { "on" } else { "off" }));
                    }
                    if let Some(m) = p.affinity {
                        parts.push(format!("{} processors", m.count_ones()));
                    }
                    ui.label(RichText::new(parts.join(" • ")).color(c_dim()));
                });
                ui.add_space(4.0);
            }
            if let Some(id) = remove {
                let mut s = shared.lock().unwrap();
                s.config.proc_profiles.retain(|x| x.id != id);
                s.config_dirty = true;
            }
        });
    });
}

pub fn schedules(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Scheduled monitoring", "Record performance, take configuration snapshots and write health reports automatically.");
    let shared = app.shared.clone();
    if ui.button("✚ New schedule").clicked() {
        let id = shared.lock().unwrap().config.new_id();
        app.auto.editing_sched = Some(Schedule { id, name: "Hourly check".into(), enabled: true, kind: ScheduleKind::EveryMinutes(60), record_secs: 120, take_snapshot: false, write_report: true, last_run: 0 });
    }
    notice(ui, "Schedules run while this app is open (it can sit minimized).", BLUE);
    let mut save = None;
    let mut cancel = false;
    if let Some(sc) = app.auto.editing_sched.as_mut() {
        card(ui, "Edit schedule", |ui| {
            egui::Grid::new("sched").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut sc.name);
                ui.end_row();
                ui.label("When");
                ui.horizontal(|ui| {
                    let every = matches!(sc.kind, ScheduleKind::EveryMinutes(_));
                    if ui.radio(every, "Every").clicked() && !every {
                        sc.kind = ScheduleKind::EveryMinutes(60);
                    }
                    if let ScheduleKind::EveryMinutes(m) = &mut sc.kind {
                        ui.add(egui::DragValue::new(m).range(1..=10080).suffix(" min"));
                    }
                    if ui.radio(!every, "Daily at").clicked() && every {
                        sc.kind = ScheduleKind::DailyAt(9, 0);
                    }
                    if let ScheduleKind::DailyAt(h, m) = &mut sc.kind {
                        ui.add(egui::DragValue::new(h).range(0..=23));
                        ui.label(":");
                        ui.add(egui::DragValue::new(m).range(0..=59));
                    }
                });
                ui.end_row();
                ui.label("Record performance for");
                ui.add(egui::DragValue::new(&mut sc.record_secs).range(0..=86400).suffix(" s (0 = no)"));
                ui.end_row();
                ui.label("Also");
                ui.horizontal(|ui| {
                    ui.checkbox(&mut sc.take_snapshot, "take a configuration snapshot");
                    ui.checkbox(&mut sc.write_report, "write a health report");
                });
                ui.end_row();
                ui.label("Enabled");
                ui.checkbox(&mut sc.enabled, "");
                ui.end_row();
            });
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Save").color(GREEN)).clicked() {
                    save = Some(sc.clone());
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    }
    if let Some(sc) = save {
        let mut s = shared.lock().unwrap();
        if let Some(x) = s.config.schedules.iter_mut().find(|x| x.id == sc.id) {
            *x = sc;
        } else {
            s.config.schedules.push(sc);
        }
        s.config_dirty = true;
        app.auto.editing_sched = None;
    }
    if cancel {
        app.auto.editing_sched = None;
    }
    let list = shared.lock().unwrap().config.schedules.clone();
    if list.is_empty() {
        ui.label(RichText::new("No schedules yet.").color(c_dim()));
    }
    let mut remove = None;
    let mut run_now = None;
    for sc in &list {
        card(ui, "", |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&sc.name).strong());
                pill(ui, if sc.enabled { "On" } else { "Off" }, if sc.enabled { GREEN } else { c_dim() });
                let when = match sc.kind {
                    ScheduleKind::EveryMinutes(m) => format!("every {m} min"),
                    ScheduleKind::DailyAt(h, m) => format!("daily at {h:02}:{m:02}"),
                };
                ui.label(when);
                ui.label(RichText::new(if sc.last_run > 0 { format!("last ran {}", util::fmt_time(sc.last_run)) } else { "never ran".into() }).color(c_dim()));
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if ui.button("🗑").clicked() {
                        remove = Some(sc.id);
                    }
                    if ui.button("✏").clicked() {
                        app.auto.editing_sched = Some(sc.clone());
                    }
                    if ui.button("▶ Run now").clicked() {
                        run_now = Some(sc.id);
                    }
                });
            });
        });
        ui.add_space(4.0);
    }
    let mut s = shared.lock().unwrap();
    if let Some(id) = remove {
        s.config.schedules.retain(|x| x.id != id);
        s.config_dirty = true;
    }
    if let Some(id) = run_now {
        if let Some(x) = s.config.schedules.iter_mut().find(|x| x.id == id) {
            x.last_run = 0;
            if let ScheduleKind::DailyAt(..) = x.kind {
                // run immediately by queuing its parts
                let x = x.clone();
                if x.record_secs > 0 {
                    let _ = s.start_recording(Some(x.record_secs));
                }
                if x.take_snapshot {
                    s.snapshot_requests += 1;
                }
                if x.write_report {
                    s.report_requests += 1;
                }
            }
        }
        s.config_dirty = true;
    }
}

pub fn scripts(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Script integrations", "Save PowerShell or cmd scripts, run them here, or let rules run them. Rules pass DTM_RULE, DTM_PID, DTM_NAME and DTM_VALUE as environment variables.");
    let shared = app.shared.clone();
    ui.horizontal(|ui| {
        if ui.button("✚ New script").clicked() {
            let id = shared.lock().unwrap().config.new_id();
            app.auto.editing_script = Some(Script { id, name: "My script".into(), shell: "PowerShell".into(), body: "Get-Date".into() });
        }
        if ui.button("📤 Export live data as JSON for other tools").clicked() {
            let path = export_json(app);
            app.result(path);
        }
    });
    let mut save = None;
    let mut cancel = false;
    if let Some(sc) = app.auto.editing_script.as_mut() {
        card(ui, "Edit script", |ui| {
            ui.horizontal(|ui| {
                ui.label("Name");
                ui.text_edit_singleline(&mut sc.name);
                ui.label("Shell");
                ui.selectable_value(&mut sc.shell, "PowerShell".to_string(), "PowerShell");
                ui.selectable_value(&mut sc.shell, "cmd".to_string(), "cmd");
            });
            ui.add(egui::TextEdit::multiline(&mut sc.body).code_editor().desired_rows(6).desired_width(f32::INFINITY));
            ui.horizontal(|ui| {
                if ui.button(RichText::new("Save").color(GREEN)).clicked() {
                    save = Some(sc.clone());
                }
                if ui.button("Cancel").clicked() {
                    cancel = true;
                }
            });
        });
    }
    if let Some(sc) = save {
        let mut s = shared.lock().unwrap();
        if let Some(x) = s.config.scripts.iter_mut().find(|x| x.id == sc.id) {
            *x = sc;
        } else {
            s.config.scripts.push(sc);
        }
        s.config_dirty = true;
        app.auto.editing_script = None;
    }
    if cancel {
        app.auto.editing_script = None;
    }
    let list = shared.lock().unwrap().config.scripts.clone();
    let mut remove = None;
    ui.columns(2, |cols| {
        for sc in &list {
            card(&mut cols[0], "", |ui| {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&sc.name).strong());
                    pill(ui, &sc.shell, BLUE);
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button("🗑").clicked() {
                            remove = Some(sc.id);
                        }
                        if ui.button("✏").clicked() {
                            app.auto.editing_script = Some(sc.clone());
                        }
                        if ui.button(RichText::new("▶ Run").color(GREEN)).clicked() {
                            let out = app.auto.script_output.clone();
                            let (shell, body, name) = (sc.shell.clone(), sc.body.clone(), sc.name.clone());
                            let ctx = ui.ctx().clone();
                            std::thread::spawn(move || {
                                let child = collector::spawn_script(&shell, &body, &[("DTM_RULE", "manual")]);
                                let o = child.wait_with_output();
                                let text = match o {
                                    Ok(o) => format!("{}{}", String::from_utf8_lossy(&o.stdout), String::from_utf8_lossy(&o.stderr)),
                                    Err(e) => e.to_string(),
                                };
                                let mut g = out.lock().unwrap();
                                g.insert(0, (format!("{} — {}", name, util::fmt_time(util::now_unix())), text));
                                g.truncate(20);
                                ctx.request_repaint();
                            });
                        }
                    });
                });
                ui.label(RichText::new(sc.body.lines().take(3).collect::<Vec<_>>().join("\n")).monospace().size(11.5).color(c_dim()));
            });
            cols[0].add_space(4.0);
        }
        card(&mut cols[1], "Output", |ui| {
            let out = app.auto.script_output.lock().unwrap();
            if out.is_empty() {
                ui.label(RichText::new("Run a script to see its output here.").color(c_dim()));
            }
            egui::ScrollArea::vertical().id_salt("sout").max_height(600.0).show(ui, |ui| {
                for (h, t) in out.iter() {
                    ui.label(RichText::new(h).strong());
                    ui.label(RichText::new(t).monospace().size(11.5));
                    ui.separator();
                }
            });
        });
    });
    if let Some(id) = remove {
        let mut s = shared.lock().unwrap();
        s.config.scripts.retain(|x| x.id != id);
        s.config_dirty = true;
    }
}

fn export_json(app: &App) -> Result<String, String> {
    let s = app.shared.lock().unwrap();
    let procs: Vec<serde_json::Value> = s
        .procs
        .values()
        .map(|p| {
            serde_json::json!({
                "pid": p.pid, "ppid": p.ppid, "name": p.name, "cpu": p.cpu, "private_bytes": p.private_bytes, "working_set": p.ws,
                "threads": p.threads, "handles": p.handles, "io_read_bps": p.io_read_rate, "io_write_bps": p.io_write_rate,
                "gpu": p.gpu, "path": p.st.path, "user": p.st.user, "connections": p.connections
            })
        })
        .collect();
    let doc = serde_json::json!({
        "time": util::fmt_time(util::now_unix()),
        "cpu": s.cpu_total, "memory_used_pct": s.mem_used_pct(), "disk_active": s.disk_active_max(),
        "net_rx_bps": s.net_rx(), "net_tx_bps": s.net_tx(), "gpu": s.gpu.total,
        "processes": procs,
    });
    let path = util::sub_dir("exports").join(format!("live-{}.json", util::stamp()));
    std::fs::write(&path, serde_json::to_string_pretty(&doc).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
    Ok(format!("Saved {}", path.display()))
}

