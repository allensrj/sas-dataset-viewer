# SAS 数据集查看器（XPT / SAS7BDAT）

简体中文 | [English](README_EN.md) | [![License](https://img.shields.io/badge/License-Apache%202.0-blue.svg)](LICENSE)

基于纯 C（[`ReadStat`](https://github.com/WizardMac/ReadStat)，与 `pyreadstat` 同源）+ Rust 列式引擎 + 原生 GPU GUI（`egui` / `egui_dock`）构建的跨平台高速 SAS 数据集桌面查看器。

## 核心特性
- **极致性能**：毫秒级冷启动，两阶段流式加载（5,000 行首屏秒开 + 后台流式补齐），面向 SAS 特性优化的字典压缩列式存储与 `rayon` 多核并行筛选/排序/统计。实测 20 万行 `.sas7bdat` 完整加载 ~50ms、多列筛选排序 ~6ms、统计计算 ~1ms。
- **三栏纵向布局**：
  - **左侧文件管理器**：直接拖入单个 `.sas7bdat` / `.xpt` 文件即可自动纳入所在整个目录的全部数据集；支持树形折叠/展开、软件内别名重命名（同步显示在底部状态栏）、刷新、侧栏移除（不删磁盘文件）、拖拽自定义排序（持久化）、拖入中间分屏打开、一键收起/展开。
  - **中间数据查看栏**：三层固定表头（数据集原始变量编号 `1..N` + 变量名 + 标签；仅点击变量名排序、`Shift+点击` 多列叠加排序，点击编号/标签不排序防误触）；行号列始终显示原始行序，点击 `# / 行号` 列头可一键恢复初始顺序；支持单元格 `Ctrl/Cmd` / `Shift` 多选与多值筛选（`In` / `Not In`）；顶部集成 VS Code 风格 `Search` 检索栏（支持字符与数值、`Aa` 大小写、`全字` 匹配、`.*` 正则与上下跳转）；支持 6–30 字符数据自适应列宽与一键重置均匀列宽；多数据集标签页可自由拖拽上下/左右分屏。
  - **右侧信息栏**：支持自由拖拽并锁定面板宽度；**变量栏**采用紧凑 5 列排版（`#` 编号 / 变量名 / 类型 / Format / 标签）与即时搜索，与数据区原始编号一一对应，支持按数据集自定义「预设勾选变量」与一键「应用预设」（持久化跟随该数据集，直至在文件管理器中移除）；**统计量栏**基于当前筛选数据展示字符与数值摘要及三列频数分布表（`取值` / `频数` / `占比`，支持点击列头正倒序排序、多选并统一按 `D` 快速筛选）；内置快捷键速查栏。
- **软件体验与版本更新**：内置专属应用 Logo（覆盖 macOS `.app` 图标、Windows `.exe` 图标、窗口图标及应用内界面）；顶部栏右侧新增「关于」圆圈 `i` 入口，展示软件简介、GitHub 仓库、作者、贡献者与作者个人微信公众号，并内置 GitHub Releases 自动/手动检查更新功能，发布新版本时及时提醒下载。
- **SAS 专业细节**：支持 SAS 日期/时间格式（`DATE9.`, `YYMMDD10.`, `DATETIME20.`, `TIME8.` 等）按格式化日期展示同时保留数值排序/筛选逻辑；支持 `.`、`.A`~`.Z`、`._` 特殊缺失值；底部状态栏展示 `目录名 - 数据集名` 及性能指标，右下角支持一键调整全局显示缩放比例（`75%–200%`，默认 `100%` 并按显示器记忆）与切换字符编码（自动检测 / UTF-8 / GB18030 / Windows-1252 / ISO-8859-1）。

## 核心快捷键
| 快捷键 | 功能 |
| :--- | :--- |
| `F` | 右侧统计量栏即时显示当前选中单元格/变量的统计摘要及具体取值分布 |
| `D` | 对选中单个或多个值执行 `=` / `IN` 快速筛选（数据查看区与统计量栏通用） |
| `Ctrl/Cmd + 点击` / `Shift + 点击` | 在数据区或统计量栏多选不连续 / 连续区间值 |
| `C` | 清除当前数据集所有筛选与排序 |
| `H` / `U` | 清空右侧变量栏勾选（隐藏全部）/ 全选恢复显示全部变量 |
| `Ctrl/Cmd + Shift + F` | 聚焦顶部 `Search` 单元格值搜索框（`Enter` / `F3` 下一个，`Shift+Enter` / `Shift+F3` 上一个） |
| `Ctrl/Cmd + F` | 聚焦右侧变量栏搜索框 |
| `Alt + ← / →` | 将选中变量列向左 / 向右移动位置 |
| `Ctrl/Cmd + C` | 复制当前选中单元格内容 |
| `Ctrl/Cmd + B` / `Ctrl/Cmd + \` | 收起 / 展开左侧文件栏 / 右侧信息栏 |
| `Ctrl/Cmd + / - / 0` | 放大 / 缩小 / 重置界面缩放（100% 低 DPI 屏默认 1.2x 清晰度放大，按显示器记忆） |

## 更新日志
- **`v0.3.2`**：新增专属应用 Logo（macOS `AppIcon.icns` / Windows `.exe` 图标 / 窗口与界面图标）；顶部栏新增「关于」圆圈 `i` 弹窗（展示软件简介、GitHub 仓库地址、作者、贡献者与个人微信公众号）；右侧变量栏新增「预设勾选变量」与「应用预设」（支持如 `STUDYID, USUBJID, SUBJID, SITEID` 批量设定并按数据集持久化跟随，直至在文件管理器中移除）；新增 GitHub Releases 自动与手动检查更新功能（集成于「关于」弹窗及顶部更新提示徽标）。
- **`v0.3.1`**：数据区表头新增原始变量编号行、右侧变量栏新增对应 `#` 编号列；统计量频数分布升级为三列表格（取值/频数/占比）并支持点击列头正倒序排序；右下角编码左侧新增全局显示缩放比例调节（默认 `100%`，`75%–200%` 适配多分辨率显示器并按显示器记忆）；低 DPI（100%）显示器默认 1.2x 清晰度放大（#4 / #5）。
- **`v0.3.0`**：右侧栏宽度锁定与紧凑 4 列变量表（变量 / 类型 / Format / 标签）；数据区与统计量栏统一支持多选及 `D` 键多值筛选、数值型全量取值罗列；新增 VS Code 风格 `Search`（字符/数值、`Aa`、`全字`、`.*` 正则）；6–30 字符自适应列宽与均匀重置列宽；行号保持原始顺序且点击 `#` 恢复初始顺序、列头仅点击变量名排序；拖入单文件自动导入所在目录、修复示例数据集生成、底部状态栏前缀显示目录名。
- **`v0.2.0`**：脱离 `eframe` 改为自建统一 `winit` 事件循环宿主（修复 Windows 拖拽冻结），拆分为 `macOS-arm64`（Metal）、`win-x64`（GPU + CPU 兜底）、`win-aws`（纯 CPU GDI）三端独立产物，并修复 macOS ad-hoc 签名打包。
- **`v0.1.x`**：初版发布（ReadStat + Rust 列式引擎 + 三栏 GUI），补充纯 CPU 软件渲染兼容无独立显卡的 AWS Windows Server。

## 构建与发布版本（三端独立产物）

本应用在 `v0.2.0` 起采用自建统一 `winit` 事件循环宿主（脱离 `eframe`，彻底消除 Windows 拖拽标题栏/边框模态循环冻结，见 #3），共享 100% 同一套底层 C (`ReadStat`) / Rust 数据引擎与三栏 UI 代码，仅在编译期通过 Cargo Feature 产出三个专用发行版：

| 发布产物 | 适用场景 | 渲染架构 | 特点 |
| :--- | :--- | :--- | :--- |
| **`SAS-Dataset-Viewer-macOS-arm64.zip`** | macOS (Apple Silicon) | 原生 **Apple Metal** (`backend-wgpu`) | 剔除 OpenGL/软渲染冗余，解压即得 `.app` |
| **`SAS-Dataset-Viewer-win-x64.exe`** | 本地 Windows 10/11 电脑 | 桌面 GPU（`glow` / `wgpu`）+ CPU 兜底 | 跳过纯平移无效重绘与缩放事件合并，流畅高刷，静态 CRT |
| **`SAS-Dataset-Viewer-win-aws.exe`** | AWS EC2 / Windows Server / RDP | **纯多核 CPU Win32 GDI** (`backend-cpu`) | 零 GPU/DX12/OpenGL/VC++ 依赖，单进程直启秒开，极致精简 |

> **macOS 首次打开提示说明**：由于未购买 Apple Developer 年度付费签名公证，浏览器从 GitHub 下载的压缩包会被系统打上隔离标记（`com.apple.quarantine`）。若首次双击提示“无法验证开发者”或“已损坏”，在终端执行以下命令清除下载隔离属性即可永久正常双击打开：
> ```bash
> xattr -cr /path/to/"SAS Dataset Viewer.app"
> ```
> （若已拖入“应用程序”目录，执行 `xattr -cr /Applications/"SAS Dataset Viewer.app"`。）

### 本地构建命令

```bash
# 1. macOS Apple Silicon (.app)
bash scripts/bundle_macos.sh

# 2. Windows 桌面端 (win-x64)
cargo build --release --bin sas-viewer --no-default-features --features "backend-glow,backend-wgpu,backend-cpu"

# 3. Windows AWS 无显卡服务器专用版 (win-aws)
cargo build --release --bin sas-viewer --no-default-features --features backend-cpu
```
也可以通过仓库内置的 `.github/workflows/release.yml` 在 GitHub Actions 上自动同时构建并发布全部三个版本。

## 贡献者（Contributors）
感谢以下开发者为本项目做出的贡献：

- [@allensrj](https://github.com/allensrj) — 项目发起人、核心架构与功能开发
- [@wtwang1998](https://github.com/wtwang1998) (wentao) — Windows 拖拽冻结根因分析（#1）、默认渲染器修复（#2）、事件循环架构 RFC 与实测验证（#3）及低 DPI 屏文字清晰度优化（#4 / #5）

## 版权与开源协议（License）
Copyright © 2026 allensrj

本项目基于 [Apache License 2.0](LICENSE) 开源协议发布。您可以自由使用、修改和分发本软件，但需保留原始版权声明、协议文本及 [NOTICE](NOTICE) 中的归属声明，且需对修改部分作出说明。

第三方组件声明：本项目内置的底层解析库 [`ReadStat`](https://github.com/WizardMac/ReadStat) 基于 MIT 协议开源（Copyright © 2013-2016 Evan Miller），详见 [NOTICE](NOTICE) 与 [`third_party/readstat/LICENSE`](third_party/readstat/LICENSE)。

