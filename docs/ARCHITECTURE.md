# Architecture

Advanced TaskManager is a single Windows executable with two threads that matter: the **sampler** and the **UI**. Everything else (PowerShell queries, dumps, script runs, ETW processing) runs on short-lived worker threads.

```
                ┌──────────────────────── UI thread (eframe/egui) ───────────────────────┐
                │  shell.rs  top bar · sidebar · status bar · palette · shortcuts          │
                │  pages     dashboard · processes · perf · manage · deep · insights · …  │
                │  widgets   live theme · charts · tables                                  │
                └───────▲──────────────────────────────┬──────────────────────────────────┘
                        │ lock, read, draw              │ Job::start(…)  (slow queries)
                        │                               ▼
      ┌─────────────────┴───────────┐        ┌─────────────────────────────┐
      │  Arc<Mutex<State>>           │        │ worker threads               │
      │  procs · history · timeline │        │ queries.rs (PowerShell/CIM)  │
      │  alerts · config · recorder │        │ sys.rs (modules, memory map, │
      └─────────────────▲───────────┘        │ handles, dumps) · scripts     │
                        │ lock once, merge    └─────────────────────────────┘
      ┌─────────────────┴──────────── sampler thread (collector.rs) ───────────────────────┐
      │ every interval:                                                                       │
      │   nt.rs   NtQuerySystemInformation(5) → all processes + threads                      │
      │           NtQuerySystemInformation(8) → per-CPU idle/kernel/user/DPC/interrupt       │
      │   sys.rs  GlobalMemoryStatusEx · GetPerformanceInfo · GetIfTable2 · CallNtPower…     │
      │           TCP/UDP owner tables (3 s) · volumes (15 s) · static info for new processes │
      │   pdh.rs  disks · GPU engines/memory · thermal zones · memory lists · system counters │
      │ then: rates & percentages → history rings → spike detection → recording → rules →    │
      │       process profiles → schedules → save config if changed → request repaint        │
      └──────────────────────────────────────────────────────────────────────────────────────┘

      etw.rs: when started from the Live system events page, a private real-time ETW session
              feeds a callback thread that decodes events with TDH into its own ring buffer.
```

## Data flow per sample

1. **Gather** (no lock): one native call returns every process with ~35 counters and every thread with ~10 counters. Static details for *new* processes (path, command line, user, integrity, architecture, version info) are read once and cached by `(pid, create_time)`, so PID reuse is handled.
2. **Merge** (lock held a few ms): compute CPU % from time deltas divided by `interval × logical CPUs`, I/O rates, page-fault rates, thread CPU and context-switch deltas, "suspended" (all threads waiting with reason *Suspended*), GPU per process (max over engine types of summed engine use, like Task Manager). Detect starts and exits → timeline and exited list.
3. **History**: system rings (3,600 samples) and per-process rings (300 samples).
4. **Analysis hooks**: spikes into the timeline; CSV row if recording; rules evaluated with per-(rule, process) "since" and cool-down timers; process profiles applied once per process instance; schedules every 20 s.

## Disk space analyzer

Scans run on their own thread (`disk.rs`). For an NTFS drive root in an elevated process the volume is opened directly, `FSCTL_GET_NTFS_VOLUME_DATA` gives the file-table location and record size, the table's own data runs are decoded, and every record is read sequentially (8 MB chunks), fixed up and parsed for name, parent, size, space on disk and dates. Parent → child links are built with a counting sort and walked from the root. Otherwise up to 12 threads share a folder queue and enumerate with `FindFirstFileExW` (basic info, large fetch), skipping junctions. Both feed the same builder, which adds folder totals bottom-up, sorts children by size and gathers file-type statistics. The treemap uses a squarified layout cached per view size.

## Uninstaller and clean-up

`cleanup.rs` reads installed programs natively from the three Uninstall registry locations (64-bit, 32-bit and per-user views) with `RegOpenKeyExW`/`RegEnumKeyExW`. Uninstallers are started through `ShellExecuteExW`, so they can show their own UAC prompt, and the app waits on the process handle. The leftover scan normalises the program, publisher and install-folder names and matches them against top-level registry keys (HKCU/HKLM `Software`, `WOW6432Node`), publisher sub-keys, services (Advanced), Program Files, ProgramData, AppData, the Start menu and the desktop; a short shared brand name must cover at least 80 % of the program name to count. `mark_shared` then unticks anything another installed program also uses. Removal exports each key with `reg export` into a dated backup folder with a manifest before deleting it, and sends files to the Recycle Bin with `SHFileOperationW`. Store apps use `Get-/Remove-AppxPackage`; browser extensions are read from Chromium `Extensions` folders (with `_locales` name lookup) and Firefox `extensions.json`. Junk categories are folder walks; the Recycle Bin uses `SHQueryRecycleBinW`/`SHEmptyRecycleBinW`.

## UI

- egui is immediate-mode: every frame the UI reads `State` and draws. The window repaints after each sample (the sampler calls `request_repaint`) and at least every 500 ms.
- `App` holds per-page UI state (selections, filters, jobs). Page functions are free functions `fn page(app: &mut App, ui: &mut Ui)`.
- **Standard vs Admin** is `App::mode`. Admin requires an elevated process; `sys::relaunch_as_admin` uses `ShellExecuteW("runas")`.
- **Preferences**: `config::UiPrefs` → `widgets::apply_prefs` → a static `Look` (colours, density, chart style) read by drawing code, plus egui visuals/style/zoom.

## Persistence

`%APPDATA%\AdvancedTaskManager\config.json` (settings, rules, profiles, schedules, scripts, dashboards, UI preferences). Written by the sampler when `config_dirty` is set. Snapshots, recordings, reports, dumps and exports live in sub-folders.

## Windows interfaces used

| Area | API |
|---|---|
| Processes, threads, handles, objects | `NtQuerySystemInformation` (5, 8, 64), `NtQueryObject`, `NtQueryInformationProcess/Thread`, `NtSuspendProcess`/`NtResumeProcess` |
| Process details | `OpenProcess`, `QueryFullProcessImageNameW`, token APIs, `IsWow64Process2`, `GetGuiResources`, `GetProcessInformation`, version-info APIs |
| Modules, memory | Toolhelp32, `VirtualQueryEx`, `GetMappedFileNameW` |
| Actions | `TerminateProcess`, `SetPriorityClass`, `SetProcessAffinityMask`, `SetProcessInformation` (power throttling), `MiniDumpWriteDump`, `SuspendThread`/`ResumeThread` |
| Counters | PDH (`PdhAddEnglishCounterW`, wildcard arrays), `CallNtPowerInformation`, `GetIfTable2`, `GetExtendedTcpTable/UdpTable` |
| Events | ETW `StartTraceW`/`EnableTraceEx2`/`OpenTraceW`/`ProcessTrace`, TDH |
| Management | PowerShell + CIM/WMI, `powercfg`, `logman`, `fsutil` |

## Design decisions

- **Rust + egui**: native speed, one small executable, no runtime to install, memory-safe outside the small audited `unsafe` areas.
- **One native call per sample** instead of opening every process: keeps cost around 10–15 ms per second for ~400 processes.
- **PowerShell for management data**: services, tasks, devices and settings change rarely and have rich, stable CIM classes; querying them on demand avoids writing dozens of COM bindings.
- **No service, no driver**: nothing to install or uninstall and nothing running when the window is closed. The trade-off is that fan/voltage sensors (which need a kernel driver) are not available.
