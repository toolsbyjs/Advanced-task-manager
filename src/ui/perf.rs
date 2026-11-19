//! Performance pages: CPU, memory, disk, network and GPU with live graphs and detail.

use super::widgets::*;
use super::App;
use crate::util::{self, fmt_bytes, fmt_num};
use egui::{RichText, Ui};

#[derive(Clone, Copy, PartialEq, Eq, Default)]
pub enum PerfTab {
    #[default]
    Cpu,
    Memory,
    Disk,
    Network,
    Gpu,
}

pub struct PerfUi {
    pub tab: PerfTab,
    pub points: usize,
    pub disk: usize,
    pub net: usize,
}

impl Default for PerfUi {
    fn default() -> Self {
        Self { tab: PerfTab::Cpu, points: 60, disk: 0, net: 0 }
    }
}

pub fn page(app: &mut App, ui: &mut Ui) {
    let shared = app.shared.clone();
    let s = shared.lock().unwrap();
    let n = app.perf.points;
    ui.horizontal(|ui| {
        ui.label(RichText::new("Performance").size(22.0).strong());
        ui.add_space(20.0);
        ui.label("Show the last");
        for (p, l) in [(60, "1 min"), (300, "5 min"), (900, "15 min"), (3600, "1 hour")] {
            ui.selectable_value(&mut app.perf.points, p, l);
        }
    });
    ui.add_space(4.0);
    egui::SidePanel::left("perf_tiles").resizable(false).exact_width(260.0).frame(egui::Frame::none()).show_inside(ui, |ui| {
        egui::ScrollArea::vertical().id_salt("perf_tiles_s").show(ui, |ui| {
            let used = s.mem.st.total.saturating_sub(s.mem.st.avail);
            let tiles: Vec<(PerfTab, usize, String, String, String, egui::Color32, Vec<f32>, Option<f32>)> = {
                let mut v = vec![
                    (PerfTab::Cpu, 0, "CPU".into(), format!("{:.0}%", s.cpu_total), format!("{} MHz avg", avg_mhz(&s)), CYAN, s.hist.cpu.to_vec(60), Some(100.0)),
                    (
                        PerfTab::Memory,
                        0,
                        "Memory".into(),
                        format!("{:.0}%", s.mem_used_pct()),
                        format!("{} / {}", fmt_bytes(used as f64), fmt_bytes(s.mem.st.total as f64)),
                        PURPLE,
                        s.hist.mem.to_vec(60),
                        Some(100.0),
                    ),
                ];
                for (i, d) in s.disks.iter().enumerate() {
                    v.push((PerfTab::Disk, i, format!("Disk {}", d.name), format!("{:.0}%", d.active), format!("R {} W {}", util::fmt_rate(d.read as f64), util::fmt_rate(d.write as f64)), GREEN, d.hist_active.to_vec(60), Some(100.0)));
                }
                for (i, nf) in s.nets.iter().enumerate().take(6) {
                    let data: Vec<f32> = nf.hist_rx.data.iter().zip(nf.hist_tx.data.iter()).map(|(a, b)| a + b).collect();
                    v.push((PerfTab::Network, i, nf.row.alias.clone(), util::fmt_bits((nf.rx + nf.tx) as f64), format!("⬇ {} ⬆ {}", util::fmt_rate(nf.rx as f64), util::fmt_rate(nf.tx as f64)), ORANGE, data, None));
                }
                v.push((PerfTab::Gpu, 0, "GPU".into(), format!("{:.0}%", s.gpu.total), format!("{} dedicated in use", fmt_bytes(s.gpu.dedicated_used as f64)), PINK, s.hist.gpu.to_vec(60), Some(100.0)));
                v
            };
            for (tab, idx, title, val, sub, color, data, max) in tiles {
                let sel = app.perf.tab == tab && (tab != PerfTab::Disk || app.perf.disk == idx) && (tab != PerfTab::Network || app.perf.net == idx);
                if tile(ui, &title, &val, &sub, color, &data, max, sel).clicked() {
                    app.perf.tab = tab;
                    if tab == PerfTab::Disk {
                        app.perf.disk = idx;
                    }
                    if tab == PerfTab::Network {
                        app.perf.net = idx;
                    }
                }
                ui.add_space(4.0);
            }
        });
    });
    egui::ScrollArea::vertical().id_salt("perf_main").auto_shrink([false, false]).show(ui, |ui| {
        ui.add_space(2.0);
        match app.perf.tab {
            PerfTab::Cpu => cpu(ui, &s, n),
            PerfTab::Memory => memory(ui, &s, n),
            PerfTab::Disk => disk(ui, &s, n, app.perf.disk),
            PerfTab::Network => network(ui, &s, n, app.perf.net),
            PerfTab::Gpu => gpu(ui, &s, n),
        }
    });
}

fn avg_mhz(s: &crate::collector::State) -> u32 {
    if s.cores.is_empty() {
        return 0;
    }
    (s.cores.iter().map(|c| c.mhz as u64).sum::<u64>() / s.cores.len() as u64) as u32
}

pub fn cpu(ui: &mut Ui, s: &crate::collector::State, n: usize) {
    card(ui, &format!("CPU — {}", s.cpu_name), |ui| {
        chart(
            ui,
            &[Series { data: s.hist.cpu.to_vec(n), color: CYAN, label: "Total" }, Series { data: s.hist.cpu_kernel.to_vec(n), color: BLUE, label: "Kernel" }],
            200.0,
            Some(100.0),
            n,
            &pct,
        );
    });
    ui.add_space(8.0);
    ui.columns(2, |cols| {
        card(&mut cols[0], "Right now", |ui| {
            let max_mhz = s.cores.first().map(|c| c.max_mhz).unwrap_or(0);
            kv(
                ui,
                "cpu_now",
                &[
                    ("Utilization", format!("{:.1}%", s.cpu_total)),
                    ("User / kernel", format!("{:.1}% / {:.1}%", s.cpu_user, s.cpu_kernel)),
                    ("Interrupt time", format!("{:.2}%", s.cpu_interrupt)),
                    ("DPC time", format!("{:.2}%", s.cpu_dpc)),
                    ("Speed (avg core)", format!("{} MHz (base {} MHz)", avg_mhz(s), max_mhz)),
                    ("Performance vs base", format!("{:.0}%", s.sys.perf_pct)),
                    ("Logical processors", s.ncpu.to_string()),
                    ("Processes / threads / handles", format!("{} / {} / {}", s.procs.len(), fmt_num(s.mem.st.threads as u64), fmt_num(s.mem.st.handles as u64))),
                    ("Context switches/s", fmt_num(s.sys.ctx_switches as u64)),
                    ("System calls/s", fmt_num(s.sys.syscalls as u64)),
                    ("Interrupts/s", fmt_num(s.sys.interrupts as u64)),
                    ("Exceptions/s", fmt_num(s.sys.exceptions as u64)),
                    ("Processor queue", format!("{:.0} waiting threads", s.sys.proc_queue)),
                    ("Up time", util::fmt_dur(s.sys.uptime)),
                ],
            );
        });
        card(&mut cols[1], "Top processes by CPU", |ui| {
            let mut ps: Vec<_> = s.procs.values().filter(|p| p.pid != 0).collect();
            ps.sort_by(|a, b| b.cpu.total_cmp(&a.cpu));
            for p in ps.iter().take(12) {
                ui.horizontal(|ui| {
                    bar(ui, p.cpu / 100.0, load_color(p.cpu), &format!("{:.1}%", p.cpu), 70.0);
                    ui.label(&p.name);
                    ui.label(RichText::new(format!("kernel {:.1}%", p.cpu_kernel)).color(c_dim()).size(11.0));
                });
            }
        });
    });
    ui.add_space(8.0);
    core_grid(ui, s);
}

pub fn core_grid(ui: &mut Ui, s: &crate::collector::State) {
    card(ui, &format!("Logical processors ({})", s.cores.len()), |ui| {
        let w = ui.available_width();
        let per_row = ((w / 150.0).floor() as usize).clamp(2, 16);
        let cw = w / per_row as f32 - 8.0;
        egui::Grid::new("cores").spacing([8.0, 8.0]).show(ui, |ui| {
            for (i, c) in s.cores.iter().enumerate() {
                ui.vertical(|ui| {
                    ui.set_width(cw);
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(format!("CPU {i}")).size(11.0).color(c_dim()));
                        ui.label(RichText::new(format!("{:.0}%", c.usage)).size(11.0).color(load_color(c.usage)));
                        ui.label(RichText::new(format!("{} MHz", c.mhz)).size(10.5).color(c_dim()));
                    });
                    sparkline(ui, &c.hist.to_vec(60), load_color(c.usage), egui::vec2(cw, 34.0), Some(100.0))
                        .on_hover_text(format!("Kernel {:.1}%  DPC {:.1}%  Interrupt {:.1}%  {:.0} interrupts/s", c.kernel, c.dpc, c.interrupt, c.interrupts_sec));
                });
                if (i + 1) % per_row == 0 {
                    ui.end_row();
                }
            }
        });
    });
}

pub fn memory(ui: &mut Ui, s: &crate::collector::State, n: usize) {
    let m = &s.mem;
    let used = m.st.total.saturating_sub(m.st.avail);
    card(ui, &format!("Memory — {} installed", fmt_bytes(m.st.total as f64)), |ui| {
        chart(
            ui,
            &[Series { data: s.hist.mem.to_vec(n), color: PURPLE, label: "In use" }, Series { data: s.hist.commit.to_vec(n), color: PINK, label: "Commit of limit" }],
            200.0,
            Some(100.0),
            n,
            &pct,
        );
    });
    ui.add_space(8.0);
    card(ui, "Memory composition", |ui| {
        let total = m.st.total.max(1) as f32;
        let in_use = used.saturating_sub(m.modified) as f32;
        let parts = [
            ("In use", in_use, PURPLE),
            ("Modified", m.modified as f32, ORANGE),
            ("Standby (cache)", m.standby as f32, BLUE),
            ("Free", m.free_zero as f32, GREEN),
        ];
        let (rect, _) = ui.allocate_exact_size(egui::vec2(ui.available_width(), 30.0), egui::Sense::hover());
        let mut x = rect.left();
        for (_, v, c) in parts {
            let w = rect.width() * (v / total).clamp(0.0, 1.0);
            ui.painter().rect_filled(egui::Rect::from_min_size(egui::pos2(x, rect.top()), egui::vec2(w, rect.height())), 3.0, c.gamma_multiply(0.8));
            x += w;
        }
        ui.horizontal_wrapped(|ui| {
            for (l, v, c) in parts {
                ui.label(RichText::new("■").color(c));
                ui.label(format!("{l} {}", fmt_bytes(v as f64)));
                ui.add_space(10.0);
            }
        });
    });
    ui.add_space(8.0);
    ui.columns(2, |cols| {
        card(&mut cols[0], "Details", |ui| {
            kv(
                ui,
                "mem_kv",
                &[
                    ("In use", format!("{} ({:.1}%)", fmt_bytes(used as f64), s.mem_used_pct())),
                    ("Available", fmt_bytes(m.st.avail as f64)),
                    ("Compressed (Memory Compression)", fmt_bytes(m.compressed as f64)),
                    ("Committed", format!("{} of {} (peak {})", fmt_bytes(m.st.commit_total as f64), fmt_bytes(m.st.commit_limit as f64), fmt_bytes(m.st.commit_peak as f64))),
                    ("System cache", fmt_bytes(m.st.cache as f64)),
                    ("Standby list", fmt_bytes(m.standby as f64)),
                    ("Modified list", fmt_bytes(m.modified as f64)),
                    ("Free + zero list", fmt_bytes(m.free_zero as f64)),
                    ("Kernel paged pool", fmt_bytes(m.pool_paged as f64)),
                    ("Kernel non-paged pool", fmt_bytes(m.pool_nonpaged as f64)),
                    ("Kernel total", fmt_bytes(m.st.kernel_total as f64)),
                    ("Page faults/s", fmt_num(m.faults_sec as u64)),
                    ("Hard paging (pages/s)", format!("{:.0}", m.pages_sec)),
                    ("Page size", fmt_bytes(m.st.page_size as f64)),
                ],
            );
        });
        card(&mut cols[1], "Top processes by memory", |ui| {
            let mut ps: Vec<_> = s.procs.values().collect();
            ps.sort_by(|a, b| b.private_bytes.cmp(&a.private_bytes));
            let maxv = ps.first().map(|p| p.private_bytes).unwrap_or(1).max(1) as f32;
            for p in ps.iter().take(14) {
                ui.horizontal(|ui| {
                    bar(ui, p.private_bytes as f32 / maxv, PURPLE, &fmt_bytes(p.private_bytes as f64), 90.0);
                    ui.label(&p.name);
                    ui.label(RichText::new(format!("WS {}", fmt_bytes(p.ws as f64))).color(c_dim()).size(11.0));
                });
            }
        });
    });
}

pub fn disk(ui: &mut Ui, s: &crate::collector::State, n: usize, idx: usize) {
    if let Some(d) = s.disks.get(idx) {
        card(ui, &format!("Disk {} — active time", d.name), |ui| {
            let h: Vec<f32> = if n > d.hist_active.len() { d.hist_active.to_vec(n) } else { d.hist_active.to_vec(n) };
            chart(ui, &[Series { data: h, color: GREEN, label: "Active" }], 160.0, Some(100.0), n.min(120), &pct);
        });
        ui.add_space(6.0);
        card(ui, "Transfer rate", |ui| {
            chart(
                ui,
                &[Series { data: d.hist_read.to_vec(120), color: TEAL, label: "Read" }, Series { data: d.hist_write.to_vec(120), color: ORANGE, label: "Write" }],
                140.0,
                None,
                120,
                &rate,
            );
        });
        ui.add_space(6.0);
        ui.columns(2, |cols| {
            card(&mut cols[0], "Right now", |ui| {
                kv(
                    ui,
                    "disk_kv",
                    &[
                        ("Active time", format!("{:.1}%", d.active)),
                        ("Read speed", util::fmt_rate(d.read as f64)),
                        ("Write speed", util::fmt_rate(d.write as f64)),
                        ("Queue length", format!("{:.0}", d.queue)),
                        ("Average response", format!("{:.2} ms", d.resp_ms)),
                    ],
                );
            });
            card(&mut cols[1], "Top processes by disk", |ui| {
                let mut ps: Vec<_> = s.procs.values().collect();
                ps.sort_by(|a, b| (b.io_read_rate + b.io_write_rate).total_cmp(&(a.io_read_rate + a.io_write_rate)));
                for p in ps.iter().take(10) {
                    ui.horizontal(|ui| {
                        ui.label(RichText::new(util::fmt_rate((p.io_read_rate + p.io_write_rate) as f64)).color(GREEN).monospace());
                        ui.label(&p.name);
                    });
                }
            });
        });
    } else {
        ui.label("No physical disk counters are available.");
    }
    ui.add_space(6.0);
    volumes_card(ui, s);
}

pub fn volumes_card(ui: &mut Ui, s: &crate::collector::State) {
    card(ui, "Drives", |ui| {
        for v in &s.volumes {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&v.root).strong().monospace());
                ui.label(RichText::new(format!("{} {} {}", v.label, v.fs, v.kind)).color(c_dim()).size(12.0));
            });
            if v.total > 0 {
                let used = v.total - v.free;
                let f = used as f32 / v.total as f32;
                bar(ui, f, load_color(f * 100.0), &format!("{} free of {}", fmt_bytes(v.free as f64), fmt_bytes(v.total as f64)), ui.available_width());
            }
            ui.add_space(3.0);
        }
    });
}

pub fn network(ui: &mut Ui, s: &crate::collector::State, _n: usize, idx: usize) {
    let Some(nf) = s.nets.get(idx) else {
        ui.label("No network adapters with traffic.");
        return;
    };
    card(ui, &format!("{} — {}", nf.row.alias, nf.row.description), |ui| {
        chart(
            ui,
            &[Series { data: nf.hist_rx.to_vec(120), color: ORANGE, label: "Receive" }, Series { data: nf.hist_tx.to_vec(120), color: CYAN, label: "Send" }],
            200.0,
            None,
            120,
            &rate,
        );
    });
    ui.add_space(6.0);
    ui.columns(2, |cols| {
        card(&mut cols[0], "Adapter", |ui| {
            let r = &nf.row;
            kv(
                ui,
                "net_kv",
                &[
                    ("Type", sys_if(r.if_type)),
                    ("Status", if r.up { "Connected".into() } else { "Disconnected".into() }),
                    ("Link speed", if r.speed > 0 { util::fmt_bits(r.speed as f64 / 8.0) } else { "-".into() }),
                    ("MAC address", r.mac.clone()),
                    ("MTU", r.mtu.to_string()),
                    ("Receive", format!("{} ({})", util::fmt_rate(nf.rx as f64), util::fmt_bits(nf.rx as f64))),
                    ("Send", format!("{} ({})", util::fmt_rate(nf.tx as f64), util::fmt_bits(nf.tx as f64))),
                    ("Received total", format!("{} in {} packets", fmt_bytes(r.in_octets as f64), fmt_num(r.in_pkts))),
                    ("Sent total", format!("{} in {} packets", fmt_bytes(r.out_octets as f64), fmt_num(r.out_pkts))),
                    ("Errors in / out", format!("{} / {}", r.in_errors, r.out_errors)),
                    ("Discards in / out", format!("{} / {}", r.in_discards, r.out_discards)),
                    ("Physical adapter", if r.hardware { "Yes".into() } else { "No (virtual)".into() }),
                ],
            );
        });
        card(&mut cols[1], "Processes with most connections", |ui| {
            let mut ps: Vec<_> = s.procs.values().filter(|p| p.connections > 0).collect();
            ps.sort_by(|a, b| b.connections.cmp(&a.connections));
            for p in ps.iter().take(12) {
                ui.horizontal(|ui| {
                    ui.label(RichText::new(format!("{:>4}", p.connections)).color(ORANGE).monospace());
                    ui.label(&p.name);
                });
            }
            ui.label(RichText::new("Per-process bandwidth needs the live events page (Admin view).").color(c_dim()).size(11.0));
        });
    });
}

fn sys_if(t: u32) -> String {
    crate::sys::if_type_name(t).to_string()
}

pub fn gpu(ui: &mut Ui, s: &crate::collector::State, n: usize) {
    card(ui, "GPU — overall", |ui| {
        chart(ui, &[Series { data: s.hist.gpu.to_vec(n), color: PINK, label: "Busiest engine" }], 160.0, Some(100.0), n, &pct);
    });
    ui.add_space(6.0);
    card(ui, "Engines", |ui| {
        let w = ui.available_width();
        let per_row = ((w / 260.0).floor() as usize).clamp(1, 4);
        let cw = w / per_row as f32 - 10.0;
        egui::Grid::new("gpu_eng").spacing([10.0, 10.0]).show(ui, |ui| {
            for (i, (name, v)) in s.gpu.engines.iter().enumerate() {
                ui.vertical(|ui| {
                    ui.set_width(cw);
                    ui.label(format!("{name}  {v:.0}%"));
                    let h = s.gpu.engine_hist.get(name).map(|r| r.to_vec(60)).unwrap_or_default();
                    sparkline(ui, &h, PINK, egui::vec2(cw, 50.0), Some(100.0));
                });
                if (i + 1) % per_row == 0 {
                    ui.end_row();
                }
            }
        });
        if s.gpu.engines.is_empty() {
            ui.label("No GPU engine counters reported by Windows.");
        }
    });
    ui.add_space(6.0);
    ui.columns(2, |cols| {
        card(&mut cols[0], "Memory", |ui| {
            kv(
                ui,
                "gpu_kv",
                &[("Dedicated in use", fmt_bytes(s.gpu.dedicated_used as f64)), ("Shared in use", fmt_bytes(s.gpu.shared_used as f64))],
            );
        });
        card(&mut cols[1], "Top processes by GPU", |ui| {
            let mut ps: Vec<_> = s.procs.values().filter(|p| p.gpu > 0.0 || p.gpu_dedicated > 0).collect();
            ps.sort_by(|a, b| b.gpu.total_cmp(&a.gpu).then(b.gpu_dedicated.cmp(&a.gpu_dedicated)));
            for p in ps.iter().take(10) {
                ui.horizontal(|ui| {
                    bar(ui, p.gpu / 100.0, PINK, &format!("{:.1}%", p.gpu), 70.0);
                    ui.label(&p.name);
                    ui.label(RichText::new(fmt_bytes(p.gpu_dedicated as f64)).color(c_dim()).size(11.0));
                });
            }
        });
    });
}
