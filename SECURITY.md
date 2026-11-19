# Security Policy

Advanced TaskManager reads deep system information and, in the Admin view, can control processes, services, drivers and network adapters. We take its safety seriously.

## Supported versions

| Version | Supported |
|---|---|
| Latest release (0.1.x) | ✅ Security fixes |
| Older builds | ❌ Please update |

## Reporting a vulnerability

**Do not open a public issue for security problems.**

Report privately to the maintainers (see the contact in the project's home folder or repository security settings). Please include:

- what the problem is and its impact,
- steps to reproduce (a minimal proof of concept if possible),
- the version and Windows build you used,
- whether it needs the Admin view.

We aim to acknowledge reports within **3 working days**, give a first assessment within **10 working days**, and release a fix as quickly as the severity requires. We'll credit you in the changelog unless you prefer otherwise. Please give us reasonable time to fix the issue before disclosing it.

## Safety design

- **Least privilege by default.** The app starts in the Standard view with normal rights. Administrator rights are only requested when you choose the Admin view, through the normal Windows UAC prompt. When elevated, it enables `SeDebugPrivilege` only so it can read protected process details.
- **No network access.** The app never connects to the internet. It sends no telemetry, crash reports or usage data and has no auto-update. (See [PRIVACY.md](PRIVACY.md).)
- **No background service or driver.** Nothing is installed; closing the window stops everything. Scheduled monitoring only runs while the app is open.
- **Read-mostly.** Changes to the system happen only when you press a button, a rule you created fires, or a profile you created is applied. Ending, suspending and stopping services ask for confirmation (configurable).
- **Careful native code.** Buffers returned by Windows are size-checked and read without alignment assumptions; every handle is closed; handle-name lookups skip pipes to avoid hangs.
- **Scripts are yours.** The script library and rule actions run only commands you typed, as your user (or as administrator in the Admin view). Treat scripts you didn't write with the same caution as any program.
- **Dump files contain memory.** Process dumps can include passwords and personal data from that process's memory. They are saved only in `%APPDATA%\AdvancedTaskManager\dumps`; delete them when no longer needed and never share them publicly.
- **Removing software and files is layered with safety nets.** The Uninstaller runs each program's own uninstaller first. Leftovers are shown for review before anything is removed; registry keys are exported to `.reg` backups (restorable from *Backups & restore*) before deletion; files and folders go to the Recycle Bin; items another installed program also uses are left unticked; the Windows folder and the app's own data are never offered. An optional Windows restore point can be made first.
- **Permanent deletions always ask first.** The Junk files cleaner, Privacy cleaner, Secure delete and emptying the Recycle Bin delete data for good. The junk cleaner skips files in use and, by default, files newer than one day. Secure delete refuses drive roots and the Windows folder.
- **Registry changes use Windows' own `reg.exe`** (export, delete, import), so every change is a plain, inspectable `.reg` file.
- **Browser extensions are removed by moving their folder to the Recycle Bin**, only while the browser is closed. Browser profiles, passwords and history are never read or changed.
- **Disk scanning is read-only.** The fast NTFS scan opens the drive for reading only.
- **ETW sessions** are private, real-time only (nothing written to disk) and stopped when you press Stop or close the app.

## Responsible use

Use the Admin features only on computers you own or are authorised to administer. Ending system processes or disabling drivers and services can make Windows unstable; the app warns for critical processes, but you are responsible for the changes you make.
