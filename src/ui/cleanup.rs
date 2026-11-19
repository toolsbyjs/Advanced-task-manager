//! Clean-up pages: uninstaller with leftover wizard and history, Windows (Store) apps,
//! browser extensions, junk files, privacy traces, secure delete, backups & restore
//! points, and a Windows tools launcher.

use super::widgets::*;
use super::{App, Confirm};
use crate::cleanup::{self, Extension, JunkCat, JunkScan, Leftover, LeftKind, Level, Program, RemoveReport, Trace};
use crate::util::{self, jget, jstr, Job};
use egui::{RichText, Sense, Ui};
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;

#[derive(Clone)]
pub enum Pending {
    Uninstall { quiet: bool },
    RemoveLeftovers,
    RemoveEntry(Program),
    RemoveAppx { full: String, name: String },
    RemoveExtension(Extension),
    CleanJunk,
    EmptyRecycle,
    CleanTraces,
    Shred,
    RestoreBackup(PathBuf),
}

#[derive(Clone, Copy, PartialEq, Eq)]
enum Stage {
    Running,
    Scanning,
    Review,
    Removing,
    Done,
}

pub struct Wizard {
    program: Program,
    stage: Stage,
    how: String,
    run: Job<(String, Result<u32, String>)>,
    level: Level,
    scan: Job<Vec<Leftover>>,
    items: Vec<Leftover>,
    remove: Job<RemoveReport>,
    report: Option<RemoveReport>,
    forced: bool,
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum UTab {
    Programs,
    History,
    Forced,
}

pub struct CleanUi {
    pub programs: Job<Vec<Program>>,
    pub sizes: Job<HashMap<String, u64>>,
    pub filter: String,
    pub show_system: bool,
    pub show_updates: bool,
    pub only_new: bool,
    pub sort: (usize, bool),
    pub selected: Option<String>,
    pub restore_point: bool,
    pub level: Level,
    pub wizard: Option<Wizard>,
    pub utab: UTab,
    pub forced_name: String,
    pub forced_publisher: String,
    pub forced_folder: String,
    pub appx: Job<Vec<serde_json::Value>>,
    pub appx_t: TableState,
    pub appx_all: bool,
    pub appx_system: bool,
    pub action: Job<String>,
    pub exts: Job<Vec<Extension>>,
    pub ext_filter: String,
    pub junk: Vec<JunkCat>,
    pub junk_sel: HashSet<&'static str>,
    pub junk_older: bool,
    pub junk_bin: bool,
    pub junk_scan: Job<(Vec<(&'static str, JunkScan)>, (u64, u64))>,
    pub traces: Job<Vec<Trace>>,
    pub trace_sel: HashSet<&'static str>,
    pub shred_paths: Vec<String>,
    pub shred_input: String,
    pub shred_passes: u32,
    pub shred_job: Job<(u64, u64, Vec<String>)>,
    pub browse: Job<String>,
    pub points: Job<Vec<serde_json::Value>>,
}

impl Default for CleanUi {
    fn default() -> Self {
        let junk = cleanup::junk_categories();
        let junk_sel = junk.iter().filter(|c| c.default_on).map(|c| c.key).collect();
        Self {
            programs: Job::default(),
            sizes: Job::default(),
            filter: String::new(),
            show_system: false,
            show_updates: false,
            only_new: false,
            sort: (0, true),
            selected: None,
            restore_point: false,
            level: Level::Moderate,
            wizard: None,
            utab: UTab::Programs,
            forced_name: String::new(),
            forced_publisher: String::new(),
            forced_folder: String::new(),
            appx: Job::default(),
            appx_t: TableState::default(),
            appx_all: false,
            appx_system: false,
            action: Job::default(),
            exts: Job::default(),
            ext_filter: String::new(),
            junk,
            junk_sel,
            junk_older: true,
            junk_bin: false,
            junk_scan: Job::default(),
            traces: Job::default(),
            trace_sel: ["recent", "run", "typed", "search"].into_iter().collect(),
            shred_paths: vec![],
            shred_input: String::new(),
            shred_passes: 1,
            shred_job: Job::default(),
            browse: Job::default(),
            points: Job::default(),
        }
    }
}

fn fixed(ui: &mut Ui, w: f32, t: impl Into<egui::WidgetText>) {
    ui.allocate_ui_with_layout(egui::vec2(w, 18.0), egui::Layout::left_to_right(egui::Align::Center), |ui| {
        ui.set_min_width(w);
        ui.add(egui::Label::new(t).truncate());
    });
}

fn ask(app: &mut App, title: &str, text: String, p: Pending) {
    app.confirm = Some(Confirm::Cleanup(title.to_string(), text, p));
}

fn poll_action(app: &mut App) {
    if app.clean.action.started && !app.clean.action.is_running() {
        let r = app.clean.action.with(|r| r.cloned());
        app.clean.action = Job::default();
        if let Some(r) = r {
            app.result(r);
        }
    }
}

/// Runs an action the person confirmed.
pub fn run_pending(app: &mut App, ctx: &egui::Context, p: Pending) {
    let c = &mut app.clean;
    match p {
        Pending::Uninstall { quiet } => {
            let Some(w) = c.wizard.as_mut() else { return };
            let prog = w.program.clone();
            let rp = c.restore_point;
            w.how = if quiet { "Quick uninstall".into() } else { "Uninstall".into() };
            w.stage = Stage::Running;
            w.run.start(ctx, move || {
                let mut note = String::new();
                if rp {
                    note = match cleanup::create_restore_point(&format!("Before removing {}", prog.name)) {
                        Ok(m) => m,
                        Err(e) => format!("No restore point: {e}"),
                    };
                }
                let Some((exe, args)) = cleanup::uninstall_command(&prog, quiet) else {
                    return Ok((note, Err("This program has no uninstaller; use the leftover scan to remove it.".into())));
                };
                Ok((note, cleanup::run_and_wait(&exe, &args)))
            });
        }
        Pending::RemoveLeftovers => {
            let Some(w) = c.wizard.as_mut() else { return };
            let items = w.items.clone();
            let name = w.program.name.clone();
            w.stage = Stage::Removing;
            w.remove.start(ctx, move || Ok(cleanup::remove_leftovers(&name, &items)));
        }
        Pending::RemoveEntry(prog) => {
            let item = Leftover { kind: LeftKind::RegistryKey, path: prog.key_path.clone(), reason: "uninstall entry".into(), size: 0, selected: true };
            let rep = cleanup::remove_leftovers(&prog.name, &[item]);
            cleanup::add_history(cleanup::HistoryEntry { date: util::fmt_time(util::now_unix()), program: prog.name.clone(), version: prog.version.clone(), publisher: prog.publisher.clone(), how: "Removed entry".into(), leftovers_removed: rep.removed, freed: 0 });
            c.programs.started = false;
            if rep.failed.is_empty() {
                app.notify(format!("Removed the entry for {} (backup saved)", prog.name), false);
            } else {
                app.notify(format!("Could not remove: {}", rep.failed.join("; ")), true);
            }
        }
        Pending::RemoveAppx { full, name } => {
            let all = c.appx_all;
            c.action = Job::default();
            c.action.start(ctx, move || cleanup::remove_store_app(&full, all).map(|m| format!("{name}: {m}")));
            c.appx.started = false;
        }
        Pending::RemoveExtension(e) => {
            let running: Vec<String> = app.shared.lock().unwrap().procs.values().map(|p| p.name.clone()).collect();
            if cleanup::is_running(e.process, &running) {
                app.notify(format!("Close {} first, then remove the extension.", e.browser), true);
                return;
            }
            match crate::disk::recycle(&e.path) {
                Ok(()) => app.notify(format!("Removed {} from {} (it's in the Recycle Bin)", e.name, e.browser), false),
                Err(err) => app.notify(err, true),
            }
            app.clean.exts.started = false;
        }
        Pending::CleanJunk => {
            let cats: Vec<JunkCat> = c.junk.iter().filter(|j| c.junk_sel.contains(j.key)).cloned().collect();
            let older = if c.junk_older { 86400 } else { 0 };
            let bin = c.junk_bin;
            c.action = Job::default();
            c.action.start(ctx, move || {
                let (mut d, mut f, mut s) = (0u64, 0u64, 0u64);
                for cat in &cats {
                    let (a, b, k) = cleanup::clean_junk(cat, older);
                    d += a;
                    f += b;
                    s += k;
                }
                let mut msg = format!("Freed {} ({} files deleted", util::fmt_bytes(f as f64), util::fmt_num(d));
                if s > 0 {
                    msg.push_str(&format!(", {} in use and skipped", util::fmt_num(s)));
                }
                msg.push(')');
                if bin {
                    let (rb, _) = cleanup::recycle_bin();
                    if cleanup::empty_recycle_bin().is_ok() {
                        msg.push_str(&format!("; Recycle Bin emptied ({})", util::fmt_bytes(rb as f64)));
                    }
                }
                Ok(msg)
            });
            c.junk_scan.started = false;
        }
        Pending::EmptyRecycle => {
            let r = cleanup::empty_recycle_bin().map(|_| "Recycle Bin emptied".to_string());
            app.result(r);
            app.clean.junk_scan.started = false;
        }
        Pending::CleanTraces => {
            let keys: Vec<&'static str> = c.trace_sel.iter().copied().collect();
            c.action = Job::default();
            c.action.start(ctx, move || {
                let mut msgs = vec![];
                for k in keys {
                    match cleanup::clean_trace(k) {
                        Ok(m) => msgs.push(format!("{k}: {m}")),
                        Err(e) => msgs.push(format!("{k}: {}", e.trim())),
                    }
                }
                Ok(format!("Privacy clean-up done — {}", msgs.join(", ")))
            });
            c.traces.started = false;
        }
        Pending::Shred => {
            let paths = c.shred_paths.clone();
            let passes = c.shred_passes;
            c.shred_job = Job::default();
            c.shred_job.start(ctx, move || Ok(cleanup::secure_delete(&paths, passes)));
            c.shred_paths.clear();
        }
        Pending::RestoreBackup(dir) => {
            let m = cleanup::backups().into_iter().find(|b| b.0 == dir).map(|b| b.1);
            if let Some(m) = m {
                let r = cleanup::restore_backup(&dir, &m);
                app.result(r);
            }
        }
    }
}

// ------------------------------------------------------------------ uninstaller

fn start_wizard(app: &mut App, p: Program, forced: bool) {
    app.clean.wizard = Some(Wizard {
        program: p,
        stage: if forced { Stage::Scanning } else { Stage::Running },
        how: if forced { "Forced uninstall".into() } else { String::new() },
        run: Job::default(),
        level: app.clean.level,
        scan: Job::default(),
        items: vec![],
        remove: Job::default(),
        report: None,
        forced,
    });
}

pub fn uninstaller(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    poll_action(app);
    page_header(ui, "Uninstaller", "Remove programs completely: run their uninstaller, then find and remove what they leave behind — with a backup of every registry change.");
    if app.clean.wizard.is_some() {
        wizard(app, ui, ctx);
        return;
    }
    ui.horizontal(|ui| {
        ui.selectable_value(&mut app.clean.utab, UTab::Programs, "📦 All programs");
        ui.selectable_value(&mut app.clean.utab, UTab::Forced, "🔍 Forced uninstall / leftovers");
        ui.selectable_value(&mut app.clean.utab, UTab::History, "⟲ Uninstall history");
    });
    match app.clean.utab {
        UTab::Programs => programs(app, ui, ctx),
        UTab::Forced => forced(app, ui),
        UTab::History => history(app, ui),
    }
}

fn programs(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    app.clean.programs.ensure(ctx, || Ok(cleanup::programs()));
    ui.horizontal_wrapped(|ui| {
        search_field(ui, &mut app.clean.filter, "🔍 Search name, publisher or notes", 260.0);
        ui.checkbox(&mut app.clean.only_new, "New (last 7 days)");
        ui.checkbox(&mut app.clean.show_updates, "Show updates");
        ui.checkbox(&mut app.clean.show_system, "Show system components");
        if ui.add_enabled(!app.clean.programs.is_running(), egui::Button::new("⟳ Refresh")).clicked() {
            app.clean.programs.started = false;
            app.clean.sizes = Job::default();
        }
        if ui.add_enabled(!app.clean.sizes.is_running(), egui::Button::new("📏 Measure missing sizes")).on_hover_text("Adds up install folders for programs that don't report their size").clicked() {
            if let Some(list) = app.clean.programs.take_ok() {
                app.clean.sizes.start(ctx, move || {
                    let mut m = HashMap::new();
                    for p in list.iter().filter(|p| p.size_kb == 0 && !p.location.is_empty()) {
                        let path = std::path::Path::new(&p.location);
                        if path.exists() {
                            let mut total = 0u64;
                            let mut stack = vec![path.to_path_buf()];
                            while let Some(d) = stack.pop() {
                                if let Ok(rd) = std::fs::read_dir(&d) {
                                    for e in rd.flatten() {
                                        if let Ok(md) = e.metadata() {
                                            if md.is_dir() {
                                                stack.push(e.path());
                                            } else {
                                                total += md.len();
                                            }
                                        }
                                    }
                                }
                            }
                            m.insert(p.key_path.clone(), total);
                        }
                    }
                    Ok(m)
                });
            }
        }
        if app.clean.sizes.is_running() {
            ui.spinner();
        }
    });
    if !app.admin() {
        notice(ui, "🛡 Programs installed for all users usually need administrator rights to uninstall; their uninstaller will ask, or use the Admin view. Removing their leftover registry keys needs the Admin view.", PURPLE);
    }
    if !job_status(ui, &app.clean.programs) {
        return;
    }
    let all = app.clean.programs.take_ok().unwrap_or_default();
    let sizes = app.clean.sizes.take_ok().unwrap_or_default();
    let size_of = |p: &Program| -> u64 { if p.size_kb > 0 { p.size_kb * 1024 } else { sizes.get(&p.key_path).copied().unwrap_or(0) } };
    let f = app.clean.filter.to_lowercase();
    let mut list: Vec<&Program> = all
        .iter()
        .filter(|p| (app.clean.show_system || !p.system) && (app.clean.show_updates || !p.update))
        .filter(|p| !app.clean.only_new || p.is_new())
        .filter(|p| f.is_empty() || p.name.to_lowercase().contains(&f) || p.publisher.to_lowercase().contains(&f) || p.comments.to_lowercase().contains(&f))
        .collect();
    let (sc, asc) = app.clean.sort;
    list.sort_by(|a, b| {
        let o = match sc {
            1 => size_of(a).cmp(&size_of(b)),
            2 => a.version.cmp(&b.version),
            3 => a.bits.cmp(b.bits),
            4 => a.install_date.cmp(&b.install_date),
            5 => a.publisher.to_lowercase().cmp(&b.publisher.to_lowercase()),
            _ => a.name.to_lowercase().cmp(&b.name.to_lowercase()),
        };
        if asc {
            o
        } else {
            o.reverse()
        }
    });
    let total: u64 = list.iter().map(|p| size_of(p)).sum();
    let new_count = all.iter().filter(|p| p.is_new() && !p.system && !p.update).count();
    ui.horizontal(|ui| {
        pill(ui, &format!("{} programs", list.len()), BLUE);
        pill(ui, &format!("{} reported size", util::fmt_bytes(total as f64)), GREEN);
        if new_count > 0 {
            pill(ui, &format!("{new_count} new this week"), ORANGE);
        }
        if ui.small_button("💾 Export list (CSV)").clicked() {
            let mut s = String::from("name,publisher,version,bits,installed,size_bytes,location,uninstall\n");
            for p in &list {
                s.push_str(&format!("\"{}\",\"{}\",\"{}\",{},{},{},\"{}\",\"{}\"\n", p.name.replace('"', "'"), p.publisher.replace('"', "'"), p.version, p.bits, p.install_date, size_of(p), p.location, p.uninstall.replace('"', "'")));
            }
            let path = util::sub_dir("exports").join(format!("programs-{}.csv", util::stamp()));
            let r = std::fs::write(&path, s).map(|_| {
                crate::sys::open_file_location(&path.to_string_lossy());
                format!("Saved {}", path.display())
            });
            app.result(r.map_err(|e| e.to_string()));
        }
    });
    // details of the selection
    let sel = app.clean.selected.clone().and_then(|k| all.iter().find(|p| p.key_path == k).cloned());
    if let Some(p) = &sel {
        card(ui, "", |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&p.name).size(17.0).strong().color(accent()));
                ui.label(RichText::new(format!("{}  {}  {}", p.version, p.bits, p.scope)).color(c_dim()));
                if p.is_new() {
                    pill(ui, "New", ORANGE);
                }
                if p.msi {
                    pill(ui, "Windows Installer", BLUE);
                }
            });
            ui.columns(2, |cols| {
                kv(
                    &mut cols[0],
                    "prog_kv",
                    &[
                        ("Publisher", p.publisher.clone()),
                        ("Installed", p.install_date.clone()),
                        ("Size", if size_of(p) > 0 { util::fmt_bytes(size_of(p) as f64) } else { "not reported".into() }),
                        ("Folder", p.location.clone()),
                    ],
                );
                kv(&mut cols[1], "prog_kv2", &[("Uninstaller", p.uninstall.clone()), ("Silent uninstall", p.quiet.clone()), ("Registry", p.key_path.clone()), ("Website", p.url.clone())]);
            });
            ui.horizontal_wrapped(|ui| {
                let can = cleanup::uninstall_command(p, false).is_some() && !p.no_remove;
                if ui.add_enabled(can, egui::Button::new(RichText::new("🗑 Uninstall").strong().color(RED))).on_hover_text("Run the program's own uninstaller, then scan for leftovers").clicked() {
                    start_wizard(app, p.clone(), false);
                    ask(app, "Uninstall program?", format!("Uninstall {}?\nIts own uninstaller will open; finish it, then this app looks for leftovers.", p.name), Pending::Uninstall { quiet: false });
                }
                let quiet_ok = !p.quiet.is_empty() || p.msi;
                if ui.add_enabled(quiet_ok, egui::Button::new("⚡ Quick uninstall")).on_hover_text("Silent uninstall where the program supports it").clicked() {
                    start_wizard(app, p.clone(), false);
                    ask(app, "Quick uninstall?", format!("Remove {} silently (no questions from its uninstaller)?", p.name), Pending::Uninstall { quiet: true });
                }
                if ui.button("🔍 Forced uninstall").on_hover_text("Skip the uninstaller (for broken programs) and remove everything found").clicked() {
                    start_wizard(app, p.clone(), true);
                }
                if ui.button("Remove entry only").on_hover_text("For programs already gone: delete their entry from the list (registry backup kept)").clicked() {
                    ask(app, "Remove the entry?", format!("Delete the list entry for {}?\nOnly use this when the program is already gone. A backup is kept.", p.name), Pending::RemoveEntry(p.clone()));
                }
                ui.separator();
                if !p.location.is_empty() && ui.button("📂 Folder").clicked() {
                    crate::sys::shell_open("open", &p.location, "");
                }
                if ui.button("🔧 Registry").on_hover_text("Open Registry Editor at this program's entry").clicked() {
                    cleanup::open_regedit_at(&p.key_path);
                }
                if !p.url.is_empty() && ui.button("🌐 Website").clicked() {
                    crate::sys::shell_open("open", &p.url, "");
                }
                ui.checkbox(&mut app.clean.restore_point, "Create a restore point first").on_hover_text("Needs the Admin view; Windows allows one per day by default");
            });
        });
    }
    let heads = ["Program", "Size", "Version", "Type", "Installed", "Publisher"];
    let mut clicked_head = None;
    let mut new_sel = None;
    let mut dbl = None;
    egui_extras::TableBuilder::new(ui)
        .id_salt("programs")
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .auto_shrink([false, false])
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(egui_extras::Column::initial(330.0).clip(true))
        .column(egui_extras::Column::initial(90.0).clip(true))
        .column(egui_extras::Column::initial(120.0).clip(true))
        .column(egui_extras::Column::initial(70.0).clip(true))
        .column(egui_extras::Column::initial(100.0).clip(true))
        .column(egui_extras::Column::remainder().clip(true))
        .header(22.0, |mut h| {
            for (i, t) in heads.iter().enumerate() {
                h.col(|ui| {
                    let arrow = if sc == i { if asc { " 🔼" } else { " 🔽" } } else { "" };
                    if ui.add(egui::Label::new(RichText::new(format!("{t}{arrow}")).strong()).sense(Sense::click())).clicked() {
                        clicked_head = Some(i);
                    }
                });
            }
        })
        .body(|body| {
            body.rows(row_h(), list.len(), |mut row| {
                let p = list[row.index()];
                row.set_selected(app.clean.selected.as_deref() == Some(p.key_path.as_str()));
                row.col(|ui| {
                    if p.is_new() {
                        ui.label(RichText::new("NEW").size(10.0).strong().color(ORANGE));
                    }
                    let c = if p.system || p.update { c_dim() } else { c_text() };
                    ui.add(egui::Label::new(RichText::new(&p.name).color(c)).truncate());
                });
                row.col(|ui| {
                    let s = size_of(p);
                    ui.label(if s > 0 { util::fmt_bytes(s as f64) } else { String::new() });
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(RichText::new(&p.version).color(c_dim())).truncate());
                });
                row.col(|ui| {
                    ui.label(RichText::new(p.bits).color(c_dim()));
                });
                row.col(|ui| {
                    ui.label(RichText::new(&p.install_date).color(c_dim()));
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(&p.publisher).truncate());
                });
                let r = row.response();
                if r.clicked() {
                    new_sel = Some(p.key_path.clone());
                }
                if r.double_clicked() {
                    dbl = Some(p.clone());
                }
            });
        });
    if let Some(i) = clicked_head {
        app.clean.sort = if app.clean.sort.0 == i { (i, !app.clean.sort.1) } else { (i, i != 1) };
    }
    if let Some(k) = new_sel {
        app.clean.selected = Some(k);
    }
    if let Some(p) = dbl {
        if cleanup::uninstall_command(&p, false).is_some() {
            app.clean.selected = Some(p.key_path.clone());
            start_wizard(app, p.clone(), false);
            ask(app, "Uninstall program?", format!("Uninstall {}?", p.name), Pending::Uninstall { quiet: false });
        }
    }
}

fn wizard(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    let elevated = app.elevated;
    let mut close = false;
    let mut confirm_remove = None;
    let mut history_entry = None;
    {
        let Some(w) = app.clean.wizard.as_mut() else { return };
        // advance stages
        if w.stage == Stage::Running && w.run.started && !w.run.is_running() {
            w.stage = Stage::Scanning;
        }
        if w.stage == Stage::Scanning && !w.scan.started {
            let p = w.program.clone();
            let lvl = w.level;
            w.scan.start(ctx, move || Ok(cleanup::scan_leftovers(&p, lvl)));
        }
        if w.stage == Stage::Scanning && w.scan.started && !w.scan.is_running() {
            w.items = w.scan.take_ok().unwrap_or_default();
            w.stage = Stage::Review;
        }
        if w.stage == Stage::Removing && w.remove.started && !w.remove.is_running() {
            let rep = w.remove.take_ok().unwrap_or_default();
            history_entry = Some(cleanup::HistoryEntry {
                date: util::fmt_time(util::now_unix()),
                program: w.program.name.clone(),
                version: w.program.version.clone(),
                publisher: w.program.publisher.clone(),
                how: w.how.clone(),
                leftovers_removed: rep.removed,
                freed: rep.freed,
            });
            w.report = Some(rep);
            w.stage = Stage::Done;
        }
        let steps = ["1 Uninstaller", "2 Scan", "3 Choose leftovers", "4 Done"];
        let cur = match w.stage {
            Stage::Running => 0,
            Stage::Scanning => 1,
            Stage::Review => 2,
            Stage::Removing | Stage::Done => 3,
        };
        card(ui, "", |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(format!("{} — {}", if w.how.is_empty() { "Uninstall" } else { &w.how }, w.program.name)).size(17.0).strong());
                for (i, s) in steps.iter().enumerate() {
                    if w.forced && i == 0 {
                        continue;
                    }
                    pill(ui, s, if i == cur { accent() } else if i < cur { GREEN } else { c_dim() });
                }
                ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                    if matches!(w.stage, Stage::Review | Stage::Done) && ui.button("✖ Close").clicked() {
                        close = true;
                    }
                });
            });
        });
        ui.add_space(6.0);
        match w.stage {
            Stage::Running => {
                card(ui, "", |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(RichText::new(if w.run.started { "Waiting for the program's uninstaller to finish…" } else { "Waiting for your confirmation…" }).size(15.0));
                    });
                    ui.label(RichText::new("Answer its questions in its own window. Some uninstallers close early and keep working in the background — if so, wait until it's gone and press the button below.").color(c_dim()));
                    if ui.button("It has finished — scan for leftovers").clicked() {
                        w.stage = Stage::Scanning;
                    }
                    if !w.run.started && ui.button("Cancel").clicked() {
                        close = true;
                    }
                });
            }
            Stage::Scanning => {
                if let Some(Ok((note, res))) = w.run.with(|r| r.cloned()) {
                    if !note.is_empty() {
                        notice(ui, &note, BLUE);
                    }
                    if let Err(e) = res {
                        notice(ui, &e, YELLOW);
                    }
                }
                card(ui, "", |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label(format!("Looking for leftovers ({:?} scan) in the registry, program folders, app data, Start menu and desktop…", w.level));
                    });
                });
            }
            Stage::Review => {
                if let Some(Ok((note, res))) = w.run.with(|r| r.cloned()) {
                    if !note.is_empty() {
                        notice(ui, &note, BLUE);
                    }
                    match res {
                        Ok(code) if code != 0 => notice(ui, &format!("The uninstaller ended with code {code} (it may have been cancelled or needed a restart)."), YELLOW),
                        Err(e) => notice(ui, &e, YELLOW),
                        _ => {}
                    }
                }
                ui.horizontal(|ui| {
                    ui.label("Scan mode:");
                    let mut lvl = w.level;
                    ui.selectable_value(&mut lvl, Level::Safe, "Safe").on_hover_text("Only exact name matches");
                    ui.selectable_value(&mut lvl, Level::Moderate, "Moderate").on_hover_text("Also names that start like the program (recommended)");
                    ui.selectable_value(&mut lvl, Level::Advanced, "Advanced").on_hover_text("Deepest scan: services, publisher folders and partial name matches. Check the list carefully.");
                    if lvl != w.level {
                        w.level = lvl;
                        w.scan = Job::default();
                        w.stage = Stage::Scanning;
                    }
                    ui.separator();
                    if ui.small_button("Select all").clicked() {
                        w.items.iter_mut().for_each(|i| i.selected = true);
                    }
                    if ui.small_button("Select none").clicked() {
                        w.items.iter_mut().for_each(|i| i.selected = false);
                    }
                });
                let sel_count = w.items.iter().filter(|i| i.selected).count();
                let sel_size: u64 = w.items.iter().filter(|i| i.selected).map(|i| i.size).sum();
                if w.items.is_empty() {
                    notice(ui, "✔ Nothing left behind was found.", GREEN);
                } else {
                    if !elevated && w.items.iter().any(|i| i.kind == LeftKind::RegistryKey && i.path.starts_with("HKLM")) {
                        notice(ui, "🛡 Keys under HKLM can only be removed in the Admin view; they'll be skipped otherwise.", PURPLE);
                    }
                    egui::ScrollArea::vertical().id_salt("leftovers").max_height(ui.available_height() - 60.0).show(ui, |ui| {
                        for (title, kind) in [("Registry keys", LeftKind::RegistryKey), ("Folders", LeftKind::Folder), ("Files and shortcuts", LeftKind::File)] {
                            let n = w.items.iter().filter(|i| i.kind == kind).count();
                            if n == 0 {
                                continue;
                            }
                            ui.label(RichText::new(format!("{title} ({n})")).strong());
                            for it in w.items.iter_mut().filter(|i| i.kind == kind) {
                                ui.horizontal(|ui| {
                                    ui.checkbox(&mut it.selected, "");
                                    ui.add(egui::Label::new(RichText::new(&it.path).monospace().size(12.0)).truncate());
                                    if it.size > 0 {
                                        ui.label(RichText::new(util::fmt_bytes(it.size as f64)).color(GREEN));
                                    }
                                    ui.label(RichText::new(&it.reason).color(c_dim()).size(11.5));
                                });
                            }
                            ui.add_space(4.0);
                        }
                    });
                }
                ui.horizontal(|ui| {
                    if ui.add_enabled(sel_count > 0, egui::Button::new(RichText::new(format!("🗑 Delete {sel_count} selected ({})", util::fmt_bytes(sel_size as f64))).strong().color(RED))).clicked() {
                        confirm_remove = Some((sel_count, w.program.name.clone()));
                    }
                    if ui.button("Finish without deleting").clicked() {
                        history_entry = Some(cleanup::HistoryEntry {
                            date: util::fmt_time(util::now_unix()),
                            program: w.program.name.clone(),
                            version: w.program.version.clone(),
                            publisher: w.program.publisher.clone(),
                            how: w.how.clone(),
                            leftovers_removed: 0,
                            freed: 0,
                        });
                        close = true;
                    }
                    ui.label(RichText::new("Registry keys are backed up before deletion (see Backups); files go to the Recycle Bin.").color(c_dim()).size(11.5));
                });
            }
            Stage::Removing => {
                card(ui, "", |ui| {
                    ui.horizontal(|ui| {
                        ui.spinner();
                        ui.label("Backing up and removing the selected leftovers…");
                    });
                });
            }
            Stage::Done => {
                if let Some(rep) = &w.report {
                    card(ui, "Finished", |ui| {
                        ui.label(RichText::new(format!("✔ Removed {} items, {} moved to the Recycle Bin.", rep.removed, util::fmt_bytes(rep.freed as f64))).color(GREEN).size(15.0));
                        if !rep.failed.is_empty() {
                            ui.label(RichText::new(format!("{} could not be removed:", rep.failed.len())).color(YELLOW));
                            for f in rep.failed.iter().take(20) {
                                ui.label(RichText::new(f).size(12.0).color(c_dim()));
                            }
                        }
                        ui.horizontal(|ui| {
                            if ui.button("📂 Open the backup").clicked() {
                                util::open_path(std::path::Path::new(&rep.backup));
                            }
                            if ui.button("Back to programs").clicked() {
                                close = true;
                            }
                        });
                    });
                }
            }
        }
    }
    if let Some(h) = history_entry {
        cleanup::add_history(h);
        app.clean.programs.started = false;
    }
    if let Some((n, name)) = confirm_remove {
        ask(app, "Delete leftovers?", format!("Delete {n} leftovers of {name}?\nRegistry keys are exported to a backup first; files and folders go to the Recycle Bin."), Pending::RemoveLeftovers);
    }
    if close {
        app.clean.wizard = None;
        app.clean.programs.started = false;
    }
}

fn forced(app: &mut App, ui: &mut Ui) {
    card(ui, "Find leftovers of a program", |ui| {
        ui.label(RichText::new("For programs that are already partly removed, won't uninstall, or don't appear in the list. Nothing is deleted until you choose.").color(c_dim()));
        egui::Grid::new("forced").num_columns(2).spacing([12.0, 8.0]).show(ui, |ui| {
            ui.label("Program name");
            ui.add(egui::TextEdit::singleline(&mut app.clean.forced_name).hint_text("e.g. Foo Player").desired_width(320.0));
            ui.end_row();
            ui.label("Publisher (optional)");
            ui.add(egui::TextEdit::singleline(&mut app.clean.forced_publisher).hint_text("e.g. Foo Software").desired_width(320.0));
            ui.end_row();
            ui.label("Install folder (optional)");
            ui.add(egui::TextEdit::singleline(&mut app.clean.forced_folder).hint_text("e.g. C:\\Program Files\\Foo").desired_width(320.0));
            ui.end_row();
            ui.label("Scan mode");
            ui.horizontal(|ui| {
                ui.selectable_value(&mut app.clean.level, Level::Safe, "Safe");
                ui.selectable_value(&mut app.clean.level, Level::Moderate, "Moderate");
                ui.selectable_value(&mut app.clean.level, Level::Advanced, "Advanced");
            });
            ui.end_row();
        });
        if ui.add_enabled(app.clean.forced_name.trim().len() >= 3, egui::Button::new(RichText::new("🔍 Scan for leftovers").strong())).clicked() {
            let p = Program {
                name: app.clean.forced_name.trim().to_string(),
                publisher: app.clean.forced_publisher.trim().to_string(),
                location: app.clean.forced_folder.trim().to_string(),
                key_path: format!("HKCU\\Software\\__none__\\{}", util::stamp()),
                ..Default::default()
            };
            start_wizard(app, p, true);
        }
    });
}

fn history(app: &mut App, ui: &mut Ui) {
    let h = cleanup::history();
    if h.is_empty() {
        notice(ui, "Programs you remove here are listed with the date and what was cleaned.", BLUE);
        return;
    }
    let freed: u64 = h.iter().map(|e| e.freed).sum();
    ui.label(RichText::new(format!("{} removals • {} of leftovers removed in total", h.len(), util::fmt_bytes(freed as f64))).color(c_dim()));
    egui_extras::TableBuilder::new(ui)
        .id_salt("uh")
        .striped(true)
        .resizable(true)
        .auto_shrink([false, false])
        .column(egui_extras::Column::initial(140.0))
        .column(egui_extras::Column::initial(280.0).clip(true))
        .column(egui_extras::Column::initial(110.0).clip(true))
        .column(egui_extras::Column::initial(140.0).clip(true))
        .column(egui_extras::Column::initial(120.0))
        .column(egui_extras::Column::remainder())
        .header(22.0, |mut hd| {
            for t in ["When", "Program", "Version", "How", "Leftovers removed", "Freed"] {
                hd.col(|ui| {
                    ui.add(egui::Label::new(RichText::new(t).strong()).truncate());
                });
            }
        })
        .body(|body| {
            body.rows(row_h(), h.len(), |mut row| {
                let e = &h[row.index()];
                for v in [e.date.clone(), e.program.clone(), e.version.clone(), e.how.clone(), e.leftovers_removed.to_string(), util::fmt_bytes(e.freed as f64)] {
                    row.col(|ui| {
                        ui.add(egui::Label::new(v).truncate());
                    });
                }
            });
        });
    let _ = app;
}

// ------------------------------------------------------------------ Windows apps

pub fn store_apps(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    poll_action(app);
    page_header(ui, "Windows apps", "Apps from the Microsoft Store and the ones that came with Windows. Remove the ones you don't use.");
    let all = app.clean.appx_all;
    app.clean.appx.ensure(ctx, move || cleanup::store_apps(all));
    ui.horizontal(|ui| {
        search_field(ui, &mut app.clean.appx_t.filter, "🔍 Filter apps…", 260.0);
        ui.checkbox(&mut app.clean.appx_system, "Show system parts and frameworks");
        if ui.add_enabled(app.admin(), egui::Checkbox::new(&mut app.clean.appx_all, "All users")).changed() {
            app.clean.appx.started = false;
        }
        if ui.button("⟳ Refresh").clicked() {
            app.clean.appx.started = false;
        }
        if app.clean.action.is_running() {
            ui.spinner();
        }
    });
    if !job_status(ui, &app.clean.appx) {
        return;
    }
    let mut rows = app.clean.appx.take_ok().unwrap_or_default();
    if !app.clean.appx_system {
        rows.retain(|r| jget(r, "IsFramework").as_bool() != Some(true) && jget(r, "NonRemovable").as_bool() != Some(true) && jstr(r, "Signature") != "System");
    }
    for r in rows.iter_mut() {
        let name = jstr(r, "Name");
        let friendly = name.split('.').skip(1).collect::<Vec<_>>().join(" ");
        let pubr = jstr(r, "Publisher");
        let cn = pubr.split(',').next().unwrap_or("").trim_start_matches("CN=").to_string();
        if let serde_json::Value::Object(o) = r {
            o.insert("Friendly".into(), serde_json::Value::String(if friendly.is_empty() { name } else { friendly }));
            o.insert("Maker".into(), serde_json::Value::String(cn));
        }
    }
    if let Some(r) = app.clean.appx_t.selected.and_then(|i| rows.get(i)).cloned() {
        card(ui, &jstr(&r, "Friendly"), |ui| {
            kv(ui, "appx_kv", &[("Package", jstr(&r, "PackageFullName")), ("Version", jstr(&r, "Version")), ("Publisher", jstr(&r, "Maker")), ("Folder", jstr(&r, "InstallLocation"))]);
            ui.horizontal(|ui| {
                let removable = jget(&r, "NonRemovable").as_bool() != Some(true);
                if ui.add_enabled(removable, egui::Button::new(RichText::new("🗑 Uninstall app").color(RED))).clicked() {
                    let full = jstr(&r, "PackageFullName");
                    let name = jstr(&r, "Friendly");
                    ask(app, "Uninstall app?", format!("Remove {name}{}?", if app.clean.appx_all { " for every user" } else { "" }), Pending::RemoveAppx { full, name });
                }
                let loc = jstr(&r, "InstallLocation");
                if !loc.is_empty() && ui.button("📂 Folder").clicked() {
                    crate::sys::shell_open("open", &loc, "");
                }
            });
        });
    }
    let cols = [col("Friendly", "App", 260.0), col("Version", "Version", 120.0), col("Architecture", "Type", 70.0), col("Maker", "Publisher", 220.0), col("PackageFullName", "Package", 400.0)];
    json_table(ui, "appx", &rows, &cols, &mut app.clean.appx_t, &|_| None, f32::INFINITY);
}

// ------------------------------------------------------------------ browser extensions

pub fn extensions(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    page_header(ui, "Browser extensions", "Add-ons installed in Chrome, Edge, Brave, Vivaldi, Opera and Firefox — including ones bundled with other software.");
    app.clean.exts.ensure(ctx, || Ok(cleanup::extensions()));
    ui.horizontal(|ui| {
        search_field(ui, &mut app.clean.ext_filter, "🔍 Filter extensions…", 260.0);
        if ui.button("⟳ Refresh").clicked() {
            app.clean.exts.started = false;
        }
    });
    notice(ui, "Close the browser before removing an extension. If you sign in to sync, the browser may bring it back; remove it in the browser's own extensions page too.", BLUE);
    if !job_status(ui, &app.clean.exts) {
        return;
    }
    let list = app.clean.exts.take_ok().unwrap_or_default();
    let f = app.clean.ext_filter.to_lowercase();
    let mut remove = None;
    egui::ScrollArea::vertical().id_salt("exts").auto_shrink([false, false]).show(ui, |ui| {
        let mut last = "";
        for e in list.iter().filter(|e| f.is_empty() || e.name.to_lowercase().contains(&f) || e.browser.to_lowercase().contains(&f) || e.id.contains(&f)) {
            if e.browser != last {
                last = e.browser;
                ui.add_space(6.0);
                ui.label(RichText::new(e.browser).size(16.0).strong());
            }
            egui::Frame::none().fill(c_card()).rounding(8.0).inner_margin(egui::Margin::same(8.0)).show(ui, |ui| {
                ui.set_width(ui.available_width());
                ui.horizontal(|ui| {
                    ui.label(RichText::new(&e.name).strong());
                    ui.label(RichText::new(&e.version).color(c_dim()));
                    pill(ui, &e.profile, BLUE);
                    match e.enabled {
                        Some(true) => pill(ui, "On", GREEN),
                        Some(false) => pill(ui, "Off", c_dim()),
                        None => {}
                    }
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        if ui.button(RichText::new("🗑 Remove").color(RED)).clicked() {
                            remove = Some(e.clone());
                        }
                        if ui.button("📂").on_hover_text("Show its files").clicked() {
                            crate::sys::open_file_location(&e.path);
                        }
                    });
                });
                if !e.description.is_empty() {
                    ui.add(egui::Label::new(RichText::new(&e.description).color(c_dim()).size(12.0)).truncate());
                }
                ui.label(RichText::new(&e.id).monospace().size(10.5).color(c_dim()));
            });
            ui.add_space(3.0);
        }
        if list.is_empty() {
            ui.label("No browser extensions found.");
        }
    });
    if let Some(e) = remove {
        ask(app, "Remove extension?", format!("Remove \"{}\" from {} ({})?\nIts files go to the Recycle Bin.", e.name, e.browser, e.profile), Pending::RemoveExtension(e));
    }
}

// ------------------------------------------------------------------ junk cleaner

pub fn junk(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    poll_action(app);
    page_header(ui, "Junk files cleaner", "Temporary files, caches, error reports and crash dumps that are safe to delete.");
    let older = if app.clean.junk_older { 86400 } else { 0 };
    let cats = app.clean.junk.clone();
    app.clean.junk_scan.ensure(ctx, move || Ok((cats.iter().map(|c| (c.key, cleanup::scan_junk(c, older))).collect(), cleanup::recycle_bin())));
    ui.horizontal(|ui| {
        if ui.checkbox(&mut app.clean.junk_older, "Only files older than one day").changed() {
            app.clean.junk_scan.started = false;
        }
        if ui.add_enabled(!app.clean.junk_scan.is_running(), egui::Button::new("⟳ Scan again")).clicked() {
            app.clean.junk_scan.started = false;
        }
        if app.clean.action.is_running() {
            ui.spinner();
            ui.label("Cleaning…");
        }
    });
    app.admin_hint(ui, "Windows' own folders (Windows temp, update downloads, delivery cache, setup logs) can only be cleaned in the Admin view.");
    if !job_status(ui, &app.clean.junk_scan) {
        return;
    }
    let Some((scans, (rb_bytes, rb_items))) = app.clean.junk_scan.take_ok() else { return };
    let admin = app.elevated;
    let mut total = 0u64;
    card(ui, "", |ui| {
        for c in app.clean.junk.clone() {
            let s = scans.iter().find(|x| x.0 == c.key).map(|x| x.1.clone()).unwrap_or_default();
            let usable = !c.admin || admin;
            ui.horizontal(|ui| {
                let mut on = app.clean.junk_sel.contains(c.key) && usable;
                if ui.add_enabled(usable, egui::Checkbox::new(&mut on, "")).changed() {
                    if on {
                        app.clean.junk_sel.insert(c.key);
                    } else {
                        app.clean.junk_sel.remove(c.key);
                    }
                }
                fixed(ui, 240.0, RichText::new(c.name).strong());
                fixed(ui, 110.0, RichText::new(util::fmt_bytes(s.bytes as f64)).color(if s.bytes > 100 << 20 { ORANGE } else { GREEN }));
                fixed(ui, 110.0, RichText::new(format!("{} files", util::fmt_num(s.files))).color(c_dim()));
                ui.label(RichText::new(c.what).color(c_dim()).size(12.0));
                if c.admin && !admin {
                    ui.label(RichText::new("🛡").color(PURPLE));
                } else if s.denied {
                    ui.label(RichText::new("partly unreadable").color(YELLOW).size(11.0));
                }
            });
            if app.clean.junk_sel.contains(c.key) && usable {
                total += s.bytes;
            }
        }
        ui.separator();
        ui.horizontal(|ui| {
            ui.checkbox(&mut app.clean.junk_bin, "");
            fixed(ui, 240.0, RichText::new("Recycle Bin").strong());
            fixed(ui, 110.0, RichText::new(util::fmt_bytes(rb_bytes as f64)).color(GREEN));
            fixed(ui, 110.0, RichText::new(format!("{} items", util::fmt_num(rb_items))).color(c_dim()));
            ui.label(RichText::new("Deleted files waiting to be thrown away for good").color(c_dim()).size(12.0));
        });
    });
    if app.clean.junk_bin {
        total += rb_bytes;
    }
    ui.add_space(6.0);
    ui.horizontal(|ui| {
        if ui.add_enabled(total > 0 && !app.clean.action.is_running(), egui::Button::new(RichText::new(format!("🧹 Clean selected — free about {}", util::fmt_bytes(total as f64))).strong().color(GREEN))).clicked() {
            ask(app, "Clean junk files?", format!("Permanently delete about {} of junk?\nFiles in use are skipped automatically.", util::fmt_bytes(total as f64)), Pending::CleanJunk);
        }
        if ui.button("Open Windows Disk Cleanup").on_hover_text("For old Windows installations and system restore space").clicked() {
            cleanup::open_tool("cleanmgr.exe");
        }
    });
}

// ------------------------------------------------------------------ privacy cleaner

pub fn privacy(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    poll_action(app);
    page_header(ui, "Privacy cleaner", "Remove traces of what you opened, typed and searched in Windows.");
    app.clean.traces.ensure(ctx, || Ok(cleanup::traces()));
    if ui.button("⟳ Refresh").clicked() {
        app.clean.traces.started = false;
    }
    if !job_status(ui, &app.clean.traces) {
        return;
    }
    let list = app.clean.traces.take_ok().unwrap_or_default();
    card(ui, "", |ui| {
        for t in &list {
            ui.horizontal(|ui| {
                let mut on = app.clean.trace_sel.contains(t.key);
                if ui.checkbox(&mut on, "").changed() {
                    if on {
                        app.clean.trace_sel.insert(t.key);
                    } else {
                        app.clean.trace_sel.remove(t.key);
                    }
                }
                fixed(ui, 260.0, RichText::new(t.name).strong());
                if !matches!(t.key, "clipboard" | "dns") {
                    fixed(ui, 90.0, RichText::new(format!("{} items", t.count)).color(if t.count > 0 { ORANGE } else { c_dim() }));
                } else {
                    ui.add_space(98.0);
                }
                ui.label(RichText::new(t.what).color(c_dim()).size(12.0));
            });
        }
    });
    ui.label(RichText::new("Browser history is cleared from inside each browser (Ctrl+Shift+Delete); this app does not touch browser profiles.").color(c_dim()).size(12.0));
    ui.horizontal(|ui| {
        if ui.add_enabled(!app.clean.trace_sel.is_empty() && !app.clean.action.is_running(), egui::Button::new(RichText::new("🧹 Clean selected").strong().color(GREEN))).clicked() {
            ask(app, "Clean traces?", format!("Clear {} selected kinds of history? This can't be undone.", app.clean.trace_sel.len()), Pending::CleanTraces);
        }
        if app.clean.action.is_running() {
            ui.spinner();
        }
    });
}

// ------------------------------------------------------------------ secure delete

pub fn secure_delete(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    page_header(ui, "Secure delete", "Overwrite files before deleting them, so recovery tools can't bring them back.");
    if app.clean.browse.started && !app.clean.browse.is_running() {
        if let Some(s) = app.clean.browse.take_ok() {
            for l in s.lines().map(|l| l.trim()).filter(|l| !l.is_empty()) {
                if !app.clean.shred_paths.iter().any(|p| p == l) {
                    app.clean.shred_paths.push(l.to_string());
                }
            }
        }
        app.clean.browse = Job::default();
    }
    notice(ui, "⚠ Deleted files can't be recovered — not even from the Recycle Bin. On SSDs and USB sticks the drive itself may keep hidden copies; for those, full-disk encryption (BitLocker) is the reliable protection.", YELLOW);
    ui.horizontal(|ui| {
        ui.add(egui::TextEdit::singleline(&mut app.clean.shred_input).hint_text("Paste a file or folder path").desired_width(360.0));
        if ui.button("Add").clicked() && !app.clean.shred_input.trim().is_empty() {
            let p = app.clean.shred_input.trim().trim_matches('"').to_string();
            app.clean.shred_paths.push(p);
            app.clean.shred_input.clear();
        }
        if ui.add_enabled(!app.clean.browse.is_running(), egui::Button::new("📄 Add files…")).clicked() {
            app.clean.browse.start(ctx, || {
                util::powershell("Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.OpenFileDialog; $d.Multiselect = $true; $d.Title = 'Files to delete for good'; if ($d.ShowDialog() -eq 'OK') { $d.FileNames -join \"`n\" }")
            });
        }
        if ui.add_enabled(!app.clean.browse.is_running(), egui::Button::new("📁 Add folder…")).clicked() {
            app.clean.browse.start(ctx, || {
                util::powershell("Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = 'Folder to delete for good'; if ($d.ShowDialog() -eq 'OK') { $d.SelectedPath }")
            });
        }
    });
    let mut remove = None;
    card(ui, &format!("To delete ({})", app.clean.shred_paths.len()), |ui| {
        if app.clean.shred_paths.is_empty() {
            ui.label(RichText::new("Nothing added yet.").color(c_dim()));
        }
        for (i, p) in app.clean.shred_paths.iter().enumerate() {
            ui.horizontal(|ui| {
                if ui.small_button("✖").clicked() {
                    remove = Some(i);
                }
                let exists = std::path::Path::new(p).exists();
                ui.label(RichText::new(p).monospace().color(if exists { c_text() } else { RED }));
                if !exists {
                    ui.label(RichText::new("not found").color(RED).size(11.0));
                }
            });
        }
    });
    if let Some(i) = remove {
        app.clean.shred_paths.remove(i);
    }
    ui.horizontal(|ui| {
        ui.label("Overwrite passes:");
        ui.selectable_value(&mut app.clean.shred_passes, 1, "1 (zeros)");
        ui.selectable_value(&mut app.clean.shred_passes, 3, "3 (zeros, ones, random)");
        ui.selectable_value(&mut app.clean.shred_passes, 7, "7");
    });
    if ui.add_enabled(!app.clean.shred_paths.is_empty() && !app.clean.shred_job.is_running(), egui::Button::new(RichText::new("🔥 Delete for good").strong().color(RED))).clicked() {
        ask(app, "Delete for good?", format!("Overwrite and delete {} item(s)? They cannot be recovered.", app.clean.shred_paths.len()), Pending::Shred);
    }
    if app.clean.shred_job.is_running() {
        ui.horizontal(|ui| {
            ui.spinner();
            ui.label("Overwriting…");
        });
    }
    app.clean.shred_job.with(|r| {
        if let Some(Ok((files, bytes, errors))) = r {
            notice(ui, &format!("✔ {} files ({}) overwritten and deleted.", util::fmt_num(*files), util::fmt_bytes(*bytes as f64)), GREEN);
            for e in errors.iter().take(20) {
                ui.label(RichText::new(e).color(YELLOW).size(12.0));
            }
        }
    });
}

// ------------------------------------------------------------------ backups & restore points

pub fn backups(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    poll_action(app);
    page_header(ui, "Backups & restore points", "Registry backups made before leftovers were removed, and Windows restore points.");
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        ui.label(RichText::new("Registry backups").size(16.0).strong());
        let list = cleanup::backups();
        if list.is_empty() {
            ui.label(RichText::new("None yet. One is made every time leftovers are removed.").color(c_dim()));
        }
        let mut restore = None;
        egui::ScrollArea::vertical().id_salt("bk").auto_shrink([false, false]).show(ui, |ui| {
            for (dir, m) in &list {
                egui::Frame::none().fill(c_card()).rounding(8.0).inner_margin(egui::Margin::same(8.0)).show(ui, |ui| {
                    ui.set_width(ui.available_width());
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(&m.program).strong());
                        ui.label(RichText::new(&m.date).color(c_dim()));
                    });
                    ui.label(RichText::new(format!("{} registry keys, {} files/folders sent to the Recycle Bin", m.keys.len(), m.files.len())).size(12.0));
                    ui.horizontal(|ui| {
                        if !m.reg_files.is_empty() && ui.button("⟲ Restore registry keys").clicked() {
                            restore = Some((dir.clone(), m.program.clone(), m.keys.len()));
                        }
                        if ui.button("📂 Open").clicked() {
                            util::open_path(dir);
                        }
                    });
                });
                ui.add_space(3.0);
            }
        });
        if let Some((dir, prog, n)) = restore {
            ask(app, "Restore backup?", format!("Put back {n} registry keys removed with {prog}?"), Pending::RestoreBackup(dir));
        }
        let ui = &mut cols[1];
        ui.label(RichText::new("Windows restore points").size(16.0).strong());
        ui.horizontal(|ui| {
            if ui.add_enabled(app.admin() && !app.clean.action.is_running(), egui::Button::new("➕ Create restore point")).clicked() {
                app.clean.action = Job::default();
                app.clean.action.start(ctx, || cleanup::create_restore_point("Advanced TaskManager manual restore point"));
                app.clean.points = Job::default();
            }
            if ui.button("Open System Restore").clicked() {
                cleanup::open_tool("rstrui.exe");
            }
            if ui.button("⟳").clicked() {
                app.clean.points = Job::default();
            }
        });
        if !app.admin() {
            ui.label(RichText::new("🛡 Listing and creating restore points needs the Admin view.").color(PURPLE));
            return;
        }
        app.clean.points.ensure(ctx, cleanup::restore_points);
        if job_status(ui, &app.clean.points) {
            let pts = app.clean.points.take_ok().unwrap_or_default();
            if pts.is_empty() {
                ui.label(RichText::new("No restore points (System Protection may be off for drive C:).").color(c_dim()));
            }
            for p in pts.iter().rev() {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(jstr(p, "Created")).monospace());
                    pill(ui, &jstr(p, "Type"), BLUE);
                    ui.label(jstr(p, "Description"));
                });
            }
        }
    });
}

// ------------------------------------------------------------------ Windows tools

pub fn tools(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Windows tools", "Windows' built-in tools, one click away.");
    let w = ui.available_width();
    let per = ((w / 280.0).floor() as usize).clamp(1, 5);
    let cw = w / per as f32 - 10.0;
    egui::ScrollArea::vertical().id_salt("tools").auto_shrink([false, false]).show(ui, |ui| {
        egui::Grid::new("tools_grid").spacing([10.0, 10.0]).show(ui, |ui| {
            for (i, (icon, name, cmd, what)) in cleanup::TOOLS.iter().enumerate() {
                let r = egui::Frame::none().fill(c_card()).rounding(10.0).stroke(egui::Stroke::new(1.0, c_border())).inner_margin(egui::Margin::same(10.0)).show(ui, |ui| {
                    ui.set_width(cw - 20.0);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(*icon).size(22.0).color(accent()));
                        ui.vertical(|ui| {
                            ui.label(RichText::new(*name).strong());
                            ui.label(RichText::new(*what).color(c_dim()).size(11.5));
                        });
                    });
                });
                if r.response.interact(Sense::click()).on_hover_cursor(egui::CursorIcon::PointingHand).clicked() && !cleanup::open_tool(cmd) {
                    app.notify(format!("Could not open {name}"), true);
                }
                if (i + 1) % per == 0 {
                    ui.end_row();
                }
            }
        });
    });
}
