# Contributing to Advanced TaskManager

Thank you for helping. Bug reports, ideas, documentation fixes and code are all welcome.
By taking part you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Ways to help

- **Report a bug** — say what you did, what you expected and what happened. Include your Windows version, whether you used the Standard or Admin view, and a screenshot if it helps. Remove anything personal (user names, computer names, paths) first.
- **Suggest a feature** — describe the problem it solves, not only the solution.
- **Improve the docs** — typos, clearer wording and new screenshots are valuable.
- **Write code** — see below.

Security problems should **not** go in a public issue; follow [SECURITY.md](SECURITY.md).

## Getting set up

1. Install Rust (stable, MSVC toolchain): <https://rustup.rs>
2. Get the code (project home: `E:\MyAPPS-AI\Advanced TaskManager`).
3. Build and run:
   ```powershell
   cargo build
   .\target\debug\advanced_taskmanager.exe
   ```
4. Read the [Developer guide](docs/DEVELOPER_GUIDE.md) and [Architecture](docs/ARCHITECTURE.md).

## Making a change

1. Create a branch named after the change: `feature/disk-latency-column`, `fix/startup-toggle-hklm`.
2. Keep changes focused — one feature or fix per pull request.
3. Follow the coding rules below.
4. Run the checks:
   ```powershell
   cargo fmt --all
   cargo clippy --all-targets
   cargo build --release
   .\target\release\advanced_taskmanager.exe --screenshot shots\check.png
   ```
   Open `shots\check.png` and make sure the window drew correctly.
5. Test the feature in **both** the Standard view and the Admin view when it touches privileged data.
6. Update `CHANGELOG.md` under *Unreleased*, and the user guide if users will notice the change.
7. Open a pull request describing what changed, why, and how you tested it. Add before/after screenshots for UI changes.

## Coding rules

- **Low overhead first.** The sampler runs every second; anything added to it must be cheap. Expensive or occasional queries belong in a background `Job` started from the UI. The status bar shows the monitor cost per sample — it should stay well under 50 ms.
- **Never block the UI thread** on PowerShell, WMI, files or the network. Use `util::Job`.
- **Hold the shared state lock briefly.** Gather data first, then lock and merge.
- **Unsafe code** lives only in `nt.rs`, `sys.rs`, `pdh.rs` and `etw.rs`. Every `unsafe` block must check sizes and return codes, use `read_unaligned` for buffers from Windows, and close every handle (use `sys::Handle`).
- **Destructive actions** (ending, suspending, stopping services, changing settings) must go through a confirmation or be clearly requested by the user, and must report success or the Windows error message.
- **Anything that removes data** must show the person exactly what will go, back up registry changes before making them, prefer the Recycle Bin over permanent deletion, protect items other programs share, and never touch the Windows folder. Try removal logic with a dry run first (for example `--leftovers-test`).
- **Admin-only features** must degrade gracefully in the Standard view: show an explanation or the lock card, never crash or show raw errors.
- **Colours** come from the live theme (`c_bg()`, `c_text()`, `accent()`, …) so every theme works. Check new UI in the Light and High contrast themes.
- **Glyphs**: egui's built-in fonts lack some symbols (`●`, `→`, `↓`, `▸`, `↵`). Use the alternatives listed in the developer guide.
- **Plain language** in the UI: write for people who are not engineers; put technical names in tooltips.
- Run `cargo fmt`; keep `clippy` warnings from growing.

## Commit messages

Use the imperative mood and explain why when it isn't obvious:

```
Add disk latency column to the process table

Users asked to see which process causes slow I/O. Reads the existing
per-process I/O counters; no extra sampling cost.
```

## Licence of contributions

By contributing you agree that your work is released under the project's [MIT Licence](LICENSE).
