//! Native (ntdll) queries: one call returns every process with its threads and
//! dozens of counters, which keeps the sampling overhead low.

use std::ffi::c_void;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct UnicodeString {
    pub length: u16,
    pub max_length: u16,
    pub buffer: *mut u16,
}

#[link(name = "ntdll")]
extern "system" {
    fn NtQuerySystemInformation(class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn NtQueryObject(handle: *mut c_void, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn NtSuspendProcess(handle: *mut c_void) -> i32;
    fn NtResumeProcess(handle: *mut c_void) -> i32;
    fn NtQueryInformationProcess(handle: *mut c_void, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
    fn NtQueryInformationThread(handle: *mut c_void, class: u32, info: *mut c_void, len: u32, ret: *mut u32) -> i32;
}

const STATUS_INFO_LENGTH_MISMATCH: i32 = 0xC000_0004u32 as i32;
const STATUS_BUFFER_TOO_SMALL: i32 = 0xC000_0023u32 as i32;
const STATUS_BUFFER_OVERFLOW: i32 = 0x8000_0005u32 as i32;

#[repr(C)]
#[derive(Clone, Copy)]
struct SysThreadInfo {
    kernel_time: i64,
    user_time: i64,
    create_time: i64,
    wait_time: u32,
    start_address: usize,
    client_pid: usize,
    client_tid: usize,
    priority: i32,
    base_priority: i32,
    context_switches: u32,
    thread_state: u32,
    wait_reason: u32,
}

#[repr(C)]
#[derive(Clone, Copy)]
struct SysProcInfo {
    next_entry_offset: u32,
    number_of_threads: u32,
    working_set_private_size: i64,
    hard_fault_count: u32,
    number_of_threads_high_watermark: u32,
    cycle_time: u64,
    create_time: i64,
    user_time: i64,
    kernel_time: i64,
    image_name: UnicodeString,
    base_priority: i32,
    unique_process_id: usize,
    inherited_from_unique_process_id: usize,
    handle_count: u32,
    session_id: u32,
    unique_process_key: usize,
    peak_virtual_size: usize,
    virtual_size: usize,
    page_fault_count: u32,
    peak_working_set_size: usize,
    working_set_size: usize,
    quota_peak_paged_pool_usage: usize,
    quota_paged_pool_usage: usize,
    quota_peak_non_paged_pool_usage: usize,
    quota_non_paged_pool_usage: usize,
    pagefile_usage: usize,
    peak_pagefile_usage: usize,
    private_page_count: usize,
    read_operation_count: i64,
    write_operation_count: i64,
    other_operation_count: i64,
    read_transfer_count: i64,
    write_transfer_count: i64,
    other_transfer_count: i64,
}

#[derive(Clone, Debug, Default)]
pub struct RawThread {
    pub tid: u32,
    pub kernel: i64,
    pub user: i64,
    pub create_time: i64,
    pub wait_time: u32,
    pub start_address: usize,
    pub priority: i32,
    pub base_priority: i32,
    pub context_switches: u32,
    pub state: u32,
    pub wait_reason: u32,
}

#[derive(Clone, Debug, Default)]
pub struct RawProc {
    pub pid: u32,
    pub ppid: u32,
    pub name: String,
    pub create_time: i64,
    pub user: i64,
    pub kernel: i64,
    pub cycle_time: u64,
    pub base_priority: i32,
    pub handles: u32,
    pub session: u32,
    pub peak_virtual: usize,
    pub virtual_size: usize,
    pub page_faults: u32,
    pub hard_faults: u32,
    pub peak_ws: usize,
    pub ws: usize,
    pub private_ws: i64,
    pub paged_pool: usize,
    pub peak_paged_pool: usize,
    pub nonpaged_pool: usize,
    pub peak_nonpaged_pool: usize,
    pub commit: usize,
    pub peak_commit: usize,
    pub private_bytes: usize,
    pub read_ops: i64,
    pub write_ops: i64,
    pub other_ops: i64,
    pub read_bytes: i64,
    pub write_bytes: i64,
    pub other_bytes: i64,
    pub threads_high_watermark: u32,
    pub threads: Vec<RawThread>,
}

/// Query class with a growing, reusable buffer (aligned to 8 bytes).
fn query_sys(class: u32, buf: &mut Vec<u64>) -> Option<usize> {
    if buf.is_empty() {
        buf.resize(1 << 17, 0);
    }
    for _ in 0..8 {
        let mut ret = 0u32;
        let bytes = (buf.len() * 8) as u32;
        let st = unsafe { NtQuerySystemInformation(class, buf.as_mut_ptr() as *mut c_void, bytes, &mut ret) };
        if st == STATUS_INFO_LENGTH_MISMATCH || st == STATUS_BUFFER_TOO_SMALL {
            let want = (ret as usize).max(bytes as usize * 2) / 8 + 4096;
            buf.resize(want, 0);
            continue;
        }
        if st < 0 {
            return None;
        }
        return Some(ret as usize);
    }
    None
}

pub fn query_processes(buf: &mut Vec<u64>) -> Vec<RawProc> {
    let mut out = Vec::with_capacity(400);
    let Some(_len) = query_sys(5, buf) else { return out };
    let base = buf.as_ptr() as *const u8;
    let mut off = 0usize;
    let total = buf.len() * 8;
    unsafe {
        loop {
            if off + std::mem::size_of::<SysProcInfo>() > total {
                break;
            }
            let p = std::ptr::read_unaligned(base.add(off) as *const SysProcInfo);
            let name = if p.image_name.buffer.is_null() || p.image_name.length == 0 {
                if p.unique_process_id == 0 { "System Idle Process".to_string() } else { "System".to_string() }
            } else {
                let sl = std::slice::from_raw_parts(p.image_name.buffer, (p.image_name.length / 2) as usize);
                String::from_utf16_lossy(sl)
            };
            let tbase = base.add(off + std::mem::size_of::<SysProcInfo>()) as *const SysThreadInfo;
            let n = p.number_of_threads.min(20000) as usize;
            let mut threads = Vec::with_capacity(n);
            for i in 0..n {
                let t = std::ptr::read_unaligned(tbase.add(i));
                threads.push(RawThread {
                    tid: t.client_tid as u32,
                    kernel: t.kernel_time,
                    user: t.user_time,
                    create_time: t.create_time,
                    wait_time: t.wait_time,
                    start_address: t.start_address,
                    priority: t.priority,
                    base_priority: t.base_priority,
                    context_switches: t.context_switches,
                    state: t.thread_state,
                    wait_reason: t.wait_reason,
                });
            }
            out.push(RawProc {
                pid: p.unique_process_id as u32,
                ppid: p.inherited_from_unique_process_id as u32,
                name,
                create_time: p.create_time,
                user: p.user_time,
                kernel: p.kernel_time,
                cycle_time: p.cycle_time,
                base_priority: p.base_priority,
                handles: p.handle_count,
                session: p.session_id,
                peak_virtual: p.peak_virtual_size,
                virtual_size: p.virtual_size,
                page_faults: p.page_fault_count,
                hard_faults: p.hard_fault_count,
                peak_ws: p.peak_working_set_size,
                ws: p.working_set_size,
                private_ws: p.working_set_private_size,
                paged_pool: p.quota_paged_pool_usage,
                peak_paged_pool: p.quota_peak_paged_pool_usage,
                nonpaged_pool: p.quota_non_paged_pool_usage,
                peak_nonpaged_pool: p.quota_peak_non_paged_pool_usage,
                commit: p.pagefile_usage,
                peak_commit: p.peak_pagefile_usage,
                private_bytes: p.private_page_count,
                read_ops: p.read_operation_count,
                write_ops: p.write_operation_count,
                other_ops: p.other_operation_count,
                read_bytes: p.read_transfer_count,
                write_bytes: p.write_transfer_count,
                other_bytes: p.other_transfer_count,
                threads_high_watermark: p.number_of_threads_high_watermark,
                threads,
            });
            if p.next_entry_offset == 0 {
                break;
            }
            off += p.next_entry_offset as usize;
        }
    }
    out
}

#[repr(C)]
#[derive(Clone, Copy, Default, Debug)]
pub struct CpuTimes {
    pub idle: i64,
    pub kernel: i64,
    pub user: i64,
    pub dpc: i64,
    pub interrupt: i64,
    pub interrupt_count: u32,
    pub _pad: u32,
}

pub fn query_cpu_times(buf: &mut Vec<u64>) -> Vec<CpuTimes> {
    if buf.len() < 4096 {
        buf.resize(4096, 0);
    }
    let Some(len) = query_sys(8, buf) else { return vec![] };
    let n = len / std::mem::size_of::<CpuTimes>();
    let mut v = Vec::with_capacity(n);
    unsafe {
        let p = buf.as_ptr() as *const CpuTimes;
        for i in 0..n {
            v.push(std::ptr::read_unaligned(p.add(i)));
        }
    }
    v
}

#[repr(C)]
#[derive(Clone, Copy)]
struct HandleEntryEx {
    object: usize,
    pid: usize,
    handle: usize,
    access: u32,
    creator_back_trace: u16,
    type_index: u16,
    attributes: u32,
    reserved: u32,
}

#[derive(Clone, Debug)]
pub struct RawHandle {
    pub pid: u32,
    pub handle: usize,
    pub object: usize,
    pub access: u32,
    pub type_index: u16,
    pub attributes: u32,
}

/// Every open handle in the system (optionally only one process's).
pub fn query_handles(filter_pid: Option<u32>) -> Vec<RawHandle> {
    let mut buf: Vec<u64> = vec![0; 1 << 19];
    let mut out = Vec::new();
    if query_sys(64, &mut buf).is_none() {
        return out;
    }
    unsafe {
        let base = buf.as_ptr() as *const u8;
        let count = std::ptr::read_unaligned(base as *const usize);
        let entries = base.add(16) as *const HandleEntryEx;
        let max = (buf.len() * 8 - 16) / std::mem::size_of::<HandleEntryEx>();
        for i in 0..count.min(max) {
            let e = std::ptr::read_unaligned(entries.add(i));
            if let Some(f) = filter_pid {
                if e.pid as u32 != f {
                    continue;
                }
            }
            out.push(RawHandle {
                pid: e.pid as u32,
                handle: e.handle,
                object: e.object,
                access: e.access,
                type_index: e.type_index,
                attributes: e.attributes,
            });
        }
    }
    out
}

#[derive(Clone, Debug)]
pub struct ObjectType {
    pub index: u16,
    pub name: String,
    pub objects: u32,
    pub handles: u32,
    pub peak_objects: u32,
    pub peak_handles: u32,
}

/// The kernel's object types with system-wide object and handle totals.
pub fn query_object_types() -> Vec<ObjectType> {
    let mut buf: Vec<u64> = vec![0; 1 << 13];
    let mut ok = false;
    for _ in 0..6 {
        let mut ret = 0u32;
        let st = unsafe { NtQueryObject(std::ptr::null_mut(), 3, buf.as_mut_ptr() as *mut c_void, (buf.len() * 8) as u32, &mut ret) };
        if st == STATUS_INFO_LENGTH_MISMATCH || st == STATUS_BUFFER_TOO_SMALL || st == STATUS_BUFFER_OVERFLOW {
            buf.resize((ret as usize).max(buf.len() * 16) / 8 + 64, 0);
            continue;
        }
        ok = st >= 0;
        break;
    }
    let mut out = Vec::new();
    if !ok {
        return out;
    }
    unsafe {
        let base = buf.as_ptr() as *const u8;
        let total = buf.len() * 8;
        let n = std::ptr::read_unaligned(base as *const u32) as usize;
        let mut off = 8usize; // aligned to pointer size
        for i in 0..n.min(512) {
            if off + 104 > total {
                break;
            }
            let p = base.add(off);
            let us = std::ptr::read_unaligned(p as *const UnicodeString);
            let objects = std::ptr::read_unaligned(p.add(16) as *const u32);
            let handles = std::ptr::read_unaligned(p.add(20) as *const u32);
            let peak_objects = std::ptr::read_unaligned(p.add(40) as *const u32);
            let peak_handles = std::ptr::read_unaligned(p.add(44) as *const u32);
            let type_index = *p.add(90);
            let name = if !us.buffer.is_null() && us.length > 0 {
                String::from_utf16_lossy(std::slice::from_raw_parts(us.buffer, (us.length / 2) as usize))
            } else {
                format!("Type{}", i + 2)
            };
            out.push(ObjectType {
                index: if type_index == 0 { (i + 2) as u16 } else { type_index as u16 },
                name,
                objects,
                handles,
                peak_objects,
                peak_handles,
            });
            let name_len = ((us.max_length as usize) + 7) & !7;
            off += 104 + name_len;
        }
    }
    out
}

/// Kernel object name of a handle owned by this process.
pub fn object_name(handle: *mut c_void) -> Option<String> {
    let mut buf: Vec<u64> = vec![0; 256];
    for _ in 0..3 {
        let mut ret = 0u32;
        let st = unsafe { NtQueryObject(handle, 1, buf.as_mut_ptr() as *mut c_void, (buf.len() * 8) as u32, &mut ret) };
        if st == STATUS_INFO_LENGTH_MISMATCH || st == STATUS_BUFFER_OVERFLOW || st == STATUS_BUFFER_TOO_SMALL {
            buf.resize((ret as usize / 8) + 16, 0);
            continue;
        }
        if st < 0 {
            return None;
        }
        unsafe {
            let us = std::ptr::read_unaligned(buf.as_ptr() as *const UnicodeString);
            if us.buffer.is_null() || us.length == 0 {
                return Some(String::new());
            }
            return Some(String::from_utf16_lossy(std::slice::from_raw_parts(us.buffer, (us.length / 2) as usize)));
        }
    }
    None
}

pub fn suspend(handle: *mut c_void) -> bool {
    unsafe { NtSuspendProcess(handle) >= 0 }
}

pub fn resume(handle: *mut c_void) -> bool {
    unsafe { NtResumeProcess(handle) >= 0 }
}

pub fn process_command_line(handle: *mut c_void) -> Option<String> {
    let mut buf: Vec<u64> = vec![0; 512];
    for _ in 0..3 {
        let mut ret = 0u32;
        let st = unsafe { NtQueryInformationProcess(handle, 60, buf.as_mut_ptr() as *mut c_void, (buf.len() * 8) as u32, &mut ret) };
        if st == STATUS_INFO_LENGTH_MISMATCH || st == STATUS_BUFFER_OVERFLOW || st == STATUS_BUFFER_TOO_SMALL {
            buf.resize(ret as usize / 8 + 16, 0);
            continue;
        }
        if st < 0 {
            return None;
        }
        unsafe {
            let us = std::ptr::read_unaligned(buf.as_ptr() as *const UnicodeString);
            if us.buffer.is_null() {
                return Some(String::new());
            }
            return Some(String::from_utf16_lossy(std::slice::from_raw_parts(us.buffer, (us.length / 2) as usize)));
        }
    }
    None
}

pub fn process_u32_info(handle: *mut c_void, class: u32) -> Option<u32> {
    let mut v = 0u32;
    let mut ret = 0u32;
    let st = unsafe { NtQueryInformationProcess(handle, class, &mut v as *mut u32 as *mut c_void, 4, &mut ret) };
    if st < 0 {
        None
    } else {
        Some(v)
    }
}

pub fn process_u8_info(handle: *mut c_void, class: u32) -> Option<u8> {
    let mut v = 0u8;
    let mut ret = 0u32;
    let st = unsafe { NtQueryInformationProcess(handle, class, &mut v as *mut u8 as *mut c_void, 1, &mut ret) };
    if st < 0 {
        None
    } else {
        Some(v)
    }
}

pub fn thread_win32_start(handle: *mut c_void) -> Option<usize> {
    let mut v = 0usize;
    let mut ret = 0u32;
    let st = unsafe {
        NtQueryInformationThread(handle, 9, &mut v as *mut usize as *mut c_void, std::mem::size_of::<usize>() as u32, &mut ret)
    };
    if st < 0 {
        None
    } else {
        Some(v)
    }
}

pub fn thread_state_name(s: u32) -> &'static str {
    match s {
        0 => "Initialized",
        1 => "Ready",
        2 => "Running",
        3 => "Standby",
        4 => "Terminated",
        5 => "Waiting",
        6 => "Transition",
        7 => "Deferred ready",
        8 => "Gate wait",
        9 => "Waiting for process in swap",
        _ => "Unknown",
    }
}

pub fn wait_reason_name(r: u32) -> &'static str {
    const N: [&str; 41] = [
        "Executive", "FreePage", "PageIn", "PoolAllocation", "DelayExecution", "Suspended", "UserRequest",
        "WrExecutive", "WrFreePage", "WrPageIn", "WrPoolAllocation", "WrDelayExecution", "WrSuspended",
        "WrUserRequest", "WrEventPair", "WrQueue", "WrLpcReceive", "WrLpcReply", "WrVirtualMemory",
        "WrPageOut", "WrRendezvous", "WrKeyedEvent", "WrTerminated", "WrProcessInSwap", "WrCpuRateControl",
        "WrCalloutStack", "WrKernel", "WrResource", "WrPushLock", "WrMutex", "WrQuantumEnd", "WrDispatchInt",
        "WrPreempted", "WrYieldExecution", "WrFastMutex", "WrGuardedMutex", "WrRundown", "WrAlertByThreadId",
        "WrDeferredPreempt", "WrPhysicalFault", "WrIoRing",
    ];
    N.get(r as usize).copied().unwrap_or("Other")
}
