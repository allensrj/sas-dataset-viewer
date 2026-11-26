# SAS Dataset Viewer (XPT / SAS7BDAT)

[简体中文](README.md) | English | [![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)

A high-performance, cross-platform desktop viewer for SAS datasets (`.sas7bdat` & `.xpt`), built with pure C ([`ReadStat`](https://github.com/WizardMac/ReadStat), the same parser behind `pyreadstat`), a Rust columnar engine, and a native GPU GUI (`egui` / `egui_dock`).

## Key Features
- **Extreme Performance**: Millisecond-level cold startup, two-phase streaming load (instant 5,000-row first screen + background streaming completion), SAS-optimized dictionary-compressed columnar storage, and `rayon` multi-core parallel filtering, sorting, and statistics. Benchmarked on a 200,000-row `.sas7bdat` file: full load ~50 ms, multi-column filter & sort ~6 ms, statistics ~1 ms.
- **Three-Pane Vertical Layout**:
  - **Left File Explorer**: Drag and drop a single `.sas7bdat` / `.xpt` file to automatically import all datasets in its parent directory; supports tree expand/collapse, in-app alias renaming (synced to the bottom status bar), refresh, sidebar removal (without deleting files on disk), drag-and-drop custom reordering (persisted), drag into the center area for split-pane viewing, and one-click collapse/expand.
  - **Center Data Grid**: Three-row pinned header (original 1-based variable index `1..N` + variable name + label; sorting only triggers when clicking the variable name, with `Shift+Click` for multi-column sorting, while clicking the index or label does not sort to prevent accidental triggers); the row-number column always shows original row order, and clicking the `#` header restores the original order; supports `Ctrl/Cmd` / `Shift` multi-cell selection and multi-value filtering (`In` / `Not In`); integrated VS Code–style top `Search` bar supporting character and numeric values, case sensitivity (`Aa`), whole-word matching, regex (`.*`), and next/previous navigation; adaptive column widths (6–30 characters) and one-click uniform width reset; multi-dataset tabs can be freely dragged into horizontal or vertical split views.
  - **Right Inspector Panel**: Freely resizable with persisted locked panel width; the **Variables** tab uses a compact 5-column layout (`#` / Name / Type / Format / Label) with instant search, 1-to-1 aligned with original dataset variable numbers, and supports per-dataset custom "Preset Variables" and one-click "Apply Preset" (persisted per dataset until removed from the file explorer); the **Statistics** tab computes character and numeric summaries on the currently filtered data along with a 3-column frequency table (`Value` / `Count` / `Percent`) supporting column-header sorting, multi-selection, and `D` quick filtering; built-in **Shortcuts** cheat sheet.
- **Polished Experience & Version Updates**: Custom application logo across macOS `.app` icon, Windows `.exe` icon, window icon, and in-app UI; circular `i` "About" button on the top-right toolbar displaying app info, GitHub repository, author, contributors, and the author's WeChat official account QR code, plus built-in automatic and manual GitHub Releases update checking with update notification badges.
- **SAS-Specific Details**: Renders SAS date/time formats (`DATE9.`, `YYMMDD10.`, `DATETIME20.`, `TIME8.`, etc.) as formatted dates while preserving numeric sorting and filtering semantics; supports special missing values (`.`, `.A`–`.Z`, `._`); bottom status bar displays `Folder - Dataset` and performance metrics, with a bottom-right global display scale selector (`75%–200%`, default `100%`, remembered per monitor) and character encoding switcher (Auto-detect / UTF-8 / GB18030 / Windows-1252 / ISO-8859-1).

## Keyboard Shortcuts
| Shortcut | Action |
| :--- | :--- |
| `F` | Immediately show statistical summary and value distribution for the selected cell/variable in the right Statistics tab |
| `D` | Quick filter (`=` / `IN`) by one or more selected values (works in both Data Grid and Statistics tab) |
| `Ctrl/Cmd + Click` / `Shift + Click` | Multi-select non-contiguous / contiguous values in Data Grid or Statistics tab |
| `C` | Clear all filters and sorting on the active dataset |
| `H` / `U` | Uncheck all variables (hide all) / check all variables (unhide all) in the right Variables tab |
| `Ctrl/Cmd + Shift + F` | Focus the top `Search` cell-value bar (`Enter` / `F3` next match, `Shift+Enter` / `Shift+F3` previous match) |
| `Ctrl/Cmd + F` | Focus the right Variables search box |
| `Alt + ← / →` | Move the selected variable column left / right |
| `Ctrl/Cmd + C` | Copy the selected cell value |
| `Ctrl/Cmd + B` / `Ctrl/Cmd + \` | Collapse / expand the left File Explorer / right Inspector panel |
| `Ctrl/Cmd + / - / 0` | Zoom in / zoom out / reset UI scale (low-DPI 100% displays default to 1.2x clarity zoom, remembered per monitor) |

## Changelog
- **`v0.3.2`**: Added custom application logo (macOS `AppIcon.icns`, Windows `.exe` icon, window and in-app icons); added top-right circular `i` "About" dialog (app overview, GitHub repo link, author, contributors, and personal WeChat official account); added per-dataset "Preset Variables" & "Apply Preset" in the right Variables tab (e.g., `STUDYID, USUBJID, SUBJID, SITEID`, persisted per dataset until removed from the file explorer); added automatic and manual GitHub Releases update checking (in the "About" dialog and via a top toolbar update badge).
- **`v0.3.1`**: Added original variable-number header row in the data grid and matching `#` column in the right Variables tab; upgraded the Statistics frequency table to 3 sortable columns (`Value` / `Count` / `Percent`); added a global display scale selector (`75%–200%`, default `100%`, remembered per monitor) on the bottom-right status bar; defaulted low-DPI (100%) displays to 1.2x clarity zoom (#4 / #5).
- **`v0.3.0`**: Locked right panel width and compact 4-column Variables table (`Name` / `Type` / `Format` / `Label`); unified multi-selection and `D` multi-value filtering across Data Grid and Statistics tab, plus full numeric value listing; added VS Code–style `Search` (character/numeric, `Aa`, whole-word, `.*` regex); 6–30 char adaptive column widths and uniform reset width; preserved original row numbers with `#` header click to restore order, and restricted column sorting to clicking the variable name; auto-imported parent directory on single-file drag-and-drop, fixed sample dataset generation, and prefixed status bar with folder name.
- **`v0.2.0`**: Replaced `eframe` with a custom unified `winit` event-loop host (fixing Windows drag freezes, #3); split releases into 3 dedicated builds — `macOS-arm64` (Metal), `win-x64` (GPU + CPU fallback), and `win-aws` (pure CPU GDI); fixed macOS ad-hoc code signing and packaging.
- **`v0.1.x`**: Initial release (`ReadStat` + Rust columnar engine + 3-pane GUI), followed by a pure-CPU software renderer for GPU-less AWS Windows Server environments.

## Builds & Releases (Three Dedicated Artifacts)

Since `v0.2.0`, the app runs on a custom unified `winit` event-loop host (replacing `eframe` to eliminate modal loop freezes when dragging the title bar or window borders on Windows; see #3). All three builds share 100% of the same C (`ReadStat`) / Rust data engine and three-pane UI, differing only by Cargo build features:

| Release Artifact | Target Environment | Rendering Architecture | Highlights |
| :--- | :--- | :--- | :--- |
| **`SAS-Dataset-Viewer-macOS-arm64.zip`** | macOS (Apple Silicon) | Native **Apple Metal** (`backend-wgpu`) | Strips OpenGL/software-renderer overhead; unzip to get `.app` |
| **`SAS-Dataset-Viewer-win-x64.exe`** | Local Windows 10/11 PCs | Desktop GPU (`glow` / `wgpu`) + CPU fallback | Skips redundant pan repaints and coalesces resize events; smooth high-FPS, static CRT |
| **`SAS-Dataset-Viewer-win-aws.exe`** | AWS EC2 / Windows Server / RDP | **Pure multi-core CPU Win32 GDI** (`backend-cpu`) | Zero GPU/DX12/OpenGL/VC++ dependencies; instant single-process startup, ultra-lean |

> **macOS First-Launch Note**: Because this app is not signed and notarized with a paid Apple Developer account, browsers downloading the `.zip` from GitHub attach a quarantine attribute (`com.apple.quarantine`). If double-clicking reports *"cannot be opened because the developer cannot be verified"* or *"is damaged and can't be opened"*, run the following command in Terminal once to remove the quarantine flag so it opens normally from then on:
> ```bash
> xattr -cr /path/to/"SAS Dataset Viewer.app"
> ```
> (If you moved the app into `/Applications`, run `xattr -cr /Applications/"SAS Dataset Viewer.app"`.)

### Build from Source

```bash
# 1. macOS Apple Silicon (.app)
bash scripts/bundle_macos.sh

# 2. Windows Desktop (win-x64)
cargo build --release --bin sas-viewer --no-default-features --features "backend-glow,backend-wgpu,backend-cpu"

# 3. Windows AWS / GPU-less Server Edition (win-aws)
cargo build --release --bin sas-viewer --no-default-features --features backend-cpu
```
You can also use the included `.github/workflows/release.yml` workflow to automatically build and publish all three release artifacts on GitHub Actions.

## Contributors
Thanks to the following developers for contributing to this project:

- [@allensrj](https://github.com/allensrj) — Project creator, core architecture & feature development
- [@wtwang1998](https://github.com/wtwang1998) (wentao) — Root-cause analysis of Windows drag freeze (#1), default renderer fix (#2), event-loop architecture RFC & empirical verification (#3), and low-DPI display text legibility optimization (#4 / #5)

## License & Copyright
Copyright © 2026 allensrj

Licensed under the [Apache License, Version 2.0](LICENSE). You may use, modify, and distribute this software in compliance with the License, provided you retain the original copyright notice, license text, and attribution notices in [NOTICE](NOTICE), and state any significant changes made.

Third-party notice: The bundled [`ReadStat`](https://github.com/WizardMac/ReadStat) C parser library is licensed under the MIT License (Copyright © 2013-2016 Evan Miller); see [NOTICE](NOTICE) and [`third_party/readstat/LICENSE`](third_party/readstat/LICENSE).
