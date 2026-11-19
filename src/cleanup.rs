//! Uninstaller and clean-up engine: installed programs, running uninstallers,
//! leftover scanning with registry backups, Store apps, browser extensions,
//! junk files, privacy traces, secure deletion, backups and uninstall history.

use crate::util::{self, from_wide, ps_quote, wide};
use serde::{Deserialize, Serialize};
use std::collections::HashSet;
use std::ffi::c_void;
use std::path::{Path, PathBuf};
use windows_sys::Win32::Foundation::FILETIME;
use windows_sys::Win32::System::Registry::{
    RegCloseKey, RegEnumKeyExW, RegEnumValueW, RegOpenKeyExW, RegQueryValueExW, HKEY, HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE,
};

// ------------------------------------------------------------------ registry helpers

const KEY_READ: u32 = 0x20019;
const WOW64_64: u32 = 0x0100;
const WOW64_32: u32 = 0x0200;

struct Key(HKEY);
impl Drop for Key {
    fn drop(&mut self) {
        unsafe { RegCloseKey(self.0) };
    }
}

fn open(root: HKEY, path: &str, view: u32) -> Option<Key> {
    let w = wide(path);
    let mut h: HKEY = std::ptr::null_mut();
    let r = unsafe { RegOpenKeyExW(root, w.as_ptr(), 0, KEY_READ | view, &mut h) };
    if r == 0 {
        Some(Key(h))
    } else {
        None
    }
}

fn subkeys(k: &Key) -> Vec<(String, i64)> {
    let mut out = vec![];
    let mut i = 0u32;
    loop {
        let mut name = vec![0u16; 512];
        let mut len = name.len() as u32;
        let mut ft = FILETIME { dwLowDateTime: 0, dwHighDateTime: 0 };
        let r = unsafe { RegEnumKeyExW(k.0, i, name.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), &mut ft) };
        if r != 0 {
            break;
        }
        out.push((String::from_utf16_lossy(&name[..len as usize]), ((ft.dwHighDateTime as i64) << 32) | ft.dwLowDateTime as i64));
        i += 1;
        if i > 100_000 {
            break;
        }
    }
    out
}

fn value_names(k: &Key) -> Vec<String> {
    let mut out = vec![];
    let mut i = 0u32;
    loop {
        let mut name = vec![0u16; 16384];
        let mut len = name.len() as u32;
        let r = unsafe { RegEnumValueW(k.0, i, name.as_mut_ptr(), &mut len, std::ptr::null(), std::ptr::null_mut(), std::ptr::null_mut(), std::ptr::null_mut()) };
        if r != 0 {
            break;
        }
        out.push(String::from_utf16_lossy(&name[..len as usize]));
        i += 1;
        if i > 10_000 {
            break;
        }
    }
    out
}

enum RegVal {
    Str(String),
    Num(u64),
}

fn value(k: &Key, name: &str) -> Option<RegVal> {
    let w = wide(name);
    let mut ty = 0u32;
    let mut size = 0u32;
    unsafe {
        if RegQueryValueExW(k.0, w.as_ptr(), std::ptr::null(), &mut ty, std::ptr::null_mut(), &mut size) != 0 {
            return None;
        }
        let mut buf = vec![0u8; size as usize + 4];
        if RegQueryValueExW(k.0, w.as_ptr(), std::ptr::null(), &mut ty, buf.as_mut_ptr(), &mut size) != 0 {
            return None;
        }
        match ty {
            1 | 2 => {
                let w: Vec<u16> = buf[..size as usize].chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
                let s = from_wide(&w);
                Some(RegVal::Str(if ty == 2 { expand_env(&s) } else { s }))
            }
            4 if size >= 4 => Some(RegVal::Num(u32::from_le_bytes(buf[..4].try_into().unwrap()) as u64)),
            11 if size >= 8 => Some(RegVal::Num(u64::from_le_bytes(buf[..8].try_into().unwrap()))),
            _ => None,
        }
    }
}

fn vstr(k: &Key, name: &str) -> String {
    match value(k, name) {
        Some(RegVal::Str(s)) => s.trim().to_string(),
        Some(RegVal::Num(n)) => n.to_string(),
        None => String::new(),
    }
}

fn vnum(k: &Key, name: &str) -> u64 {
    match value(k, name) {
        Some(RegVal::Num(n)) => n,
        Some(RegVal::Str(s)) => s.trim().parse().unwrap_or(0),
        None => 0,
    }
}

pub fn expand_env(s: &str) -> String {
    let mut out = String::new();
    let mut rest = s;
    while let Some(a) = rest.find('%') {
        out.push_str(&rest[..a]);
        let after = &rest[a + 1..];
        if let Some(b) = after.find('%') {
            let var = &after[..b];
            match std::env::var(var) {
                Ok(v) if !var.is_empty() => out.push_str(&v),
                _ => {
                    out.push('%');
                    out.push_str(var);
                    out.push('%');
                }
            }
            rest = &after[b + 1..];
        } else {
            out.push_str(&rest[a..]);
            rest = "";
        }
    }
    out.push_str(rest);
    out
}

fn env(name: &str) -> String {
    std::env::var(name).unwrap_or_default()
}

// ------------------------------------------------------------------ installed programs

#[derive(Clone, Debug, Default)]
pub struct Program {
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub install_date: String,
    pub size_kb: u64,
    pub location: String,
    pub uninstall: String,
    pub quiet: String,
    pub icon: String,
    pub url: String,
    pub help: String,
    pub comments: String,
    pub key_path: String,
    pub key_name: String,
    pub bits: &'static str,
    pub scope: &'static str,
    pub msi: bool,
    pub system: bool,
    pub update: bool,
    pub no_remove: bool,
    pub modified: i64,
}

impl Program {
    pub fn is_new(&self) -> bool {
        util::filetime_to_unix(self.modified) > util::now_unix() - 7 * 86400
    }
}

pub fn programs() -> Vec<Program> {
    let sources: [(HKEY, &str, u32, &'static str, &'static str, &str); 3] = [
        (HKEY_LOCAL_MACHINE, "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall", WOW64_64, "64-bit", "All users", "HKLM"),
        (HKEY_LOCAL_MACHINE, "SOFTWARE\\Microsoft\\Windows\\CurrentVersion\\Uninstall", WOW64_32, "32-bit", "All users", "HKLM"),
        (HKEY_CURRENT_USER, "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall", 0, "", "This user", "HKCU"),
    ];
    let mut out = vec![];
    for (root, path, view, bits, scope, prefix) in sources {
        let Some(k) = open(root, path, view) else { continue };
        for (name, ft) in subkeys(&k) {
            let Some(sk) = open(root, &format!("{path}\\{name}"), view) else { continue };
            let display = vstr(&sk, "DisplayName");
            if display.is_empty() {
                continue;
            }
            let release = vstr(&sk, "ReleaseType");
            let parent = vstr(&sk, "ParentKeyName");
            let date = vstr(&sk, "InstallDate");
            let date = if date.len() == 8 && date.chars().all(|c| c.is_ascii_digit()) { format!("{}-{}-{}", &date[..4], &date[4..6], &date[6..]) } else { date };
            let reg_path = if view == WOW64_32 { format!("{prefix}\\SOFTWARE\\WOW6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall\\{name}") } else { format!("{prefix}\\{path}\\{name}") };
            let mut bits_v = bits;
            if bits.is_empty() {
                let loc = vstr(&sk, "InstallLocation").to_lowercase();
                bits_v = if loc.contains("(x86)") { "32-bit" } else { "64-bit" };
            }
            out.push(Program {
                name: display,
                publisher: vstr(&sk, "Publisher"),
                version: vstr(&sk, "DisplayVersion"),
                install_date: if date.is_empty() { util::fmt_time(util::filetime_to_unix(ft)).chars().take(10).collect() } else { date },
                size_kb: vnum(&sk, "EstimatedSize"),
                location: vstr(&sk, "InstallLocation").trim_matches('"').to_string(),
                uninstall: vstr(&sk, "UninstallString"),
                quiet: vstr(&sk, "QuietUninstallString"),
                icon: vstr(&sk, "DisplayIcon"),
                url: { let u = vstr(&sk, "URLInfoAbout"); if u.is_empty() { vstr(&sk, "URLUpdateInfo") } else { u } },
                help: vstr(&sk, "HelpLink"),
                comments: vstr(&sk, "Comments"),
                key_path: reg_path,
                key_name: name.clone(),
                bits: bits_v,
                scope,
                msi: vnum(&sk, "WindowsInstaller") == 1,
                system: vnum(&sk, "SystemComponent") == 1,
                update: !parent.is_empty() || release.contains("Update") || release.contains("Hotfix"),
                no_remove: vnum(&sk, "NoRemove") == 1,
                modified: ft,
            });
        }
    }
    // the same product can be listed twice (32/64-bit views of shared keys)
    let mut seen = HashSet::new();
    out.retain(|p| seen.insert(p.key_path.to_lowercase()));
    out.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out
}

fn split_command(cmd: &str) -> (String, String) {
    let c = cmd.trim();
    if let Some(rest) = c.strip_prefix('"') {
        if let Some(end) = rest.find('"') {
            return (rest[..end].to_string(), rest[end + 1..].trim().to_string());
        }
    }
    let lower = c.to_lowercase();
    if let Some(i) = lower.find(".exe") {
        let exe = &c[..i + 4];
        return (exe.to_string(), c[i + 4..].trim().to_string());
    }
    match c.find(' ') {
        Some(i) => (c[..i].to_string(), c[i + 1..].to_string()),
        None => (c.to_string(), String::new()),
    }
}

fn is_guid(s: &str) -> bool {
    s.len() == 38 && s.starts_with('{') && s.ends_with('}')
}

/// The command to run for an uninstall: (program, arguments).
pub fn uninstall_command(p: &Program, quiet: bool) -> Option<(String, String)> {
    if p.msi && is_guid(&p.key_name) {
        let mut args = format!("/x {}", p.key_name);
        if quiet {
            args.push_str(" /qb /norestart");
        }
        return Some(("msiexec.exe".into(), args));
    }
    let s = if quiet && !p.quiet.is_empty() { &p.quiet } else { &p.uninstall };
    if s.is_empty() {
        return None;
    }
    let (exe, mut args) = split_command(s);
    if exe.to_lowercase().ends_with("msiexec.exe") || exe.eq_ignore_ascii_case("msiexec") {
        args = args.replace("/I{", "/X{").replace("/i{", "/x{");
        if quiet && !args.contains("/q") {
            args.push_str(" /qb /norestart");
        }
    }
    Some((exe, args))
}

/// Start a program (through the shell, so it can ask for administrator rights) and wait for it.
pub fn run_and_wait(exe: &str, args: &str) -> Result<u32, String> {
    #[repr(C)]
    struct ShellExecuteInfoW {
        cb_size: u32,
        f_mask: u32,
        hwnd: *mut c_void,
        verb: *const u16,
        file: *const u16,
        params: *const u16,
        dir: *const u16,
        show: i32,
        inst_app: *mut c_void,
        id_list: *mut c_void,
        class: *const u16,
        hkey_class: *mut c_void,
        hot_key: u32,
        icon_or_monitor: *mut c_void,
        process: *mut c_void,
    }
    #[link(name = "shell32")]
    extern "system" {
        fn ShellExecuteExW(info: *mut ShellExecuteInfoW) -> i32;
    }
    use windows_sys::Win32::System::Threading::{GetExitCodeProcess, WaitForSingleObject};
    let f = wide(exe);
    let p = wide(args);
    let mut info = ShellExecuteInfoW {
        cb_size: std::mem::size_of::<ShellExecuteInfoW>() as u32,
        f_mask: 0x40 | 0x100, // keep the process handle, run synchronously
        hwnd: std::ptr::null_mut(),
        verb: std::ptr::null(),
        file: f.as_ptr(),
        params: p.as_ptr(),
        dir: std::ptr::null(),
        show: 1,
        inst_app: std::ptr::null_mut(),
        id_list: std::ptr::null_mut(),
        class: std::ptr::null(),
        hkey_class: std::ptr::null_mut(),
        hot_key: 0,
        icon_or_monitor: std::ptr::null_mut(),
        process: std::ptr::null_mut(),
    };
    unsafe {
        if ShellExecuteExW(&mut info) == 0 {
            return Err(format!("Could not start the uninstaller: {}", std::io::Error::last_os_error()));
        }
        if info.process.is_null() {
            return Ok(0);
        }
        let h = crate::sys::Handle(info.process);
        WaitForSingleObject(h.0, u32::MAX);
        let mut code = 0u32;
        GetExitCodeProcess(h.0, &mut code);
        Ok(code)
    }
}

pub fn create_restore_point(desc: &str) -> Result<String, String> {
    let out = util::powershell(&format!(
        "try {{ Checkpoint-Computer -Description {} -RestorePointType APPLICATION_UNINSTALL -ErrorAction Stop; 'OK' }} catch {{ 'ERROR: ' + $_.Exception.Message }}",
        ps_quote(desc)
    ))?;
    if out.contains("ERROR:") {
        Err(out.trim().replace("ERROR: ", ""))
    } else {
        Ok("Restore point created".into())
    }
}

// ------------------------------------------------------------------ leftovers

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Level {
    Safe,
    Moderate,
    Advanced,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum LeftKind {
    RegistryKey,
    Folder,
    File,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Leftover {
    pub kind: LeftKind,
    pub path: String,
    pub reason: String,
    pub size: u64,
    pub selected: bool,
}

fn norm(s: &str) -> String {
    s.chars().filter(|c| c.is_alphanumeric()).collect::<String>().to_lowercase()
}

fn clean_name(name: &str) -> String {
    // drop versions, bitness and edition noise: "Foo Bar 2.3.1 (x64)" -> "Foo Bar"
    let mut words = vec![];
    for w in name.split_whitespace() {
        let wl = w.to_lowercase();
        let t = wl.trim_matches(|c: char| !c.is_alphanumeric());
        if t.is_empty() || t.chars().next().map(|c| c.is_ascii_digit()).unwrap_or(false) && t.contains('.') {
            break;
        }
        if ["x64", "x86", "64-bit", "32-bit", "(x64)", "(x86)", "version", "v"].contains(&wl.as_str()) {
            break;
        }
        if wl.starts_with('(') || wl.starts_with('-') {
            break;
        }
        words.push(w);
    }
    if words.is_empty() {
        name.to_string()
    } else {
        words.join(" ")
    }
}

fn clean_publisher(p: &str) -> String {
    let mut s = p.replace(',', " ").replace('.', " ");
    for suffix in [" inc", " llc", " ltd", " limited", " corporation", " corp", " gmbh", " co", " s a", " sa", " ag", " bv", " srl", " software", " technologies", " pty"] {
        let lower = s.to_lowercase();
        if lower.ends_with(suffix) {
            s.truncate(s.len() - suffix.len());
        }
    }
    s.trim().to_string()
}

const PROTECTED: &[&str] = &[
    "microsoft", "windows", "common files", "microsoft shared", "windowsapps", "internet explorer", "windows nt", "system32", "packages",
    "temp", "programs", "start menu", "startup", "desktop", "documents", "intel", "nvidia corporation", "amd", "realtek", "google", "mozilla",
];

fn folder_size(p: &Path) -> u64 {
    let mut total = 0u64;
    let mut stack = vec![p.to_path_buf()];
    let mut n = 0;
    while let Some(d) = stack.pop() {
        let Ok(rd) = std::fs::read_dir(&d) else { continue };
        for e in rd.flatten() {
            n += 1;
            if n > 300_000 {
                return total;
            }
            if let Ok(m) = e.metadata() {
                if m.is_dir() {
                    if !m.file_type().is_symlink() {
                        stack.push(e.path());
                    }
                } else {
                    total += m.len();
                }
            }
        }
    }
    total
}

/// Find what a program left behind (or would leave behind) in the registry and on disk.
pub fn scan_leftovers(p: &Program, level: Level) -> Vec<Leftover> {
    let others: Vec<Program> = programs().into_iter().filter(|o| o.key_path != p.key_path).collect();
    let mut out = scan_leftovers_inner(p, level);
    mark_shared(p, &others, &mut out);
    out
}

/// Untick anything another installed program also uses, and say which one.
fn mark_shared(p: &Program, others: &[Program], out: &mut [Leftover]) {
    let n_name = norm(&clean_name(&p.name));
    let n_pub = norm(&clean_publisher(&p.publisher));
    let loc = p.location.trim_end_matches('\\').to_string();
    let n_loc = Path::new(&loc).file_name().map(|f| norm(&f.to_string_lossy())).unwrap_or_default();
    let same_pub: Vec<&Program> = if n_pub.is_empty() { vec![] } else { others.iter().filter(|o| norm(&clean_publisher(&o.publisher)) == n_pub).collect() };
    for l in out.iter_mut() {
        if l.reason.contains("uninstall entry") {
            continue;
        }
        let lp = l.path.to_lowercase().trim_end_matches('\\').to_string();
        if l.kind != LeftKind::RegistryKey {
            if let Some(o) = others.iter().find(|o| !o.location.is_empty() && o.location.to_lowercase().trim_end_matches('\\').starts_with(&lp)) {
                l.selected = false;
                l.reason = format!("{} — also used by {}", l.reason, o.name);
                continue;
            }
        }
        let last = norm(l.path.rsplit('\\').next().unwrap_or(""));
        if !same_pub.is_empty() && !n_loc.is_empty() && last == n_loc && n_loc != n_name {
            l.selected = false;
            l.reason = format!("{} — may be shared with {} other {} program(s)", l.reason, same_pub.len(), p.publisher);
        } else if !same_pub.is_empty() && l.reason.contains("publisher") {
            l.selected = false;
            l.reason = format!("{} — {} has other programs installed", l.reason, p.publisher);
        }
    }
}

fn scan_leftovers_inner(p: &Program, level: Level) -> Vec<Leftover> {
    let mut out: Vec<Leftover> = vec![];
    let mut seen = HashSet::new();
    let name = clean_name(&p.name);
    let pubr = clean_publisher(&p.publisher);
    let n_name = norm(&name);
    let n_pub = norm(&pubr);
    let loc = p.location.trim_end_matches('\\').to_string();
    let n_loc = Path::new(&loc).file_name().map(|f| norm(&f.to_string_lossy())).unwrap_or_default();
    let is_ms = n_pub.starts_with("microsoft");
    let matches = |cand: &str| -> Option<&'static str> {
        let c = norm(cand);
        if c.len() < 3 || PROTECTED.contains(&cand.to_lowercase().as_str()) {
            return None;
        }
        if c == n_name || (!n_loc.is_empty() && c == n_loc) {
            return Some("same name as the program");
        }
        // A longer name that starts with the program's name, or a shorter one covering most of it.
        // ("Epson" alone must not match "Epson Photo+": every Epson product shares it.)
        if level != Level::Safe && n_name.len() >= 4 && (c.starts_with(&n_name) || (c.len() >= 5 && n_name.starts_with(&c) && c.len() * 10 >= n_name.len() * 8)) {
            return Some("name starts like the program");
        }
        if level == Level::Advanced && n_name.len() >= 5 && c.contains(&n_name) {
            return Some("name contains the program's name");
        }
        None
    };
    let mut push = |out: &mut Vec<Leftover>, kind: LeftKind, path: String, reason: String| {
        if seen.insert(path.to_lowercase()) {
            let size = match kind {
                LeftKind::Folder => folder_size(Path::new(&path)),
                LeftKind::File => std::fs::metadata(&path).map(|m| m.len()).unwrap_or(0),
                LeftKind::RegistryKey => 0,
            };
            out.push(Leftover { kind, path, reason, size, selected: true });
        }
    };

    // ---- registry
    let (root_s, rest) = p.key_path.split_once('\\').unwrap_or(("", ""));
    let still_there = if root_s == "HKLM" {
        open(HKEY_LOCAL_MACHINE, rest, WOW64_64).is_some()
    } else {
        open(HKEY_CURRENT_USER, rest, 0).is_some()
    };
    if still_there {
        push(&mut out, LeftKind::RegistryKey, p.key_path.clone(), "the program's uninstall entry".into());
    }
    let bases: [(HKEY, &str, &str, u32); 3] = [(HKEY_CURRENT_USER, "Software", "HKCU\\Software", 0), (HKEY_LOCAL_MACHINE, "SOFTWARE", "HKLM\\SOFTWARE", WOW64_64), (HKEY_LOCAL_MACHINE, "SOFTWARE\\WOW6432Node", "HKLM\\SOFTWARE\\WOW6432Node", WOW64_64)];
    for (root, path, label, view) in bases {
        let Some(k) = open(root, path, view) else { continue };
        for (sub, _) in subkeys(&k) {
            let sub_n = norm(&sub);
            if let Some(why) = matches(&sub) {
                if !(is_ms && sub_n == "microsoft") {
                    push(&mut out, LeftKind::RegistryKey, format!("{label}\\{sub}"), why.into());
                }
                continue;
            }
            // publisher key: look one level deeper
            if !n_pub.is_empty() && (sub_n == n_pub || (level != Level::Safe && n_pub.len() >= 4 && sub_n.starts_with(&n_pub))) && !PROTECTED.contains(&sub.to_lowercase().as_str()) {
                if let Some(pk) = open(root, &format!("{path}\\{sub}"), view) {
                    let kids = subkeys(&pk);
                    let mut matched = 0;
                    for (kid, _) in &kids {
                        if let Some(why) = matches(kid) {
                            push(&mut out, LeftKind::RegistryKey, format!("{label}\\{sub}\\{kid}"), format!("{why} (under {sub})"));
                            matched += 1;
                        }
                    }
                    if level == Level::Advanced && matched > 0 && matched == kids.len() && value_names(&pk).is_empty() {
                        push(&mut out, LeftKind::RegistryKey, format!("{label}\\{sub}"), "publisher key with nothing else in it".into());
                    }
                }
            }
        }
    }
    if level == Level::Advanced && !loc.is_empty() {
        // services whose program lives in the install folder
        if let Some(k) = open(HKEY_LOCAL_MACHINE, "SYSTEM\\CurrentControlSet\\Services", WOW64_64) {
            let l = loc.to_lowercase();
            for (sub, _) in subkeys(&k) {
                if let Some(sk) = open(HKEY_LOCAL_MACHINE, &format!("SYSTEM\\CurrentControlSet\\Services\\{sub}"), WOW64_64) {
                    let img = vstr(&sk, "ImagePath").to_lowercase();
                    if !img.is_empty() && img.contains(&l) {
                        push(&mut out, LeftKind::RegistryKey, format!("HKLM\\SYSTEM\\CurrentControlSet\\Services\\{sub}"), "service that runs from the program's folder".into());
                    }
                }
            }
        }
    }

    // ---- files and folders
    if !loc.is_empty() && loc.len() > 3 && Path::new(&loc).exists() {
        let ln = loc.to_lowercase();
        let unsafe_loc = ["\\windows", "\\program files", "\\program files (x86)", "\\programdata", "\\users"].iter().any(|r| ln.ends_with(r));
        if !unsafe_loc {
            push(&mut out, LeftKind::Folder, loc.clone(), "the program's install folder".into());
        }
    }
    let local = env("LOCALAPPDATA");
    let roaming = env("APPDATA");
    let mut roots: Vec<String> = vec![env("ProgramFiles"), env("ProgramFiles(x86)"), env("ProgramData"), roaming.clone(), local.clone(), format!("{local}\\Programs")];
    if level != Level::Safe {
        roots.push(format!("{}\\Documents", env("USERPROFILE")));
        roots.push(format!("{local}\\Temp"));
    }
    for r in roots.iter().filter(|r| !r.is_empty()) {
        let Ok(rd) = std::fs::read_dir(r) else { continue };
        for e in rd.flatten() {
            let fname = e.file_name().to_string_lossy().to_string();
            let is_dir = e.file_type().map(|t| t.is_dir()).unwrap_or(false);
            if !is_dir {
                continue;
            }
            let fp = e.path().to_string_lossy().to_string();
            if let Some(why) = matches(&fname) {
                push(&mut out, LeftKind::Folder, fp, why.into());
                continue;
            }
            let fnn = norm(&fname);
            if !n_pub.is_empty() && fnn == n_pub && !PROTECTED.contains(&fname.to_lowercase().as_str()) {
                if let Ok(inner) = std::fs::read_dir(e.path()) {
                    let kids: Vec<_> = inner.flatten().collect();
                    let mut matched = 0;
                    for k in &kids {
                        let kn = k.file_name().to_string_lossy().to_string();
                        if let Some(why) = matches(&kn) {
                            let kind = if k.file_type().map(|t| t.is_dir()).unwrap_or(false) { LeftKind::Folder } else { LeftKind::File };
                            push(&mut out, kind, k.path().to_string_lossy().to_string(), format!("{why} (in {fname})"));
                            matched += 1;
                        }
                    }
                    if level == Level::Advanced && matched > 0 && matched == kids.len() {
                        push(&mut out, LeftKind::Folder, fp, "publisher folder with nothing else in it".into());
                    }
                }
            }
        }
    }
    // Start menu and desktop shortcuts
    let menus = [
        format!("{roaming}\\Microsoft\\Windows\\Start Menu\\Programs"),
        format!("{}\\Microsoft\\Windows\\Start Menu\\Programs", env("ProgramData")),
        format!("{}\\Desktop", env("USERPROFILE")),
        format!("{}\\Desktop", env("PUBLIC")),
    ];
    for m in menus {
        let Ok(rd) = std::fs::read_dir(&m) else { continue };
        for e in rd.flatten() {
            let fname = e.file_name().to_string_lossy().to_string();
            let stem = fname.trim_end_matches(".lnk").trim_end_matches(".url");
            if let Some(why) = matches(stem) {
                let kind = if e.file_type().map(|t| t.is_dir()).unwrap_or(false) { LeftKind::Folder } else { LeftKind::File };
                push(&mut out, kind, e.path().to_string_lossy().to_string(), format!("shortcut: {why}"));
            }
        }
    }
    // never offer anything inside the Windows folder or our own data
    let win = env("WINDIR").to_lowercase();
    let ours = util::data_dir().to_string_lossy().to_lowercase();
    out.retain(|l| {
        let lp = l.path.to_lowercase();
        l.kind == LeftKind::RegistryKey || (!lp.starts_with(&win) && !lp.starts_with(&ours))
    });
    out
}

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct BackupManifest {
    pub program: String,
    pub date: String,
    pub keys: Vec<String>,
    pub reg_files: Vec<String>,
    pub files: Vec<String>,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct RemoveReport {
    pub removed: usize,
    pub failed: Vec<String>,
    pub freed: u64,
    pub backup: String,
}

fn safe_file_name(s: &str) -> String {
    s.chars().map(|c| if c.is_alphanumeric() || c == '-' || c == '_' { c } else { '_' }).take(60).collect()
}

/// Remove the chosen leftovers: registry keys are exported to a .reg backup first,
/// files and folders go to the Recycle Bin.
pub fn remove_leftovers(program: &str, items: &[Leftover]) -> RemoveReport {
    let mut rep = RemoveReport::default();
    let dir = util::sub_dir("backups").join(format!("{}-{}", util::stamp(), safe_file_name(program)));
    let _ = std::fs::create_dir_all(&dir);
    let mut manifest = BackupManifest { program: program.into(), date: util::fmt_time(util::now_unix()), keys: vec![], reg_files: vec![], files: vec![] };
    // deepest keys first so parents are removed after children
    let mut keys: Vec<&Leftover> = items.iter().filter(|l| l.selected && l.kind == LeftKind::RegistryKey).collect();
    keys.sort_by_key(|l| std::cmp::Reverse(l.path.len()));
    for (i, l) in keys.iter().enumerate() {
        let file = dir.join(format!("key{:03}.reg", i + 1));
        let exp = util::run_hidden("reg.exe", &["export", &l.path, &file.to_string_lossy(), "/y"]);
        if exp.is_err() {
            // already gone (e.g. a parent was removed) or no access
            if util::run_hidden("reg.exe", &["query", &l.path]).is_err() {
                rep.removed += 1;
                continue;
            }
            rep.failed.push(format!("{} (could not back up — skipped)", l.path));
            continue;
        }
        manifest.keys.push(l.path.clone());
        manifest.reg_files.push(file.file_name().unwrap().to_string_lossy().to_string());
        match util::run_hidden("reg.exe", &["delete", &l.path, "/f"]) {
            Ok(_) => rep.removed += 1,
            Err(e) => rep.failed.push(format!("{} ({})", l.path, e.trim())),
        }
    }
    for l in items.iter().filter(|l| l.selected && l.kind != LeftKind::RegistryKey) {
        if !Path::new(&l.path).exists() {
            rep.removed += 1;
            continue;
        }
        match crate::disk::recycle(&l.path) {
            Ok(()) => {
                rep.removed += 1;
                rep.freed += l.size;
                manifest.files.push(l.path.clone());
            }
            Err(e) => rep.failed.push(format!("{} ({e})", l.path)),
        }
    }
    let _ = std::fs::write(dir.join("manifest.json"), serde_json::to_string_pretty(&manifest).unwrap_or_default());
    rep.backup = dir.to_string_lossy().to_string();
    rep
}

pub fn backups() -> Vec<(PathBuf, BackupManifest)> {
    let mut v: Vec<(PathBuf, BackupManifest)> = std::fs::read_dir(util::sub_dir("backups"))
        .map(|rd| {
            rd.flatten()
                .filter_map(|e| {
                    let m = std::fs::read_to_string(e.path().join("manifest.json")).ok()?;
                    Some((e.path(), serde_json::from_str(&m).ok()?))
                })
                .collect()
        })
        .unwrap_or_default();
    v.sort_by(|a, b| b.0.cmp(&a.0));
    v
}

pub fn restore_backup(dir: &Path, m: &BackupManifest) -> Result<String, String> {
    let mut ok = 0;
    let mut errs = vec![];
    for f in &m.reg_files {
        match util::run_hidden("reg.exe", &["import", &dir.join(f).to_string_lossy()]) {
            Ok(_) => ok += 1,
            Err(e) => errs.push(e.trim().to_string()),
        }
    }
    if errs.is_empty() {
        Ok(format!("Restored {ok} registry keys. Files can be restored from the Recycle Bin."))
    } else {
        Err(format!("Restored {ok}, failed {}: {}", errs.len(), errs.join("; ")))
    }
}

pub fn restore_points() -> Result<Vec<serde_json::Value>, String> {
    util::ps_json("Get-ComputerRestorePoint -ErrorAction Stop | Select-Object SequenceNumber,Description,@{n='Created';e={$_.ConvertToDateTime($_.CreationTime).ToString('yyyy-MM-dd HH:mm')}},@{n='Type';e={switch($_.RestorePointType){0{'Install'}1{'Uninstall'}10{'Driver install'}12{'Settings change'}13{'Cancelled'}default{[string]$_.RestorePointType}}}}")
}

// ------------------------------------------------------------------ uninstall history

#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct HistoryEntry {
    pub date: String,
    pub program: String,
    pub version: String,
    pub publisher: String,
    pub how: String,
    pub leftovers_removed: usize,
    pub freed: u64,
}

fn history_path() -> PathBuf {
    util::data_dir().join("uninstall-history.json")
}

pub fn history() -> Vec<HistoryEntry> {
    std::fs::read_to_string(history_path()).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or_default()
}

pub fn add_history(e: HistoryEntry) {
    let mut h = history();
    h.insert(0, e);
    h.truncate(2000);
    let _ = std::fs::write(history_path(), serde_json::to_string_pretty(&h).unwrap_or_default());
}

// ------------------------------------------------------------------ Store apps

pub fn store_apps(all_users: bool) -> Result<Vec<serde_json::Value>, String> {
    util::ps_json(&format!(
        "Get-AppxPackage {} | Select-Object Name,PackageFullName,PackageFamilyName,Publisher,Version,@{{n='Architecture';e={{[string]$_.Architecture}}}},InstallLocation,IsFramework,NonRemovable,IsBundle,@{{n='Signature';e={{[string]$_.SignatureKind}}}}",
        if all_users { "-AllUsers" } else { "" }
    ))
}

pub fn remove_store_app(full_name: &str, all_users: bool) -> Result<String, String> {
    let out = util::powershell(&format!(
        "try {{ Remove-AppxPackage -Package {} {} -ErrorAction Stop; 'OK' }} catch {{ 'ERROR: ' + $_.Exception.Message }}",
        ps_quote(full_name),
        if all_users { "-AllUsers" } else { "" }
    ))?;
    if out.contains("ERROR:") {
        Err(out.trim().replace("ERROR: ", ""))
    } else {
        Ok("Removed".into())
    }
}

// ------------------------------------------------------------------ browser extensions

#[derive(Clone, Debug)]
pub struct Extension {
    pub browser: &'static str,
    pub profile: String,
    pub id: String,
    pub name: String,
    pub version: String,
    pub description: String,
    pub path: String,
    pub enabled: Option<bool>,
    pub process: &'static str,
}

fn read_json(p: &Path) -> Option<serde_json::Value> {
    let s = std::fs::read(p).ok()?;
    let s = String::from_utf8_lossy(&s);
    serde_json::from_str(s.trim_start_matches('\u{feff}')).ok()
}

fn chromium_msg(ext_dir: &Path, manifest: &serde_json::Value, raw: &str) -> String {
    let Some(key) = raw.strip_prefix("__MSG_").and_then(|r| r.strip_suffix("__")) else { return raw.to_string() };
    let def = manifest.get("default_locale").and_then(|v| v.as_str()).unwrap_or("en");
    for loc in ["en", "en_US", def] {
        if let Some(m) = read_json(&ext_dir.join("_locales").join(loc).join("messages.json")) {
            if let Some(obj) = m.as_object() {
                for (k, v) in obj {
                    if k.eq_ignore_ascii_case(key) {
                        if let Some(s) = v.get("message").and_then(|x| x.as_str()) {
                            return s.to_string();
                        }
                    }
                }
            }
        }
    }
    raw.to_string()
}

pub fn extensions() -> Vec<Extension> {
    let local = env("LOCALAPPDATA");
    let roaming = env("APPDATA");
    let chromium: [(&'static str, String, &'static str); 5] = [
        ("Chrome", format!("{local}\\Google\\Chrome\\User Data"), "chrome.exe"),
        ("Edge", format!("{local}\\Microsoft\\Edge\\User Data"), "msedge.exe"),
        ("Brave", format!("{local}\\BraveSoftware\\Brave-Browser\\User Data"), "brave.exe"),
        ("Vivaldi", format!("{local}\\Vivaldi\\User Data"), "vivaldi.exe"),
        ("Opera", format!("{roaming}\\Opera Software"), "opera.exe"),
    ];
    let mut out = vec![];
    for (browser, base, process) in chromium {
        let Ok(rd) = std::fs::read_dir(&base) else { continue };
        for prof in rd.flatten() {
            let pdir = prof.path();
            let ext_root = pdir.join("Extensions");
            let Ok(exts) = std::fs::read_dir(&ext_root) else { continue };
            let profile_name = read_json(&pdir.join("Preferences"))
                .and_then(|p| p.pointer("/profile/name").and_then(|v| v.as_str()).map(|s| s.to_string()))
                .unwrap_or_else(|| prof.file_name().to_string_lossy().to_string());
            let prefs = read_json(&pdir.join("Secure Preferences")).or_else(|| read_json(&pdir.join("Preferences")));
            for e in exts.flatten() {
                let id = e.file_name().to_string_lossy().to_string();
                if id.len() != 32 {
                    continue;
                }
                let Ok(vers) = std::fs::read_dir(e.path()) else { continue };
                let Some(vdir) = vers.flatten().filter(|v| v.path().join("manifest.json").exists()).max_by_key(|v| v.file_name()) else { continue };
                let Some(m) = read_json(&vdir.path().join("manifest.json")) else { continue };
                let raw_name = m.get("name").and_then(|v| v.as_str()).unwrap_or(&id).to_string();
                let raw_desc = m.get("description").and_then(|v| v.as_str()).unwrap_or("").to_string();
                let enabled = prefs.as_ref().and_then(|p| p.pointer(&format!("/extensions/settings/{id}/state")).and_then(|v| v.as_i64())).map(|s| s == 1);
                out.push(Extension {
                    browser,
                    profile: profile_name.clone(),
                    name: chromium_msg(&vdir.path(), &m, &raw_name),
                    description: chromium_msg(&vdir.path(), &m, &raw_desc),
                    version: m.get("version").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    path: e.path().to_string_lossy().to_string(),
                    id,
                    enabled,
                    process,
                });
            }
        }
    }
    // Firefox
    if let Ok(rd) = std::fs::read_dir(format!("{roaming}\\Mozilla\\Firefox\\Profiles")) {
        for prof in rd.flatten() {
            let Some(j) = read_json(&prof.path().join("extensions.json")) else { continue };
            let pname = prof.file_name().to_string_lossy().split('.').next_back().unwrap_or("").to_string();
            for a in j.get("addons").and_then(|v| v.as_array()).cloned().unwrap_or_default() {
                if a.get("type").and_then(|v| v.as_str()) != Some("extension") || a.get("location").and_then(|v| v.as_str()) != Some("app-profile") {
                    continue;
                }
                out.push(Extension {
                    browser: "Firefox",
                    profile: pname.clone(),
                    id: a.get("id").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    name: a.pointer("/defaultLocale/name").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    version: a.get("version").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    description: a.pointer("/defaultLocale/description").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    path: a.get("path").and_then(|v| v.as_str()).unwrap_or("").to_string(),
                    enabled: a.get("active").and_then(|v| v.as_bool()),
                    process: "firefox.exe",
                });
            }
        }
    }
    out.sort_by(|a, b| a.browser.cmp(b.browser).then(a.name.to_lowercase().cmp(&b.name.to_lowercase())));
    out
}

// ------------------------------------------------------------------ junk files

#[derive(Clone, Debug)]
pub struct JunkCat {
    pub key: &'static str,
    pub name: &'static str,
    pub what: &'static str,
    pub admin: bool,
    pub default_on: bool,
    pub paths: Vec<PathBuf>,
    pub pattern: Option<&'static str>,
}

pub fn junk_categories() -> Vec<JunkCat> {
    let local = env("LOCALAPPDATA");
    let win = env("WINDIR");
    let pd = env("ProgramData");
    let mut browser_caches = vec![];
    for base in [format!("{local}\\Google\\Chrome\\User Data"), format!("{local}\\Microsoft\\Edge\\User Data"), format!("{local}\\BraveSoftware\\Brave-Browser\\User Data"), format!("{local}\\Vivaldi\\User Data")] {
        if let Ok(rd) = std::fs::read_dir(&base) {
            for p in rd.flatten() {
                for sub in ["Cache", "Code Cache", "GPUCache", "Service Worker\\CacheStorage"] {
                    let d = p.path().join(sub);
                    if d.exists() {
                        browser_caches.push(d);
                    }
                }
            }
        }
    }
    if let Ok(rd) = std::fs::read_dir(format!("{local}\\Mozilla\\Firefox\\Profiles")) {
        for p in rd.flatten() {
            browser_caches.push(p.path().join("cache2"));
        }
    }
    let pb = |v: &[String]| v.iter().map(PathBuf::from).collect::<Vec<_>>();
    vec![
        JunkCat { key: "usertemp", name: "Your temporary files", what: "Files programs left in your Temp folder", admin: false, default_on: true, paths: pb(&[env("TEMP")]), pattern: None },
        JunkCat { key: "wintemp", name: "Windows temporary files", what: "Windows' own Temp folder", admin: true, default_on: true, paths: pb(&[format!("{win}\\Temp")]), pattern: None },
        JunkCat { key: "thumbs", name: "Thumbnail cache", what: "Picture previews; Windows rebuilds them when needed", admin: false, default_on: false, paths: pb(&[format!("{local}\\Microsoft\\Windows\\Explorer")]), pattern: Some("thumbcache_") },
        JunkCat { key: "wer", name: "Error reports", what: "Problem reports already sent or no longer needed", admin: false, default_on: true, paths: pb(&[format!("{local}\\Microsoft\\Windows\\WER"), format!("{pd}\\Microsoft\\Windows\\WER")]), pattern: None },
        JunkCat { key: "dumps", name: "Crash dumps", what: "Memory dumps from crashed programs and Windows", admin: false, default_on: true, paths: pb(&[format!("{local}\\CrashDumps"), format!("{win}\\Minidump")]), pattern: None },
        JunkCat { key: "shaders", name: "Graphics shader caches", what: "Recreated automatically by games and apps", admin: false, default_on: false, paths: pb(&[format!("{local}\\D3DSCache"), format!("{local}\\NVIDIA\\DXCache"), format!("{local}\\NVIDIA\\GLCache"), format!("{local}\\AMD\\DxCache")]), pattern: None },
        JunkCat { key: "browsers", name: "Browser caches", what: "Saved copies of web pages (close browsers first)", admin: false, default_on: false, paths: browser_caches, pattern: None },
        JunkCat { key: "wudl", name: "Windows Update downloads", what: "Installer files Windows Update already used", admin: true, default_on: false, paths: pb(&[format!("{win}\\SoftwareDistribution\\Download")]), pattern: None },
        JunkCat { key: "dosvc", name: "Delivery Optimization cache", what: "Update pieces shared with other PCs", admin: true, default_on: false, paths: pb(&[format!("{win}\\ServiceProfiles\\NetworkService\\AppData\\Local\\Microsoft\\Windows\\DeliveryOptimization\\Cache")]), pattern: None },
        JunkCat { key: "logs", name: "Old setup logs", what: "Windows setup and servicing logs", admin: true, default_on: false, paths: pb(&[format!("{win}\\Logs\\CBS"), format!("{win}\\Panther")]), pattern: Some(".log") },
    ]
}

#[derive(Clone, Debug, Default)]
pub struct JunkScan {
    pub files: u64,
    pub bytes: u64,
    pub denied: bool,
}

fn walk_files(root: &Path, pattern: Option<&str>, older_than: i64, f: &mut dyn FnMut(&Path, u64)) -> bool {
    let mut ok = true;
    let mut stack = vec![root.to_path_buf()];
    while let Some(d) = stack.pop() {
        let rd = match std::fs::read_dir(&d) {
            Ok(r) => r,
            Err(_) => {
                ok = false;
                continue;
            }
        };
        for e in rd.flatten() {
            let Ok(m) = e.metadata() else { continue };
            if m.is_dir() {
                if pattern.is_none() && !m.file_type().is_symlink() {
                    stack.push(e.path());
                }
                continue;
            }
            if let Some(p) = pattern {
                let n = e.file_name().to_string_lossy().to_lowercase();
                if !(n.starts_with(p) || n.ends_with(p)) {
                    continue;
                }
            }
            if older_than > 0 {
                let age = m.modified().ok().and_then(|t| t.elapsed().ok()).map(|d| d.as_secs() as i64).unwrap_or(0);
                if age < older_than {
                    continue;
                }
            }
            f(&e.path(), m.len());
        }
    }
    ok
}

pub fn scan_junk(c: &JunkCat, older_than: i64) -> JunkScan {
    let mut s = JunkScan::default();
    for p in &c.paths {
        if !p.exists() {
            continue;
        }
        let ok = walk_files(p, c.pattern, older_than, &mut |_, len| {
            s.files += 1;
            s.bytes += len;
        });
        if !ok {
            s.denied = true;
        }
    }
    s
}

/// Delete junk files permanently; files in use are skipped. Returns (deleted, freed, skipped).
pub fn clean_junk(c: &JunkCat, older_than: i64) -> (u64, u64, u64) {
    let (mut del, mut freed, mut skipped) = (0u64, 0u64, 0u64);
    for p in &c.paths {
        if !p.exists() {
            continue;
        }
        walk_files(p, c.pattern, older_than, &mut |f, len| {
            if std::fs::remove_file(f).is_ok() {
                del += 1;
                freed += len;
            } else {
                skipped += 1;
            }
        });
        if c.pattern.is_none() {
            remove_empty_dirs(p, true);
        }
    }
    (del, freed, skipped)
}

fn remove_empty_dirs(p: &Path, keep_root: bool) {
    if let Ok(rd) = std::fs::read_dir(p) {
        for e in rd.flatten() {
            if e.file_type().map(|t| t.is_dir() && !t.is_symlink()).unwrap_or(false) {
                remove_empty_dirs(&e.path(), false);
            }
        }
    }
    if !keep_root {
        let _ = std::fs::remove_dir(p);
    }
}

pub fn recycle_bin() -> (u64, u64) {
    #[repr(C)]
    struct Info {
        size: u32,
        bytes: i64,
        items: i64,
    }
    #[link(name = "shell32")]
    extern "system" {
        fn SHQueryRecycleBinW(root: *const u16, info: *mut Info) -> i32;
    }
    let mut i = Info { size: std::mem::size_of::<Info>() as u32, bytes: 0, items: 0 };
    unsafe {
        SHQueryRecycleBinW(std::ptr::null(), &mut i);
    }
    (i.bytes.max(0) as u64, i.items.max(0) as u64)
}

pub fn empty_recycle_bin() -> Result<(), String> {
    #[link(name = "shell32")]
    extern "system" {
        fn SHEmptyRecycleBinW(hwnd: *mut c_void, root: *const u16, flags: u32) -> i32;
    }
    let r = unsafe { SHEmptyRecycleBinW(std::ptr::null_mut(), std::ptr::null(), 0x1 | 0x2 | 0x4) };
    if r == 0 || r == 0x8000FFFFu32 as i32 {
        Ok(())
    } else {
        Err(format!("Windows could not empty the Recycle Bin (code {r:#x})"))
    }
}

// ------------------------------------------------------------------ privacy traces

#[derive(Clone, Debug)]
pub struct Trace {
    pub key: &'static str,
    pub name: &'static str,
    pub what: &'static str,
    pub count: u64,
}

fn count_dir(p: &str) -> u64 {
    std::fs::read_dir(p).map(|r| r.flatten().filter(|e| e.file_type().map(|t| t.is_file()).unwrap_or(false)).count() as u64).unwrap_or(0)
}

fn count_values(path: &str) -> u64 {
    open(HKEY_CURRENT_USER, path, 0).map(|k| value_names(&k).into_iter().filter(|n| !n.is_empty()).count() as u64).unwrap_or(0)
}

const EXPLORER: &str = "Software\\Microsoft\\Windows\\CurrentVersion\\Explorer";

pub fn traces() -> Vec<Trace> {
    let recent = format!("{}\\Microsoft\\Windows\\Recent", env("APPDATA"));
    vec![
        Trace { key: "recent", name: "Recent files list", what: "Shortcuts to files you opened lately (the files themselves stay)", count: count_dir(&recent) },
        Trace { key: "jump", name: "Jump lists", what: "Recent items shown when you right-click taskbar icons", count: count_dir(&format!("{recent}\\AutomaticDestinations")) + count_dir(&format!("{recent}\\CustomDestinations")) },
        Trace { key: "run", name: "Run box history", what: "Commands typed in Win+R", count: count_values(&format!("{EXPLORER}\\RunMRU")) },
        Trace { key: "typed", name: "Typed addresses in File Explorer", what: "Paths typed in Explorer's address bar", count: count_values(&format!("{EXPLORER}\\TypedPaths")) },
        Trace { key: "search", name: "File Explorer search history", what: "Words searched in Explorer", count: count_values(&format!("{EXPLORER}\\WordWheelQuery")) },
        Trace { key: "clipboard", name: "Clipboard", what: "Whatever is copied right now", count: 1 },
        Trace { key: "dns", name: "DNS cache", what: "Remembered website addresses (refilled as you browse)", count: 1 },
    ]
}

pub fn clean_trace(key: &str) -> Result<String, String> {
    let recent = format!("{}\\Microsoft\\Windows\\Recent", env("APPDATA"));
    let del_files = |dir: &str| -> u64 {
        let mut n = 0;
        if let Ok(rd) = std::fs::read_dir(dir) {
            for e in rd.flatten() {
                if e.file_type().map(|t| t.is_file()).unwrap_or(false) && std::fs::remove_file(e.path()).is_ok() {
                    n += 1;
                }
            }
        }
        n
    };
    match key {
        "recent" => Ok(format!("{} items cleared", del_files(&recent))),
        "jump" => Ok(format!("{} items cleared", del_files(&format!("{recent}\\AutomaticDestinations")) + del_files(&format!("{recent}\\CustomDestinations")))),
        "run" | "typed" | "search" => {
            let sub = match key {
                "run" => "RunMRU",
                "typed" => "TypedPaths",
                _ => "WordWheelQuery",
            };
            util::run_hidden("reg.exe", &["delete", &format!("HKCU\\{EXPLORER}\\{sub}"), "/f"]).map(|_| "Cleared".to_string())
        }
        "clipboard" => util::powershell("Add-Type -AssemblyName System.Windows.Forms; [System.Windows.Forms.Clipboard]::Clear(); 'OK'").map(|_| "Clipboard cleared".into()),
        "dns" => util::run_hidden("ipconfig.exe", &["/flushdns"]).map(|_| "DNS cache cleared".into()),
        _ => Err("Unknown item".into()),
    }
}

// ------------------------------------------------------------------ secure delete

fn shred_file(p: &Path, passes: u32) -> Result<u64, String> {
    use std::io::{Seek, SeekFrom, Write};
    let len = std::fs::metadata(p).map_err(|e| e.to_string())?.len();
    let mut perms = std::fs::metadata(p).map_err(|e| e.to_string())?.permissions();
    if perms.readonly() {
        #[allow(clippy::permissions_set_readonly_false)]
        perms.set_readonly(false);
        let _ = std::fs::set_permissions(p, perms);
    }
    {
        let mut f = std::fs::OpenOptions::new().write(true).open(p).map_err(|e| e.to_string())?;
        let mut buf = vec![0u8; 1 << 20];
        let mut seed: u64 = 0x9E37_79B9_7F4A_7C15 ^ len ^ util::now_unix() as u64;
        for pass in 0..passes.max(1) {
            f.seek(SeekFrom::Start(0)).map_err(|e| e.to_string())?;
            let mut left = len;
            while left > 0 {
                let n = left.min(buf.len() as u64) as usize;
                if pass % 2 == 1 {
                    buf[..n].iter_mut().for_each(|b| *b = 0xFF);
                } else if pass > 0 {
                    for b in buf[..n].iter_mut() {
                        seed ^= seed << 13;
                        seed ^= seed >> 7;
                        seed ^= seed << 17;
                        *b = seed as u8;
                    }
                } else {
                    buf[..n].iter_mut().for_each(|b| *b = 0);
                }
                f.write_all(&buf[..n]).map_err(|e| e.to_string())?;
                left -= n as u64;
            }
            f.sync_all().map_err(|e| e.to_string())?;
        }
        f.set_len(0).map_err(|e| e.to_string())?;
    }
    // hide the original name before deleting
    let tmp = p.with_file_name(format!("{:016x}.del", util::now_unix() as u64 ^ len.rotate_left(17)));
    let target = if std::fs::rename(p, &tmp).is_ok() { tmp } else { p.to_path_buf() };
    std::fs::remove_file(&target).map_err(|e| e.to_string())?;
    Ok(len)
}

/// Overwrite and delete files or whole folders. Returns (files, bytes, errors).
pub fn secure_delete(paths: &[String], passes: u32) -> (u64, u64, Vec<String>) {
    let (mut files, mut bytes, mut errors) = (0u64, 0u64, vec![]);
    for p in paths {
        let path = Path::new(p);
        let pl = p.to_lowercase();
        if pl.len() <= 3 || pl.starts_with(&env("WINDIR").to_lowercase()) {
            errors.push(format!("{p}: refused (drive root or Windows folder)"));
            continue;
        }
        if path.is_dir() {
            let mut list = vec![];
            walk_files(path, None, 0, &mut |f, _| list.push(f.to_path_buf()));
            for f in list {
                match shred_file(&f, passes) {
                    Ok(n) => {
                        files += 1;
                        bytes += n;
                    }
                    Err(e) => errors.push(format!("{}: {e}", f.display())),
                }
            }
            remove_empty_dirs(path, false);
        } else if path.is_file() {
            match shred_file(path, passes) {
                Ok(n) => {
                    files += 1;
                    bytes += n;
                }
                Err(e) => errors.push(format!("{p}: {e}")),
            }
        } else {
            errors.push(format!("{p}: not found"));
        }
    }
    (files, bytes, errors)
}

// ------------------------------------------------------------------ Windows tools

pub const TOOLS: &[(&str, &str, &str, &str)] = &[
    ("🖥", "System Configuration", "msconfig.exe", "Boot options and services at start-up"),
    ("🚮", "Disk Cleanup", "cleanmgr.exe", "Windows' own space cleaner, incl. old Windows installs"),
    ("💽", "Defragment and Optimize Drives", "dfrgui.exe", "Optimise and TRIM drives"),
    ("⟲", "System Restore", "rstrui.exe", "Go back to an earlier restore point"),
    ("🔌", "Device Manager", "devmgmt.msc", "Hardware and drivers"),
    ("📜", "Event Viewer", "eventvwr.msc", "Windows' logs"),
    ("⚙", "Services", "services.msc", "Background services"),
    ("🖧", "Computer Management", "compmgmt.msc", "Disks, users, shares and more"),
    ("💾", "Disk Management", "diskmgmt.msc", "Partitions and volumes"),
    ("🔧", "Registry Editor", "regedit.exe", "Advanced settings database (be careful)"),
    ("⏰", "Task Scheduler", "taskschd.msc", "Scheduled tasks"),
    ("📈", "Resource Monitor", "resmon.exe", "Windows' live resource view"),
    ("📊", "Reliability Monitor", "perfmon.exe /rel", "Crash and problem history"),
    ("ℹ", "System Information", "msinfo32.exe", "Full hardware and software report"),
    ("🛡", "Windows Security", "windowsdefender:", "Antivirus and firewall"),
    ("🌐", "Network Connections", "ncpa.cpl", "Network adapters"),
    ("📦", "Programs and Features", "appwiz.cpl", "Windows' classic uninstaller"),
    ("🔋", "Power Options", "powercfg.cpl", "Power plans"),
    ("🔥", "Windows Firewall", "wf.msc", "Firewall rules"),
    ("📋", "DirectX Diagnostics", "dxdiag.exe", "Graphics and sound report"),
    ("💻", "Memory Diagnostic", "mdsched.exe", "Test RAM at next restart"),
    ("☰", "Command Prompt", "cmd.exe", "Classic command line"),
    ("⚡", "PowerShell", "powershell.exe", "Modern command line"),
];

pub fn open_tool(cmd: &str) -> bool {
    let (exe, args) = match cmd.split_once(' ') {
        Some((e, a)) => (e, a),
        None => (cmd, ""),
    };
    crate::sys::shell_open("open", exe, args)
}

/// Open Registry Editor at a key (Regedit remembers its last key).
pub fn open_regedit_at(key: &str) {
    let full = key.replacen("HKLM\\", "HKEY_LOCAL_MACHINE\\", 1).replacen("HKCU\\", "HKEY_CURRENT_USER\\", 1);
    let _ = util::run_hidden(
        "reg.exe",
        &["add", "HKCU\\Software\\Microsoft\\Windows\\CurrentVersion\\Applets\\Regedit", "/v", "LastKey", "/t", "REG_SZ", "/d", &format!("Computer\\{full}"), "/f"],
    );
    crate::sys::shell_open("open", "regedit.exe", "-m");
}

pub fn is_running(process: &str, procs: &[String]) -> bool {
    procs.iter().any(|p| p.eq_ignore_ascii_case(process))
}
