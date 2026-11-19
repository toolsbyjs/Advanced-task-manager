//! Disk space analyzer page: scan a drive or folder, then explore it as a folder tree,
//! a largest-files list, file types, duplicates and an interactive treemap.

use super::widgets::*;
use super::{App, Confirm};
use crate::disk::{self, DupGroup, Progress, Tree, NONE};
use crate::util::{self, Job};
use egui::{Align2, Color32, FontId, Pos2, Rect, RichText, Sense, Stroke, Ui, Vec2};
use std::collections::HashSet;
use std::sync::atomic::Ordering;
use std::sync::{Arc, Mutex};
use std::time::Instant;

const FREE: u32 = u32::MAX - 1;

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Unit {
    Auto,
    B,
    KB,
    MB,
    GB,
    TB,
}

impl Unit {
    const ALL: [Unit; 6] = [Unit::Auto, Unit::B, Unit::KB, Unit::MB, Unit::GB, Unit::TB];
    fn label(&self) -> &'static str {
        match self {
            Unit::Auto => "Auto",
            Unit::B => "Bytes",
            Unit::KB => "KB",
            Unit::MB => "MB",
            Unit::GB => "GB",
            Unit::TB => "TB",
        }
    }
    fn fmt(&self, v: u64) -> String {
        let f = v as f64;
        match self {
            Unit::Auto => util::fmt_bytes(f),
            Unit::B => util::fmt_num(v),
            Unit::KB => format!("{:.1} KB", f / 1024.0),
            Unit::MB => format!("{:.1} MB", f / 1048576.0),
            Unit::GB => format!("{:.2} GB", f / 1073741824.0),
            Unit::TB => format!("{:.3} TB", f / 1099511627776.0),
        }
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum DTab {
    Folders,
    Files,
    Types,
    Duplicates,
}

pub fn tab_from_name(n: &str) -> DTab {
    match n {
        "files" => DTab::Files,
        "types" => DTab::Types,
        "duplicates" => DTab::Duplicates,
        _ => DTab::Folders,
    }
}

pub struct Run {
    prog: Arc<Progress>,
    result: Arc<Mutex<Option<Result<Tree, String>>>>,
    started: Instant,
    target: String,
}

pub struct DiskUi {
    pub target: String,
    pub run: Option<Run>,
    pub tree: Option<Arc<Tree>>,
    pub version: u64,
    pub tab: DTab,
    pub unit: Unit,
    pub use_alloc: bool,
    pub expanded: HashSet<u32>,
    pub selected: Option<u32>,
    pub rows: Option<(u64, u64, Vec<(u32, u16)>)>,
    pub rows_ver: u64,
    pub scroll_to: bool,
    pub filter: String,
    pub match_path: bool,
    pub include: String,
    pub exclude: String,
    pub limit: usize,
    pub files_cache: Option<(String, Vec<u32>, usize, u64)>,
    pub dup_min_mb: f32,
    pub dup_date: bool,
    pub dups: Job<Vec<DupGroup>>,
    pub dup_for: u64,
    pub tm_show: bool,
    pub tm_root: u32,
    pub tm_names: bool,
    pub tm_free: bool,
    pub tm_shade: bool,
    pub tm_cb: bool,
    pub tm_depth: u32,
    pub tm_cache: Option<(String, Vec<(u32, Rect, u8)>)>,
    pub browse: Job<String>,
    pub last_error: Option<String>,
    pub autostart: bool,
}

impl Default for DiskUi {
    fn default() -> Self {
        Self {
            target: "C:\\".into(),
            run: None,
            tree: None,
            version: 0,
            tab: DTab::Folders,
            unit: Unit::Auto,
            use_alloc: false,
            expanded: HashSet::new(),
            selected: None,
            rows: None,
            rows_ver: 0,
            scroll_to: false,
            filter: String::new(),
            match_path: false,
            include: String::new(),
            exclude: String::new(),
            limit: 1000,
            files_cache: None,
            dup_min_mb: 1.0,
            dup_date: false,
            dups: Job::default(),
            dup_for: 0,
            tm_show: true,
            tm_root: 0,
            tm_names: true,
            tm_free: false,
            tm_shade: true,
            tm_cb: false,
            tm_depth: 12,
            tm_cache: None,
            browse: Job::default(),
            last_error: None,
            autostart: false,
        }
    }
}

fn sz(n: &disk::Node, alloc: bool) -> u64 {
    if alloc {
        n.alloc
    } else {
        n.size
    }
}

fn start_scan(app: &mut App, ctx: &egui::Context) {
    if app.disk.run.is_some() {
        return;
    }
    let target = app.disk.target.trim().to_string();
    let prog = Arc::new(Progress::default());
    let result = Arc::new(Mutex::new(None));
    let (p2, r2, t2, c2) = (prog.clone(), result.clone(), target.clone(), ctx.clone());
    let elevated = app.elevated;
    std::thread::Builder::new()
        .name("disk-scan".into())
        .spawn(move || {
            let r = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| disk::scan(&t2, elevated, &p2)))
                .unwrap_or_else(|_| Err("The scan stopped unexpectedly. Try again, or scan a folder instead of the whole drive.".into()));
            *r2.lock().unwrap() = Some(r);
            c2.request_repaint();
        })
        .ok();
    app.disk.last_error = None;
    app.disk.run = Some(Run { prog, result, started: Instant::now(), target });
}

fn poll_scan(app: &mut App) {
    let done = match &app.disk.run {
        Some(r) => r.result.lock().unwrap().take(),
        None => return,
    };
    let Some(res) = done else { return };
    let run = app.disk.run.take().unwrap();
    match res {
        Ok(tree) => {
            let msg = format!("Scanned {} in {:.1} s ({}): {} files, {} folders, {}", run.target, tree.secs, tree.method, util::fmt_num(tree.nodes[0].files as u64), util::fmt_num(tree.nodes[0].dirs as u64), util::fmt_bytes(tree.nodes[0].size as f64));
            app.shared.lock().unwrap().log("Disk", crate::collector::Severity::Info, msg.clone(), None);
            let d = &mut app.disk;
            d.expanded.clear();
            d.expanded.insert(0);
            d.selected = Some(0);
            d.tm_root = 0;
            d.tm_cache = None;
            d.files_cache = None;
            d.dups = Job::default();
            d.rows = None;
            d.tree = Some(Arc::new(tree));
            d.version += 1;
            app.notify(msg, false);
        }
        Err(e) => {
            app.disk.last_error = Some(e.clone());
            app.notify(e, true);
        }
    }
}

/// Called after the person confirmed: move to the Recycle Bin and update the tree.
pub fn recycle_node(app: &mut App, idx: u32, path: String) {
    match disk::recycle(&path) {
        Ok(()) => {
            let mut updated = false;
            if let Some(t) = app.disk.tree.as_mut() {
                if let Some(tm) = Arc::get_mut(t) {
                    tm.remove(idx);
                    updated = true;
                }
            }
            let d = &mut app.disk;
            d.version += 1;
            d.tm_cache = None;
            d.files_cache = None;
            d.rows = None;
            if d.selected == Some(idx) {
                d.selected = None;
            }
            if d.tm_root == idx {
                d.tm_root = 0;
            }
            app.shared.lock().unwrap().log("Disk", crate::collector::Severity::Info, format!("Moved to Recycle Bin: {path}"), None);
            app.notify(if updated { format!("Moved to Recycle Bin: {path}") } else { "Moved to Recycle Bin — scan again to refresh the totals".to_string() }, false);
        }
        Err(e) => app.notify(e, true),
    }
}

fn reveal(d: &mut DiskUi, t: &Tree, i: u32) {
    for a in t.ancestors(i).into_iter().skip(1) {
        d.expanded.insert(a);
    }
    d.selected = Some(i);
    d.rows_ver += 1;
    d.scroll_to = true;
}

#[derive(Clone, Copy)]
enum Act {
    Explore,
    Open,
    CopyPath,
    CopyInfo,
    Cmd,
    ShowInTree,
    ShowInMap,
    Recycle,
}

fn item_menu(ui: &mut Ui, is_dir: bool) -> Option<Act> {
    let mut a = None;
    if ui.button("📂 Show in Explorer").clicked() {
        a = Some(Act::Explore);
    }
    if ui.button(if is_dir { "📂 Open folder" } else { "📄 Open file" }).clicked() {
        a = Some(Act::Open);
    }
    if ui.button("☰ Command prompt here").clicked() {
        a = Some(Act::Cmd);
    }
    ui.separator();
    if ui.button("📋 Copy path").clicked() {
        a = Some(Act::CopyPath);
    }
    if ui.button("📋 Copy name and size").clicked() {
        a = Some(Act::CopyInfo);
    }
    ui.separator();
    if ui.button("🌳 Show in folder tree").clicked() {
        a = Some(Act::ShowInTree);
    }
    if is_dir && ui.button("💠 Show in treemap").clicked() {
        a = Some(Act::ShowInMap);
    }
    ui.separator();
    if ui.button(RichText::new("🗑 Move to Recycle Bin").color(RED)).clicked() {
        a = Some(Act::Recycle);
    }
    if a.is_some() {
        ui.close_menu();
    }
    a
}

fn do_act(app: &mut App, ctx: &egui::Context, t: &Tree, i: u32, a: Act) {
    let path = t.path(i);
    let n = &t.nodes[i as usize];
    match a {
        Act::Explore => crate::sys::open_file_location(&path),
        Act::Open => {
            crate::sys::shell_open("open", &path, "");
        }
        Act::CopyPath => {
            ctx.output_mut(|o| o.copied_text = path);
            app.notify("Path copied", false);
        }
        Act::CopyInfo => {
            ctx.output_mut(|o| o.copied_text = format!("{}\t{}\t{} bytes\t{}", n.name, util::fmt_bytes(n.size as f64), n.size, path));
            app.notify("Copied", false);
        }
        Act::Cmd => {
            let dir = if n.is_dir { path } else { t.path(n.parent) };
            use std::os::windows::process::CommandExt;
            let _ = std::process::Command::new("cmd.exe").args(["/K", "cd", "/d", &dir]).creation_flags(0x0000_0010).spawn();
        }
        Act::ShowInTree => {
            app.disk.tab = DTab::Folders;
            reveal(&mut app.disk, t, i);
        }
        Act::ShowInMap => {
            app.disk.tm_root = i;
            app.disk.tm_show = true;
        }
        Act::Recycle => {
            if n.parent == NONE {
                app.notify("The scanned drive or folder itself can't be deleted here.", true);
            } else {
                app.confirm = Some(Confirm::Recycle(path, i));
            }
        }
    }
}

pub fn page(app: &mut App, ui: &mut Ui) {
    let ctx = ui.ctx().clone();
    poll_scan(app);
    if app.disk.autostart {
        app.disk.autostart = false;
        start_scan(app, &ctx);
    }
    if app.disk.browse.started && !app.disk.browse.is_running() {
        if let Some(p) = app.disk.browse.take_ok() {
            if !p.trim().is_empty() {
                app.disk.target = p.trim().to_string();
            }
        }
        app.disk.browse = Job::default();
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new("Disk space analyzer").size(22.0).strong());
        ui.label(RichText::new("Find what fills your drives — folders, biggest files, file types and duplicates.").color(c_dim()));
    });
    ui.add_space(4.0);
    toolbar(app, ui, &ctx);
    if !app.elevated {
        notice(ui, "🛡 Tip: in the Admin view whole NTFS drives are scanned in seconds by reading the drive's file table directly. Here folders are walked one by one, which is slower and skips folders you can't open.", PURPLE);
    }
    if let Some(run) = &app.disk.run {
        let p = &run.prog;
        let phase = p.phase.lock().unwrap().clone();
        card(ui, "", |ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(RichText::new(format!("Scanning {}", run.target)).strong());
                ui.label(RichText::new(phase).color(c_dim()));
            });
            ui.label(format!(
                "{} entries • {} folders read • {} found • {:.1} s",
                util::fmt_num(p.files.load(Ordering::Relaxed)),
                util::fmt_num(p.dirs.load(Ordering::Relaxed)),
                util::fmt_bytes(p.bytes.load(Ordering::Relaxed) as f64),
                run.started.elapsed().as_secs_f32()
            ));
        });
        ctx.request_repaint_after(std::time::Duration::from_millis(150));
    }
    if let Some(e) = &app.disk.last_error {
        notice(ui, e, RED);
    }
    let Some(tree) = app.disk.tree.clone() else {
        if app.disk.run.is_none() {
            ui.add_space(30.0);
            ui.vertical_centered(|ui| {
                ui.label(RichText::new("📁").size(48.0).color(accent()));
                ui.label(RichText::new("Pick a drive or folder above and press Scan").size(17.0));
                ui.label(RichText::new("You'll see every folder's size, the biggest files, which file types use the most space and a map of the whole drive.").color(c_dim()));
            });
        }
        return;
    };
    summary(app, ui, &tree);
    ui.horizontal(|ui| {
        for (t, l) in [(DTab::Folders, "🌳 Folders"), (DTab::Files, "📄 Largest files"), (DTab::Types, "📊 File types"), (DTab::Duplicates, "📋 Duplicates")] {
            ui.selectable_value(&mut app.disk.tab, t, l);
        }
        ui.separator();
        ui.toggle_value(&mut app.disk.tm_show, "💠 Treemap");
    });
    if app.disk.tm_show {
        egui::TopBottomPanel::bottom("disk_treemap")
            .resizable(true)
            .default_height(330.0)
            .min_height(140.0)
            .frame(egui::Frame::none().fill(c_panel()).inner_margin(egui::Margin::same(8.0)).rounding(8.0))
            .show_inside(ui, |ui| treemap(app, ui, &tree, &ctx));
    }
    let r = ui.available_rect_before_wrap();
    ui.allocate_ui_at_rect(r, |ui| {
        ui.set_clip_rect(r.intersect(ui.clip_rect()));
        match app.disk.tab {
            DTab::Folders => folders(app, ui, &tree, &ctx),
            DTab::Files => files(app, ui, &tree, &ctx),
            DTab::Types => types(app, ui, &tree),
            DTab::Duplicates => duplicates(app, ui, &tree, &ctx),
        }
    });
}

fn toolbar(app: &mut App, ui: &mut Ui, ctx: &egui::Context) {
    let vols: Vec<crate::sys::Volume> = app.shared.lock().unwrap().volumes.clone();
    ui.horizontal_wrapped(|ui| {
        for v in vols.iter().filter(|v| v.total > 0) {
            let root = v.root.trim_end_matches('\\').to_string() + "\\";
            let sel = app.disk.target.eq_ignore_ascii_case(&root);
            let used = 1.0 - v.free as f32 / v.total as f32;
            let label = format!("💽 {}  {} free", root.trim_end_matches('\\'), util::fmt_bytes(v.free as f64));
            if ui
                .add(egui::SelectableLabel::new(sel, label))
                .on_hover_text(format!("{} {} — {:.0}% used of {}", v.label, v.fs, used * 100.0, util::fmt_bytes(v.total as f64)))
                .clicked()
            {
                app.disk.target = root;
            }
        }
        ui.separator();
        ui.add(egui::TextEdit::singleline(&mut app.disk.target).hint_text("Drive or folder, e.g. C:\\ or D:\\Projects").desired_width(260.0));
        if ui.add_enabled(!app.disk.browse.is_running(), egui::Button::new("📂 Browse…")).clicked() {
            app.disk.browse.start(ctx, || {
                util::powershell("Add-Type -AssemblyName System.Windows.Forms; $d = New-Object System.Windows.Forms.FolderBrowserDialog; $d.Description = 'Pick a folder to analyse'; $d.ShowNewFolderButton = $false; if ($d.ShowDialog() -eq 'OK') { $d.SelectedPath }")
            });
        }
        if app.disk.run.is_some() {
            if ui.button(RichText::new("■ Cancel").color(RED)).clicked() {
                if let Some(r) = &app.disk.run {
                    r.prog.cancel.store(true, Ordering::Relaxed);
                }
            }
        } else if ui.button(RichText::new("▶ Scan").strong().color(GREEN)).clicked() {
            start_scan(app, ctx);
        }
        ui.separator();
        ui.label("Show sizes in");
        egui::ComboBox::from_id_salt("disk_unit").selected_text(app.disk.unit.label()).width(70.0).show_ui(ui, |ui| {
            for u in Unit::ALL {
                ui.selectable_value(&mut app.disk.unit, u, u.label());
            }
        });
        if ui
            .checkbox(&mut app.disk.use_alloc, "Space on disk")
            .on_hover_text("Use the space files actually take on the drive (clusters, compression) instead of their length")
            .changed()
        {
            app.disk.tm_cache = None;
            app.disk.files_cache = None;
        }
    });
}

fn summary(app: &mut App, ui: &mut Ui, t: &Tree) {
    let root = &t.nodes[0];
    let u = app.disk.unit;
    ui.horizontal_wrapped(|ui| {
        pill(ui, &format!("{} files", util::fmt_num(root.files as u64)), BLUE);
        pill(ui, &format!("{} folders", util::fmt_num(root.dirs as u64)), BLUE);
        pill(ui, &format!("{} in files", u.fmt(root.size)), GREEN);
        pill(ui, &format!("{} on disk", u.fmt(root.alloc)), TEAL);
        pill(ui, &format!("{} • {:.2} s", t.method, t.secs), if t.method.starts_with("Fast") { PURPLE } else { c_dim() });
        if t.errors > 0 {
            pill(ui, &format!("{} folders could not be opened", util::fmt_num(t.errors)), YELLOW);
        }
        if disk::is_drive_root(&t.root_path).is_some() && t.total_space > 0 {
            let used = t.total_space.saturating_sub(t.free_space);
            ui.label(RichText::new(format!("Drive: {} used, {} free of {}", u.fmt(used), u.fmt(t.free_space), u.fmt(t.total_space))).color(c_dim()));
            let f = used as f32 / t.total_space as f32;
            bar(ui, f, load_color(f * 100.0), &format!("{:.0}% used", f * 100.0), 160.0);
            let unaccounted = used.saturating_sub(root.alloc);
            if unaccounted > t.total_space / 50 {
                ui.label(RichText::new(format!("{} used by Windows outside files (system areas, snapshots, or folders that couldn't be read)", u.fmt(unaccounted))).color(c_dim()).size(11.5));
            }
        }
    });
    ui.add_space(4.0);
}

// ------------------------------------------------------------------ folders

fn folders(app: &mut App, ui: &mut Ui, t: &Tree, ctx: &egui::Context) {
    let d = &mut app.disk;
    let key = (t.id, d.version + d.rows_ver);
    if d.rows.as_ref().map(|r| (r.0, r.1) != key).unwrap_or(true) {
        let mut rows = Vec::new();
        let mut stack = vec![(0u32, 0u16)];
        while let Some((i, depth)) = stack.pop() {
            rows.push((i, depth));
            if d.expanded.contains(&i) {
                for &c in t.nodes[i as usize].children.iter().rev() {
                    stack.push((c, depth + 1));
                }
            }
        }
        d.rows = Some((key.0, key.1, rows));
    }
    let rows = d.rows.as_ref().unwrap().2.clone();
    let alloc = d.use_alloc;
    let unit = d.unit;
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} rows • click ⏵ or double-click to open a folder", rows.len())).color(c_dim()).size(12.0));
        if ui.small_button("Collapse all").clicked() {
            app.disk.expanded.clear();
            app.disk.expanded.insert(0);
            app.disk.rows_ver += 1;
        }
        if ui.small_button("Open biggest path").on_hover_text("Unfold the largest folder at each level").clicked() {
            let mut i = 0u32;
            loop {
                app.disk.expanded.insert(i);
                match t.nodes[i as usize].children.iter().find(|c| t.nodes[**c as usize].is_dir) {
                    Some(&c) => i = c,
                    None => break,
                }
            }
            app.disk.selected = Some(i);
            app.disk.rows_ver += 1;
            app.disk.scroll_to = true;
        }
        if ui.small_button("💾 Export folders (CSV)").clicked() {
            let r = export_folders(t);
            app.result(r);
        }
    });
    let sel = app.disk.selected;
    let scroll_row = if app.disk.scroll_to { sel.and_then(|s| rows.iter().position(|r| r.0 == s)) } else { None };
    app.disk.scroll_to = false;
    let mut toggle = None;
    let mut select = None;
    let mut act: Option<(u32, Act)> = None;
    let mut tb = egui_extras::TableBuilder::new(ui)
        .id_salt("disk_tree")
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .auto_shrink([false, false])
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(egui_extras::Column::initial(380.0).at_least(160.0).clip(true))
        .column(egui_extras::Column::initial(150.0).clip(true))
        .column(egui_extras::Column::initial(95.0).clip(true))
        .column(egui_extras::Column::initial(95.0).clip(true))
        .column(egui_extras::Column::initial(80.0).clip(true))
        .column(egui_extras::Column::initial(70.0).clip(true))
        .column(egui_extras::Column::remainder().at_least(120.0).clip(true));
    if let Some(r) = scroll_row {
        tb = tb.scroll_to_row(r, Some(egui::Align::Center));
    }
    tb.header(22.0, |mut h| {
        for x in ["Name", "% of parent", "Size", "On disk", "Files", "Folders", "Modified"] {
            h.col(|ui| {
                ui.add(egui::Label::new(RichText::new(x).strong()).truncate());
            });
        }
    })
    .body(|body| {
        body.rows(row_h(), rows.len(), |mut row| {
            let (i, depth) = rows[row.index()];
            let n = &t.nodes[i as usize];
            row.set_selected(sel == Some(i));
            let parent_size = if n.parent == NONE { sz(n, alloc) } else { sz(&t.nodes[n.parent as usize], alloc) };
            let frac = if parent_size == 0 { 0.0 } else { sz(n, alloc) as f32 / parent_size as f32 };
            row.col(|ui| {
                ui.add_space(depth as f32 * 14.0);
                if n.is_dir && !n.children.is_empty() {
                    let open = app.disk.expanded.contains(&i);
                    if ui.add(egui::Label::new(RichText::new(if open { "⏷" } else { "⏵" }).color(c_dim())).sense(Sense::click())).clicked() {
                        toggle = Some(i);
                    }
                } else {
                    ui.add_space(12.0);
                }
                let (icon, color) = if n.is_dir {
                    ("📁", YELLOW)
                } else {
                    ("📄", if n.ext != NONE { t.exts[n.ext as usize].color } else { c_dim() })
                };
                ui.label(RichText::new(icon).color(color));
                let name = if n.parent == NONE { t.root_path.as_str() } else { &n.name };
                ui.add(egui::Label::new(RichText::new(name).strong_if(n.is_dir)).truncate());
            });
            row.col(|ui| {
                bar(ui, frac, if n.is_dir { accent() } else { GREEN }, &format!("{:.1}%", frac * 100.0), ui.available_width().min(140.0));
            });
            row.col(|ui| {
                ui.label(unit.fmt(n.size));
            });
            row.col(|ui| {
                ui.label(RichText::new(unit.fmt(n.alloc)).color(c_dim()));
            });
            row.col(|ui| {
                ui.label(if n.is_dir { util::fmt_num(n.files as u64) } else { String::new() });
            });
            row.col(|ui| {
                ui.label(if n.is_dir { util::fmt_num(n.dirs as u64) } else { String::new() });
            });
            row.col(|ui| {
                ui.label(RichText::new(if n.modified > 0 { util::fmt_time(util::filetime_to_unix(n.modified)) } else { String::new() }).color(c_dim()));
            });
            let resp = row.response();
            if resp.clicked() {
                select = Some(i);
            }
            if resp.double_clicked() && n.is_dir {
                toggle = Some(i);
            }
            resp.context_menu(|ui| {
                ui.label(RichText::new(&*n.name).strong());
                if let Some(a) = item_menu(ui, n.is_dir) {
                    act = Some((i, a));
                }
            });
        });
    });
    if let Some(i) = toggle {
        if !app.disk.expanded.remove(&i) {
            app.disk.expanded.insert(i);
        }
        app.disk.rows_ver += 1;
    }
    if let Some(i) = select {
        app.disk.selected = Some(i);
        let n = &t.nodes[i as usize];
        app.disk.tm_root = if n.is_dir { i } else if n.parent != NONE { n.parent } else { 0 };
    }
    if let Some((i, a)) = act {
        do_act(app, ctx, t, i, a);
    }
}

trait StrongIf {
    fn strong_if(self, b: bool) -> Self;
}
impl StrongIf for RichText {
    fn strong_if(self, b: bool) -> Self {
        if b {
            self.strong()
        } else {
            self
        }
    }
}

// ------------------------------------------------------------------ largest files

fn matches_any(patterns: &str, name: &str) -> bool {
    patterns.split(';').map(|p| p.trim()).filter(|p| !p.is_empty()).any(|p| disk::wild(p, name))
}

fn files(app: &mut App, ui: &mut Ui, t: &Tree, ctx: &egui::Context) {
    ui.horizontal_wrapped(|ui| {
        search_field(ui, &mut app.disk.filter, "🔍 Find files: name, part of name, or *.mp4", 240.0);
        ui.checkbox(&mut app.disk.match_path, "Match whole path").on_hover_text("Also look in folder names (slower on very large drives)");
        ui.label("Only");
        ui.add(egui::TextEdit::singleline(&mut app.disk.include).hint_text("*.iso; *.zip").desired_width(110.0));
        ui.label("Skip");
        ui.add(egui::TextEdit::singleline(&mut app.disk.exclude).hint_text("*.sys; *.dll").desired_width(110.0));
        ui.label("Show");
        egui::ComboBox::from_id_salt("disk_limit")
            .selected_text(if app.disk.limit == usize::MAX { "All".to_string() } else { util::fmt_num(app.disk.limit as u64) })
            .width(70.0)
            .show_ui(ui, |ui| {
                for l in [100usize, 1000, 10000, 100000, usize::MAX] {
                    ui.selectable_value(&mut app.disk.limit, l, if l == usize::MAX { "All".to_string() } else { util::fmt_num(l as u64) });
                }
            });
        if ui.small_button("Clear").clicked() {
            app.disk.filter.clear();
            app.disk.include.clear();
            app.disk.exclude.clear();
        }
    });
    let d = &mut app.disk;
    let key = format!("{}|{}|{}|{}|{}|{}|{}|{}", t.id, d.version, d.filter, d.match_path, d.include, d.exclude, d.limit, d.use_alloc);
    if d.files_cache.as_ref().map(|c| c.0 != key).unwrap_or(true) {
        let f = d.filter.trim().to_lowercase();
        let wildcard = f.contains('*') || f.contains('?');
        let alloc = d.use_alloc;
        let mut list: Vec<u32> = Vec::new();
        let mut matched_size = 0u64;
        for (i, n) in t.nodes.iter().enumerate() {
            if n.is_dir || n.parent == NONE {
                continue;
            }
            if !d.include.trim().is_empty() && !matches_any(&d.include, &n.name) {
                continue;
            }
            if !d.exclude.trim().is_empty() && matches_any(&d.exclude, &n.name) {
                continue;
            }
            if !f.is_empty() {
                let hay = if d.match_path { t.path(i as u32).to_lowercase() } else { n.name.to_lowercase() };
                let ok = if wildcard { disk::wild(&f, &hay) || (d.match_path && disk::wild(&f, &n.name.to_lowercase())) } else { hay.contains(&f) };
                if !ok {
                    continue;
                }
            }
            matched_size += sz(n, alloc);
            list.push(i as u32);
        }
        let matched = list.len();
        let nodes = &t.nodes;
        let key_of = |i: &u32| std::cmp::Reverse(sz(&nodes[*i as usize], alloc));
        if d.limit < list.len() {
            list.select_nth_unstable_by_key(d.limit, key_of);
            list.truncate(d.limit);
        }
        list.sort_unstable_by_key(key_of);
        d.files_cache = Some((key, list, matched, matched_size));
    }
    let (_, list, matched, matched_size) = d.files_cache.clone().unwrap();
    let total = sz(&t.nodes[0], d.use_alloc).max(1);
    let unit = d.unit;
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} matching files using {} ({:.1}% of the scan) — showing the {} largest", util::fmt_num(matched as u64), unit.fmt(matched_size), matched_size as f64 / total as f64 * 100.0, util::fmt_num(list.len() as u64))).color(c_dim()).size(12.0));
        if ui.small_button("💾 Export list (CSV)").clicked() {
            let r = export_files(t, &list);
            app.result(r);
        }
    });
    let sel = app.disk.selected;
    let mut select = None;
    let mut act = None;
    egui_extras::TableBuilder::new(ui)
        .id_salt("disk_files")
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .auto_shrink([false, false])
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(egui_extras::Column::initial(300.0).clip(true))
        .column(egui_extras::Column::initial(95.0).clip(true))
        .column(egui_extras::Column::initial(95.0).clip(true))
        .column(egui_extras::Column::initial(120.0).clip(true))
        .column(egui_extras::Column::initial(140.0).clip(true))
        .column(egui_extras::Column::remainder().clip(true))
        .header(22.0, |mut h| {
            for x in ["File", "Size", "On disk", "% of scan", "Modified", "Folder"] {
                h.col(|ui| {
                    ui.add(egui::Label::new(RichText::new(x).strong()).truncate());
                });
            }
        })
        .body(|body| {
            body.rows(row_h(), list.len(), |mut row| {
                let i = list[row.index()];
                let n = &t.nodes[i as usize];
                row.set_selected(sel == Some(i));
                let color = if n.ext != NONE { t.exts[n.ext as usize].color } else { c_dim() };
                row.col(|ui| {
                    ui.label(RichText::new("■").color(color));
                    ui.add(egui::Label::new(&*n.name).truncate());
                });
                row.col(|ui| {
                    ui.label(unit.fmt(n.size));
                });
                row.col(|ui| {
                    ui.label(RichText::new(unit.fmt(n.alloc)).color(c_dim()));
                });
                row.col(|ui| {
                    let f = sz(n, app.disk.use_alloc) as f32 / total as f32;
                    bar(ui, (f * 20.0).min(1.0), GREEN, &format!("{:.2}%", f * 100.0), 100.0);
                });
                row.col(|ui| {
                    ui.label(RichText::new(util::fmt_time(util::filetime_to_unix(n.modified))).color(c_dim()));
                });
                row.col(|ui| {
                    ui.add(egui::Label::new(RichText::new(t.path(n.parent)).color(c_dim())).truncate());
                });
                let resp = row.response();
                if resp.clicked() {
                    select = Some(i);
                }
                resp.context_menu(|ui| {
                    if let Some(a) = item_menu(ui, false) {
                        act = Some((i, a));
                    }
                });
            });
        });
    if let Some(i) = select {
        app.disk.selected = Some(i);
        app.disk.tm_root = t.nodes[i as usize].parent;
    }
    if let Some((i, a)) = act {
        do_act(app, ctx, t, i, a);
    }
}

// ------------------------------------------------------------------ file types

fn types(app: &mut App, ui: &mut Ui, t: &Tree) {
    let unit = app.disk.unit;
    let total: u64 = t.exts.iter().map(|e| if app.disk.use_alloc { e.alloc } else { e.size }).sum::<u64>().max(1);
    // stacked bar of the top types
    let (rect, _) = ui.allocate_exact_size(Vec2::new(ui.available_width(), 22.0), Sense::hover());
    let mut x = rect.left();
    for e in t.exts.iter().take(20) {
        let v = if app.disk.use_alloc { e.alloc } else { e.size };
        let w = rect.width() * v as f32 / total as f32;
        if w >= 1.0 {
            ui.painter().rect_filled(Rect::from_min_size(Pos2::new(x, rect.top()), Vec2::new(w, rect.height())), 2.0, e.color);
        }
        x += w;
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} file types • click a row to list those files", util::fmt_num(t.exts.len() as u64))).color(c_dim()).size(12.0));
        if ui.small_button("💾 Export types (CSV)").clicked() {
            let r = export_types(t);
            app.result(r);
        }
    });
    let mut pick: Option<String> = None;
    egui_extras::TableBuilder::new(ui)
        .id_salt("disk_types")
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .auto_shrink([false, false])
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center))
        .column(egui_extras::Column::initial(40.0))
        .column(egui_extras::Column::initial(160.0).clip(true))
        .column(egui_extras::Column::initial(160.0).clip(true))
        .column(egui_extras::Column::initial(100.0).clip(true))
        .column(egui_extras::Column::initial(100.0).clip(true))
        .column(egui_extras::Column::initial(90.0).clip(true))
        .column(egui_extras::Column::remainder().clip(true))
        .header(22.0, |mut h| {
            for x in ["", "Type", "Share", "Size", "On disk", "Files", "Average size"] {
                h.col(|ui| {
                    ui.add(egui::Label::new(RichText::new(x).strong()).truncate());
                });
            }
        })
        .body(|body| {
            body.rows(row_h(), t.exts.len(), |mut row| {
                let e = &t.exts[row.index()];
                let v = if app.disk.use_alloc { e.alloc } else { e.size };
                let f = v as f32 / total as f32;
                row.col(|ui| {
                    let (r, _) = ui.allocate_exact_size(Vec2::new(18.0, 12.0), Sense::hover());
                    ui.painter().rect_filled(r, 3.0, e.color);
                });
                row.col(|ui| {
                    ui.label(if e.ext.is_empty() { "(no extension)".to_string() } else { format!(".{}", e.ext) });
                });
                row.col(|ui| {
                    bar(ui, f, e.color, &format!("{:.1}%", f * 100.0), 140.0);
                });
                row.col(|ui| {
                    ui.label(unit.fmt(e.size));
                });
                row.col(|ui| {
                    ui.label(RichText::new(unit.fmt(e.alloc)).color(c_dim()));
                });
                row.col(|ui| {
                    ui.label(util::fmt_num(e.count));
                });
                row.col(|ui| {
                    ui.label(RichText::new(unit.fmt(e.size / e.count.max(1))).color(c_dim()));
                });
                if row.response().clicked() {
                    pick = Some(e.ext.clone());
                }
            });
        });
    if let Some(ext) = pick {
        app.disk.include = if ext.is_empty() { String::new() } else { format!("*.{ext}") };
        app.disk.filter.clear();
        app.disk.tab = DTab::Files;
    }
}

// ------------------------------------------------------------------ duplicates

fn duplicates(app: &mut App, ui: &mut Ui, t: &Tree, ctx: &egui::Context) {
    ui.horizontal_wrapped(|ui| {
        ui.label("Files at least");
        ui.add(egui::DragValue::new(&mut app.disk.dup_min_mb).range(0.0..=100000.0).speed(0.5).suffix(" MB"));
        ui.checkbox(&mut app.disk.dup_date, "Same modified time too").on_hover_text("Stricter: name, size and date must all match");
        if ui.add_enabled(!app.disk.dups.is_running(), egui::Button::new("🔍 Find duplicates")).clicked() || (app.disk.dup_for != t.id && !app.disk.dups.started) {
            let tree = app.disk.tree.clone().unwrap();
            let min = (app.disk.dup_min_mb as f64 * 1048576.0) as u64;
            let with_date = app.disk.dup_date;
            app.disk.dups = Job::default();
            app.disk.dups.start(ctx, move || Ok(disk::duplicates(&tree, min, with_date)));
            app.disk.dup_for = t.id;
        }
        ui.label(RichText::new("Matches files by name and size; open them to be sure before deleting.").color(c_dim()).size(11.5));
    });
    if !job_status(ui, &app.disk.dups) {
        return;
    }
    let unit = app.disk.unit;
    let mut act = None;
    app.disk.dups.with(|r| {
        let Some(Ok(groups)) = r else { return };
        let wasted: u64 = groups.iter().map(|g| g.size * (g.nodes.len() as u64 - 1)).sum();
        ui.label(RichText::new(format!("{} groups of duplicates • {} could be freed by keeping one copy of each", util::fmt_num(groups.len() as u64), unit.fmt(wasted))).strong());
        egui::ScrollArea::vertical().id_salt("dups").auto_shrink([false, false]).show(ui, |ui| {
            for (gi, g) in groups.iter().take(1000).enumerate() {
                let live: Vec<u32> = g.nodes.iter().copied().filter(|i| t.nodes[*i as usize].parent != NONE).collect();
                if live.len() < 2 {
                    continue;
                }
                let head = format!("{}  —  {} × {} copies  (frees {})", g.name, unit.fmt(g.size), live.len(), unit.fmt(g.size * (live.len() as u64 - 1)));
                egui::CollapsingHeader::new(head).id_salt(("dup", gi)).show(ui, |ui| {
                    for &i in &live {
                        let n = &t.nodes[i as usize];
                        let r = ui.add(egui::Label::new(RichText::new(format!("{}   •   {}", t.path(i), util::fmt_time(util::filetime_to_unix(n.modified)))).size(12.5)).sense(Sense::click()));
                        r.context_menu(|ui| {
                            if let Some(a) = item_menu(ui, false) {
                                act = Some((i, a));
                            }
                        });
                    }
                });
            }
        });
    });
    if let Some((i, a)) = act {
        do_act(app, ctx, t, i, a);
    }
}

// ------------------------------------------------------------------ treemap

fn layout(t: &Tree, root: u32, rect: Rect, d: &DiskUi) -> Vec<(u32, Rect, u8)> {
    let mut out = Vec::with_capacity(4096);
    let alloc = d.use_alloc;
    let budget = 40_000usize;
    fn go(t: &Tree, node: u32, rect: Rect, depth: u8, alloc: bool, names: bool, maxd: u8, budget: usize, out: &mut Vec<(u32, Rect, u8)>, free: u64) {
        let n = &t.nodes[node as usize];
        let mut items: Vec<(u32, f64)> = n.children.iter().map(|c| (*c, sz(&t.nodes[*c as usize], alloc) as f64)).filter(|x| x.1 > 0.0).collect();
        if free > 0 {
            items.push((FREE, free as f64));
            items.sort_by(|a, b| b.1.total_cmp(&a.1));
        }
        for (id, r) in disk::squarify(&items, rect) {
            if out.len() >= budget {
                return;
            }
            out.push((id, r, depth));
            if id == FREE {
                continue;
            }
            let c = &t.nodes[id as usize];
            if c.is_dir && depth < maxd && r.width() > 8.0 && r.height() > 8.0 && !c.children.is_empty() {
                let header = if names && r.height() > 44.0 && r.width() > 70.0 { 14.0 } else { 0.0 };
                let inner = Rect::from_min_max(Pos2::new(r.left() + 2.0, r.top() + 2.0 + header), Pos2::new(r.right() - 2.0, r.bottom() - 2.0));
                if inner.width() > 2.0 && inner.height() > 2.0 {
                    go(t, id, inner, depth + 1, alloc, names, maxd, budget, out, 0);
                }
            }
        }
    }
    let free = if d.tm_free && root == 0 && disk::is_drive_root(&t.root_path).is_some() { t.free_space } else { 0 };
    go(t, root, rect, 0, alloc, d.tm_names, d.tm_depth.min(60) as u8, budget, &mut out, free);
    out
}

fn color_of(t: &Tree, i: u32, cb: bool) -> Color32 {
    let n = &t.nodes[i as usize];
    if n.ext == NONE {
        return Color32::from_rgb(120, 120, 120);
    }
    if cb {
        disk::PALETTE_CB[(n.ext as usize).min(disk::PALETTE_CB.len() - 1)]
    } else {
        t.exts[n.ext as usize].color
    }
}

fn treemap(app: &mut App, ui: &mut Ui, t: &Tree, ctx: &egui::Context) {
    let root = if (app.disk.tm_root as usize) < t.nodes.len() && t.nodes[app.disk.tm_root as usize].is_dir { app.disk.tm_root } else { 0 };
    ui.horizontal_wrapped(|ui| {
        if ui.add_enabled(t.nodes[root as usize].parent != NONE, egui::Button::new("⬆ Up")).clicked() {
            app.disk.tm_root = t.nodes[root as usize].parent;
        }
        if ui.add_enabled(root != 0, egui::Button::new("⏫ Top")).clicked() {
            app.disk.tm_root = 0;
        }
        // breadcrumb
        let chain: Vec<u32> = t.ancestors(root).into_iter().rev().collect();
        for (k, &a) in chain.iter().enumerate() {
            if k > 0 {
                ui.label(RichText::new("›").color(c_dim()));
            }
            let name = if t.nodes[a as usize].parent == NONE { t.root_path.clone() } else { t.nodes[a as usize].name.to_string() };
            if ui.add(egui::Label::new(RichText::new(name).color(if a == root { accent() } else { c_text() })).sense(Sense::click())).clicked() {
                app.disk.tm_root = a;
            }
        }
        ui.label(RichText::new(format!("— {}", app.disk.unit.fmt(sz(&t.nodes[root as usize], app.disk.use_alloc)))).color(c_dim()));
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.checkbox(&mut app.disk.tm_cb, "Colour-blind colours");
            ui.checkbox(&mut app.disk.tm_shade, "Shading");
            ui.checkbox(&mut app.disk.tm_free, "Free space").on_hover_text("Show the drive's free space as a grey block (whole-drive view)");
            ui.checkbox(&mut app.disk.tm_names, "Names");
            ui.add(egui::DragValue::new(&mut app.disk.tm_depth).range(1..=40).prefix("depth "));
        });
    });
    let rect = ui.available_rect_before_wrap();
    let resp = ui.allocate_rect(rect, Sense::click());
    if rect.width() < 20.0 || rect.height() < 20.0 {
        return;
    }
    let d = &mut app.disk;
    let key = format!("{}|{}|{}|{:.0}x{:.0}|{}|{}|{}|{}", t.id, d.version, root, rect.width(), rect.height(), d.use_alloc, d.tm_names, d.tm_free, d.tm_depth);
    if d.tm_cache.as_ref().map(|c| c.0 != key).unwrap_or(true) {
        let local = Rect::from_min_size(Pos2::ZERO, rect.size());
        let l = layout(t, root, local, d);
        d.tm_cache = Some((key, l));
    }
    let cells = &d.tm_cache.as_ref().unwrap().1;
    let p = ui.painter_at(rect);
    p.rect_filled(rect, 4.0, c_bg());
    let off = rect.min.to_vec2();
    let shade = d.tm_shade;
    let cb = d.tm_cb;
    let mut mesh = egui::Mesh::default();
    let mut lines: Vec<egui::Shape> = Vec::new();
    let mut labels: Vec<(Pos2, String, Color32, f32)> = Vec::new();
    for &(id, r, depth) in cells.iter() {
        let r = r.translate(off);
        if id == FREE {
            p.rect_filled(r, 0.0, Color32::from_gray(70));
            labels.push((r.center(), format!("Free space {}", d.unit.fmt(t.free_space)), Color32::from_gray(220), 12.0));
            continue;
        }
        let n = &t.nodes[id as usize];
        let base = if n.is_dir { c_card_hi().lerp_to_gamma(c_bg(), (depth as f32 * 0.08).min(0.5)) } else { color_of(t, id, cb) };
        if shade && !n.is_dir && r.width() > 2.0 && r.height() > 2.0 {
            let i0 = mesh.vertices.len() as u32;
            let light = base.lerp_to_gamma(Color32::WHITE, 0.35);
            let dark = base.lerp_to_gamma(Color32::BLACK, 0.35);
            mesh.colored_vertex(r.left_top(), light);
            mesh.colored_vertex(r.right_top(), base);
            mesh.colored_vertex(r.right_bottom(), dark);
            mesh.colored_vertex(r.left_bottom(), base);
            mesh.add_triangle(i0, i0 + 1, i0 + 2);
            mesh.add_triangle(i0, i0 + 2, i0 + 3);
        } else {
            mesh.add_colored_rect(r, base);
        }
        if r.width() > 3.0 && r.height() > 3.0 {
            lines.push(egui::Shape::rect_stroke(r, 0.0, Stroke::new(0.5, Color32::from_black_alpha(120))));
        }
        if d.tm_names {
            if n.is_dir && r.height() > 44.0 && r.width() > 70.0 {
                labels.push((Pos2::new(r.left() + 4.0, r.top() + 8.0), format!("{}  {}", n.name, d.unit.fmt(sz(n, d.use_alloc))), c_text(), 11.0));
            } else if !n.is_dir && r.width() > 70.0 && r.height() > 18.0 {
                labels.push((r.center(), n.name.to_string(), Color32::from_black_alpha(220), 11.0));
            }
        }
    }
    p.add(egui::Shape::mesh(mesh));
    p.extend(lines);
    for (pos, text, color, size) in labels {
        let align = if color == c_text() { Align2::LEFT_CENTER } else { Align2::CENTER_CENTER };
        p.text(pos, align, text, FontId::proportional(size), color);
    }
    // selection outline
    if let Some(sel) = d.selected {
        if let Some(&(_, r, _)) = cells.iter().find(|c| c.0 == sel) {
            p.rect_stroke(r.translate(off), 0.0, Stroke::new(2.5, Color32::WHITE));
            p.rect_stroke(r.translate(off).shrink(2.5), 0.0, Stroke::new(1.0, Color32::BLACK));
        }
    }
    // hover
    let hovered = resp.hover_pos().and_then(|pos| cells.iter().rev().find(|c| c.1.translate(off).contains(pos)).map(|c| (c.0, c.1.translate(off))));
    if let Some((id, r)) = hovered {
        p.rect_stroke(r, 0.0, Stroke::new(1.5, accent()));
        let root_size = sz(&t.nodes[root as usize], d.use_alloc).max(1);
        if id == FREE {
            egui::show_tooltip_at_pointer(ctx, ui.layer_id(), egui::Id::new("tm_tip"), |ui| {
                ui.label(format!("Free space: {}", d.unit.fmt(t.free_space)));
            });
        } else {
            let n = &t.nodes[id as usize];
            let path = t.path(id);
            let ext = t.ext_name(id).to_string();
            let unit = d.unit;
            let s = sz(n, d.use_alloc);
            egui::show_tooltip_at_pointer(ctx, ui.layer_id(), egui::Id::new("tm_tip"), |ui| {
                ui.label(RichText::new(&path).strong());
                ui.label(format!("{} ({:.2}% of this view)", unit.fmt(s), s as f64 / root_size as f64 * 100.0));
                if n.is_dir {
                    ui.label(format!("{} files in {} folders", util::fmt_num(n.files as u64), util::fmt_num(n.dirs as u64)));
                    ui.label(RichText::new("Double-click to zoom in").color(c_dim()));
                } else {
                    ui.label(if ext.is_empty() { "No extension".to_string() } else { format!(".{ext} file") });
                }
            });
        }
    }
    let hovered_id = hovered.map(|h| h.0).filter(|i| *i != FREE);
    if resp.clicked() {
        if let Some(id) = hovered_id {
            reveal(&mut app.disk, t, id);
        }
    }
    if resp.double_clicked() {
        if let Some(id) = hovered_id {
            let n = &t.nodes[id as usize];
            // zoom into the outermost folder under the pointer below the current root
            let mut target = if n.is_dir { id } else { n.parent };
            let chain = t.ancestors(target);
            if let Some(pos) = chain.iter().position(|a| *a == root) {
                if pos >= 1 {
                    target = chain[pos - 1];
                }
            }
            app.disk.tm_root = target;
        }
    }
    let mut act = None;
    if let Some(id) = hovered_id {
        let is_dir = t.nodes[id as usize].is_dir;
        resp.context_menu(|ui| {
            ui.label(RichText::new(&*t.nodes[id as usize].name).strong());
            if let Some(a) = item_menu(ui, is_dir) {
                act = Some((id, a));
            }
        });
    }
    if let Some((i, a)) = act {
        do_act(app, ctx, t, i, a);
    }
}

// ------------------------------------------------------------------ export

fn csv_escape(s: &str) -> String {
    if s.contains([',', '"', '\n']) {
        format!("\"{}\"", s.replace('"', "\"\""))
    } else {
        s.to_string()
    }
}

fn save_csv(name: &str, body: String) -> Result<String, String> {
    let path = util::sub_dir("exports").join(format!("{name}-{}.csv", util::stamp()));
    std::fs::write(&path, body).map_err(|e| e.to_string())?;
    crate::sys::open_file_location(&path.to_string_lossy());
    Ok(format!("Saved {}", path.display()))
}

fn export_files(t: &Tree, list: &[u32]) -> Result<String, String> {
    let mut s = String::from("path,size_bytes,on_disk_bytes,modified\n");
    for &i in list {
        let n = &t.nodes[i as usize];
        s.push_str(&format!("{},{},{},{}\n", csv_escape(&t.path(i)), n.size, n.alloc, util::fmt_time(util::filetime_to_unix(n.modified))));
    }
    save_csv("disk-files", s)
}

fn export_types(t: &Tree) -> Result<String, String> {
    let mut s = String::from("extension,size_bytes,on_disk_bytes,files\n");
    for e in &t.exts {
        s.push_str(&format!("{},{},{},{}\n", csv_escape(&e.ext), e.size, e.alloc, e.count));
    }
    save_csv("disk-types", s)
}

fn export_folders(t: &Tree) -> Result<String, String> {
    let mut s = String::from("path,size_bytes,on_disk_bytes,files,folders,modified\n");
    for (i, n) in t.nodes.iter().enumerate() {
        if n.is_dir && (n.parent != NONE || i == 0) {
            s.push_str(&format!("{},{},{},{},{},{}\n", csv_escape(&t.path(i as u32)), n.size, n.alloc, n.files, n.dirs, util::fmt_time(util::filetime_to_unix(n.modified))));
        }
    }
    save_csv("disk-folders", s)
}
