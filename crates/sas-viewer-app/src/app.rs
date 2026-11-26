use crate::assets::AppTextures;
use crate::config::{parse_var_preset_input, AppConfig};
use crate::panels::data_grid::{show_data_grid, GridAction};
use crate::panels::file_tree::{format_file_size, FileTreeAction, FileTreeState, OpenSplitMode};
use crate::panels::right_panel::{show_right_panel, RightPanelAction};
use crate::sample_gen::{generate_sample_datasets, resolve_sample_dir};
use crate::state::{DatasetTabState, FocusRegion, RightTab};
use crate::theme::{apply_theme, setup_cjk_fonts, AppPalette};
use crate::updater::{UpdateCheckState, UpdateChecker, APP_VERSION, RELEASES_URL, REPO_URL};
use egui::{Align2, Color32, FontId, Key, OpenUrl, Pos2, RichText, Sense, Stroke, Ui, Vec2};
use egui_dock::{DockArea, DockState, NodeIndex, Style as DockStyle, SurfaceIndex, TabViewer};
use sas_engine::{ColumnStats, EncodingChoice, SasFormatKind};
use std::collections::HashMap;
use std::path::{Path, PathBuf};

/// Monitor DPI baseline passed from `WindowHost` so the bottom-right display scale
/// control starts cleanly at `100%` on every monitor while cooperating with `ZoomPrefs`.
#[derive(Debug, Clone, Copy)]
pub struct DisplayScaleContext {
    pub native_key: i32,
    pub base_zoom: f32,
}

const DISPLAY_SCALE_PRESETS: &[(f32, &str)] = &[
    (0.75, "75%"),
    (0.85, "85%"),
    (0.90, "90%"),
    (1.00, "100% (默认)"),
    (1.10, "110%"),
    (1.20, "120%"),
    (1.25, "125%"),
    (1.40, "140%"),
    (1.50, "150%"),
    (1.75, "175%"),
    (2.00, "200%"),
];

#[derive(Debug, Clone)]
struct VarPresetDialogState {
    tab_id: u64,
    dataset_path: PathBuf,
    dataset_name: String,
    input_text: String,
    just_opened: bool,
}

pub struct SasViewerApp {
    config: AppConfig,
    file_tree: FileTreeState,
    dock_state: DockState<u64>,
    tabs: HashMap<u64, DatasetTabState>,
    next_tab_id: u64,
    active_tab_id: Option<u64>,
    right_tab: RightTab,
    focus_var_search: bool,
    focus_cell_search: bool,
    status_toast: Option<(String, std::time::Instant)>,
    textures: AppTextures,
    about_open: bool,
    updater: UpdateChecker,
    var_preset_dialog: Option<VarPresetDialogState>,
}

impl SasViewerApp {
    pub fn from_ctx(egui_ctx: &egui::Context, initial_paths: Vec<PathBuf>) -> Self {
        setup_cjk_fonts(egui_ctx);
        let config = AppConfig::load();
        apply_theme(egui_ctx, config.dark_mode);

        let mut app = Self {
            config,
            file_tree: FileTreeState::default(),
            dock_state: DockState::new(Vec::new()),
            tabs: HashMap::new(),
            next_tab_id: 1,
            active_tab_id: None,
            right_tab: RightTab::Variables,
            focus_var_search: false,
            focus_cell_search: false,
            status_toast: None,
            textures: AppTextures::default(),
            about_open: false,
            updater: UpdateChecker::default(),
            var_preset_dialog: None,
        };

        for p in initial_paths {
            app.ingest_dropped_path(p, Some(egui_ctx.clone()));
        }
        app.file_tree.refresh_all(&app.config);
        app.updater.start_check(false, Some(egui_ctx.clone()));
        app
    }

    pub fn saved_monitor_ui_scales(&self) -> &HashMap<i32, f32> {
        &self.config.monitor_ui_scales
    }

    fn toast(&mut self, msg: impl Into<String>) {
        self.status_toast = Some((msg.into(), std::time::Instant::now()));
    }

    /// Dropping a folder adds that folder; dropping a single `.xpt` / `.sas7bdat` file
    /// automatically adds its parent folder (including sibling datasets) AND opens the file!
    fn ingest_dropped_path(&mut self, path: PathBuf, ctx: Option<egui::Context>) {
        if path.is_dir() {
            self.config.add_root_folder(path.clone());
            self.file_tree.refresh_root(&path, &self.config);
            self.toast(format!("已添加目录: {}", self.config.display_name_for(&path)));
        } else if path.is_file() && SasFormatKind::from_path(&path).is_some() {
            if let Some(parent) = path.parent() {
                let parent_buf = parent.to_path_buf();
                self.config.add_root_folder(parent_buf.clone());
                self.file_tree.refresh_root(&parent_buf, &self.config);
            } else {
                self.config.add_root_file(path.clone());
            }
            self.open_dataset(path, OpenSplitMode::Tab, ctx);
        }
    }

    fn open_dataset(&mut self, path: PathBuf, mode: OpenSplitMode, ctx: Option<egui::Context>) {
        if SasFormatKind::from_path(&path).is_none() {
            self.toast("仅支持 .sas7bdat 和 .xpt 数据集文件");
            return;
        }

        if mode == OpenSplitMode::Tab {
            if let Some((&existing_id, _)) = self.tabs.iter().find(|(_, t)| t.path == path) {
                if let Some((surf, node, tab_idx)) = self.dock_state.find_tab(&existing_id) {
                    self.dock_state.set_active_tab((surf, node, tab_idx));
                    self.dock_state.set_focused_node_and_surface((surf, node));
                    self.active_tab_id = Some(existing_id);
                    return;
                }
            }
        }

        let id = self.next_tab_id;
        self.next_tab_id += 1;
        let disp_name = self.config.display_name_for(&path);
        let tab = DatasetTabState::new(id, path, disp_name, EncodingChoice::Auto, ctx);
        self.tabs.insert(id, tab);

        let focused_node = self
            .dock_state
            .focused_leaf()
            .filter(|(s, _)| *s == SurfaceIndex::main())
            .map(|(_, n)| n);

        match mode {
            OpenSplitMode::Tab => {
                if self.dock_state.iter_all_tabs().next().is_none() {
                    self.dock_state = DockState::new(vec![id]);
                } else {
                    self.dock_state.push_to_focused_leaf(id);
                }
            }
            OpenSplitMode::SplitRight => {
                if let Some(node) = focused_node {
                    self.dock_state
                        .main_surface_mut()
                        .split_right(node, 0.5, vec![id]);
                } else if self.dock_state.iter_all_tabs().next().is_none() {
                    self.dock_state = DockState::new(vec![id]);
                } else {
                    self.dock_state
                        .main_surface_mut()
                        .split_right(NodeIndex::root(), 0.5, vec![id]);
                }
            }
            OpenSplitMode::SplitBottom => {
                if let Some(node) = focused_node {
                    self.dock_state
                        .main_surface_mut()
                        .split_below(node, 0.5, vec![id]);
                } else if self.dock_state.iter_all_tabs().next().is_none() {
                    self.dock_state = DockState::new(vec![id]);
                } else {
                    self.dock_state
                        .main_surface_mut()
                        .split_below(NodeIndex::root(), 0.5, vec![id]);
                }
            }
        }

        self.active_tab_id = Some(id);
    }

    fn pick_folder_and_add(&mut self) {
        if let Some(folder) = rfd::FileDialog::new()
            .set_title("选择包含 SAS / XPT 数据集的文件夹")
            .pick_folder()
        {
            self.config.add_root_folder(folder.clone());
            self.file_tree.refresh_root(&folder, &self.config);
            self.toast(format!("已添加目录: {}", self.config.display_name_for(&folder)));
        }
    }

    fn pick_files_and_open(&mut self, ctx: egui::Context) {
        if let Some(files) = rfd::FileDialog::new()
            .set_title("打开 SAS / XPT 数据集")
            .add_filter("SAS 数据集 (*.sas7bdat, *.xpt)", &["sas7bdat", "xpt"])
            .pick_files()
        {
            for f in files {
                self.ingest_dropped_path(f, Some(ctx.clone()));
            }
        }
    }

    fn generate_and_load_samples(&mut self, ctx: egui::Context) {
        let sample_dir = resolve_sample_dir();
        match generate_sample_datasets(&sample_dir) {
            Ok(files) => {
                self.config.add_root_folder(sample_dir.clone());
                self.file_tree.refresh_root(&sample_dir, &self.config);
                if let Some(first) = files.first() {
                    self.open_dataset(first.clone(), OpenSplitMode::Tab, Some(ctx.clone()));
                }
                if let Some(second) = files.get(1) {
                    self.open_dataset(second.clone(), OpenSplitMode::SplitRight, Some(ctx));
                }
                self.toast(format!(
                    "已生成示例数据集至 {}（含 dm / ae / 20万行 lb_200k）",
                    sample_dir.display()
                ));
            }
            Err(e) => {
                self.toast(format!("生成示例失败: {e}"));
                let _ = rfd::MessageDialog::new()
                    .set_level(rfd::MessageLevel::Error)
                    .set_title("生成示例数据集失败")
                    .set_description(&format!("无法在 {} 生成示例数据集：\n{}", sample_dir.display(), e))
                    .show();
            }
        }
    }

    fn handle_os_dropped_files(&mut self, ctx: &egui::Context) {
        let dropped: Vec<PathBuf> = ctx.input(|i| {
            i.raw
                .dropped_files
                .iter()
                .filter_map(|f| f.path.clone())
                .collect()
        });
        for p in dropped {
            self.ingest_dropped_path(p, Some(ctx.clone()));
        }
    }

    /// Unified keyboard shortcut dispatcher connecting File Tree, Data Grid, and Right Panel.
    fn handle_shortcuts(&mut self, ctx: &egui::Context) {
        let typing_in_text = ctx.wants_keyboard_input();

        let (cmd_b, cmd_backslash, cmd_f, cmd_shift_f, cmd_c, cmd_w, f3_next, f3_prev) = ctx.input(|i| {
            let cmd = i.modifiers.command;
            let shift = i.modifiers.shift;
            (
                cmd && i.key_pressed(Key::B),
                cmd && i.key_pressed(Key::Backslash),
                cmd && !shift && i.key_pressed(Key::F),
                cmd && shift && i.key_pressed(Key::F),
                cmd && i.key_pressed(Key::C),
                cmd && i.key_pressed(Key::W),
                !shift && i.key_pressed(Key::F3),
                shift && i.key_pressed(Key::F3),
            )
        });

        if cmd_b {
            self.config.left_panel_open = !self.config.left_panel_open;
            self.config.save();
        }
        if cmd_backslash {
            self.config.right_panel_open = !self.config.right_panel_open;
            self.config.save();
        }
        if cmd_f {
            self.config.right_panel_open = true;
            self.right_tab = RightTab::Variables;
            self.focus_var_search = true;
        }
        if cmd_shift_f {
            self.focus_cell_search = true;
        }
        if f3_next || f3_prev {
            if let Some(id) = self.active_tab_id {
                if let Some(tab) = self.tabs.get_mut(&id) {
                    if f3_prev {
                        tab.search_prev();
                    } else {
                        tab.search_next();
                    }
                }
            }
        }
        if cmd_c && !typing_in_text {
            if let Some(id) = self.active_tab_id {
                if let Some(tab) = self.tabs.get(&id) {
                    if let Some(text) = tab.selected_cell_text() {
                        ctx.copy_text(text);
                        self.toast("已复制单元格内容");
                    }
                }
            }
        }
        if cmd_w && !typing_in_text {
            if let Some(id) = self.active_tab_id {
                if let Some((surf, node, idx)) = self.dock_state.find_tab(&id) {
                    self.dock_state.remove_tab((surf, node, idx));
                    self.tabs.remove(&id);
                    self.sync_active_tab_from_dock();
                }
            }
        }

        if typing_in_text || self.about_open || self.var_preset_dialog.is_some() {
            if ctx.input(|i| i.key_pressed(Key::Escape)) {
                if self.var_preset_dialog.is_some() {
                    self.var_preset_dialog = None;
                } else if self.about_open {
                    self.about_open = false;
                }
            }
            return;
        }

        let Some(active_id) = self.active_tab_id else {
            return;
        };
        if !self.tabs.contains_key(&active_id) {
            return;
        }

        let (
            key_f,
            key_d,
            key_c,
            key_h,
            key_u,
            alt_left,
            alt_right,
            arrow_up,
            arrow_down,
            arrow_left,
            arrow_right,
            page_up,
            page_down,
        ) = ctx.input(|i| {
            let no_mod = !i.modifiers.command && !i.modifiers.ctrl && !i.modifiers.alt;
            let alt_only = i.modifiers.alt && !i.modifiers.command && !i.modifiers.ctrl;
            (
                no_mod && i.key_pressed(Key::F),
                no_mod && i.key_pressed(Key::D),
                no_mod && i.key_pressed(Key::C),
                no_mod && i.key_pressed(Key::H),
                no_mod && i.key_pressed(Key::U),
                alt_only && i.key_pressed(Key::ArrowLeft),
                alt_only && i.key_pressed(Key::ArrowRight),
                no_mod && i.key_pressed(Key::ArrowUp),
                no_mod && i.key_pressed(Key::ArrowDown),
                no_mod && i.key_pressed(Key::ArrowLeft),
                no_mod && i.key_pressed(Key::ArrowRight),
                no_mod && i.key_pressed(Key::PageUp),
                no_mod && i.key_pressed(Key::PageDown),
            )
        });

        let mut switch_to_stats = false;
        let mut toast_msg: Option<String> = None;

        if let Some(tab) = self.tabs.get_mut(&active_id) {
            // F: show stats + value frequencies in Right Statistics panel
            if key_f {
                if let Some(col) = tab.active_target_col() {
                    tab.refresh_stats_for(col);
                    switch_to_stats = true;
                } else {
                    toast_msg = Some("请先选中一个单元格或变量".to_string());
                }
            }

            // D: unified `=` / multi-value `IN` quick filter (works in DataGrid AND StatsList!)
            if key_d {
                if let Some(msg) = tab.trigger_quick_d_filter() {
                    toast_msg = Some(msg);
                }
            }

            // C: clear all filters and sorts
            if key_c {
                tab.clear_all_filters_and_sorts();
                toast_msg = Some("已清除全部筛选与排序 (C)".to_string());
            }

            // H: hide all variables
            if key_h {
                tab.hide_all_variables();
                toast_msg = Some("已隐藏所有变量 (按 U 恢复)".to_string());
            }

            // U: show all variables
            if key_u {
                tab.show_all_variables();
                toast_msg = Some("已全选恢复所有变量 (U)".to_string());
            }

            // Alt + Left / Right: move selected variable column
            if alt_left {
                if let Some(col) = tab.active_target_col() {
                    tab.move_column_by(col, -1);
                }
            }
            if alt_right {
                if let Some(col) = tab.active_target_col() {
                    tab.move_column_by(col, 1);
                }
            }

            // Arrow navigation: route to StatsList when focused there, else DataGrid!
            if (arrow_up || arrow_down) && tab.focus_region == FocusRegion::StatsList {
                let len = match &tab.cached_stats {
                    Some(ColumnStats::String(ss)) => ss.frequencies.len(),
                    Some(ColumnStats::Numeric(ns)) => ns.frequencies.len(),
                    None => 0,
                };
                if len > 0 {
                    let cur = tab.stats_anchor_idx.unwrap_or(0).min(len - 1);
                    let next = if arrow_up {
                        cur.saturating_sub(1)
                    } else {
                        (cur + 1).min(len - 1)
                    };
                    tab.stats_selected_indices.clear();
                    tab.stats_selected_indices.insert(next);
                    tab.stats_anchor_idx = Some(next);
                }
            } else if arrow_up || arrow_down || arrow_left || arrow_right || page_up || page_down {
                let vis_cols = tab.visible_columns();
                let nrows = tab.visible_rows.len();
                if nrows > 0 && !vis_cols.is_empty() {
                    let (cur_r, cur_c_vis) = match tab.selected_cell {
                        Some((r, orig_c)) => {
                            let cv = vis_cols.iter().position(|&c| c == orig_c).unwrap_or(0);
                            (r.min(nrows - 1), cv)
                        }
                        None => (0, 0),
                    };
                    let new_r = if arrow_up {
                        cur_r.saturating_sub(1)
                    } else if arrow_down {
                        (cur_r + 1).min(nrows - 1)
                    } else if page_up {
                        cur_r.saturating_sub(25)
                    } else if page_down {
                        (cur_r + 25).min(nrows - 1)
                    } else {
                        cur_r
                    };
                    let new_c_vis = if arrow_left {
                        cur_c_vis.saturating_sub(1)
                    } else if arrow_right {
                        (cur_c_vis + 1).min(vis_cols.len() - 1)
                    } else {
                        cur_c_vis
                    };
                    let new_orig_col = vis_cols[new_c_vis];
                    tab.select_single_cell(new_r, new_orig_col);
                    tab.scroll_to_row = Some(new_r);
                    tab.scroll_to_col = Some(new_orig_col);
                }
            }
        }

        if switch_to_stats {
            self.config.right_panel_open = true;
            self.right_tab = RightTab::Statistics;
        }
        if let Some(msg) = toast_msg {
            self.toast(msg);
        }
    }

    fn sync_active_tab_from_dock(&mut self) {
        if let Some((_, tab_id)) = self.dock_state.find_active_focused() {
            self.active_tab_id = Some(*tab_id);
        } else if let Some((_, &first_id)) = self.dock_state.iter_all_tabs().next() {
            self.active_tab_id = Some(first_id);
        } else {
            self.active_tab_id = None;
        }
    }
}

struct DatasetDockViewer<'a> {
    tabs: &'a mut HashMap<u64, DatasetTabState>,
    active_tab_id: &'a mut Option<u64>,
    pal: AppPalette,
    pending_grid_actions: Vec<(u64, GridAction)>,
    closed_tabs: Vec<u64>,
}

impl<'a> TabViewer for DatasetDockViewer<'a> {
    type Tab = u64;

    fn title(&mut self, tab: &mut Self::Tab) -> egui::WidgetText {
        if let Some(state) = self.tabs.get(tab) {
            let prefix = if state.loading { "⏳ " } else { "" };
            let suffix = if !state.query.is_empty() { " ●" } else { "" };
            format!("{}{}{}", prefix, state.display_name, suffix).into()
        } else {
            format!("Dataset #{tab}").into()
        }
    }

    fn ui(&mut self, ui: &mut Ui, tab: &mut Self::Tab) {
        let tab_id = *tab;
        if ui.rect_contains_pointer(ui.max_rect())
            && ui.input(|i| i.pointer.any_pressed() || i.pointer.any_click())
        {
            *self.active_tab_id = Some(tab_id);
        }

        let is_active = *self.active_tab_id == Some(tab_id);
        if let Some(state) = self.tabs.get_mut(&tab_id) {
            let acts = show_data_grid(ui, state, &self.pal, is_active);
            for a in acts {
                self.pending_grid_actions.push((tab_id, a));
            }
        }
    }

    fn on_close(&mut self, tab: &mut Self::Tab) -> bool {
        self.closed_tabs.push(*tab);
        true
    }
}

impl SasViewerApp {
    pub fn update_ui(&mut self, ctx: &egui::Context, display_scale: DisplayScaleContext) {
        let mut any_loading = false;
        for tab in self.tabs.values_mut() {
            tab.poll_loader();
            if tab.loading {
                any_loading = true;
            }
        }
        self.updater.poll();
        if any_loading {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        } else if self.updater.is_checking() {
            ctx.request_repaint_after(std::time::Duration::from_millis(100));
        }

        self.handle_os_dropped_files(ctx);
        self.handle_shortcuts(ctx);

        let pal = AppPalette::for_mode(self.config.dark_mode);
        let logo_tex_id = self.textures.app_logo(ctx).map(|t| t.id());

        // Top compact toolbar (streamlined: app logo + panel toggles + dataset Search + theme + About circle-i)
        egui::TopBottomPanel::top("top_toolbar")
            .exact_height(34.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(tex_id) = logo_tex_id {
                        let logo_resp = ui
                            .add(
                                egui::Image::new(egui::load::SizedTexture::new(
                                    tex_id,
                                    Vec2::splat(22.0),
                                ))
                                .sense(Sense::click()),
                            )
                            .on_hover_text(format!(
                                "SAS 数据集查看器 v{APP_VERSION}\n点击查看关于与检查更新"
                            ));
                        if logo_resp.hovered() {
                            ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
                        }
                        if logo_resp.clicked() {
                            self.about_open = true;
                        }
                    }

                    let left_lbl = if self.config.left_panel_open {
                        "◧ 隐藏文件栏"
                    } else {
                        "◧ 显示文件栏"
                    };
                    if ui
                        .button(left_lbl)
                        .on_hover_text("收起/展开左侧文件管理器 (Ctrl/Cmd+B)")
                        .clicked()
                    {
                        self.config.left_panel_open = !self.config.left_panel_open;
                        self.config.save();
                    }
                    ui.separator();

                    if ui
                        .button("⚡ 生成示例数据集")
                        .on_hover_text("在本地可写目录生成 dm.sas7bdat / ae.xpt / 20万行 lb_200k.sas7bdat")
                        .clicked()
                    {
                        self.generate_and_load_samples(ctx.clone());
                    }

                    if let Some(active_id) = self.active_tab_id {
                        if let Some(tab) = self.tabs.get_mut(&active_id) {
                            ui.separator();
                            if ui
                                .button("自适应列宽")
                                .on_hover_text("批量调整所有可见列宽至数据实际长度（约 6~30 字符）")
                                .clicked()
                            {
                                tab.auto_fit_all_visible_columns();
                            }
                            if ui
                                .button("重置列宽")
                                .on_hover_text("恢复均匀平衡的默认列宽")
                                .clicked()
                            {
                                tab.reset_all_column_widths();
                            }

                            ui.separator();
                            // VS Code-style Dataset Cell Search bar (strings + numbers + Aa / Whole Word / Regex)
                            ui.spacing_mut().item_spacing.x = 3.0;
                            ui.label(RichText::new("Search").strong().size(12.0).color(pal.text_muted));
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut tab.search_query)
                                    .hint_text("查找字符/数值... (Ctrl+Shift+F)")
                                    .desired_width(142.0),
                            );
                            if self.focus_cell_search {
                                resp.request_focus();
                                self.focus_cell_search = false;
                            }

                            let mut search_dirty = resp.changed();
                            if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                if ui.input(|i| i.modifiers.shift) {
                                    tab.search_prev();
                                } else {
                                    tab.search_next();
                                }
                                resp.request_focus();
                            } else if resp.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                                tab.clear_cell_search();
                                resp.surrender_focus();
                                search_dirty = false;
                            }

                            if search_option_toggle(
                                ui,
                                tab.search_options.match_case,
                                "Aa",
                                "区分大小写 (Match Case)",
                                &pal,
                            ) {
                                tab.search_options.match_case = !tab.search_options.match_case;
                                search_dirty = true;
                            }
                            if search_option_toggle(
                                ui,
                                tab.search_options.whole_word,
                                "全字",
                                "全字匹配 (Match Whole Word)\n字符串按完整词边界匹配；数值按精确值匹配（如搜 65 匹配 65 / 65.00，不匹配 65.3 或 165）",
                                &pal,
                            ) {
                                tab.search_options.whole_word = !tab.search_options.whole_word;
                                search_dirty = true;
                            }
                            if search_option_toggle(
                                ui,
                                tab.search_options.use_regex,
                                ".*",
                                "使用正则表达式 (Regular Expression)\n支持 ^ $ . * + ? | () [] \\d \\w \\s \\b",
                                &pal,
                            ) {
                                tab.search_options.use_regex = !tab.search_options.use_regex;
                                search_dirty = true;
                            }

                            if search_dirty {
                                if tab.search_query.is_empty() {
                                    tab.clear_cell_search();
                                } else {
                                    tab.run_cell_search(true);
                                }
                            }

                            if !tab.search_query.is_empty()
                                && ui.small_button("×").on_hover_text("清空搜索 (Esc)").clicked()
                            {
                                tab.clear_cell_search();
                            }
                            if ui
                                .small_button("↑")
                                .on_hover_text("上一个匹配 (Shift+Enter / Shift+F3)")
                                .clicked()
                            {
                                tab.search_prev();
                            }
                            if ui
                                .small_button("↓")
                                .on_hover_text("下一个匹配 (Enter / F3)")
                                .clicked()
                            {
                                tab.search_next();
                            }

                            if let Some(err) = &tab.search_error {
                                ui.label(
                                    RichText::new("正则无效")
                                        .size(11.0)
                                        .strong()
                                        .color(Color32::from_rgb(205, 70, 70)),
                                )
                                .on_hover_text(err);
                            } else if !tab.search_executed_query.is_empty() {
                                let match_txt = if tab.search_matches.is_empty() {
                                    "0 匹配".to_string()
                                } else {
                                    let cur = tab.search_current_idx.map(|i| i + 1).unwrap_or(0);
                                    let suffix = if tab.search_truncated { "+" } else { "" };
                                    format!("{}/{}{}", cur, tab.search_matches.len(), suffix)
                                };
                                ui.label(
                                    RichText::new(match_txt)
                                        .monospace()
                                        .size(11.2)
                                        .color(if tab.search_matches.is_empty() {
                                            Color32::from_rgb(205, 70, 70)
                                        } else {
                                            pal.accent
                                        }),
                                );
                            }

                            if !tab.query.is_empty() {
                                ui.separator();
                                if ui
                                    .button(RichText::new("清除筛选/排序 (C)").color(pal.accent))
                                    .clicked()
                                {
                                    tab.clear_all_filters_and_sorts();
                                }
                            }
                        }
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let update_info = self.updater.available_update().cloned();
                        if about_circle_i_button(ui, &pal, update_info.as_ref()) {
                            self.about_open = !self.about_open;
                        }

                        if let Some(info) = self.updater.should_show_toolbar_badge().cloned() {
                            let badge = egui::Button::new(
                                RichText::new(format!("⬆ 新版本 {}", info.tag_name))
                                    .size(11.0)
                                    .strong()
                                    .color(Color32::WHITE),
                            )
                            .small()
                            .fill(Color32::from_rgb(217, 119, 6))
                            .stroke(Stroke::NONE);
                            if ui
                                .add(badge)
                                .on_hover_text(format!(
                                    "检测到新版本 {}（当前 v{}），点击查看并前往 GitHub Release 下载",
                                    info.tag_name, APP_VERSION
                                ))
                                .clicked()
                            {
                                self.about_open = true;
                            }
                        }

                        let right_lbl = if self.config.right_panel_open {
                            "隐藏右侧栏 ◨"
                        } else {
                            "显示右侧栏 ◨"
                        };
                        if ui
                            .button(right_lbl)
                            .on_hover_text("收起/展开右侧变量/统计/快捷键栏 (Ctrl/Cmd+\\)")
                            .clicked()
                        {
                            self.config.right_panel_open = !self.config.right_panel_open;
                            self.config.save();
                        }

                        let theme_lbl = if self.config.dark_mode {
                            "☀ 浅色"
                        } else {
                            "🌙 深色"
                        };
                        if ui.button(theme_lbl).clicked() {
                            self.config.dark_mode = !self.config.dark_mode;
                            apply_theme(ctx, self.config.dark_mode);
                            self.config.save();
                        }
                    });
                });
            });

        // Bottom Status Bar (folder prefix + metrics | display scale selector + Encoding Switcher!)
        egui::TopBottomPanel::bottom("bottom_status_bar")
            .exact_height(26.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    let mut active_enc_state: Option<(u64, EncodingChoice)> = None;

                    if let Some(active_id) = self.active_tab_id {
                        if let Some(tab) = self.tabs.get(&active_id) {
                            let title_with_dir = if let Some(parent) = tab.path.parent() {
                                let dir_disp = self.config.display_name_for(parent);
                                if !dir_disp.is_empty() {
                                    format!("{} - {}", dir_disp, tab.display_name)
                                } else {
                                    tab.display_name.clone()
                                }
                            } else {
                                tab.display_name.clone()
                            };

                            if let Some(table) = &tab.table {
                                let row_info = if tab.query.filters.is_empty() {
                                    format!("{} 行", table.row_count)
                                } else {
                                    format!("筛选 {} / {} 行", tab.visible_rows.len(), table.row_count)
                                };
                                let vis_c = tab.visible_columns().len();
                                let mem_mb =
                                    table.approx_memory_bytes() as f64 / (1024.0 * 1024.0);
                                ui.label(
                                    RichText::new(format!(
                                        "{} [{}]  |  {} × {}/{} 列  |  文件: {}  |  内存: {:.1} MB  |  加载: {}ms  |  查询: {}ms",
                                        title_with_dir,
                                        table.metadata.format_kind.display_badge(),
                                        row_info,
                                        vis_c,
                                        table.variables.len(),
                                        format_file_size(table.metadata.file_size_bytes),
                                        mem_mb,
                                        table.load_elapsed_ms,
                                        tab.last_query_ms,
                                    ))
                                    .size(11.3)
                                    .color(pal.text_muted),
                                );
                                active_enc_state = Some((active_id, tab.encoding));
                            } else if tab.loading {
                                ui.label(
                                    RichText::new(format!("正在加载 {} ...", title_with_dir))
                                        .size(11.3)
                                        .color(pal.accent),
                                );
                            }
                        }
                    } else {
                        ui.label(
                            RichText::new(
                                "就绪 | 直接拖入任意 .sas7bdat / .xpt 文件将自动导入同目录全部数据集 | 快捷键: F(统计) D(筛选) C(清空) H(全隐) U(全显)",
                            )
                            .size(11.3)
                            .color(pal.text_muted),
                        );
                    }

                    if let Some((msg, ts)) = &self.status_toast {
                        if ts.elapsed().as_secs_f32() < 3.5 {
                            ui.separator();
                            ui.label(RichText::new(msg).size(11.3).strong().color(pal.accent));
                        } else {
                            self.status_toast = None;
                        }
                    }

                    // Bottom-right controls (right_to_left): [Encoding]  [Display Scale / 缩放: 100%]
                    let mut new_encoding: Option<(u64, EncodingChoice)> = None;
                    let mut new_ui_scale: Option<f32> = None;

                    ui.with_layout(
                        egui::Layout::right_to_left(egui::Align::Center),
                        |ui| {
                            if let Some((active_id, cur_enc)) = active_enc_state {
                                let mut chosen_enc = cur_enc;
                                egui::ComboBox::from_id_salt(("enc_combo", active_id))
                                    .width(118.0)
                                    .selected_text(format!("编码: {}", chosen_enc.label()))
                                    .show_ui(ui, |ui| {
                                        for &enc in EncodingChoice::ALL {
                                            ui.selectable_value(
                                                &mut chosen_enc,
                                                enc,
                                                enc.label(),
                                            );
                                        }
                                    });
                                if chosen_enc != cur_enc {
                                    new_encoding = Some((active_id, chosen_enc));
                                }
                            }

                            let base = display_scale.base_zoom.max(0.5);
                            let cur_scale = (ctx.zoom_factor() / base).clamp(0.5, 3.0);
                            let cur_pct = (cur_scale * 100.0).round() as i32;
                            let mut chosen_pct = cur_pct;
                            egui::ComboBox::from_id_salt("ui_display_scale_combo")
                                .width(88.0)
                                .selected_text(format!("缩放: {}%", cur_pct))
                                .show_ui(ui, |ui| {
                                    for &(scale, label) in DISPLAY_SCALE_PRESETS {
                                        let pct = (scale * 100.0).round() as i32;
                                        if ui
                                            .selectable_value(&mut chosen_pct, pct, label)
                                            .clicked()
                                        {
                                            new_ui_scale = Some(scale);
                                        }
                                    }
                                })
                                .response
                                .on_hover_text(
                                    "调整整个软件的显示比例与字号大小，适配不同分辨率显示器\n初始 100%，按显示器自动记忆（亦可用 Ctrl/Cmd + / - / 0 微调）",
                                );
                        },
                    );

                    if let Some((tab_id, enc)) = new_encoding {
                        if let Some(tab) = self.tabs.get_mut(&tab_id) {
                            tab.reload_with_encoding(enc, ctx.clone());
                        }
                    }
                    if let Some(scale) = new_ui_scale {
                        let target_zoom = (display_scale.base_zoom * scale).clamp(0.5, 3.0);
                        ctx.set_zoom_factor(target_zoom);
                        self.config
                            .set_ui_scale_for_monitor(display_scale.native_key, scale);
                        self.toast(format!(
                            "界面显示比例已调整为 {}%",
                            (scale * 100.0).round() as i32
                        ));
                    }
                });
            });

        // Left Panel: Collapsible File Manager
        if self.config.left_panel_open {
            let left_resp = egui::SidePanel::left("left_file_manager")
                .resizable(true)
                .default_width(self.config.left_panel_width.clamp(160.0, 520.0))
                .width_range(160.0..=520.0)
                .show(ctx, |ui| {
                    let panel_rect = ui.max_rect();
                    ui.set_min_width(panel_rect.width());
                    let mut child_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(panel_rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    child_ui.set_clip_rect(panel_rect);
                    let tree_actions = self.file_tree.show(&mut child_ui, &mut self.config, &pal);
                    for act in tree_actions {
                        match act {
                            FileTreeAction::OpenFile { path, mode } => {
                                self.open_dataset(path, mode, Some(ctx.clone()));
                            }
                            FileTreeAction::AddFolderDialog => self.pick_folder_and_add(),
                            FileTreeAction::OpenFileDialog => self.pick_files_and_open(ctx.clone()),
                            FileTreeAction::AliasChanged { path, new_name } => {
                                for tab in self.tabs.values_mut() {
                                    if tab.path == path {
                                        tab.display_name = new_name.clone();
                                    }
                                }
                            }
                            FileTreeAction::RemovedFromSidebar { path } => {
                                if let Some(dlg) = &self.var_preset_dialog {
                                    if dlg.dataset_path == path
                                        || dlg.dataset_path.starts_with(&path)
                                    {
                                        self.var_preset_dialog = None;
                                    }
                                }
                            }
                        }
                    }
                });
            let cur_w = left_resp.response.rect.width().clamp(160.0, 520.0);
            if (cur_w - self.config.left_panel_width).abs() > 1.0
                && ctx.input(|i| !i.pointer.any_down())
            {
                self.config.left_panel_width = cur_w;
                self.config.save();
            }
        }

        // Right Panel: Variables (4-col) / Statistics (selectable values) / Shortcuts
        // `ui.set_min_width(panel_rect.width())` keeps SidePanel's persisted width locked to the exact
        // user-dragged width (neither shrinking on mouse move nor expanding from inner widgets!).
        let mut right_actions = Vec::new();
        if self.config.right_panel_open {
            let right_resp = egui::SidePanel::right("right_inspector_panel")
                .resizable(true)
                .default_width(self.config.right_panel_width.clamp(160.0, 760.0))
                .width_range(160.0..=760.0)
                .show(ctx, |ui| {
                    let panel_rect = ui.max_rect();
                    ui.set_min_width(panel_rect.width());
                    let mut child_ui = ui.new_child(
                        egui::UiBuilder::new()
                            .max_rect(panel_rect)
                            .layout(egui::Layout::top_down(egui::Align::Min)),
                    );
                    child_ui.set_clip_rect(panel_rect);
                    let active_path = self
                        .active_tab_id
                        .and_then(|id| self.tabs.get(&id))
                        .map(|t| t.path.clone());
                    let saved_var_preset = active_path
                        .as_deref()
                        .and_then(|p| self.config.get_dataset_var_preset(p));
                    let active_tab = self
                        .active_tab_id
                        .and_then(|id| self.tabs.get_mut(&id));
                    right_actions = show_right_panel(
                        &mut child_ui,
                        active_tab,
                        saved_var_preset,
                        &mut self.right_tab,
                        &mut self.focus_var_search,
                        &pal,
                    );
                });
            let cur_w = right_resp.response.rect.width().clamp(160.0, 760.0);
            if (cur_w - self.config.right_panel_width).abs() > 1.0
                && ctx.input(|i| !i.pointer.any_down())
            {
                self.config.right_panel_width = cur_w;
                self.config.save();
            }
        }
        for act in right_actions {
            match act {
                RightPanelAction::OpenVarPresetDialog { tab_id, path } => {
                    let existing = self
                        .config
                        .get_dataset_var_preset(&path)
                        .map(|vars| vars.join(", "))
                        .unwrap_or_default();
                    let dataset_name = self
                        .tabs
                        .get(&tab_id)
                        .map(|t| t.display_name.clone())
                        .unwrap_or_else(|| self.config.display_name_for(&path));
                    self.var_preset_dialog = Some(VarPresetDialogState {
                        tab_id,
                        dataset_path: path,
                        dataset_name,
                        input_text: existing,
                        just_opened: true,
                    });
                }
                RightPanelAction::ApplyVarPreset { tab_id, path } => {
                    self.apply_saved_var_preset(tab_id, &path);
                }
            }
        }

        // Center Panel: Multi-Dataset Docking & Split Workspace
        egui::CentralPanel::default()
            .frame(egui::Frame::NONE.fill(pal.bg_canvas))
            .show(ctx, |ui| {
                let center_rect = ui.max_rect();
                if let Some(dropped_path) = self.file_tree.take_dropped_dataset_for_dock(ctx) {
                    if let Some(pos) = ctx.pointer_latest_pos() {
                        if center_rect.contains(pos) {
                            let mode = if self.tabs.is_empty() {
                                OpenSplitMode::Tab
                            } else if pos.x > center_rect.center().x + center_rect.width() * 0.15 {
                                OpenSplitMode::SplitRight
                            } else if pos.y > center_rect.center().y + center_rect.height() * 0.15 {
                                OpenSplitMode::SplitBottom
                            } else {
                                OpenSplitMode::Tab
                            };
                            self.open_dataset(dropped_path, mode, Some(ctx.clone()));
                        }
                    }
                }

                if self.tabs.is_empty() {
                    ui.centered_and_justified(|ui| {
                        ui.vertical_centered(|ui| {
                            ui.add_space(64.0);
                            if let Some(tex_id) = logo_tex_id {
                                ui.add(egui::Image::new(egui::load::SizedTexture::new(
                                    tex_id,
                                    Vec2::splat(72.0),
                                )));
                                ui.add_space(12.0);
                            }
                            ui.label(
                                RichText::new("SAS 数据集高速查看器")
                                    .size(22.0)
                                    .strong()
                                    .color(pal.text_primary),
                            );
                            ui.add_space(4.0);
                            ui.label(
                                RichText::new(format!("v{APP_VERSION}"))
                                    .monospace()
                                    .size(11.5)
                                    .color(pal.accent),
                            );
                            ui.add_space(8.0);
                            ui.label(
                                RichText::new(
                                    "基于纯 C (ReadStat) + Rust 列式引擎构建，专为 .sas7bdat 与 .xpt 大数据集优化",
                                )
                                .size(13.0)
                                .color(pal.text_muted),
                            );
                            ui.add_space(18.0);
                            if ui.button("⚡ 生成示例数据集并体验").clicked() {
                                self.generate_and_load_samples(ctx.clone());
                            }
                            ui.add_space(14.0);
                            ui.label(
                                RichText::new(
                                    "提示：直接将任意 .sas7bdat / .xpt 文件或文件夹拖入窗口，将自动纳入该目录全部数据集；打开多个数据集后可拖拽标签页上下/左右并排对比",
                                )
                                .size(11.8)
                                .color(pal.text_muted),
                            );
                        });
                    });
                } else {
                    let mut viewer = DatasetDockViewer {
                        tabs: &mut self.tabs,
                        active_tab_id: &mut self.active_tab_id,
                        pal,
                        pending_grid_actions: Vec::new(),
                        closed_tabs: Vec::new(),
                    };

                    let dock_style = DockStyle::from_egui(ui.style().as_ref());
                    DockArea::new(&mut self.dock_state)
                        .style(dock_style)
                        .show_leaf_close_all_buttons(false)
                        .show_inside(ui, &mut viewer);

                    let pending_actions = viewer.pending_grid_actions;
                    let closed_tabs = viewer.closed_tabs;

                    for closed_id in closed_tabs {
                        self.tabs.remove(&closed_id);
                        if self.active_tab_id == Some(closed_id) {
                            self.active_tab_id = None;
                        }
                    }

                    if let Some((_, focused_id)) = self.dock_state.find_active_focused() {
                        if self.tabs.contains_key(focused_id) {
                            self.active_tab_id = Some(*focused_id);
                        }
                    } else if self.active_tab_id.is_none() {
                        self.sync_active_tab_from_dock();
                    }

                    for (tab_id, action) in pending_actions {
                        match action {
                            GridAction::ShowStatsFor(col) => {
                                self.active_tab_id = Some(tab_id);
                                if let Some(tab) = self.tabs.get_mut(&tab_id) {
                                    tab.refresh_stats_for(col);
                                }
                                self.config.right_panel_open = true;
                                self.right_tab = RightTab::Statistics;
                            }
                            GridAction::CopyText(text) => {
                                ctx.copy_text(text);
                                self.toast("已复制单元格内容");
                            }
                        }
                    }
                }
            });

        self.show_var_preset_dialog(ctx, &pal);
        self.show_about_window(ctx, &pal);
    }

    fn apply_saved_var_preset(&mut self, tab_id: u64, path: &Path) {
        let Some(preset) = self.config.get_dataset_var_preset(path).map(|v| v.to_vec()) else {
            self.toast("当前数据集尚未设置预设勾选变量");
            return;
        };
        let Some(tab) = self.tabs.get_mut(&tab_id) else {
            return;
        };
        let (matched, unmatched) = tab.apply_var_preset(&preset);
        if matched == 0 {
            self.toast("预设中的变量在当前数据集中均未找到，未更改勾选状态");
        } else if unmatched.is_empty() {
            self.toast(format!("已应用预设：勾选 {matched} 个变量"));
        } else {
            self.toast(format!(
                "已应用预设：勾选 {matched} 个变量（未找到 {} 个：{}）",
                unmatched.len(),
                unmatched.join(", ")
            ));
        }
    }

    fn show_var_preset_dialog(&mut self, ctx: &egui::Context, pal: &AppPalette) {
        let Some(mut dlg) = self.var_preset_dialog.take() else {
            return;
        };

        let mut keep_open = true;
        let mut action_cancel = false;
        let mut action_save = false;
        let mut action_save_and_apply = false;
        let mut action_clear_preset = false;
        let mut action_import_checked = false;

        let has_existing_preset = self
            .config
            .get_dataset_var_preset(&dlg.dataset_path)
            .is_some();

        let parsed = parse_var_preset_input(&dlg.input_text);
        let (matched_preview, unmatched_preview) = self
            .tabs
            .get(&dlg.tab_id)
            .map(|tab| tab.preview_var_preset(&parsed))
            .unwrap_or_else(|| (Vec::new(), Vec::new()));

        egui::Window::new(format!("预设勾选变量 - {}", dlg.dataset_name))
            .id(egui::Id::new("var_preset_dialog_window"))
            .open(&mut keep_open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .fixed_size(Vec2::new(440.0, 0.0))
            .show(ctx, |ui| {
                ui.label(
                    RichText::new("输入要预设勾选的变量名（逗号、分号、空格或换行分隔，不区分大小写）：")
                        .size(12.0)
                        .color(pal.text_primary),
                );
                ui.label(
                    RichText::new("示例：STUDYID, USUBJID, SUBJID, SITEID")
                        .size(11.2)
                        .color(pal.text_muted),
                );
                ui.add_space(6.0);

                let text_resp = ui.add(
                    egui::TextEdit::multiline(&mut dlg.input_text)
                        .desired_rows(4)
                        .desired_width(f32::INFINITY)
                        .font(egui::TextStyle::Monospace)
                        .hint_text("STUDYID, USUBJID, SUBJID, SITEID"),
                );
                if dlg.just_opened {
                    text_resp.request_focus();
                    dlg.just_opened = false;
                }

                ui.add_space(5.0);
                ui.horizontal(|ui| {
                    if ui
                        .small_button("导入当前已勾选变量")
                        .on_hover_text("将当前数据集中已勾选的变量名填入上方输入框")
                        .clicked()
                    {
                        action_import_checked = true;
                    }
                    if ui
                        .add_enabled(!dlg.input_text.is_empty(), egui::Button::new("清空输入").small())
                        .clicked()
                    {
                        dlg.input_text.clear();
                    }
                });

                ui.add_space(6.0);
                // Live matching feedback
                egui::Frame::NONE
                    .fill(pal.bg_header_name)
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(egui::Margin::same(8))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        if parsed.is_empty() {
                            ui.label(
                                RichText::new("当前未输入变量名（保存空内容将清除此数据集的预设）")
                                    .size(11.2)
                                    .color(pal.text_muted),
                            );
                        } else {
                            ui.label(
                                RichText::new(format!(
                                    "匹配预览：当前数据集可匹配 {} / {} 个变量",
                                    matched_preview.len(),
                                    parsed.len()
                                ))
                                .size(11.4)
                                .strong()
                                .color(if matched_preview.is_empty() {
                                    Color32::from_rgb(205, 70, 70)
                                } else {
                                    pal.accent
                                }),
                            );
                            if !matched_preview.is_empty() {
                                ui.label(
                                    RichText::new(format!("已匹配：{}", matched_preview.join(", ")))
                                        .size(11.0)
                                        .color(pal.text_primary),
                                );
                            }
                            if !unmatched_preview.is_empty() {
                                ui.label(
                                    RichText::new(format!(
                                        "未在当前数据集找到：{}",
                                        unmatched_preview.join(", ")
                                    ))
                                    .size(11.0)
                                    .color(Color32::from_rgb(217, 119, 6)),
                                );
                            }
                        }
                    });

                ui.add_space(6.0);
                ui.label(
                    RichText::new(
                        "存储说明：保存后此预设会在后台跟随该数据集持久化记忆，直到在左侧文件管理器中将其「从侧栏移除」。",
                    )
                    .size(10.8)
                    .color(pal.text_muted),
                );

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(6.0);

                ui.horizontal(|ui| {
                    if has_existing_preset
                        && ui
                            .button(
                                RichText::new("清除预设").color(Color32::from_rgb(205, 70, 70)),
                            )
                            .on_hover_text("删除此数据集已保存的预设勾选变量")
                            .clicked()
                    {
                        action_clear_preset = true;
                    }

                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        let save_apply_btn = egui::Button::new(
                            RichText::new("保存并应用").color(Color32::WHITE).strong(),
                        )
                        .fill(pal.accent);
                        if ui
                            .add_enabled(!parsed.is_empty(), save_apply_btn)
                            .on_hover_text("保存此预设并立即勾选对应变量")
                            .clicked()
                        {
                            action_save_and_apply = true;
                        }
                        if ui
                            .button("保存预设")
                            .on_hover_text("保存预设（稍后可点击右侧「应用预设」生效）")
                            .clicked()
                        {
                            action_save = true;
                        }
                        if ui.button("取消").clicked() {
                            action_cancel = true;
                        }
                    });
                });
            });

        if action_cancel {
            keep_open = false;
        }

        if action_import_checked {
            if let Some(tab) = self.tabs.get(&dlg.tab_id) {
                let checked = tab.checked_variable_names();
                dlg.input_text = checked.join(", ");
            }
        }

        if action_clear_preset {
            self.config.clear_dataset_var_preset(&dlg.dataset_path);
            self.toast(format!("已清除 {} 的预设勾选变量", dlg.dataset_name));
            return;
        }

        if action_save || action_save_and_apply {
            let vars = parse_var_preset_input(&dlg.input_text);
            let path = dlg.dataset_path.clone();
            let tab_id = dlg.tab_id;
            let name = dlg.dataset_name.clone();
            self.config.set_dataset_var_preset(path.clone(), vars.clone());
            if action_save_and_apply {
                self.apply_saved_var_preset(tab_id, &path);
            } else if vars.is_empty() {
                self.toast(format!("已清除 {name} 的预设勾选变量"));
            } else {
                self.toast(format!(
                    "已保存 {name} 的预设勾选变量（{} 个），点击「应用预设」即可生效",
                    vars.len()
                ));
            }
            return;
        }

        if keep_open {
            self.var_preset_dialog = Some(dlg);
        }
    }

    fn show_about_window(&mut self, ctx: &egui::Context, pal: &AppPalette) {
        if !self.about_open {
            return;
        }

        let logo_tex = self.textures.app_logo(ctx).map(|t| t.id());
        let wechat_tex = self.textures.wechat_qr(ctx).map(|t| (t.id(), t.size_vec2()));
        let mut open = self.about_open;
        let mut trigger_manual_check = false;
        let mut copied_msg: Option<&'static str> = None;

        egui::Window::new("关于 SAS 数据集查看器")
            .id(egui::Id::new("about_app_window"))
            .open(&mut open)
            .collapsible(false)
            .resizable(false)
            .anchor(Align2::CENTER_CENTER, Vec2::ZERO)
            .fixed_size(Vec2::new(450.0, 0.0))
            .show(ctx, |ui| {
                // 1. Header: Logo + App Name + Version Badge
                ui.horizontal(|ui| {
                    if let Some(tex_id) = logo_tex {
                        ui.add(egui::Image::new(egui::load::SizedTexture::new(
                            tex_id,
                            Vec2::splat(56.0),
                        )));
                        ui.add_space(8.0);
                    }
                    ui.vertical(|ui| {
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new("SAS 数据集查看器")
                                    .size(18.0)
                                    .strong()
                                    .color(pal.text_primary),
                            );
                            egui::Frame::NONE
                                .fill(pal.accent_soft)
                                .corner_radius(egui::CornerRadius::same(4))
                                .inner_margin(egui::Margin::symmetric(6, 2))
                                .show(ui, |ui| {
                                    ui.label(
                                        RichText::new(format!("v{APP_VERSION}"))
                                            .monospace()
                                            .strong()
                                            .size(11.5)
                                            .color(pal.accent),
                                    );
                                });
                        });
                        ui.add_space(3.0);
                        ui.add(
                            egui::Label::new(
                                RichText::new(
                                    "基于纯 C (ReadStat) + Rust 列式引擎构建的跨平台高速 SAS 数据集（.sas7bdat / .xpt）桌面查看器。",
                                )
                                .size(12.0)
                                .color(pal.text_muted),
                            )
                            .wrap(),
                        );
                    });
                });

                ui.add_space(10.0);
                ui.separator();
                ui.add_space(8.0);

                // 2. Project Info: GitHub Repo, Author, Contributors
                egui::Grid::new("about_info_grid")
                    .num_columns(2)
                    .spacing([12.0, 7.0])
                    .show(ui, |ui| {
                        ui.label(RichText::new("GitHub 仓库").strong().size(12.0).color(pal.text_muted));
                        ui.horizontal(|ui| {
                            ui.hyperlink_to(
                                RichText::new("allensrj/sas-dataset-viewer").size(12.0),
                                REPO_URL,
                            );
                            if ui
                                .small_button("复制")
                                .on_hover_text("复制 GitHub 仓库地址")
                                .clicked()
                            {
                                ctx.copy_text(REPO_URL.to_string());
                                copied_msg = Some("已复制 GitHub 仓库地址");
                            }
                        });
                        ui.end_row();

                        ui.label(RichText::new("作者").strong().size(12.0).color(pal.text_muted));
                        ui.horizontal(|ui| {
                            ui.hyperlink_to(
                                RichText::new("@allensrj").strong().size(12.0),
                                "https://github.com/allensrj",
                            );
                            ui.label(
                                RichText::new("（药企SP程序员Allen）")
                                    .size(11.5)
                                    .color(pal.text_muted),
                            );
                        });
                        ui.end_row();

                        ui.label(RichText::new("贡献者").strong().size(12.0).color(pal.text_muted));
                        ui.vertical(|ui| {
                            ui.spacing_mut().item_spacing.y = 3.0;
                            ui.horizontal_wrapped(|ui| {
                                ui.hyperlink_to(
                                    RichText::new("@allensrj").size(11.8),
                                    "https://github.com/allensrj",
                                );
                                ui.label(
                                    RichText::new("— 项目发起人、核心架构与功能开发")
                                        .size(11.5)
                                        .color(pal.text_primary),
                                );
                            });
                            ui.horizontal_wrapped(|ui| {
                                ui.hyperlink_to(
                                    RichText::new("@wtwang1998 (wentao)").size(11.8),
                                    "https://github.com/wtwang1998",
                                );
                                ui.label(
                                    RichText::new(
                                        "— Windows 拖拽冻结修复、渲染架构与低 DPI 清晰度优化",
                                    )
                                    .size(11.5)
                                    .color(pal.text_primary),
                                );
                            });
                        });
                        ui.end_row();

                        ui.label(RichText::new("开源协议").strong().size(12.0).color(pal.text_muted));
                        ui.horizontal_wrapped(|ui| {
                            ui.hyperlink_to(
                                RichText::new("Apache License 2.0").size(12.0),
                                format!("{REPO_URL}/blob/main/LICENSE"),
                            );
                            ui.label(
                                RichText::new("· Copyright © 2026 allensrj")
                                    .size(11.5)
                                    .color(pal.text_muted),
                            );
                        });
                        ui.end_row();
                    });

                ui.add_space(10.0);

                // 3. Version Update Checker Section
                let is_checking = self.updater.is_checking();
                egui::Frame::NONE
                    .fill(pal.bg_header_name)
                    .corner_radius(egui::CornerRadius::same(6))
                    .inner_margin(egui::Margin::same(10))
                    .stroke(Stroke::new(1.0, pal.border_grid))
                    .show(ui, |ui| {
                        ui.set_min_width(ui.available_width());
                        ui.horizontal(|ui| {
                            ui.label(
                                RichText::new(format!("版本检查（当前 v{APP_VERSION}）"))
                                    .strong()
                                    .size(12.2)
                                    .color(pal.text_primary),
                            );
                            ui.with_layout(
                                egui::Layout::right_to_left(egui::Align::Center),
                                |ui| {
                                    let btn_lbl = if is_checking {
                                        "正在检查..."
                                    } else {
                                        "检查更新"
                                    };
                                    if ui
                                        .add_enabled(!is_checking, egui::Button::new(btn_lbl).small())
                                        .clicked()
                                    {
                                        trigger_manual_check = true;
                                    }
                                },
                            );
                        });

                        ui.add_space(5.0);
                        match &self.updater.state {
                            UpdateCheckState::Idle => {
                                ui.label(
                                    RichText::new("点击右侧「检查更新」获取 GitHub Releases 最新版本信息。")
                                        .size(11.3)
                                        .color(pal.text_muted),
                                );
                            }
                            UpdateCheckState::Checking => {
                                ui.horizontal(|ui| {
                                    ui.spinner();
                                    ui.label(
                                        RichText::new("正在连接 GitHub Releases 检查最新版本...")
                                            .size(11.5)
                                            .color(pal.text_muted),
                                    );
                                });
                            }
                            UpdateCheckState::UpToDate { latest_tag } => {
                                ui.label(
                                    RichText::new(format!(
                                        "✓ 当前已是最新版本（GitHub 最新发布：{latest_tag}）"
                                    ))
                                    .size(11.6)
                                    .strong()
                                    .color(Color32::from_rgb(22, 163, 74)),
                                );
                            }
                            UpdateCheckState::UpdateAvailable(info) => {
                                let date_suffix = info
                                    .published_at
                                    .as_deref()
                                    .map(|d| format!(" · 发布于 {d}"))
                                    .unwrap_or_default();
                                ui.label(
                                    RichText::new(format!(
                                        "发现新版本 {}{}，建议下载更新！",
                                        info.tag_name, date_suffix
                                    ))
                                    .size(12.0)
                                    .strong()
                                    .color(Color32::from_rgb(217, 119, 6)),
                                );
                                ui.add_space(5.0);
                                let download_url = info.html_url.clone();
                                ui.horizontal(|ui| {
                                    let dl_btn = egui::Button::new(
                                        RichText::new("前往 GitHub Release 下载最新版本")
                                            .color(Color32::WHITE)
                                            .strong()
                                            .size(11.8),
                                    )
                                    .fill(pal.accent);
                                    if ui.add(dl_btn).clicked() {
                                        ctx.open_url(OpenUrl::new_tab(&download_url));
                                    }
                                    if ui
                                        .small_button("复制链接")
                                        .on_hover_text(&download_url)
                                        .clicked()
                                    {
                                        ctx.copy_text(download_url);
                                        copied_msg = Some("已复制最新版下载链接");
                                    }
                                });
                            }
                            UpdateCheckState::Error(err) => {
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(format!("检查更新失败：{err}"))
                                            .size(11.2)
                                            .color(Color32::from_rgb(205, 70, 70)),
                                    )
                                    .wrap(),
                                );
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new("您也可以手动访问：")
                                            .size(11.2)
                                            .color(pal.text_muted),
                                    );
                                    ui.hyperlink_to(
                                        RichText::new("GitHub Releases 页面").size(11.2),
                                        RELEASES_URL,
                                    );
                                });
                            }
                        }
                    });

                // 4. Personal WeChat Official Account Banner (at the very bottom)
                if let Some((tex_id, orig_size)) = wechat_tex {
                    ui.add_space(10.0);
                    ui.separator();
                    ui.add_space(6.0);
                    ui.vertical_centered(|ui| {
                        ui.label(
                            RichText::new("关注个人微信公众号")
                                .size(11.5)
                                .color(pal.text_muted),
                        );
                        ui.add_space(5.0);
                        let target_w = ui.available_width().min(416.0);
                        let aspect = if orig_size.x > 0.0 {
                            orig_size.y / orig_size.x
                        } else {
                            312.0 / 1080.0
                        };
                        let target_h = (target_w * aspect).max(60.0);
                        ui.add(
                            egui::Image::new(egui::load::SizedTexture::new(
                                tex_id,
                                Vec2::new(target_w, target_h),
                            ))
                            .corner_radius(egui::CornerRadius::same(8)),
                        );
                    });
                }
            });

        self.about_open = open;
        if trigger_manual_check {
            self.updater.start_check(true, Some(ctx.clone()));
        }
        if let Some(msg) = copied_msg {
            self.toast(msg);
        }
    }
}

/// Custom vector-painted circle-`i` About button (crisp on all OS/DPIs without Unicode tofu).
/// Displays a subtle amber/red dot when a newer GitHub Release version is detected.
fn about_circle_i_button(
    ui: &mut Ui,
    pal: &AppPalette,
    update_info: Option<&crate::updater::ReleaseInfo>,
) -> bool {
    let size = Vec2::splat(22.0);
    let (rect, resp) = ui.allocate_exact_size(size, Sense::click());
    if resp.hovered() {
        ui.ctx().set_cursor_icon(egui::CursorIcon::PointingHand);
    }

    let center = rect.center();
    let radius = 8.5;
    let hovered = resp.hovered();
    let stroke_color = if hovered { pal.accent } else { pal.text_muted };

    if hovered {
        ui.painter().circle_filled(center, radius + 1.5, pal.accent_soft);
    }
    ui.painter()
        .circle_stroke(center, radius, Stroke::new(1.4, stroke_color));
    ui.painter().text(
        Pos2::new(center.x, center.y + 0.3),
        Align2::CENTER_CENTER,
        "i",
        FontId::proportional(12.2),
        stroke_color,
    );

    if update_info.is_some() {
        let dot_center = Pos2::new(rect.right() - 3.0, rect.top() + 3.5);
        ui.painter()
            .circle_filled(dot_center, 3.3, Color32::from_rgb(239, 68, 68));
    }

    let hover_txt = if let Some(info) = update_info {
        format!(
            "关于 SAS 数据集查看器 (v{APP_VERSION})\n发现新版本 {}，点击查看详情与下载链接",
            info.tag_name
        )
    } else {
        format!("关于 SAS 数据集查看器 (v{APP_VERSION})\n查看软件信息、作者、公众号与检查更新")
    };
    resp.on_hover_text(hover_txt).clicked()
}

/// Compact VS Code-style toggle button for Search options (`Aa`, `全字`, `.*`).
fn search_option_toggle(
    ui: &mut Ui,
    active: bool,
    label: &str,
    tooltip: &str,
    pal: &AppPalette,
) -> bool {
    let text = RichText::new(label)
        .size(11.0)
        .strong()
        .color(if active { pal.accent } else { pal.text_muted });
    let mut btn = egui::Button::new(text).small();
    if active {
        btn = btn
            .fill(pal.accent_soft)
            .stroke(egui::Stroke::new(1.0, pal.accent));
    }
    ui.add(btn).on_hover_text(tooltip).clicked()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_right_side_panel_resize_persists_across_mouse_moves() {
        for ppp in [1.0f32, 1.25, 2.0] {
            let ctx = egui::Context::default();
            ctx.set_pixels_per_point(ppp);
            let screen = egui::Rect::from_min_size(egui::Pos2::ZERO, egui::Vec2::new(1280.0, 800.0));
            let pal = AppPalette::for_mode(false);
            let mut right_tab = RightTab::Variables;
            let mut focus_var = false;

            let run_frame = |events: Vec<egui::Event>,
                             right_tab: &mut RightTab,
                             focus_var: &mut bool|
             -> f32 {
                let mut measured_w = 0.0f32;
                let raw = egui::RawInput {
                    screen_rect: Some(screen),
                    events,
                    ..Default::default()
                };
                let _ = ctx.run(raw, |ctx| {
                    let resp = egui::SidePanel::right("right_inspector_panel")
                        .resizable(true)
                        .default_width(360.0)
                        .width_range(160.0..=760.0)
                        .show(ctx, |ui| {
                            let panel_rect = ui.max_rect();
                            ui.set_min_width(panel_rect.width());
                            let mut child_ui = ui.new_child(
                                egui::UiBuilder::new()
                                    .max_rect(panel_rect)
                                    .layout(egui::Layout::top_down(egui::Align::Min)),
                            );
                            child_ui.set_clip_rect(panel_rect);
                            let _ = show_right_panel(
                                &mut child_ui,
                                None,
                                None,
                                right_tab,
                                focus_var,
                                &pal,
                            );
                        });
                    measured_w = resp.response.rect.width();
                    egui::CentralPanel::default().show(ctx, |_ui| {});
                });
                measured_w
            };

            // 1. Initial width stays pinned at 360px across 25 frames of mouse moves
            let mut w = 0.0;
            for i in 0..25 {
                w = run_frame(
                    vec![egui::Event::PointerMoved(egui::Pos2::new(
                        200.0 + (i as f32) * 5.0,
                        300.0,
                    ))],
                    &mut right_tab,
                    &mut focus_var,
                );
            }
            assert!(
                (w - 360.0).abs() < 1.5,
                "ppp={ppp}: initial width drifted to {w}, expected ~360.0"
            );

            // 2. Drag left border inward -> width=240, then move mouse for 30 frames
            let border_x = 1280.0 - w;
            let _ = run_frame(
                vec![egui::Event::PointerMoved(egui::Pos2::new(border_x, 400.0))],
                &mut right_tab,
                &mut focus_var,
            );
            let _ = run_frame(
                vec![egui::Event::PointerButton {
                    pos: egui::Pos2::new(border_x, 400.0),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                &mut right_tab,
                &mut focus_var,
            );
            let _ = run_frame(
                vec![egui::Event::PointerMoved(egui::Pos2::new(1040.0, 400.0))],
                &mut right_tab,
                &mut focus_var,
            );
            let _ = run_frame(
                vec![egui::Event::PointerButton {
                    pos: egui::Pos2::new(1040.0, 400.0),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                &mut right_tab,
                &mut focus_var,
            );

            for i in 0..30 {
                w = run_frame(
                    vec![egui::Event::PointerMoved(egui::Pos2::new(
                        500.0 + (i as f32) * 3.0,
                        250.0,
                    ))],
                    &mut right_tab,
                    &mut focus_var,
                );
            }
            assert!(
                (w - 240.0).abs() < 2.0,
                "ppp={ppp}: dragged narrow width drifted to {w}, expected ~240.0"
            );

            // 3. Drag left border outward -> width=500, switch tabs, move mouse for 30 frames
            let border_x2 = 1280.0 - w;
            let _ = run_frame(
                vec![egui::Event::PointerMoved(egui::Pos2::new(border_x2, 400.0))],
                &mut right_tab,
                &mut focus_var,
            );
            let _ = run_frame(
                vec![egui::Event::PointerButton {
                    pos: egui::Pos2::new(border_x2, 400.0),
                    button: egui::PointerButton::Primary,
                    pressed: true,
                    modifiers: egui::Modifiers::NONE,
                }],
                &mut right_tab,
                &mut focus_var,
            );
            let _ = run_frame(
                vec![egui::Event::PointerMoved(egui::Pos2::new(780.0, 400.0))],
                &mut right_tab,
                &mut focus_var,
            );
            let _ = run_frame(
                vec![egui::Event::PointerButton {
                    pos: egui::Pos2::new(780.0, 400.0),
                    button: egui::PointerButton::Primary,
                    pressed: false,
                    modifiers: egui::Modifiers::NONE,
                }],
                &mut right_tab,
                &mut focus_var,
            );

            right_tab = RightTab::Statistics;
            for i in 0..30 {
                w = run_frame(
                    vec![egui::Event::PointerMoved(egui::Pos2::new(
                        400.0 + (i as f32) * 3.0,
                        250.0,
                    ))],
                    &mut right_tab,
                    &mut focus_var,
                );
            }
            assert!(
                (w - 500.0).abs() < 2.0,
                "ppp={ppp}: dragged wide width drifted to {w}, expected ~500.0"
            );
        }
    }
}
