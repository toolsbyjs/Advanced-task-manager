# Advanced TaskManager — Developer Guide

Everything you need to build, change and extend the app. Read [ARCHITECTURE.md](ARCHITECTURE.md) for the big picture first.

## 1. Toolchain

| Tool | Version | Notes |
|---|---|---|
| Rust | 1.80+ stable, `x86_64-pc-windows-msvc` | `rustup default stable-msvc` |
| Visual Studio Build Tools | 2019 or newer | "Desktop development with C++" (linker + Windows SDK) |
| Windows | 10 1809+ / 11 | Needed to run; the code targets Windows only |
| Optional | `cargo-license`, `cargo-outdated` | Licence and dependency checks |

## 2. Building and running

```powershell
cargo build                     # debug build: target\debug\advanced_taskmanager.exe
cargo build --release           # optimised:   target\release\advanced_taskmanager.exe
cargo run -- --page processes   # run with arguments
```

The debug profile uses `opt-level = 1` for our code and `opt-level = 2` for dependencies, so debug builds are fast enough to use. Release builds hide the console window (`windows_subsystem = "windows"`).

### Command-line options (useful while developing)

| Option | Effect |
|---|---|
| `--page <name>` | Open a page (see `page_from_name` in `main.rs`) |
| `--theme <name>` | Theme for this run |
| `--compact` | Icons-only sidebar |
| `--view admin` | Show the Admin view layout without elevation (data will be partial) |
| `--pid <n>` / `--tab <name>` | Pre-select a process and detail tab (`threads`, `modules`, `memory`, `handles`, `security`, …) |
| `--screenshot <file>` | Draw ≥120 frames after ≥6 samples, save the window as PNG, exit. Opens without focus and behind other windows. |
| `--leftovers-test <name> <file>` | Write what a leftover scan finds for a program (all three levels, ticked or not) to a text file; deletes nothing |
| `--wait <secs>` | With `--screenshot`, wait longer (for slow PowerShell pages) |

Example — check the memory map tab of Explorer:
```powershell
$ex = (Get-Process explorer).Id
.\target\debug\advanced_taskmanager.exe --screenshot shots\mem.png --view admin --page inspector --pid $ex --tab memory
```

### Testing the Admin view
Run the exe from an elevated terminal, or click **🛡 Admin** and accept UAC. The elevated copy is started with `--admin` (ignored, informational).

## 3. Source layout

```
src/
  main.rs        arguments, window options, screenshot mode, start-up page
  config.rs      saved settings (serde): columns, rules, profiles, schedules, scripts, dashboards, UiPrefs
  collector.rs   the sampler thread, shared State, history rings, timeline, rule/profile/schedule engine, CSV recorder
  nt.rs          ntdll: processes+threads, per-CPU times, handles, object types, object names, suspend/resume
  sys.rs         Win32: elevation, process static/dynamic info, token, modules, memory map, handles, threads,
                 actions, TCP/UDP tables, memory status, volumes, interfaces, kernel modules, registry
  pdh.rs         Performance Data Helper wrapper (wildcard counters)
  etw.rs         real-time ETW session + TDH field decoder
  queries.rs     PowerShell/CIM/powercfg queries (services, startup, tasks, drivers, devices, power,
                 adapters, Windows config, events, sensors, disk health, snapshots)
  cleanup.rs     uninstaller & clean-up: installed programs (registry), uninstall runner, leftover
                 scan (+ shared-item check), reg backups, Store apps, browser extensions, junk,
                 privacy traces, secure delete, restore points, history, Windows tools
  disk.rs        disk space analysis: fast NTFS file-table scan, threaded folder walk, size tree,
                 file types, duplicates, squarified treemap layout, Recycle Bin
  intel.rs       bottlenecks, behaviour, leaks, correlations, health score, recommendations, HTML report
  util.rs        formatting, Ring buffer, Job (background task), hidden commands, PowerShell→JSON, data dir
  ui/
    mod.rs       App struct, Page enum, NAV table, routing, dialogs, toasts, screenshot handling
    shell.rs     top bar, sidebar, status bar, command palette, shortcuts, preference sync
    widgets.rs   live theme (Look), apply_prefs, cards, charts, sparklines, bars, JSON tables
    dashboard.rs overview + custom dashboards (widgets)
    processes.rs process table, columns, presets, actions, detail tabs, history page
    perf.rs      CPU / memory / disk / network / GPU pages
    manage.rs    system info, services, startup, tasks, drivers, devices, power, adapters, Windows config,
                 snapshots, change impact
    cleanup.rs   clean-up pages (uninstaller wizard, apps, extensions, junk, privacy, shred, backups, tools)
    diskspace.rs disk space analyzer page (tree, largest files, types, duplicates, treemap)
    deep.rs      inspector, kernel objects, ETW, Event Viewer, timeline, recording, graphs, sensors, disk health
    insights.rs  health, bottlenecks, behaviour, leaks, startup analysis, correlations
    automation.rs rules, alerts, profiles, schedules, scripts
    settings.rs  settings tabs
```

## 4. Key patterns

### Shared state
`collector::State` lives in `Arc<Mutex<State>>` (`App::shared`). The sampler gathers everything **without** the lock, then locks once to merge. UI code locks for the duration of drawing a page:

```rust
let shared = app.shared.clone();
let s = shared.lock().unwrap();     // read-only use
// …draw with &s…
drop(s);                            // before calling anything that locks again
```
Never call a function that locks `shared` while you already hold the guard (deadlock).

### Background jobs
Anything slow (PowerShell, file version info, memory maps, handle lists) runs in a `util::Job<T>`:

```rust
app.manage.services.ensure(ctx, queries::services);   // start once
if !job_status(ui, &app.manage.services) { return; }   // spinner / error
let rows = app.manage.services.take_ok().unwrap_or_default();
```
Reset with `job.started = false` (refresh) or `job = Job::default()`.

### PowerShell queries
`util::ps_json(script)` returns `Vec<serde_json::Value>`; `util::ps_object(script)` returns one object. Scripts run hidden with UTF-8 output. Gotchas:
- Wrap multi-statement object scripts — `ps_object` already uses `& { … }`.
- **Aliases beat functions**: don't name helpers `rv`, `gc`, `sc`, `ls`, … (`RV` is `Remove-Variable`). Use Verb-Noun names.
- Commands that need elevation may throw *terminating* errors even with `-EA SilentlyContinue`; wrap them in `try { } catch { }`.
- Enums come back as numbers; cast with `[string]` in the script.
- Quote user values with `util::ps_quote`.

### Tables
`widgets::json_table(ui, id, &rows, &[col("Key","Label",width), …], &mut TableState, &tint, height)` gives filtering, sorting and selection for any JSON list. Process and thread tables use `egui_extras::TableBuilder` directly with `row_h()` for density.

### Theme and colours
Use `c_bg()`, `c_panel()`, `c_card()`, `c_card_hi()`, `c_border()`, `c_text()`, `c_dim()`, `accent()` — never hard-coded greys — so all six themes work. Semantic colours (`CYAN`, `GREEN`, `RED`, …) are constants. `apply_prefs` sets egui visuals from `UiPrefs`; `App::sync_prefs` re-applies whenever `app.prefs` changes and saves it.

### Glyphs
egui's bundled fonts lack: `● → ↓ ↑ ▸ ▾ ◆ ▲ ▼ ↵ │ ✕ ⬤`. Use `⏺ ➡ ⬇ ⬆ ⏵ ⏷ 💠 🔼 🔽 Enter | ✖`. To test a set of characters:
```powershell
Start-Process target\debug\advanced_taskmanager.exe -Environment @{ DTM_GLYPHS = "★☆⌨" } -Wait
Get-Content glyphs_missing.txt
```

## 5. How to…

### Add a page
1. Add a variant to `Page` and an entry to `NAV` in `ui/mod.rs` (icon, label, `admin_only`).
2. Add a `route` arm calling your function `fn my_page(app: &mut App, ui: &mut Ui)`.
3. Optional: a name in `page_from_name` (`main.rs`) for `--page`.
4. If it needs administrator rights: `if app.locked(ui, "why") { return; }` at the top.

### Add a process column
1. Add a `ColDef` to `COLUMNS` in `ui/processes.rs` (`admin: true` for expert columns).
2. Add arms to `key()` (sorting) and `text()` (display).
3. If the data isn't collected yet, add the field to `collector::ProcInfo` and fill it in `collector::run` (keep it cheap!).

### Add a rule metric
1. Add a `Metric` variant (and to `Metric::ALL`, `label`, `is_process`) in `config.rs`.
2. Return its value in `metric_system` or `metric_proc` in `collector.rs`.

### Add an ETW source
Add a `ProviderDef` (name, GUID, keywords, level) to `etw::PROVIDERS`. Fields are decoded automatically through TDH.

### Add a dashboard widget
Add a `Widget` variant (+ `ALL`, `label`) in `config.rs` and an arm in `dashboard::widget`.

### Add a setting
Add a field to `UiPrefs` (it has `#[serde(default)]`, so old settings files still load), set its default, use it via `look()` or `app.prefs`, and add a control in `ui/settings.rs`.

### Add a junk-file category
Add a `JunkCat` to `cleanup::junk_categories()` (key, name, plain-language explanation, `admin`, `default_on`, folders, optional file-name pattern). Scanning, sizes, cleaning and the page pick it up automatically. Only add locations that are safe to empty while Windows runs.

### Add a leftover location
Add the folder to the `roots` list (or a registry base to `bases`) in `cleanup::scan_leftovers_inner`. Check the result with
```powershell
.\target\debug\advanced_taskmanager.exe --leftovers-test "Program name" shots\lo.txt
```
on several installed programs, including ones whose publisher has other products (shared folders must come out unticked).

### Add a Windows tool
Append `(icon, name, command, description)` to `cleanup::TOOLS`; check the icon exists in egui's fonts (see *Glyphs*).

### Add a confirmation for a new risky action
Add a variant to `ui::cleanup::Pending`, open it with `ask(app, title, text, Pending::…)`, and handle it in `run_pending`. The shared dialog in `ui/mod.rs` shows it.

## 6. Performance budget

- The sampler must stay well under 50 ms per sample (currently ~10–15 ms with ~400 processes). The status bar shows it.
- Static per-process data (path, user, version info) is read once per process and cached by `(pid, create_time)`.
- Connections are refreshed every 3 s, volumes every 15 s.
- Histories: 3,600 system samples (1 h) and 300 per process.

## 7. Native code safety checklist

- Use `Vec<u64>` buffers for structures Windows returns (8-byte alignment) and read with `ptr::read_unaligned`.
- Grow buffers on `STATUS_INFO_LENGTH_MISMATCH` / `ERROR_INSUFFICIENT_BUFFER`; cap loops.
- Wrap handles in `sys::Handle` so they close on drop.
- Never query names of file handles that are pipes (`GetFileType != FILE_TYPE_DISK`) — it can hang.
- Check every return code and turn failures into readable messages (`last_error()`).

## 8. Checks before a pull request

```powershell
cargo fmt --all
cargo clippy --all-targets
cargo build --release
.\target\release\advanced_taskmanager.exe --screenshot shots\check.png
```
Look at the picture; test in the Standard view and, if relevant, the Admin view and the Light theme. Update `CHANGELOG.md`.

## 9. Releasing

1. Bump `version` in `Cargo.toml`; move *Unreleased* in `CHANGELOG.md` under the new version and date.
2. `cargo build --release`.
3. Ship `target\release\advanced_taskmanager.exe` together with `LICENSE`, `README.md` and `THIRD_PARTY_NOTICES.md`.
4. Optionally sign the executable (`signtool sign /fd SHA256 …`) so Windows SmartScreen trusts it.

## 10. Project location

The project's home is `E:\MyAPPS-AI\Advanced TaskManager`. If you work in another folder, run `.\sync-to-home.ps1` to mirror your changes there (build output and screenshots are skipped).
