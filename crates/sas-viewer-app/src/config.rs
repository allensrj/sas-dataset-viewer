use directories::ProjectDirs;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::path::{Path, PathBuf};

/// Persisted workspace configuration (in-app aliases, custom tree order, sidebar state, etc.).
/// Never mutates or deletes user files on disk.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AppConfig {
    pub root_folders: Vec<PathBuf>,
    /// Standalone files added directly without a parent folder.
    #[serde(default)]
    pub root_files: Vec<PathBuf>,
    /// In-app display alias per file/folder path (Rename only changes display alias).
    #[serde(default)]
    pub aliases: HashMap<PathBuf, String>,
    /// Custom display order of child file/folder names within a parent directory.
    #[serde(default)]
    pub custom_orders: HashMap<PathBuf, Vec<String>>,
    /// Paths removed from the sidebar display without deleting on disk.
    #[serde(default)]
    pub hidden_paths: HashSet<PathBuf>,
    /// Expanded folder paths.
    #[serde(default)]
    pub expanded_folders: HashSet<PathBuf>,
    #[serde(default = "default_true")]
    pub left_panel_open: bool,
    #[serde(default = "default_true")]
    pub right_panel_open: bool,
    #[serde(default)]
    pub dark_mode: bool,
}

fn default_true() -> bool {
    true
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            root_folders: Vec::new(),
            root_files: Vec::new(),
            aliases: HashMap::new(),
            custom_orders: HashMap::new(),
            hidden_paths: HashSet::new(),
            expanded_folders: HashSet::new(),
            left_panel_open: true,
            right_panel_open: true,
            dark_mode: false,
        }
    }
}

impl AppConfig {
    pub fn config_path() -> Option<PathBuf> {
        let proj = ProjectDirs::from("com", "sasviewer", "SasDatasetViewer")?;
        Some(proj.config_dir().join("config.json"))
    }

    pub fn load() -> Self {
        let Some(path) = Self::config_path() else {
            return Self::default();
        };
        if let Ok(text) = std::fs::read_to_string(&path) {
            if let Ok(cfg) = serde_json::from_str::<Self>(&text) {
                return cfg;
            }
        }
        Self::default()
    }

    pub fn save(&self) {
        let Some(path) = Self::config_path() else {
            return;
        };
        if let Some(parent) = path.parent() {
            let _ = std::fs::create_dir_all(parent);
        }
        if let Ok(json) = serde_json::to_string_pretty(self) {
            let _ = std::fs::write(&path, json);
        }
    }

    pub fn display_name_for(&self, path: &Path) -> String {
        if let Some(alias) = self.aliases.get(path) {
            if !alias.trim().is_empty() {
                return alias.clone();
            }
        }
        path.file_name()
            .and_then(|s| s.to_str())
            .map(|s| s.to_string())
            .unwrap_or_else(|| path.display().to_string())
    }

    pub fn add_root_folder(&mut self, folder: PathBuf) {
        self.hidden_paths.remove(&folder);
        if !self.root_folders.contains(&folder) {
            self.root_folders.push(folder.clone());
        }
        self.expanded_folders.insert(folder);
        self.save();
    }

    pub fn add_root_file(&mut self, file: PathBuf) {
        self.hidden_paths.remove(&file);
        if !self.root_files.contains(&file) {
            self.root_files.push(file);
        }
        self.save();
    }

    /// Remove from sidebar only (never deletes on disk).
    pub fn remove_from_sidebar(&mut self, path: &Path) {
        if let Some(pos) = self.root_folders.iter().position(|p| p == path) {
            self.root_folders.remove(pos);
        } else if let Some(pos) = self.root_files.iter().position(|p| p == path) {
            self.root_files.remove(pos);
        } else {
            self.hidden_paths.insert(path.to_path_buf());
        }
        self.save();
    }

    /// Rename in-app display alias only (never renames on disk).
    pub fn set_alias(&mut self, path: PathBuf, new_name: String) {
        let trimmed = new_name.trim().to_string();
        if trimmed.is_empty() {
            self.aliases.remove(&path);
        } else {
            self.aliases.insert(path, trimmed);
        }
        self.save();
    }

    /// Reorder root folders by moving `src` before/after `dst`.
    pub fn reorder_root_folder(&mut self, src: &Path, dst: &Path) {
        let Some(from_idx) = self.root_folders.iter().position(|p| p == src) else {
            return;
        };
        let Some(to_idx) = self.root_folders.iter().position(|p| p == dst) else {
            return;
        };
        if from_idx == to_idx {
            return;
        }
        let item = self.root_folders.remove(from_idx);
        let insert_idx = if from_idx < to_idx { to_idx } else { to_idx };
        let insert_idx = insert_idx.min(self.root_folders.len());
        self.root_folders.insert(insert_idx, item);
        self.save();
    }

    /// Reorder sibling entries inside `parent_dir` given current visible sibling names.
    pub fn reorder_sibling(
        &mut self,
        parent_dir: &Path,
        current_names: &[String],
        src_name: &str,
        dst_name: &str,
    ) {
        let mut order: Vec<String> = current_names.to_vec();
        let Some(from_idx) = order.iter().position(|n| n == src_name) else {
            return;
        };
        let Some(to_idx) = order.iter().position(|n| n == dst_name) else {
            return;
        };
        if from_idx == to_idx {
            return;
        }
        let item = order.remove(from_idx);
        let insert_idx = to_idx.min(order.len());
        order.insert(insert_idx, item);
        self.custom_orders.insert(parent_dir.to_path_buf(), order);
        self.save();
    }
}
