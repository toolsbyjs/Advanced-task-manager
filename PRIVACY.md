# Privacy Policy

**Short version: Advanced TaskManager keeps everything on your computer. It sends nothing to anyone.**

## What the app reads

To show you what your PC is doing, the app reads — only while it runs, and only on your computer:

- processes and threads (names, paths, command lines, user accounts, resource use),
- system counters (CPU, memory, disks, network adapters, GPU, temperatures),
- network endpoints (local/remote addresses and ports per process — not the content of traffic),
- Windows configuration (services, startup entries, scheduled tasks, drivers, devices, power, security settings, installed updates and programs),
- Windows event logs you choose to view,
- the list of installed programs, Store apps and browser extensions (names, versions and folders — never browsing history, passwords or cookies),
- file and folder names and sizes when you scan in the Disk space analyzer, and sizes of temporary and cache folders in the Junk files cleaner,
- live system events (ETW) only while you run a trace in the Admin view.

## What the app stores

Only in `%APPDATA%\AdvancedTaskManager` on your PC:

| File | Contents | When |
|---|---|---|
| `config.json` | Your settings, rules, profiles, schedules, scripts, dashboards | Whenever you change a setting |
| `snapshots\*.json` | Lists of services, drivers, startup entries, tasks, programs, updates, adapters and devices, plus load figures | When you (or a schedule) take a snapshot |
| `recordings\*.csv` | Performance counters and the names of the top processes | While recording |
| `reports\*.html` | Health reports, battery reports | When you save one |
| `dumps\*.dmp` | Memory of a process you chose | Only when you create a dump |
| `exports\*` | Live process list (JSON); program, disk and file lists (CSV) | Only when you export |
| `backups\*` | `.reg` copies of registry keys removed by the Uninstaller, with a manifest | When you remove leftovers or an entry |
| `uninstall-history.json` | Programs you removed, when, and how much was cleaned | When you uninstall |

Nothing else is written, and nothing is kept in memory after the app closes except these files. Delete the folder at any time to remove all data (Settings → Your data → Open folder).

## What the app sends

**Nothing.** There is no telemetry, analytics, crash reporting, advertising, account or auto-update. The app opens no network connections of its own. Windows tools it calls (PowerShell, `powercfg`, `logman`, `nvidia-smi`) run locally.

## What the app deletes

Only what you choose: leftovers you tick (registry keys are backed up first, files go to the Recycle Bin), junk files, privacy traces (recent-files list, Run and Explorer history, clipboard, DNS cache), files you add to Secure delete, and browser extensions you remove. The Privacy cleaner does not touch browser history, passwords or cookies.

## Sharing

Snapshots, recordings, reports and especially dump files can contain personal details (user names, computer name, file paths, program lists, memory contents). Review them before sharing with anyone.

## Children

The app does not collect any data, from anyone.

## Changes

Changes to this policy will be listed in the [changelog](CHANGELOG.md).
