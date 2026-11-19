//! Background sampler: collects every counter once per interval, keeps history,
//! records process starts/exits, runs automation rules and performance recording.

use crate::config::{Action, Config, Metric, Op, ScheduleKind};
use crate::nt::{self, RawThread};
use crate::pdh::Pdh;
use crate::sys::{self, ProcStatic};
use crate::util::{self, Ring};
use std::collections::{HashMap, HashSet, VecDeque};
use std::io::Write;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

pub const HIST_LONG: usize = 3600;
pub const HIST_PROC: usize = 300;

#[derive(Clone, Debug, Default)]
pub struct ThreadInfo {
    pub raw: RawThread,
    pub cpu: f32,
    pub cswitch_delta: u32,
}

#[derive(Clone, Debug)]
pub struct ProcInfo {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub create_time: i64,
    pub cpu: f32,
    pub cpu_kernel: f32,
    pub cpu_user: f32,
    pub cycles_delta: u64,
    pub cpu_time: f64,
    pub threads: u32,
    pub threads_peak: u32,
    pub handles: u32,
    pub session: u32,
    pub base_priority: i32,
    pub ws: u64,
    pub private_ws: u64,
    pub private_bytes: u64,
    pub commit: u64,
    pub peak_commit: u64,
    pub peak_ws: u64,
    pub virtual_size: u64,
    pub peak_virtual: u64,
    pub paged_pool: u64,
    pub nonpaged_pool: u64,
    pub page_faults: u32,
    pub page_faults_sec: f32,
    pub hard_faults: u32,
    pub read_bytes: u64,
    pub write_bytes: u64,
    pub other_bytes: u64,
    pub read_ops: u64,
    pub write_ops: u64,
    pub other_ops: u64,
    pub io_read_rate: f32,
    pub io_write_rate: f32,
    pub io_other_rate: f32,
    pub gpu: f32,
    pub gpu_dedicated: u64,
    pub gpu_shared: u64,
    pub connections: u32,
    pub suspended: bool,
    pub st: Arc<ProcStatic>,
    pub hist_cpu: Ring,
    pub hist_mem: Ring,
    pub hist_io: Ring,
    pub hist_handles: Ring,
    pub hist_threads: Ring,
    pub first_seen: i64,
    pub peak_cpu: f32,
    pub thread_list: Vec<ThreadInfo>,
}

#[derive(Clone, Debug)]
pub struct ExitedProc {
    pub pid: u32,
    pub name: String,
    pub path: String,
    pub started: i64,
    pub exited: i64,
    pub peak_cpu: f32,
    pub peak_private: u64,
    pub cpu_time: f64,
    pub user: String,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Severity {
    Info,
    Warning,
    Critical,
}

#[derive(Clone, Debug)]
pub struct TimelineEvent {
    pub time: i64,
    pub category: &'static str,
    pub severity: Severity,
    pub text: String,
    pub pid: Option<u32>,
}

#[derive(Clone, Debug)]
pub struct Alert {
    pub time: i64,
    pub severity: Severity,
    pub title: String,
    pub detail: String,
    pub acknowledged: bool,
}

#[derive(Clone, Debug, Default)]
pub struct CoreInfo {
    pub usage: f32,
    pub kernel: f32,
    pub dpc: f32,
    pub interrupt: f32,
    pub interrupts_sec: f32,
    pub mhz: u32,
    pub max_mhz: u32,
    pub hist: Ring,
}

#[derive(Clone, Debug, Default)]
pub struct MemInfo {
    pub st: sys::MemStatus,
    pub standby: u64,
    pub modified: u64,
    pub free_zero: u64,
    pub compressed: u64,
    pub pages_sec: f32,
    pub faults_sec: f32,
    pub pool_paged: u64,
    pub pool_nonpaged: u64,
}

#[derive(Clone, Debug)]
pub struct DiskPerf {
    pub name: String,
    pub active: f32,
    pub read: f32,
    pub write: f32,
    pub queue: f32,
    pub resp_ms: f32,
    pub hist_active: Ring,
    pub hist_read: Ring,
    pub hist_write: Ring,
}

#[derive(Clone, Debug)]
pub struct NetIf {
    pub row: sys::IfRow,
    pub rx: f32,
    pub tx: f32,
    pub hist_rx: Ring,
    pub hist_tx: Ring,
}

#[derive(Clone, Debug, Default)]
pub struct GpuInfo {
    pub engines: Vec<(String, f32)>,
    pub total: f32,
    pub dedicated_used: u64,
    pub shared_used: u64,
    pub engine_hist: HashMap<String, Ring>,
}

#[derive(Clone, Debug, Default)]
pub struct SysCounters {
    pub ctx_switches: f32,
    pub syscalls: f32,
    pub proc_queue: f32,
    pub interrupts: f32,
    pub uptime: u64,
    pub perf_pct: f32,
    pub exceptions: f32,
}

pub struct History {
    pub cpu: Ring,
    pub cpu_kernel: Ring,
    pub mem: Ring,
    pub commit: Ring,
    pub disk: Ring,
    pub disk_read: Ring,
    pub disk_write: Ring,
    pub net_rx: Ring,
    pub net_tx: Ring,
    pub gpu: Ring,
    pub temp: Ring,
    pub procs: Ring,
    pub times: VecDeque<i64>,
}

impl History {
    fn new() -> Self {
        Self {
            cpu: Ring::new(HIST_LONG),
            cpu_kernel: Ring::new(HIST_LONG),
            mem: Ring::new(HIST_LONG),
            commit: Ring::new(HIST_LONG),
            disk: Ring::new(HIST_LONG),
            disk_read: Ring::new(HIST_LONG),
            disk_write: Ring::new(HIST_LONG),
            net_rx: Ring::new(HIST_LONG),
            net_tx: Ring::new(HIST_LONG),
            gpu: Ring::new(HIST_LONG),
            temp: Ring::new(HIST_LONG),
            procs: Ring::new(HIST_LONG),
            times: VecDeque::with_capacity(HIST_LONG),
        }
    }
}

pub struct Recorder {
    pub path: std::path::PathBuf,
    pub file: std::fs::File,
    pub started: Instant,
    pub until: Option<Instant>,
    pub rows: u64,
}

pub struct State {
    pub procs: HashMap<u32, ProcInfo>,
    pub exited: VecDeque<ExitedProc>,
    pub timeline: VecDeque<TimelineEvent>,
    pub alerts: VecDeque<Alert>,
    pub cpu_total: f32,
    pub cpu_kernel: f32,
    pub cpu_user: f32,
    pub cpu_dpc: f32,
    pub cpu_interrupt: f32,
    pub cores: Vec<CoreInfo>,
    pub mem: MemInfo,
    pub disks: Vec<DiskPerf>,
    pub volumes: Vec<sys::Volume>,
    pub nets: Vec<NetIf>,
    pub gpu: GpuInfo,
    pub temps: Vec<(String, f32)>,
    pub sys: SysCounters,
    pub hist: History,
    pub cpu_name: String,
    pub ncpu: usize,
    pub samples: u64,
    pub paused: bool,
    pub recorder: Option<Recorder>,
    pub config: Config,
    pub config_dirty: bool,
    pub elevated: bool,
    pub last_sample: i64,
    pub sample_cost_ms: f32,
    pub pending_actions: Vec<String>,
    pub snapshot_requests: u32,
    pub report_requests: u32,
}

impl State {
    pub fn log(&mut self, category: &'static str, severity: Severity, text: String, pid: Option<u32>) {
        if self.timeline.len() >= 3000 {
            self.timeline.pop_front();
        }
        self.timeline.push_back(TimelineEvent { time: util::now_unix(), category, severity, text, pid });
    }
    pub fn alert(&mut self, severity: Severity, title: String, detail: String) {
        if self.alerts.len() >= 500 {
            self.alerts.pop_front();
        }
        self.log("Alert", severity, format!("{title}: {detail}"), None);
        self.alerts.push_back(Alert { time: util::now_unix(), severity, title, detail, acknowledged: false });
    }
    pub fn unacked(&self) -> usize {
        self.alerts.iter().filter(|a| !a.acknowledged).count()
    }
    pub fn mem_used_pct(&self) -> f32 {
        if self.mem.st.total == 0 {
            0.0
        } else {
            (self.mem.st.total - self.mem.st.avail) as f32 / self.mem.st.total as f32 * 100.0
        }
    }
    pub fn disk_active_max(&self) -> f32 {
        self.disks.iter().map(|d| d.active).fold(0.0, f32::max)
    }
    pub fn net_rx(&self) -> f32 {
        self.nets.iter().map(|n| n.rx).sum()
    }
    pub fn net_tx(&self) -> f32 {
        self.nets.iter().map(|n| n.tx).sum()
    }
    pub fn max_temp(&self) -> f32 {
        self.temps.iter().map(|t| t.1).fold(0.0, f32::max)
    }

    pub fn start_recording(&mut self, secs: Option<u32>) -> Result<std::path::PathBuf, String> {
        if self.recorder.is_some() {
            return Err("Already recording".into());
        }
        let path = util::sub_dir("recordings").join(format!("recording-{}.csv", util::stamp()));
        let mut file = std::fs::File::create(&path).map_err(|e| e.to_string())?;
        let _ = writeln!(
            file,
            "time,cpu_pct,kernel_pct,mem_used_pct,mem_used_bytes,commit_bytes,disk_active_pct,disk_read_bps,disk_write_bps,net_rx_bps,net_tx_bps,gpu_pct,temp_c,processes,threads,handles,top_cpu_process,top_cpu_pct,top_mem_process,top_mem_bytes"
        );
        self.log("Recording", Severity::Info, format!("Recording started: {}", path.display()), None);
        self.recorder = Some(Recorder {
            path: path.clone(),
            file,
            started: Instant::now(),
            until: secs.map(|s| Instant::now() + Duration::from_secs(s as u64)),
            rows: 0,
        });
        Ok(path)
    }

    pub fn stop_recording(&mut self) {
        if let Some(r) = self.recorder.take() {
            self.log("Recording", Severity::Info, format!("Recording saved ({} rows): {}", r.rows, r.path.display()), None);
        }
    }
}

pub type Shared = Arc<Mutex<State>>;

struct Prev {
    proc_times: HashMap<(u32, i64), (i64, i64, u64, u64, u64, u64, u32)>,
    thread_times: HashMap<u32, (i64, u32)>,
    cpu: Vec<nt::CpuTimes>,
    net: HashMap<u32, (u64, u64)>,
    at: Instant,
}

pub fn start(ctx: egui::Context, elevated: bool) -> Shared {
    let config = Config::load();
    let ncpu = sys::cpu_count();
    let cpu_name = sys::reg_string("HARDWARE\\DESCRIPTION\\System\\CentralProcessor\\0", "ProcessorNameString")
        .unwrap_or_else(|| "Processor".into());
    let state = State {
        procs: HashMap::new(),
        exited: VecDeque::new(),
        timeline: VecDeque::new(),
        alerts: VecDeque::new(),
        cpu_total: 0.0,
        cpu_kernel: 0.0,
        cpu_user: 0.0,
        cpu_dpc: 0.0,
        cpu_interrupt: 0.0,
        cores: (0..ncpu).map(|_| CoreInfo { hist: Ring::new(120), ..Default::default() }).collect(),
        mem: MemInfo::default(),
        disks: vec![],
        volumes: sys::volumes(),
        nets: vec![],
        gpu: GpuInfo::default(),
        temps: vec![],
        sys: SysCounters::default(),
        hist: History::new(),
        cpu_name,
        ncpu,
        samples: 0,
        paused: false,
        recorder: None,
        config,
        config_dirty: false,
        elevated,
        last_sample: 0,
        sample_cost_ms: 0.0,
        pending_actions: vec![],
        snapshot_requests: 0,
        report_requests: 0,
    };
    let shared = Arc::new(Mutex::new(state));
    {
        let mut s = shared.lock().unwrap();
        s.log(
            "App",
            Severity::Info,
            format!("Monitoring started ({} view available)", if elevated { "Admin" } else { "Standard" }),
            None,
        );
    }
    let sh = shared.clone();
    std::thread::Builder::new()
        .name("sampler".into())
        .spawn(move || run(sh, ctx))
        .expect("sampler thread");
    shared
}

fn parse_gpu_instance(name: &str) -> (Option<u32>, String) {
    // pid_1234_luid_0x..._phys_0_eng_0_engtype_3D
    let mut pid = None;
    let mut eng = String::from("Other");
    let parts: Vec<&str> = name.split('_').collect();
    for i in 0..parts.len() {
        if parts[i] == "pid" && i + 1 < parts.len() {
            pid = parts[i + 1].parse().ok();
        }
        if parts[i] == "engtype" && i + 1 < parts.len() {
            eng = parts[i + 1..].join(" ");
            if eng.is_empty() {
                eng = "Other".into();
            }
        }
    }
    (pid, eng)
}

fn run(shared: Shared, ctx: egui::Context) {
    let mut buf_proc: Vec<u64> = Vec::new();
    let mut buf_cpu: Vec<u64> = Vec::new();
    let mut statics: HashMap<(u32, i64), Arc<ProcStatic>> = HashMap::new();
    let mut prev = Prev { proc_times: HashMap::new(), thread_times: HashMap::new(), cpu: vec![], net: HashMap::new(), at: Instant::now() };
    let mut pdh = Pdh::new();
    if let Some(p) = pdh.as_mut() {
        p.add("disk_idle", "\\PhysicalDisk(*)\\% Idle Time");
        p.add("disk_read", "\\PhysicalDisk(*)\\Disk Read Bytes/sec");
        p.add("disk_write", "\\PhysicalDisk(*)\\Disk Write Bytes/sec");
        p.add("disk_queue", "\\PhysicalDisk(*)\\Current Disk Queue Length");
        p.add("disk_resp", "\\PhysicalDisk(*)\\Avg. Disk sec/Transfer");
        p.add("gpu_eng", "\\GPU Engine(*)\\Utilization Percentage");
        p.add("gpu_ded", "\\GPU Adapter Memory(*)\\Dedicated Usage");
        p.add("gpu_shr", "\\GPU Adapter Memory(*)\\Shared Usage");
        p.add("gpu_pded", "\\GPU Process Memory(*)\\Dedicated Usage");
        p.add("gpu_pshr", "\\GPU Process Memory(*)\\Shared Usage");
        p.add("thermal", "\\Thermal Zone Information(*)\\Temperature");
        p.add("ctx", "\\System\\Context Switches/sec");
        p.add("syscalls", "\\System\\System Calls/sec");
        p.add("pqueue", "\\System\\Processor Queue Length");
        p.add("exceptions", "\\System\\Exception Dispatches/sec");
        p.add("perf", "\\Processor Information(_Total)\\% Processor Performance");
        p.add("interrupts", "\\Processor(_Total)\\Interrupts/sec");
        p.add("standby_c", "\\Memory\\Standby Cache Core Bytes");
        p.add("standby_n", "\\Memory\\Standby Cache Normal Priority Bytes");
        p.add("standby_r", "\\Memory\\Standby Cache Reserve Bytes");
        p.add("modified", "\\Memory\\Modified Page List Bytes");
        p.add("freezero", "\\Memory\\Free & Zero Page List Bytes");
        p.add("pages", "\\Memory\\Pages/sec");
        p.add("faults", "\\Memory\\Page Faults/sec");
        p.add("pool_p", "\\Memory\\Pool Paged Bytes");
        p.add("pool_np", "\\Memory\\Pool Nonpaged Bytes");
        p.collect();
    }
    let mut first = true;
    let mut last_conn = Instant::now() - Duration::from_secs(60);
    let mut last_vol = Instant::now();
    let mut conn_counts: HashMap<u32, u32> = HashMap::new();
    let mut rule_since: HashMap<(u64, u32), Instant> = HashMap::new();
    let mut rule_last: HashMap<(u64, u32), Instant> = HashMap::new();
    let mut profiled: HashSet<(u64, u32, i64)> = HashSet::new();
    let mut last_schedule_check = Instant::now();

    loop {
        let (interval, paused) = {
            let s = shared.lock().unwrap();
            (s.config.sample_ms.clamp(250, 10_000), s.paused)
        };
        if !first {
            std::thread::sleep(Duration::from_millis(if prev.cpu.is_empty() { 400 } else { interval }));
        }
        first = false;
        if paused {
            continue;
        }
        let t0 = Instant::now();
        let elapsed = prev.at.elapsed().as_secs_f64().max(0.05);
        prev.at = Instant::now();

        // ---- gather (outside the lock)
        let raw = nt::query_processes(&mut buf_proc);
        let cpu_times = nt::query_cpu_times(&mut buf_cpu);
        let mem = sys::mem_status();
        let ncpu = cpu_times.len().max(1);
        let power = sys::cpu_power(ncpu);
        let ifs = sys::interfaces();
        if let Some(p) = pdh.as_mut() {
            p.collect();
        }
        if last_conn.elapsed() > Duration::from_secs(3) {
            last_conn = Instant::now();
            conn_counts.clear();
            for c in sys::connections() {
                *conn_counts.entry(c.pid).or_default() += 1;
            }
        }
        let vols = if last_vol.elapsed() > Duration::from_secs(15) {
            last_vol = Instant::now();
            Some(sys::volumes())
        } else {
            None
        };
        // static info for new processes
        let mut new_statics = vec![];
        for r in &raw {
            let key = (r.pid, r.create_time);
            if !statics.contains_key(&key) {
                new_statics.push(key);
            }
        }
        for key in new_statics {
            statics.insert(key, Arc::new(sys::query_static(key.0)));
        }
        let live: HashSet<(u32, i64)> = raw.iter().map(|r| (r.pid, r.create_time)).collect();
        statics.retain(|k, _| live.contains(k));

        // GPU per process
        let mut gpu_proc: HashMap<u32, HashMap<String, f64>> = HashMap::new();
        let mut gpu_eng: HashMap<String, f64> = HashMap::new();
        let mut gpu_pded: HashMap<u32, u64> = HashMap::new();
        let mut gpu_pshr: HashMap<u32, u64> = HashMap::new();
        let mut disks_raw: Vec<(String, f32, f32, f32, f32, f32)> = vec![];
        let mut temps = vec![];
        let mut sysc = SysCounters::default();
        let mut meminfo = MemInfo { st: mem.clone(), ..Default::default() };
        let mut gpu_ded = 0u64;
        let mut gpu_shr = 0u64;
        if let Some(p) = pdh.as_ref() {
            for (inst, v) in p.array("gpu_eng") {
                let (pid, eng) = parse_gpu_instance(&inst);
                if let Some(pid) = pid {
                    *gpu_proc.entry(pid).or_default().entry(eng.clone()).or_default() += v;
                }
                *gpu_eng.entry(eng).or_default() += v;
            }
            for (inst, v) in p.array("gpu_pded") {
                if let (Some(pid), _) = parse_gpu_instance(&inst) {
                    *gpu_pded.entry(pid).or_default() += v as u64;
                }
            }
            for (inst, v) in p.array("gpu_pshr") {
                if let (Some(pid), _) = parse_gpu_instance(&inst) {
                    *gpu_pshr.entry(pid).or_default() += v as u64;
                }
            }
            gpu_ded = p.array("gpu_ded").iter().map(|x| x.1 as u64).sum();
            gpu_shr = p.array("gpu_shr").iter().map(|x| x.1 as u64).sum();
            let idle = p.array("disk_idle");
            let rd = p.array("disk_read");
            let wr = p.array("disk_write");
            let q = p.array("disk_queue");
            let rs = p.array("disk_resp");
            for (inst, v) in &idle {
                if inst == "_Total" {
                    continue;
                }
                let find = |a: &Vec<(String, f64)>| a.iter().find(|x| &x.0 == inst).map(|x| x.1 as f32).unwrap_or(0.0);
                disks_raw.push((inst.clone(), (100.0 - *v as f32).clamp(0.0, 100.0), find(&rd), find(&wr), find(&q), find(&rs) * 1000.0));
            }
            for (inst, v) in p.array("thermal") {
                let c = v as f32 - 273.15;
                if c > -50.0 && c < 150.0 {
                    temps.push((inst, c));
                }
            }
            sysc.ctx_switches = p.single("ctx").unwrap_or(0.0) as f32;
            sysc.syscalls = p.single("syscalls").unwrap_or(0.0) as f32;
            sysc.proc_queue = p.single("pqueue").unwrap_or(0.0) as f32;
            sysc.exceptions = p.single("exceptions").unwrap_or(0.0) as f32;
            sysc.perf_pct = p.single("perf").unwrap_or(0.0) as f32;
            sysc.interrupts = p.single("interrupts").unwrap_or(0.0) as f32;
            meminfo.standby = (p.single("standby_c").unwrap_or(0.0) + p.single("standby_n").unwrap_or(0.0) + p.single("standby_r").unwrap_or(0.0)) as u64;
            meminfo.modified = p.single("modified").unwrap_or(0.0) as u64;
            meminfo.free_zero = p.single("freezero").unwrap_or(0.0) as u64;
            meminfo.pages_sec = p.single("pages").unwrap_or(0.0) as f32;
            meminfo.faults_sec = p.single("faults").unwrap_or(0.0) as f32;
            meminfo.pool_paged = p.single("pool_p").unwrap_or(0.0) as u64;
            meminfo.pool_nonpaged = p.single("pool_np").unwrap_or(0.0) as u64;
        }
        sysc.uptime = sys::uptime_secs();

        let interval_100ns = elapsed * 10_000_000.0;
        let total_cap = interval_100ns * ncpu as f64;

        // ---- merge (inside the lock)
        let mut s = shared.lock().unwrap();
        let now = util::now_unix();
        s.ncpu = ncpu;
        s.sys = sysc;
        if let Some(v) = vols {
            s.volumes = v;
        }
        // CPU totals and cores
        if !prev.cpu.is_empty() && prev.cpu.len() == cpu_times.len() {
            let mut tot_busy = 0.0;
            let mut tot_k = 0.0;
            let mut tot_dpc = 0.0;
            let mut tot_int = 0.0;
            let mut tot_all = 0.0;
            if s.cores.len() != ncpu {
                s.cores = (0..ncpu).map(|_| CoreInfo { hist: Ring::new(120), ..Default::default() }).collect();
            }
            for (i, (c, p)) in cpu_times.iter().zip(prev.cpu.iter()).enumerate() {
                let idle = (c.idle - p.idle) as f64;
                let kernel = (c.kernel - p.kernel) as f64; // includes idle
                let user = (c.user - p.user) as f64;
                let dpc = (c.dpc - p.dpc) as f64;
                let intr = (c.interrupt - p.interrupt) as f64;
                let all = (kernel + user).max(1.0);
                let busy = (all - idle).max(0.0);
                tot_busy += busy;
                tot_k += (kernel - idle).max(0.0);
                tot_dpc += dpc;
                tot_int += intr;
                tot_all += all;
                let core = &mut s.cores[i];
                core.usage = (busy / all * 100.0) as f32;
                core.kernel = ((kernel - idle).max(0.0) / all * 100.0) as f32;
                core.dpc = (dpc / all * 100.0) as f32;
                core.interrupt = (intr / all * 100.0) as f32;
                core.interrupts_sec = (c.interrupt_count.wrapping_sub(p.interrupt_count)) as f32 / elapsed as f32;
                if let Some(pw) = power.get(i) {
                    core.mhz = pw.current_mhz;
                    core.max_mhz = pw.max_mhz;
                }
                let u = core.usage;
                core.hist.push(u);
            }
            let tot_all = tot_all.max(1.0);
            s.cpu_total = (tot_busy / tot_all * 100.0) as f32;
            s.cpu_kernel = (tot_k / tot_all * 100.0) as f32;
            s.cpu_user = s.cpu_total - s.cpu_kernel;
            s.cpu_dpc = (tot_dpc / tot_all * 100.0) as f32;
            s.cpu_interrupt = (tot_int / tot_all * 100.0) as f32;
        }
        prev.cpu = cpu_times;

        // Processes
        let mut seen = HashSet::new();
        let mut new_procs = vec![];
        let have_prev = !prev.proc_times.is_empty();
        let mut next_times = HashMap::with_capacity(raw.len());
        let mut next_threads = HashMap::new();
        for r in &raw {
            seen.insert(r.pid);
            let key = (r.pid, r.create_time);
            let st = statics.get(&key).cloned().unwrap_or_default();
            let tot = r.user + r.kernel;
            let (cpu, cpu_k, cpu_u, cyc, rr, wr, orr, pf) = if let Some(p) = prev.proc_times.get(&key) {
                let du = (r.user - p.0).max(0) as f64;
                let dk = (r.kernel - p.1).max(0) as f64;
                (
                    ((du + dk) / total_cap * 100.0) as f32,
                    (dk / total_cap * 100.0) as f32,
                    (du / total_cap * 100.0) as f32,
                    r.cycle_time.saturating_sub(p.2),
                    ((r.read_bytes as u64).saturating_sub(p.3) as f64 / elapsed) as f32,
                    ((r.write_bytes as u64).saturating_sub(p.4) as f64 / elapsed) as f32,
                    ((r.other_bytes as u64).saturating_sub(p.5) as f64 / elapsed) as f32,
                    (r.page_faults.saturating_sub(p.6) as f64 / elapsed) as f32,
                )
            } else {
                (0.0, 0.0, 0.0, 0, 0.0, 0.0, 0.0, 0.0)
            };
            next_times.insert(key, (r.user, r.kernel, r.cycle_time, r.read_bytes as u64, r.write_bytes as u64, r.other_bytes as u64, r.page_faults));
            // threads
            let mut tl = Vec::with_capacity(r.threads.len());
            let mut all_suspended = !r.threads.is_empty() && r.pid > 4;
            for t in &r.threads {
                let tt = t.user + t.kernel;
                let (tcpu, dcs) = match prev.thread_times.get(&t.tid) {
                    Some(&(pt, pcs)) => (((tt - pt).max(0) as f64 / interval_100ns * 100.0) as f32, t.context_switches.wrapping_sub(pcs)),
                    None => (0.0, 0),
                };
                next_threads.insert(t.tid, (tt, t.context_switches));
                if !(t.state == 5 && t.wait_reason == 5) {
                    all_suspended = false;
                }
                tl.push(ThreadInfo { raw: t.clone(), cpu: tcpu, cswitch_delta: dcs });
            }
            let gp = gpu_proc.get(&r.pid).map(|m| m.values().cloned().fold(0.0, f64::max) as f32).unwrap_or(0.0);
            let entry_new = match s.procs.get(&r.pid) {
                Some(p) => p.create_time != r.create_time,
                None => true,
            };
            if entry_new {
                if let Some(old) = s.procs.remove(&r.pid) {
                    push_exit(&mut s, old, now);
                }
                let pi = ProcInfo {
                    pid: r.pid,
                    ppid: r.ppid,
                    name: r.name.clone(),
                    create_time: r.create_time,
                    cpu: 0.0,
                    cpu_kernel: 0.0,
                    cpu_user: 0.0,
                    cycles_delta: 0,
                    cpu_time: 0.0,
                    threads: 0,
                    threads_peak: 0,
                    handles: 0,
                    session: r.session,
                    base_priority: r.base_priority,
                    ws: 0,
                    private_ws: 0,
                    private_bytes: 0,
                    commit: 0,
                    peak_commit: 0,
                    peak_ws: 0,
                    virtual_size: 0,
                    peak_virtual: 0,
                    paged_pool: 0,
                    nonpaged_pool: 0,
                    page_faults: 0,
                    page_faults_sec: 0.0,
                    hard_faults: 0,
                    read_bytes: 0,
                    write_bytes: 0,
                    other_bytes: 0,
                    read_ops: 0,
                    write_ops: 0,
                    other_ops: 0,
                    io_read_rate: 0.0,
                    io_write_rate: 0.0,
                    io_other_rate: 0.0,
                    gpu: 0.0,
                    gpu_dedicated: 0,
                    gpu_shared: 0,
                    connections: 0,
                    suspended: false,
                    st: st.clone(),
                    hist_cpu: Ring::new(HIST_PROC),
                    hist_mem: Ring::new(HIST_PROC),
                    hist_io: Ring::new(HIST_PROC),
                    hist_handles: Ring::new(HIST_PROC),
                    hist_threads: Ring::new(HIST_PROC),
                    first_seen: now,
                    peak_cpu: 0.0,
                    thread_list: vec![],
                };
                s.procs.insert(r.pid, pi);
                if have_prev {
                    new_procs.push(r.pid);
                }
            }
            let p = s.procs.get_mut(&r.pid).unwrap();
            p.ppid = r.ppid;
            p.cpu = cpu.min(100.0);
            p.cpu_kernel = cpu_k;
            p.cpu_user = cpu_u;
            p.cycles_delta = cyc;
            p.cpu_time = tot as f64 / 1e7;
            p.threads = r.threads.len() as u32;
            p.threads_peak = r.threads_high_watermark;
            p.handles = r.handles;
            p.base_priority = r.base_priority;
            p.ws = r.ws as u64;
            p.private_ws = r.private_ws.max(0) as u64;
            p.private_bytes = r.private_bytes as u64;
            p.commit = r.commit as u64;
            p.peak_commit = r.peak_commit as u64;
            p.peak_ws = r.peak_ws as u64;
            p.virtual_size = r.virtual_size as u64;
            p.peak_virtual = r.peak_virtual as u64;
            p.paged_pool = r.paged_pool as u64;
            p.nonpaged_pool = r.nonpaged_pool as u64;
            p.page_faults = r.page_faults;
            p.page_faults_sec = pf;
            p.hard_faults = r.hard_faults;
            p.read_bytes = r.read_bytes as u64;
            p.write_bytes = r.write_bytes as u64;
            p.other_bytes = r.other_bytes as u64;
            p.read_ops = r.read_ops as u64;
            p.write_ops = r.write_ops as u64;
            p.other_ops = r.other_ops as u64;
            p.io_read_rate = rr;
            p.io_write_rate = wr;
            p.io_other_rate = orr;
            p.gpu = gp.min(100.0);
            p.gpu_dedicated = gpu_pded.get(&r.pid).copied().unwrap_or(0);
            p.gpu_shared = gpu_pshr.get(&r.pid).copied().unwrap_or(0);
            p.connections = conn_counts.get(&r.pid).copied().unwrap_or(0);
            p.suspended = all_suspended;
            p.peak_cpu = p.peak_cpu.max(p.cpu);
            p.hist_cpu.push(p.cpu);
            p.hist_mem.push(p.private_bytes as f32);
            p.hist_io.push(rr + wr);
            p.hist_handles.push(r.handles as f32);
            p.hist_threads.push(p.threads as f32);
            p.thread_list = tl;
        }
        prev.proc_times = next_times;
        prev.thread_times = next_threads;
        let gone: Vec<u32> = s.procs.keys().filter(|k| !seen.contains(k)).copied().collect();
        let mut exited_now = vec![];
        for pid in gone {
            if let Some(p) = s.procs.remove(&pid) {
                exited_now.push((p.pid, p.name.clone()));
                push_exit(&mut s, p, now);
            }
        }
        for &pid in &new_procs {
            if let Some(p) = s.procs.get(&pid) {
                let txt = format!("Started {} (PID {}) {}", p.name, p.pid, p.st.path);
                s.log("Process", Severity::Info, txt, Some(pid));
            }
        }

        // Memory
        meminfo.compressed = s.procs.values().find(|p| p.name == "Memory Compression").map(|p| p.ws).unwrap_or(0);
        s.mem = meminfo;

        // Disks
        let mut disks: Vec<DiskPerf> = vec![];
        for (name, act, rd, wr, q, resp) in disks_raw {
            let mut d = s.disks.iter().find(|d| d.name == name).cloned().unwrap_or(DiskPerf {
                name: name.clone(),
                active: 0.0,
                read: 0.0,
                write: 0.0,
                queue: 0.0,
                resp_ms: 0.0,
                hist_active: Ring::new(120),
                hist_read: Ring::new(120),
                hist_write: Ring::new(120),
            });
            d.active = act;
            d.read = rd;
            d.write = wr;
            d.queue = q;
            d.resp_ms = resp;
            d.hist_active.push(act);
            d.hist_read.push(rd);
            d.hist_write.push(wr);
            disks.push(d);
        }
        disks.sort_by(|a, b| a.name.cmp(&b.name));
        s.disks = disks;

        // Network
        let mut nets = vec![];
        for row in ifs {
            let (rx, tx) = match prev.net.get(&row.index) {
                Some(&(pi, po)) => (
                    (row.in_octets.saturating_sub(pi) as f64 / elapsed) as f32,
                    (row.out_octets.saturating_sub(po) as f64 / elapsed) as f32,
                ),
                None => (0.0, 0.0),
            };
            prev.net.insert(row.index, (row.in_octets, row.out_octets));
            let mut n = s.nets.iter().find(|n| n.row.index == row.index).cloned().unwrap_or(NetIf {
                row: row.clone(),
                rx: 0.0,
                tx: 0.0,
                hist_rx: Ring::new(120),
                hist_tx: Ring::new(120),
            });
            n.row = row;
            n.rx = rx;
            n.tx = tx;
            n.hist_rx.push(rx);
            n.hist_tx.push(tx);
            nets.push(n);
        }
        nets.sort_by(|a, b| b.row.up.cmp(&a.row.up).then(b.row.hardware.cmp(&a.row.hardware)).then(a.row.alias.cmp(&b.row.alias)));
        s.nets = nets;

        // GPU
        let mut engines: Vec<(String, f32)> = gpu_eng.into_iter().map(|(k, v)| (k, (v as f32).min(100.0))).collect();
        engines.sort_by(|a, b| a.0.cmp(&b.0));
        let total = engines.iter().map(|e| e.1).fold(0.0, f32::max);
        for (k, v) in &engines {
            s.gpu.engine_hist.entry(k.clone()).or_insert_with(|| Ring::new(120)).push(*v);
        }
        s.gpu.engines = engines;
        s.gpu.total = total;
        s.gpu.dedicated_used = gpu_ded;
        s.gpu.shared_used = gpu_shr;
        temps.sort_by(|a, b| a.0.cmp(&b.0));
        s.temps = temps;

        // History
        let (cpu, ck, memp) = (s.cpu_total, s.cpu_kernel, s.mem_used_pct());
        let commit = if s.mem.st.commit_limit > 0 { s.mem.st.commit_total as f32 / s.mem.st.commit_limit as f32 * 100.0 } else { 0.0 };
        let dact = s.disk_active_max();
        let drd: f32 = s.disks.iter().map(|d| d.read).sum();
        let dwr: f32 = s.disks.iter().map(|d| d.write).sum();
        let (rx, tx) = (s.net_rx(), s.net_tx());
        let g = s.gpu.total;
        let mt = s.max_temp();
        let np = s.procs.len() as f32;
        s.hist.cpu.push(cpu);
        s.hist.cpu_kernel.push(ck);
        s.hist.mem.push(memp);
        s.hist.commit.push(commit);
        s.hist.disk.push(dact);
        s.hist.disk_read.push(drd);
        s.hist.disk_write.push(dwr);
        s.hist.net_rx.push(rx);
        s.hist.net_tx.push(tx);
        s.hist.gpu.push(g);
        s.hist.temp.push(mt);
        s.hist.procs.push(np);
        if s.hist.times.len() >= HIST_LONG {
            s.hist.times.pop_front();
        }
        s.hist.times.push_back(now);
        s.samples += 1;
        s.last_sample = now;

        // Spike detection into the timeline
        if s.samples > 30 {
            let avg = s.hist.cpu.avg(60);
            if cpu > 80.0 && cpu > avg + 30.0 {
                let top = top_proc(&s, |p| p.cpu);
                s.log("Spike", Severity::Warning, format!("CPU spike to {cpu:.0}% (average {avg:.0}%), top: {top}"), None);
            }
            let davg = s.hist.disk.avg(60);
            if dact > 90.0 && davg < 60.0 {
                let top = top_proc(&s, |p| p.io_read_rate + p.io_write_rate);
                s.log("Spike", Severity::Warning, format!("Disk busy {dact:.0}%, top: {top}"), None);
            }
        }

        // Recording
        if s.recorder.is_some() {
            let top_cpu = s.procs.values().filter(|p| p.pid != 0).max_by(|a, b| a.cpu.total_cmp(&b.cpu)).map(|p| (p.name.clone(), p.cpu)).unwrap_or_default();
            let top_mem = s.procs.values().max_by_key(|p| p.private_bytes).map(|p| (p.name.clone(), p.private_bytes)).unwrap_or_default();
            let line = format!(
                "{},{:.1},{:.1},{:.1},{},{},{:.1},{:.0},{:.0},{:.0},{:.0},{:.1},{:.1},{},{},{},{},{:.1},{},{}",
                util::fmt_time(now),
                cpu,
                ck,
                memp,
                s.mem.st.total - s.mem.st.avail,
                s.mem.st.commit_total,
                dact,
                drd,
                dwr,
                rx,
                tx,
                g,
                mt,
                s.procs.len(),
                s.mem.st.threads,
                s.mem.st.handles,
                top_cpu.0.replace(',', " "),
                top_cpu.1,
                top_mem.0.replace(',', " "),
                top_mem.1
            );
            let mut stop = false;
            if let Some(r) = s.recorder.as_mut() {
                let _ = writeln!(r.file, "{line}");
                r.rows += 1;
                if let Some(u) = r.until {
                    if Instant::now() >= u {
                        stop = true;
                    }
                }
            }
            if stop {
                s.stop_recording();
            }
        }

        // Automation
        run_rules(&mut s, &new_procs, &exited_now, &mut rule_since, &mut rule_last);
        apply_proc_profiles(&mut s, &mut profiled);
        if last_schedule_check.elapsed() > Duration::from_secs(20) {
            last_schedule_check = Instant::now();
            run_schedules(&mut s);
        }
        if s.config_dirty {
            s.config_dirty = false;
            s.config.save();
        }
        s.sample_cost_ms = t0.elapsed().as_secs_f32() * 1000.0;
        drop(s);
        ctx.request_repaint();
    }
}

fn top_proc(s: &State, f: impl Fn(&ProcInfo) -> f32) -> String {
    s.procs
        .values()
        .filter(|p| p.pid != 0)
        .max_by(|a, b| f(a).total_cmp(&f(b)))
        .map(|p| format!("{} (PID {})", p.name, p.pid))
        .unwrap_or_default()
}

fn push_exit(s: &mut State, p: ProcInfo, now: i64) {
    let txt = format!("Exited {} (PID {}) after {}", p.name, p.pid, util::fmt_dur((now - util::filetime_to_unix(p.create_time)).max(0) as u64));
    s.log("Process", Severity::Info, txt, Some(p.pid));
    if s.exited.len() >= 1000 {
        s.exited.pop_front();
    }
    s.exited.push_back(ExitedProc {
        pid: p.pid,
        name: p.name,
        path: p.st.path.clone(),
        started: util::filetime_to_unix(p.create_time),
        exited: now,
        peak_cpu: p.peak_cpu,
        peak_private: p.hist_mem.data.iter().fold(0.0f32, |a, &b| a.max(b)) as u64,
        cpu_time: p.cpu_time,
        user: p.st.user.clone(),
    });
}

fn metric_system(s: &State, m: Metric) -> f64 {
    match m {
        Metric::SystemCpu => s.cpu_total as f64,
        Metric::SystemMemory => s.mem_used_pct() as f64,
        Metric::CommitCharge => {
            if s.mem.st.commit_limit > 0 {
                s.mem.st.commit_total as f64 / s.mem.st.commit_limit as f64 * 100.0
            } else {
                0.0
            }
        }
        Metric::DiskActive => s.disk_active_max() as f64,
        Metric::GpuUsage => s.gpu.total as f64,
        Metric::NetReceiveMBps => s.net_rx() as f64 / 1048576.0,
        Metric::NetSendMBps => s.net_tx() as f64 / 1048576.0,
        Metric::Temperature => s.max_temp() as f64,
        _ => 0.0,
    }
}

fn metric_proc(p: &ProcInfo, m: Metric) -> f64 {
    match m {
        Metric::ProcessCpu => p.cpu as f64,
        Metric::ProcessMemoryMB => p.private_bytes as f64 / 1048576.0,
        Metric::ProcessHandles => p.handles as f64,
        Metric::ProcessThreads => p.threads as f64,
        Metric::ProcessIoMBps => (p.io_read_rate + p.io_write_rate) as f64 / 1048576.0,
        _ => 0.0,
    }
}

fn matches_target(target: &str, name: &str) -> bool {
    let t = target.trim();
    t.is_empty() || t == "*" || util::wildcard_match(t, name)
}

fn run_rules(
    s: &mut State,
    started: &[u32],
    exited: &[(u32, String)],
    since: &mut HashMap<(u64, u32), Instant>,
    last: &mut HashMap<(u64, u32), Instant>,
) {
    let rules = s.config.rules.clone();
    let mut fires: Vec<(usize, u32, String, f64)> = vec![];
    for (ri, r) in rules.iter().enumerate() {
        if !r.enabled {
            continue;
        }
        let cond = |v: f64| match r.op {
            Op::Above => v > r.threshold,
            Op::Below => v < r.threshold,
        };
        if r.metric.is_event() {
            let list: Vec<(u32, String)> = if r.metric == Metric::ProcessStarted {
                started.iter().filter_map(|pid| s.procs.get(pid).map(|p| (*pid, p.name.clone()))).collect()
            } else {
                exited.to_vec()
            };
            for (pid, name) in list {
                if matches_target(&r.target, &name) {
                    fires.push((ri, pid, name, 1.0));
                }
            }
            continue;
        }
        if r.metric.is_process() {
            let mut active = HashSet::new();
            for p in s.procs.values() {
                if p.pid <= 4 || !matches_target(&r.target, &p.name) {
                    continue;
                }
                let v = metric_proc(p, r.metric);
                let key = (r.id, p.pid);
                if cond(v) {
                    active.insert(p.pid);
                    let t = *since.entry(key).or_insert_with(Instant::now);
                    if t.elapsed().as_secs() >= r.duration_s as u64 {
                        fires.push((ri, p.pid, p.name.clone(), v));
                    }
                }
            }
            since.retain(|k, _| k.0 != r.id || active.contains(&k.1));
        } else {
            let v = metric_system(s, r.metric);
            let key = (r.id, 0);
            if cond(v) {
                let t = *since.entry(key).or_insert_with(Instant::now);
                if t.elapsed().as_secs() >= r.duration_s as u64 {
                    fires.push((ri, 0, String::new(), v));
                }
            } else {
                since.remove(&key);
            }
        }
    }
    for (ri, pid, name, v) in fires {
        let r = &rules[ri];
        let key = (r.id, pid);
        if let Some(t) = last.get(&key) {
            if t.elapsed().as_secs() < r.cooldown_s as u64 {
                continue;
            }
        }
        last.insert(key, Instant::now());
        if let Some(rr) = s.config.rules.get_mut(ri) {
            rr.fired += 1;
        }
        let who = if pid > 0 { format!("{name} (PID {pid})") } else { "System".into() };
        let detail = format!("{} = {:.1} ({} {:.1}) — {}", r.metric.label(), v, if r.op == Op::Above { ">" } else { "<" }, r.threshold, who);
        let result = execute_action(s, &r.action, pid, &name, &r.name, v);
        let sev = if matches!(r.action, Action::Alert) { Severity::Warning } else { Severity::Info };
        s.alert(sev, r.name.clone(), format!("{detail}. {result}"));
    }
}

pub fn execute_action(s: &mut State, a: &Action, pid: u32, name: &str, rule: &str, value: f64) -> String {
    match a {
        Action::Alert => String::new(),
        Action::KillProcess => {
            if pid == 0 {
                return "No process to end".into();
            }
            match sys::terminate(pid) {
                Ok(()) => format!("Ended {name}"),
                Err(e) => format!("Could not end {name}: {e}"),
            }
        }
        Action::SuspendProcess => match sys::suspend_process(pid, true) {
            Ok(()) => format!("Suspended {name}"),
            Err(e) => format!("Could not suspend: {e}"),
        },
        Action::SetPriority(p) => match sys::set_priority(pid, p.class()) {
            Ok(()) => format!("Priority set to {}", p.label()),
            Err(e) => format!("Could not set priority: {e}"),
        },
        Action::EfficiencyMode => match sys::set_efficiency(pid, true) {
            Ok(()) => "Efficiency mode on".into(),
            Err(e) => format!("Could not set efficiency mode: {e}"),
        },
        Action::RunScript(id_or_body) => {
            let body = s
                .config
                .scripts
                .iter()
                .find(|sc| sc.name == *id_or_body)
                .map(|sc| (sc.shell.clone(), sc.body.clone()))
                .unwrap_or(("PowerShell".into(), id_or_body.clone()));
            spawn_script(&body.0, &body.1, &[("DTM_RULE", rule), ("DTM_PID", &pid.to_string()), ("DTM_NAME", name), ("DTM_VALUE", &format!("{value:.2}"))]);
            "Script started".into()
        }
        Action::StartRecording(secs) => match s.start_recording(Some(*secs)) {
            Ok(p) => format!("Recording to {}", p.display()),
            Err(e) => e,
        },
        Action::ApplySystemProfile(n) => {
            s.pending_actions.push(format!("profile:{n}"));
            format!("Profile '{n}' queued")
        }
    }
}

pub fn spawn_script(shell: &str, body: &str, env: &[(&str, &str)]) -> std::process::Child {
    let mut cmd = if shell.eq_ignore_ascii_case("cmd") {
        let mut c = std::process::Command::new("cmd.exe");
        c.args(["/C", body]);
        c
    } else {
        let mut c = std::process::Command::new("powershell.exe");
        c.args(["-NoProfile", "-ExecutionPolicy", "Bypass", "-Command", body]);
        c
    };
    for (k, v) in env {
        cmd.env(k, v);
    }
    cmd.stdout(std::process::Stdio::piped()).stderr(std::process::Stdio::piped());
    {
        use std::os::windows::process::CommandExt;
        cmd.creation_flags(0x0800_0000);
    }
    cmd.spawn().unwrap_or_else(|_| std::process::Command::new("cmd.exe").args(["/C", "exit"]).spawn().expect("cmd"))
}

fn apply_proc_profiles(s: &mut State, done: &mut HashSet<(u64, u32, i64)>) {
    if s.config.proc_profiles.is_empty() {
        return;
    }
    let profiles = s.config.proc_profiles.clone();
    let mut logs = vec![];
    for pr in profiles.iter().filter(|p| p.enabled) {
        for p in s.procs.values() {
            if p.pid <= 4 || !util::wildcard_match(&pr.pattern, &p.name) {
                continue;
            }
            let key = (pr.id, p.pid, p.create_time);
            if done.contains(&key) {
                continue;
            }
            done.insert(key);
            let mut res = vec![];
            if let Some(pri) = pr.priority {
                res.push(match sys::set_priority(p.pid, pri.class()) {
                    Ok(()) => format!("priority {}", pri.label()),
                    Err(e) => format!("priority failed ({e})"),
                });
            }
            if let Some(mask) = pr.affinity {
                res.push(match sys::set_affinity(p.pid, mask) {
                    Ok(()) => format!("CPU mask 0x{mask:x}"),
                    Err(e) => format!("affinity failed ({e})"),
                });
            }
            if let Some(eff) = pr.efficiency {
                res.push(match sys::set_efficiency(p.pid, eff) {
                    Ok(()) => format!("efficiency {}", if eff { "on" } else { "off" }),
                    Err(e) => format!("efficiency failed ({e})"),
                });
            }
            logs.push((pr.id, format!("Profile '{}' applied to {} (PID {}): {}", pr.pattern, p.name, p.pid, res.join(", ")), p.pid));
        }
    }
    let live: HashSet<u32> = s.procs.keys().copied().collect();
    done.retain(|k| live.contains(&k.1));
    for (id, l, pid) in logs {
        if let Some(pp) = s.config.proc_profiles.iter_mut().find(|p| p.id == id) {
            pp.applied += 1;
        }
        s.log("Automation", Severity::Info, l, Some(pid));
    }
}

fn run_schedules(s: &mut State) {
    use chrono::Timelike;
    let now = util::now_unix();
    let local = chrono::Local::now();
    let mut due = vec![];
    for (i, sc) in s.config.schedules.iter().enumerate() {
        if !sc.enabled {
            continue;
        }
        let is_due = match sc.kind {
            ScheduleKind::EveryMinutes(m) => now - sc.last_run >= (m.max(1) as i64) * 60,
            ScheduleKind::DailyAt(h, m) => {
                local.hour() == h && local.minute() >= m && local.minute() < m + 2 && now - sc.last_run > 3600
            }
        };
        if is_due {
            due.push(i);
        }
    }
    for i in due {
        let sc = s.config.schedules[i].clone();
        s.config.schedules[i].last_run = now;
        s.config_dirty = true;
        if sc.record_secs > 0 {
            let _ = s.start_recording(Some(sc.record_secs));
        }
        if sc.take_snapshot {
            s.snapshot_requests += 1;
        }
        if sc.write_report {
            s.report_requests += 1;
        }
        s.log("Schedule", Severity::Info, format!("Scheduled monitoring '{}' ran", sc.name), None);
    }
}
