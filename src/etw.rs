//! Live Event Tracing for Windows (ETW): a private real-time session with
//! generic decoding of every event's fields through TDH.

use crate::util::{self, wide};
use std::collections::{HashMap, VecDeque};
use std::ffi::c_void;
use std::sync::{Arc, Mutex};
use windows_sys::core::GUID;
use windows_sys::Win32::System::Diagnostics::Etw::*;

pub struct ProviderDef {
    pub name: &'static str,
    pub guid: u128,
    pub keywords: u64,
    pub level: u8,
    pub what: &'static str,
}

pub const PROVIDERS: &[ProviderDef] = &[
    ProviderDef { name: "Kernel-Process", guid: 0x22fb2cd6_0e7b_422b_a0c7_2fad1fd0e716, keywords: 0x10 | 0x20, level: 4, what: "Process and thread start/stop" },
    ProviderDef { name: "Kernel-Process (image loads)", guid: 0x22fb2cd6_0e7b_422b_a0c7_2fad1fd0e716, keywords: 0x40, level: 4, what: "Every DLL/EXE loaded" },
    ProviderDef { name: "Kernel-Network", guid: 0x7dd42a49_5329_4832_8dfd_43d979153a88, keywords: 0x10 | 0x20, level: 4, what: "TCP/UDP send, receive, connect" },
    ProviderDef { name: "DNS-Client", guid: 0x1c95126e_7eea_49a9_a3fe_a378b03ddb4d, keywords: u64::MAX, level: 4, what: "Name lookups" },
    ProviderDef { name: "Kernel-File (create/delete)", guid: 0xedd08927_9cc4_4e65_b970_c2560fb5c289, keywords: 0x1000 | 0x400, level: 4, what: "New and deleted files" },
    ProviderDef { name: "Kernel-Power", guid: 0x331c3b3a_2005_44c2_ac5e_77220c37d6b4, keywords: u64::MAX, level: 4, what: "Sleep, wake, power changes" },
    ProviderDef { name: "PowerShell", guid: 0xa0c1853b_5c40_4b15_8766_3cf1c58f985a, keywords: u64::MAX, level: 4, what: "Scripts and commands run" },
];

#[derive(Clone, Debug)]
pub struct EtwEvent {
    pub time: i64,
    pub provider: String,
    pub name: String,
    pub id: u16,
    pub pid: u32,
    pub tid: u32,
    pub props: Vec<(String, String)>,
}

#[derive(Default)]
pub struct Sink {
    pub events: VecDeque<EtwEvent>,
    pub counts: HashMap<String, u64>,
    pub total: u64,
}

pub struct Session {
    pub sink: Arc<Mutex<Sink>>,
    handle: CONTROLTRACE_HANDLE,
    trace: PROCESSTRACE_HANDLE,
    name: Vec<u16>,
    ctx_ptr: *const Mutex<Sink>,
    pub providers: Vec<String>,
    pub started: i64,
}

unsafe impl Send for Session {}

const SESSION_NAME: &str = "AdvancedTaskManager-Live";

fn props_buffer(name: &[u16]) -> Vec<u64> {
    let base = std::mem::size_of::<EVENT_TRACE_PROPERTIES>();
    let size = base + name.len() * 2 + 64;
    let mut buf: Vec<u64> = vec![0; size / 8 + 1];
    unsafe {
        let p = buf.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES;
        (*p).Wnode.BufferSize = (buf.len() * 8) as u32;
        (*p).Wnode.Flags = 0x0002_0000; // WNODE_FLAG_TRACED_GUID
        (*p).Wnode.ClientContext = 1;
        (*p).LogFileMode = 0x0000_0100; // real time
        (*p).BufferSize = 64;
        (*p).MinimumBuffers = 8;
        (*p).MaximumBuffers = 64;
        (*p).FlushTimer = 1;
        (*p).LoggerNameOffset = base as u32;
    }
    buf
}

pub fn start(selected: &[usize]) -> Result<Session, String> {
    if selected.is_empty() {
        return Err("Pick at least one source".into());
    }
    let name = wide(SESSION_NAME);
    unsafe {
        // stop a leftover session from an earlier run
        let mut b = props_buffer(&name);
        ControlTraceW(CONTROLTRACE_HANDLE { Value: 0 }, name.as_ptr(), b.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES, 1);
        let mut b = props_buffer(&name);
        let mut handle = CONTROLTRACE_HANDLE { Value: 0 };
        let r = StartTraceW(&mut handle, name.as_ptr(), b.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES);
        if r != 0 {
            return Err(match r {
                5 => "Windows refused to start a trace: this needs the Admin view (run as administrator).".into(),
                _ => format!("Could not start the trace session (error {r})"),
            });
        }
        let mut names = vec![];
        for &i in selected {
            let pd = &PROVIDERS[i];
            let g = GUID::from_u128(pd.guid);
            let r = EnableTraceEx2(handle, &g, 1, pd.level, pd.keywords, 0, 0, std::ptr::null());
            if r == 0 {
                names.push(pd.name.to_string());
            }
        }
        let sink = Arc::new(Mutex::new(Sink::default()));
        let ctx_ptr = Arc::into_raw(sink.clone());
        let mut logfile: EVENT_TRACE_LOGFILEW = std::mem::zeroed();
        let mut name_mut = name.clone();
        logfile.LoggerName = name_mut.as_mut_ptr();
        logfile.Anonymous1.ProcessTraceMode = 0x0000_0100 | 0x1000_0000;
        logfile.Anonymous2.EventRecordCallback = Some(callback);
        logfile.Context = ctx_ptr as *mut c_void;
        let trace = OpenTraceW(&mut logfile);
        if trace.Value == u64::MAX {
            let mut b = props_buffer(&name);
            ControlTraceW(handle, name.as_ptr(), b.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES, 1);
            drop(Arc::from_raw(ctx_ptr));
            return Err("Could not open the live trace".into());
        }
        let th = trace.Value;
        std::thread::spawn(move || {
            let h = PROCESSTRACE_HANDLE { Value: th };
            ProcessTrace(&h, 1, std::ptr::null(), std::ptr::null());
        });
        Ok(Session { sink, handle, trace, name, ctx_ptr, providers: names, started: util::now_unix() })
    }
}

impl Session {
    pub fn stop(self) {
        drop(self);
    }
}

impl Drop for Session {
    fn drop(&mut self) {
        unsafe {
            let mut b = props_buffer(&self.name);
            ControlTraceW(self.handle, self.name.as_ptr(), b.as_mut_ptr() as *mut EVENT_TRACE_PROPERTIES, 1);
            CloseTrace(self.trace);
            // The callback may still run briefly; keep the sink alive by leaking this reference.
            let _ = self.ctx_ptr;
        }
    }
}

unsafe fn wstr_at(base: *const u8, off: u32) -> String {
    if off == 0 {
        return String::new();
    }
    util::from_wide_ptr(base.add(off as usize) as *const u16).trim().to_string()
}

unsafe extern "system" fn callback(rec: *mut EVENT_RECORD) {
    let r = &*rec;
    if r.UserContext.is_null() {
        return;
    }
    let sink = &*(r.UserContext as *const Mutex<Sink>);
    let hdr = &r.EventHeader;
    let mut ev = EtwEvent {
        time: util::filetime_to_unix(hdr.TimeStamp),
        provider: String::new(),
        name: String::new(),
        id: hdr.EventDescriptor.Id,
        pid: hdr.ProcessId,
        tid: hdr.ThreadId,
        props: vec![],
    };
    let mut size = 0u32;
    let st = TdhGetEventInformation(rec, 0, std::ptr::null(), std::ptr::null_mut(), &mut size);
    if st == 122 && size > 0 && size < 1 << 20 {
        let mut buf: Vec<u64> = vec![0; size as usize / 8 + 1];
        let info = buf.as_mut_ptr() as *mut TRACE_EVENT_INFO;
        if TdhGetEventInformation(rec, 0, std::ptr::null(), info, &mut size) == 0 {
            let base = info as *const u8;
            let ti = &*info;
            ev.provider = wstr_at(base, ti.ProviderNameOffset).replace("Microsoft-Windows-", "");
            let task = wstr_at(base, ti.TaskNameOffset);
            let opcode = wstr_at(base, ti.OpcodeNameOffset);
            let evname = wstr_at(base, ti.Anonymous1.EventNameOffset);
            ev.name = if !evname.is_empty() {
                evname
            } else if !opcode.is_empty() && opcode != "Info" {
                format!("{task} {opcode}")
            } else if !task.is_empty() {
                task
            } else {
                format!("Event {}", ev.id)
            };
            let props = std::ptr::addr_of!(ti.EventPropertyInfoArray) as *const EVENT_PROPERTY_INFO;
            for i in 0..ti.TopLevelPropertyCount.min(32) as usize {
                let epi = &*props.add(i);
                if epi.Flags & 0x1 != 0 {
                    continue; // struct
                }
                if epi.Flags & 0x4 == 0 && epi.Anonymous2.count > 1 {
                    continue; // fixed arrays
                }
                let pname = wstr_at(base, epi.NameOffset);
                let pdd = PROPERTY_DATA_DESCRIPTOR { PropertyName: base.add(epi.NameOffset as usize) as u64, ArrayIndex: u32::MAX, Reserved: 0 };
                let mut psize = 0u32;
                if TdhGetPropertySize(rec, 0, std::ptr::null(), 1, &pdd, &mut psize) != 0 || psize == 0 || psize > 8192 {
                    continue;
                }
                let mut data = vec![0u8; psize as usize + 2];
                if TdhGetProperty(rec, 0, std::ptr::null(), 1, &pdd, psize, data.as_mut_ptr()) != 0 {
                    continue;
                }
                let it = epi.Anonymous1.nonStructType.InType;
                let ot = epi.Anonymous1.nonStructType.OutType;
                ev.props.push((pname, format_value(it, ot, &data[..psize as usize])));
            }
        }
    }
    if ev.provider.is_empty() {
        ev.provider = format!("{:?}", hdr.ProviderId.data1);
    }
    let mut s = sink.lock().unwrap();
    s.total += 1;
    *s.counts.entry(format!("{} / {}", ev.provider, ev.name)).or_default() += 1;
    if s.events.len() >= 5000 {
        s.events.pop_front();
    }
    s.events.push_back(ev);
}

fn rd<const N: usize>(d: &[u8]) -> Option<[u8; N]> {
    d.get(..N).and_then(|s| s.try_into().ok())
}

fn format_value(intype: u16, outtype: u16, d: &[u8]) -> String {
    match intype {
        1 => {
            let w: Vec<u16> = d.chunks_exact(2).map(|c| u16::from_le_bytes([c[0], c[1]])).collect();
            util::from_wide(&w)
        }
        2 => String::from_utf8_lossy(d.split(|&b| b == 0).next().unwrap_or(&[])).to_string(),
        3 => rd::<1>(d).map(|b| (b[0] as i8).to_string()).unwrap_or_default(),
        4 => rd::<1>(d).map(|b| b[0].to_string()).unwrap_or_default(),
        5 => rd::<2>(d).map(|b| i16::from_le_bytes(b).to_string()).unwrap_or_default(),
        6 => rd::<2>(d)
            .map(|b| if outtype == 22 { u16::from_be_bytes(b).to_string() } else { u16::from_le_bytes(b).to_string() })
            .unwrap_or_default(),
        7 => rd::<4>(d).map(|b| i32::from_le_bytes(b).to_string()).unwrap_or_default(),
        8 => rd::<4>(d)
            .map(|b| if outtype == 23 { format!("{}.{}.{}.{}", b[0], b[1], b[2], b[3]) } else { u32::from_le_bytes(b).to_string() })
            .unwrap_or_default(),
        9 => rd::<8>(d).map(|b| i64::from_le_bytes(b).to_string()).unwrap_or_default(),
        10 => rd::<8>(d).map(|b| u64::from_le_bytes(b).to_string()).unwrap_or_default(),
        11 => rd::<4>(d).map(|b| format!("{:.3}", f32::from_le_bytes(b))).unwrap_or_default(),
        12 => rd::<8>(d).map(|b| format!("{:.3}", f64::from_le_bytes(b))).unwrap_or_default(),
        13 => rd::<4>(d).map(|b| if u32::from_le_bytes(b) != 0 { "true".into() } else { "false".into() }).unwrap_or_default(),
        15 => rd::<16>(d)
            .map(|b| {
                format!(
                    "{{{:08X}-{:04X}-{:04X}-{}}}",
                    u32::from_le_bytes([b[0], b[1], b[2], b[3]]),
                    u16::from_le_bytes([b[4], b[5]]),
                    u16::from_le_bytes([b[6], b[7]]),
                    b[8..].iter().map(|x| format!("{x:02X}")).collect::<String>()
                )
            })
            .unwrap_or_default(),
        16 | 21 => {
            if d.len() >= 8 {
                format!("0x{:x}", u64::from_le_bytes(rd::<8>(d).unwrap()))
            } else if d.len() >= 4 {
                format!("0x{:x}", u32::from_le_bytes(rd::<4>(d).unwrap()))
            } else {
                String::new()
            }
        }
        17 => rd::<8>(d).map(|b| util::fmt_time(util::filetime_to_unix(i64::from_le_bytes(b)))).unwrap_or_default(),
        20 => rd::<4>(d).map(|b| format!("0x{:x}", u32::from_le_bytes(b))).unwrap_or_default(),
        _ => {
            if outtype == 24 && d.len() >= 16 {
                let a: [u8; 16] = d[..16].try_into().unwrap();
                std::net::Ipv6Addr::from(a).to_string()
            } else {
                let hex: String = d.iter().take(32).map(|b| format!("{b:02x}")).collect();
                if d.len() > 32 {
                    format!("{hex}… ({} bytes)", d.len())
                } else {
                    hex
                }
            }
        }
    }
}
