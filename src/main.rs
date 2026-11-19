#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]
//! Advanced TaskManager — an advanced, low-overhead Windows task manager with a
//! Standard view and an Admin view.

mod cleanup;
mod collector;
mod config;
mod disk;
mod etw;
mod intel;
mod nt;
mod pdh;
mod queries;
mod sys;
mod ui;
mod util;

fn main() -> eframe::Result<()> {
    let args: Vec<String> = std::env::args().collect();
    // Dry run of the leftover scanner (lists matches, deletes nothing): --leftovers-test <name> <out.txt>
    if args.len() >= 4 && args[1] == "--leftovers-test" {
        let progs = cleanup::programs();
        let p = progs.iter().find(|p| p.name.to_lowercase().contains(&args[2].to_lowercase())).cloned().unwrap_or(cleanup::Program { name: args[2].clone(), ..Default::default() });
        let mut out = format!("{} | {} | {}\n", p.name, p.publisher, p.location);
        for lvl in [cleanup::Level::Safe, cleanup::Level::Moderate, cleanup::Level::Advanced] {
            for l in cleanup::scan_leftovers(&p, lvl) {
                out.push_str(&format!("{lvl:?}\t{}\t{:?}\t{}\t{}\n", if l.selected { "x" } else { " " }, l.kind, l.path, l.reason));
            }
        }
        let _ = std::fs::write(&args[3], out);
        return Ok(());
    }
    let mut screenshot = None;
    let mut page = None;
    let mut opts = ui::StartOpts::default();
    let mut i = 1;
    while i < args.len() {
        match args[i].as_str() {
            "--screenshot" if i + 1 < args.len() => {
                screenshot = Some(std::path::PathBuf::from(&args[i + 1]));
                i += 1;
            }
            "--view" if i + 1 < args.len() => {
                opts.preview_admin = args[i + 1] == "admin";
                i += 1;
            }
            "--pid" if i + 1 < args.len() => {
                opts.pid = args[i + 1].parse().ok();
                i += 1;
            }
            "--theme" if i + 1 < args.len() => {
                opts.theme = Some(args[i + 1].clone());
                i += 1;
            }
            "--compact" => opts.compact = true,
            "--scan" if i + 1 < args.len() => {
                opts.scan = Some(args[i + 1].clone());
                if page.is_none() {
                    page = Some(ui::Page::DiskSpace);
                }
                i += 1;
            }
            "--wait" if i + 1 < args.len() => {
                opts.wait_secs = args[i + 1].parse().unwrap_or(0);
                i += 1;
            }
            "--tab" if i + 1 < args.len() => {
                opts.tab = Some(args[i + 1].clone());
                i += 1;
            }
            "--page" if i + 1 < args.len() => {
                page = page_from_name(&args[i + 1]);
                i += 1;
            }
            _ => {}
        }
        i += 1;
    }

    let mut viewport = egui::ViewportBuilder::default()
        .with_title("Advanced TaskManager")
        .with_inner_size([1480.0, 920.0])
        .with_min_inner_size([1000.0, 640.0]);
    if screenshot.is_some() {
        viewport = viewport.with_active(false);
        #[cfg(windows)]
        {
            viewport = viewport.with_window_level(egui::WindowLevel::AlwaysOnBottom);
        }
    }
    let options = eframe::NativeOptions { viewport, ..Default::default() };
    eframe::run_native("Advanced TaskManager", options, Box::new(move |cc| Ok(Box::new(ui::App::new(cc, screenshot, page, opts)))))
}

fn page_from_name(n: &str) -> Option<ui::Page> {
    use ui::Page::*;
    Some(match n.to_lowercase().as_str() {
        "dashboard" => Dashboard,
        "processes" => Processes,
        "performance" => Performance,
        "services" => Services,
        "startup" => Startup,
        "health" => Health,
        "timeline" => Timeline,
        "graphs" => Graphs,
        "rules" => Rules,
        "sysinfo" => SystemInfo,
        "devices" => Devices,
        "custom" => CustomDash,
        "behaviour" => Behaviour,
        "inspector" => Inspector,
        "power" => Power,
        "uninstaller" => Uninstaller,
        "storeapps" => StoreApps,
        "extensions" => Extensions,
        "junk" => Junk,
        "privacy" => Privacy,
        "shred" => Shred,
        "backups" => Backups,
        "tools" => WinTools,
        "diskspace" | "disk" => DiskSpace,
        "adapters" => Adapters,
        "winconfig" => WinConfig,
        "snapshots" => Snapshots,
        "events" => Events,
        "recording" => Recording,
        "sensors" => Sensors,
        "diskhealth" => DiskHealth,
        "bottlenecks" => Bottlenecks,
        "leaks" => Leaks,
        "startupanalysis" => StartupAnalysis,
        "correlations" => Correlations,
        "changeimpact" => ChangeImpact,
        "alerts" => Alerts,
        "profiles" => Profiles,
        "schedules" => Schedules,
        "scripts" => Scripts,
        "settings" => Settings,
        "history" => History,
        "tasks" => Tasks,
        "drivers" => Drivers,
        "handles" => Handles,
        "etw" => Etw,
        _ => return None,
    })
}
