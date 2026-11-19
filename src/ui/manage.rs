//! System management pages: system information, services, startup, scheduled tasks,
//! drivers, devices, power, network adapters, Windows settings, snapshots and changes.

use super::widgets::*;
use super::{App, Confirm};
use crate::queries::{self, PowerInfo, Rows};
use crate::util::{self, jf64, jget, json_str, jstr, Job};
use egui::{RichText, Ui};
use serde_json::Value;

#[derive(Default)]
pub struct ManageUi {
    pub sysinfo: Job<Value>,
    pub services: Job<Rows>,
    pub services_t: TableState,
    pub svc_deps: Job<String>,
    pub startup: Job<Rows>,
    pub startup_t: TableState,
    pub tasks: Job<Rows>,
    pub tasks_t: TableState,
    pub task_info: Job<Value>,
    pub hide_ms_tasks: bool,
    pub drivers: Job<Rows>,
    pub drivers_t: TableState,
    pub signed: Job<Rows>,
    pub signed_t: TableState,
    pub kmods: Job<Vec<crate::sys::KernelModule>>,
    pub driver_tab: u8,
    pub devices: Job<Rows>,
    pub devices_t: TableState,
    pub only_problems: bool,
    pub power: Job<PowerInfo>,
    pub adapters: Job<Value>,
    pub adapters_t: TableState,
    pub addr_t: TableState,
    pub winconf: Job<Value>,
    pub env_t: TableState,
    pub action: Job<String>,
    pub snapshot_job: Job<std::path::PathBuf>,
    pub snap_a: Option<std::path::PathBuf>,
    pub snap_b: Option<std::path::PathBuf>,
    pub diff_filter: String,
}

fn refresh_button<T: Send + 'static>(ui: &mut Ui, job: &mut Job<T>) -> bool {
    let r = ui.add_enabled(!job.is_running(), egui::Button::new("⟳ Refresh")).clicked();
    if r {
        job.started = false;
    }
    r
}

fn poll_action(app: &mut App) {
    if !app.manage.action.started || app.manage.action.is_running() {
        return;
    }
    let r = app.manage.action.with(|r| r.cloned());
    if let Some(r) = r {
        app.manage.action = Job::default();
        app.result(r);
    }
}

pub fn run_service_action(app: &mut App, ctx: &egui::Context, name: String, action: &'static str) {
    let n2 = name.clone();
    app.manage.action = Job::default();
    app.manage.action.start(ctx, move || queries::service_action(&n2, action).map(|m| format!("{n2}: {m}")));
    app.manage.services.started = false;
}

// ------------------------------------------------------------------ system info

pub fn system_info(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    page_header(ui, "System information", "Hardware and Windows details, read live from the system.");
    app.manage.sysinfo.ensure(ctx, queries::system_info);
    {
        let s = app.shared.lock().unwrap();
        card(ui, "At a glance", |ui| {
            kv(
                ui,
                "glance",
                &[
                    ("Processor", s.cpu_name.clone()),
                    ("Logical processors", s.ncpu.to_string()),
                    ("Memory", util::fmt_bytes(s.mem.st.total as f64)),
                    ("Up time", util::fmt_dur(s.sys.uptime)),
                    ("Windows", crate::sys::reg_string("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion", "ProductName").unwrap_or_default()),
                    (
                        "Build",
                        format!(
                            "{} ({})",
                            crate::sys::reg_string("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion", "CurrentBuild").unwrap_or_default(),
                            crate::sys::reg_string("SOFTWARE\\Microsoft\\Windows NT\\CurrentVersion", "DisplayVersion").unwrap_or_default()
                        ),
                    ),
                    ("Computer name", std::env::var("COMPUTERNAME").unwrap_or_default()),
                    ("Running as administrator", if s.elevated { "Yes".into() } else { "No".into() }),
                ],
            );
        });
    }
    ui.add_space(6.0);
    refresh_button(ui, &mut app.manage.sysinfo);
    if !job_status(ui, &app.manage.sysinfo) {
        return;
    }
    let v = app.manage.sysinfo.take_ok().unwrap_or_default();
    egui::ScrollArea::vertical().id_salt("sysinfo").auto_shrink([false, false]).show(ui, |ui| {
        ui.columns(3, |cols| {
            for (i, key) in ["Computer", "OS", "Bios"].iter().enumerate() {
                let rows = value_rows(jget(&v, key));
                let title = match *key {
                    "OS" => "Operating system",
                    "Bios" => "Firmware and motherboard",
                    _ => "Computer",
                };
                card(&mut cols[i], title, |ui| {
                    let r: Vec<(&str, String)> = rows.iter().map(|(k, v)| (k.as_str(), if k == "TotalMemory" { util::fmt_bytes(v.parse().unwrap_or(0.0)) } else { v.clone() })).collect();
                    kv(ui, key, &r);
                });
            }
        });
        for (key, title) in [("Cpu", "Processor"), ("Memory", "Memory modules"), ("Gpu", "Graphics"), ("Disks", "Disk drives"), ("Monitors", "Monitors"), ("Sound", "Sound devices")] {
            let list = value_list(jget(&v, key));
            if list.is_empty() {
                continue;
            }
            ui.add_space(6.0);
            card(ui, title, |ui| {
                for (i, item) in list.iter().enumerate() {
                    let rows = value_rows(item);
                    let r: Vec<(&str, String)> = rows
                        .iter()
                        .map(|(k, val)| {
                            let pretty = match k.as_str() {
                                "Capacity" | "Size" | "AdapterRAM" => util::fmt_bytes(val.parse().unwrap_or(0.0)),
                                "L2CacheSize" | "L3CacheSize" => util::fmt_bytes(val.parse::<f64>().unwrap_or(0.0) * 1024.0),
                                "MaxClockSpeed" | "CurrentClockSpeed" | "Speed" | "ConfiguredClockSpeed" => format!("{val} MHz"),
                                _ => val.clone(),
                            };
                            (k.as_str(), pretty)
                        })
                        .collect();
                    kv(ui, &format!("{key}{i}"), &r);
                    if i + 1 < list.len() {
                        ui.separator();
                    }
                }
            });
        }
    });
}

// ------------------------------------------------------------------ services

pub fn services(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    poll_action(app);
    page_header(ui, "Services", "Background programs Windows runs. Start, stop and change how they start.");
    app.manage.services.ensure(ctx, queries::services);
    ui.horizontal(|ui| {
        search_field(ui, &mut app.manage.services_t.filter, "🔍 Filter services…", 300.0);
        refresh_button(ui, &mut app.manage.services);
        if app.manage.action.is_running() {
            ui.spinner();
        }
    });
    app.admin_hint(ui, "Starting, stopping and changing services needs the Admin view.");
    if !job_status(ui, &app.manage.services) {
        return;
    }
    let rows = app.manage.services.take_ok().unwrap_or_default();
    let running = rows.iter().filter(|r| jstr(r, "State") == "Running").count();
    let auto_stopped = rows.iter().filter(|r| jstr(r, "StartMode") == "Auto" && jstr(r, "State") != "Running").count();
    ui.horizontal(|ui| {
        pill(ui, &format!("{} services", rows.len()), BLUE);
        pill(ui, &format!("{running} running"), GREEN);
        pill(ui, &format!("{auto_stopped} automatic but stopped"), if auto_stopped > 0 { YELLOW } else { c_dim() });
    });
    let sel = app.manage.services_t.selected.and_then(|i| rows.get(i)).cloned();
    if let Some(r) = &sel {
        let name = jstr(r, "Name");
        card(ui, &jstr(r, "DisplayName"), |ui| {
            ui.columns(2, |cols| {
                kv(
                    &mut cols[0],
                    "svc_kv",
                    &[
                        ("Service name", name.clone()),
                        ("State", jstr(r, "State")),
                        ("Start type", format!("{}{}", jstr(r, "StartMode"), if jget(r, "DelayedAutoStart").as_bool() == Some(true) { " (delayed)" } else { "" })),
                        ("Runs as", jstr(r, "StartName")),
                        ("Process ID", jstr(r, "ProcessId")),
                        ("Type", jstr(r, "ServiceType")),
                        ("Last exit code", jstr(r, "ExitCode")),
                    ],
                );
                kv(&mut cols[1], "svc_kv2", &[("Program", jstr(r, "PathName")), ("Description", jstr(r, "Description"))]);
                app.manage.svc_deps.with(|d| {
                    if let Some(Ok(t)) = d {
                        cols[1].label(RichText::new(t.trim()).color(c_dim()).size(12.0));
                    }
                });
            });
            ui.horizontal(|ui| {
                let admin = app.admin();
                for (label, act) in [("▶ Start", "start"), ("■ Stop", "stop"), ("⟳ Restart", "restart")] {
                    if ui.add_enabled(admin, egui::Button::new(label)).clicked() {
                        if act == "stop" {
                            app.confirm = Some(Confirm::ServiceStop(name.clone()));
                        } else {
                            run_service_action(app, ctx, name.clone(), act);
                        }
                    }
                }
                ui.separator();
                ui.label("Start type:");
                for (label, act) in [("Automatic", "auto"), ("Manual", "manual"), ("Disabled", "disabled")] {
                    if ui.add_enabled(admin, egui::Button::new(label)).clicked() {
                        run_service_action(app, ctx, name.clone(), act);
                    }
                }
                ui.separator();
                if ui.button("Show dependencies").clicked() {
                    let n = name.clone();
                    app.manage.svc_deps = Job::default();
                    app.manage.svc_deps.start(ctx, move || queries::dependent_services(&n));
                }
                let pid = jf64(r, "ProcessId") as u32;
                if pid > 0 && ui.button("Show process").clicked() {
                    app.procs.selected = Some(pid);
                    app.page = super::Page::Processes;
                }
            });
        });
    }
    let cols = [
        col("DisplayName", "Name", 260.0),
        col("Name", "Service", 140.0),
        col("State", "State", 80.0),
        col("StartMode", "Start type", 80.0),
        col("StartName", "Runs as", 160.0),
        col("ProcessId", "PID", 60.0),
        col("Description", "Description", 300.0),
    ];
    json_table(
        ui,
        "services",
        &rows,
        &cols,
        &mut app.manage.services_t,
        &|r| {
            if jstr(r, "State") == "Running" {
                Some(GREEN)
            } else if jstr(r, "StartMode") == "Auto" {
                Some(YELLOW)
            } else {
                None
            }
        },
        f32::INFINITY,
    );
}

// ------------------------------------------------------------------ startup

pub fn startup(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    poll_action(app);
    page_header(ui, "Startup apps", "Everything that starts with Windows: Run keys, startup folders, logon tasks and third-party auto-start services.");
    app.manage.startup.ensure(ctx, queries::startup);
    ui.horizontal(|ui| {
        search_field(ui, &mut app.manage.startup_t.filter, "🔍 Filter…", 300.0);
        refresh_button(ui, &mut app.manage.startup);
        if app.manage.action.is_running() {
            ui.spinner();
        }
    });
    app.admin_hint(ui, "Entries for all users (HKLM) can only be switched in the Admin view.");
    if !job_status(ui, &app.manage.startup) {
        return;
    }
    let rows = app.manage.startup.take_ok().unwrap_or_default();
    let enabled = rows.iter().filter(|r| jstr(r, "State") != "Disabled").count();
    ui.horizontal(|ui| {
        pill(ui, &format!("{} entries", rows.len()), BLUE);
        pill(ui, &format!("{enabled} enabled"), GREEN);
        if ui.link("See startup analysis ➡").clicked() {
            app.page = super::Page::StartupAnalysis;
        }
    });
    if let Some(r) = app.manage.startup_t.selected.and_then(|i| rows.get(i)).cloned() {
        card(ui, &jstr(&r, "Name"), |ui| {
            kv(
                ui,
                "st_kv",
                &[("Command", jstr(&r, "Command")), ("Where", jstr(&r, "Location")), ("Kind", jstr(&r, "Kind")), ("State", jstr(&r, "State")), ("Publisher", jstr(&r, "Publisher")), ("File size", util::fmt_bytes(jf64(&r, "FileSize")))],
            );
            ui.horizontal(|ui| {
                let key = jstr(&r, "Approved");
                let hklm = key.starts_with("HKLM");
                let can = !key.is_empty() && (!hklm || app.admin());
                let name = jstr(&r, "Name");
                let disabled = jstr(&r, "State") == "Disabled";
                if ui.add_enabled(can, egui::Button::new(if disabled { "Enable" } else { "Disable" })).clicked() {
                    let (k, n) = (key.clone(), name.clone());
                    app.manage.action = Job::default();
                    app.manage.action.start(ctx, move || queries::startup_toggle(&k, &n, disabled).map(|m| format!("{n}: {m}")));
                    app.manage.startup.started = false;
                }
                if key.is_empty() {
                    ui.label(RichText::new("Manage this one from its own page (Services or Scheduled tasks).").color(c_dim()));
                }
                let exe = jstr(&r, "Executable");
                if !exe.is_empty() && ui.button("📂 Open file location").clicked() {
                    crate::sys::open_file_location(&exe);
                }
            });
        });
    }
    let cols = [
        col("Name", "Name", 200.0),
        col("State", "State", 80.0),
        col("Kind", "Kind", 120.0),
        col("Publisher", "Publisher", 160.0),
        col("Location", "Where", 260.0),
        col("Command", "Command", 400.0),
    ];
    json_table(ui, "startup", &rows, &cols, &mut app.manage.startup_t, &|r| if jstr(r, "State") == "Disabled" { Some(c_dim()) } else { None }, f32::INFINITY);
}

// ------------------------------------------------------------------ tasks

pub fn tasks(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    poll_action(app);
    page_header(ui, "Scheduled tasks", "Programs Windows runs on a timer, at logon, at startup or on events.");
    app.manage.tasks.ensure(ctx, queries::tasks);
    ui.horizontal(|ui| {
        search_field(ui, &mut app.manage.tasks_t.filter, "🔍 Filter tasks…", 300.0);
        ui.checkbox(&mut app.manage.hide_ms_tasks, "Hide Microsoft tasks");
        refresh_button(ui, &mut app.manage.tasks);
    });
    app.admin_hint(ui, "Enabling or disabling most tasks needs the Admin view.");
    if !job_status(ui, &app.manage.tasks) {
        return;
    }
    let mut rows = app.manage.tasks.take_ok().unwrap_or_default();
    if app.manage.hide_ms_tasks {
        rows.retain(|r| !jstr(r, "TaskPath").starts_with("\\Microsoft\\"));
    }
    if let Some(r) = app.manage.tasks_t.selected.and_then(|i| rows.get(i)).cloned() {
        let (path, name) = (jstr(&r, "TaskPath"), jstr(&r, "TaskName"));
        card(ui, &format!("{path}{name}"), |ui| {
            ui.columns(2, |cols| {
                kv(
                    &mut cols[0],
                    "task_kv",
                    &[("State", jstr(&r, "State")), ("Author", jstr(&r, "Author")), ("Runs as", jstr(&r, "RunAs")), ("Run level", jstr(&r, "RunLevel")), ("Triggers", jstr(&r, "Triggers")), ("Actions", jstr(&r, "Actions"))],
                );
                cols[1].label(RichText::new(jstr(&r, "Description")).color(c_dim()));
                app.manage.task_info.with(|i| {
                    if let Some(Ok(v)) = i {
                        let rr = value_rows(v);
                        let r2: Vec<(&str, String)> = rr.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
                        kv(&mut cols[1], "task_info", &r2);
                    }
                });
            });
            ui.horizontal(|ui| {
                if ui.button("Last/next run").clicked() {
                    let (p, n) = (path.clone(), name.clone());
                    app.manage.task_info = Job::default();
                    app.manage.task_info.start(ctx, move || queries::task_info(&p, &n));
                }
                for (label, act) in [("▶ Run now", "run"), ("■ End", "end"), ("Enable", "enable"), ("Disable", "disable")] {
                    let need_admin = act == "enable" || act == "disable";
                    if ui.add_enabled(!need_admin || app.admin(), egui::Button::new(label)).clicked() {
                        let (p, n) = (path.clone(), name.clone());
                        app.manage.action = Job::default();
                        app.manage.action.start(ctx, move || queries::task_action(&p, &n, act).map(|m| format!("{n}: {m}")));
                        app.manage.tasks.started = false;
                    }
                }
            });
        });
    }
    let cols = [
        col("TaskName", "Name", 240.0),
        col("TaskPath", "Folder", 220.0),
        col("State", "State", 80.0),
        col("Triggers", "Triggers", 150.0),
        col("RunAs", "Runs as", 120.0),
        col("Actions", "Runs", 420.0),
    ];
    json_table(ui, "tasks", &rows, &cols, &mut app.manage.tasks_t, &|r| if jstr(r, "State") == "Disabled" { Some(c_dim()) } else { None }, f32::INFINITY);
}

// ------------------------------------------------------------------ drivers

pub fn drivers(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    page_header(ui, "Drivers", "Kernel drivers, signed device drivers and the modules loaded into the kernel right now.");
    ui.horizontal(|ui| {
        ui.selectable_value(&mut app.manage.driver_tab, 0, "Driver services");
        ui.selectable_value(&mut app.manage.driver_tab, 1, "Device drivers (signatures)");
        ui.selectable_value(&mut app.manage.driver_tab, 2, "Loaded kernel modules");
    });
    match app.manage.driver_tab {
        0 => {
            app.manage.drivers.ensure(ctx, queries::drivers);
            ui.horizontal(|ui| {
                search_field(ui, &mut app.manage.drivers_t.filter, "🔍 Filter…", 300.0);
                refresh_button(ui, &mut app.manage.drivers);
            });
            if job_status(ui, &app.manage.drivers) {
                let rows = app.manage.drivers.take_ok().unwrap_or_default();
                let running = rows.iter().filter(|r| jstr(r, "State") == "Running").count();
                ui.horizontal(|ui| {
                    pill(ui, &format!("{} drivers", rows.len()), BLUE);
                    pill(ui, &format!("{running} running"), GREEN);
                });
                let cols = [col("Name", "Name", 140.0), col("DisplayName", "Display name", 240.0), col("State", "State", 80.0), col("StartMode", "Start", 80.0), col("ServiceType", "Type", 110.0), col("ErrorControl", "On error", 80.0), col("PathName", "File", 360.0)];
                json_table(ui, "drivers", &rows, &cols, &mut app.manage.drivers_t, &|r| if jstr(r, "State") == "Running" { Some(GREEN) } else { None }, f32::INFINITY);
            }
        }
        1 => {
            app.manage.signed.ensure(ctx, queries::signed_drivers);
            ui.horizontal(|ui| {
                search_field(ui, &mut app.manage.signed_t.filter, "🔍 Filter…", 300.0);
                refresh_button(ui, &mut app.manage.signed);
            });
            if job_status(ui, &app.manage.signed) {
                let rows = app.manage.signed.take_ok().unwrap_or_default();
                let unsigned = rows.iter().filter(|r| jget(r, "IsSigned").as_bool() == Some(false)).count();
                ui.horizontal(|ui| {
                    pill(ui, &format!("{} device drivers", rows.len()), BLUE);
                    pill(ui, &format!("{unsigned} unsigned"), if unsigned > 0 { RED } else { GREEN });
                });
                let cols = [
                    col("DeviceName", "Device", 260.0),
                    col("DeviceClass", "Class", 100.0),
                    col("DriverVersion", "Version", 120.0),
                    col("DriverDate", "Date", 90.0),
                    col("DriverProviderName", "Provider", 150.0),
                    col("IsSigned", "Signed", 60.0),
                    col("Signer", "Signer", 220.0),
                    col("InfName", "INF", 100.0),
                ];
                json_table(ui, "signed", &rows, &cols, &mut app.manage.signed_t, &|r| if jget(r, "IsSigned").as_bool() == Some(false) { Some(RED) } else { None }, f32::INFINITY);
            }
        }
        _ => {
            app.manage.kmods.ensure(ctx, || Ok(crate::sys::kernel_modules()));
            refresh_button(ui, &mut app.manage.kmods);
            if !app.admin() {
                notice(ui, "🛡 Kernel addresses are hidden from normal users; the Admin view shows them.", PURPLE);
            }
            if job_status(ui, &app.manage.kmods) {
                let mods = app.manage.kmods.take_ok().unwrap_or_default();
                ui.label(RichText::new(format!("{} modules loaded in the kernel", mods.len())).color(c_dim()));
                let admin = app.admin();
                let mut tb = egui_extras::TableBuilder::new(ui).id_salt("kmods").striped(true).resizable(true).auto_shrink([false, false]);
                tb = tb.column(egui_extras::Column::initial(180.0)).column(egui_extras::Column::initial(160.0)).column(egui_extras::Column::remainder());
                tb.header(20.0, |mut h| {
                    for t in ["Name", "Base address", "Path"] {
                        h.col(|ui| {
                            ui.add(egui::Label::new(RichText::new(t).strong()).truncate());
                        });
                    }
                })
                .body(|body| {
                    body.rows(row_h(), mods.len(), |mut row| {
                        let m = &mods[row.index()];
                        row.col(|ui| {
                            ui.label(&m.name);
                        });
                        row.col(|ui| {
                            ui.monospace(if admin && m.base != 0 { format!("0x{:x}", m.base) } else { "hidden".into() });
                        });
                        row.col(|ui| {
                            ui.label(&m.path);
                        });
                    });
                });
            }
        }
    }
}

// ------------------------------------------------------------------ devices

pub fn devices(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    page_header(ui, "Device inventory", "Every device Windows knows about, with its driver service and any problem code.");
    app.manage.devices.ensure(ctx, queries::devices);
    ui.horizontal(|ui| {
        search_field(ui, &mut app.manage.devices_t.filter, "🔍 Filter devices…", 300.0);
        ui.checkbox(&mut app.manage.only_problems, "Only devices with problems");
        refresh_button(ui, &mut app.manage.devices);
    });
    if !job_status(ui, &app.manage.devices) {
        return;
    }
    let mut rows = app.manage.devices.take_ok().unwrap_or_default();
    for r in rows.iter_mut() {
        let code = jf64(r, "ConfigManagerErrorCode") as i64;
        if let Value::Object(o) = r {
            o.insert("Problem".into(), Value::String(queries::device_problem(code).into()));
        }
    }
    let problems = rows.iter().filter(|r| jf64(r, "ConfigManagerErrorCode") as i64 != 0).count();
    let mut classes: std::collections::BTreeMap<String, usize> = Default::default();
    for r in &rows {
        *classes.entry(jstr(r, "PNPClass")).or_default() += 1;
    }
    ui.horizontal_wrapped(|ui| {
        pill(ui, &format!("{} devices", rows.len()), BLUE);
        pill(ui, &format!("{problems} with problems"), if problems > 0 { RED } else { GREEN });
        let mut cl: Vec<_> = classes.into_iter().filter(|c| !c.0.is_empty()).collect();
        cl.sort_by(|a, b| b.1.cmp(&a.1));
        for (c, n) in cl.iter().take(12) {
            if ui.small_button(format!("{c} {n}")).clicked() {
                app.manage.devices_t.filter = c.clone();
            }
        }
    });
    if app.manage.only_problems {
        rows.retain(|r| jf64(r, "ConfigManagerErrorCode") as i64 != 0);
    }
    let cols = [
        col("Name", "Device", 300.0),
        col("PNPClass", "Class", 110.0),
        col("Manufacturer", "Manufacturer", 160.0),
        col("Status", "Status", 70.0),
        col("Problem", "Problem", 170.0),
        col("Service", "Driver service", 110.0),
        col("DeviceID", "Device ID", 380.0),
    ];
    json_table(ui, "devices", &rows, &cols, &mut app.manage.devices_t, &|r| if jf64(r, "ConfigManagerErrorCode") as i64 != 0 { Some(RED) } else { None }, f32::INFINITY);
}

// ------------------------------------------------------------------ power

pub fn power(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    poll_action(app);
    page_header(ui, "Power configuration", "Power plans, sleep states, battery and the settings of the active plan.");
    app.manage.power.ensure(ctx, queries::power);
    refresh_button(ui, &mut app.manage.power);
    if !job_status(ui, &app.manage.power) {
        return;
    }
    let mut set_plan = None;
    let mut report = false;
    app.manage.power.with(|r| {
        let Some(Ok(p)) = r else { return };
        egui::ScrollArea::vertical().id_salt("power").auto_shrink([false, false]).show(ui, |ui| {
            ui.columns(2, |cols| {
                card(&mut cols[0], "Power plans", |ui| {
                    for (guid, name, active) in &p.plans {
                        ui.horizontal(|ui| {
                            if *active {
                                pill(ui, "Active", GREEN);
                            } else if ui.button("Use").clicked() {
                                set_plan = Some(guid.clone());
                            }
                            ui.label(RichText::new(name).strong());
                            ui.label(RichText::new(guid).color(c_dim()).size(11.0).monospace());
                        });
                    }
                });
                card(&mut cols[1], "Active plan settings", |ui| {
                    egui::Grid::new("pset").striped(true).show(ui, |ui| {
                        ui.strong("Setting");
                        ui.strong("Plugged in");
                        ui.strong("On battery");
                        ui.end_row();
                        for s in &p.settings {
                            ui.label(jstr(s, "Setting"));
                            ui.label(jstr(s, "PluggedIn"));
                            ui.label(jstr(s, "OnBattery"));
                            ui.end_row();
                        }
                    });
                });
            });
            ui.add_space(6.0);
            ui.columns(2, |cols| {
                card(&mut cols[0], "Battery", |ui| {
                    if p.battery.is_empty() {
                        ui.label(RichText::new("No battery (desktop PC or not reported).").color(c_dim()));
                    }
                    for b in &p.battery {
                        let rows = value_rows(b);
                        let r: Vec<(&str, String)> = rows.iter().map(|(k, v)| (k.as_str(), v.clone())).collect();
                        kv(ui, "batt", &r);
                    }
                    if ui.button("Create detailed battery report").clicked() {
                        report = true;
                    }
                });
                card(&mut cols[1], "Sleep states", |ui| {
                    ui.label(RichText::new(p.sleep_states.trim()).monospace().size(11.5));
                });
            });
        });
    });
    if let Some(g) = set_plan {
        app.manage.action = Job::default();
        app.manage.action.start(ctx, move || queries::set_power_plan(&g));
        app.manage.power.started = false;
    }
    if report {
        match queries::battery_report() {
            Ok(p) => {
                crate::sys::shell_open("open", &p.to_string_lossy(), "");
                app.notify("Battery report created", false);
            }
            Err(e) => app.notify(e, true),
        }
    }
}

// ------------------------------------------------------------------ adapters

pub fn adapters(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    poll_action(app);
    page_header(ui, "Network adapters", "Live traffic per adapter plus drivers, addresses, DNS servers, gateways and network profiles.");
    {
        let s = app.shared.lock().unwrap();
        card(ui, "Live", |ui| {
            egui::Grid::new("live_nets").striped(true).spacing([14.0, 4.0]).show(ui, |ui| {
                for t in ["Adapter", "Type", "Status", "Speed", "Down", "Up", "Errors", "Activity"] {
                    ui.add(egui::Label::new(RichText::new(t).strong()).truncate());
                }
                ui.end_row();
                for n in &s.nets {
                    ui.label(&n.row.alias);
                    ui.label(crate::sys::if_type_name(n.row.if_type));
                    ui.label(RichText::new(if n.row.up { "Up" } else { "Down" }).color(if n.row.up { GREEN } else { c_dim() }));
                    ui.label(if n.row.speed > 0 { util::fmt_bits(n.row.speed as f64 / 8.0) } else { "-".into() });
                    ui.label(util::fmt_rate(n.rx as f64));
                    ui.label(util::fmt_rate(n.tx as f64));
                    ui.label(format!("{}", n.row.in_errors + n.row.out_errors));
                    let d: Vec<f32> = n.hist_rx.data.iter().zip(n.hist_tx.data.iter()).map(|(a, b)| a + b).collect();
                    sparkline(ui, &d[d.len().saturating_sub(60)..], ORANGE, egui::vec2(140.0, 20.0), None);
                    ui.end_row();
                }
            });
        });
    }
    ui.add_space(6.0);
    app.manage.adapters.ensure(ctx, queries::adapters);
    refresh_button(ui, &mut app.manage.adapters);
    app.admin_hint(ui, "Turning adapters on or off needs the Admin view.");
    if !job_status(ui, &app.manage.adapters) {
        return;
    }
    let v = app.manage.adapters.take_ok().unwrap_or_default();
    let ads = value_list(jget(&v, "Adapters"));
    if let Some(r) = app.manage.adapters_t.selected.and_then(|i| ads.get(i)).cloned() {
        let name = jstr(&r, "Name");
        ui.horizontal(|ui| {
            ui.label(RichText::new(&name).strong());
            for (l, en) in [("Enable", true), ("Disable", false)] {
                if ui.add_enabled(app.admin(), egui::Button::new(l)).clicked() {
                    let n = name.clone();
                    app.manage.action = Job::default();
                    app.manage.action.start(ctx, move || queries::adapter_action(&n, en).map(|m| format!("{n}: {m}")));
                    app.manage.adapters.started = false;
                }
            }
        });
    }
    let cols = [
        col("Name", "Name", 150.0),
        col("InterfaceDescription", "Hardware", 240.0),
        col("Status", "Status", 80.0),
        col("LinkSpeed", "Speed", 80.0),
        col("MacAddress", "MAC", 130.0),
        col("MediaType", "Media", 100.0),
        col("DriverVersion", "Driver", 110.0),
        col("DriverProvider", "Provider", 120.0),
        col("MtuSize", "MTU", 60.0),
        col("Virtual", "Virtual", 60.0),
    ];
    json_table(ui, "adapters", &ads, &cols, &mut app.manage.adapters_t, &|r| if jstr(r, "Status") == "Up" { Some(GREEN) } else { None }, 220.0);
    ui.add_space(6.0);
    ui.columns(2, |colsx| {
        card(&mut colsx[0], "IP addresses", |ui| {
            let addrs = value_list(jget(&v, "Addresses"));
            let c = [col("InterfaceAlias", "Adapter", 140.0), col("IPAddress", "Address", 220.0), col("PrefixLength", "Prefix", 50.0), col("Family", "Family", 60.0), col("Origin", "Origin", 80.0)];
            json_table(ui, "addrs", &addrs, &c, &mut app.manage.addr_t, &|_| None, 260.0);
        });
        card(&mut colsx[1], "DNS, gateways and profiles", |ui| {
            for d in value_list(jget(&v, "Dns")) {
                ui.label(format!("DNS {} ({}): {}", jstr(&d, "InterfaceAlias"), jstr(&d, "Family"), jstr(&d, "Servers")));
            }
            ui.separator();
            for d in value_list(jget(&v, "Routes")) {
                ui.label(format!("Gateway {} via {} (metric {})", jstr(&d, "InterfaceAlias"), jstr(&d, "NextHop"), jstr(&d, "RouteMetric")));
            }
            ui.separator();
            for d in value_list(jget(&v, "Profiles")) {
                ui.label(format!("Network '{}' on {}: {} ({})", jstr(&d, "Name"), jstr(&d, "InterfaceAlias"), jstr(&d, "Category"), jstr(&d, "IPv4")));
            }
        });
    });
}

// ------------------------------------------------------------------ windows config

pub fn windows_config(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    page_header(ui, "Windows configuration", "Security, system behaviour, page files, recent updates, enabled features and environment.");
    app.manage.winconf.ensure(ctx, queries::windows_config);
    refresh_button(ui, &mut app.manage.winconf);
    if !job_status(ui, &app.manage.winconf) {
        return;
    }
    let v = app.manage.winconf.take_ok().unwrap_or_default();
    egui::ScrollArea::vertical().id_salt("winconf").auto_shrink([false, false]).show(ui, |ui| {
        ui.columns(2, |cols| {
            card(&mut cols[0], "Security", |ui| {
                let rows = value_rows(jget(&v, "Security"));
                let r: Vec<(&str, String)> = rows.iter().map(|(k, val)| (k.as_str(), explain(k, val))).collect();
                kv(ui, "sec", &r);
                ui.separator();
                for f in value_list(jget(&v, "Firewall")) {
                    ui.label(format!("Firewall {}: {} (inbound {}, outbound {})", jstr(&f, "Name"), if jstr(&f, "Enabled") == "Yes" || jstr(&f, "Enabled") == "1" { "on" } else { "off" }, jstr(&f, "DefaultInboundAction"), jstr(&f, "DefaultOutboundAction")));
                }
                for b in value_list(jget(&v, "BitLocker")) {
                    ui.label(format!("BitLocker {}: {} ({}% encrypted)", jstr(&b, "MountPoint"), jstr(&b, "ProtectionStatus"), jstr(&b, "EncryptionPercentage")));
                }
            });
            card(&mut cols[1], "System behaviour", |ui| {
                let rows = value_rows(jget(&v, "System"));
                let r: Vec<(&str, String)> = rows.iter().map(|(k, val)| (k.as_str(), explain(k, val))).collect();
                kv(ui, "sysb", &r);
            });
        });
        ui.add_space(6.0);
        ui.columns(2, |cols| {
            card(&mut cols[0], "Page files", |ui| {
                for p in value_list(jget(&v, "PageFiles")) {
                    ui.label(format!("{}: {} MB allocated, {} MB in use (peak {} MB)", jstr(&p, "Name"), jstr(&p, "AllocatedBaseSize"), jstr(&p, "CurrentUsage"), jstr(&p, "PeakUsage")));
                }
            });
            card(&mut cols[1], "Latest updates", |ui| {
                for u in value_list(jget(&v, "Updates")) {
                    ui.label(format!("{}  {}  {}", jstr(&u, "HotFixID"), jstr(&u, "Description"), jstr(&u, "InstalledOn")));
                }
            });
        });
        ui.add_space(6.0);
        card(ui, "Enabled optional features", |ui| {
            let f: Vec<String> = value_list(jget(&v, "Features")).iter().map(|x| jstr(x, "FeatureName")).collect();
            if f.is_empty() {
                ui.label(RichText::new("Listing features needs the Admin view.").color(c_dim()));
            } else {
                ui.label(f.join(", "));
            }
        });
        ui.add_space(6.0);
        card(ui, "Environment variables", |ui| {
            let env = value_list(jget(&v, "Environment"));
            search_field(ui, &mut app.manage.env_t.filter, "Filter…", 200.0);
            json_table(ui, "env", &env, &[col("Name", "Name", 200.0), col("Value", "Value", 600.0)], &mut app.manage.env_t, &|_| None, 300.0);
        });
    });
}

fn explain(k: &str, v: &str) -> String {
    match (k, v) {
        ("UacEnabled", "1") => "On".into(),
        ("UacEnabled", "0") => "OFF (not recommended)".into(),
        ("FastStartup", "1") | ("Hibernation", "1") | ("GameMode", "1") | ("LongPaths", "1") | ("DeveloperMode", "1") | ("Transparency", "1") => "On".into(),
        ("FastStartup", "0") | ("Hibernation", "0") | ("GameMode", "0") | ("LongPaths", "0") | ("DeveloperMode", "0") | ("Transparency", "0") => "Off".into(),
        ("HardwareGpuScheduling", "2") => "On".into(),
        ("HardwareGpuScheduling", "1") => "Off".into(),
        ("DarkModeApps", "0") => "Dark".into(),
        ("DarkModeApps", "1") => "Light".into(),
        ("UacPromptLevel", "5") => "Notify for apps (default)".into(),
        ("UacPromptLevel", "0") => "Never notify".into(),
        ("UacPromptLevel", "2") => "Always notify".into(),
        (_, "") => "-".into(),
        _ => v.to_string(),
    }
}

// ------------------------------------------------------------------ snapshots

pub fn take_snapshot(app: &mut App, ctx: &egui::Context) {
    if app.manage.snapshot_job.is_running() {
        return;
    }
    let perf = {
        let s = app.shared.lock().unwrap();
        serde_json::json!({
            "cpu_avg": s.hist.cpu.avg(300),
            "mem_used_pct": s.mem_used_pct(),
            "mem_used": s.mem.st.total.saturating_sub(s.mem.st.avail),
            "processes": s.procs.len(),
            "threads": s.mem.st.threads,
            "handles": s.mem.st.handles,
            "disk_avg": s.hist.disk.avg(300),
            "uptime": s.sys.uptime,
        })
    };
    app.manage.snapshot_job = Job::default();
    app.manage.snapshot_job.start(ctx, move || {
        let data = queries::snapshot()?;
        let doc = serde_json::json!({"perf": perf, "data": data});
        let path = util::sub_dir("snapshots").join(format!("snapshot-{}.json", util::stamp()));
        std::fs::write(&path, serde_json::to_string(&doc).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        Ok(path)
    });
    let mut s = app.shared.lock().unwrap();
    s.log("Snapshot", crate::collector::Severity::Info, "Configuration snapshot started".into(), None);
}

fn list_snapshots() -> Vec<std::path::PathBuf> {
    let mut v: Vec<_> = std::fs::read_dir(util::sub_dir("snapshots"))
        .map(|d| d.filter_map(|e| e.ok()).map(|e| e.path()).filter(|p| p.extension().map(|x| x == "json").unwrap_or(false)).collect())
        .unwrap_or_default();
    v.sort();
    v.reverse();
    v
}

fn load(p: &std::path::Path) -> Option<Value> {
    std::fs::read_to_string(p).ok().and_then(|s| serde_json::from_str(&s).ok())
}

pub struct DiffCat {
    pub name: &'static str,
    pub added: Vec<(String, String)>,
    pub removed: Vec<(String, String)>,
    pub changed: Vec<(String, String, String)>,
}

pub const CATS: [&str; 10] = ["Services", "Drivers", "Startup", "Tasks", "Programs", "Updates", "Adapters", "Devices", "PowerPlan", "OS"];

pub fn diff(a: &Value, b: &Value) -> Vec<DiffCat> {
    let mut out = vec![];
    for cat in CATS {
        let to_map = |v: &Value| -> std::collections::BTreeMap<String, String> {
            let list = value_list(jget(jget(v, "data"), cat));
            list.iter()
                .map(|x| {
                    let n = jstr(x, "Name");
                    let val = if x.get("Value").is_some() { jstr(x, "Value") } else { json_str(x) };
                    (if n.is_empty() { cat.to_string() } else { n }, val)
                })
                .collect()
        };
        let (ma, mb) = (to_map(a), to_map(b));
        let mut d = DiffCat { name: cat, added: vec![], removed: vec![], changed: vec![] };
        for (k, vb) in &mb {
            match ma.get(k) {
                None => d.added.push((k.clone(), vb.clone())),
                Some(va) if va != vb => d.changed.push((k.clone(), va.clone(), vb.clone())),
                _ => {}
            }
        }
        for (k, va) in &ma {
            if !mb.contains_key(k) {
                d.removed.push((k.clone(), va.clone()));
            }
        }
        out.push(d);
    }
    out
}

pub fn snapshots(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    page_header(ui, "Configuration snapshots & change tracking", "Save the state of services, drivers, startup, tasks, programs, updates, adapters and devices — then compare any two.");
    ui.horizontal(|ui| {
        if ui.add_enabled(!app.manage.snapshot_job.is_running(), egui::Button::new("📷 Take snapshot now")).clicked() {
            take_snapshot(app, ctx);
        }
        if app.manage.snapshot_job.is_running() {
            ui.spinner();
            ui.label("Collecting… (takes about 10–30 seconds)");
        }
        if ui.button("📂 Open snapshot folder").clicked() {
            util::open_path(&util::sub_dir("snapshots"));
        }
    });
    if app.manage.snapshot_job.started && !app.manage.snapshot_job.is_running() {
        let r = app.manage.snapshot_job.with(|r| r.cloned());
        app.manage.snapshot_job = Job::default();
        match r {
            Some(Ok(p)) => {
                app.notify(format!("Snapshot saved: {}", p.file_name().unwrap_or_default().to_string_lossy()), false);
                let mut s = app.shared.lock().unwrap();
                s.log("Snapshot", crate::collector::Severity::Info, format!("Snapshot saved {}", p.display()), None);
            }
            Some(Err(e)) => app.notify(e, true),
            None => {}
        }
    }
    let snaps = list_snapshots();
    if snaps.is_empty() {
        notice(ui, "No snapshots yet. Take one now, change something (install an app, toggle a service), take another and compare.", BLUE);
        return;
    }
    if app.manage.snap_a.is_none() && snaps.len() >= 2 {
        app.manage.snap_a = Some(snaps[1].clone());
        app.manage.snap_b = Some(snaps[0].clone());
    }
    ui.horizontal(|ui| {
        let name = |p: &Option<std::path::PathBuf>| p.as_ref().map(|p| p.file_name().unwrap_or_default().to_string_lossy().to_string()).unwrap_or_else(|| "pick…".into());
        ui.label("Compare older");
        egui::ComboBox::from_id_salt("snap_a").selected_text(name(&app.manage.snap_a)).width(240.0).show_ui(ui, |ui| {
            for s in &snaps {
                ui.selectable_value(&mut app.manage.snap_a, Some(s.clone()), s.file_name().unwrap_or_default().to_string_lossy());
            }
        });
        ui.label("with newer");
        egui::ComboBox::from_id_salt("snap_b").selected_text(name(&app.manage.snap_b)).width(240.0).show_ui(ui, |ui| {
            for s in &snaps {
                ui.selectable_value(&mut app.manage.snap_b, Some(s.clone()), s.file_name().unwrap_or_default().to_string_lossy());
            }
        });
        search_field(ui, &mut app.manage.diff_filter, "Filter changes…", 200.0);
    });
    let (Some(pa), Some(pb)) = (app.manage.snap_a.clone(), app.manage.snap_b.clone()) else {
        ui.label(format!("{} snapshot(s) saved. Take another to compare.", snaps.len()));
        return;
    };
    let (Some(a), Some(b)) = (load(&pa), load(&pb)) else {
        notice(ui, "Could not read one of the snapshots.", RED);
        return;
    };
    let d = diff(&a, &b);
    let total: usize = d.iter().map(|c| c.added.len() + c.removed.len() + c.changed.len()).sum();
    ui.horizontal(|ui| {
        pill(ui, &format!("{total} changes"), if total > 0 { YELLOW } else { GREEN });
        ui.label(RichText::new(format!("{}  ➡  {}", jstr(jget(&a, "data"), "Taken"), jstr(jget(&b, "data"), "Taken"))).color(c_dim()));
    });
    let f = app.manage.diff_filter.to_lowercase();
    egui::ScrollArea::vertical().id_salt("diff").auto_shrink([false, false]).show(ui, |ui| {
        for c in &d {
            let n = c.added.len() + c.removed.len() + c.changed.len();
            egui::CollapsingHeader::new(RichText::new(format!("{}  ({n})", c.name)).strong()).default_open(n > 0 && n < 60).id_salt(c.name).show(ui, |ui| {
                for (k, v) in c.added.iter().filter(|x| f.is_empty() || x.0.to_lowercase().contains(&f)) {
                    ui.label(RichText::new(format!("✚ {k}  {v}")).color(GREEN));
                }
                for (k, v) in c.removed.iter().filter(|x| f.is_empty() || x.0.to_lowercase().contains(&f)) {
                    ui.label(RichText::new(format!("- {k}  {v}")).color(RED));
                }
                for (k, va, vb) in c.changed.iter().filter(|x| f.is_empty() || x.0.to_lowercase().contains(&f)) {
                    ui.label(RichText::new(format!("~ {k}: {va}  ➡  {vb}")).color(YELLOW));
                }
            });
        }
    });
}

pub fn change_impact(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    page_header(ui, "Change impact analysis", "What changed between two snapshots, and how the system's load differed when each was taken.");
    let snaps = list_snapshots();
    if snaps.len() < 2 {
        notice(ui, "Needs two snapshots. Take one before and one after a change (an install, an update, a driver change).", BLUE);
        if ui.add_enabled(!app.manage.snapshot_job.is_running(), egui::Button::new("📷 Take snapshot now")).clicked() {
            take_snapshot(app, ctx);
        }
        if app.manage.snapshot_job.is_running() {
            ui.spinner();
        }
        return;
    }
    let pa = app.manage.snap_a.clone().unwrap_or(snaps[1].clone());
    let pb = app.manage.snap_b.clone().unwrap_or(snaps[0].clone());
    let (Some(a), Some(b)) = (load(&pa), load(&pb)) else { return };
    ui.label(RichText::new(format!("Comparing {}  ➡  {} (pick others on the Snapshots page)", jstr(jget(&a, "data"), "Taken"), jstr(jget(&b, "data"), "Taken"))).color(c_dim()));
    let d = diff(&a, &b);
    let (pa_, pb_) = (jget(&a, "perf"), jget(&b, "perf"));
    card(ui, "Load when each snapshot was taken", |ui| {
        egui::Grid::new("impact").striped(true).spacing([30.0, 4.0]).show(ui, |ui| {
            for t in ["Measure", "Before", "After", "Change"] {
                ui.add(egui::Label::new(RichText::new(t).strong()).truncate());
            }
            ui.end_row();
            for (k, label, is_bytes) in [
                ("cpu_avg", "CPU (5 min average) %", false),
                ("mem_used", "Memory in use", true),
                ("processes", "Processes", false),
                ("threads", "Threads", false),
                ("handles", "Handles", false),
                ("disk_avg", "Disk active (5 min avg) %", false),
            ] {
                let (x, y) = (jf64(pa_, k), jf64(pb_, k));
                let f = |v: f64| if is_bytes { util::fmt_bytes(v) } else { format!("{v:.1}") };
                ui.label(label);
                ui.label(f(x));
                ui.label(f(y));
                let dlt = y - x;
                let c = if dlt > 0.0 { ORANGE } else { GREEN };
                ui.label(RichText::new(format!("{}{}", if dlt >= 0.0 { "+" } else { "-" }, f(dlt.abs()))).color(c));
                ui.end_row();
            }
        });
    });
    ui.add_space(6.0);
    card(ui, "Likely impact", |ui| {
        let get = |n: &str| d.iter().find(|c| c.name == n);
        let mut lines = vec![];
        if let Some(c) = get("Startup") {
            if !c.added.is_empty() {
                lines.push((ORANGE, format!("{} new startup entries — sign-in will take longer: {}", c.added.len(), c.added.iter().map(|x| x.0.clone()).collect::<Vec<_>>().join(", "))));
            }
            if !c.removed.is_empty() {
                lines.push((GREEN, format!("{} startup entries removed — faster sign-in", c.removed.len())));
            }
        }
        if let Some(c) = get("Services") {
            let new_auto = c.added.iter().filter(|x| x.1.starts_with("Auto")).count() + c.changed.iter().filter(|x| x.2.starts_with("Auto") && !x.1.starts_with("Auto")).count();
            if new_auto > 0 {
                lines.push((ORANGE, format!("{new_auto} services now start automatically — more background load")));
            }
            let stopped = c.changed.iter().filter(|x| x.1.contains("Running") && !x.2.contains("Running")).count();
            if stopped > 0 {
                lines.push((YELLOW, format!("{stopped} services stopped running since the earlier snapshot")));
            }
        }
        if let Some(c) = get("Drivers") {
            if !c.added.is_empty() || !c.changed.is_empty() {
                lines.push((YELLOW, format!("{} new and {} changed drivers — watch for stability or stutter issues", c.added.len(), c.changed.len())));
            }
        }
        if let Some(c) = get("Programs") {
            if !c.added.is_empty() {
                lines.push((BLUE, format!("{} programs installed or updated", c.added.len() + c.changed.len())));
            }
        }
        if let Some(c) = get("Updates") {
            if !c.added.is_empty() {
                lines.push((BLUE, format!("{} Windows updates installed: {}", c.added.len(), c.added.iter().map(|x| x.0.clone()).collect::<Vec<_>>().join(", "))));
            }
        }
        if let Some(c) = get("Devices") {
            let newly_broken = c.changed.iter().filter(|x| x.1 == "0" && x.2 != "0").count();
            if newly_broken > 0 {
                lines.push((RED, format!("{newly_broken} devices developed a problem")));
            }
        }
        if let Some(c) = get("Tasks") {
            if !c.added.is_empty() {
                lines.push((YELLOW, format!("{} new scheduled tasks", c.added.len())));
            }
        }
        let cpu_d = jf64(pb_, "cpu_avg") - jf64(pa_, "cpu_avg");
        if cpu_d > 10.0 {
            lines.push((ORANGE, format!("Average CPU load rose by {cpu_d:.0} points between the snapshots")));
        }
        let mem_d = jf64(pb_, "mem_used") - jf64(pa_, "mem_used");
        if mem_d > 1e9 {
            lines.push((ORANGE, format!("Memory in use grew by {}", util::fmt_bytes(mem_d))));
        }
        if lines.is_empty() {
            lines.push((GREEN, "No changes that usually affect performance.".into()));
        }
        for (c, l) in lines {
            ui.label(RichText::new(format!("• {l}")).color(c));
        }
        ui.label(RichText::new("Load figures are what was measured at each snapshot; other activity at the time can also explain differences.").color(c_dim()).size(11.5));
    });
}
