# Changelog

All notable changes to Advanced TaskManager are listed here.
The format follows [Keep a Changelog](https://keepachangelog.com/en/1.1.0/) and the project uses [Semantic Versioning](https://semver.org/).

## [Unreleased]

## [0.2.0] — 2026-11-19

### Documentation
- New **Feature reference** (`docs/FEATURES.md`): every page, what it does, what needs the Admin view, command-line names, and what's intentionally not included.
- User guide reorganised into 14 numbered chapters, with a new "Your data and safety nets" chapter and an expanded FAQ (uninstaller safety, restoring leftovers, disk scan speed, junk cleaner, browser sync, SSDs).
- Security and privacy policies cover the Disk space analyzer, Uninstaller, cleaners, backups and secure delete; architecture notes describe the clean-up engine; contributing rules for code that removes data.

### Added
- **Clean up** section: Uninstaller (all programs with search, new/updates/system filters, sizes, export; uninstall, quick uninstall, forced uninstall, remove entry; leftover wizard with Safe/Moderate/Advanced scans of registry, folders, app data, shortcuts and services; registry backup before deletion, files to Recycle Bin; shared items unticked; optional restore point; uninstall history), Windows apps (Store packages), Browser extensions (Chromium browsers and Firefox), Junk files cleaner, Privacy cleaner, Secure delete, Backups & restore points, Windows tools launcher.
- `--leftovers-test <name> <file>` dry run that lists what a leftover scan would find.
- **Disk space analyzer** page: scan a drive or folder; folder tree with percent-of-parent bars, size, space on disk, file and folder counts and newest change; largest-files list with name/path/wildcard search, include and exclude patterns and result limits; file-type breakdown; duplicate finder (name + size, optionally date); zoomable squarified treemap with shading, names, free-space block and colour-blind palette; CSV export of files, folders and types; copy path/info, open, show in Explorer, command prompt here, move to Recycle Bin.
- Fast NTFS scanning in the Admin view (reads the Master File Table directly); multi-threaded folder walk otherwise.
- `--scan <path>` start-up option.
- Full documentation set: README, user guide, developer guide, architecture notes, contributing guide, code of conduct, security and privacy policies, third-party notices, MIT licence.
- Project webpage (`docs/website/index.html`) with an illustrated tour of every feature.

## [0.1.0] — 2026-11-19

### Added — monitoring core
- Background sampler reading every process and thread in one native call (`NtQuerySystemInformation`), plus per-CPU times, memory lists, PDH disk/GPU/thermal counters, network interface tables and TCP/UDP endpoints.
- Dashboard, process explorer (flat and tree), 44 columns, search, sorting, column sets, column ordering, "hold order".
- Process actions: end, end tree, suspend/resume, priority, efficiency mode, CPU affinity, dump files, open location, file properties, copy details.
- Detail tabs: general, performance, threads, modules, memory map, handles, network, security token, history.
- Performance pages for CPU (per core), memory composition, disks, network adapters and GPU engines.
- Process history (starts, exits, lifetimes, peaks) and system information.

### Added — system management
- Services (start/stop/restart, start type, dependencies), startup apps (enable/disable like Task Manager), scheduled tasks, drivers (services, signatures, loaded kernel modules), device inventory with problem codes, power plans/settings/battery report, network adapters (live counters, addresses, DNS, gateways, profiles), Windows configuration (security, behaviour, page files, updates, features, environment), configuration snapshots and change tracking.

### Added — deep observability
- Live ETW tracing with field-by-field decoding (process, image loads, network, DNS, file, power, PowerShell).
- Kernel object and system-wide handle overview, Event Viewer, diagnostics timeline, CSV performance recording, historical graphs (up to one hour, or a saved recording), hardware sensors (thermal zones, clocks, battery wear, NVIDIA GPU via `nvidia-smi` when present), SSD/disk health.

### Added — intelligence and automation
- Health score and HTML report, bottleneck detection, behaviour classification, leak candidates, startup impact analysis, spike correlation, recommendations, change impact analysis.
- Rule engine, alerts, system profiles, per-process profiles, scheduled monitoring, script library with environment variables, JSON export, custom dashboards.

### Added — user experience
- Standard and Admin views with UAC relaunch.
- Six themes, accent colours, zoom, text size, density, rounding, graph style, network units.
- Sidebar favorites, collapsible groups, icons-only mode, hideable pages, start-up page.
- Command palette (Ctrl+K), keyboard shortcuts, status bar.

### Changed
- Renamed from "Deep Task Manager" to **Advanced TaskManager**; settings folder moves automatically from `%APPDATA%\DeepTaskManager` to `%APPDATA%\AdvancedTaskManager`.

[Unreleased]: https://github.com/toolsbyjs/Advanced-task-manager/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/toolsbyjs/Advanced-task-manager/releases/tag/v0.2.0
