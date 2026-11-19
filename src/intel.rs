//! Intelligence: bottleneck detection, process behaviour, leak candidates,
//! correlations, health score, recommendations and the health report.

use crate::collector::{Severity, State};
use crate::util::{self, fmt_bytes, regression, std_dev};

#[derive(Clone, Debug)]
pub struct Finding {
    pub severity: Severity,
    pub area: &'static str,
    pub title: String,
    pub detail: String,
    pub advice: String,
}

fn f(sev: Severity, area: &'static str, title: impl Into<String>, detail: impl Into<String>, advice: impl Into<String>) -> Finding {
    Finding { severity: sev, area, title: title.into(), detail: detail.into(), advice: advice.into() }
}

fn top_by(s: &State, n: usize, key: impl Fn(&crate::collector::ProcInfo) -> f32) -> Vec<(String, u32, f32)> {
    let mut v: Vec<_> = s.procs.values().filter(|p| p.pid > 0).map(|p| (p.name.clone(), p.pid, key(p))).collect();
    v.sort_by(|a, b| b.2.total_cmp(&a.2));
    v.truncate(n);
    v
}

pub fn bottlenecks(s: &State) -> Vec<Finding> {
    let mut out = vec![];
    let window = 30.min(s.hist.cpu.len().max(1));
    let cpu = s.hist.cpu.avg(window);
    if s.hist.cpu.len() < 5 {
        return out;
    }
    if cpu > 85.0 {
        let top = top_by(s, 3, |p| p.cpu);
        out.push(f(
            Severity::Critical,
            "CPU",
            format!("Processor is the bottleneck ({cpu:.0}% over {window}s)"),
            format!("Busiest: {}", top.iter().map(|t| format!("{} {:.0}%", t.0, t.2)).collect::<Vec<_>>().join(", ")),
            "Close or lower the priority of the busiest apps, or check them in Behaviour for runaway loops.",
        ));
    } else if cpu > 65.0 {
        out.push(f(Severity::Warning, "CPU", format!("Processor is busy ({cpu:.0}% average)"), "Sustained load leaves little headroom.", "Watch for slowdowns when starting new work."));
    }
    let q = s.sys.proc_queue;
    if q > (s.ncpu as f32 * 2.0) {
        out.push(f(Severity::Warning, "CPU", format!("{q:.0} threads waiting for a processor"), "The ready queue is longer than twice the number of logical processors.", "Too many threads compete for CPU time."));
    }
    if s.cpu_dpc + s.cpu_interrupt > 15.0 {
        out.push(f(
            Severity::Warning,
            "Drivers",
            format!("High driver time ({:.1}% interrupts + DPCs)", s.cpu_dpc + s.cpu_interrupt),
            "Hardware drivers are taking a large share of CPU time.",
            "Update network, storage, audio and graphics drivers; this often causes stutter.",
        ));
    }
    let mem = s.mem_used_pct();
    if mem > 90.0 {
        let top = top_by(s, 3, |p| p.private_bytes as f32);
        out.push(f(
            Severity::Critical,
            "Memory",
            format!("Memory nearly full ({mem:.0}% used)"),
            format!("Largest: {}", top.iter().map(|t| format!("{} {}", t.0, fmt_bytes(t.2 as f64))).collect::<Vec<_>>().join(", ")),
            "Close large apps or add more RAM; Windows is paging to disk.",
        ));
    } else if mem > 80.0 {
        out.push(f(Severity::Warning, "Memory", format!("Memory getting full ({mem:.0}% used)"), "", "Keep an eye on large apps."));
    }
    if s.mem.st.commit_limit > 0 {
        let c = s.mem.st.commit_total as f64 / s.mem.st.commit_limit as f64 * 100.0;
        if c > 90.0 {
            out.push(f(Severity::Critical, "Memory", format!("Commit charge at {c:.0}% of its limit"), "Programs may fail to allocate memory.", "Close apps or enlarge the page file."));
        }
    }
    if s.mem.pages_sec > 1500.0 {
        out.push(f(
            Severity::Warning,
            "Memory",
            format!("Heavy paging ({:.0} pages/s)", s.mem.pages_sec),
            "Memory pages are read from or written to disk a lot.",
            "This makes everything slow; more free RAM helps.",
        ));
    }
    for d in &s.disks {
        let a = d.hist_active.avg(20);
        if a > 90.0 {
            let top = top_by(s, 3, |p| p.io_read_rate + p.io_write_rate);
            out.push(f(
                Severity::Critical,
                "Disk",
                format!("Disk {} is saturated ({a:.0}% active)", d.name),
                format!("Queue {:.1}, response {:.1} ms. Busiest: {}", d.queue, d.resp_ms, top.iter().map(|t| format!("{} {}", t.0, util::fmt_rate(t.2 as f64))).collect::<Vec<_>>().join(", ")),
                "Pause backups, indexing or updates, or move heavy work to a faster drive.",
            ));
        } else if d.resp_ms > 50.0 && a > 30.0 {
            out.push(f(Severity::Warning, "Disk", format!("Disk {} responds slowly ({:.0} ms)", d.name, d.resp_ms), "", "A slow or failing drive; check Disk health."));
        }
    }
    for v in &s.volumes {
        if v.total > 0 && v.kind == "Fixed" {
            let free = v.free as f64 / v.total as f64 * 100.0;
            if free < 10.0 {
                out.push(f(Severity::Warning, "Disk", format!("Drive {} almost full ({free:.0}% free)", v.root), format!("{} free of {}", fmt_bytes(v.free as f64), fmt_bytes(v.total as f64)), "Free some space: Windows needs room for updates and the page file."));
            }
        }
    }
    for n in &s.nets {
        if n.row.speed > 0 {
            let util_pct = (n.rx.max(n.tx) as f64 * 8.0) / n.row.speed as f64 * 100.0;
            if util_pct > 80.0 {
                out.push(f(Severity::Warning, "Network", format!("{} link is {util_pct:.0}% used", n.row.alias), "", "A download or upload is using nearly all bandwidth."));
            }
        }
        if n.row.in_errors + n.row.out_errors > 1000 {
            out.push(f(Severity::Info, "Network", format!("{} has {} errors", n.row.alias, n.row.in_errors + n.row.out_errors), "Packet errors since the adapter started.", "Check the cable, Wi-Fi signal or adapter driver."));
        }
    }
    if s.hist.gpu.avg(window) > 95.0 {
        let top = top_by(s, 2, |p| p.gpu);
        out.push(f(Severity::Info, "GPU", "Graphics processor is fully used", format!("Busiest: {}", top.iter().map(|t| format!("{} {:.0}%", t.0, t.2)).collect::<Vec<_>>().join(", ")), "Normal while gaming; otherwise check the app."));
    }
    let t = s.max_temp();
    if t > 90.0 {
        out.push(f(Severity::Critical, "Thermal", format!("Very hot: {t:.0} °C"), "The processor may slow itself down to cool off.", "Clean the fans and vents; check cooling."));
    } else if t > 80.0 {
        out.push(f(Severity::Warning, "Thermal", format!("Running warm: {t:.0} °C"), "", "Check airflow."));
    }
    if s.sys.perf_pct > 0.0 && s.sys.perf_pct < 60.0 && cpu > 50.0 {
        out.push(f(
            Severity::Warning,
            "CPU",
            format!("Processor running at {:.0}% of its base speed while busy", s.sys.perf_pct),
            "It may be throttled by heat or a power-saving plan.",
            "Switch to a balanced or high-performance power plan.",
        ));
    }
    out
}

#[derive(Clone, Debug)]
pub struct Behaviour {
    pub pid: u32,
    pub name: String,
    pub traits: Vec<&'static str>,
    pub cpu_avg: f32,
    pub cpu_sd: f32,
    pub mem_trend_mb_min: f64,
    pub io_avg: f32,
    pub handle_trend: f64,
    pub thread_trend: f64,
    pub summary: String,
}

pub fn behaviour(s: &State) -> Vec<Behaviour> {
    let per_min = 60_000.0 / s.config.sample_ms.max(250) as f64;
    let mut out = vec![];
    for p in s.procs.values() {
        if p.pid <= 4 || p.hist_cpu.len() < 10 {
            continue;
        }
        let cpu = p.hist_cpu.to_vec(120);
        let mem = p.hist_mem.to_vec(300);
        let io = p.hist_io.to_vec(120);
        let cpu_avg = cpu.iter().sum::<f32>() / cpu.len() as f32;
        let cpu_sd = std_dev(&cpu);
        let enough = mem.len() >= 30;
        let (ms, mr2) = if enough { regression(&mem) } else { (0.0, 0.0) };
        let mem_trend = ms * per_min / 1048576.0;
        let io_avg = io.iter().sum::<f32>() / io.len().max(1) as f32;
        let (hs, _) = if enough { regression(&p.hist_handles.to_vec(300)) } else { (0.0, 0.0) };
        let (ts, _) = if enough { regression(&p.hist_threads.to_vec(300)) } else { (0.0, 0.0) };
        let mut traits = vec![];
        if cpu_avg > 20.0 {
            traits.push("CPU-bound");
        }
        if cpu_avg > 2.0 && cpu_sd > cpu_avg * 1.5 {
            traits.push("Bursty");
        }
        if io_avg > 5.0 * 1048576.0 {
            traits.push("Disk-heavy");
        }
        if mem_trend > 2.0 && mr2 > 0.7 {
            traits.push("Memory growing");
        }
        if hs * per_min > 20.0 {
            traits.push("Handles growing");
        }
        if ts * per_min > 2.0 {
            traits.push("Threads growing");
        }
        if p.gpu > 10.0 {
            traits.push("GPU user");
        }
        if p.connections > 20 {
            traits.push("Network-chatty");
        }
        if p.suspended {
            traits.push("Suspended");
        }
        if cpu_avg < 0.1 && io_avg < 1024.0 {
            traits.push("Idle");
        }
        if p.page_faults_sec > 5000.0 {
            traits.push("Page-fault heavy");
        }
        if traits.is_empty() {
            traits.push("Steady");
        }
        let summary = format!(
            "avg CPU {cpu_avg:.1}% (±{cpu_sd:.1}), memory {}{:.1} MB/min, disk {}",
            if mem_trend >= 0.0 { "+" } else { "" },
            mem_trend,
            util::fmt_rate(io_avg as f64)
        );
        out.push(Behaviour {
            pid: p.pid,
            name: p.name.clone(),
            traits,
            cpu_avg,
            cpu_sd,
            mem_trend_mb_min: mem_trend,
            io_avg,
            handle_trend: hs * per_min,
            thread_trend: ts * per_min,
            summary,
        });
    }
    out.sort_by(|a, b| b.cpu_avg.total_cmp(&a.cpu_avg));
    out
}

#[derive(Clone, Debug)]
pub struct LeakCandidate {
    pub pid: u32,
    pub name: String,
    pub kind: &'static str,
    pub growth_per_min: f64,
    pub r2: f64,
    pub grown: f64,
    pub now: f64,
    pub samples: usize,
    pub score: f64,
}

pub fn leak_candidates(s: &State) -> Vec<LeakCandidate> {
    let per_min = 60_000.0 / s.config.sample_ms.max(250) as f64;
    let mut out = vec![];
    for p in s.procs.values() {
        if p.pid <= 4 || p.hist_mem.len() < 30 {
            continue;
        }
        let mem = p.hist_mem.to_vec(300);
        let (slope, r2) = regression(&mem);
        let grown = (*mem.last().unwrap() - mem[0]) as f64;
        let rises = mem.windows(2).filter(|w| w[1] >= w[0]).count() as f64 / (mem.len() - 1) as f64;
        if slope > 0.0 && r2 > 0.6 && grown > 5.0 * 1048576.0 && rises > 0.6 {
            out.push(LeakCandidate {
                pid: p.pid,
                name: p.name.clone(),
                kind: "Private memory",
                growth_per_min: slope * per_min,
                r2,
                grown,
                now: p.private_bytes as f64,
                samples: mem.len(),
                score: r2 * rises * (grown / 1048576.0).ln().max(0.0),
            });
        }
        let h = p.hist_handles.to_vec(300);
        let (hs, hr2) = regression(&h);
        let hg = (*h.last().unwrap() - h[0]) as f64;
        if hs > 0.0 && hr2 > 0.7 && hg > 200.0 {
            out.push(LeakCandidate {
                pid: p.pid,
                name: p.name.clone(),
                kind: "Handles",
                growth_per_min: hs * per_min,
                r2: hr2,
                grown: hg,
                now: p.handles as f64,
                samples: h.len(),
                score: hr2 * (hg / 50.0).ln().max(0.0),
            });
        }
        let gdi_like = p.hist_threads.to_vec(300);
        let (ts, tr2) = regression(&gdi_like);
        let tg = (*gdi_like.last().unwrap() - gdi_like[0]) as f64;
        if ts > 0.0 && tr2 > 0.7 && tg > 30.0 {
            out.push(LeakCandidate {
                pid: p.pid,
                name: p.name.clone(),
                kind: "Threads",
                growth_per_min: ts * per_min,
                r2: tr2,
                grown: tg,
                now: p.threads as f64,
                samples: gdi_like.len(),
                score: tr2 * (tg / 10.0).ln().max(0.0),
            });
        }
    }
    out.sort_by(|a, b| b.score.total_cmp(&a.score));
    out
}

#[derive(Clone, Debug)]
pub struct Correlation {
    pub time: i64,
    pub metric: &'static str,
    pub value: f32,
    pub related: Vec<String>,
}

pub fn correlations(s: &State) -> Vec<Correlation> {
    let mut out = vec![];
    let n = s.hist.times.len();
    if n < 20 {
        return out;
    }
    let series: [(&'static str, &crate::util::Ring); 4] =
        [("CPU", &s.hist.cpu), ("Memory", &s.hist.mem), ("Disk", &s.hist.disk), ("GPU", &s.hist.gpu)];
    for (name, ring) in series {
        let v: Vec<f32> = ring.data.iter().copied().collect();
        if v.len() != n {
            continue;
        }
        let mean = v.iter().sum::<f32>() / n as f32;
        let sd = std_dev(&v);
        let thr = (mean + 2.0 * sd).max(if name == "Memory" { mean + 5.0 } else { 50.0 });
        let mut i = 1;
        while i < n {
            if v[i] > thr && v[i - 1] <= thr {
                let t = s.hist.times[i];
                let related: Vec<String> = s
                    .timeline
                    .iter()
                    .filter(|e| (e.time - t).abs() <= 5 && e.category != "Spike" && e.category != "Alert")
                    .map(|e| format!("{} {}", util::fmt_clock(e.time), e.text))
                    .take(6)
                    .collect();
                out.push(Correlation { time: t, metric: name, value: v[i], related });
                i += 5;
            } else {
                i += 1;
            }
        }
    }
    out.sort_by(|a, b| b.time.cmp(&a.time));
    out
}

pub struct Health {
    pub score: u32,
    pub parts: Vec<(&'static str, u32, String)>,
}

pub fn health(s: &State) -> Health {
    let mut parts = vec![];
    let cpu = s.hist.cpu.avg(60);
    let c = (100.0 - (cpu - 50.0).max(0.0) * 2.0).clamp(0.0, 100.0) as u32;
    parts.push(("CPU load", c, format!("{cpu:.0}% average over the last minute")));
    let mem = s.mem_used_pct();
    let m = (100.0 - (mem - 60.0).max(0.0) * 2.5).clamp(0.0, 100.0) as u32;
    parts.push(("Memory", m, format!("{mem:.0}% in use")));
    let disk = s.hist.disk.avg(60);
    let d = (100.0 - (disk - 40.0).max(0.0) * 1.6).clamp(0.0, 100.0) as u32;
    parts.push(("Disk activity", d, format!("{disk:.0}% busiest disk active")));
    let min_free = s
        .volumes
        .iter()
        .filter(|v| v.total > 0 && v.kind == "Fixed")
        .map(|v| v.free as f64 / v.total as f64 * 100.0)
        .fold(100.0, f64::min);
    let sp = (min_free * 5.0).clamp(0.0, 100.0) as u32;
    parts.push(("Free space", sp, format!("{min_free:.0}% free on the fullest drive")));
    let t = s.max_temp();
    let th = if t <= 0.0 { 100 } else { (100.0 - (t - 70.0).max(0.0) * 4.0).clamp(0.0, 100.0) as u32 };
    parts.push(("Temperature", th, if t > 0.0 { format!("{t:.0} °C hottest sensor") } else { "No sensor reading".into() }));
    let unacked = s.unacked();
    let al = (100i64 - unacked as i64 * 10).clamp(0, 100) as u32;
    parts.push(("Alerts", al, format!("{unacked} unread alerts")));
    let up_days = s.sys.uptime as f64 / 86400.0;
    let up = (100.0 - (up_days - 7.0).max(0.0) * 4.0).clamp(40.0, 100.0) as u32;
    parts.push(("Uptime", up, format!("{:.1} days since restart", up_days)));
    let w = [0.25, 0.2, 0.15, 0.15, 0.1, 0.1, 0.05];
    let score = parts.iter().zip(w.iter()).map(|(p, w)| p.1 as f64 * w).sum::<f64>().round() as u32;
    Health { score, parts }
}

pub fn recommendations(s: &State, startup_count: Option<usize>, power_plan: Option<&str>) -> Vec<Finding> {
    let mut out = bottlenecks(s);
    let up_days = s.sys.uptime / 86400;
    if up_days >= 14 {
        out.push(f(Severity::Info, "System", format!("Not restarted for {up_days} days"), "Long uptimes collect leaked memory and pending updates.", "Restart when convenient."));
    }
    if let Some(n) = startup_count {
        if n > 15 {
            out.push(f(Severity::Info, "Startup", format!("{n} programs start with Windows"), "Each adds to sign-in time and background load.", "Open Startup and turn off what you don't need right away."));
        }
    }
    if let Some(p) = power_plan {
        if p.to_lowercase().contains("saver") {
            out.push(f(Severity::Info, "Power", "Power saver plan is active", "The processor is held back to save energy.", "Use Balanced when plugged in."));
        }
    }
    for l in leak_candidates(s).iter().take(3) {
        out.push(f(
            Severity::Warning,
            "Leaks",
            format!("{} keeps growing ({})", l.name, l.kind),
            format!("{:+.1} per minute, fit {:.0}%", if l.kind == "Private memory" { l.growth_per_min / 1048576.0 } else { l.growth_per_min }, l.r2 * 100.0),
            "Restart this app if it slows down; report it to its maker if it repeats.",
        ));
    }
    let suspended: Vec<_> = s.procs.values().filter(|p| p.suspended).map(|p| p.name.clone()).collect();
    if !suspended.is_empty() {
        out.push(f(Severity::Info, "Processes", format!("{} suspended processes", suspended.len()), suspended.iter().take(6).cloned().collect::<Vec<_>>().join(", "), "Usually Store apps paused in the background; resume one if it seems frozen."));
    }
    let not_resp_handles = s.procs.values().filter(|p| p.handles > 50_000).map(|p| format!("{} ({})", p.name, p.handles)).collect::<Vec<_>>();
    if !not_resp_handles.is_empty() {
        out.push(f(Severity::Warning, "Processes", "Processes with very many handles", not_resp_handles.join(", "), "Above 50,000 handles often means a leak."));
    }
    if out.is_empty() {
        out.push(f(Severity::Info, "System", "Everything looks healthy", "No bottlenecks or warning signs right now.", "Nothing to do."));
    }
    out
}

pub fn report_html(s: &State, recs: &[Finding]) -> String {
    let h = health(s);
    let esc = |t: &str| t.replace('&', "&amp;").replace('<', "&lt;").replace('>', "&gt;");
    let mut o = String::new();
    o.push_str("<!doctype html><html><head><meta charset='utf-8'><title>System health report</title><style>body{font-family:Segoe UI,Arial;background:#0f1419;color:#d8dee9;margin:32px}h1{color:#5ad1e6}h2{color:#a3be8c;border-bottom:1px solid #2b3440;padding-bottom:4px}table{border-collapse:collapse;width:100%;margin:8px 0 20px}td,th{border-bottom:1px solid #2b3440;padding:6px 8px;text-align:left}th{color:#88c0d0}.c{color:#bf616a}.w{color:#ebcb8b}.i{color:#81a1c1}.score{font-size:48px;font-weight:600}</style></head><body>");
    o.push_str(&format!("<h1>System health report</h1><p>{} — {}</p>", util::fmt_time(util::now_unix()), esc(&s.cpu_name)));
    o.push_str(&format!("<div class='score'>{}/100</div><table><tr><th>Area</th><th>Score</th><th>Detail</th></tr>", h.score));
    for (n, sc, d) in &h.parts {
        o.push_str(&format!("<tr><td>{n}</td><td>{sc}</td><td>{}</td></tr>", esc(d)));
    }
    o.push_str("</table><h2>Findings and recommendations</h2><table><tr><th>Severity</th><th>Area</th><th>Finding</th><th>Detail</th><th>What to do</th></tr>");
    for r in recs {
        let (cls, name) = match r.severity {
            Severity::Critical => ("c", "Critical"),
            Severity::Warning => ("w", "Warning"),
            Severity::Info => ("i", "Info"),
        };
        o.push_str(&format!("<tr><td class='{cls}'>{name}</td><td>{}</td><td>{}</td><td>{}</td><td>{}</td></tr>", r.area, esc(&r.title), esc(&r.detail), esc(&r.advice)));
    }
    o.push_str("</table><h2>Resources now</h2><table>");
    let rows = [
        ("CPU", format!("{:.1}% (kernel {:.1}%, DPC {:.1}%, interrupts {:.1}%)", s.cpu_total, s.cpu_kernel, s.cpu_dpc, s.cpu_interrupt)),
        ("Memory", format!("{} of {} used ({:.0}%)", fmt_bytes((s.mem.st.total - s.mem.st.avail) as f64), fmt_bytes(s.mem.st.total as f64), s.mem_used_pct())),
        ("Commit", format!("{} / {}", fmt_bytes(s.mem.st.commit_total as f64), fmt_bytes(s.mem.st.commit_limit as f64))),
        ("Processes / threads / handles", format!("{} / {} / {}", s.mem.st.processes, s.mem.st.threads, s.mem.st.handles)),
        ("Disk (busiest)", format!("{:.0}% active", s.disk_active_max())),
        ("Network", format!("down {} up {}", util::fmt_rate(s.net_rx() as f64), util::fmt_rate(s.net_tx() as f64))),
        ("GPU", format!("{:.0}%", s.gpu.total)),
        ("Uptime", util::fmt_dur(s.sys.uptime)),
    ];
    for (k, v) in rows {
        o.push_str(&format!("<tr><td>{k}</td><td>{}</td></tr>", esc(&v)));
    }
    o.push_str("</table><h2>Top processes by CPU</h2><table><tr><th>Name</th><th>PID</th><th>CPU</th><th>Private memory</th><th>Disk</th></tr>");
    let mut ps: Vec<_> = s.procs.values().filter(|p| p.pid > 0).collect();
    ps.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
    for p in ps.iter().take(15) {
        o.push_str(&format!(
            "<tr><td>{}</td><td>{}</td><td>{:.1}%</td><td>{}</td><td>{}</td></tr>",
            esc(&p.name),
            p.pid,
            p.cpu,
            fmt_bytes(p.private_bytes as f64),
            util::fmt_rate((p.io_read_rate + p.io_write_rate) as f64)
        ));
    }
    o.push_str("</table><h2>Recent events</h2><table>");
    for e in s.timeline.iter().rev().take(40) {
        o.push_str(&format!("<tr><td>{}</td><td>{}</td><td>{}</td></tr>", util::fmt_time(e.time), e.category, esc(&e.text)));
    }
    o.push_str("</table></body></html>");
    o
}
