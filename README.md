# SAS 数据集查看器（XPT / SAS7BDAT）

基于纯 C（[`ReadStat`](https://github.com/WizardMac/ReadStat)，与 `pyreadstat` 同源）+ Rust 列式引擎 + 原生 GPU GUI（`egui` / `egui_dock`）构建的跨平台高速 SAS 数据集桌面查看器。

## 核心特性
- **极致性能**：毫秒级冷启动，两阶段流式加载（5,000 行首屏秒开 + 后台流式补齐），面向 SAS 特性优化的字典压缩列式存储与 `rayon` 多核并行筛选/排序/统计。实测 20 万行 `.sas7bdat` 完整加载 ~50ms、多列筛选排序 ~6ms、统计计算 ~1ms。
- **三栏纵向布局**：
  - **左侧文件管理器**：树形目录（仅显示 `.sas7bdat` 与 `.xpt`）、折叠/展开、软件内别名重命名、刷新、从侧栏移除（不删磁盘文件）、拖拽自定义排序（持久化）、拖入中间分屏打开、一键收起/展开。
  - **中间数据查看栏**：双层固定表头（变量名 + 标签）、多变量叠加排序（`Shift+点击` 或右键）、列头拖拽与快捷键左右移列、合理初始列宽 + 右键批量自适应列宽、单元格右键筛选（字符串 `=`/`!=`/`包含`/`不包含`/`为空`/`非空`；数值 `=`/`!=`/`>`/`>=`/`<`/`<=`/`区间`/`缺失`/`非缺失`）、多数据集标签页自由拖拽上下/左右并排分屏。
  - **右侧信息栏**：变量栏（勾选联动显示 + 变量名/标签双检索）、统计量栏（基于当前筛选数据的字符频数/百分比与数值描述统计）、快捷键说明栏。
- **SAS 专业细节**：支持 SAS 日期/时间格式（`DATE9.`, `YYMMDD10.`, `DATETIME20.`, `TIME8.` 等）按格式化日期展示同时保留数值排序/筛选逻辑；支持 `.`、`.A`~`.Z`、`._` 特殊缺失值；底部状态栏支持一键切换字符编码（自动检测 / UTF-8 / GB18030 / Windows-1252 / ISO-8859-1）。

## 核心快捷键
| 快捷键 | 功能 |
| :--- | :--- |
| `F` | 右侧统计量栏即时显示当前选中单元格或变量的统计信息 |
| `D` | 对选中单元格执行 `=` 快速筛选（不同列叠加 AND，同列替换） |
| `C` | 清除当前数据集所有筛选与排序 |
| `H` | 清空右侧变量栏勾选（隐藏所有变量） |
| `U` | 全选右侧变量栏（恢复显示所有变量） |
| `Alt + ← / →` | 将选中变量列向左 / 向右移动位置 |
| `Ctrl/Cmd + C` | 复制当前选中单元格内容 |
| `Ctrl/Cmd + F` | 聚焦右侧变量搜索框 |
| `Ctrl/Cmd + B` | 收起 / 展开左侧文件管理器 |
| `Ctrl/Cmd + \` | 收起 / 展开最右侧信息栏 |

## 构建与发布版本（三端独立产物）

本应用在 `v0.2.0` 起采用自建统一 `winit` 事件循环宿主（脱离 `eframe`，彻底消除 Windows 拖拽标题栏/边框模态循环冻结，见 #3），共享 100% 同一套底层 C (`ReadStat`) / Rust 数据引擎与三栏 UI 代码，仅在编译期通过 Cargo Feature 产出三个专用发行版：

| 发布产物 | 适用场景 | 渲染架构 | 特点 |
| :--- | :--- | :--- | :--- |
| **`SAS-Dataset-Viewer-macOS-arm64.zip`** | macOS (Apple Silicon) | 原生 **Apple Metal** (`backend-wgpu`) | 剔除 OpenGL/软渲染冗余，解压即得 `.app` |
| **`SAS-Dataset-Viewer-win-x64.exe`** | 本地 Windows 10/11 电脑 | 桌面 GPU（`glow` / `wgpu`）+ CPU 兜底 | 跳过纯平移无效重绘与缩放事件合并，流畅高刷，静态 CRT |
| **`SAS-Dataset-Viewer-win-aws.exe`** | AWS EC2 / Windows Server / RDP | **纯多核 CPU Win32 GDI** (`backend-cpu`) | 零 GPU/DX12/OpenGL/VC++ 依赖，单进程直启秒开，极致精简 |

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
- [@wtwang1998](https://github.com/wtwang1998) (wentao) — Windows 拖拽冻结根因分析（#1）、默认渲染器修复（#2）及事件循环架构 RFC（#3）

