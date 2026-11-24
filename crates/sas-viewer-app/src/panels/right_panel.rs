use crate::state::{DatasetTabState, FocusRegion, RightTab};
use crate::theme::AppPalette;
use egui::{Align2, Color32, FontId, Id, Pos2, Rect, RichText, ScrollArea, Sense, Stroke, Ui, Vec2};
use sas_engine::{format_numeric_value, format_plain_number, ColumnStats, VarType};

/// Render the Right Sidebar containing 3 tabs: Variables / Statistics / Shortcuts.
pub fn show_right_panel(
    ui: &mut Ui,
    active_tab: Option<&mut DatasetTabState>,
    current_right_tab: &mut RightTab,
    focus_var_search: &mut bool,
    pal: &AppPalette,
) {
    // Lock max width strictly to current panel width so searching / filtering NEVER pops the side panel outward!
    let panel_w = ui.available_width().max(180.0);
    ui.set_max_width(panel_w);

    ui.horizontal(|ui| {
        ui.selectable_value(
            current_right_tab,
            RightTab::Variables,
            RichText::new("变量").strong(),
        );
        ui.selectable_value(
            current_right_tab,
            RightTab::Statistics,
            RichText::new("统计量").strong(),
        );
        ui.selectable_value(
            current_right_tab,
            RightTab::Shortcuts,
            RichText::new("快捷键").strong(),
        );
    });
    ui.separator();

    match *current_right_tab {
        RightTab::Variables => {
            if let Some(tab) = active_tab {
                show_variables_tab(ui, tab, current_right_tab, focus_var_search, pal);
            } else {
                show_no_dataset_placeholder(ui, pal);
            }
        }
        RightTab::Statistics => {
            if let Some(tab) = active_tab {
                show_statistics_tab(ui, tab, pal);
            } else {
                show_no_dataset_placeholder(ui, pal);
            }
        }
        RightTab::Shortcuts => {
            show_shortcuts_tab(ui, pal);
        }
    }
}

fn show_no_dataset_placeholder(ui: &mut Ui, pal: &AppPalette) {
    ui.add_space(32.0);
    ui.vertical_centered(|ui| {
        ui.label(RichText::new("未打开数据集").color(pal.text_muted).size(13.0));
        ui.add_space(4.0);
        ui.label(
            RichText::new("拖入 .sas7bdat 或 .xpt 文件，或在左侧双击打开")
                .color(pal.text_muted)
                .size(11.5),
        );
    });
}

fn show_variables_tab(
    ui: &mut Ui,
    tab: &mut DatasetTabState,
    current_right_tab: &mut RightTab,
    focus_var_search: &mut bool,
    pal: &AppPalette,
) {
    let Some(table) = tab.table.clone() else {
        ui.label(RichText::new("数据集加载中...").color(pal.text_muted));
        return;
    };

    let avail_w = ui.available_width().max(180.0);

    // Search box (fixed width so typing never triggers side-panel reflow/pop!)
    ui.horizontal(|ui| {
        let input_w = (avail_w - 26.0).max(80.0);
        let resp = ui.add(
            egui::TextEdit::singleline(&mut tab.var_search)
                .hint_text("搜索变量名或标签... (Ctrl+F)")
                .desired_width(input_w),
        );
        if *focus_var_search {
            resp.request_focus();
            *focus_var_search = false;
        }
        let clear_enabled = !tab.var_search.is_empty();
        if ui
            .add_enabled(clear_enabled, egui::Button::new("×").small())
            .on_hover_text("清空搜索")
            .clicked()
        {
            tab.var_search.clear();
            resp.request_focus();
        }
    });

    let search_lower = tab.var_search.trim().to_lowercase();
    let matched_cols: Vec<usize> = tab
        .column_order
        .iter()
        .copied()
        .filter(|&c| {
            if search_lower.is_empty() {
                return true;
            }
            if let Some(meta) = table.variables.get(c) {
                meta.name.to_lowercase().contains(&search_lower)
                    || meta.label.to_lowercase().contains(&search_lower)
            } else {
                false
            }
        })
        .collect();

    let checked_count = tab.column_visible.iter().filter(|&&v| v).count();
    let total_vars = table.variables.len();

    // Stable toolbar: keep exact same buttons always laid out (never add/remove buttons while typing!)
    ui.horizontal_wrapped(|ui| {
        ui.spacing_mut().item_spacing.x = 4.0;
        if ui.small_button("全选(U)").on_hover_text("显示全部变量").clicked() {
            tab.show_all_variables();
        }
        if ui.small_button("清空(H)").on_hover_text("隐藏全部变量").clicked() {
            tab.hide_all_variables();
        }
        if ui
            .add_enabled(!search_lower.is_empty(), egui::Button::new("选匹配").small())
            .on_hover_text("勾选当前搜索匹配到的变量")
            .clicked()
        {
            for &c in &matched_cols {
                if c < tab.column_visible.len() {
                    tab.column_visible[c] = true;
                }
            }
        }
        ui.label(
            RichText::new(format!("{}/{}", checked_count, total_vars))
                .size(11.0)
                .color(pal.text_muted),
        );
    });

    // Two-column compact header bar: [Checkbox] | Col 1 (变量 / 类型 / Format) | Col 2 (标签)
    let row_h = 23.0;
    let cb_w = 22.0;
    let inner_w = (avail_w - 8.0).max(160.0);
    let col1_w = ((inner_w - cb_w) * 0.56).max(85.0);

    let (hdr_rect, _) = ui.allocate_exact_size(Vec2::new(inner_w, 20.0), Sense::hover());
    ui.painter().rect_filled(hdr_rect, 0.0, pal.bg_header_name);
    ui.painter().line_segment(
        [hdr_rect.left_bottom(), hdr_rect.right_bottom()],
        Stroke::new(1.0, pal.border_header),
    );
    let col1_x = hdr_rect.left() + cb_w;
    let col2_x = col1_x + col1_w;
    ui.painter().line_segment(
        [Pos2::new(col2_x, hdr_rect.top()), Pos2::new(col2_x, hdr_rect.bottom())],
        Stroke::new(1.0, pal.border_grid),
    );
    ui.painter().text(
        Pos2::new(col1_x + 4.0, hdr_rect.center().y),
        Align2::LEFT_CENTER,
        "变量 / 类型 / Format",
        FontId::proportional(10.8),
        pal.text_muted,
    );
    ui.painter().text(
        Pos2::new(col2_x + 5.0, hdr_rect.center().y),
        Align2::LEFT_CENTER,
        "标签",
        FontId::proportional(10.8),
        pal.text_muted,
    );

    let mut jump_to_col: Option<usize> = None;
    let mut stats_for_col: Option<usize> = None;
    let font_name = FontId::proportional(11.8);
    let font_meta = FontId::monospace(10.0);
    let font_label = FontId::proportional(11.0);

    ScrollArea::vertical()
        .id_salt(("var_list_scroll", tab.id))
        .auto_shrink([false, false])
        .show_rows(ui, row_h, matched_cols.len(), |ui, row_range| {
            let list_w = ui.available_width().max(160.0);
            let c1_w = ((list_w - cb_w) * 0.56).max(85.0);
            let _c2_w = (list_w - cb_w - c1_w).max(40.0);

            for list_idx in row_range {
                let col_idx = matched_cols[list_idx];
                let Some(meta) = table.variables.get(col_idx) else {
                    continue;
                };
                let is_selected = tab.selected_var == Some(col_idx);

                let (row_rect, _) = ui.allocate_exact_size(Vec2::new(list_w, row_h), Sense::hover());
                let bg = if is_selected {
                    pal.accent_soft
                } else if list_idx % 2 == 0 {
                    pal.bg_row_even
                } else {
                    pal.bg_row_odd
                };
                ui.painter().rect_filled(row_rect, 0.0, bg);
                ui.painter().line_segment(
                    [row_rect.left_bottom(), row_rect.right_bottom()],
                    Stroke::new(1.0, pal.border_grid),
                );

                // Dedicated checkbox area on the far left (never overlapped by row click!)
                let cb_rect = Rect::from_min_size(row_rect.min, Vec2::new(cb_w, row_h));
                if col_idx < tab.column_visible.len() {
                    let mut checked = tab.column_visible[col_idx];
                    let cb_resp = ui.interact(
                        cb_rect,
                        Id::new(("var_cb", tab.id, col_idx)),
                        Sense::click(),
                    );
                    if cb_resp.clicked() {
                        checked = !checked;
                        tab.column_visible[col_idx] = checked;
                    }
                    let box_rect = Rect::from_center_size(cb_rect.center(), Vec2::splat(13.0));
                    if checked {
                        ui.painter().rect_filled(box_rect, 2.5, pal.accent);
                        ui.painter().text(
                            box_rect.center(),
                            Align2::CENTER_CENTER,
                            "✓",
                            FontId::proportional(10.0),
                            Color32::WHITE,
                        );
                    } else {
                        ui.painter().rect_stroke(
                            box_rect,
                            2.5,
                            Stroke::new(1.2, pal.text_muted),
                            egui::StrokeKind::Inside,
                        );
                    }
                }

                // Two-column painted info area (strictly clipped so panel width NEVER changes!)
                let c1_rect = Rect::from_min_size(
                    Pos2::new(row_rect.left() + cb_w, row_rect.top()),
                    Vec2::new(c1_w, row_h),
                );
                let c2_rect = Rect::from_min_max(
                    Pos2::new(c1_rect.right(), row_rect.top()),
                    row_rect.max,
                );
                ui.painter().line_segment(
                    [c1_rect.right_top(), c1_rect.right_bottom()],
                    Stroke::new(1.0, pal.border_grid),
                );

                // Column 1: VARNAME + compact Type + Format
                let (type_str, type_col) = match meta.var_type {
                    VarType::Numeric => ("N", pal.badge_num),
                    VarType::String => ("C", pal.badge_char),
                };
                let c1_clip = ui.painter().with_clip_rect(c1_rect.shrink2(Vec2::new(3.0, 0.0)));
                let name_galley = c1_clip.layout_no_wrap(
                    meta.name.clone(),
                    font_name.clone(),
                    if is_selected { pal.accent } else { pal.text_primary },
                );
                let name_w = name_galley.size().x;
                c1_clip.galley(
                    Pos2::new(c1_rect.left() + 4.0, c1_rect.center().y - name_galley.size().y * 0.5),
                    name_galley,
                    pal.text_primary,
                );

                let tag_x = c1_rect.left() + 4.0 + name_w + 5.0;
                if tag_x < c1_rect.right() - 8.0 {
                    c1_clip.text(
                        Pos2::new(tag_x, c1_rect.center().y),
                        Align2::LEFT_CENTER,
                        type_str,
                        font_meta.clone(),
                        type_col,
                    );
                    if !meta.format.is_empty() {
                        c1_clip.text(
                            Pos2::new(tag_x + 12.0, c1_rect.center().y),
                            Align2::LEFT_CENTER,
                            &meta.format,
                            font_meta.clone(),
                            pal.text_muted,
                        );
                    }
                }

                // Column 2: Variable Label
                let lbl = if meta.label.is_empty() {
                    "-"
                } else {
                    meta.label.as_str()
                };
                ui.painter()
                    .with_clip_rect(c2_rect.shrink2(Vec2::new(4.0, 0.0)))
                    .text(
                        Pos2::new(c2_rect.left() + 5.0, c2_rect.center().y),
                        Align2::LEFT_CENTER,
                        lbl,
                        font_label.clone(),
                        pal.text_muted,
                    );

                // Interactive hit area for columns 1+2 (excludes checkbox!)
                let info_rect = Rect::from_min_max(c1_rect.min, c2_rect.max);
                let click_resp = ui.interact(
                    info_rect,
                    Id::new(("var_row_click", tab.id, col_idx)),
                    Sense::click(),
                );
                if click_resp.clicked() {
                    tab.selected_var = Some(col_idx);
                    tab.focus_region = FocusRegion::VarList;
                    if let Some((_, sel_c)) = tab.selected_cell {
                        if sel_c != col_idx {
                            tab.selected_cell = None;
                            tab.selected_cells.clear();
                        }
                    }
                }
                if click_resp.double_clicked() {
                    tab.selected_var = Some(col_idx);
                    tab.focus_region = FocusRegion::VarList;
                    if col_idx < tab.column_visible.len() {
                        tab.column_visible[col_idx] = true;
                    }
                    jump_to_col = Some(col_idx);
                }

                click_resp.clone().on_hover_text(format!(
                    "{} ({} | {})\n标签: {}\n单击选中(按 F 看统计 / 按 D 筛选) | 双击跳转列",
                    meta.name,
                    meta.var_type.short_label(),
                    if meta.format.is_empty() { "-" } else { &meta.format },
                    if meta.label.is_empty() { "(无)" } else { &meta.label },
                ));

                click_resp.context_menu(|ui| {
                    if ui.button("查看统计量 (F)").clicked() {
                        stats_for_col = Some(col_idx);
                        ui.close_menu();
                    }
                    if ui.button("跳转到此列").clicked() {
                        if col_idx < tab.column_visible.len() {
                            tab.column_visible[col_idx] = true;
                        }
                        jump_to_col = Some(col_idx);
                        ui.close_menu();
                    }
                    if ui.button("仅显示此变量").clicked() {
                        for v in &mut tab.column_visible {
                            *v = false;
                        }
                        if col_idx < tab.column_visible.len() {
                            tab.column_visible[col_idx] = true;
                        }
                        ui.close_menu();
                    }
                });
            }
        });

    if let Some(col) = jump_to_col {
        tab.scroll_to_col = Some(col);
    }
    if let Some(col) = stats_for_col {
        tab.selected_var = Some(col);
        tab.refresh_stats_for(col);
        *current_right_tab = RightTab::Statistics;
    }
}

fn show_statistics_tab(ui: &mut Ui, tab: &mut DatasetTabState, pal: &AppPalette) {
    let Some(table) = tab.table.clone() else {
        ui.label(RichText::new("数据集加载中...").color(pal.text_muted));
        return;
    };

    if tab.stats_col.is_none() {
        if let Some(col) = tab.active_target_col() {
            tab.refresh_stats_for(col);
        }
    }

    let Some(col_idx) = tab.stats_col else {
        ui.add_space(28.0);
        ui.vertical_centered(|ui| {
            ui.label(RichText::new("尚未选择统计变量").color(pal.text_muted).size(13.0));
            ui.add_space(6.0);
            ui.label(
                RichText::new("选中数据单元格或变量栏中的变量后\n按快捷键 F 即可立即显示统计量与取值分布")
                    .color(pal.text_muted)
                    .size(11.5),
            );
        });
        return;
    };

    if tab.stats_cache_key != Some((tab.filter_version, col_idx)) {
        tab.refresh_stats_for(col_idx);
    }

    let Some(meta) = table.variables.get(col_idx) else {
        return;
    };

    // Variable summary header
    egui::Frame::NONE
        .fill(pal.bg_header_name)
        .inner_margin(egui::Margin::same(7))
        .corner_radius(egui::CornerRadius::same(4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&meta.name).strong().size(13.5).color(pal.accent));
                let (t_str, t_col) = match meta.var_type {
                    VarType::Numeric => ("Num", pal.badge_num),
                    VarType::String => ("Char", pal.badge_char),
                };
                ui.label(RichText::new(t_str).strong().size(10.8).color(t_col));
                if !meta.format.is_empty() {
                    ui.label(
                        RichText::new(&meta.format)
                            .monospace()
                            .size(10.8)
                            .color(pal.text_muted),
                    );
                }
            });
            if !meta.label.is_empty() {
                ui.add(
                    egui::Label::new(
                        RichText::new(&meta.label)
                            .size(11.4)
                            .color(pal.text_primary),
                    )
                    .truncate(),
                );
            }
            let scope_text = if tab.query.filters.is_empty() {
                format!("当前数据行: N = {}", tab.visible_rows.len())
            } else {
                format!(
                    "当前筛选数据: N = {} / {} 行",
                    tab.visible_rows.len(),
                    table.row_count
                )
            };
            ui.label(RichText::new(scope_text).size(10.8).color(pal.text_muted));
        });

    ui.add_space(5.0);

    let Some(stats) = tab.cached_stats.clone() else {
        return;
    };

    // Compact summary section above the selectable value frequency list
    match &stats {
        ColumnStats::String(ss) => {
            egui::Grid::new(("str_summary_grid", tab.id))
                .num_columns(4)
                .spacing([8.0, 3.0])
                .striped(true)
                .show(ui, |ui| {
                    stat_kv_compact(ui, "总计", &ss.total_rows.to_string(), pal);
                    stat_kv_compact(ui, "非空", &ss.valid_count.to_string(), pal);
                    ui.end_row();
                    stat_kv_compact(ui, "空值", &ss.missing_count.to_string(), pal);
                    stat_kv_compact(ui, "唯一值", &ss.distinct_count.to_string(), pal);
                    ui.end_row();
                });
        }
        ColumnStats::Numeric(ns) => {
            let fmt_val = |opt: Option<f64>| -> String {
                match opt {
                    Some(v) => format_plain_number(v, None),
                    None => "-".to_string(),
                }
            };
            let fmt_maybe_date = |opt: Option<f64>| -> String {
                match opt {
                    Some(v) if meta.date_kind.is_temporal() => format_numeric_value(v, meta),
                    Some(v) => format_plain_number(v, None),
                    None => "-".to_string(),
                }
            };

            egui::Grid::new(("num_summary_grid", tab.id))
                .num_columns(4)
                .spacing([8.0, 3.0])
                .striped(true)
                .show(ui, |ui| {
                    stat_kv_compact(ui, "有效N", &ns.valid_count.to_string(), pal);
                    stat_kv_compact(ui, "缺失", &ns.missing_count.to_string(), pal);
                    ui.end_row();
                    stat_kv_compact(ui, "唯一值", &ns.distinct_count.to_string(), pal);
                    stat_kv_compact(ui, "Mean", &fmt_val(ns.mean), pal);
                    ui.end_row();
                    stat_kv_compact(ui, "SD", &fmt_val(ns.std_dev), pal);
                    stat_kv_compact(ui, "Sum", &fmt_val(ns.sum), pal);
                    ui.end_row();
                    stat_kv_compact(ui, "Min", &fmt_maybe_date(ns.min), pal);
                    stat_kv_compact(ui, "Max", &fmt_maybe_date(ns.max), pal);
                    ui.end_row();
                    stat_kv_compact(ui, "Q1", &fmt_maybe_date(ns.q1), pal);
                    stat_kv_compact(ui, "Median", &fmt_maybe_date(ns.median), pal);
                    ui.end_row();
                    stat_kv_compact(ui, "Q3", &fmt_maybe_date(ns.q3), pal);
                    stat_kv_compact(ui, "总计", &ns.total_rows.to_string(), pal);
                    ui.end_row();
                });

            if !ns.tagged_missing_counts.is_empty() {
                ui.horizontal_wrapped(|ui| {
                    ui.label(RichText::new("缺失明细:").size(10.8).color(pal.text_muted));
                    for (tag_str, cnt) in &ns.tagged_missing_counts {
                        ui.label(
                            RichText::new(format!("{}({})", tag_str, cnt))
                                .monospace()
                                .size(10.8)
                                .color(pal.text_missing),
                        );
                    }
                });
            }
        }
    }

    ui.add_space(6.0);

    let (entries, truncated) = match &stats {
        ColumnStats::String(ss) => (ss.frequencies.as_slice(), ss.truncated),
        ColumnStats::Numeric(ns) => (ns.frequencies.as_slice(), ns.truncated),
    };

    // Value list header with selection indicator & quick filter hint
    ui.horizontal_wrapped(|ui| {
        ui.label(RichText::new("具体取值分布").strong().size(12.0));
        if truncated {
            ui.label(RichText::new("(前 500 项)").size(10.5).color(pal.text_muted));
        }
        let n_sel = tab.stats_selected_indices.len();
        if n_sel > 0 {
            ui.label(
                RichText::new(format!("已选 {} 项 · 按 D 筛选", n_sel))
                    .size(11.0)
                    .strong()
                    .color(pal.accent),
            );
        } else {
            ui.label(
                RichText::new("(点击/Ctrl/Shift多选 · 按 D 筛选)")
                    .size(10.5)
                    .color(pal.text_muted),
            );
        }
    });
    ui.separator();

    let mut apply_filter_negate: Option<bool> = None;
    let row_h = 22.0;

    ScrollArea::vertical()
        .id_salt(("stats_freq_scroll", tab.id, col_idx))
        .auto_shrink([false, false])
        .show_rows(ui, row_h, entries.len(), |ui, row_range| {
            let avail_w = ui.available_width().max(150.0);
            for idx in row_range {
                let entry = &entries[idx];
                let (rect, resp) =
                    ui.allocate_exact_size(Vec2::new(avail_w, row_h), Sense::click());

                let is_selected = tab.stats_selected_indices.contains(&idx);
                if is_selected {
                    ui.painter().rect_filled(rect, 0.0, pal.bg_selected_cell);
                    ui.painter().rect_filled(
                        Rect::from_min_size(rect.min, Vec2::new(3.0, row_h)),
                        0.0,
                        pal.accent,
                    );
                } else if resp.hovered() {
                    ui.painter().rect_filled(rect, 0.0, pal.bg_row_hover);
                } else if idx % 2 == 1 {
                    ui.painter().rect_filled(rect, 0.0, pal.bg_row_odd);
                }

                // Inline frequency bar along bottom
                let bar_w = (avail_w * ((entry.percent as f32) / 100.0)).clamp(1.0, avail_w);
                let bar_rect = Rect::from_min_size(
                    Pos2::new(rect.left(), rect.bottom() - 2.5),
                    Vec2::new(bar_w, 2.0),
                );
                ui.painter().rect_filled(bar_rect, 1.0, pal.bar_fill);

                let right_text = format!("{} ({:.1}%)", entry.count, entry.percent);
                let right_w = 96.0;
                let val_clip = Rect::from_min_max(
                    rect.min,
                    Pos2::new((rect.right() - right_w).max(rect.left() + 30.0), rect.bottom()),
                );
                ui.painter().with_clip_rect(val_clip).text(
                    Pos2::new(rect.left() + 6.0, rect.center().y - 1.0),
                    Align2::LEFT_CENTER,
                    &entry.value,
                    FontId::proportional(11.6),
                    if entry.is_missing {
                        pal.text_missing
                    } else {
                        pal.text_primary
                    },
                );
                ui.painter().text(
                    Pos2::new(rect.right() - 4.0, rect.center().y - 1.0),
                    Align2::RIGHT_CENTER,
                    &right_text,
                    FontId::monospace(10.8),
                    pal.text_muted,
                );

                // Selection interactions: Click / Ctrl+Click / Shift+Click + Right-Click + Double-Click
                if resp.clicked() {
                    let (ctrl, shift) = ui.input(|i| (i.modifiers.command || i.modifiers.ctrl, i.modifiers.shift));
                    tab.focus_region = FocusRegion::StatsList;
                    if shift {
                        let anchor = tab.stats_anchor_idx.unwrap_or(idx);
                        let lo = anchor.min(idx);
                        let hi = anchor.max(idx);
                        tab.stats_selected_indices.clear();
                        for k in lo..=hi {
                            tab.stats_selected_indices.insert(k);
                        }
                    } else if ctrl {
                        if tab.stats_selected_indices.contains(&idx)
                            && tab.stats_selected_indices.len() > 1
                        {
                            tab.stats_selected_indices.remove(&idx);
                        } else {
                            tab.stats_selected_indices.insert(idx);
                        }
                        tab.stats_anchor_idx = Some(idx);
                    } else {
                        tab.stats_selected_indices.clear();
                        tab.stats_selected_indices.insert(idx);
                        tab.stats_anchor_idx = Some(idx);
                    }
                }

                if resp.secondary_clicked() {
                    tab.focus_region = FocusRegion::StatsList;
                    if !tab.stats_selected_indices.contains(&idx) {
                        tab.stats_selected_indices.clear();
                        tab.stats_selected_indices.insert(idx);
                        tab.stats_anchor_idx = Some(idx);
                    }
                }

                if resp.double_clicked() {
                    tab.focus_region = FocusRegion::StatsList;
                    if !tab.stats_selected_indices.contains(&idx) {
                        tab.stats_selected_indices.clear();
                        tab.stats_selected_indices.insert(idx);
                        tab.stats_anchor_idx = Some(idx);
                    }
                    apply_filter_negate = Some(false);
                }

                resp.clone().on_hover_text(format!(
                    "值: {}\n频数: {}\n占比: {:.2}%\n单击选中 / Ctrl或Shift多选 / 按 D 筛选",
                    entry.value, entry.count, entry.percent
                ));

                resp.context_menu(|ui| {
                    let n_sel = tab.stats_selected_indices.len();
                    let lbl_eq = if n_sel > 1 {
                        format!("筛选: 属于已选 {} 个值  (D)", n_sel)
                    } else {
                        "筛选: 等于选中值  (D)".to_string()
                    };
                    let lbl_neq = if n_sel > 1 {
                        format!("筛选: 不属于已选 {} 个值", n_sel)
                    } else {
                        "筛选: 不等于选中值".to_string()
                    };
                    if ui.button(lbl_eq).clicked() {
                        apply_filter_negate = Some(false);
                        ui.close_menu();
                    }
                    if ui.button(lbl_neq).clicked() {
                        apply_filter_negate = Some(true);
                        ui.close_menu();
                    }
                    if ui.button("复制该值").clicked() {
                        ui.ctx().copy_text(entry.value.clone());
                        ui.close_menu();
                    }
                });
            }
        });

    if let Some(negate) = apply_filter_negate {
        if let Some((c, op)) = tab.build_stats_selection_filter(negate) {
            if negate {
                tab.query.add_filter(c, op);
            } else {
                tab.query.set_quick_filter_for_column(c, op);
            }
            tab.recompute_view();
        }
    }
}

fn stat_kv_compact(ui: &mut Ui, key: &str, val: &str, pal: &AppPalette) {
    ui.label(RichText::new(key).size(11.2).color(pal.text_muted));
    ui.label(
        RichText::new(val)
            .monospace()
            .strong()
            .size(11.2)
            .color(pal.text_primary),
    );
}

fn show_shortcuts_tab(ui: &mut Ui, pal: &AppPalette) {
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            shortcut_section(
                ui,
                "核心联动快捷键",
                &[
                    ("F", "在统计量栏显示选中单元格/变量的统计摘要及全部具体取值"),
                    ("D", "按选中值执行 = / 多值 IN 筛选（数据区和统计量栏通用！）"),
                    ("Ctrl/Cmd + 点击", "在数据区或统计量栏多选多个不连续值"),
                    ("Shift + 点击", "在数据区或统计量栏连续区间多选"),
                    ("C", "清除当前数据集的所有筛选与排序条件"),
                    ("H", "清空右侧变量栏勾选（隐藏所有变量）"),
                    ("U", "全选右侧变量栏（恢复显示所有变量）"),
                ],
                pal,
            );

            ui.add_space(10.0);
            shortcut_section(
                ui,
                "搜索、列与排序操作",
                &[
                    ("顶部 Search 框", "在当前数据集内检索单元格值（Enter 下一个 / Shift+Enter 上一个）"),
                    ("Alt + ← / →", "将当前选中的变量列向左 / 向右移动位置"),
                    ("拖动列头", "自由拖拽调整变量列顺序"),
                    ("点击列头", "对该变量进行升序/降序/取消排序"),
                    ("Shift + 点击列头", "叠加多个变量进行排序"),
                    ("右键列头", "自适应列宽 / 批量自适应列宽（约 6~30 字符范围）"),
                    ("右键单元格", "单值或多值筛选（等于/不等于/属于/包含/大于/小于等）"),
                ],
                pal,
            );

            ui.add_space(10.0);
            shortcut_section(
                ui,
                "导航与界面控制",
                &[
                    ("方向键", "在数据单元格或统计取值列表间快速移动焦点"),
                    ("PgUp / PgDn", "数据表格快速上下翻页"),
                    ("Ctrl/Cmd + C", "复制当前选中单元格内容（不与 C 键冲突）"),
                    ("Ctrl/Cmd + F", "聚焦右侧变量栏搜索框"),
                    ("Ctrl/Cmd + B", "收起 / 展开左侧文件管理器"),
                    ("Ctrl/Cmd + \\", "收起 / 展开最右侧信息栏"),
                    ("拖入数据集文件", "自动纳入文件所在整个目录的全部 XPT / SAS7BDAT 数据集"),
                ],
                pal,
            );

            ui.add_space(10.0);
            ui.label(
                RichText::new("提示：光标位于任何文本输入框中时，单字母快捷键（F/D/C/H/U）会自动暂停以防误触。")
                    .size(11.0)
                    .color(pal.text_muted),
            );
        });
}

fn shortcut_section(ui: &mut Ui, title: &str, items: &[(&str, &str)], pal: &AppPalette) {
    ui.label(RichText::new(title).strong().size(12.5).color(pal.accent));
    ui.add_space(3.0);
    egui::Grid::new(("sc_grid", title))
        .num_columns(2)
        .spacing([8.0, 5.0])
        .striped(true)
        .show(ui, |ui| {
            for (key, desc) in items {
                egui::Frame::NONE
                    .fill(pal.bg_header_name)
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(egui::Margin::symmetric(5, 2))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(*key)
                                .monospace()
                                .strong()
                                .size(10.8)
                                .color(pal.text_primary),
                        );
                    });
                ui.add(
                    egui::Label::new(RichText::new(*desc).size(11.4).color(pal.text_primary))
                        .wrap(),
                );
                ui.end_row();
            }
        });
}
