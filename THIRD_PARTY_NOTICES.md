# Third-Party Notices

Advanced TaskManager is built on these open-source libraries. Each is used under its own licence; the full licence texts ship with the crates (see `cargo` registry sources or crates.io).

| Library | Purpose | Licence |
|---|---|---|
| [eframe / egui](https://github.com/emilk/egui) 0.29 | Window and user interface | MIT OR Apache-2.0 |
| [egui_extras](https://github.com/emilk/egui) 0.29 | Tables | MIT OR Apache-2.0 |
| [windows-sys](https://github.com/microsoft/windows-rs) 0.59 | Windows API bindings | MIT OR Apache-2.0 |
| [serde](https://serde.rs) / serde_json | Settings and data files | MIT OR Apache-2.0 |
| [image](https://github.com/image-rs/image) 0.25 (PNG only) | Saving screenshots | MIT OR Apache-2.0 |
| [chrono](https://github.com/chronotope/chrono) 0.4 | Dates and times | MIT OR Apache-2.0 |

Their own dependencies (for example `winit`, `glow`, `glutin`, `ab_glyph`, `png`) are likewise MIT, Apache-2.0, BSD, Zlib or similarly permissive. To list every dependency and licence:

```powershell
cargo install cargo-license
cargo license
```

## Fonts

egui's default fonts are bundled:

- **Ubuntu Light** — Ubuntu Font Licence 1.0
- **Hack** — MIT / Bitstream Vera licence
- **Noto Emoji** — SIL Open Font Licence 1.1
- **emoji-icon-font** — SIL Open Font Licence 1.1 / MIT

## Windows tools

At run time the app may call tools that are part of Windows (PowerShell, `powercfg`, `logman`, `fsutil`) and, if installed, NVIDIA's `nvidia-smi`. These are not distributed with the app.

"Windows" is a trademark of Microsoft Corporation. "NVIDIA" is a trademark of NVIDIA Corporation. Advanced TaskManager is an independent project and is not affiliated with or endorsed by either company.
