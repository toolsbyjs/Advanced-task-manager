//! Small helpers shared across the app: string conversion, formatting, ring buffers,
//! background jobs and hidden command / PowerShell execution.

use std::collections::VecDeque;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex};

pub fn wide(s: &str) -> Vec<u16> {
    s.encode_utf16().chain(std::iter::once(0)).collect()
}

pub fn from_wide(buf: &[u16]) -> String {
    let end = buf.iter().position(|&c| c == 0).unwrap_or(buf.len());
    String::from_utf16_lossy(&buf[..end])
}

/// # Safety
/// `p` must point to a NUL terminated UTF-16 string or be null.
pub unsafe fn from_wide_ptr(p: *const u16) -> String {
    if p.is_null() {
        return String::new();
    }
    let mut len = 0usize;
    while *p.add(len) != 0 {
        len += 1;
        if len > 1 << 20 {
            break;
        }
    }
    String::from_utf16_lossy(std::slice::from_raw_parts(p, len))
}

pub fn fmt_bytes(b: f64) -> String {
    let b = b.max(0.0);
    const U: [&str; 6] = ["B", "KB", "MB", "GB", "TB", "PB"];
    let mut v = b;
    let mut i = 0;
    while v >= 1024.0 && i < U.len() - 1 {
        v /= 1024.0;
        i += 1;
    }
    if i == 0 {
        format!("{:.0} {}", v, U[i])
    } else if v >= 100.0 {
        format!("{:.0} {}", v, U[i])
    } else if v >= 10.0 {
        format!("{:.1} {}", v, U[i])
    } else {
        format!("{:.2} {}", v, U[i])
    }
}

pub fn fmt_rate(b: f64) -> String {
    format!("{}/s", fmt_bytes(b))
}

pub fn fmt_bits(bytes_per_sec: f64) -> String {
    let b = bytes_per_sec * 8.0;
    if b >= 1e9 {
        format!("{:.2} Gbps", b / 1e9)
    } else if b >= 1e6 {
        format!("{:.1} Mbps", b / 1e6)
    } else if b >= 1e3 {
        format!("{:.0} Kbps", b / 1e3)
    } else {
        format!("{:.0} bps", b)
    }
}

pub fn fmt_dur(secs: u64) -> String {
    let d = secs / 86400;
    let h = (secs % 86400) / 3600;
    let m = (secs % 3600) / 60;
    let s = secs % 60;
    if d > 0 {
        format!("{}d {:02}:{:02}:{:02}", d, h, m, s)
    } else {
        format!("{:02}:{:02}:{:02}", h, m, s)
    }
}

pub fn fmt_num(n: u64) -> String {
    let s = n.to_string();
    let mut out = String::new();
    for (i, c) in s.chars().enumerate() {
        if i > 0 && (s.len() - i) % 3 == 0 {
            out.push(',');
        }
        out.push(c);
    }
    out
}

/// Windows FILETIME (100ns since 1601) to unix seconds.
pub fn filetime_to_unix(ft: i64) -> i64 {
    if ft <= 0 {
        return 0;
    }
    ft / 10_000_000 - 11_644_473_600
}

pub fn now_unix() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0)
}

pub fn now_unix_f() -> f64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs_f64())
        .unwrap_or(0.0)
}

pub fn fmt_time(unix: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(unix, 0) {
        chrono::LocalResult::Single(t) => t.format("%Y-%m-%d %H:%M:%S").to_string(),
        _ => "-".into(),
    }
}

pub fn fmt_clock(unix: i64) -> String {
    use chrono::{Local, TimeZone};
    match Local.timestamp_opt(unix, 0) {
        chrono::LocalResult::Single(t) => t.format("%H:%M:%S").to_string(),
        _ => "-".into(),
    }
}

pub fn stamp() -> String {
    chrono::Local::now().format("%Y%m%d-%H%M%S").to_string()
}

/// Fixed capacity ring buffer of samples used for history graphs.
#[derive(Clone, Debug)]
pub struct Ring {
    pub data: VecDeque<f32>,
    pub cap: usize,
}

impl Default for Ring {
    fn default() -> Self {
        Ring::new(120)
    }
}

impl Ring {
    pub fn new(cap: usize) -> Self {
        Self { data: VecDeque::with_capacity(cap), cap }
    }
    pub fn push(&mut self, v: f32) {
        if self.data.len() >= self.cap {
            self.data.pop_front();
        }
        self.data.push_back(v);
    }
    pub fn last(&self) -> f32 {
        self.data.back().copied().unwrap_or(0.0)
    }
    pub fn len(&self) -> usize {
        self.data.len()
    }
    pub fn tail(&self, n: usize) -> impl Iterator<Item = f32> + '_ {
        let skip = self.data.len().saturating_sub(n);
        self.data.iter().skip(skip).copied()
    }
    pub fn avg(&self, n: usize) -> f32 {
        let k = n.min(self.data.len());
        if k == 0 {
            return 0.0;
        }
        self.tail(k).sum::<f32>() / k as f32
    }
    pub fn max(&self, n: usize) -> f32 {
        self.tail(n).fold(0.0, f32::max)
    }
    pub fn to_vec(&self, n: usize) -> Vec<f32> {
        self.tail(n).collect()
    }
}

/// Least squares slope (units per sample) and r^2 of a series.
pub fn regression(v: &[f32]) -> (f64, f64) {
    let n = v.len();
    if n < 3 {
        return (0.0, 0.0);
    }
    let nf = n as f64;
    let mx = (nf - 1.0) / 2.0;
    let my = v.iter().map(|&x| x as f64).sum::<f64>() / nf;
    let mut sxy = 0.0;
    let mut sxx = 0.0;
    let mut syy = 0.0;
    for (i, &y) in v.iter().enumerate() {
        let dx = i as f64 - mx;
        let dy = y as f64 - my;
        sxy += dx * dy;
        sxx += dx * dx;
        syy += dy * dy;
    }
    if sxx == 0.0 {
        return (0.0, 0.0);
    }
    let slope = sxy / sxx;
    let r2 = if syy == 0.0 { 0.0 } else { (sxy * sxy) / (sxx * syy) };
    (slope, r2)
}

pub fn std_dev(v: &[f32]) -> f32 {
    if v.len() < 2 {
        return 0.0;
    }
    let m = v.iter().sum::<f32>() / v.len() as f32;
    (v.iter().map(|x| (x - m) * (x - m)).sum::<f32>() / v.len() as f32).sqrt()
}

/// Folder where settings, snapshots, recordings and reports live.
pub fn data_dir() -> PathBuf {
    let base = std::env::var("APPDATA")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let p = base.join("AdvancedTaskManager");
    // Carry over settings, snapshots and recordings from the earlier name once.
    let old = base.join("DeepTaskManager");
    if !p.exists() && old.exists() {
        let _ = std::fs::rename(&old, &p);
    }
    let _ = std::fs::create_dir_all(&p);
    p
}

pub fn sub_dir(name: &str) -> PathBuf {
    let p = data_dir().join(name);
    let _ = std::fs::create_dir_all(&p);
    p
}

/// A one-shot background job whose result the UI polls every frame.
pub struct Job<T> {
    result: Arc<Mutex<Option<Result<T, String>>>>,
    running: Arc<AtomicBool>,
    pub started: bool,
    pub finished_at: Option<std::time::Instant>,
}

impl<T> Default for Job<T> {
    fn default() -> Self {
        Self {
            result: Arc::new(Mutex::new(None)),
            running: Arc::new(AtomicBool::new(false)),
            started: false,
            finished_at: None,
        }
    }
}

impl<T: Send + 'static> Job<T> {
    pub fn start<F>(&mut self, ctx: &egui::Context, f: F)
    where
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        if self.running.load(Ordering::SeqCst) {
            return;
        }
        self.started = true;
        self.running.store(true, Ordering::SeqCst);
        let result = self.result.clone();
        let running = self.running.clone();
        let ctx = ctx.clone();
        std::thread::spawn(move || {
            let r = f();
            *result.lock().unwrap() = Some(r);
            running.store(false, Ordering::SeqCst);
            ctx.request_repaint();
        });
    }
    pub fn ensure<F>(&mut self, ctx: &egui::Context, f: F)
    where
        F: FnOnce() -> Result<T, String> + Send + 'static,
    {
        if !self.started {
            self.start(ctx, f);
        }
    }
}

impl<T> Job<T> {
    pub fn is_running(&self) -> bool {
        self.running.load(Ordering::SeqCst)
    }
    pub fn with<R>(&self, f: impl FnOnce(Option<&Result<T, String>>) -> R) -> R {
        let g = self.result.lock().unwrap();
        f(g.as_ref())
    }
    pub fn take_ok(&self) -> Option<T>
    where
        T: Clone,
    {
        let g = self.result.lock().unwrap();
        match g.as_ref() {
            Some(Ok(v)) => Some(v.clone()),
            _ => None,
        }
    }
}

const CREATE_NO_WINDOW: u32 = 0x0800_0000;

pub fn run_hidden(program: &str, args: &[&str]) -> Result<String, String> {
    let mut cmd = std::process::Command::new(program);
    cmd.args(args);
    #[cfg(windows)]
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(CREATE_NO_WINDOW);
    }
    let out = cmd.output().map_err(|e| format!("{program}: {e}"))?;
    let mut s = decode_console(&out.stdout);
    if !out.status.success() {
        let err = decode_console(&out.stderr);
        if s.trim().is_empty() {
            return Err(if err.trim().is_empty() { format!("{program} exited with {}", out.status) } else { err });
        }
        if !err.trim().is_empty() {
            s.push_str("\n");
            s.push_str(&err);
        }
    }
    Ok(s)
}

fn decode_console(b: &[u8]) -> String {
    match String::from_utf8(b.to_vec()) {
        Ok(s) => s,
        Err(_) => b.iter().map(|&c| c as char).collect(),
    }
}

fn base64(data: &[u8]) -> String {
    const T: &[u8; 64] = b"ABCDEFGHIJKLMNOPQRSTUVWXYZabcdefghijklmnopqrstuvwxyz0123456789+/";
    let mut out = String::with_capacity((data.len() + 2) / 3 * 4);
    for chunk in data.chunks(3) {
        let b = [chunk[0], *chunk.get(1).unwrap_or(&0), *chunk.get(2).unwrap_or(&0)];
        let n = ((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32;
        out.push(T[(n >> 18) as usize & 63] as char);
        out.push(T[(n >> 12) as usize & 63] as char);
        out.push(if chunk.len() > 1 { T[(n >> 6) as usize & 63] as char } else { '=' });
        out.push(if chunk.len() > 2 { T[n as usize & 63] as char } else { '=' });
    }
    out
}

/// Run a PowerShell script without a console window; output is UTF-8.
pub fn powershell(script: &str) -> Result<String, String> {
    let full = format!(
        "$ProgressPreference='SilentlyContinue';$ErrorActionPreference='Continue';[Console]::OutputEncoding=[Text.Encoding]::UTF8;{script}"
    );
    let utf16: Vec<u8> = full.encode_utf16().flat_map(|c| c.to_le_bytes()).collect();
    let enc = base64(&utf16);
    run_hidden(
        "powershell.exe",
        &["-NoProfile", "-NonInteractive", "-ExecutionPolicy", "Bypass", "-EncodedCommand", &enc],
    )
}

/// Run a PowerShell pipeline and parse its result as a list of JSON objects.
pub fn ps_json(script: &str) -> Result<Vec<serde_json::Value>, String> {
    let s = powershell(&format!("@({script}) | ConvertTo-Json -Depth 4 -Compress"))?;
    parse_json_list(&s)
}

pub fn parse_json_list(s: &str) -> Result<Vec<serde_json::Value>, String> {
    let start = s.find(|c| c == '[' || c == '{');
    let Some(start) = start else {
        if s.trim().is_empty() {
            return Ok(vec![]);
        }
        return Err(s.trim().chars().take(600).collect());
    };
    let body = &s[start..];
    let end = body.rfind(|c| c == ']' || c == '}').map(|i| i + 1).unwrap_or(body.len());
    match serde_json::from_str::<serde_json::Value>(&body[..end]) {
        Ok(serde_json::Value::Array(a)) => Ok(a),
        Ok(v @ serde_json::Value::Object(_)) => Ok(vec![v]),
        Ok(_) => Ok(vec![]),
        Err(e) => Err(format!("Could not read the answer: {e}")),
    }
}

/// Run a PowerShell script returning a single JSON object.
pub fn ps_object(script: &str) -> Result<serde_json::Value, String> {
    let s = powershell(&format!("& {{\n{script}\n}} | Select-Object -Last 1 | ConvertTo-Json -Depth 5 -Compress"))?;
    let start = s.find('{').ok_or_else(|| {
        let t: String = s.trim().chars().take(600).collect();
        if t.is_empty() { "Windows returned no answer".to_string() } else { t }
    })?;
    let end = s.rfind('}').map(|i| i + 1).unwrap_or(s.len());
    serde_json::from_str(&s[start..end]).map_err(|e| format!("Could not read the answer: {e}"))
}

/// Escape a value for a single-quoted PowerShell string.
pub fn ps_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "''"))
}

pub fn json_str(v: &serde_json::Value) -> String {
    match v {
        serde_json::Value::Null => String::new(),
        serde_json::Value::String(s) => {
            // PowerShell 5 dates come as "/Date(1700000000000)/"
            if let Some(rest) = s.strip_prefix("/Date(") {
                if let Some(num) = rest.split(|c| c == ')' || c == '+' || c == '-').next() {
                    if let Ok(ms) = num.parse::<i64>() {
                        return fmt_time(ms / 1000);
                    }
                }
            }
            s.clone()
        }
        serde_json::Value::Bool(b) => if *b { "Yes".into() } else { "No".into() },
        serde_json::Value::Number(n) => n.to_string(),
        serde_json::Value::Array(a) => a.iter().map(json_str).collect::<Vec<_>>().join(", "),
        serde_json::Value::Object(o) => {
            if let Some(v) = o.get("value") {
                return json_str(v);
            }
            if let Some(v) = o.get("DateTime") {
                return json_str(v);
            }
            o.iter().map(|(k, v)| format!("{k}={}", json_str(v))).collect::<Vec<_>>().join("; ")
        }
    }
}

pub fn jget<'a>(v: &'a serde_json::Value, k: &str) -> &'a serde_json::Value {
    static NULL: serde_json::Value = serde_json::Value::Null;
    v.get(k).unwrap_or(&NULL)
}

pub fn jstr(v: &serde_json::Value, k: &str) -> String {
    json_str(jget(v, k))
}

pub fn jf64(v: &serde_json::Value, k: &str) -> f64 {
    match jget(v, k) {
        serde_json::Value::Number(n) => n.as_f64().unwrap_or(0.0),
        serde_json::Value::String(s) => s.trim().parse().unwrap_or(0.0),
        serde_json::Value::Bool(b) => {
            if *b {
                1.0
            } else {
                0.0
            }
        }
        _ => 0.0,
    }
}

pub fn open_path(p: &std::path::Path) {
    let _ = std::process::Command::new("explorer.exe").arg(p).spawn();
}

pub fn wildcard_match(pattern: &str, text: &str) -> bool {
    let p = pattern.to_lowercase();
    let t = text.to_lowercase();
    if !p.contains('*') {
        return t == p;
    }
    let parts: Vec<&str> = p.split('*').collect();
    let mut pos = 0usize;
    for (i, part) in parts.iter().enumerate() {
        if part.is_empty() {
            continue;
        }
        if i == 0 {
            if !t.starts_with(part) {
                return false;
            }
            pos = part.len();
        } else if let Some(f) = t[pos..].find(part) {
            pos += f + part.len();
        } else {
            return false;
        }
    }
    if let Some(last) = parts.last() {
        if !last.is_empty() && !t.ends_with(last) {
            return false;
        }
    }
    true
}
