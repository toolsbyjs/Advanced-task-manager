# Feature reference

Every page in Advanced TaskManager, what it does, and whether it needs the Admin view.
🛡 = needs the Admin view · ◐ = works in the Standard view, more in the Admin view · — = works fully in the Standard view.

Open any page from the sidebar, with **Ctrl+K** (type its name) or at start-up with `--page <name>`.

## Overview

| Page | `--page` | What it does | Admin |
|---|---|---|---|
| Dashboard | `dashboard` | Live tiles for CPU, memory, disk, network and GPU; health score; system summary; current bottlenecks; 2-minute graphs; top processes by CPU, memory and disk; latest events; drives. Sections can be hidden (⚙ Customize). | — |
| My dashboards | `custom` | Build any number of dashboards from 15 widgets in 1–4 columns. | — |

## Monitor

| Page | `--page` | What it does | Admin |
|---|---|---|---|
| Processes | `processes` | Flat or tree list; search; 44 columns (17 in Standard view); 7 column sets; column ordering; hold order; heat colours; actions (end, end tree, priority, efficiency mode, suspend/resume, affinity, dump, location, properties, copy); detail tabs General, Performance, Network, History, and in Admin view Threads, Modules, Memory map, Handles, Security. | ◐ |
| Performance | `performance` | CPU (per core, kernel/DPC/interrupt time, clocks, context switches, system calls, run queue), memory composition and pools, each physical disk (active time, speed, queue, response), each network adapter, GPU engines and memory; 1 min – 1 h windows. | — |
| Process history | `history` | Every start and exit seen, with lifetime, peak CPU and peak memory. | — |
| System information | `sysinfo` | Computer, Windows, firmware/board, processor, memory modules, graphics, disks, monitors, sound. | — |

## Manage

| Page | `--page` | What it does | Admin |
|---|---|---|---|
| Disk space analyzer | `diskspace` | Scan a drive or folder; folder tree with % of parent; largest files with wildcard/include/exclude filters; file types; duplicates; zoomable treemap; CSV export; open, copy path, command prompt, Recycle Bin. Fast NTFS file-table scan in Admin view. | ◐ |
| Services | `services` | All services; start, stop, restart, start type, dependencies. | ◐ (control) |
| Startup apps | `startup` | Run keys, startup folders, logon tasks, auto-start services; enable/disable. | ◐ (all users) |
| Scheduled tasks | `tasks` | All tasks with triggers, actions, account, last/next run; run, end, enable, disable. | ◐ (enable/disable) |
| Drivers | `drivers` | Driver services, signed device drivers (unsigned highlighted), loaded kernel modules. | ◐ (addresses) |
| Devices | `devices` | Every device with class, maker, driver and problem description. | — |
| Power | `power` | Power plans, active plan settings, battery, sleep states, battery report. | — |
| Network adapters | `adapters` | Live traffic, drivers, addresses, DNS, gateways, profiles; enable/disable. | ◐ (control) |
| Windows settings | `winconfig` | Security (UAC, Secure Boot, Defender, VBS, firewall, BitLocker), behaviour, page files, updates, features, environment. | ◐ |
| Snapshots & changes | `snapshots` | Save configuration snapshots and compare any two. | — |

## Clean up

| Page | `--page` | What it does | Admin |
|---|---|---|---|
| Uninstaller | `uninstaller` | All programs (filters: new this week, updates, system components; sizes; export). Uninstall, quick (silent) uninstall, forced uninstall, remove entry. Leftover wizard (Safe / Moderate / Advanced) over registry, folders, app data, shortcuts and services, with registry backup and Recycle Bin. Forced scan by name. Uninstall history. Optional restore point. | ◐ |
| Windows apps | `storeapps` | Store and built-in apps; uninstall (all users in Admin view). | ◐ |
| Browser extensions | `extensions` | Extensions in Chrome, Edge, Brave, Vivaldi, Opera and Firefox per profile; show files; remove. | — |
| Junk files cleaner | `junk` | Temp files, thumbnail cache, error reports, crash dumps, shader and browser caches, Recycle Bin; Windows temp, update downloads, delivery cache and setup logs in Admin view. | ◐ |
| Privacy cleaner | `privacy` | Recent files, jump lists, Run history, Explorer typed paths and searches, clipboard, DNS cache. | — |
| Secure delete | `shred` | Overwrite (1/3/7 passes), rename and delete files or folders. | — |
| Backups & restore | `backups` | Restore registry backups made by the Uninstaller; list and create Windows restore points; open System Restore. | ◐ (restore points) |
| Windows tools | `tools` | One-click launcher for 23 built-in Windows tools. | — |

## Deep

| Page | `--page` | What it does | Admin |
|---|---|---|---|
| Process inspector | `inspector` | Full-screen process details with a picker. | 🛡 |
| Kernel objects | `handles` | Object types with object/handle counts; processes holding most handles. | 🛡 |
| Live system events | `etw` | Real-time ETW trace: processes, threads, DLL loads, network, DNS, file create/delete, power, PowerShell. | 🛡 |
| Event Viewer | `events` | Errors and warnings from any Windows log for 1 h – 30 days. | ◐ (Security log) |
| Diagnostics timeline | `timeline` | Starts, exits, spikes, alerts, automation, snapshots and Windows errors in order. | — |
| Recording | `recording` | Record all counters to CSV for a set time or until stopped. | — |
| Historical graphs | `graphs` | Up to 1 hour of history, or a saved recording. | — |
| Hardware sensors | `sensors` | Thermal zones, clocks, fans, battery, NVIDIA GPU (when its tools are installed). | ◐ |
| Disk health | `diskhealth` | Drive health, wear, temperature, hours, errors, SMART prediction, TRIM. | ◐ |

## Insights

| Page | `--page` | What it does | Admin |
|---|---|---|---|
| Health report | `health` | 0–100 score with breakdown, recommendations, HTML report. | — |
| Bottlenecks | `bottlenecks` | CPU, run queue, driver time, memory, commit, paging, disks, free space, network, GPU, heat, throttling. | — |
| Process behaviour | `behaviour` | Labels: CPU-bound, bursty, disk-heavy, growing memory/handles/threads, GPU, network-chatty, idle… | — |
| Leak candidates | `leaks` | Steady growth of memory, handles or threads. | — |
| Startup analysis | `startupanalysis` | Startup entries ranked by estimated impact; recent boot times. | ◐ (boot times) |
| Correlated events | `correlations` | Spikes matched with what happened within ±5 s. | — |
| Change impact | `changeimpact` | Differences between two snapshots and load change. | — |

## Automate

| Page | `--page` | What it does | Admin |
|---|---|---|---|
| Rules | `rules` | Metric + threshold + duration → alert, end, suspend, priority, efficiency, script, recording, profile. | ◐ (other users' processes) |
| Alerts | `alerts` | Everything rules raised; acknowledge; filter. | — |
| Profiles | `profiles` | System profiles (power plan, priorities, sampling) and per-process profiles applied at every start. | ◐ |
| Scheduled monitoring | `schedules` | Every N minutes or daily: record, snapshot, health report. | — |
| Scripts | `scripts` | PowerShell/cmd library with output; JSON export of live data. | — |
| Settings | `settings` | Appearance, layout & navigation, process table, monitoring, shortcuts, your data. | — |

## Command line

| Option | Effect |
|---|---|
| `--page <name>` | Open a page (names above) |
| `--scan <drive or folder>` | Open the Disk space analyzer and scan straight away |
| `--theme <name>` | Midnight, Graphite, Ocean, Forest, Light or High contrast for this run |
| `--compact` | Icons-only sidebar |
| `--screenshot <file.png>` | Draw, save a picture of the window, exit (automated checks) |
| `--wait <seconds>` | With `--screenshot`: wait longer before the picture |
| `--view admin`, `--pid <n>`, `--tab <name>` | Developer previews (see the developer guide) |
| `--leftovers-test <name> <file>` | List what a leftover scan would find for a program; deletes nothing |

## Not included (and why)

| Feature seen in other tools | Reason |
|---|---|
| Tracing new installations as they happen | Needs a kernel filter driver; this app installs no drivers |
| Online database of uninstall logs | Belongs to another vendor's service |
| "Hunter mode" (target a program by its window/icon) | Not planned |
| Wiping free space on a whole drive | Fills the drive temporarily; use `cipher /w:` or BitLocker instead |
| Fan speeds and voltages on most PCs | Windows exposes no standard reading without vendor drivers |
| Bandwidth per process in the process table | Windows doesn't count it by default; see Live system events |
