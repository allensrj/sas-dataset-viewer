use crate::config::AppConfig;
use crate::theme::AppPalette;
use egui::{Color32, RichText, Sense, Ui};
use sas_engine::SasFormatKind;
use std::collections::HashMap;
use std::path::{Path, PathBuf};

#[derive(Debug, Clone)]
pub struct FsNode {
    pub path: PathBuf,
    pub file_name: String,
    pub is_dir: bool,
    pub format_kind: Option<SasFormatKind>,
    pub file_size: u64,
    pub children: Vec<FsNode>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OpenSplitMode {
    Tab,
    SplitRight,
    SplitBottom,
}

#[derive(Debug, Clone)]
pub enum FileTreeAction {
    OpenFile { path: PathBuf, mode: OpenSplitMode },
    AddFolderDialog,
    OpenFileDialog,
    AliasChanged { path: PathBuf, new_name: String },
    RemovedFromSidebar { path: PathBuf },
}

#[derive(Debug, Clone)]
enum DragKind {
    RootFolder(PathBuf),
    Sibling { parent: PathBuf, file_name: String },
    DatasetToDock(PathBuf),
}

pub struct FileTreeState {
    /// Cached scanned directory trees keyed by root folder path.
    pub cache: HashMap<PathBuf, FsNode>,
    /// Currently active inline rename target (modifies in-app alias only).
    pub renaming_path: Option<PathBuf>,
    pub rename_buffer: String,
    pub rename_just_started: bool,
    /// Selected node in file tree.
    pub selected_path: Option<PathBuf>,
    /// Active drag payload inside the file tree.
    dragging: Option<DragKind>,
    /// Filter search box inside file tree.
    pub tree_filter: String,
}

impl Default for FileTreeState {
    fn default() -> Self {
        Self {
            cache: HashMap::new(),
            renaming_path: None,
            rename_buffer: String::new(),
            rename_just_started: false,
            selected_path: None,
            dragging: None,
            tree_filter: String::new(),
        }
    }
}

impl FileTreeState {
    pub fn refresh_all(&mut self, config: &AppConfig) {
        self.cache.clear();
        for root in &config.root_folders {
            if root.exists() {
                let node = scan_directory(root, config, 0);
                self.cache.insert(root.clone(), node);
            }
        }
    }

    pub fn refresh_root(&mut self, root: &Path, config: &AppConfig) {
        if root.exists() {
            let node = scan_directory(root, config, 0);
            self.cache.insert(root.to_path_buf(), node);
        } else {
            self.cache.remove(root);
        }
    }

    pub fn ensure_loaded(&mut self, config: &AppConfig) {
        for root in &config.root_folders {
            if !self.cache.contains_key(root) && root.exists() {
                let node = scan_directory(root, config, 0);
                self.cache.insert(root.clone(), node);
            }
        }
    }

    pub fn expand_all(&mut self, config: &mut AppConfig) {
        for node in self.cache.values() {
            collect_all_dirs(node, &mut config.expanded_folders);
        }
        config.save();
    }

    pub fn collapse_all(&mut self, config: &mut AppConfig) {
        config.expanded_folders.clear();
        config.save();
    }

    /// Take active dragged dataset path if user drops it over the center workspace.
    pub fn take_dropped_dataset_for_dock(&mut self, ctx: &egui::Context) -> Option<PathBuf> {
        if !ctx.input(|i| i.pointer.any_released()) {
            return None;
        }
        if let Some(DragKind::DatasetToDock(p)) = &self.dragging {
            let path = p.clone();
            self.dragging = None;
            return Some(path);
        }
        None
    }

    pub fn show(
        &mut self,
        ui: &mut Ui,
        config: &mut AppConfig,
        pal: &AppPalette,
    ) -> Vec<FileTreeAction> {
        self.ensure_loaded(config);
        let mut actions = Vec::new();
        let mut pending_reorder_root: Option<(PathBuf, PathBuf)> = None;
        let mut pending_reorder_sibling: Option<(PathBuf, Vec<String>, String, String)> = None;
        let mut pending_remove: Option<PathBuf> = None;
        let mut pending_refresh_root: Option<PathBuf> = None;
        let mut need_refresh_all = false;

        // Header toolbar
        ui.horizontal(|ui| {
            ui.label(RichText::new("文件管理器").strong().size(13.5));
            ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                if ui.small_button("折叠").on_hover_text("全部折叠").clicked() {
                    self.collapse_all(config);
                }
                if ui.small_button("展开").on_hover_text("全部展开").clicked() {
                    self.expand_all(config);
                }
                if ui.small_button("刷新").on_hover_text("刷新目录树").clicked() {
                    need_refresh_all = true;
                }
                if ui
                    .small_button("+目录")
                    .on_hover_text("添加包含 SAS/XPT 数据的文件夹")
                    .clicked()
                {
                    actions.push(FileTreeAction::AddFolderDialog);
                }
            });
        });

        // Quick filter input inside file manager
        ui.horizontal(|ui| {
            let resp = ui.add(
                egui::TextEdit::singleline(&mut self.tree_filter)
                    .hint_text("筛选文件名...")
                    .desired_width(ui.available_width() - 24.0),
            );
            if !self.tree_filter.is_empty() && ui.small_button("×").clicked() {
                self.tree_filter.clear();
                resp.request_focus();
            }
        });
        ui.separator();

        let filter_lower = self.tree_filter.trim().to_lowercase();

        egui::ScrollArea::vertical()
            .auto_shrink([false, false])
            .show(ui, |ui| {
                if config.root_folders.is_empty() && config.root_files.is_empty() {
                    ui.add_space(24.0);
                    ui.vertical_centered(|ui| {
                        ui.label(RichText::new("尚未添加数据集目录").color(pal.text_muted));
                        ui.add_space(6.0);
                        ui.label(
                            RichText::new("直接拖入任意 .sas7bdat / .xpt 文件或文件夹\n将自动纳入该目录下全部数据集")
                                .size(11.5)
                                .color(pal.text_muted),
                        );
                        ui.add_space(10.0);
                        if ui.button("添加文件夹...").clicked() {
                            actions.push(FileTreeAction::AddFolderDialog);
                        }
                        if ui.button("打开单个文件...").clicked() {
                            actions.push(FileTreeAction::OpenFileDialog);
                        }
                    });
                    return;
                }

                // Standalone root files
                let root_files = config.root_files.clone();
                for file_path in root_files {
                    if !file_path.exists() {
                        continue;
                    }
                    let disp = config.display_name_for(&file_path);
                    if !filter_lower.is_empty() && !disp.to_lowercase().contains(&filter_lower) {
                        continue;
                    }
                    let fsize = std::fs::metadata(&file_path).map(|m| m.len()).unwrap_or(0);
                    let fmt = SasFormatKind::from_path(&file_path);
                    self.render_file_row(
                        ui,
                        &file_path,
                        &disp,
                        fmt,
                        fsize,
                        0,
                        None,
                        &[],
                        config,
                        pal,
                        &mut actions,
                        &mut pending_remove,
                        &mut pending_reorder_sibling,
                    );
                }

                // Root folders (support drag-reorder among root folders!)
                let roots = config.root_folders.clone();
                for root_path in roots {
                    let node_opt = self.cache.get(&root_path).cloned();
                    let Some(node) = node_opt else {
                        continue;
                    };
                    if !filter_lower.is_empty() && !node_matches_filter(&node, &filter_lower, config)
                    {
                        continue;
                    }
                    self.render_dir_node(
                        ui,
                        &node,
                        &root_path,
                        true,
                        0,
                        &filter_lower,
                        None,
                        &[],
                        config,
                        pal,
                        &mut actions,
                        &mut pending_reorder_root,
                        &mut pending_reorder_sibling,
                        &mut pending_remove,
                        &mut pending_refresh_root,
                    );
                }
            });

        // Apply deferred mutations cleanly
        if let Some((src, dst)) = pending_reorder_root {
            config.reorder_root_folder(&src, &dst);
        }
        if let Some((parent, names, src_name, dst_name)) = pending_reorder_sibling {
            config.reorder_sibling(&parent, &names, &src_name, &dst_name);
            need_refresh_all = true;
        }
        if let Some(rem) = pending_remove {
            config.remove_from_sidebar(&rem);
            actions.push(FileTreeAction::RemovedFromSidebar { path: rem });
            need_refresh_all = true;
        }
        if let Some(root) = pending_refresh_root {
            self.refresh_root(&root, config);
        }
        if need_refresh_all {
            self.refresh_all(config);
        }

        // Clear drag state if pointer released inside/outside
        if ui.input(|i| i.pointer.any_released()) {
            // Leave DatasetToDock for dock area check on same frame; clear internal tree drags
            if !matches!(self.dragging, Some(DragKind::DatasetToDock(_))) {
                self.dragging = None;
            }
        }

        // Floating drag preview cursor badge
        if let Some(drag) = &self.dragging {
            if let Some(pos) = ui.ctx().pointer_latest_pos() {
                let text = match drag {
                    DragKind::RootFolder(p) => format!("↕ {}", config.display_name_for(p)),
                    DragKind::Sibling { file_name, .. } => format!("↕ {}", file_name),
                    DragKind::DatasetToDock(p) => {
                        format!("📄 拖入分屏打开: {}", config.display_name_for(p))
                    }
                };
                egui::Area::new(egui::Id::new("file_tree_drag_badge"))
                    .order(egui::Order::Tooltip)
                    .fixed_pos(pos + egui::vec2(12.0, 12.0))
                    .interactable(false)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.label(RichText::new(text).size(11.5).strong());
                        });
                    });
            }
        }

        actions
    }

    #[allow(clippy::too_many_arguments)]
    fn render_dir_node(
        &mut self,
        ui: &mut Ui,
        node: &FsNode,
        root_owner: &Path,
        is_root: bool,
        depth: usize,
        filter_lower: &str,
        parent_dir: Option<&Path>,
        sibling_names: &[String],
        config: &mut AppConfig,
        pal: &AppPalette,
        actions: &mut Vec<FileTreeAction>,
        pending_reorder_root: &mut Option<(PathBuf, PathBuf)>,
        pending_reorder_sibling: &mut Option<(PathBuf, Vec<String>, String, String)>,
        pending_remove: &mut Option<PathBuf>,
        pending_refresh_root: &mut Option<PathBuf>,
    ) {
        let is_expanded =
            !filter_lower.is_empty() || config.expanded_folders.contains(&node.path);
        let disp_name = config.display_name_for(&node.path);
        let indent = (depth as f32) * 14.0;

        let is_renaming = self.renaming_path.as_ref() == Some(&node.path);

        let row_resp = ui
            .horizontal(|ui| {
                ui.add_space(indent);
                let arrow = if is_expanded { "▼" } else { "▶" };
                if ui
                    .add(egui::Button::new(RichText::new(arrow).size(10.0).color(pal.text_muted)).frame(false))
                    .clicked()
                {
                    if config.expanded_folders.contains(&node.path) {
                        config.expanded_folders.remove(&node.path);
                    } else {
                        config.expanded_folders.insert(node.path.clone());
                    }
                    config.save();
                }

                if is_renaming {
                    self.render_rename_input(ui, &node.path, config, actions);
                    None
                } else {
                    let selected = self.selected_path.as_ref() == Some(&node.path);
                    let label_text = RichText::new(format!("📁 {}", disp_name))
                        .size(12.5)
                        .strong()
                        .color(if selected { pal.accent } else { pal.text_primary });
                    let resp = ui.add(
                        egui::Label::new(label_text)
                            .truncate()
                            .sense(Sense::click_and_drag()),
                    );
                    Some(resp)
                }
            })
            .inner;

        if let Some(resp) = row_resp {
            if resp.clicked() {
                self.selected_path = Some(node.path.clone());
                if config.expanded_folders.contains(&node.path) {
                    config.expanded_folders.remove(&node.path);
                } else {
                    config.expanded_folders.insert(node.path.clone());
                }
                config.save();
            }

            // Drag start
            if resp.drag_started() {
                if is_root {
                    self.dragging = Some(DragKind::RootFolder(node.path.clone()));
                } else if let Some(parent) = parent_dir {
                    self.dragging = Some(DragKind::Sibling {
                        parent: parent.to_path_buf(),
                        file_name: node.file_name.clone(),
                    });
                }
            }

            // Drop target visual & execution
            if resp.hovered() {
                if let Some(drag) = &self.dragging {
                    let can_drop = match drag {
                        DragKind::RootFolder(src) => is_root && src != &node.path,
                        DragKind::Sibling { parent, file_name } => {
                            parent_dir == Some(parent.as_path()) && file_name != &node.file_name
                        }
                        DragKind::DatasetToDock(_) => false,
                    };
                    if can_drop {
                        let rect = resp.rect;
                        ui.painter().line_segment(
                            [rect.left_bottom(), rect.right_bottom()],
                            egui::Stroke::new(2.0, pal.accent),
                        );
                        if ui.input(|i| i.pointer.any_released()) {
                            match drag.clone() {
                                DragKind::RootFolder(src) => {
                                    *pending_reorder_root = Some((src, node.path.clone()));
                                }
                                DragKind::Sibling { parent, file_name } => {
                                    *pending_reorder_sibling = Some((
                                        parent,
                                        sibling_names.to_vec(),
                                        file_name,
                                        node.file_name.clone(),
                                    ));
                                }
                                DragKind::DatasetToDock(_) => {}
                            }
                            self.dragging = None;
                        }
                    }
                }
            }

            resp.context_menu(|ui| {
                if ui.button("展开 / 折叠").clicked() {
                    if config.expanded_folders.contains(&node.path) {
                        config.expanded_folders.remove(&node.path);
                    } else {
                        config.expanded_folders.insert(node.path.clone());
                    }
                    config.save();
                    ui.close_menu();
                }
                if ui.button("重命名 (仅软件内别名)").clicked() {
                    self.renaming_path = Some(node.path.clone());
                    self.rename_buffer = config.display_name_for(&node.path);
                    self.rename_just_started = true;
                    ui.close_menu();
                }
                if ui.button("刷新此目录").clicked() {
                    *pending_refresh_root = Some(root_owner.to_path_buf());
                    ui.close_menu();
                }
                if ui.button("恢复此目录下已移除项").clicked() {
                    let prefix = node.path.clone();
                    config.hidden_paths.retain(|p| !p.starts_with(&prefix));
                    config.save();
                    *pending_refresh_root = Some(root_owner.to_path_buf());
                    ui.close_menu();
                }
                ui.separator();
                if ui
                    .button(RichText::new("从侧栏移除").color(Color32::from_rgb(200, 60, 60)))
                    .clicked()
                {
                    *pending_remove = Some(node.path.clone());
                    ui.close_menu();
                }
            });
        }

        if is_expanded {
            let child_names: Vec<String> =
                node.children.iter().map(|c| c.file_name.clone()).collect();
            for child in &node.children {
                if !filter_lower.is_empty() && !node_matches_filter(child, filter_lower, config) {
                    continue;
                }
                if child.is_dir {
                    self.render_dir_node(
                        ui,
                        child,
                        root_owner,
                        false,
                        depth + 1,
                        filter_lower,
                        Some(&node.path),
                        &child_names,
                        config,
                        pal,
                        actions,
                        pending_reorder_root,
                        pending_reorder_sibling,
                        pending_remove,
                        pending_refresh_root,
                    );
                } else {
                    let disp = config.display_name_for(&child.path);
                    self.render_file_row(
                        ui,
                        &child.path,
                        &disp,
                        child.format_kind,
                        child.file_size,
                        depth + 1,
                        Some(&node.path),
                        &child_names,
                        config,
                        pal,
                        actions,
                        pending_remove,
                        pending_reorder_sibling,
                    );
                }
            }
        }
    }

    #[allow(clippy::too_many_arguments)]
    fn render_file_row(
        &mut self,
        ui: &mut Ui,
        path: &Path,
        disp_name: &str,
        fmt: Option<SasFormatKind>,
        file_size: u64,
        depth: usize,
        parent_dir: Option<&Path>,
        sibling_names: &[String],
        config: &mut AppConfig,
        pal: &AppPalette,
        actions: &mut Vec<FileTreeAction>,
        pending_remove: &mut Option<PathBuf>,
        pending_reorder_sibling: &mut Option<(PathBuf, Vec<String>, String, String)>,
    ) {
        let indent = (depth as f32) * 14.0 + 14.0;
        let is_renaming = self.renaming_path.as_ref() == Some(&path.to_path_buf());
        let file_name_orig = path
            .file_name()
            .and_then(|s| s.to_str())
            .unwrap_or("")
            .to_string();

        let resp_opt = ui
            .horizontal(|ui| {
                ui.add_space(indent);
                let (badge, badge_color) = match fmt {
                    Some(SasFormatKind::Sas7bdat) => ("SAS", pal.badge_sas),
                    Some(SasFormatKind::Xport) => ("XPT", pal.badge_xpt),
                    None => ("DAT", pal.text_muted),
                };
                ui.label(
                    RichText::new(badge)
                        .size(9.5)
                        .strong()
                        .color(Color32::WHITE)
                        .background_color(badge_color),
                );

                if is_renaming {
                    self.render_rename_input(ui, path, config, actions);
                    None
                } else {
                    let selected = self.selected_path.as_deref() == Some(path);
                    let resp = ui.add(
                        egui::Label::new(
                            RichText::new(disp_name)
                                .size(12.2)
                                .color(if selected { pal.accent } else { pal.text_primary }),
                        )
                        .truncate()
                        .sense(Sense::click_and_drag()),
                    );
                    ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
                        ui.label(
                            RichText::new(format_file_size(file_size))
                                .size(10.5)
                                .color(pal.text_muted),
                        );
                    });
                    Some(resp)
                }
            })
            .inner;

        if let Some(resp) = resp_opt {
            if resp.clicked() {
                self.selected_path = Some(path.to_path_buf());
            }
            if resp.double_clicked() {
                actions.push(FileTreeAction::OpenFile {
                    path: path.to_path_buf(),
                    mode: OpenSplitMode::Tab,
                });
            }

            if resp.drag_started() {
                // If user drags within sibling list, reorder; if dragged outside left panel, open in dock!
                if let Some(parent) = parent_dir {
                    self.dragging = Some(DragKind::Sibling {
                        parent: parent.to_path_buf(),
                        file_name: file_name_orig.clone(),
                    });
                } else {
                    self.dragging = Some(DragKind::DatasetToDock(path.to_path_buf()));
                }
            }

            // Upgrade drag to DatasetToDock if dragged outside file tree x-bounds
            if resp.dragged() {
                if let Some(pos) = ui.ctx().pointer_latest_pos() {
                    if pos.x > ui.max_rect().right() + 20.0 {
                        self.dragging = Some(DragKind::DatasetToDock(path.to_path_buf()));
                    }
                }
            }

            // Sibling drop target
            if resp.hovered() {
                if let Some(DragKind::Sibling { parent, file_name }) = &self.dragging {
                    if parent_dir == Some(parent.as_path()) && file_name != &file_name_orig {
                        let rect = resp.rect;
                        ui.painter().line_segment(
                            [rect.left_bottom(), rect.right_bottom()],
                            egui::Stroke::new(2.0, pal.accent),
                        );
                        if ui.input(|i| i.pointer.any_released()) {
                            *pending_reorder_sibling = Some((
                                parent.clone(),
                                sibling_names.to_vec(),
                                file_name.clone(),
                                file_name_orig.clone(),
                            ));
                            self.dragging = None;
                        }
                    }
                }
            }

            resp.clone().on_hover_text(format!(
                "{}\n大小: {}\n双击打开，或拖拽调整排序 / 拖入右侧分屏",
                path.display(),
                format_file_size(file_size)
            ));

            resp.context_menu(|ui| {
                if ui.button("打开数据集").clicked() {
                    actions.push(FileTreeAction::OpenFile {
                        path: path.to_path_buf(),
                        mode: OpenSplitMode::Tab,
                    });
                    ui.close_menu();
                }
                if ui.button("在右侧分屏并排打开").clicked() {
                    actions.push(FileTreeAction::OpenFile {
                        path: path.to_path_buf(),
                        mode: OpenSplitMode::SplitRight,
                    });
                    ui.close_menu();
                }
                if ui.button("在下方分屏并排打开").clicked() {
                    actions.push(FileTreeAction::OpenFile {
                        path: path.to_path_buf(),
                        mode: OpenSplitMode::SplitBottom,
                    });
                    ui.close_menu();
                }
                ui.separator();
                if ui.button("重命名 (仅软件内别名)").clicked() {
                    self.renaming_path = Some(path.to_path_buf());
                    self.rename_buffer = config.display_name_for(path);
                    self.rename_just_started = true;
                    ui.close_menu();
                }
                if ui
                    .button(RichText::new("从侧栏移除").color(Color32::from_rgb(200, 60, 60)))
                    .clicked()
                {
                    *pending_remove = Some(path.to_path_buf());
                    ui.close_menu();
                }
            });
        }
    }

    fn render_rename_input(
        &mut self,
        ui: &mut Ui,
        path: &Path,
        config: &mut AppConfig,
        actions: &mut Vec<FileTreeAction>,
    ) {
        let resp = ui.add(
            egui::TextEdit::singleline(&mut self.rename_buffer)
                .desired_width(ui.available_width() - 8.0),
        );
        if self.rename_just_started {
            resp.request_focus();
            self.rename_just_started = false;
        }
        let enter = ui.input(|i| i.key_pressed(egui::Key::Enter));
        let esc = ui.input(|i| i.key_pressed(egui::Key::Escape));
        if enter || resp.lost_focus() {
            let new_name = self.rename_buffer.trim().to_string();
            config.set_alias(path.to_path_buf(), new_name.clone());
            actions.push(FileTreeAction::AliasChanged {
                path: path.to_path_buf(),
                new_name: config.display_name_for(path),
            });
            self.renaming_path = None;
        } else if esc {
            self.renaming_path = None;
        }
    }
}

fn scan_directory(dir: &Path, config: &AppConfig, depth: usize) -> FsNode {
    let file_name = dir
        .file_name()
        .and_then(|s| s.to_str())
        .map(|s| s.to_string())
        .unwrap_or_else(|| dir.display().to_string());

    let mut children = Vec::new();
    if depth < 8 {
        if let Ok(read_dir) = std::fs::read_dir(dir) {
            for entry in read_dir.flatten() {
                let p = entry.path();
                if config.hidden_paths.contains(&p) {
                    continue;
                }
                let name = entry.file_name().to_string_lossy().to_string();
                if name.starts_with('.') {
                    continue;
                }
                let Ok(meta) = entry.metadata() else {
                    continue;
                };
                if meta.is_dir() {
                    let sub = scan_directory(&p, config, depth + 1);
                    if !sub.children.is_empty() {
                        children.push(sub);
                    }
                } else if let Some(fmt) = SasFormatKind::from_path(&p) {
                    children.push(FsNode {
                        path: p,
                        file_name: name,
                        is_dir: false,
                        format_kind: Some(fmt),
                        file_size: meta.len(),
                        children: Vec::new(),
                    });
                }
            }
        }
    }

    // Apply persisted custom order if available; otherwise directories first then alphabetical
    if let Some(order) = config.custom_orders.get(dir) {
        let rank: HashMap<&str, usize> = order
            .iter()
            .enumerate()
            .map(|(i, name)| (name.as_str(), i))
            .collect();
        children.sort_by(|a, b| {
            let ra = rank.get(a.file_name.as_str()).copied().unwrap_or(usize::MAX);
            let rb = rank.get(b.file_name.as_str()).copied().unwrap_or(usize::MAX);
            ra.cmp(&rb)
                .then_with(|| b.is_dir.cmp(&a.is_dir))
                .then_with(|| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()))
        });
    } else {
        children.sort_by(|a, b| {
            b.is_dir
                .cmp(&a.is_dir)
                .then_with(|| a.file_name.to_lowercase().cmp(&b.file_name.to_lowercase()))
        });
    }

    FsNode {
        path: dir.to_path_buf(),
        file_name,
        is_dir: true,
        format_kind: None,
        file_size: 0,
        children,
    }
}

fn node_matches_filter(node: &FsNode, filter_lower: &str, config: &AppConfig) -> bool {
    let disp = config.display_name_for(&node.path).to_lowercase();
    if disp.contains(filter_lower) || node.file_name.to_lowercase().contains(filter_lower) {
        return true;
    }
    node.children
        .iter()
        .any(|c| node_matches_filter(c, filter_lower, config))
}

fn collect_all_dirs(node: &FsNode, out: &mut std::collections::HashSet<PathBuf>) {
    if node.is_dir {
        out.insert(node.path.clone());
        for c in &node.children {
            collect_all_dirs(c, out);
        }
    }
}

pub fn format_file_size(bytes: u64) -> String {
    const KB: f64 = 1024.0;
    const MB: f64 = KB * 1024.0;
    const GB: f64 = MB * 1024.0;
    let b = bytes as f64;
    if b >= GB {
        format!("{:.2} GB", b / GB)
    } else if b >= MB {
        format!("{:.1} MB", b / MB)
    } else if b >= KB {
        format!("{:.0} KB", b / KB)
    } else {
        format!("{} B", bytes)
    }
}
