use crate::config::AppConfig;
use crate::panels::data_grid::{show_data_grid, GridAction};
use crate::panels::file_tree::{format_file_size, FileTreeAction, FileTreeState, OpenSplitMode};
use crate::panels::right_panel::show_right_panel;
use crate::sample_gen::{generate_sample_datasets, resolve_sample_dir};
use crate::state::{DatasetTabState, FocusRegion, RightTab};
use crate::theme::{apply_theme, setup_cjk_fonts, AppPalette};
use egui::{Color32, Key, RichText, Ui};
use egui_dock::{DockArea, DockState, NodeIndex, Style as DockStyle, SurfaceIndex, TabViewer};
use sas_engine::{ColumnStats, EncodingChoice, SasFormatKind};
use std::collections::HashMap;
use std::path::PathBuf;

pub struct SasViewerApp {
    config: AppConfig,
    file_tree: FileTreeState,
    dock_state: DockState<u64>,
    tabs: HashMap<u64, DatasetTabState>,
    next_tab_id: u64,
    active_tab_id: Option<u64>,
    right_tab: RightTab,
    focus_var_search: bool,
    status_toast: Option<(String, std::time::Instant)>,
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
            status_toast: None,
        };

        for p in initial_paths {
            app.ingest_dropped_path(p, Some(egui_ctx.clone()));
        }
        app.file_tree.refresh_all(&app.config);
        app
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

        let (cmd_b, cmd_backslash, cmd_f, cmd_c, cmd_w) = ctx.input(|i| {
            let cmd = i.modifiers.command;
            (
                cmd && i.key_pressed(Key::B),
                cmd && i.key_pressed(Key::Backslash),
                cmd && i.key_pressed(Key::F),
                cmd && i.key_pressed(Key::C),
                cmd && i.key_pressed(Key::W),
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

        if typing_in_text {
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
    pub fn update_ui(&mut self, ctx: &egui::Context) {
        let mut any_loading = false;
        for tab in self.tabs.values_mut() {
            tab.poll_loader();
            if tab.loading {
                any_loading = true;
            }
        }
        if any_loading {
            ctx.request_repaint_after(std::time::Duration::from_millis(33));
        }

        self.handle_os_dropped_files(ctx);
        self.handle_shortcuts(ctx);

        let pal = AppPalette::for_mode(self.config.dark_mode);

        // Top compact toolbar (streamlined: removed redundant Add Folder / Open File; added dataset Search!)
        egui::TopBottomPanel::top("top_toolbar")
            .exact_height(34.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
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
                                .on_hover_text("批量调整所有可见列宽至数据实际长度（范围约 6~30 字符）")
                                .clicked()
                            {
                                tab.auto_fit_all_visible_columns();
                            }

                            ui.separator();
                            // Dataset cell-value Search bar + Prev / Next
                            ui.label(RichText::new("Search").strong().size(12.0).color(pal.text_muted));
                            let resp = ui.add(
                                egui::TextEdit::singleline(&mut tab.search_query)
                                    .hint_text("检索单元格值...")
                                    .desired_width(145.0),
                            );
                            if resp.lost_focus() && ui.input(|i| i.key_pressed(Key::Enter)) {
                                if ui.input(|i| i.modifiers.shift) {
                                    tab.search_prev();
                                } else {
                                    tab.search_next();
                                }
                                resp.request_focus();
                            } else if resp.has_focus() && ui.input(|i| i.key_pressed(Key::Escape)) {
                                tab.clear_cell_search();
                            }

                            if !tab.search_query.is_empty()
                                && ui.small_button("×").on_hover_text("清空搜索").clicked()
                            {
                                tab.clear_cell_search();
                            }
                            if ui
                                .small_button("↑ 上一个")
                                .on_hover_text("跳转到上一个匹配单元格 (Shift+Enter)")
                                .clicked()
                            {
                                tab.search_prev();
                            }
                            if ui
                                .small_button("↓ 下一个")
                                .on_hover_text("跳转到下一个匹配单元格 (Enter)")
                                .clicked()
                            {
                                tab.search_next();
                            }

                            if !tab.search_executed_query.is_empty() {
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
                                            Color32::from_rgb(200, 75, 75)
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

        // Bottom Status Bar (with folder display name prefix + Encoding Switcher!)
        egui::TopBottomPanel::bottom("bottom_status_bar")
            .exact_height(26.0)
            .show(ctx, |ui| {
                ui.horizontal_centered(|ui| {
                    if let Some(active_id) = self.active_tab_id {
                        if let Some(tab) = self.tabs.get_mut(&active_id) {
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

                                ui.with_layout(
                                    egui::Layout::right_to_left(egui::Align::Center),
                                    |ui| {
                                        let mut chosen_enc = tab.encoding;
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
                                        if chosen_enc != tab.encoding {
                                            tab.reload_with_encoding(chosen_enc, ctx.clone());
                                        }
                                    },
                                );
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
                });
            });

        // Left Panel: Collapsible File Manager
        if self.config.left_panel_open {
            egui::SidePanel::left("left_file_manager")
                .resizable(true)
                .default_width(250.0)
                .width_range(160.0..=480.0)
                .show(ctx, |ui| {
                    let tree_actions = self.file_tree.show(ui, &mut self.config, &pal);
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
                        }
                    }
                });
        }

        // Right Panel: Variables (2-col) / Statistics (selectable values) / Shortcuts
        if self.config.right_panel_open {
            egui::SidePanel::right("right_inspector_panel")
                .resizable(true)
                .default_width(320.0)
                .width_range(220.0..=580.0)
                .show(ctx, |ui| {
                    let active_tab = self
                        .active_tab_id
                        .and_then(|id| self.tabs.get_mut(&id));
                    show_right_panel(
                        ui,
                        active_tab,
                        &mut self.right_tab,
                        &mut self.focus_var_search,
                        &pal,
                    );
                });
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
                            ui.add_space(80.0);
                            ui.label(
                                RichText::new("SAS 数据集高速查看器")
                                    .size(22.0)
                                    .strong()
                                    .color(pal.text_primary),
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
    }
}
