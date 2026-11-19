//! Disk space analysis: scan a drive or folder into a size tree.
//!
//! Two scanners:
//! * **Fast NTFS scan** (administrator, local NTFS drive root): reads the volume's
//!   Master File Table directly — every file record in one sequential pass.
//! * **Folder walk** (everything else): several threads enumerate folders with
//!   `FindFirstFileExW` (large fetch, basic info).
//!
//! Both produce a flat list of entries (parent before child), which `build` turns into
//! a tree with folder totals, file-type statistics and size-sorted children.

use crate::util::{self, wide};
use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, Mutex};
use std::time::Instant;

pub const NONE: u32 = u32::MAX;

#[derive(Clone, Debug)]
pub struct Node {
    pub name: Box<str>,
    pub parent: u32,
    pub size: u64,
    pub alloc: u64,
    /// FILETIME of last modification (for folders: newest inside).
    pub modified: i64,
    pub files: u32,
    pub dirs: u32,
    pub is_dir: bool,
    pub ext: u32,
    pub children: Vec<u32>,
}

#[derive(Clone, Debug)]
pub struct ExtStat {
    pub ext: String,
    pub size: u64,
    pub alloc: u64,
    pub count: u64,
    pub color: egui::Color32,
}

pub struct Tree {
    pub root_path: String,
    pub nodes: Vec<Node>,
    pub exts: Vec<ExtStat>,
    pub method: &'static str,
    pub secs: f64,
    pub errors: u64,
    pub total_space: u64,
    pub free_space: u64,
    pub id: u64,
}

#[derive(Default)]
pub struct Progress {
    pub files: AtomicU64,
    pub dirs: AtomicU64,
    pub bytes: AtomicU64,
    pub errors: AtomicU64,
    pub cancel: AtomicBool,
    pub phase: Mutex<String>,
}

impl Progress {
    fn set_phase(&self, s: &str) {
        *self.phase.lock().unwrap() = s.to_string();
    }
}

/// One scanned entry; `parent` always refers to an earlier entry (or NONE for the root).
struct Raw {
    name: Box<str>,
    parent: u32,
    size: u64,
    alloc: u64,
    modified: i64,
    is_dir: bool,
}

static NEXT_ID: AtomicU64 = AtomicU64::new(1);

const PALETTE: [egui::Color32; 20] = [
    egui::Color32::from_rgb(66, 135, 245),
    egui::Color32::from_rgb(236, 92, 92),
    egui::Color32::from_rgb(76, 201, 120),
    egui::Color32::from_rgb(245, 176, 66),
    egui::Color32::from_rgb(166, 110, 245),
    egui::Color32::from_rgb(52, 196, 214),
    egui::Color32::from_rgb(240, 118, 196),
    egui::Color32::from_rgb(163, 207, 62),
    egui::Color32::from_rgb(255, 140, 70),
    egui::Color32::from_rgb(110, 150, 255),
    egui::Color32::from_rgb(214, 70, 120),
    egui::Color32::from_rgb(40, 170, 150),
    egui::Color32::from_rgb(200, 160, 90),
    egui::Color32::from_rgb(140, 90, 200),
    egui::Color32::from_rgb(90, 200, 250),
    egui::Color32::from_rgb(230, 210, 70),
    egui::Color32::from_rgb(120, 220, 170),
    egui::Color32::from_rgb(250, 120, 120),
    egui::Color32::from_rgb(150, 120, 90),
    egui::Color32::from_rgb(100, 180, 100),
];

/// Colour-blind friendly alternative palette (blue/orange/yellow/grey family).
pub const PALETTE_CB: [egui::Color32; 8] = [
    egui::Color32::from_rgb(0, 114, 178),
    egui::Color32::from_rgb(230, 159, 0),
    egui::Color32::from_rgb(86, 180, 233),
    egui::Color32::from_rgb(240, 228, 66),
    egui::Color32::from_rgb(213, 94, 0),
    egui::Color32::from_rgb(0, 158, 115),
    egui::Color32::from_rgb(204, 121, 167),
    egui::Color32::from_rgb(160, 160, 160),
];

pub fn is_drive_root(p: &str) -> Option<char> {
    let b = p.as_bytes();
    if (b.len() == 2 || b.len() == 3) && b[0].is_ascii_alphabetic() && b[1] == b':' && (b.len() == 2 || b[2] == b'\\' || b[2] == b'/') {
        Some((b[0] as char).to_ascii_uppercase())
    } else {
        None
    }
}

fn normalize(p: &str) -> String {
    let mut s = p.trim().trim_matches('"').replace('/', "\\");
    if let Some(d) = is_drive_root(&s) {
        return format!("{d}:\\");
    }
    while s.len() > 3 && s.ends_with('\\') {
        s.pop();
    }
    s
}

fn fs_name(root: &str) -> String {
    use windows_sys::Win32::Storage::FileSystem::GetVolumeInformationW;
    let w = wide(root);
    let mut fs = vec![0u16; 64];
    let ok = unsafe { GetVolumeInformationW(w.as_ptr(), std::ptr::null_mut(), 0, std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut(), fs.as_mut_ptr(), 64) };
    if ok == 0 {
        String::new()
    } else {
        util::from_wide(&fs)
    }
}

fn space(root: &str) -> (u64, u64, u64) {
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDiskFreeSpaceW};
    let w = wide(root);
    let (mut avail, mut total, mut free) = (0u64, 0u64, 0u64);
    let (mut spc, mut bps, mut fc, mut tc) = (0u32, 0u32, 0u32, 0u32);
    unsafe {
        GetDiskFreeSpaceExW(w.as_ptr(), &mut avail, &mut total, &mut free);
        let cluster = if GetDiskFreeSpaceW(w.as_ptr(), &mut spc, &mut bps, &mut fc, &mut tc) != 0 { (spc as u64 * bps as u64).max(512) } else { 4096 };
        (total, free, cluster)
    }
}

/// Scan a drive or folder. `elevated` enables the fast NTFS scanner for drive roots.
pub fn scan(path: &str, elevated: bool, prog: &Progress) -> Result<Tree, String> {
    let root = normalize(path);
    if root.is_empty() {
        return Err("Pick a drive or folder first".into());
    }
    let t0 = Instant::now();
    let volume_root = if let Some(d) = is_drive_root(&root) { format!("{d}:\\") } else { root.chars().take(3).collect() };
    let (total, free, cluster) = space(&volume_root);
    let mut method = "Folder walk";
    let mut raws = None;
    if let (Some(d), true) = (is_drive_root(&root), elevated) {
        if fs_name(&root).eq_ignore_ascii_case("NTFS") {
            prog.set_phase("Reading the drive's file table…");
            match mft_scan(d, prog) {
                Ok(r) => {
                    raws = Some(r);
                    method = "Fast NTFS scan";
                }
                Err(e) => {
                    prog.set_phase(&format!("Fast scan unavailable ({e}); walking folders…"));
                    prog.files.store(0, Ordering::Relaxed);
                }
            }
        }
    }
    let raws = match raws {
        Some(r) => r,
        None => {
            if !std::path::Path::new(&root).exists() {
                return Err(format!("{root} was not found"));
            }
            prog.set_phase("Walking folders…");
            walk_scan(&root, cluster, prog)?
        }
    };
    if prog.cancel.load(Ordering::Relaxed) {
        return Err("Scan cancelled".into());
    }
    prog.set_phase("Adding up folders…");
    let mut tree = build(raws, root, prog.errors.load(Ordering::Relaxed));
    tree.method = method;
    tree.total_space = total;
    tree.free_space = free;
    tree.secs = t0.elapsed().as_secs_f64();
    Ok(tree)
}

fn ext_of(name: &str) -> String {
    match name.rfind('.') {
        Some(i) if i > 0 && i + 1 < name.len() && name.len() - i <= 16 => name[i + 1..].to_lowercase(),
        _ => String::new(),
    }
}

fn build(raws: Vec<Raw>, root_path: String, errors: u64) -> Tree {
    let n = raws.len();
    let mut ext_idx: HashMap<String, u32> = HashMap::new();
    let mut ext_raw: Vec<(String, u64, u64, u64)> = vec![];
    let mut nodes: Vec<Node> = Vec::with_capacity(n);
    for r in raws {
        let ext = if r.is_dir {
            NONE
        } else {
            let e = ext_of(&r.name);
            let id = *ext_idx.entry(e.clone()).or_insert_with(|| {
                ext_raw.push((e, 0, 0, 0));
                (ext_raw.len() - 1) as u32
            });
            let s = &mut ext_raw[id as usize];
            s.1 += r.size;
            s.2 += r.alloc;
            s.3 += 1;
            id
        };
        nodes.push(Node {
            name: r.name,
            parent: r.parent,
            size: r.size,
            alloc: r.alloc,
            modified: r.modified,
            files: if r.is_dir { 0 } else { 1 },
            dirs: 0,
            is_dir: r.is_dir,
            ext,
            children: vec![],
        });
    }
    // children lists
    for i in 1..n {
        let p = nodes[i].parent;
        if p != NONE && (p as usize) < i {
            nodes[p as usize].children.push(i as u32);
        }
    }
    // totals bottom-up (children always come after their parent)
    for i in (1..n).rev() {
        let p = nodes[i].parent;
        if p == NONE || p as usize >= i {
            continue;
        }
        let (s, a, f, d, m, is_dir) = { let c = &nodes[i]; (c.size, c.alloc, c.files, c.dirs, c.modified, c.is_dir) };
        let par = &mut nodes[p as usize];
        par.size += s;
        par.alloc += a;
        par.files += f;
        par.dirs += d + if is_dir { 1 } else { 0 };
        if m > par.modified {
            par.modified = m;
        }
    }
    // sort children biggest first
    let sizes: Vec<u64> = nodes.iter().map(|x| x.size).collect();
    for node in nodes.iter_mut() {
        if node.children.len() > 1 {
            node.children.sort_unstable_by(|a, b| sizes[*b as usize].cmp(&sizes[*a as usize]));
        }
    }
    // file types, biggest first, with colours
    let mut order: Vec<usize> = (0..ext_raw.len()).collect();
    order.sort_by(|a, b| ext_raw[*b].1.cmp(&ext_raw[*a].1));
    let mut remap = vec![0u32; ext_raw.len()];
    let mut exts = Vec::with_capacity(ext_raw.len());
    for (rank, &i) in order.iter().enumerate() {
        remap[i] = rank as u32;
        let (e, s, a, c) = &ext_raw[i];
        let color = if rank < PALETTE.len() {
            PALETTE[rank]
        } else {
            let h = e.bytes().fold(5381u32, |h, b| h.wrapping_mul(33) ^ b as u32);
            let v = 110 + (h % 70) as u8;
            egui::Color32::from_rgb(v, v.saturating_add((h >> 8) as u8 % 20), v.saturating_add((h >> 16) as u8 % 25))
        };
        exts.push(ExtStat { ext: e.clone(), size: *s, alloc: *a, count: *c, color });
    }
    for node in nodes.iter_mut() {
        if node.ext != NONE {
            node.ext = remap[node.ext as usize];
        }
    }
    Tree { root_path, nodes, exts, method: "", secs: 0.0, errors, total_space: 0, free_space: 0, id: NEXT_ID.fetch_add(1, Ordering::Relaxed) }
}

// ------------------------------------------------------------------ folder walk

struct WalkShared {
    nodes: Mutex<Vec<Raw>>,
    queue: Mutex<(Vec<(u32, String)>, usize)>,
    cv: Condvar,
}

fn list_path(dir: &str) -> String {
    let base = if let Some(rest) = dir.strip_prefix("\\\\") {
        if rest.starts_with("?\\") { dir.to_string() } else { format!("\\\\?\\UNC\\{rest}") }
    } else {
        format!("\\\\?\\{dir}")
    };
    if base.ends_with('\\') {
        format!("{base}*")
    } else {
        format!("{base}\\*")
    }
}

fn join(dir: &str, name: &str) -> String {
    if dir.ends_with('\\') {
        format!("{dir}{name}")
    } else {
        format!("{dir}\\{name}")
    }
}

fn walk_scan(root: &str, cluster: u64, prog: &Progress) -> Result<Vec<Raw>, String> {
    let shared = Arc::new(WalkShared {
        nodes: Mutex::new(vec![Raw { name: root.into(), parent: NONE, size: 0, alloc: 0, modified: 0, is_dir: true }]),
        queue: Mutex::new((vec![(0, root.to_string())], 0)),
        cv: Condvar::new(),
    });
    let threads = std::thread::available_parallelism().map(|n| n.get()).unwrap_or(4).clamp(2, 12);
    std::thread::scope(|sc| {
        for _ in 0..threads {
            let sh = shared.clone();
            sc.spawn(move || walk_worker(&sh, cluster, prog));
        }
    });
    if prog.cancel.load(Ordering::Relaxed) {
        return Err("Scan cancelled".into());
    }
    let sh = Arc::try_unwrap(shared).map_err(|_| "internal error".to_string())?;
    Ok(sh.nodes.into_inner().unwrap())
}

fn walk_worker(sh: &WalkShared, cluster: u64, prog: &Progress) {
    use windows_sys::Win32::Storage::FileSystem::{FindClose, FindFirstFileExW, FindNextFileW, WIN32_FIND_DATAW};
    loop {
        let job = {
            let mut q = sh.queue.lock().unwrap();
            loop {
                if prog.cancel.load(Ordering::Relaxed) {
                    break None;
                }
                if let Some(j) = q.0.pop() {
                    q.1 += 1;
                    break Some(j);
                }
                if q.1 == 0 {
                    break None;
                }
                q = sh.cv.wait_timeout(q, std::time::Duration::from_millis(50)).unwrap().0;
            }
        };
        let Some((idx, dir)) = job else {
            sh.cv.notify_all();
            return;
        };
        let mut entries: Vec<(Box<str>, bool, u64, i64)> = Vec::new();
        let pattern = wide(&list_path(&dir));
        unsafe {
            let mut fd: WIN32_FIND_DATAW = std::mem::zeroed();
            let h = FindFirstFileExW(pattern.as_ptr(), 1, &mut fd as *mut _ as *mut core::ffi::c_void, 0, std::ptr::null(), 2);
            if h as isize == -1 {
                prog.errors.fetch_add(1, Ordering::Relaxed);
            } else {
                loop {
                    let name = util::from_wide(&fd.cFileName);
                    if name != "." && name != ".." {
                        let is_dir = fd.dwFileAttributes & 0x10 != 0;
                        let reparse = fd.dwFileAttributes & 0x400 != 0;
                        let size = ((fd.nFileSizeHigh as u64) << 32) | fd.nFileSizeLow as u64;
                        let mt = ((fd.ftLastWriteTime.dwHighDateTime as i64) << 32) | fd.ftLastWriteTime.dwLowDateTime as i64;
                        // Skip junctions/symlinked folders so nothing is counted twice.
                        if !(is_dir && reparse) {
                            entries.push((name.into_boxed_str(), is_dir, if is_dir { 0 } else { size }, mt));
                        }
                    }
                    if FindNextFileW(h, &mut fd) == 0 {
                        break;
                    }
                }
                FindClose(h);
            }
        }
        let mut subdirs = Vec::new();
        {
            let mut nodes = sh.nodes.lock().unwrap();
            let mut files = 0u64;
            let mut bytes = 0u64;
            for (name, is_dir, size, mt) in entries {
                let i = nodes.len() as u32;
                if is_dir {
                    subdirs.push((i, join(&dir, &name)));
                } else {
                    files += 1;
                    bytes += size;
                }
                let alloc = if is_dir || size == 0 { 0 } else { size.div_ceil(cluster) * cluster };
                nodes.push(Raw { name, parent: idx, size, alloc, modified: mt, is_dir });
            }
            prog.files.fetch_add(files, Ordering::Relaxed);
            prog.bytes.fetch_add(bytes, Ordering::Relaxed);
        }
        prog.dirs.fetch_add(1, Ordering::Relaxed);
        let mut q = sh.queue.lock().unwrap();
        q.0.extend(subdirs);
        q.1 -= 1;
        drop(q);
        sh.cv.notify_all();
    }
}

// ------------------------------------------------------------------ fast NTFS scan

fn rd16(b: &[u8], o: usize) -> u16 {
    u16::from_le_bytes([b[o], b[o + 1]])
}
fn rd32(b: &[u8], o: usize) -> u32 {
    u32::from_le_bytes(b[o..o + 4].try_into().unwrap())
}
fn rd64(b: &[u8], o: usize) -> u64 {
    u64::from_le_bytes(b[o..o + 8].try_into().unwrap())
}

/// Decode a non-resident attribute's data runs into (lcn, clusters) pairs.
fn data_runs(b: &[u8]) -> Vec<(i64, u64)> {
    let mut out = vec![];
    let mut i = 0usize;
    let mut lcn: i64 = 0;
    while i < b.len() && b[i] != 0 {
        let h = b[i];
        let len_sz = (h & 0x0F) as usize;
        let off_sz = (h >> 4) as usize;
        i += 1;
        if len_sz == 0 || len_sz > 8 || off_sz > 8 || i + len_sz + off_sz > b.len() {
            break;
        }
        let mut len: u64 = 0;
        for k in 0..len_sz {
            len |= (b[i + k] as u64) << (8 * k);
        }
        i += len_sz;
        if off_sz > 0 {
            let mut off: i64 = 0;
            for k in 0..off_sz {
                off |= (b[i + k] as i64) << (8 * k);
            }
            if b[i + off_sz - 1] & 0x80 != 0 {
                off |= -1i64 << (8 * off_sz);
            }
            lcn += off;
            out.push((lcn, len));
        } // sparse runs (no offset) hold no MFT records
        i += off_sz;
    }
    out
}

fn apply_fixup(rec: &mut [u8]) -> bool {
    if rec.len() < 48 || &rec[0..4] != b"FILE" {
        return false;
    }
    let usa_off = rd16(rec, 4) as usize;
    let usa_cnt = rd16(rec, 6) as usize;
    if usa_cnt < 2 || usa_off + usa_cnt * 2 > rec.len() {
        return false;
    }
    let stride = rec.len() / (usa_cnt - 1);
    let seq = [rec[usa_off], rec[usa_off + 1]];
    for k in 1..usa_cnt {
        let end = k * stride;
        if end > rec.len() || end < 2 {
            return false;
        }
        if rec[end - 2] != seq[0] || rec[end - 1] != seq[1] {
            return false;
        }
        rec[end - 2] = rec[usa_off + 2 * k];
        rec[end - 1] = rec[usa_off + 2 * k + 1];
    }
    true
}

struct MftRec {
    name: Option<Box<str>>,
    ns: u8,
    parent: u32,
    size: u64,
    alloc: u64,
    modified: i64,
    flags: u8, // 1 = in use, 2 = directory, 4 = has data size
}

fn parse_record(rec: &[u8], idx: usize, recs: &mut [MftRec]) {
    let flags = rd16(rec, 0x16);
    if flags & 1 == 0 {
        return;
    }
    let base = (rd64(rec, 0x20) & 0x0000_FFFF_FFFF_FFFF) as usize;
    let target = if base != 0 { base } else { idx };
    if target >= recs.len() {
        return;
    }
    if base == 0 {
        recs[target].flags |= 1 | if flags & 2 != 0 { 2 } else { 0 };
    }
    let mut off = rd16(rec, 0x14) as usize;
    while off + 16 <= rec.len() {
        let ty = rd32(rec, off);
        if ty == 0xFFFF_FFFF {
            break;
        }
        let len = rd32(rec, off + 4) as usize;
        if len < 16 || off + len > rec.len() {
            break;
        }
        let a = &rec[off..off + len];
        let non_res = a[8] != 0;
        let name_len = a[9];
        match ty {
            0x10 if !non_res && len >= 0x18 => {
                let vo = rd16(a, 0x14) as usize;
                if vo + 0x10 <= a.len() {
                    let mt = rd64(a, vo + 0x08) as i64;
                    let r = &mut recs[target];
                    if mt > r.modified {
                        r.modified = mt;
                    }
                }
            }
            0x30 if !non_res => {
                let vo = rd16(a, 0x14) as usize;
                if vo + 0x42 <= a.len() {
                    let v = &a[vo..];
                    let parent = (rd64(v, 0) & 0x0000_FFFF_FFFF_FFFF) as u32;
                    let nlen = v[0x40] as usize;
                    let ns = v[0x41];
                    if 0x42 + nlen * 2 <= v.len() {
                        let r = &mut recs[target];
                        let better = r.name.is_none() || (r.ns == 2 && ns != 2);
                        if better {
                            let w: Vec<u16> = (0..nlen).map(|k| rd16(v, 0x42 + 2 * k)).collect();
                            r.name = Some(String::from_utf16_lossy(&w).into_boxed_str());
                            r.ns = ns;
                            r.parent = parent;
                            if r.modified == 0 {
                                r.modified = rd64(v, 0x10) as i64;
                            }
                        }
                    }
                }
            }
            0x80 if name_len == 0 => {
                let r = &mut recs[target];
                if non_res {
                    if len >= 0x40 && rd64(a, 0x10) == 0 {
                        let compressed_or_sparse = rd16(a, 0x0C) & 0x8001 != 0;
                        let alloc = if compressed_or_sparse && len >= 0x48 { rd64(a, 0x40) } else { rd64(a, 0x28) };
                        r.size = rd64(a, 0x30);
                        r.alloc = alloc;
                        r.flags |= 4;
                    }
                } else if r.flags & 4 == 0 && len >= 0x18 {
                    r.size = rd32(a, 0x10) as u64;
                    r.alloc = 0;
                    r.flags |= 4;
                }
            }
            _ => {}
        }
        off += len;
    }
}

struct Aligned {
    buf: Vec<u8>,
    start: usize,
    len: usize,
}

impl Aligned {
    fn new(len: usize) -> Self {
        let buf = vec![0u8; len + 4096];
        let start = (4096 - (buf.as_ptr() as usize % 4096)) % 4096;
        Self { buf, start, len }
    }
    fn slice(&mut self, n: usize) -> &mut [u8] {
        &mut self.buf[self.start..self.start + n.min(self.len)]
    }
}

fn read_exact_at(f: &std::fs::File, buf: &mut [u8], offset: u64) -> Result<(), String> {
    use std::os::windows::fs::FileExt;
    let mut done = 0usize;
    while done < buf.len() {
        let n = f.seek_read(&mut buf[done..], offset + done as u64).map_err(|e| e.to_string())?;
        if n == 0 {
            return Err("unexpected end of volume".into());
        }
        done += n;
    }
    Ok(())
}

fn mft_scan(drive: char, prog: &Progress) -> Result<Vec<Raw>, String> {
    use std::os::windows::fs::OpenOptionsExt;
    use std::os::windows::io::AsRawHandle;
    use windows_sys::Win32::System::IO::DeviceIoControl;
    let f = std::fs::OpenOptions::new()
        .read(true)
        .share_mode(7)
        .open(format!("\\\\.\\{drive}:"))
        .map_err(|e| format!("cannot open the drive: {e}"))?;
    let mut vd = [0u8; 128];
    let mut ret = 0u32;
    let ok = unsafe {
        DeviceIoControl(f.as_raw_handle() as _, 0x0009_0064, std::ptr::null(), 0, vd.as_mut_ptr() as *mut _, vd.len() as u32, &mut ret, std::ptr::null_mut())
    };
    if ok == 0 || ret < 72 {
        return Err("not an NTFS volume".into());
    }
    let bytes_per_cluster = rd32(&vd, 44) as u64;
    let rec_size = rd32(&vd, 48) as usize;
    let mft_valid = rd64(&vd, 56);
    let mft_lcn = rd64(&vd, 64);
    if bytes_per_cluster == 0 || !(512..=65536).contains(&rec_size) {
        return Err("unexpected NTFS layout".into());
    }
    // Record 0 describes the MFT itself: find where all of it lives.
    let mut first = Aligned::new(rec_size.max(4096));
    {
        let b = first.slice(rec_size.max(4096));
        read_exact_at(&f, b, mft_lcn as u64 * bytes_per_cluster)?;
    }
    let mut rec0 = first.slice(rec_size).to_vec();
    if !apply_fixup(&mut rec0) {
        return Err("file table header is damaged".into());
    }
    let mut runs = vec![];
    let mut off = rd16(&rec0, 0x14) as usize;
    while off + 16 <= rec0.len() {
        let ty = rd32(&rec0, off);
        if ty == 0xFFFF_FFFF {
            break;
        }
        let len = rd32(&rec0, off + 4) as usize;
        if len < 16 || off + len > rec0.len() {
            break;
        }
        if ty == 0x80 && rec0[off + 8] != 0 && rec0[off + 9] == 0 {
            let ro = rd16(&rec0, off + 0x20) as usize;
            if ro < len {
                runs = data_runs(&rec0[off + ro..off + len]);
            }
            break;
        }
        off += len;
    }
    if runs.is_empty() {
        return Err("could not locate the file table".into());
    }
    let total_recs = (mft_valid / rec_size as u64) as usize;
    let mut recs: Vec<MftRec> = (0..total_recs).map(|_| MftRec { name: None, ns: 0, parent: 0, size: 0, alloc: 0, modified: 0, flags: 0 }).collect();
    const CHUNK: usize = 8 << 20;
    let mut buf = Aligned::new(CHUNK);
    let mut rec_index = 0usize;
    'runs: for (lcn, clusters) in runs {
        let mut pos = lcn as u64 * bytes_per_cluster;
        let mut left = clusters * bytes_per_cluster;
        while left > 0 {
            if prog.cancel.load(Ordering::Relaxed) {
                return Err("cancelled".into());
            }
            let n = (left as usize).min(CHUNK);
            let n = n - n % rec_size.max(1);
            if n == 0 {
                break;
            }
            let b = buf.slice(n);
            read_exact_at(&f, b, pos)?;
            for k in 0..n / rec_size {
                if rec_index >= total_recs {
                    break 'runs;
                }
                let rec = &mut b[k * rec_size..(k + 1) * rec_size];
                if apply_fixup(rec) {
                    parse_record(rec, rec_index, &mut recs);
                }
                rec_index += 1;
            }
            prog.files.store(rec_index as u64, Ordering::Relaxed);
            pos += n as u64;
            left -= n as u64;
        }
    }
    // Build parent → children with a counting sort, then walk from the root (record 5).
    const ROOT: usize = 5;
    if recs.len() <= ROOT || recs[ROOT].flags & 1 == 0 {
        return Err("root folder record missing".into());
    }
    let valid = |i: usize, recs: &Vec<MftRec>| -> bool {
        let r = &recs[i];
        i != ROOT && r.flags & 1 != 0 && r.name.is_some() && (r.parent as usize) < recs.len() && recs[r.parent as usize].flags & 3 == 3
    };
    let mut counts = vec![0u32; recs.len() + 1];
    for i in 0..recs.len() {
        if valid(i, &recs) {
            counts[recs[i].parent as usize + 1] += 1;
        }
    }
    for i in 1..counts.len() {
        counts[i] += counts[i - 1];
    }
    let mut fill = counts.clone();
    let mut kids = vec![0u32; *counts.last().unwrap() as usize];
    for i in 0..recs.len() {
        if valid(i, &recs) {
            let p = recs[i].parent as usize;
            kids[fill[p] as usize] = i as u32;
            fill[p] += 1;
        }
    }
    let mut out: Vec<Raw> = Vec::with_capacity(kids.len() + 1);
    out.push(Raw { name: format!("{drive}:\\").into(), parent: NONE, size: 0, alloc: 0, modified: recs[ROOT].modified, is_dir: true });
    let mut stack: Vec<(usize, u32)> = vec![(ROOT, 0)];
    let mut seen = vec![false; recs.len()];
    seen[ROOT] = true;
    let mut bytes = 0u64;
    while let Some((rec, new_idx)) = stack.pop() {
        for &k in &kids[counts[rec] as usize..counts[rec + 1] as usize] {
            let k = k as usize;
            if seen[k] {
                continue;
            }
            seen[k] = true;
            let r = &mut recs[k];
            let is_dir = r.flags & 2 != 0;
            let idx = out.len() as u32;
            if !is_dir {
                bytes += r.size;
            }
            out.push(Raw {
                name: r.name.take().unwrap_or_default(),
                parent: new_idx,
                size: if is_dir { 0 } else { r.size },
                alloc: if is_dir { 0 } else { r.alloc },
                modified: r.modified,
                is_dir,
            });
            if is_dir {
                stack.push((k, idx));
            }
        }
    }
    prog.bytes.store(bytes, Ordering::Relaxed);
    prog.files.store(out.len() as u64, Ordering::Relaxed);
    Ok(out)
}

// ------------------------------------------------------------------ helpers on a finished tree

impl Tree {
    pub fn path(&self, mut i: u32) -> String {
        let mut parts: Vec<&str> = vec![];
        while i != NONE {
            let n = &self.nodes[i as usize];
            if n.parent == NONE {
                let root = self.root_path.trim_end_matches('\\');
                parts.push(root);
                break;
            }
            parts.push(&n.name);
            i = n.parent;
        }
        parts.reverse();
        let s = parts.join("\\");
        if s.len() == 2 && s.ends_with(':') {
            format!("{s}\\")
        } else {
            s
        }
    }

    pub fn ancestors(&self, mut i: u32) -> Vec<u32> {
        let mut v = vec![];
        while i != NONE {
            v.push(i);
            i = self.nodes[i as usize].parent;
        }
        v
    }

    pub fn ext_name(&self, i: u32) -> &str {
        let e = self.nodes[i as usize].ext;
        if e == NONE {
            ""
        } else {
            &self.exts[e as usize].ext
        }
    }

    /// Remove a node (after deleting it from disk) and subtract it from every ancestor.
    pub fn remove(&mut self, i: u32) {
        let (size, alloc, files, dirs, is_dir, parent) = {
            let n = &self.nodes[i as usize];
            (n.size, n.alloc, n.files, n.dirs, n.is_dir, n.parent)
        };
        if parent == NONE {
            return;
        }
        let pn = &mut self.nodes[parent as usize];
        pn.children.retain(|c| *c != i);
        let mut a = parent;
        while a != NONE {
            let n = &mut self.nodes[a as usize];
            n.size = n.size.saturating_sub(size);
            n.alloc = n.alloc.saturating_sub(alloc);
            n.files = n.files.saturating_sub(files);
            n.dirs = n.dirs.saturating_sub(dirs + if is_dir { 1 } else { 0 });
            a = n.parent;
        }
        let n = &mut self.nodes[i as usize];
        n.size = 0;
        n.alloc = 0;
        n.files = 0;
        n.dirs = 0;
        n.children.clear();
        n.parent = NONE;
        n.name = "(deleted)".into();
    }
}

pub struct DupGroup {
    pub name: String,
    pub size: u64,
    pub nodes: Vec<u32>,
}

/// Files with the same name and size (and optionally the same modified time).
pub fn duplicates(t: &Tree, min_size: u64, with_date: bool) -> Vec<DupGroup> {
    let mut map: HashMap<(String, u64, i64), Vec<u32>> = HashMap::new();
    for (i, n) in t.nodes.iter().enumerate() {
        if n.is_dir || n.size < min_size || n.parent == NONE {
            continue;
        }
        let key = (n.name.to_lowercase(), n.size, if with_date { n.modified / 10_000_000 } else { 0 });
        map.entry(key).or_default().push(i as u32);
    }
    let mut out: Vec<DupGroup> = map
        .into_iter()
        .filter(|(_, v)| v.len() > 1)
        .map(|((name, size, _), nodes)| DupGroup { name: t.nodes[nodes[0] as usize].name.to_string().if_empty(name), size, nodes })
        .collect();
    out.sort_by(|a, b| (b.size * (b.nodes.len() as u64 - 1)).cmp(&(a.size * (a.nodes.len() as u64 - 1))));
    out
}

trait IfEmpty {
    fn if_empty(self, other: String) -> String;
}
impl IfEmpty for String {
    fn if_empty(self, other: String) -> String {
        if self.is_empty() {
            other
        } else {
            self
        }
    }
}

/// Simple wildcard match supporting `*` and `?` (case-insensitive).
pub fn wild(pattern: &str, text: &str) -> bool {
    let p: Vec<char> = pattern.to_lowercase().chars().collect();
    let t: Vec<char> = text.to_lowercase().chars().collect();
    let (mut pi, mut ti) = (0usize, 0usize);
    let (mut star, mut mark) = (usize::MAX, 0usize);
    while ti < t.len() {
        if pi < p.len() && (p[pi] == '?' || p[pi] == t[ti]) {
            pi += 1;
            ti += 1;
        } else if pi < p.len() && p[pi] == '*' {
            star = pi;
            mark = ti;
            pi += 1;
        } else if star != usize::MAX {
            pi = star + 1;
            mark += 1;
            ti = mark;
        } else {
            return false;
        }
    }
    while pi < p.len() && p[pi] == '*' {
        pi += 1;
    }
    pi == p.len()
}

/// Squarified treemap layout of weighted items inside `rect`.
pub fn squarify(items: &[(u32, f64)], rect: egui::Rect) -> Vec<(u32, egui::Rect)> {
    let mut out = Vec::with_capacity(items.len());
    let total: f64 = items.iter().map(|x| x.1).sum();
    if total <= 0.0 || rect.width() < 1.0 || rect.height() < 1.0 {
        return out;
    }
    let scale = (rect.width() as f64 * rect.height() as f64) / total;
    let areas: Vec<(u32, f64)> = items.iter().filter(|x| x.1 > 0.0).map(|x| (x.0, x.1 * scale)).collect();
    let mut r = rect;
    let mut i = 0;
    while i < areas.len() {
        let short = (r.width().min(r.height()) as f64).max(1e-6);
        let mut row_end = i + 1;
        let mut row_sum = areas[i].1;
        let worst = |sum: f64, mn: f64, mx: f64| -> f64 {
            let s2 = sum * sum;
            let w2 = short * short;
            (w2 * mx / s2).max(s2 / (w2 * mn))
        };
        let mut mn = areas[i].1;
        let mut mx = areas[i].1;
        let mut cur = worst(row_sum, mn, mx);
        while row_end < areas.len() {
            let a = areas[row_end].1;
            let nmn = mn.min(a);
            let nmx = mx.max(a);
            let w = worst(row_sum + a, nmn, nmx);
            if w > cur {
                break;
            }
            cur = w;
            mn = nmn;
            mx = nmx;
            row_sum += a;
            row_end += 1;
        }
        let horizontal = r.width() >= r.height();
        let thick = (row_sum / short) as f32;
        let mut pos = 0.0f32;
        for &(id, a) in &areas[i..row_end] {
            let len = (a / row_sum * short) as f32;
            let cell = if horizontal {
                egui::Rect::from_min_size(egui::pos2(r.left(), r.top() + pos), egui::vec2(thick, len))
            } else {
                egui::Rect::from_min_size(egui::pos2(r.left() + pos, r.top()), egui::vec2(len, thick))
            };
            out.push((id, cell));
            pos += len;
        }
        r = if horizontal {
            egui::Rect::from_min_max(egui::pos2(r.left() + thick, r.top()), r.max)
        } else {
            egui::Rect::from_min_max(egui::pos2(r.left(), r.top() + thick), r.max)
        };
        i = row_end;
    }
    out
}

/// Move a file or folder to the Recycle Bin.
pub fn recycle(path: &str) -> Result<(), String> {
    #[repr(C)]
    struct ShFileOp {
        hwnd: *mut core::ffi::c_void,
        func: u32,
        from: *const u16,
        to: *const u16,
        flags: u16,
        aborted: i32,
        mappings: *mut core::ffi::c_void,
        title: *const u16,
    }
    #[link(name = "shell32")]
    extern "system" {
        fn SHFileOperationW(op: *mut ShFileOp) -> i32;
    }
    let mut from: Vec<u16> = path.encode_utf16().collect();
    from.push(0);
    from.push(0);
    let mut op = ShFileOp {
        hwnd: std::ptr::null_mut(),
        func: 3,              // FO_DELETE
        from: from.as_ptr(),
        to: std::ptr::null(),
        flags: 0x40 | 0x10 | 0x04 | 0x400, // allow undo, no confirmation (we asked), silent, no error UI
        aborted: 0,
        mappings: std::ptr::null_mut(),
        title: std::ptr::null(),
    };
    let r = unsafe { SHFileOperationW(&mut op) };
    if r != 0 || op.aborted != 0 {
        Err(format!("Windows could not move it to the Recycle Bin (code {r:#x}). It may be in use or need the Admin view."))
    } else {
        Ok(())
    }
}
