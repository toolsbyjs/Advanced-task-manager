//! The application window: top bar, navigation, view modes and page routing.

pub mod automation;
pub mod cleanup;
pub mod dashboard;
pub mod deep;
pub mod diskspace;
pub mod insights;
pub mod manage;
pub mod perf;
pub mod processes;
pub mod settings;
pub mod shell;
pub mod widgets;

use crate::collector::{self, Shared};
use crate::sys;
use egui::{Color32, RichText};
use std::time::Instant;
use widgets::*;

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Standard,
    Admin,
}

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Dashboard,
    CustomDash,
    Processes,
    Performance,
    History,
    SystemInfo,
    DiskSpace,
    Services,
    Startup,
    Tasks,
    Drivers,
    Devices,
    Power,
    Adapters,
    WinConfig,
    Snapshots,
    Uninstaller,
    StoreApps,
    Extensions,
    Junk,
    Privacy,
    Shred,
    Backups,
    WinTools,
    Inspector,
    Handles,
    Etw,
    Events,
    Timeline,
    Recording,
    Graphs,
    Sensors,
    DiskHealth,
    Bottlenecks,
    Behaviour,
    Leaks,
    StartupAnalysis,
    Correlations,
    Health,
    ChangeImpact,
    Rules,
    Alerts,
    Profiles,
    Schedules,
    Scripts,
    Settings,
}

pub struct PageInfo {
    pub page: Page,
    pub label: &'static str,
    pub icon: &'static str,
    pub admin_only: bool,
}

const fn pi(page: Page, icon: &'static str, label: &'static str, admin_only: bool) -> PageInfo {
    PageInfo { page, label, icon, admin_only }
}

pub const NAV: &[(&str, &[PageInfo])] = &[
    ("OVERVIEW", &[pi(Page::Dashboard, "🏠", "Dashboard", false), pi(Page::CustomDash, "📋", "My dashboards", false)]),
    (
        "MONITOR",
        &[
            pi(Page::Processes, "☰", "Processes", false),
            pi(Page::Performance, "📈", "Performance", false),
            pi(Page::History, "⟲", "Process history", false),
            pi(Page::SystemInfo, "🖥", "System information", false),
        ],
    ),
    (
        "MANAGE",
        &[
            pi(Page::DiskSpace, "📁", "Disk space analyzer", false),
            pi(Page::Services, "⚙", "Services", false),
            pi(Page::Startup, "🚀", "Startup apps", false),
            pi(Page::Tasks, "⏰", "Scheduled tasks", false),
            pi(Page::Drivers, "🔩", "Drivers", false),
            pi(Page::Devices, "🔌", "Devices", false),
            pi(Page::Power, "🔋", "Power", false),
            pi(Page::Adapters, "🌐", "Network adapters", false),
            pi(Page::WinConfig, "🔧", "Windows settings", false),
            pi(Page::Snapshots, "📷", "Snapshots & changes", false),
        ],
    ),
    (
        "CLEAN UP",
        &[
            pi(Page::Uninstaller, "📦", "Uninstaller", false),
            pi(Page::StoreApps, "📱", "Windows apps", false),
            pi(Page::Extensions, "📎", "Browser extensions", false),
            pi(Page::Junk, "♻", "Junk files cleaner", false),
            pi(Page::Privacy, "🔒", "Privacy cleaner", false),
            pi(Page::Shred, "🔥", "Secure delete", false),
            pi(Page::Backups, "⟲", "Backups & restore", false),
            pi(Page::WinTools, "🛠", "Windows tools", false),
        ],
    ),
    (
        "DEEP",
        &[
            pi(Page::Inspector, "🔬", "Process inspector", true),
            pi(Page::Handles, "🔗", "Kernel objects", true),
            pi(Page::Etw, "⚡", "Live system events", true),
            pi(Page::Events, "📜", "Event Viewer", false),
            pi(Page::Timeline, "⏱", "Diagnostics timeline", false),
            pi(Page::Recording, "⏺", "Recording", false),
            pi(Page::Graphs, "📊", "Historical graphs", false),
            pi(Page::Sensors, "♨", "Hardware sensors", false),
            pi(Page::DiskHealth, "💽", "Disk health", false),
        ],
    ),
    (
        "INSIGHTS",
        &[
            pi(Page::Health, "❤", "Health report", false),
            pi(Page::Bottlenecks, "⛔", "Bottlenecks", false),
            pi(Page::Behaviour, "💡", "Process behaviour", false),
            pi(Page::Leaks, "💧", "Leak candidates", false),
            pi(Page::StartupAnalysis, "⏳", "Startup analysis", false),
            pi(Page::Correlations, "🔀", "Correlated events", false),
            pi(Page::ChangeImpact, "Δ", "Change impact", false),
        ],
    ),
    (
        "AUTOMATE",
        &[
            pi(Page::Rules, "⚖", "Rules", false),
            pi(Page::Alerts, "🔔", "Alerts", false),
            pi(Page::Profiles, "👤", "Profiles", false),
            pi(Page::Schedules, "📅", "Scheduled monitoring", false),
            pi(Page::Scripts, "📝", "Scripts", false),
        ],
    ),
    ("", &[pi(Page::Settings, "⚙", "Settings", false)]),
];

pub fn page_key(p: Page) -> String {
    format!("{p:?}")
}

pub fn page_by_key(k: &str) -> Option<Page> {
    NAV.iter().flat_map(|(_, items)| items.iter()).find(|i| page_key(i.page) == k).map(|i| i.page)
}

pub fn page_info(p: Page) -> &'static PageInfo {
    for (_, items) in NAV {
        for i in items.iter() {
            if i.page == p {
                return i;
            }
        }
    }
    &NAV[0].1[0]
}

pub struct Toast {
    pub text: String,
    pub error: bool,
    pub at: Instant,
}

pub enum Confirm {
    Kill(u32, String),
    KillTree(u32, String),
    Suspend(u32, String),
    ServiceStop(String),
    Recycle(String, u32),
    Cleanup(String, String, cleanup::Pending),
}

/// Extra start-up options (used for checks and shortcuts).
#[derive(Default)]
pub struct StartOpts {
    pub preview_admin: bool,
    pub pid: Option<u32>,
    pub tab: Option<String>,
    pub wait_secs: u64,
    pub theme: Option<String>,
    pub compact: bool,
    pub scan: Option<String>,
}

pub struct ScreenshotState {
    pub path: std::path::PathBuf,
    pub frames: u32,
    pub requested: bool,
    pub started: Instant,
    pub wait_secs: u64,
}

pub struct App {
    pub shared: Shared,
    pub elevated: bool,
    pub mode: Mode,
    pub page: Page,
    pub toasts: Vec<Toast>,
    pub confirm: Option<Confirm>,
    pub ask_elevate: bool,
    pub shot: Option<ScreenshotState>,
    pub procs: processes::ProcUi,
    pub perf: perf::PerfUi,
    pub dash: dashboard::DashUi,
    pub manage: manage::ManageUi,
    pub deep: deep::DeepUi,
    pub insights: insights::InsightsUi,
    pub auto: automation::AutoUi,
    pub disk: diskspace::DiskUi,
    pub clean: cleanup::CleanUi,
    pub nav_filter: String,
    pub prefs: crate::config::UiPrefs,
    pub applied: Option<crate::config::UiPrefs>,
    pub palette: PaletteState,
    pub show_help: bool,
    pub settings_tab: settings::SetTab,
}

#[derive(Default)]
pub struct PaletteState {
    pub open: bool,
    pub query: String,
    pub sel: usize,
    pub just_opened: bool,
}

impl App {
    pub fn new(cc: &eframe::CreationContext, screenshot: Option<std::path::PathBuf>, start_page: Option<Page>, opts: StartOpts) -> Self {
        let elevated = sys::is_elevated();
        if elevated {
            sys::enable_debug_privilege();
        }
        let shared = collector::start(cc.egui_ctx.clone(), elevated);
        let mut prefs = shared.lock().unwrap().config.ui.clone();
        if let Some(t) = &opts.theme {
            prefs.theme = t.clone();
        }
        if opts.compact {
            prefs.nav_compact = true;
        }
        apply_prefs(&cc.egui_ctx, &prefs);
        let start = start_page.or_else(|| page_by_key(&prefs.start_page)).unwrap_or(Page::Dashboard);
        Self {
            shared,
            elevated,
            mode: if elevated || opts.preview_admin { Mode::Admin } else { Mode::Standard },
            page: start,
            toasts: vec![],
            confirm: None,
            ask_elevate: false,
            shot: screenshot.map(|path| ScreenshotState { path, frames: 0, requested: false, started: Instant::now(), wait_secs: opts.wait_secs }),
            procs: processes::ProcUi { selected: opts.pid, tab: processes::tab_from_name(opts.tab.as_deref().unwrap_or("")), ..Default::default() },
            perf: perf::PerfUi::default(),
            dash: dashboard::DashUi::default(),
            manage: manage::ManageUi::default(),
            deep: deep::DeepUi::default(),
            insights: insights::InsightsUi::default(),
            auto: automation::AutoUi::default(),
            disk: diskspace::DiskUi { tab: diskspace::tab_from_name(opts.tab.as_deref().unwrap_or("")), autostart: opts.scan.is_some(), target: opts.scan.clone().unwrap_or_else(|| "C:\\".into()), ..Default::default() },
            nav_filter: String::new(),
            applied: Some(prefs.clone()),
            prefs,
            clean: cleanup::CleanUi::default(),
            palette: PaletteState::default(),
            show_help: false,
            settings_tab: settings::SetTab::default(),
        }
    }

    pub fn admin(&self) -> bool {
        self.mode == Mode::Admin
    }

    pub fn notify(&mut self, text: impl Into<String>, error: bool) {
        self.toasts.push(Toast { text: text.into(), error, at: Instant::now() });
    }

    pub fn result(&mut self, r: Result<String, String>) {
        match r {
            Ok(s) => self.notify(s, false),
            Err(e) => self.notify(e, true),
        }
    }

    /// Shows a locked card for pages that need the Admin view; returns true when locked.
    pub fn locked(&mut self, ui: &mut egui::Ui, what: &str) -> bool {
        if self.admin() {
            return false;
        }
        ui.add_space(30.0);
        card(ui, "", |ui| {
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("🛡").size(42.0).color(PURPLE));
                ui.label(RichText::new("This needs the Admin view").size(20.0).strong());
                ui.label(RichText::new(what).color(c_dim()));
                ui.add_space(8.0);
                if ui.button(RichText::new(if self.elevated { "Switch to Admin view" } else { "Restart as administrator" }).size(15.0)).clicked() {
                    self.request_admin();
                }
            });
        });
        true
    }

    pub fn request_admin(&mut self) {
        if self.elevated {
            self.mode = Mode::Admin;
        } else {
            self.ask_elevate = true;
        }
    }

    pub fn admin_hint(&mut self, ui: &mut egui::Ui, text: &str) {
        if !self.admin() {
            ui.horizontal(|ui| {
                notice(ui, &format!("🛡 {text}"), PURPLE);
            });
        }
    }
}

impl eframe::App for App {
    fn update(&mut self, ctx: &egui::Context, _frame: &mut eframe::Frame) {
        if let Ok(chars) = std::env::var("DTM_GLYPHS") {
            let missing: String = ctx.fonts(|f| {
                let id = egui::FontId::proportional(14.0);
                chars.chars().filter(|c| !c.is_whitespace() && !f.has_glyph(&id, *c)).collect()
            });
            let _ = std::fs::write("glyphs_missing.txt", missing);
            std::process::exit(0);
        }
        if self.shot.is_some() {
            ctx.request_repaint();
        } else {
            ctx.request_repaint_after(std::time::Duration::from_millis(500));
        }
        self.sync_prefs(ctx);
        self.shortcuts(ctx);
        self.handle_screenshot(ctx);
        self.top_bar(ctx);
        if self.prefs.status_bar {
            self.status_bar(ctx);
        }
        self.side_nav(ctx);
        egui::CentralPanel::default()
            .frame(egui::Frame::none().fill(c_bg()).inner_margin(egui::Margin::same(14.0)))
            .show(ctx, |ui| {
                self.route(ui);
            });
        self.dialogs(ctx);
        self.command_palette(ctx);
        self.help_window(ctx);
        self.draw_toasts(ctx);
        // background requests from the sampler
        let (snaps, reports, actions) = {
            let mut s = self.shared.lock().unwrap();
            let r = (s.snapshot_requests, s.report_requests, std::mem::take(&mut s.pending_actions));
            s.snapshot_requests = 0;
            s.report_requests = 0;
            r
        };
        if snaps > 0 {
            manage::take_snapshot(self, ctx);
        }
        if reports > 0 {
            insights::write_report(self, false);
        }
        for a in actions {
            if let Some(name) = a.strip_prefix("profile:") {
                automation::apply_sys_profile_by_name(self, name);
            }
        }
    }
}

impl App {
    fn handle_screenshot(&mut self, ctx: &egui::Context) {
        let Some(st) = self.shot.as_mut() else { return };
        st.frames += 1;
        let mut done = None;
        ctx.input(|i| {
            for e in &i.raw.events {
                if let egui::Event::Screenshot { image, .. } = e {
                    done = Some(image.clone());
                }
            }
        });
        if let Some(img) = done {
            let w = img.size[0] as u32;
            let h = img.size[1] as u32;
            let mut raw = Vec::with_capacity((w * h * 4) as usize);
            for p in &img.pixels {
                raw.extend_from_slice(&[p.r(), p.g(), p.b(), 255]);
            }
            if let Some(parent) = st.path.parent() {
                let _ = std::fs::create_dir_all(parent);
            }
            let res = image::save_buffer(&st.path, &raw, w, h, image::ColorType::Rgba8);
            if let Err(e) = res {
                eprintln!("screenshot failed: {e}");
            }
            std::process::exit(0);
        }
        let samples = self.shared.lock().map(|s| s.samples).unwrap_or(0);
        let ready = (samples >= 6 && st.started.elapsed().as_secs() >= st.wait_secs) || st.started.elapsed().as_secs() > 20 + st.wait_secs;
        if st.frames >= 120 && ready && !st.requested {
            st.requested = true;
            ctx.send_viewport_cmd(egui::ViewportCommand::Screenshot);
        }
        if st.started.elapsed().as_secs() > 60 + st.wait_secs {
            eprintln!("screenshot timed out");
            std::process::exit(1);
        }
    }

    fn route(&mut self, ui: &mut egui::Ui) {
        let ctx = ui.ctx().clone();
        match self.page {
            Page::Dashboard => dashboard::overview(self, ui),
            Page::CustomDash => dashboard::custom(self, ui),
            Page::Processes => processes::page(self, ui),
            Page::Performance => perf::page(self, ui),
            Page::History => processes::history_page(self, ui),
            Page::SystemInfo => manage::system_info(self, ui, &ctx),
            Page::DiskSpace => diskspace::page(self, ui),
            Page::Services => manage::services(self, ui, &ctx),
            Page::Startup => manage::startup(self, ui, &ctx),
            Page::Tasks => manage::tasks(self, ui, &ctx),
            Page::Drivers => manage::drivers(self, ui, &ctx),
            Page::Devices => manage::devices(self, ui, &ctx),
            Page::Power => manage::power(self, ui, &ctx),
            Page::Adapters => manage::adapters(self, ui, &ctx),
            Page::WinConfig => manage::windows_config(self, ui, &ctx),
            Page::Snapshots => manage::snapshots(self, ui, &ctx),
            Page::Uninstaller => cleanup::uninstaller(self, ui, &ctx),
            Page::StoreApps => cleanup::store_apps(self, ui, &ctx),
            Page::Extensions => cleanup::extensions(self, ui, &ctx),
            Page::Junk => cleanup::junk(self, ui, &ctx),
            Page::Privacy => cleanup::privacy(self, ui, &ctx),
            Page::Shred => cleanup::secure_delete(self, ui, &ctx),
            Page::Backups => cleanup::backups(self, ui, &ctx),
            Page::WinTools => cleanup::tools(self, ui),
            Page::Inspector => deep::inspector(self, ui),
            Page::Handles => deep::kernel_objects(self, ui, &ctx),
            Page::Etw => deep::etw(self, ui),
            Page::Events => deep::events(self, ui, &ctx),
            Page::Timeline => deep::timeline(self, ui),
            Page::Recording => deep::recording(self, ui),
            Page::Graphs => deep::graphs(self, ui),
            Page::Sensors => deep::sensors(self, ui, &ctx),
            Page::DiskHealth => deep::disk_health(self, ui, &ctx),
            Page::Bottlenecks => insights::bottlenecks(self, ui),
            Page::Behaviour => insights::behaviour(self, ui),
            Page::Leaks => insights::leaks(self, ui),
            Page::StartupAnalysis => insights::startup_analysis(self, ui, &ctx),
            Page::Correlations => insights::correlations(self, ui),
            Page::Health => insights::health(self, ui, &ctx),
            Page::ChangeImpact => manage::change_impact(self, ui, &ctx),
            Page::Rules => automation::rules(self, ui),
            Page::Alerts => automation::alerts(self, ui),
            Page::Profiles => automation::profiles(self, ui),
            Page::Schedules => automation::schedules(self, ui),
            Page::Scripts => automation::scripts(self, ui),
            Page::Settings => settings::page(self, ui),
        }
    }

    fn dialogs(&mut self, ctx: &egui::Context) {
        if self.ask_elevate {
            let mut open = true;
            egui::Window::new("Switch to Admin view")
                .collapsible(false)
                .resizable(false)
                .anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0])
                .open(&mut open)
                .show(ctx, |ui| {
                    ui.label("The Admin view shows every detail (threads, handles, memory maps, live system events)");
                    ui.label("and lets you control services, drivers and other users' processes.");
                    ui.label(RichText::new("Windows will ask you to confirm. This window will close and reopen as administrator.").color(c_dim()));
                    ui.add_space(8.0);
                    ui.horizontal(|ui| {
                        if ui.button(RichText::new("🛡 Restart as administrator").color(PURPLE)).clicked() {
                            if sys::relaunch_as_admin("--admin") {
                                std::process::exit(0);
                            } else {
                                self.notify("Windows did not start the administrator copy (the prompt was cancelled?)", true);
                            }
                            self.ask_elevate = false;
                        }
                        if ui.button("Stay in Standard view").clicked() {
                            self.ask_elevate = false;
                        }
                    });
                });
            if !open {
                self.ask_elevate = false;
            }
        }
        if let Some(c) = self.confirm.take() {
            let (title, text) = match &c {
                Confirm::Kill(pid, n) => ("End process?", format!("End {n} (PID {pid})? Unsaved work in it will be lost.")),
                Confirm::KillTree(pid, n) => ("End process tree?", format!("End {n} (PID {pid}) and every process it started?")),
                Confirm::Suspend(pid, n) => ("Suspend process?", format!("Freeze {n} (PID {pid})? It will stop responding until resumed.")),
                Confirm::ServiceStop(n) => ("Stop service?", format!("Stop the service '{n}'? Programs that need it may stop working.")),
                Confirm::Recycle(p, _) => ("Move to Recycle Bin?", format!("Move this to the Recycle Bin?\n{p}\nYou can restore it from the Recycle Bin. Don't remove files Windows or your programs need.")),
                Confirm::Cleanup(t, x, _) => (t.as_str(), x.clone()),
            };
            let mut keep = true;
            let mut go = false;
            egui::Window::new(title).collapsible(false).resizable(false).anchor(egui::Align2::CENTER_CENTER, [0.0, 0.0]).show(ctx, |ui| {
                ui.label(text);
                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    if ui.button(RichText::new("Yes, do it").color(RED)).clicked() {
                        go = true;
                        keep = false;
                    }
                    if ui.button("Cancel").clicked() {
                        keep = false;
                    }
                });
            });
            if go {
                match c {
                    Confirm::Kill(pid, n) => {
                        let r = sys::terminate(pid).map(|_| format!("Ended {n}"));
                        self.result(r);
                    }
                    Confirm::KillTree(pid, n) => {
                        let r = processes::kill_tree(self, pid).map(|k| format!("Ended {n} and {k} child processes"));
                        self.result(r);
                    }
                    Confirm::Suspend(pid, n) => {
                        let r = sys::suspend_process(pid, true).map(|_| format!("Suspended {n}"));
                        self.result(r);
                    }
                    Confirm::ServiceStop(n) => manage::run_service_action(self, ctx, n, "stop"),
                    Confirm::Recycle(p, i) => diskspace::recycle_node(self, i, p),
                    Confirm::Cleanup(_, _, p) => cleanup::run_pending(self, ctx, p),
                }
            } else if keep {
                self.confirm = Some(c);
            }
        }
    }

    fn draw_toasts(&mut self, ctx: &egui::Context) {
        let secs = self.prefs.toast_secs.max(2);
        self.toasts.retain(|t| t.at.elapsed().as_secs() < secs);
        if self.toasts.is_empty() {
            return;
        }
        egui::Area::new(egui::Id::new("toasts")).anchor(egui::Align2::RIGHT_BOTTOM, [-16.0, -16.0]).show(ctx, |ui| {
            for t in &self.toasts {
                let c: Color32 = if t.error { RED } else { GREEN };
                egui::Frame::none().fill(c_card_hi()).stroke(egui::Stroke::new(1.0, c)).rounding(8.0).inner_margin(egui::Margin::symmetric(12.0, 8.0)).show(ui, |ui| {
                    ui.set_max_width(420.0);
                    ui.label(RichText::new(&t.text).color(c));
                });
                ui.add_space(4.0);
            }
        });
    }
}
