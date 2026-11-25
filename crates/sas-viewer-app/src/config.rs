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
    #[serde(default = "default_left_panel_width")]
    pub left_panel_width: f32,
    #[serde(default = "default_right_panel_width")]
    pub right_panel_width: f32,
    /// Per-monitor user display scale multiplier keyed by `round(native_scale_factor * 1000)`
    /// (`1.0` = initial 100% baseline on that monitor).
    #[serde(default)]
    pub monitor_ui_scales: HashMap<i32, f32>,
    /// Per-dataset saved variable checkbox presets (keyed by dataset file path).
    /// Persists across tab closes and restarts until the dataset (or its parent folder)
    /// is removed from the File Manager sidebar.
    #[serde(default)]
    pub dataset_var_presets: HashMap<PathBuf, Vec<String>>,
    #[serde(default)]
    pub dark_mode: bool,
}

fn default_true() -> bool {
    true
}

fn default_left_panel_width() -> f32 {
    250.0
}

fn default_right_panel_width() -> f32 {
    390.0
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
            left_panel_width: default_left_panel_width(),
            right_panel_width: default_right_panel_width(),
            monitor_ui_scales: HashMap::new(),
            dataset_var_presets: HashMap::new(),
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
    /// Also removes any per-dataset saved variable presets attached to the removed dataset
    /// (or datasets inside a removed folder).
    pub fn remove_from_sidebar(&mut self, path: &Path) {
        let removed_as_file = if let Some(pos) = self.root_folders.iter().position(|p| p == path) {
            self.root_folders.remove(pos);
            false
        } else if let Some(pos) = self.root_files.iter().position(|p| p == path) {
            self.root_files.remove(pos);
            true
        } else {
            self.hidden_paths.insert(path.to_path_buf());
            !path.is_dir()
        };
        self.clear_var_presets_under(path, removed_as_file);
        self.save();
    }

    fn clear_var_presets_under(&mut self, removed_path: &Path, removed_as_file: bool) {
        let root_files = &self.root_files;
        let root_folders = &self.root_folders;
        let hidden_paths = &self.hidden_paths;
        self.dataset_var_presets.retain(|dataset_path, _| {
            if dataset_path == removed_path {
                return false;
            }
            if !removed_as_file && dataset_path.starts_with(removed_path) {
                // Keep only if this exact dataset is still independently referenced elsewhere
                let still_in_root_files = root_files.iter().any(|f| f == dataset_path);
                let still_in_other_root = root_folders
                    .iter()
                    .any(|r| dataset_path.starts_with(r) && !hidden_paths.contains(dataset_path));
                return still_in_root_files || still_in_other_root;
            }
            true
        });
    }

    /// Return the saved variable checkbox preset for a dataset path, if any.
    pub fn get_dataset_var_preset(&self, path: &Path) -> Option<&[String]> {
        self.dataset_var_presets
            .get(path)
            .filter(|vars| !vars.is_empty())
            .map(Vec::as_slice)
    }

    /// Save (or clear if empty) the variable checkbox preset for a dataset path.
    pub fn set_dataset_var_preset(&mut self, path: PathBuf, vars: Vec<String>) {
        if vars.is_empty() {
            self.dataset_var_presets.remove(&path);
        } else {
            self.dataset_var_presets.insert(path, vars);
        }
        self.save();
    }

    /// Clear the saved variable checkbox preset for a dataset path.
    pub fn clear_dataset_var_preset(&mut self, path: &Path) {
        if self.dataset_var_presets.remove(path).is_some() {
            self.save();
        }
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

    /// Persist user-chosen display scale multiplier (`1.0` = default 100%) for a monitor DPI key.
    pub fn set_ui_scale_for_monitor(&mut self, native_key: i32, scale: f32) {
        let clamped = scale.clamp(0.6, 2.5);
        if (clamped - 1.0).abs() < 0.005 {
            self.monitor_ui_scales.remove(&native_key);
        } else {
            self.monitor_ui_scales.insert(native_key, clamped);
        }
        self.save();
    }
}

/// Parse user-entered preset variable names (supports `,`, `，`, `;`, `；`, whitespace, and newlines).
/// Preserves entered order and deduplicates case-insensitively (SAS variable names are case-insensitive).
pub fn parse_var_preset_input(raw: &str) -> Vec<String> {
    let mut result = Vec::new();
    let mut seen_upper = HashSet::new();
    for token in raw.split(|c: char| {
        c == ',' || c == '，' || c == ';' || c == '；' || c.is_whitespace()
    }) {
        let cleaned = token
            .trim()
            .trim_matches(|c| c == '"' || c == '\'' || c == '`');
        if cleaned.is_empty() {
            continue;
        }
        let key = cleaned.to_ascii_uppercase();
        if seen_upper.insert(key) {
            result.push(cleaned.to_string());
        }
    }
    result
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_var_preset_handles_commas_spaces_newlines_and_dups() {
        assert_eq!(
            parse_var_preset_input("STUDYID, USUBJID, SUBJID, SITEID"),
            vec!["STUDYID", "USUBJID", "SUBJID", "SITEID"]
        );
        assert_eq!(
            parse_var_preset_input("studyid，USUBJID; \"SUBJID\"\nSITEID\tusubjid"),
            vec!["studyid", "USUBJID", "SUBJID", "SITEID"]
        );
        assert!(parse_var_preset_input("   , ， ; \n ").is_empty());
    }

    #[test]
    fn dataset_var_preset_follows_dataset_until_removed_from_sidebar() {
        let mut cfg = AppConfig::default();
        let dir_a = PathBuf::from("/tmp/study_a");
        let dm_a = dir_a.join("dm.sas7bdat");
        let ae_a = dir_a.join("ae.sas7bdat");
        let dm_b = PathBuf::from("/tmp/study_b/dm.sas7bdat");

        cfg.root_folders.push(dir_a.clone());
        cfg.root_folders.push(PathBuf::from("/tmp/study_b"));

        // Freshly opened dataset has no preset
        assert!(cfg.get_dataset_var_preset(&dm_a).is_none());

        // Saving presets binds each preset to its dataset
        cfg.dataset_var_presets
            .insert(dm_a.clone(), vec!["STUDYID".into(), "USUBJID".into()]);
        cfg.dataset_var_presets
            .insert(ae_a.clone(), vec!["USUBJID".into(), "AETERM".into()]);
        cfg.dataset_var_presets
            .insert(dm_b.clone(), vec!["SUBJID".into()]);

        assert_eq!(
            cfg.get_dataset_var_preset(&dm_a),
            Some(&["STUDYID".to_string(), "USUBJID".to_string()][..])
        );

        // Removing a single file clears ONLY that file's preset
        cfg.hidden_paths.insert(dm_a.clone());
        cfg.clear_var_presets_under(&dm_a, true);
        assert!(cfg.get_dataset_var_preset(&dm_a).is_none());
        assert!(cfg.get_dataset_var_preset(&ae_a).is_some());
        assert!(cfg.get_dataset_var_preset(&dm_b).is_some());

        // Removing the parent root folder clears remaining presets under it, leaving other folders intact
        cfg.root_folders.retain(|p| p != &dir_a);
        cfg.clear_var_presets_under(&dir_a, false);
        assert!(cfg.get_dataset_var_preset(&ae_a).is_none());
        assert!(cfg.get_dataset_var_preset(&dm_b).is_some());
    }
}
