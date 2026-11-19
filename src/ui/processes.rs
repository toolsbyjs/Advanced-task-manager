//! Process explorer: flat and tree views, search, sorting, customizable columns,
//! actions, and the detail tabs (threads, modules, memory map, handles, network...).

use super::widgets::*;
use super::{App, Confirm, Page};
use crate::collector::{ProcInfo, State};
use crate::config::Priority;
use crate::nt;
use crate::sys;
use crate::util::{self, fmt_bytes, fmt_num, Job};
use egui::{Color32, RichText, Sense, Ui};
use std::collections::{HashMap, HashSet};
use std::time::Instant;

pub struct ColDef {
    pub id: &'static str,
    pub label: &'static str,
    pub width: f32,
    pub admin: bool,
    pub tip: &'static str,
}

const fn c(id: &'static str, label: &'static str, width: f32, admin: bool, tip: &'static str) -> ColDef {
    ColDef { id, label, width, admin, tip }
}

pub const COLUMNS: &[ColDef] = &[
    c("name", "Name", 230.0, false, "Program name"),
    c("pid", "PID", 62.0, false, "Process ID"),
    c("ppid", "Parent PID", 72.0, true, "The process that started it"),
    c("status", "Status", 78.0, false, "Running or suspended"),
    c("user", "User", 150.0, false, "Account the process runs as"),
    c("description", "Description", 200.0, false, "File description"),
    c("company", "Publisher", 150.0, false, "Company in the file's version info"),
    c("cpu", "CPU", 62.0, false, "Share of all processors"),
    c("cpu_kernel", "Kernel CPU", 74.0, true, "Time spent inside Windows on its behalf"),
    c("cpu_time", "CPU time", 84.0, true, "Total processor time used since start"),
    c("cycles", "Cycles Δ", 92.0, true, "CPU cycles used in the last interval"),
    c("private", "Memory", 92.0, false, "Private bytes (memory only this process uses)"),
    c("ws", "Working set", 92.0, false, "Physical memory currently used"),
    c("private_ws", "Private WS", 90.0, true, "Physical memory not shared with others"),
    c("peak_ws", "Peak WS", 90.0, true, "Highest working set"),
    c("commit", "Commit", 90.0, true, "Commit charge"),
    c("virtual", "Virtual size", 96.0, true, "Address space reserved"),
    c("paged_pool", "Paged pool", 84.0, true, "Kernel paged pool charged to it"),
    c("nonpaged_pool", "Non-paged pool", 96.0, true, "Kernel non-paged pool charged to it"),
    c("page_faults", "Page faults", 90.0, true, "Total page faults"),
    c("pf_sec", "Faults/s", 72.0, true, "Page faults per second"),
    c("hard_faults", "Hard faults", 80.0, true, "Faults that needed disk reads"),
    c("disk", "Disk", 84.0, false, "Read + write per second"),
    c("io_read", "Read/s", 82.0, true, "Bytes read per second"),
    c("io_write", "Write/s", 82.0, true, "Bytes written per second"),
    c("io_other", "Other I/O/s", 84.0, true, "Control I/O per second"),
    c("read_total", "Total read", 90.0, true, "Bytes read since start"),
    c("write_total", "Total written", 94.0, true, "Bytes written since start"),
    c("read_ops", "Read ops", 80.0, true, "Read operations since start"),
    c("write_ops", "Write ops", 80.0, true, "Write operations since start"),
    c("gpu", "GPU", 60.0, false, "Graphics engine use"),
    c("gpu_mem", "GPU memory", 92.0, false, "Dedicated graphics memory"),
    c("conns", "Connections", 84.0, false, "Open network connections"),
    c("threads", "Threads", 64.0, false, "Thread count"),
    c("handles", "Handles", 68.0, false, "Open kernel object handles"),
    c("session", "Session", 60.0, true, "Logon session"),
    c("priority", "Base priority", 84.0, true, "Scheduling base priority"),
    c("integrity", "Integrity", 84.0, true, "Mandatory integrity level"),
    c("elevated", "Elevated", 70.0, true, "Running with admin rights"),
    c("arch", "Architecture", 86.0, true, "64-bit, 32-bit or ARM"),
    c("version", "Version", 110.0, true, "File version"),
    c("started", "Started", 140.0, false, "When it started"),
    c("path", "Path", 320.0, false, "Program file"),
    c("cmdline", "Command line", 420.0, true, "Full command line"),
];

pub fn default_columns() -> Vec<String> {
    ["name", "pid", "status", "cpu", "private", "ws", "disk", "gpu", "conns", "threads", "handles", "user", "description"]
        .iter()
        .map(|s| s.to_string())
        .collect()
}

#[derive(Clone, Copy, PartialEq, Eq)]
pub enum Tab {
    General,
    Performance,
    Threads,
    Modules,
    Memory,
    Handles,
    Network,
    Security,
    History,
}

pub fn tab_from_name(n: &str) -> Tab {
    match n {
        "performance" => Tab::Performance,
        "threads" => Tab::Threads,
        "modules" => Tab::Modules,
        "memory" => Tab::Memory,
        "handles" => Tab::Handles,
        "network" => Tab::Network,
        "security" => Tab::Security,
        "history" => Tab::History,
        _ => Tab::General,
    }
}

impl Tab {
    fn label(&self) -> &'static str {
        match self {
            Tab::General => "General",
            Tab::Performance => "Performance",
            Tab::Threads => "Threads",
            Tab::Modules => "Modules",
            Tab::Memory => "Memory map",
            Tab::Handles => "Handles",
            Tab::Network => "Network",
            Tab::Security => "Security",
            Tab::History => "History",
        }
    }
    fn admin(&self) -> bool {
        matches!(self, Tab::Threads | Tab::Modules | Tab::Memory | Tab::Handles | Tab::Security)
    }
}

#[derive(Default)]
pub struct Detail {
    pub pid: u32,
    pub create_time: i64,
    pub modules: Job<Vec<sys::Module>>,
    pub memmap: Job<Vec<sys::Region>>,
    pub handles: Job<Vec<sys::HandleInfo>>,
    pub dynamic: Job<sys::ProcDynamic>,
    pub dyn_at: Option<Instant>,
    pub conns: Job<Vec<sys::Conn>>,
    pub conns_at: Option<Instant>,
    pub thread_extra: Job<HashMap<u32, (Option<usize>, String)>>,
    pub mem_filter: String,
    pub mem_committed: bool,
    pub handle_filter: String,
    pub mod_filter: String,
    pub sel_thread: Option<u32>,
    pub affinity_edit: Option<u64>,
}

pub struct ProcUi {
    pub search: String,
    pub selected: Option<u32>,
    pub sort_col: String,
    pub asc: bool,
    pub collapsed: HashSet<u32>,
    pub show_columns: bool,
    pub tab: Tab,
    pub detail: Detail,
    pub only_mine: bool,
    pub hide_system: bool,
    pub hist_filter: String,
    pub focus_search: bool,
    pub frozen: Option<Vec<(u32, usize, bool)>>,
}

impl Default for ProcUi {
    fn default() -> Self {
        Self {
            search: String::new(),
            selected: None,
            sort_col: "cpu".into(),
            asc: false,
            collapsed: HashSet::new(),
            show_columns: false,
            tab: Tab::General,
            detail: Detail::default(),
            only_mine: false,
            hide_system: false,
            hist_filter: String::new(),
            focus_search: false,
            frozen: None,
        }
    }
}

enum Key {
    N(f64),
    S(String),
}

fn key(p: &ProcInfo, id: &str) -> Key {
    use Key::*;
    match id {
        "name" => S(p.name.to_lowercase()),
        "pid" => N(p.pid as f64),
        "ppid" => N(p.ppid as f64),
        "status" => S(if p.suspended { "suspended".into() } else { "running".into() }),
        "user" => S(p.st.user.to_lowercase()),
        "description" => S(p.st.description.to_lowercase()),
        "company" => S(p.st.company.to_lowercase()),
        "cpu" => N(p.cpu as f64),
        "cpu_kernel" => N(p.cpu_kernel as f64),
        "cpu_time" => N(p.cpu_time),
        "cycles" => N(p.cycles_delta as f64),
        "private" => N(p.private_bytes as f64),
        "ws" => N(p.ws as f64),
        "private_ws" => N(p.private_ws as f64),
        "peak_ws" => N(p.peak_ws as f64),
        "commit" => N(p.commit as f64),
        "virtual" => N(p.virtual_size as f64),
        "paged_pool" => N(p.paged_pool as f64),
        "nonpaged_pool" => N(p.nonpaged_pool as f64),
        "page_faults" => N(p.page_faults as f64),
        "pf_sec" => N(p.page_faults_sec as f64),
        "hard_faults" => N(p.hard_faults as f64),
        "disk" => N((p.io_read_rate + p.io_write_rate) as f64),
        "io_read" => N(p.io_read_rate as f64),
        "io_write" => N(p.io_write_rate as f64),
        "io_other" => N(p.io_other_rate as f64),
        "read_total" => N(p.read_bytes as f64),
        "write_total" => N(p.write_bytes as f64),
        "read_ops" => N(p.read_ops as f64),
        "write_ops" => N(p.write_ops as f64),
        "gpu" => N(p.gpu as f64),
        "gpu_mem" => N(p.gpu_dedicated as f64),
        "conns" => N(p.connections as f64),
        "threads" => N(p.threads as f64),
        "handles" => N(p.handles as f64),
        "session" => N(p.session as f64),
        "priority" => N(p.base_priority as f64),
        "integrity" => S(p.st.integrity.clone()),
        "elevated" => S(format!("{:?}", p.st.elevated)),
        "arch" => S(p.st.arch.clone()),
        "version" => S(p.st.version.clone()),
        "started" => N(p.create_time as f64),
        "path" => S(p.st.path.to_lowercase()),
        "cmdline" => S(p.st.cmdline.to_lowercase()),
        _ => N(0.0),
    }
}

fn text(p: &ProcInfo, id: &str) -> String {
    match id {
        "name" => p.name.clone(),
        "pid" => p.pid.to_string(),
        "ppid" => p.ppid.to_string(),
        "status" => if p.suspended { "Suspended".into() } else { "Running".into() },
        "user" => {
            if p.st.access_denied {
                "(access denied)".into()
            } else {
                p.st.user.rsplit('\\').next().unwrap_or("").to_string()
            }
        }
        "description" => p.st.description.clone(),
        "company" => p.st.company.clone(),
        "cpu" => if p.cpu < 0.05 { "0%".into() } else { format!("{:.1}%", p.cpu) },
        "cpu_kernel" => format!("{:.1}%", p.cpu_kernel),
        "cpu_time" => util::fmt_dur(p.cpu_time as u64),
        "cycles" => fmt_num(p.cycles_delta),
        "private" => fmt_bytes(p.private_bytes as f64),
        "ws" => fmt_bytes(p.ws as f64),
        "private_ws" => fmt_bytes(p.private_ws as f64),
        "peak_ws" => fmt_bytes(p.peak_ws as f64),
        "commit" => fmt_bytes(p.commit as f64),
        "virtual" => fmt_bytes(p.virtual_size as f64),
        "paged_pool" => fmt_bytes(p.paged_pool as f64),
        "nonpaged_pool" => fmt_bytes(p.nonpaged_pool as f64),
        "page_faults" => fmt_num(p.page_faults as u64),
        "pf_sec" => format!("{:.0}", p.page_faults_sec),
        "hard_faults" => fmt_num(p.hard_faults as u64),
        "disk" => {
            let r = p.io_read_rate + p.io_write_rate;
            if r < 1.0 { "0 B/s".into() } else { util::fmt_rate(r as f64) }
        }
        "io_read" => util::fmt_rate(p.io_read_rate as f64),
        "io_write" => util::fmt_rate(p.io_write_rate as f64),
        "io_other" => util::fmt_rate(p.io_other_rate as f64),
        "read_total" => fmt_bytes(p.read_bytes as f64),
        "write_total" => fmt_bytes(p.write_bytes as f64),
        "read_ops" => fmt_num(p.read_ops),
        "write_ops" => fmt_num(p.write_ops),
        "gpu" => if p.gpu < 0.05 { "0%".into() } else { format!("{:.1}%", p.gpu) },
        "gpu_mem" => if p.gpu_dedicated == 0 { "-".into() } else { fmt_bytes(p.gpu_dedicated as f64) },
        "conns" => if p.connections == 0 { "-".into() } else { p.connections.to_string() },
        "threads" => p.threads.to_string(),
        "handles" => fmt_num(p.handles as u64),
        "session" => p.session.to_string(),
        "priority" => p.base_priority.to_string(),
        "integrity" => p.st.integrity.clone(),
        "elevated" => match p.st.elevated {
            Some(true) => "Yes".into(),
            Some(false) => "No".into(),
            None => "".into(),
        },
        "arch" => p.st.arch.clone(),
        "version" => p.st.version.clone(),
        "started" => util::fmt_time(util::filetime_to_unix(p.create_time)),
        "path" => p.st.path.clone(),
        "cmdline" => p.st.cmdline.clone(),
        _ => String::new(),
    }
}

fn cell_heat(p: &ProcInfo, id: &str, total_mem: u64) -> Color32 {
    if p.pid == 0 || !look().heat {
        return Color32::TRANSPARENT;
    }
    match id {
        "cpu" => heat(p.cpu * 2.0),
        "private" => heat(p.private_bytes as f32 / total_mem.max(1) as f32 * 100.0 * 20.0),
        "disk" => heat((p.io_read_rate + p.io_write_rate) / (20.0 * 1048576.0) * 100.0),
        "gpu" => heat(p.gpu * 1.5),
        _ => Color32::TRANSPARENT,
    }
}

fn matches(p: &ProcInfo, f: &str) -> bool {
    if f.is_empty() {
        return true;
    }
    p.name.to_lowercase().contains(f)
        || p.pid.to_string() == f
        || p.st.description.to_lowercase().contains(f)
        || p.st.path.to_lowercase().contains(f)
        || p.st.user.to_lowercase().contains(f)
        || p.st.cmdline.to_lowercase().contains(f)
        || p.st.company.to_lowercase().contains(f)
}

fn sort_pids(s: &State, pids: &mut [u32], col: &str, asc: bool) {
    pids.sort_by(|a, b| {
        let (pa, pb) = (&s.procs[a], &s.procs[b]);
        let o = match (key(pa, col), key(pb, col)) {
            (Key::N(x), Key::N(y)) => x.total_cmp(&y),
            (Key::S(x), Key::S(y)) => x.cmp(&y),
            _ => std::cmp::Ordering::Equal,
        };
        let o = o.then(pa.pid.cmp(&pb.pid));
        if asc {
            o
        } else {
            o.reverse()
        }
    });
}

/// Rows in display order: (pid, depth, has_children).
fn build_rows(app: &App, s: &State) -> Vec<(u32, usize, bool)> {
    let u = &app.procs;
    let f = u.search.trim().to_lowercase();
    let me = std::env::var("USERNAME").unwrap_or_default().to_lowercase();
    let keep = |p: &ProcInfo| -> bool {
        if u.hide_system && (p.session == 0 || p.pid <= 4) {
            return false;
        }
        if u.only_mine && !p.st.user.to_lowercase().ends_with(&format!("\\{me}")) {
            return false;
        }
        true
    };
    let tree = s.config.tree_view && f.is_empty();
    if !tree {
        let mut pids: Vec<u32> = s.procs.values().filter(|p| keep(p) && matches(p, &f)).map(|p| p.pid).collect();
        sort_pids(s, &mut pids, &u.sort_col, u.asc);
        return pids.into_iter().map(|p| (p, 0, false)).collect();
    }
    let mut children: HashMap<u32, Vec<u32>> = HashMap::new();
    let mut roots = vec![];
    for p in s.procs.values() {
        if !keep(p) {
            continue;
        }
        let parent_ok = p.pid != 0
            && p.ppid != p.pid
            && s.procs.get(&p.ppid).map(|pp| pp.create_time <= p.create_time && keep(pp)).unwrap_or(false);
        if parent_ok {
            children.entry(p.ppid).or_default().push(p.pid);
        } else {
            roots.push(p.pid);
        }
    }
    sort_pids(s, &mut roots, &u.sort_col, u.asc);
    for v in children.values_mut() {
        sort_pids(s, v, &u.sort_col, u.asc);
    }
    let mut out = Vec::with_capacity(s.procs.len());
    fn walk(pid: u32, depth: usize, ch: &HashMap<u32, Vec<u32>>, col: &HashSet<u32>, out: &mut Vec<(u32, usize, bool)>) {
        let kids = ch.get(&pid);
        out.push((pid, depth, kids.map(|k| !k.is_empty()).unwrap_or(false)));
        if col.contains(&pid) || depth > 60 {
            return;
        }
        if let Some(k) = kids {
            for &c in k {
                walk(c, depth + 1, ch, col, out);
            }
        }
    }
    for r in roots {
        walk(r, 0, &children, &u.collapsed, &mut out);
    }
    out
}

pub fn kill_tree(app: &App, pid: u32) -> Result<usize, String> {
    let s = app.shared.lock().unwrap();
    let mut list = vec![pid];
    let mut i = 0;
    while i < list.len() {
        let cur = list[i];
        let cur_ct = s.procs.get(&cur).map(|p| p.create_time).unwrap_or(0);
        for p in s.procs.values() {
            if p.ppid == cur && p.pid != cur && p.create_time >= cur_ct && !list.contains(&p.pid) {
                list.push(p.pid);
            }
        }
        i += 1;
    }
    drop(s);
    let mut errs = vec![];
    for &p in list.iter().rev() {
        if let Err(e) = sys::terminate(p) {
            errs.push(format!("{p}: {e}"));
        }
    }
    if errs.len() == list.len() {
        Err(errs.join("; "))
    } else {
        Ok(list.len() - 1)
    }
}

pub fn page(app: &mut App, ui: &mut Ui) {
    let shared = app.shared.clone();
    // Header
    ui.horizontal(|ui| {
        ui.label(RichText::new("Processes").size(22.0).strong());
        ui.add_space(10.0);
        let sr = search_field(ui, &mut app.procs.search, "🔍 Search name, PID, path, user…   Ctrl+F", 300.0);
        if app.procs.focus_search {
            sr.request_focus();
            app.procs.focus_search = false;
        }
        if !app.procs.search.is_empty() && ui.small_button("✖").on_hover_text("Clear search").clicked() {
            app.procs.search.clear();
        }
        let mut s = shared.lock().unwrap();
        let mut tv = s.config.tree_view;
        if ui.toggle_value(&mut tv, "🌳 Tree").on_hover_text("Show which process started which").changed() {
            s.config.tree_view = tv;
            s.config_dirty = true;
        }
        drop(s);
        ui.toggle_value(&mut app.procs.show_columns, "☰ Columns").on_hover_text("Choose which details to show and in what order");
        let mut preset = None;
        ui.menu_button("Column sets ⏷", |ui| {
            for (name, cols) in PRESETS {
                if ui.button(*name).clicked() {
                    preset = Some(*cols);
                    ui.close_menu();
                }
            }
        });
        if let Some(cols) = preset {
            let mut s = shared.lock().unwrap();
            s.config.columns = cols.iter().map(|c| c.to_string()).collect();
            s.config_dirty = true;
        }
        let mut hold = app.procs.frozen.is_some();
        if ui.toggle_value(&mut hold, "📌 Hold order").on_hover_text("Stop rows jumping around while you look; values keep updating").changed() {
            app.procs.frozen = if hold { Some(vec![]) } else { None };
        }
        ui.checkbox(&mut app.procs.only_mine, "Only mine");
        ui.checkbox(&mut app.procs.hide_system, "Hide Windows services");
    });
    if app.procs.show_columns {
        column_picker(app, ui);
    }
    let selected = app.procs.selected;
    let show_detail = selected.map(|p| shared.lock().unwrap().procs.contains_key(&p)).unwrap_or(false);
    if show_detail {
        egui::TopBottomPanel::bottom("proc_detail")
            .resizable(true)
            .default_height(app.prefs.detail_height)
            .min_height(160.0)
            .frame(egui::Frame::none().fill(c_panel()).inner_margin(egui::Margin::same(10.0)).rounding(8.0))
            .show_inside(ui, |ui| {
                details(app, ui, selected.unwrap());
            });
    }
    let r = ui.available_rect_before_wrap();
    ui.allocate_ui_at_rect(r, |ui| {
        ui.set_clip_rect(r.intersect(ui.clip_rect()));
        table(app, ui);
    });
}

fn column_picker(app: &mut App, ui: &mut Ui) {
    let shared = app.shared.clone();
    let mut s = shared.lock().unwrap();
    let admin = app.admin();
    card(ui, "Columns", |ui| {
        ui.horizontal_wrapped(|ui| {
            for cd in COLUMNS {
                if cd.admin && !admin {
                    continue;
                }
                let mut on = s.config.columns.iter().any(|c| c == cd.id);
                if ui.checkbox(&mut on, cd.label).on_hover_text(cd.tip).changed() {
                    if on {
                        s.config.columns.push(cd.id.to_string());
                    } else if cd.id != "name" {
                        s.config.columns.retain(|c| c != cd.id);
                    }
                    s.config_dirty = true;
                }
            }
        });
        ui.add_space(4.0);
        ui.label(RichText::new("Order (left to right):").color(c_dim()).size(12.0));
        let mut mv: Option<(usize, i32)> = None;
        ui.horizontal_wrapped(|ui| {
            for (i, id) in s.config.columns.iter().enumerate() {
                let label = COLUMNS.iter().find(|c| c.id == id).map(|c| c.label).unwrap_or("?");
                egui::Frame::none().fill(c_card_hi()).rounding(6.0).inner_margin(egui::Margin::symmetric(4.0, 1.0)).show(ui, |ui| {
                    ui.horizontal(|ui| {
                        if ui.small_button("◀").clicked() {
                            mv = Some((i, -1));
                        }
                        ui.label(RichText::new(label).size(12.0));
                        if ui.small_button("▶").clicked() {
                            mv = Some((i, 1));
                        }
                        if id != "name" && ui.small_button("✖").clicked() {
                            mv = Some((i, 0));
                        }
                    });
                });
            }
        });
        if let Some((i, d)) = mv {
            if d == 0 {
                s.config.columns.remove(i);
            } else {
                let j = i as i32 + d;
                if j >= 0 && (j as usize) < s.config.columns.len() {
                    s.config.columns.swap(i, j as usize);
                }
            }
            s.config_dirty = true;
        }
        ui.horizontal(|ui| {
            if ui.button("Reset to default").clicked() {
                s.config.columns = default_columns();
                s.config_dirty = true;
            }
            if admin && ui.button("Show everything").clicked() {
                s.config.columns = COLUMNS.iter().map(|c| c.id.to_string()).collect();
                s.config_dirty = true;
            }
            if !admin {
                ui.label(RichText::new("🛡 30+ more columns (kernel time, page faults, pools, integrity, command line…) in the Admin view").color(PURPLE).size(12.0));
            }
        });
    });
    ui.add_space(4.0);
}

pub const PRESETS: &[(&str, &[&str])] = &[
    ("Everyday", &["name", "pid", "status", "cpu", "private", "ws", "disk", "gpu", "conns", "threads", "handles", "user", "description"]),
    ("Memory detail", &["name", "pid", "private", "ws", "private_ws", "peak_ws", "commit", "virtual", "paged_pool", "nonpaged_pool", "page_faults", "pf_sec", "hard_faults"]),
    ("Disk & I/O", &["name", "pid", "disk", "io_read", "io_write", "io_other", "read_total", "write_total", "read_ops", "write_ops"]),
    ("CPU & threads", &["name", "pid", "cpu", "cpu_kernel", "cpu_time", "cycles", "threads", "priority", "session"]),
    ("Graphics & network", &["name", "pid", "gpu", "gpu_mem", "conns", "cpu", "private"]),
    ("Security", &["name", "pid", "user", "integrity", "elevated", "arch", "session", "company", "path"]),
    ("Who made it", &["name", "pid", "description", "company", "version", "started", "path", "cmdline"]),
];

fn table(app: &mut App, ui: &mut Ui) {
    let shared = app.shared.clone();
    let s = shared.lock().unwrap();
    let admin = app.admin();
    let cols: Vec<&ColDef> = s
        .config
        .columns
        .iter()
        .filter_map(|id| COLUMNS.iter().find(|c| c.id == id))
        .filter(|c| admin || !c.admin)
        .collect();
    let mut rows = build_rows(app, &s);
    if let Some(fz) = app.procs.frozen.as_mut() {
        if fz.is_empty() {
            *fz = rows.clone();
        } else {
            let live: HashSet<u32> = rows.iter().map(|r| r.0).collect();
            fz.retain(|r| live.contains(&r.0));
            let known: HashSet<u32> = fz.iter().map(|r| r.0).collect();
            for r in &rows {
                if !known.contains(&r.0) {
                    fz.push(*r);
                }
            }
            rows = fz.clone();
        }
    }
    let total_mem = s.mem.st.total;
    let tree = s.config.tree_view && app.procs.search.trim().is_empty();
    ui.horizontal(|ui| {
        ui.label(RichText::new(format!("{} processes shown of {}  •  {} threads  •  {} handles", rows.len(), s.procs.len(), fmt_num(s.mem.st.threads as u64), fmt_num(s.mem.st.handles as u64))).color(c_dim()).size(12.0));
        if tree {
            if ui.small_button("Expand all").clicked() {
                app.procs.collapsed.clear();
            }
            if ui.small_button("Collapse all").clicked() {
                app.procs.collapsed = s.procs.keys().copied().collect();
            }
        }
    });
    let mut tb = egui_extras::TableBuilder::new(ui)
        .id_salt("proc_table")
        .striped(true)
        .resizable(true)
        .sense(Sense::click())
        .auto_shrink([false, false])
        .cell_layout(egui::Layout::left_to_right(egui::Align::Center));
    for (i, cd) in cols.iter().enumerate() {
        tb = if i + 1 == cols.len() {
            tb.column(egui_extras::Column::remainder().at_least(60.0).clip(true))
        } else {
            tb.column(egui_extras::Column::initial(cd.width).at_least(36.0).clip(true))
        };
    }
    let mut clicked_col: Option<&'static str> = None;
    let mut select: Option<u32> = None;
    let mut toggle: Option<u32> = None;
    let mut action: Option<(u32, MenuAction)> = None;
    let sort_col = app.procs.sort_col.clone();
    let asc = app.procs.asc;
    let sel = app.procs.selected;
    let collapsed = app.procs.collapsed.clone();
    tb.header(24.0, |mut h| {
        for cd in &cols {
            h.col(|ui| {
                let arrow = if sort_col == cd.id { if asc { " 🔼" } else { " 🔽" } } else { "" };
                let mut t = RichText::new(format!("{}{arrow}", cd.label)).strong().size(12.5);
                if sort_col == cd.id {
                    t = t.color(CYAN);
                }
                if ui.add(egui::Label::new(t).sense(Sense::click())).on_hover_text(cd.tip).clicked() {
                    clicked_col = Some(cd.id);
                }
            });
        }
    })
    .body(|body| {
        body.rows(row_h(), rows.len(), |mut row| {
            let (pid, depth, has_kids) = rows[row.index()];
            let Some(p) = s.procs.get(&pid) else { return };
            row.set_selected(sel == Some(pid));
            for cd in &cols {
                row.col(|ui| {
                    let bg = cell_heat(p, cd.id, total_mem);
                    if bg != Color32::TRANSPARENT {
                        ui.painter().rect_filled(ui.max_rect().expand2(egui::vec2(4.0, 1.0)), 0.0, bg);
                    }
                    if cd.id == "name" {
                        if tree {
                            ui.add_space(depth as f32 * 14.0);
                            if has_kids {
                                let icon = if collapsed.contains(&pid) { "⏵" } else { "⏷" };
                                if ui.add(egui::Label::new(RichText::new(icon).color(c_dim())).sense(Sense::click())).clicked() {
                                    toggle = Some(pid);
                                }
                            } else {
                                ui.add_space(12.0);
                            }
                        }
                        let color = if !look().row_colors {
                            c_text()
                        } else if p.suspended {
                            c_dim()
                        } else if p.st.elevated == Some(true) && p.session != 0 {
                            ORANGE
                        } else if p.session == 0 && pid > 4 {
                            Color32::from_rgb(150, 190, 255)
                        } else {
                            c_text()
                        };
                        ui.add(egui::Label::new(RichText::new(&p.name).color(color)).truncate());
                    } else {
                        let t = text(p, cd.id);
                        let rt = RichText::new(t).size(12.5);
                        let rt = if cd.id == "status" && p.suspended { rt.color(YELLOW) } else { rt };
                        ui.add(egui::Label::new(rt).truncate());
                    }
                });
            }
            let resp = row.response();
            if resp.clicked() {
                select = Some(pid);
            }
            resp.context_menu(|ui| {
                ui.label(RichText::new(format!("{} (PID {})", p.name, p.pid)).strong());
                ui.separator();
                if let Some(a) = action_menu(ui, admin, p.suspended) {
                    action = Some((pid, a));
                    ui.close_menu();
                }
            });
        });
    });
    drop(s);
    if let Some(c) = clicked_col {
        if app.procs.sort_col == c {
            app.procs.asc = !app.procs.asc;
        } else {
            app.procs.sort_col = c.to_string();
            app.procs.asc = matches!(c, "name" | "user" | "description" | "company" | "path" | "pid" | "status");
        }
    }
    if let Some(p) = select {
        app.procs.selected = Some(p);
    }
    if let Some(p) = toggle {
        if !app.procs.collapsed.remove(&p) {
            app.procs.collapsed.insert(p);
        }
    }
    if let Some((pid, a)) = action {
        do_action(app, ui.ctx(), pid, a);
    }
}

#[derive(Clone, Copy, PartialEq)]
pub enum MenuAction {
    End,
    EndTree,
    Suspend,
    Resume,
    Priority(Priority),
    Efficiency(bool),
    Affinity,
    Dump(bool),
    OpenLocation,
    Properties,
    CopyDetails,
    Inspect,
}

pub fn action_menu(ui: &mut Ui, admin: bool, suspended: bool) -> Option<MenuAction> {
    let mut a = None;
    if ui.button("✖ End task").clicked() {
        a = Some(MenuAction::End);
    }
    if ui.button("✖ End process tree").clicked() {
        a = Some(MenuAction::EndTree);
    }
    ui.menu_button("⏫ Set priority", |ui| {
        for p in Priority::ALL {
            if p == Priority::Realtime && !admin {
                continue;
            }
            if ui.button(p.label()).clicked() {
                a = Some(MenuAction::Priority(p));
                ui.close_menu();
            }
        }
    });
    ui.menu_button("🍃 Efficiency mode", |ui| {
        if ui.button("Turn on").clicked() {
            a = Some(MenuAction::Efficiency(true));
            ui.close_menu();
        }
        if ui.button("Turn off").clicked() {
            a = Some(MenuAction::Efficiency(false));
            ui.close_menu();
        }
    });
    if admin {
        ui.separator();
        if suspended {
            if ui.button("▶ Resume").clicked() {
                a = Some(MenuAction::Resume);
            }
        } else if ui.button("⏸ Suspend").clicked() {
            a = Some(MenuAction::Suspend);
        }
        if ui.button("🖥 Set CPU affinity…").clicked() {
            a = Some(MenuAction::Affinity);
        }
        ui.menu_button("💾 Create dump file", |ui| {
            if ui.button("Small (stacks and handles)").clicked() {
                a = Some(MenuAction::Dump(false));
                ui.close_menu();
            }
            if ui.button("Full memory").clicked() {
                a = Some(MenuAction::Dump(true));
                ui.close_menu();
            }
        });
        if ui.button("🔬 Open in inspector").clicked() {
            a = Some(MenuAction::Inspect);
        }
    }
    ui.separator();
    if ui.button("📂 Open file location").clicked() {
        a = Some(MenuAction::OpenLocation);
    }
    if ui.button("📄 File properties").clicked() {
        a = Some(MenuAction::Properties);
    }
    if ui.button("📋 Copy details").clicked() {
        a = Some(MenuAction::CopyDetails);
    }
    a
}

pub fn do_action(app: &mut App, ctx: &egui::Context, pid: u32, a: MenuAction) {
    let (name, path, summary) = {
        let s = app.shared.lock().unwrap();
        match s.procs.get(&pid) {
            Some(p) => (
                p.name.clone(),
                p.st.path.clone(),
                format!(
                    "Name: {}\nPID: {}\nParent PID: {}\nPath: {}\nCommand line: {}\nUser: {}\nCPU: {:.1}%\nPrivate memory: {}\nWorking set: {}\nThreads: {}\nHandles: {}\nDescription: {}\nPublisher: {}\nVersion: {}",
                    p.name, p.pid, p.ppid, p.st.path, p.st.cmdline, p.st.user, p.cpu, fmt_bytes(p.private_bytes as f64), fmt_bytes(p.ws as f64), p.threads, p.handles, p.st.description, p.st.company, p.st.version
                ),
            ),
            None => return,
        }
    };
    let confirm = app.shared.lock().unwrap().config.confirm_actions;
    match a {
        MenuAction::End => {
            if confirm {
                app.confirm = Some(Confirm::Kill(pid, name));
            } else {
                let r = sys::terminate(pid).map(|_| format!("Ended {name}"));
                app.result(r);
            }
        }
        MenuAction::EndTree => app.confirm = Some(Confirm::KillTree(pid, name)),
        MenuAction::Suspend => {
            if confirm {
                app.confirm = Some(Confirm::Suspend(pid, name));
            } else {
                let r = sys::suspend_process(pid, true).map(|_| format!("Suspended {name}"));
                app.result(r);
            }
        }
        MenuAction::Resume => {
            let r = sys::suspend_process(pid, false).map(|_| format!("Resumed {name}"));
            app.result(r);
        }
        MenuAction::Priority(p) => {
            let r = sys::set_priority(pid, p.class()).map(|_| format!("{name}: priority {}", p.label()));
            app.result(r);
            app.procs.detail.dyn_at = None;
        }
        MenuAction::Efficiency(on) => {
            let r = sys::set_efficiency(pid, on).map(|_| format!("{name}: efficiency mode {}", if on { "on" } else { "off" }));
            app.result(r);
            app.procs.detail.dyn_at = None;
        }
        MenuAction::Affinity => {
            app.procs.selected = Some(pid);
            app.procs.tab = Tab::General;
            let d = sys::query_dynamic(pid);
            app.procs.detail.affinity_edit = Some(if d.affinity == 0 { u64::MAX } else { d.affinity });
        }
        MenuAction::Dump(full) => {
            let path = util::sub_dir("dumps").join(format!("{}-{}-{}.dmp", name.trim_end_matches(".exe"), pid, util::stamp()));
            let p2 = path.clone();
            let ctx2 = ctx.clone();
            let n2 = name.clone();
            let shared = app.shared.clone();
            app.notify(format!("Writing dump of {name}…"), false);
            std::thread::spawn(move || {
                let r = sys::create_dump(pid, &p2, full);
                let mut s = shared.lock().unwrap();
                match r {
                    Ok(()) => s.log("Dump", crate::collector::Severity::Info, format!("Dump of {n2} saved to {}", p2.display()), Some(pid)),
                    Err(e) => s.log("Dump", crate::collector::Severity::Warning, format!("Dump of {n2} failed: {e}"), Some(pid)),
                }
                ctx2.request_repaint();
            });
            let _ = path;
        }
        MenuAction::OpenLocation => sys::open_file_location(&path),
        MenuAction::Properties => {
            if !path.is_empty() {
                sys::shell_open("properties", &path, "");
            }
        }
        MenuAction::CopyDetails => {
            ctx.output_mut(|o| o.copied_text = summary);
            app.notify("Details copied", false);
        }
        MenuAction::Inspect => {
            app.procs.selected = Some(pid);
            app.page = Page::Inspector;
        }
    }
}

// ------------------------------------------------------------------ details

fn ensure_detail(app: &mut App, pid: u32, create_time: i64) {
    let d = &mut app.procs.detail;
    if d.pid != pid || d.create_time != create_time {
        *d = Detail { pid, create_time, mem_committed: true, ..Default::default() };
    }
}

pub fn details(app: &mut App, ui: &mut Ui, pid: u32) {
    let shared = app.shared.clone();
    let ctx = ui.ctx().clone();
    let (ct, name, suspended) = {
        let s = shared.lock().unwrap();
        match s.procs.get(&pid) {
            Some(p) => (p.create_time, p.name.clone(), p.suspended),
            None => {
                ui.label("This process has exited.");
                return;
            }
        }
    };
    ensure_detail(app, pid, ct);
    let admin = app.admin();
    if !admin && app.procs.tab.admin() {
        app.procs.tab = Tab::General;
    }
    // refresh dynamic info every 2 s
    if app.procs.detail.dyn_at.map(|t| t.elapsed().as_secs() >= 2).unwrap_or(true) && !app.procs.detail.dynamic.is_running() {
        app.procs.detail.dyn_at = Some(Instant::now());
        app.procs.detail.dynamic.start(&ctx, move || Ok(sys::query_dynamic(pid)));
    }
    ui.horizontal(|ui| {
        ui.label(RichText::new(&name).size(17.0).strong().color(CYAN));
        ui.label(RichText::new(format!("PID {pid}")).color(c_dim()));
        if suspended {
            pill(ui, "Suspended", YELLOW);
        }
        ui.add_space(10.0);
        for t in [Tab::General, Tab::Performance, Tab::Threads, Tab::Modules, Tab::Memory, Tab::Handles, Tab::Network, Tab::Security, Tab::History] {
            if t.admin() && !admin {
                continue;
            }
            ui.selectable_value(&mut app.procs.tab, t, t.label());
        }
        if !admin {
            ui.label(RichText::new("🛡 +5 tabs in Admin view").color(PURPLE).size(11.5)).on_hover_text("Threads, modules, memory map, handles and security token");
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            if ui.button("✖").on_hover_text("Close details").clicked() {
                app.procs.selected = None;
            }
            let mut act = None;
            ui.menu_button("Actions ⏷", |ui| {
                if let Some(a) = action_menu(ui, admin, suspended) {
                    act = Some(a);
                    ui.close_menu();
                }
            });
            if ui.button(RichText::new("End task").color(RED)).clicked() {
                act = Some(MenuAction::End);
            }
            if let Some(a) = act {
                do_action(app, &ctx, pid, a);
            }
        });
    });
    ui.separator();
    egui::ScrollArea::vertical().id_salt("detail_scroll").auto_shrink([false, false]).show(ui, |ui| match app.procs.tab {
        Tab::General => tab_general(app, ui, pid),
        Tab::Performance => tab_perf(app, ui, pid),
        Tab::Threads => tab_threads(app, ui, pid, &ctx),
        Tab::Modules => tab_modules(app, ui, pid, &ctx),
        Tab::Memory => tab_memory(app, ui, pid, &ctx),
        Tab::Handles => tab_handles(app, ui, pid, &ctx),
        Tab::Network => tab_network(app, ui, pid, &ctx),
        Tab::Security => tab_security(app, ui),
        Tab::History => tab_history(app, ui, pid),
    });
}

fn tab_general(app: &mut App, ui: &mut Ui, pid: u32) {
    let shared = app.shared.clone();
    let s = shared.lock().unwrap();
    let Some(p) = s.procs.get(&pid) else { return };
    let parent = s.procs.get(&p.ppid).filter(|pp| pp.create_time <= p.create_time).map(|pp| format!("{} ({})", pp.name, pp.pid)).unwrap_or_else(|| format!("{} (exited)", p.ppid));
    let started = util::filetime_to_unix(p.create_time);
    let mut rows: Vec<(&str, String)> = vec![
        ("Description", p.st.description.clone()),
        ("Publisher", p.st.company.clone()),
        ("Version", p.st.version.clone()),
        ("Path", p.st.path.clone()),
        ("Command line", p.st.cmdline.clone()),
        ("User", p.st.user.clone()),
        ("Parent", parent),
        ("Started", format!("{} ({} ago)", util::fmt_time(started), util::fmt_dur((util::now_unix() - started).max(0) as u64))),
        ("CPU now", format!("{:.2}% (user {:.2}%, kernel {:.2}%)", p.cpu, p.cpu_user, p.cpu_kernel)),
        ("CPU time used", util::fmt_dur(p.cpu_time as u64)),
        ("Memory", format!("{} private, {} in RAM", fmt_bytes(p.private_bytes as f64), fmt_bytes(p.ws as f64))),
        ("Threads / handles", format!("{} / {}", p.threads, p.handles)),
        ("Session", p.session.to_string()),
    ];
    let admin = app.admin();
    if admin {
        rows.push(("Architecture", p.st.arch.clone()));
        rows.push(("Integrity", p.st.integrity.clone()));
        rows.push(("Elevated", yes_no(p.st.elevated)));
        rows.push(("Base priority", p.base_priority.to_string()));
        rows.push(("Peak threads", p.threads_peak.to_string()));
    }
    drop(s);
    let dynamic = app.procs.detail.dynamic.take_ok();
    ui.columns(2, |cols| {
        kv(&mut cols[0], "gen_kv", &rows);
        if let Some(d) = &dynamic {
            let mut r2: Vec<(&str, String)> = vec![
                ("Priority class", d.priority_class.clone()),
                ("Efficiency mode", d.efficiency_mode.map(|b| if b { "On" } else { "Off" }.to_string()).unwrap_or_else(|| "-".into())),
                ("GDI objects", format!("{} (peak {})", d.gdi_objects, d.gdi_peak)),
                ("USER objects", format!("{} (peak {})", d.user_objects, d.user_peak)),
            ];
            if admin {
                r2.push(("I/O priority", d.io_priority.clone()));
                r2.push(("Memory priority", d.page_priority.clone()));
                r2.push(("Protection", d.protection.clone()));
                r2.push(("Critical process", d.critical.map(|b| if b { "Yes (ending it crashes Windows)" } else { "No" }.to_string()).unwrap_or_default()));
                r2.push((
                    "CPU affinity",
                    if d.affinity == 0 {
                        "-".into()
                    } else if d.affinity == d.system_affinity {
                        "All processors".into()
                    } else {
                        format!("0x{:x} ({} of {} processors)", d.affinity, d.affinity.count_ones(), d.system_affinity.count_ones())
                    },
                ));
            }
            kv(&mut cols[1], "gen_kv2", &r2);
            if admin {
                if let Some(mask) = app.procs.detail.affinity_edit.as_mut() {
                    let n = d.system_affinity.count_ones().max(1);
                    cols[1].add_space(6.0);
                    cols[1].label(RichText::new("CPU affinity — tick the processors it may use").strong());
                    cols[1].horizontal_wrapped(|ui| {
                        for i in 0..n.min(64) {
                            let mut on = *mask & (1u64 << i) != 0;
                            if ui.checkbox(&mut on, format!("{i}")).changed() {
                                if on {
                                    *mask |= 1 << i;
                                } else {
                                    *mask &= !(1u64 << i);
                                }
                            }
                        }
                    });
                    let m = *mask & d.system_affinity;
                    let mut close = false;
                    cols[1].horizontal(|ui| {
                        if ui.button("Apply").clicked() && m != 0 {
                            let r = sys::set_affinity(pid, m).map(|_| "Affinity changed".to_string());
                            app.result(r);
                            close = true;
                        }
                        if ui.button("Cancel").clicked() {
                            close = true;
                        }
                    });
                    if close {
                        app.procs.detail.affinity_edit = None;
                        app.procs.detail.dyn_at = None;
                    }
                }
            }
        } else {
            cols[1].spinner();
        }
    });
}

fn tab_perf(app: &mut App, ui: &mut Ui, pid: u32) {
    let s = app.shared.lock().unwrap();
    let Some(p) = s.procs.get(&pid) else { return };
    let n = 120;
    ui.columns(2, |cols| {
        cols[0].label(RichText::new(format!("CPU {:.1}%", p.cpu)).color(CYAN));
        chart(&mut cols[0], &[Series { data: p.hist_cpu.to_vec(n), color: CYAN, label: "" }], 90.0, None, n, &|v| format!("{v:.1}%"));
        cols[0].label(RichText::new(format!("Disk {}", util::fmt_rate((p.io_read_rate + p.io_write_rate) as f64))).color(GREEN));
        chart(&mut cols[0], &[Series { data: p.hist_io.to_vec(n), color: GREEN, label: "" }], 90.0, None, n, &rate);
        cols[1].label(RichText::new(format!("Private memory {}", fmt_bytes(p.private_bytes as f64))).color(PURPLE));
        chart(&mut cols[1], &[Series { data: p.hist_mem.to_vec(n), color: PURPLE, label: "" }], 90.0, None, n, &bytes);
        cols[1].label(RichText::new(format!("Handles {}  •  Threads {}", p.handles, p.threads)).color(ORANGE));
        chart(
            &mut cols[1],
            &[Series { data: p.hist_handles.to_vec(n), color: ORANGE, label: "Handles" }],
            90.0,
            None,
            n,
            &|v| format!("{v:.0}"),
        );
    });
    ui.add_space(6.0);
    kv(
        ui,
        "io_kv",
        &[
            ("Reads", format!("{} in {} operations ({}/s now)", fmt_bytes(p.read_bytes as f64), fmt_num(p.read_ops), util::fmt_rate(p.io_read_rate as f64))),
            ("Writes", format!("{} in {} operations ({}/s now)", fmt_bytes(p.write_bytes as f64), fmt_num(p.write_ops), util::fmt_rate(p.io_write_rate as f64))),
            ("Other I/O", format!("{} in {} operations", fmt_bytes(p.other_bytes as f64), fmt_num(p.other_ops))),
            ("Page faults", format!("{} total, {:.0}/s now, {} hard", fmt_num(p.page_faults as u64), p.page_faults_sec, fmt_num(p.hard_faults as u64))),
            ("Working set", format!("{} (peak {}), private {}", fmt_bytes(p.ws as f64), fmt_bytes(p.peak_ws as f64), fmt_bytes(p.private_ws as f64))),
            ("Commit", format!("{} (peak {})", fmt_bytes(p.commit as f64), fmt_bytes(p.peak_commit as f64))),
            ("Virtual size", format!("{} (peak {})", fmt_bytes(p.virtual_size as f64), fmt_bytes(p.peak_virtual as f64))),
            ("Kernel pools", format!("paged {}, non-paged {}", fmt_bytes(p.paged_pool as f64), fmt_bytes(p.nonpaged_pool as f64))),
            ("GPU", format!("{:.1}%, dedicated {}, shared {}", p.gpu, fmt_bytes(p.gpu_dedicated as f64), fmt_bytes(p.gpu_shared as f64))),
            ("CPU cycles (last interval)", fmt_num(p.cycles_delta)),
        ],
    );
}

fn tab_threads(app: &mut App, ui: &mut Ui, pid: u32, ctx: &egui::Context) {
    let d = &mut app.procs.detail;
    d.modules.ensure(ctx, move || sys::modules(pid));
    let tids: Vec<u32> = app.shared.lock().unwrap().procs.get(&pid).map(|p| p.thread_list.iter().map(|t| t.raw.tid).collect()).unwrap_or_default();
    let d = &mut app.procs.detail;
    d.thread_extra.ensure(ctx, move || Ok(tids.iter().map(|&t| (t, sys::thread_extra(t))).collect()));
    let mods = d.modules.take_ok().unwrap_or_default();
    let extra = d.thread_extra.take_ok().unwrap_or_default();
    ui.horizontal(|ui| {
        if ui.button("⟳ Refresh start addresses").clicked() {
            app.procs.detail.thread_extra = Job::default();
            app.procs.detail.modules = Job::default();
        }
        if let Some(tid) = app.procs.detail.sel_thread {
            if ui.button(format!("⏸ Suspend thread {tid}")).clicked() {
                let r = sys::suspend_thread(tid, true).map(|_| "Thread suspended".to_string());
                app.result(r);
            }
            if ui.button(format!("▶ Resume thread {tid}")).clicked() {
                let r = sys::suspend_thread(tid, false).map(|_| "Thread resumed".to_string());
                app.result(r);
            }
        }
    });
    let s = app.shared.lock().unwrap();
    let Some(p) = s.procs.get(&pid) else { return };
    let mut threads = p.thread_list.clone();
    drop(s);
    threads.sort_by(|a, b| b.cpu.total_cmp(&a.cpu).then(b.cswitch_delta.cmp(&a.cswitch_delta)));
    let sel = app.procs.detail.sel_thread;
    let mut new_sel = sel;
    let heads = ["TID", "CPU", "State", "Wait reason", "Priority", "Base", "Switches/s", "Context switches", "Kernel time", "User time", "Start address", "Description", "Created"];
    let widths = [60.0, 56.0, 76.0, 120.0, 56.0, 44.0, 76.0, 106.0, 80.0, 80.0, 230.0, 140.0, 140.0];
    let mut tb = egui_extras::TableBuilder::new(ui).id_salt("threads").striped(true).resizable(true).sense(Sense::click()).auto_shrink([false, true]).max_scroll_height(900.0);
    for (i, w) in widths.iter().enumerate() {
        tb = if i + 1 == widths.len() { tb.column(egui_extras::Column::remainder()) } else { tb.column(egui_extras::Column::initial(*w).clip(true)) };
    }
    tb.header(20.0, |mut h| {
        for t in heads {
            h.col(|ui| {
                ui.add(egui::Label::new(RichText::new(t).strong()).truncate());
            });
        }
    })
    .body(|body| {
        body.rows(row_h(), threads.len(), |mut row| {
            let t = &threads[row.index()];
            let r = &t.raw;
            row.set_selected(sel == Some(r.tid));
            let (start, desc) = extra.get(&r.tid).cloned().unwrap_or((None, String::new()));
            let start_txt = sys::resolve_address(&mods, start.unwrap_or(r.start_address));
            let vals = [
                r.tid.to_string(),
                format!("{:.1}%", t.cpu),
                nt::thread_state_name(r.state).to_string(),
                if r.state == 5 { nt::wait_reason_name(r.wait_reason).to_string() } else { String::new() },
                r.priority.to_string(),
                r.base_priority.to_string(),
                t.cswitch_delta.to_string(),
                fmt_num(r.context_switches as u64),
                util::fmt_dur((r.kernel / 10_000_000) as u64),
                util::fmt_dur((r.user / 10_000_000) as u64),
                start_txt,
                desc,
                util::fmt_time(util::filetime_to_unix(r.create_time)),
            ];
            for (i, v) in vals.iter().enumerate() {
                row.col(|ui| {
                    let mut rt = RichText::new(v).size(12.0);
                    if i == 1 && t.cpu > 5.0 {
                        rt = rt.color(ORANGE);
                    }
                    if i == 3 && r.wait_reason == 5 {
                        rt = rt.color(YELLOW);
                    }
                    ui.add(egui::Label::new(rt).truncate());
                });
            }
            if row.response().clicked() {
                new_sel = Some(r.tid);
            }
        });
    });
    app.procs.detail.sel_thread = new_sel;
}

fn tab_modules(app: &mut App, ui: &mut Ui, pid: u32, ctx: &egui::Context) {
    let d = &mut app.procs.detail;
    d.modules.ensure(ctx, move || {
        let mut m = sys::modules(pid)?;
        for x in m.iter_mut() {
            let (desc, comp, ver) = sys::file_version_info(&x.path);
            x.description = desc;
            x.company = comp;
            x.version = ver;
        }
        Ok(m)
    });
    if !job_status(ui, &d.modules) {
        return;
    }
    let mods = d.modules.take_ok().unwrap_or_default();
    ui.horizontal(|ui| {
        search_field(ui, &mut app.procs.detail.mod_filter, "Filter modules…", 240.0);
        let total: u64 = mods.iter().map(|m| m.size as u64).sum();
        ui.label(RichText::new(format!("{} modules, {} mapped", mods.len(), fmt_bytes(total as f64))).color(c_dim()));
        let non_ms = mods.iter().filter(|m| !m.company.contains("Microsoft")).count();
        ui.label(RichText::new(format!("{non_ms} not from Microsoft")).color(if non_ms > 0 { YELLOW } else { c_dim() }));
        if ui.button("⟳").clicked() {
            app.procs.detail.modules = Job::default();
        }
    });
    let f = app.procs.detail.mod_filter.to_lowercase();
    let list: Vec<&sys::Module> = mods.iter().filter(|m| f.is_empty() || m.name.to_lowercase().contains(&f) || m.path.to_lowercase().contains(&f) || m.company.to_lowercase().contains(&f)).collect();
    let mut tb = egui_extras::TableBuilder::new(ui).id_salt("mods").striped(true).resizable(true).auto_shrink([false, true]).max_scroll_height(900.0);
    for w in [170.0, 130.0, 80.0, 200.0, 150.0, 110.0] {
        tb = tb.column(egui_extras::Column::initial(w).clip(true));
    }
    tb = tb.column(egui_extras::Column::remainder());
    tb.header(20.0, |mut h| {
        for t in ["Name", "Base address", "Size", "Description", "Publisher", "Version", "Path"] {
            h.col(|ui| {
                ui.add(egui::Label::new(RichText::new(t).strong()).truncate());
            });
        }
    })
    .body(|body| {
        body.rows(row_h(), list.len(), |mut row| {
            let m = list[row.index()];
            let vals = [m.name.clone(), format!("0x{:x}", m.base), fmt_bytes(m.size as f64), m.description.clone(), m.company.clone(), m.version.clone(), m.path.clone()];
            for (i, v) in vals.iter().enumerate() {
                row.col(|ui| {
                    let mut rt = RichText::new(v).size(12.0);
                    if i == 4 && !m.company.contains("Microsoft") {
                        rt = rt.color(YELLOW);
                    }
                    ui.add(egui::Label::new(rt).truncate());
                });
            }
        });
    });
}

fn tab_memory(app: &mut App, ui: &mut Ui, pid: u32, ctx: &egui::Context) {
    let d = &mut app.procs.detail;
    d.memmap.ensure(ctx, move || sys::memory_map(pid));
    if !job_status(ui, &d.memmap) {
        return;
    }
    let regions = d.memmap.take_ok().unwrap_or_default();
    // summary
    let mut sum: HashMap<&str, (u64, u64, usize)> = HashMap::new();
    let mut rwx = 0;
    for r in &regions {
        let e = sum.entry(sys::kind_name(r.kind)).or_default();
        if r.state == 0x1000 {
            e.0 += r.size as u64;
        } else {
            e.1 += r.size as u64;
        }
        e.2 += 1;
        if r.state == 0x1000 && (r.protect & 0xFF == 0x40 || r.protect & 0xFF == 0x80) {
            rwx += 1;
        }
    }
    ui.columns(4, |cols| {
        for (i, k) in ["Image", "Mapped", "Private"].iter().enumerate() {
            let (c, r, n) = sum.get(k).copied().unwrap_or_default();
            card(&mut cols[i], "", |ui| {
                ui.vertical(|ui| {
                    ui.label(RichText::new(*k).strong());
                    ui.label(format!("{} committed", fmt_bytes(c as f64)));
                    ui.label(RichText::new(format!("{} reserved, {n} regions", fmt_bytes(r as f64))).color(c_dim()).size(11.5));
                });
            });
        }
        card(&mut cols[3], "", |ui| {
            ui.vertical(|ui| {
                ui.label(RichText::new("Executable + writable").strong());
                ui.label(RichText::new(format!("{rwx} regions")).color(if rwx > 0 { YELLOW } else { GREEN }));
                ui.label(RichText::new("Often JIT code; rarely injected code").color(c_dim()).size(11.5));
            });
        });
    });
    ui.horizontal(|ui| {
        search_field(ui, &mut app.procs.detail.mem_filter, "Filter by file, type or protection…", 260.0);
        ui.checkbox(&mut app.procs.detail.mem_committed, "Committed only");
        if ui.button("⟳").clicked() {
            app.procs.detail.memmap = Job::default();
        }
    });
    let f = app.procs.detail.mem_filter.to_lowercase();
    let only_c = app.procs.detail.mem_committed;
    let list: Vec<&sys::Region> = regions
        .iter()
        .filter(|r| (!only_c || r.state == 0x1000) && (f.is_empty() || r.mapped.to_lowercase().contains(&f) || sys::kind_name(r.kind).to_lowercase().contains(&f) || sys::protect_name(r.protect).to_lowercase().contains(&f)))
        .collect();
    let mut tb = egui_extras::TableBuilder::new(ui).id_salt("memmap").striped(true).resizable(true).auto_shrink([false, true]).max_scroll_height(900.0);
    for w in [140.0, 90.0, 70.0, 70.0, 80.0, 140.0, 80.0] {
        tb = tb.column(egui_extras::Column::initial(w).clip(true));
    }
    tb = tb.column(egui_extras::Column::remainder());
    tb.header(20.0, |mut h| {
        for t in ["Base address", "Size", "State", "Type", "Protection", "Allocation base", "Initial prot.", "Mapped file"] {
            h.col(|ui| {
                ui.add(egui::Label::new(RichText::new(t).strong()).truncate());
            });
        }
    })
    .body(|body| {
        body.rows(row_h(), list.len(), |mut row| {
            let r = list[row.index()];
            let vals = [
                format!("0x{:012x}", r.base),
                fmt_bytes(r.size as f64),
                sys::state_name(r.state).to_string(),
                sys::kind_name(r.kind).to_string(),
                sys::protect_name(r.protect),
                format!("0x{:x}", r.alloc_base),
                sys::protect_name(r.alloc_protect),
                r.mapped.clone(),
            ];
            for (i, v) in vals.iter().enumerate() {
                row.col(|ui| {
                    let mut rt = RichText::new(v).size(12.0).monospace();
                    if i == 4 && v.starts_with("RWX") {
                        rt = rt.color(YELLOW);
                    }
                    ui.add(egui::Label::new(rt).truncate());
                });
            }
        });
    });
}

fn tab_handles(app: &mut App, ui: &mut Ui, pid: u32, ctx: &egui::Context) {
    let d = &mut app.procs.detail;
    d.handles.ensure(ctx, move || {
        let types = nt::query_object_types();
        sys::handles(pid, &types)
    });
    if !job_status(ui, &d.handles) {
        return;
    }
    let hs = d.handles.take_ok().unwrap_or_default();
    let mut by_type: HashMap<&str, usize> = HashMap::new();
    for h in &hs {
        *by_type.entry(h.type_name.as_str()).or_default() += 1;
    }
    let mut bt: Vec<_> = by_type.into_iter().collect();
    bt.sort_by(|a, b| b.1.cmp(&a.1));
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new(format!("{} handles:", hs.len())).strong());
        for (t, n) in bt.iter().take(14) {
            if ui.add(egui::Button::new(format!("{t} {n}")).small()).clicked() {
                app.procs.detail.handle_filter = t.to_string();
            }
        }
    });
    ui.horizontal(|ui| {
        search_field(ui, &mut app.procs.detail.handle_filter, "Filter by type or name…", 260.0);
        if ui.button("⟳").clicked() {
            app.procs.detail.handles = Job::default();
        }
    });
    let f = app.procs.detail.handle_filter.to_lowercase();
    let list: Vec<&sys::HandleInfo> = hs.iter().filter(|h| f.is_empty() || h.type_name.to_lowercase().contains(&f) || h.name.to_lowercase().contains(&f)).collect();
    let mut tb = egui_extras::TableBuilder::new(ui).id_salt("handles").striped(true).resizable(true).auto_shrink([false, true]).max_scroll_height(900.0);
    for w in [80.0, 120.0, 100.0, 150.0] {
        tb = tb.column(egui_extras::Column::initial(w).clip(true));
    }
    tb = tb.column(egui_extras::Column::remainder());
    tb.header(20.0, |mut h| {
        for t in ["Handle", "Type", "Access", "Object address", "Name"] {
            h.col(|ui| {
                ui.add(egui::Label::new(RichText::new(t).strong()).truncate());
            });
        }
    })
    .body(|body| {
        body.rows(row_h(), list.len(), |mut row| {
            let h = list[row.index()];
            let vals = [format!("0x{:x}", h.handle), h.type_name.clone(), format!("0x{:08x}", h.access), if h.object == 0 { "-".into() } else { format!("0x{:x}", h.object) }, h.name.clone()];
            for v in vals {
                row.col(|ui| {
                    ui.add(egui::Label::new(RichText::new(v).size(12.0)).truncate());
                });
            }
        });
    });
}

fn tab_network(app: &mut App, ui: &mut Ui, pid: u32, ctx: &egui::Context) {
    let d = &mut app.procs.detail;
    if d.conns_at.map(|t| t.elapsed().as_secs() >= 2).unwrap_or(true) && !d.conns.is_running() {
        d.conns_at = Some(Instant::now());
        d.conns.start(ctx, move || Ok(sys::connections().into_iter().filter(|c| c.pid == pid).collect()));
    }
    let conns = d.conns.take_ok().unwrap_or_default();
    let est = conns.iter().filter(|c| c.state == "Established").count();
    let listen = conns.iter().filter(|c| c.state == "Listening" || c.proto.starts_with("UDP")).count();
    ui.label(RichText::new(format!("{} endpoints: {est} connected, {listen} listening/open ports", conns.len())).color(c_dim()));
    if conns.is_empty() {
        ui.label("No open network connections.");
        return;
    }
    let mut tb = egui_extras::TableBuilder::new(ui).id_salt("conns").striped(true).resizable(true).auto_shrink([false, true]).max_scroll_height(900.0);
    for w in [70.0, 240.0, 260.0] {
        tb = tb.column(egui_extras::Column::initial(w).clip(true));
    }
    tb = tb.column(egui_extras::Column::remainder());
    tb.header(20.0, |mut h| {
        for t in ["Protocol", "Local address", "Remote address", "State"] {
            h.col(|ui| {
                ui.add(egui::Label::new(RichText::new(t).strong()).truncate());
            });
        }
    })
    .body(|body| {
        body.rows(row_h(), conns.len(), |mut row| {
            let c = &conns[row.index()];
            for (i, v) in [c.proto.to_string(), c.local.clone(), c.remote.clone(), c.state.to_string()].iter().enumerate() {
                row.col(|ui| {
                    let mut rt = RichText::new(v).size(12.0);
                    if i == 3 && c.state == "Established" {
                        rt = rt.color(GREEN);
                    }
                    ui.label(rt);
                });
            }
        });
    });
}

fn yes_no(v: Option<bool>) -> String {
    match v {
        Some(true) => "Yes".into(),
        Some(false) => "No".into(),
        None => "-".into(),
    }
}

fn tab_security(app: &mut App, ui: &mut Ui) {
    let Some(d) = app.procs.detail.dynamic.take_ok() else {
        ui.spinner();
        return;
    };
    let st = {
        let s = app.shared.lock().unwrap();
        s.procs.get(&app.procs.detail.pid).map(|p| p.st.clone())
    };
    if let Some(st) = st {
        kv(
            ui,
            "sec_kv",
            &[
                ("User", st.user.clone()),
                ("SID", st.sid.clone()),
                ("Integrity level", st.integrity.clone()),
                ("Elevated", yes_no(st.elevated)),
                ("Token type", d.token_type.clone()),
                ("UAC virtualization", yes_no(d.virtualization)),
                ("App container (sandbox)", yes_no(d.app_container)),
                ("Protection", d.protection.clone()),
            ],
        );
    }
    ui.add_space(6.0);
    ui.columns(2, |cols| {
        cols[0].label(RichText::new(format!("Privileges ({})", d.privileges.len())).strong());
        egui::Grid::new("privs").striped(true).show(&mut cols[0], |ui| {
            for (n, s) in &d.privileges {
                ui.label(n);
                ui.label(RichText::new(s).color(if s.starts_with("Enabled") { GREEN } else { c_dim() }));
                ui.end_row();
            }
        });
        cols[1].label(RichText::new(format!("Groups ({})", d.groups.len())).strong());
        egui::Grid::new("groups").striped(true).show(&mut cols[1], |ui| {
            for (n, s) in &d.groups {
                ui.label(n);
                ui.label(RichText::new(s).color(c_dim()));
                ui.end_row();
            }
        });
    });
}

fn tab_history(app: &mut App, ui: &mut Ui, pid: u32) {
    let s = app.shared.lock().unwrap();
    let Some(p) = s.procs.get(&pid) else { return };
    let n = p.hist_cpu.len();
    ui.label(RichText::new(format!("Last {} samples since it was first seen at {}", n, util::fmt_time(p.first_seen))).color(c_dim()));
    kv(
        ui,
        "hist_kv",
        &[
            ("Peak CPU", format!("{:.1}%", p.peak_cpu)),
            ("Peak private memory", fmt_bytes(p.hist_mem.data.iter().fold(0.0f32, |a, &b| a.max(b)) as f64)),
            ("Peak working set", fmt_bytes(p.peak_ws as f64)),
            ("Peak threads", p.threads_peak.to_string()),
            ("Average CPU", format!("{:.2}%", p.hist_cpu.avg(n))),
        ],
    );
    chart(
        ui,
        &[Series { data: p.hist_cpu.to_vec(n), color: CYAN, label: "CPU" }],
        110.0,
        None,
        crate::collector::HIST_PROC,
        &|v| format!("{v:.1}%"),
    );
    chart(ui, &[Series { data: p.hist_mem.to_vec(n), color: PURPLE, label: "Memory" }], 110.0, None, crate::collector::HIST_PROC, &bytes);
    ui.label(RichText::new("Events for this process").strong());
    for e in s.timeline.iter().rev().filter(|e| e.pid == Some(pid)).take(40) {
        ui.label(format!("{}  {}", util::fmt_time(e.time), e.text));
    }
}

// ------------------------------------------------------------------ history page

pub fn history_page(app: &mut App, ui: &mut Ui) {
    page_header(ui, "Process history", "Every process that started or exited while this app was running, with peaks and lifetime.");
    search_field(ui, &mut app.procs.hist_filter, "Filter…", 300.0);
    let s = app.shared.lock().unwrap();
    let f = app.procs.hist_filter.to_lowercase();
    ui.columns(2, |cols| {
        let ui = &mut cols[0];
        ui.label(RichText::new(format!("Exited processes ({})", s.exited.len())).strong());
        let list: Vec<_> = s.exited.iter().rev().filter(|e| f.is_empty() || e.name.to_lowercase().contains(&f) || e.path.to_lowercase().contains(&f)).collect();
        let mut tb = egui_extras::TableBuilder::new(ui).id_salt("exited").striped(true).resizable(true).auto_shrink([false, false]);
        for w in [150.0, 55.0, 130.0, 80.0, 60.0, 80.0] {
            tb = tb.column(egui_extras::Column::initial(w).clip(true));
        }
        tb = tb.column(egui_extras::Column::remainder());
        tb.header(20.0, |mut h| {
            for t in ["Name", "PID", "Exited", "Lifetime", "Peak CPU", "Peak memory", "Path"] {
                h.col(|ui| {
                    ui.add(egui::Label::new(RichText::new(t).strong()).truncate());
                });
            }
        })
        .body(|body| {
            body.rows(row_h(), list.len(), |mut row| {
                let e = list[row.index()];
                let vals = [
                    e.name.clone(),
                    e.pid.to_string(),
                    util::fmt_time(e.exited),
                    util::fmt_dur((e.exited - e.started).max(0) as u64),
                    format!("{:.1}%", e.peak_cpu),
                    fmt_bytes(e.peak_private as f64),
                    e.path.clone(),
                ];
                for v in vals {
                    row.col(|ui| {
                        ui.add(egui::Label::new(RichText::new(v).size(12.0)).truncate());
                    });
                }
            });
        });
        let ui = &mut cols[1];
        ui.label(RichText::new("Starts and exits").strong());
        egui::ScrollArea::vertical().id_salt("pevents").auto_shrink([false, false]).show(ui, |ui| {
            for e in s.timeline.iter().rev().filter(|e| e.category == "Process" && (f.is_empty() || e.text.to_lowercase().contains(&f))).take(500) {
                let c = if e.text.starts_with("Started") { GREEN } else { ORANGE };
                ui.horizontal(|ui| {
                    ui.label(RichText::new(util::fmt_clock(e.time)).color(c_dim()).monospace());
                    ui.label(RichText::new(&e.text).color(c).size(12.5));
                });
            }
        });
    });
}
