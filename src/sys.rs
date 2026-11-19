//! Win32 helpers: per-process details (path, user, token, modules, memory map,
//! handles, connections), process actions, and system-wide counters.

#![allow(clippy::missing_safety_doc)]

use crate::nt;
use crate::util::{from_wide, from_wide_ptr, wide};
use std::ffi::c_void;
use windows_sys::Win32::Foundation::{CloseHandle, DuplicateHandle, LocalFree, HANDLE, LUID};
use windows_sys::Win32::Security::{
    AdjustTokenPrivileges, GetSidSubAuthority, GetSidSubAuthorityCount, GetTokenInformation, LookupAccountSidW,
    LookupPrivilegeNameW, LookupPrivilegeValueW, LUID_AND_ATTRIBUTES, TOKEN_PRIVILEGES,
};
use windows_sys::Win32::System::Threading::{
    GetCurrentProcess, GetPriorityClass, GetProcessAffinityMask, GetProcessId, GetThreadDescription, GetThreadId,
    OpenProcess, OpenProcessToken, OpenThread, QueryFullProcessImageNameW, ResumeThread, SetPriorityClass,
    SetProcessAffinityMask, SetProcessInformation, SuspendThread, TerminateProcess, GetGuiResources,
};

pub const PROCESS_TERMINATE: u32 = 0x0001;
pub const PROCESS_VM_READ: u32 = 0x0010;
pub const PROCESS_DUP_HANDLE: u32 = 0x0040;
pub const PROCESS_SET_INFORMATION: u32 = 0x0200;
pub const PROCESS_QUERY_INFORMATION: u32 = 0x0400;
pub const PROCESS_SUSPEND_RESUME: u32 = 0x0800;
pub const PROCESS_QUERY_LIMITED_INFORMATION: u32 = 0x1000;
const TOKEN_QUERY: u32 = 0x0008;
const TOKEN_ADJUST_PRIVILEGES: u32 = 0x0020;
const THREAD_SUSPEND_RESUME: u32 = 0x0002;
const THREAD_QUERY_INFORMATION: u32 = 0x0040;
const THREAD_QUERY_LIMITED_INFORMATION: u32 = 0x0800;

/// Owned kernel handle closed on drop.
pub struct Handle(pub HANDLE);
impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() && self.0 as isize != -1 {
            unsafe { CloseHandle(self.0) };
        }
    }
}

pub fn open_process(pid: u32, access: u32) -> Option<Handle> {
    let h = unsafe { OpenProcess(access, 0, pid) };
    if h.is_null() {
        None
    } else {
        Some(Handle(h))
    }
}

fn last_error() -> String {
    let e = std::io::Error::last_os_error();
    e.to_string()
}

// ---------------------------------------------------------------- elevation

pub fn is_elevated() -> bool {
    unsafe {
        let mut tok: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut tok) == 0 {
            return false;
        }
        let tok = Handle(tok);
        let mut elev = 0u32;
        let mut ret = 0u32;
        let ok = GetTokenInformation(tok.0, 20, &mut elev as *mut u32 as *mut c_void, 4, &mut ret);
        ok != 0 && elev != 0
    }
}

pub fn enable_debug_privilege() -> bool {
    unsafe {
        let mut tok: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(GetCurrentProcess(), TOKEN_ADJUST_PRIVILEGES | TOKEN_QUERY, &mut tok) == 0 {
            return false;
        }
        let tok = Handle(tok);
        let mut luid: LUID = std::mem::zeroed();
        let name = wide("SeDebugPrivilege");
        if LookupPrivilegeValueW(std::ptr::null(), name.as_ptr(), &mut luid) == 0 {
            return false;
        }
        let tp = TOKEN_PRIVILEGES { PrivilegeCount: 1, Privileges: [LUID_AND_ATTRIBUTES { Luid: luid, Attributes: 2 }] };
        AdjustTokenPrivileges(tok.0, 0, &tp, 0, std::ptr::null_mut(), std::ptr::null_mut()) != 0
    }
}

/// Start this program again through the UAC prompt with the given arguments.
pub fn relaunch_as_admin(args: &str) -> bool {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let Ok(exe) = std::env::current_exe() else { return false };
    let exe_w = wide(&exe.to_string_lossy());
    let verb = wide("runas");
    let params = wide(args);
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), verb.as_ptr(), exe_w.as_ptr(), params.as_ptr(), std::ptr::null(), 1) };
    (r as isize) > 32
}

pub fn shell_open(verb: &str, file: &str, params: &str) -> bool {
    use windows_sys::Win32::UI::Shell::ShellExecuteW;
    let v = wide(verb);
    let f = wide(file);
    let p = wide(params);
    let r = unsafe { ShellExecuteW(std::ptr::null_mut(), v.as_ptr(), f.as_ptr(), p.as_ptr(), std::ptr::null(), 1) };
    (r as isize) > 32
}

// ---------------------------------------------------------------- static process info

#[derive(Clone, Debug, Default)]
pub struct ProcStatic {
    pub path: String,
    pub cmdline: String,
    pub user: String,
    pub sid: String,
    pub integrity: String,
    pub elevated: Option<bool>,
    pub arch: String,
    pub description: String,
    pub company: String,
    pub version: String,
    pub access_denied: bool,
}

pub fn image_path(h: HANDLE) -> Option<String> {
    let mut buf = vec![0u16; 1024];
    let mut len = buf.len() as u32;
    let ok = unsafe { QueryFullProcessImageNameW(h, 0, buf.as_mut_ptr(), &mut len) };
    if ok == 0 {
        None
    } else {
        Some(String::from_utf16_lossy(&buf[..len as usize]))
    }
}

unsafe fn sid_to_name(sid: *mut c_void) -> String {
    let mut name = vec![0u16; 256];
    let mut dom = vec![0u16; 256];
    let mut nl = 256u32;
    let mut dl = 256u32;
    let mut use_ = 0i32;
    if LookupAccountSidW(std::ptr::null(), sid, name.as_mut_ptr(), &mut nl, dom.as_mut_ptr(), &mut dl, &mut use_) != 0 {
        let n = from_wide(&name);
        let d = from_wide(&dom);
        if d.is_empty() {
            n
        } else {
            format!("{d}\\{n}")
        }
    } else {
        sid_string(sid)
    }
}

unsafe fn sid_string(sid: *mut c_void) -> String {
    use windows_sys::Win32::Security::Authorization::ConvertSidToStringSidW;
    let mut p: *mut u16 = std::ptr::null_mut();
    if ConvertSidToStringSidW(sid, &mut p) != 0 {
        let s = from_wide_ptr(p);
        LocalFree(p as *mut c_void);
        s
    } else {
        String::new()
    }
}

unsafe fn token_info(tok: HANDLE, class: i32) -> Option<Vec<u64>> {
    let mut ret = 0u32;
    GetTokenInformation(tok, class, std::ptr::null_mut(), 0, &mut ret);
    if ret == 0 {
        return None;
    }
    let mut buf: Vec<u64> = vec![0; ret as usize / 8 + 2];
    if GetTokenInformation(tok, class, buf.as_mut_ptr() as *mut c_void, (buf.len() * 8) as u32, &mut ret) == 0 {
        return None;
    }
    Some(buf)
}

fn integrity_name(rid: u32) -> &'static str {
    match rid {
        0x0000 => "Untrusted",
        0x1000 => "Low",
        0x2000 => "Medium",
        0x2100 => "Medium Plus",
        0x3000 => "High",
        0x4000 => "System",
        0x5000 => "Protected",
        _ if rid < 0x1000 => "Untrusted",
        _ if rid < 0x2000 => "Low",
        _ if rid < 0x3000 => "Medium",
        _ if rid < 0x4000 => "High",
        _ => "System",
    }
}

pub fn query_static(pid: u32) -> ProcStatic {
    let mut s = ProcStatic::default();
    if pid == 0 || pid == 4 {
        s.user = "NT AUTHORITY\\SYSTEM".into();
        s.integrity = "System".into();
        s.description = if pid == 0 { "Percentage of time the processors are idle".into() } else { "NT Kernel & System".into() };
        s.company = "Microsoft Corporation".into();
        return s;
    }
    let Some(h) = open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION) else {
        s.access_denied = true;
        return s;
    };
    s.path = image_path(h.0).unwrap_or_default();
    s.cmdline = nt::process_command_line(h.0).unwrap_or_default();
    unsafe {
        let mut tok: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(h.0, TOKEN_QUERY, &mut tok) != 0 {
            let tok = Handle(tok);
            if let Some(b) = token_info(tok.0, 1) {
                let sid = std::ptr::read_unaligned(b.as_ptr() as *const *mut c_void);
                s.user = sid_to_name(sid);
                s.sid = sid_string(sid);
            }
            if let Some(b) = token_info(tok.0, 25) {
                let sid = std::ptr::read_unaligned(b.as_ptr() as *const *mut c_void);
                let cnt = *GetSidSubAuthorityCount(sid);
                if cnt > 0 {
                    let rid = *GetSidSubAuthority(sid, (cnt - 1) as u32);
                    s.integrity = integrity_name(rid).into();
                }
            }
            let mut elev = 0u32;
            let mut ret = 0u32;
            if GetTokenInformation(tok.0, 20, &mut elev as *mut u32 as *mut c_void, 4, &mut ret) != 0 {
                s.elevated = Some(elev != 0);
            }
        }
        s.arch = process_arch(h.0);
    }
    if !s.path.is_empty() {
        let (d, c, v) = file_version_info(&s.path);
        s.description = d;
        s.company = c;
        s.version = v;
    }
    s
}

unsafe fn process_arch(h: HANDLE) -> String {
    use windows_sys::Win32::System::Threading::IsWow64Process2;
    let mut pm: u16 = 0;
    let mut nm: u16 = 0;
    if IsWow64Process2(h, &mut pm, &mut nm) == 0 {
        return String::new();
    }
    let m = if pm == 0 { nm } else { pm };
    match m {
        0x014c => "x86 (32-bit)".into(),
        0x8664 => "x64".into(),
        0xAA64 => "ARM64".into(),
        0x01c4 => "ARM".into(),
        _ => format!("0x{m:04x}"),
    }
}

pub fn file_version_info(path: &str) -> (String, String, String) {
    use windows_sys::Win32::Storage::FileSystem::{GetFileVersionInfoSizeW, GetFileVersionInfoW, VerQueryValueW};
    let pw = wide(path);
    unsafe {
        let mut dummy = 0u32;
        let size = GetFileVersionInfoSizeW(pw.as_ptr(), &mut dummy);
        if size == 0 {
            return Default::default();
        }
        let mut data = vec![0u8; size as usize];
        if GetFileVersionInfoW(pw.as_ptr(), 0, size, data.as_mut_ptr() as *mut c_void) == 0 {
            return Default::default();
        }
        let mut p: *mut c_void = std::ptr::null_mut();
        let mut len = 0u32;
        let tr = wide("\\VarFileInfo\\Translation");
        let mut prefixes = vec![];
        if VerQueryValueW(data.as_ptr() as *const c_void, tr.as_ptr(), &mut p, &mut len) != 0 && len >= 4 {
            let lang = std::ptr::read_unaligned(p as *const u16);
            let cp = std::ptr::read_unaligned((p as *const u16).add(1));
            prefixes.push(format!("{lang:04x}{cp:04x}"));
        }
        prefixes.push("040904b0".into());
        prefixes.push("040904e4".into());
        let get = |key: &str| -> String {
            for pre in &prefixes {
                let q = wide(&format!("\\StringFileInfo\\{pre}\\{key}"));
                let mut p: *mut c_void = std::ptr::null_mut();
                let mut len = 0u32;
                if VerQueryValueW(data.as_ptr() as *const c_void, q.as_ptr(), &mut p, &mut len) != 0 && len > 0 {
                    let s = from_wide_ptr(p as *const u16);
                    if !s.trim().is_empty() {
                        return s.trim().to_string();
                    }
                }
            }
            String::new()
        };
        (get("FileDescription"), get("CompanyName"), get("FileVersion"))
    }
}

// ---------------------------------------------------------------- dynamic details for one process

#[derive(Clone, Debug, Default)]
pub struct ProcDynamic {
    pub priority_class: String,
    pub affinity: u64,
    pub system_affinity: u64,
    pub gdi_objects: u32,
    pub user_objects: u32,
    pub gdi_peak: u32,
    pub user_peak: u32,
    pub io_priority: String,
    pub page_priority: String,
    pub protection: String,
    pub critical: Option<bool>,
    pub efficiency_mode: Option<bool>,
    pub privileges: Vec<(String, String)>,
    pub groups: Vec<(String, String)>,
    pub token_type: String,
    pub virtualization: Option<bool>,
    pub app_container: Option<bool>,
}

pub fn priority_name(c: u32) -> &'static str {
    match c {
        0x40 => "Idle",
        0x4000 => "Below normal",
        0x20 => "Normal",
        0x8000 => "Above normal",
        0x80 => "High",
        0x100 => "Realtime",
        _ => "Unknown",
    }
}

pub fn query_dynamic(pid: u32) -> ProcDynamic {
    let mut d = ProcDynamic::default();
    let h = open_process(pid, PROCESS_QUERY_INFORMATION).or_else(|| open_process(pid, PROCESS_QUERY_LIMITED_INFORMATION));
    let Some(h) = h else { return d };
    unsafe {
        d.priority_class = priority_name(GetPriorityClass(h.0)).into();
        let mut pa = 0usize;
        let mut sa = 0usize;
        if GetProcessAffinityMask(h.0, &mut pa, &mut sa) != 0 {
            d.affinity = pa as u64;
            d.system_affinity = sa as u64;
        }
        d.gdi_objects = GetGuiResources(h.0, 0);
        d.user_objects = GetGuiResources(h.0, 1);
        d.gdi_peak = GetGuiResources(h.0, 2);
        d.user_peak = GetGuiResources(h.0, 4);
        if let Some(v) = nt::process_u32_info(h.0, 33) {
            d.io_priority = ["Very low", "Low", "Normal", "High", "Critical"].get(v as usize).unwrap_or(&"?").to_string();
        }
        if let Some(v) = nt::process_u32_info(h.0, 39) {
            d.page_priority = format!("{v} {}", ["", "(Very low)", "(Low)", "(Medium)", "(Below normal)", "(Normal)"].get(v as usize).unwrap_or(&""));
        }
        if let Some(v) = nt::process_u32_info(h.0, 29) {
            d.critical = Some(v != 0);
        }
        if let Some(p) = nt::process_u8_info(h.0, 61) {
            let ty = p & 7;
            let signer = p >> 4;
            d.protection = if ty == 0 {
                "None".into()
            } else {
                let t = match ty {
                    1 => "Protected Light (PPL)",
                    2 => "Protected (PP)",
                    _ => "Protected",
                };
                let s = match signer {
                    1 => "Authenticode",
                    2 => "CodeGen",
                    3 => "Antimalware",
                    4 => "Lsa",
                    5 => "Windows",
                    6 => "WinTcb",
                    7 => "WinSystem",
                    8 => "App",
                    _ => "",
                };
                format!("{t} {s}")
            };
        }
        // Power throttling (efficiency mode)
        #[repr(C)]
        struct Pts {
            version: u32,
            control: u32,
            state: u32,
        }
        let mut pts = Pts { version: 1, control: 0, state: 0 };
        use windows_sys::Win32::System::Threading::GetProcessInformation;
        if GetProcessInformation(h.0, 4, &mut pts as *mut Pts as *mut c_void, 12) != 0 {
            d.efficiency_mode = Some(pts.control & 1 != 0 && pts.state & 1 != 0);
        }
        let mut tok: HANDLE = std::ptr::null_mut();
        if OpenProcessToken(h.0, TOKEN_QUERY, &mut tok) != 0 {
            let tok = Handle(tok);
            if let Some(b) = token_info(tok.0, 3) {
                let base = b.as_ptr() as *const u8;
                let n = std::ptr::read_unaligned(base as *const u32) as usize;
                for i in 0..n.min(128) {
                    let e = base.add(4 + i * 12);
                    let luid = std::ptr::read_unaligned(e as *const LUID);
                    let attrs = std::ptr::read_unaligned(e.add(8) as *const u32);
                    let mut name = vec![0u16; 128];
                    let mut nl = 128u32;
                    let nm = if LookupPrivilegeNameW(std::ptr::null(), &luid, name.as_mut_ptr(), &mut nl) != 0 {
                        from_wide(&name)
                    } else {
                        format!("LUID {}", luid.LowPart)
                    };
                    let st = if attrs & 2 != 0 {
                        if attrs & 1 != 0 { "Enabled (default)" } else { "Enabled" }
                    } else {
                        "Disabled"
                    };
                    d.privileges.push((nm, st.into()));
                }
            }
            if let Some(b) = token_info(tok.0, 2) {
                let base = b.as_ptr() as *const u8;
                let n = std::ptr::read_unaligned(base as *const u32) as usize;
                for i in 0..n.min(128) {
                    let e = base.add(8 + i * 16);
                    let sid = std::ptr::read_unaligned(e as *const *mut c_void);
                    let attrs = std::ptr::read_unaligned(e.add(8) as *const u32);
                    let mut flags = vec![];
                    if attrs & 0x4 != 0 { flags.push("Enabled"); }
                    if attrs & 0x8 != 0 { flags.push("Owner"); }
                    if attrs & 0x10 != 0 { flags.push("Deny only"); }
                    if attrs & 0x20 != 0 { flags.push("Integrity"); }
                    if attrs & 0xC000_0000 == 0xC000_0000 { flags.push("Logon id"); }
                    d.groups.push((sid_to_name(sid), flags.join(", ")));
                }
            }
            let mut v = 0u32;
            let mut ret = 0u32;
            if GetTokenInformation(tok.0, 18, &mut v as *mut u32 as *mut c_void, 4, &mut ret) != 0 {
                d.token_type = match v {
                    1 => "Default (UAC off or built-in)",
                    2 => "Full (elevated)",
                    3 => "Limited (filtered)",
                    _ => "?",
                }
                .into();
            }
            if GetTokenInformation(tok.0, 24, &mut v as *mut u32 as *mut c_void, 4, &mut ret) != 0 {
                d.virtualization = Some(v != 0);
            }
            if GetTokenInformation(tok.0, 29, &mut v as *mut u32 as *mut c_void, 4, &mut ret) != 0 {
                d.app_container = Some(v != 0);
            }
        }
    }
    d
}

// ---------------------------------------------------------------- modules

#[derive(Clone, Debug, Default)]
pub struct Module {
    pub name: String,
    pub path: String,
    pub base: usize,
    pub size: u32,
    pub description: String,
    pub company: String,
    pub version: String,
}

pub fn modules(pid: u32) -> Result<Vec<Module>, String> {
    use windows_sys::Win32::System::Diagnostics::ToolHelp::{CreateToolhelp32Snapshot, Module32FirstW, Module32NextW, MODULEENTRY32W};
    unsafe {
        let snap = CreateToolhelp32Snapshot(0x8 | 0x10, pid);
        if snap as isize == -1 {
            return Err(format!("Could not read modules: {}", last_error()));
        }
        let snap = Handle(snap);
        let mut me: MODULEENTRY32W = std::mem::zeroed();
        me.dwSize = std::mem::size_of::<MODULEENTRY32W>() as u32;
        let mut out = vec![];
        if Module32FirstW(snap.0, &mut me) != 0 {
            loop {
                out.push(Module {
                    name: from_wide(&me.szModule),
                    path: from_wide(&me.szExePath),
                    base: me.modBaseAddr as usize,
                    size: me.modBaseSize,
                    ..Default::default()
                });
                if Module32NextW(snap.0, &mut me) == 0 {
                    break;
                }
            }
        }
        out.sort_by_key(|m| m.base);
        Ok(out)
    }
}

pub fn resolve_address(mods: &[Module], addr: usize) -> String {
    if addr == 0 {
        return "-".into();
    }
    for m in mods {
        if addr >= m.base && addr < m.base + m.size as usize {
            return format!("{}+0x{:x}", m.name, addr - m.base);
        }
    }
    format!("0x{addr:x}")
}

// ---------------------------------------------------------------- memory map

#[derive(Clone, Debug, Default)]
pub struct Region {
    pub base: usize,
    pub alloc_base: usize,
    pub size: usize,
    pub state: u32,
    pub protect: u32,
    pub alloc_protect: u32,
    pub kind: u32,
    pub mapped: String,
}

pub fn protect_name(p: u32) -> String {
    if p == 0 {
        return String::new();
    }
    let base = match p & 0xFF {
        0x01 => "NA",
        0x02 => "R",
        0x04 => "RW",
        0x08 => "WC",
        0x10 => "X",
        0x20 => "RX",
        0x40 => "RWX",
        0x80 => "WCX",
        _ => "?",
    };
    let mut s = base.to_string();
    if p & 0x100 != 0 {
        s.push_str("+G");
    }
    if p & 0x200 != 0 {
        s.push_str("+NC");
    }
    if p & 0x400 != 0 {
        s.push_str("+WCB");
    }
    s
}

pub fn state_name(s: u32) -> &'static str {
    match s {
        0x1000 => "Commit",
        0x2000 => "Reserve",
        0x10000 => "Free",
        _ => "?",
    }
}

pub fn kind_name(t: u32) -> &'static str {
    match t {
        0x1000000 => "Image",
        0x40000 => "Mapped",
        0x20000 => "Private",
        _ => "",
    }
}

pub fn memory_map(pid: u32) -> Result<Vec<Region>, String> {
    use windows_sys::Win32::System::Memory::{VirtualQueryEx, MEMORY_BASIC_INFORMATION};
    use windows_sys::Win32::System::ProcessStatus::GetMappedFileNameW;
    let h = open_process(pid, PROCESS_QUERY_INFORMATION | PROCESS_VM_READ)
        .or_else(|| open_process(pid, PROCESS_QUERY_INFORMATION))
        .ok_or_else(|| format!("Access denied: {}", last_error()))?;
    let mut out = vec![];
    let mut addr: usize = 0;
    unsafe {
        loop {
            let mut mbi: MEMORY_BASIC_INFORMATION = std::mem::zeroed();
            let r = VirtualQueryEx(h.0, addr as *const c_void, &mut mbi, std::mem::size_of::<MEMORY_BASIC_INFORMATION>());
            if r == 0 {
                break;
            }
            let mut mapped = String::new();
            if mbi.Type == 0x1000000 || mbi.Type == 0x40000 {
                let mut buf = vec![0u16; 512];
                let n = GetMappedFileNameW(h.0, mbi.BaseAddress, buf.as_mut_ptr(), 512);
                if n > 0 {
                    mapped = String::from_utf16_lossy(&buf[..n as usize]);
                }
            }
            if mbi.State != 0x10000 {
                out.push(Region {
                    base: mbi.BaseAddress as usize,
                    alloc_base: mbi.AllocationBase as usize,
                    size: mbi.RegionSize,
                    state: mbi.State,
                    protect: mbi.Protect,
                    alloc_protect: mbi.AllocationProtect,
                    kind: mbi.Type,
                    mapped,
                });
            }
            let next = (mbi.BaseAddress as usize).wrapping_add(mbi.RegionSize);
            if next <= addr || out.len() > 60000 {
                break;
            }
            addr = next;
        }
    }
    Ok(out)
}

// ---------------------------------------------------------------- handles

#[derive(Clone, Debug, Default)]
pub struct HandleInfo {
    pub handle: usize,
    pub type_name: String,
    pub name: String,
    pub access: u32,
    pub object: usize,
    pub attributes: u32,
}

pub fn handles(pid: u32, types: &[nt::ObjectType]) -> Result<Vec<HandleInfo>, String> {
    use windows_sys::Win32::Storage::FileSystem::{GetFileType, GetFinalPathNameByHandleW};
    let raw = nt::query_handles(Some(pid));
    if raw.is_empty() {
        return Err("No handles could be read (this needs administrator rights for most processes).".into());
    }
    let tname = |i: u16| -> String {
        types.iter().find(|t| t.index == i).map(|t| t.name.clone()).unwrap_or_else(|| format!("Type {i}"))
    };
    let src = open_process(pid, PROCESS_DUP_HANDLE);
    let me = unsafe { GetCurrentProcess() };
    let mut out = Vec::with_capacity(raw.len());
    for r in raw {
        let type_name = tname(r.type_index);
        let mut name = String::new();
        if let Some(src) = &src {
            unsafe {
                let mut dup: HANDLE = std::ptr::null_mut();
                if DuplicateHandle(src.0, r.handle as HANDLE, me, &mut dup, 0, 0, 0x2) != 0 {
                    let dup = Handle(dup);
                    match type_name.as_str() {
                        "File" => {
                            // Only ask disk files for a name: pipes can block forever.
                            if GetFileType(dup.0) == 1 {
                                let mut buf = vec![0u16; 1024];
                                let n = GetFinalPathNameByHandleW(dup.0, buf.as_mut_ptr(), 1024, 0);
                                if n > 0 && (n as usize) < buf.len() {
                                    name = String::from_utf16_lossy(&buf[..n as usize]).trim_start_matches("\\\\?\\").to_string();
                                }
                            } else {
                                name = "(pipe or device)".into();
                            }
                        }
                        "Process" => {
                            let p = GetProcessId(dup.0);
                            name = format!("Process {p}");
                        }
                        "Thread" => {
                            let t = GetThreadId(dup.0);
                            name = format!("Thread {t}");
                        }
                        "Key" | "Event" | "Mutant" | "Section" | "Semaphore" | "Directory" | "SymbolicLink" | "ALPC Port"
                        | "Timer" | "Desktop" | "WindowStation" | "Job" | "Session" | "Partition" | "Callback" | "IRTimer"
                        | "WaitCompletionPacket" | "IoCompletion" | "TpWorkerFactory" | "EtwRegistration" | "Token" => {
                            if type_name != "Token" {
                                name = nt::object_name(dup.0).unwrap_or_default();
                            }
                        }
                        _ => {}
                    }
                }
            }
        }
        out.push(HandleInfo { handle: r.handle, type_name, name, access: r.access, object: r.object, attributes: r.attributes });
    }
    Ok(out)
}

// ---------------------------------------------------------------- threads (per thread actions/details)

pub fn thread_extra(tid: u32) -> (Option<usize>, String) {
    unsafe {
        let mut h = OpenThread(THREAD_QUERY_INFORMATION, 0, tid);
        let full = !h.is_null();
        if h.is_null() {
            h = OpenThread(THREAD_QUERY_LIMITED_INFORMATION, 0, tid);
        }
        if h.is_null() {
            return (None, String::new());
        }
        let h = Handle(h);
        let start = if full { nt::thread_win32_start(h.0) } else { None };
        let mut p: *mut u16 = std::ptr::null_mut();
        let mut desc = String::new();
        if GetThreadDescription(h.0, &mut p) >= 0 && !p.is_null() {
            desc = from_wide_ptr(p);
            LocalFree(p as *mut c_void);
        }
        (start, desc)
    }
}

pub fn suspend_thread(tid: u32, suspend: bool) -> Result<(), String> {
    unsafe {
        let h = OpenThread(THREAD_SUSPEND_RESUME, 0, tid);
        if h.is_null() {
            return Err(last_error());
        }
        let h = Handle(h);
        let r = if suspend { SuspendThread(h.0) } else { ResumeThread(h.0) };
        if r == u32::MAX {
            Err(last_error())
        } else {
            Ok(())
        }
    }
}

// ---------------------------------------------------------------- actions

pub fn terminate(pid: u32) -> Result<(), String> {
    let h = open_process(pid, PROCESS_TERMINATE).ok_or_else(last_error)?;
    if unsafe { TerminateProcess(h.0, 1) } == 0 {
        return Err(last_error());
    }
    Ok(())
}

pub fn suspend_process(pid: u32, suspend: bool) -> Result<(), String> {
    let h = open_process(pid, PROCESS_SUSPEND_RESUME).ok_or_else(last_error)?;
    let ok = if suspend { nt::suspend(h.0) } else { nt::resume(h.0) };
    if ok {
        Ok(())
    } else {
        Err("The system refused the request".into())
    }
}

pub fn set_priority(pid: u32, class: u32) -> Result<(), String> {
    let h = open_process(pid, PROCESS_SET_INFORMATION).ok_or_else(last_error)?;
    if unsafe { SetPriorityClass(h.0, class) } == 0 {
        return Err(last_error());
    }
    Ok(())
}

pub fn set_affinity(pid: u32, mask: u64) -> Result<(), String> {
    let h = open_process(pid, PROCESS_SET_INFORMATION).ok_or_else(last_error)?;
    if unsafe { SetProcessAffinityMask(h.0, mask as usize) } == 0 {
        return Err(last_error());
    }
    Ok(())
}

pub fn set_efficiency(pid: u32, on: bool) -> Result<(), String> {
    #[repr(C)]
    struct Pts {
        version: u32,
        control: u32,
        state: u32,
    }
    let h = open_process(pid, PROCESS_SET_INFORMATION).ok_or_else(last_error)?;
    let pts = Pts { version: 1, control: 1, state: if on { 1 } else { 0 } };
    unsafe {
        if SetProcessInformation(h.0, 4, &pts as *const Pts as *const c_void, 12) == 0 {
            return Err(last_error());
        }
        SetPriorityClass(h.0, if on { 0x40 } else { 0x20 });
    }
    Ok(())
}

pub fn create_dump(pid: u32, path: &std::path::Path, full: bool) -> Result<(), String> {
    use windows_sys::Win32::Storage::FileSystem::CreateFileW;
    use windows_sys::Win32::System::Diagnostics::Debug::MiniDumpWriteDump;
    let h = open_process(pid, PROCESS_QUERY_INFORMATION | PROCESS_VM_READ).ok_or_else(last_error)?;
    let pw = wide(&path.to_string_lossy());
    unsafe {
        let f = CreateFileW(pw.as_ptr(), 0x4000_0000, 0, std::ptr::null(), 2, 0x80, std::ptr::null_mut());
        if f as isize == -1 {
            return Err(last_error());
        }
        let f = Handle(f);
        let kind = if full { 0x2 | 0x4 | 0x800 } else { 0x0 | 0x4 | 0x100 };
        if MiniDumpWriteDump(h.0, pid, f.0, kind, std::ptr::null(), std::ptr::null(), std::ptr::null()) == 0 {
            return Err(last_error());
        }
    }
    Ok(())
}

pub fn open_file_location(path: &str) {
    if path.is_empty() {
        return;
    }
    let _ = std::process::Command::new("explorer.exe").arg(format!("/select,{path}")).spawn();
}

// ---------------------------------------------------------------- network connections

#[derive(Clone, Debug)]
pub struct Conn {
    pub proto: &'static str,
    pub local: String,
    pub remote: String,
    pub state: &'static str,
    pub pid: u32,
}

fn tcp_state(s: u32) -> &'static str {
    match s {
        1 => "Closed",
        2 => "Listening",
        3 => "SYN sent",
        4 => "SYN received",
        5 => "Established",
        6 => "FIN wait 1",
        7 => "FIN wait 2",
        8 => "Close wait",
        9 => "Closing",
        10 => "Last ACK",
        11 => "Time wait",
        12 => "Delete TCB",
        _ => "?",
    }
}

fn port(p: u32) -> u16 {
    u16::from_be((p & 0xFFFF) as u16)
}

pub fn connections() -> Vec<Conn> {
    use std::net::{Ipv4Addr, Ipv6Addr};
    use windows_sys::Win32::NetworkManagement::IpHelper::{
        GetExtendedTcpTable, GetExtendedUdpTable, MIB_TCP6ROW_OWNER_PID, MIB_TCPROW_OWNER_PID, MIB_UDP6ROW_OWNER_PID,
        MIB_UDPROW_OWNER_PID,
    };
    let mut out = vec![];
    unsafe fn table(tcp: bool, af: u32) -> Vec<u8> {
        let mut size = 0u32;
        let class = if tcp { 5 } else { 1 };
        for _ in 0..4 {
            let mut buf = vec![0u8; size as usize + 1024];
            size = buf.len() as u32;
            let r = if tcp {
                GetExtendedTcpTable(buf.as_mut_ptr() as *mut c_void, &mut size, 0, af, class, 0)
            } else {
                GetExtendedUdpTable(buf.as_mut_ptr() as *mut c_void, &mut size, 0, af, class, 0)
            };
            if r == 0 {
                return buf;
            }
            if r != 122 {
                return vec![];
            }
        }
        vec![]
    }
    unsafe {
        let b = table(true, 2);
        if b.len() >= 4 {
            let n = std::ptr::read_unaligned(b.as_ptr() as *const u32) as usize;
            let rows = b.as_ptr().add(4) as *const MIB_TCPROW_OWNER_PID;
            for i in 0..n {
                let r = std::ptr::read_unaligned(rows.add(i));
                out.push(Conn {
                    proto: "TCP",
                    local: format!("{}:{}", Ipv4Addr::from(u32::from_be(r.dwLocalAddr)), port(r.dwLocalPort)),
                    remote: if r.dwState == 2 { String::new() } else { format!("{}:{}", Ipv4Addr::from(u32::from_be(r.dwRemoteAddr)), port(r.dwRemotePort)) },
                    state: tcp_state(r.dwState),
                    pid: r.dwOwningPid,
                });
            }
        }
        let b = table(true, 23);
        if b.len() >= 4 {
            let n = std::ptr::read_unaligned(b.as_ptr() as *const u32) as usize;
            let rows = b.as_ptr().add(4) as *const MIB_TCP6ROW_OWNER_PID;
            for i in 0..n {
                let r = std::ptr::read_unaligned(rows.add(i));
                out.push(Conn {
                    proto: "TCPv6",
                    local: format!("[{}]:{}", Ipv6Addr::from(r.ucLocalAddr), port(r.dwLocalPort)),
                    remote: if r.dwState == 2 { String::new() } else { format!("[{}]:{}", Ipv6Addr::from(r.ucRemoteAddr), port(r.dwRemotePort)) },
                    state: tcp_state(r.dwState),
                    pid: r.dwOwningPid,
                });
            }
        }
        let b = table(false, 2);
        if b.len() >= 4 {
            let n = std::ptr::read_unaligned(b.as_ptr() as *const u32) as usize;
            let rows = b.as_ptr().add(4) as *const MIB_UDPROW_OWNER_PID;
            for i in 0..n {
                let r = std::ptr::read_unaligned(rows.add(i));
                out.push(Conn {
                    proto: "UDP",
                    local: format!("{}:{}", Ipv4Addr::from(u32::from_be(r.dwLocalAddr)), port(r.dwLocalPort)),
                    remote: String::new(),
                    state: "",
                    pid: r.dwOwningPid,
                });
            }
        }
        let b = table(false, 23);
        if b.len() >= 4 {
            let n = std::ptr::read_unaligned(b.as_ptr() as *const u32) as usize;
            let rows = b.as_ptr().add(4) as *const MIB_UDP6ROW_OWNER_PID;
            for i in 0..n {
                let r = std::ptr::read_unaligned(rows.add(i));
                out.push(Conn {
                    proto: "UDPv6",
                    local: format!("[{}]:{}", Ipv6Addr::from(r.ucLocalAddr), port(r.dwLocalPort)),
                    remote: String::new(),
                    state: "",
                    pid: r.dwOwningPid,
                });
            }
        }
    }
    out
}

// ---------------------------------------------------------------- system-wide counters

#[derive(Clone, Debug, Default)]
pub struct MemStatus {
    pub load: u32,
    pub total: u64,
    pub avail: u64,
    pub commit_total: u64,
    pub commit_limit: u64,
    pub commit_peak: u64,
    pub cache: u64,
    pub kernel_total: u64,
    pub kernel_paged: u64,
    pub kernel_nonpaged: u64,
    pub handles: u32,
    pub processes: u32,
    pub threads: u32,
    pub page_size: u64,
}

pub fn mem_status() -> MemStatus {
    use windows_sys::Win32::System::ProcessStatus::{GetPerformanceInfo, PERFORMANCE_INFORMATION};
    use windows_sys::Win32::System::SystemInformation::{GlobalMemoryStatusEx, MEMORYSTATUSEX};
    let mut m = MemStatus::default();
    unsafe {
        let mut ms: MEMORYSTATUSEX = std::mem::zeroed();
        ms.dwLength = std::mem::size_of::<MEMORYSTATUSEX>() as u32;
        if GlobalMemoryStatusEx(&mut ms) != 0 {
            m.load = ms.dwMemoryLoad;
            m.total = ms.ullTotalPhys;
            m.avail = ms.ullAvailPhys;
        }
        let mut pi: PERFORMANCE_INFORMATION = std::mem::zeroed();
        pi.cb = std::mem::size_of::<PERFORMANCE_INFORMATION>() as u32;
        if GetPerformanceInfo(&mut pi, pi.cb) != 0 {
            let ps = pi.PageSize as u64;
            m.page_size = ps;
            m.commit_total = pi.CommitTotal as u64 * ps;
            m.commit_limit = pi.CommitLimit as u64 * ps;
            m.commit_peak = pi.CommitPeak as u64 * ps;
            m.cache = pi.SystemCache as u64 * ps;
            m.kernel_total = pi.KernelTotal as u64 * ps;
            m.kernel_paged = pi.KernelPaged as u64 * ps;
            m.kernel_nonpaged = pi.KernelNonpaged as u64 * ps;
            m.handles = pi.HandleCount;
            m.processes = pi.ProcessCount;
            m.threads = pi.ThreadCount;
        }
    }
    m
}

pub fn uptime_secs() -> u64 {
    unsafe { windows_sys::Win32::System::SystemInformation::GetTickCount64() / 1000 }
}

pub fn cpu_count() -> usize {
    use windows_sys::Win32::System::SystemInformation::{GetSystemInfo, SYSTEM_INFO};
    unsafe {
        let mut si: SYSTEM_INFO = std::mem::zeroed();
        GetSystemInfo(&mut si);
        (si.dwNumberOfProcessors as usize).max(1)
    }
}

#[derive(Clone, Copy, Debug, Default)]
#[repr(C)]
pub struct CpuPower {
    pub number: u32,
    pub max_mhz: u32,
    pub current_mhz: u32,
    pub mhz_limit: u32,
    pub max_idle_state: u32,
    pub current_idle_state: u32,
}

pub fn cpu_power(n: usize) -> Vec<CpuPower> {
    use windows_sys::Win32::System::Power::CallNtPowerInformation;
    let mut v = vec![CpuPower::default(); n];
    let st = unsafe {
        CallNtPowerInformation(11, std::ptr::null(), 0, v.as_mut_ptr() as *mut c_void, (n * std::mem::size_of::<CpuPower>()) as u32)
    };
    if st < 0 {
        return vec![];
    }
    v
}

pub fn reg_string(subkey: &str, value: &str) -> Option<String> {
    use windows_sys::Win32::System::Registry::{RegGetValueW, HKEY_LOCAL_MACHINE};
    let sk = wide(subkey);
    let vn = wide(value);
    let mut buf = vec![0u16; 512];
    let mut cb = (buf.len() * 2) as u32;
    let r = unsafe {
        RegGetValueW(HKEY_LOCAL_MACHINE, sk.as_ptr(), vn.as_ptr(), 0x2, std::ptr::null_mut(), buf.as_mut_ptr() as *mut c_void, &mut cb)
    };
    if r == 0 {
        Some(from_wide(&buf).trim().to_string())
    } else {
        None
    }
}

#[derive(Clone, Debug, Default)]
pub struct Volume {
    pub root: String,
    pub label: String,
    pub fs: String,
    pub total: u64,
    pub free: u64,
    pub kind: &'static str,
}

pub fn volumes() -> Vec<Volume> {
    use windows_sys::Win32::Storage::FileSystem::{GetDiskFreeSpaceExW, GetDriveTypeW, GetLogicalDriveStringsW, GetVolumeInformationW};
    let mut out = vec![];
    unsafe {
        let mut buf = vec![0u16; 512];
        let n = GetLogicalDriveStringsW(buf.len() as u32, buf.as_mut_ptr());
        let s = String::from_utf16_lossy(&buf[..n as usize]);
        for root in s.split('\0').filter(|r| !r.is_empty()) {
            let rw = wide(root);
            let t = GetDriveTypeW(rw.as_ptr());
            let kind = match t {
                2 => "Removable",
                3 => "Fixed",
                4 => "Network",
                5 => "CD/DVD",
                6 => "RAM disk",
                _ => "Unknown",
            };
            if t == 5 {
                out.push(Volume { root: root.into(), kind, ..Default::default() });
                continue;
            }
            let mut label = vec![0u16; 261];
            let mut fs = vec![0u16; 261];
            let mut serial = 0u32;
            let mut maxc = 0u32;
            let mut flags = 0u32;
            GetVolumeInformationW(rw.as_ptr(), label.as_mut_ptr(), 261, &mut serial, &mut maxc, &mut flags, fs.as_mut_ptr(), 261);
            let mut avail = 0u64;
            let mut total = 0u64;
            let mut free = 0u64;
            GetDiskFreeSpaceExW(rw.as_ptr(), &mut avail, &mut total, &mut free);
            out.push(Volume { root: root.into(), label: from_wide(&label), fs: from_wide(&fs), total, free, kind });
        }
    }
    out
}

#[derive(Clone, Debug, Default)]
pub struct IfRow {
    pub index: u32,
    pub alias: String,
    pub description: String,
    pub mac: String,
    pub if_type: u32,
    pub speed: u64,
    pub in_octets: u64,
    pub out_octets: u64,
    pub in_pkts: u64,
    pub out_pkts: u64,
    pub in_errors: u64,
    pub out_errors: u64,
    pub in_discards: u64,
    pub out_discards: u64,
    pub up: bool,
    pub mtu: u32,
    pub hardware: bool,
}

pub fn if_type_name(t: u32) -> &'static str {
    match t {
        6 => "Ethernet",
        71 => "Wi-Fi",
        24 => "Loopback",
        131 => "Tunnel",
        243 | 244 => "Mobile broadband",
        23 => "PPP",
        _ => "Other",
    }
}

pub fn interfaces() -> Vec<IfRow> {
    use windows_sys::Win32::NetworkManagement::IpHelper::{FreeMibTable, GetIfTable2, MIB_IF_TABLE2};
    let mut out: Vec<IfRow> = vec![];
    unsafe {
        let mut t: *mut MIB_IF_TABLE2 = std::ptr::null_mut();
        if GetIfTable2(&mut t) != 0 || t.is_null() {
            return out;
        }
        let n = (*t).NumEntries as usize;
        let rows = std::ptr::addr_of!((*t).Table) as *const windows_sys::Win32::NetworkManagement::IpHelper::MIB_IF_ROW2;
        for i in 0..n {
            let r = &*rows.add(i);
            let flags = r.InterfaceAndOperStatusFlags._bitfield;
            let hardware = flags & 1 != 0;
            let filter = flags & 2 != 0;
            if filter || r.Type == 24 {
                continue;
            }
            if !hardware && r.InOctets == 0 && r.OutOctets == 0 {
                continue;
            }
            let alias = from_wide(&r.Alias);
            if alias.contains('-') && (alias.contains("WFP") || alias.contains("QoS") || alias.contains("Filter")) {
                continue;
            }
            let ml = (r.PhysicalAddressLength as usize).min(8);
            let mac = r.PhysicalAddress[..ml].iter().map(|b| format!("{b:02X}")).collect::<Vec<_>>().join("-");
            if out.iter().any(|o| o.alias == alias) {
                continue;
            }
            out.push(IfRow {
                index: r.InterfaceIndex,
                alias,
                description: from_wide(&r.Description),
                mac,
                if_type: r.Type,
                speed: r.ReceiveLinkSpeed.max(r.TransmitLinkSpeed),
                in_octets: r.InOctets,
                out_octets: r.OutOctets,
                in_pkts: r.InUcastPkts + r.InNUcastPkts,
                out_pkts: r.OutUcastPkts + r.OutNUcastPkts,
                in_errors: r.InErrors,
                out_errors: r.OutErrors,
                in_discards: r.InDiscards,
                out_discards: r.OutDiscards,
                up: r.OperStatus == 1,
                mtu: r.Mtu,
                hardware,
            });
        }
        FreeMibTable(t as *const c_void);
    }
    out
}

#[derive(Clone, Debug, Default)]
pub struct KernelModule {
    pub name: String,
    pub path: String,
    pub base: usize,
}

pub fn kernel_modules() -> Vec<KernelModule> {
    use windows_sys::Win32::System::ProcessStatus::{EnumDeviceDrivers, GetDeviceDriverBaseNameW, GetDeviceDriverFileNameW};
    let mut out = vec![];
    unsafe {
        let mut bases: Vec<*mut c_void> = vec![std::ptr::null_mut(); 2048];
        let mut needed = 0u32;
        if EnumDeviceDrivers(bases.as_mut_ptr(), (bases.len() * 8) as u32, &mut needed) == 0 {
            return out;
        }
        let n = (needed as usize / 8).min(bases.len());
        for &b in &bases[..n] {
            let mut name = vec![0u16; 260];
            let mut path = vec![0u16; 520];
            GetDeviceDriverBaseNameW(b, name.as_mut_ptr(), 260);
            GetDeviceDriverFileNameW(b, path.as_mut_ptr(), 520);
            out.push(KernelModule { name: from_wide(&name), path: from_wide(&path), base: b as usize });
        }
    }
    out
}
