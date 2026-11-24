use crate::state::{DatasetTabState, FilterDialogOpKind, FilterDialogState};
use crate::theme::AppPalette;
use egui::{
    Align, Align2, Color32, CornerRadius, FontId, Id, Pos2, Rect, RichText, ScrollArea, Sense,
    Stroke, Ui, Vec2,
};
use sas_engine::{
    build_cell_equals_filter, build_multi_cell_filter, format_missing_tag, format_numeric_value,
    ColumnData, FilterOp, VarType, MISSING_VALID,
};

const ROW_NUM_WIDTH: f32 = 58.0;
const HDR_NAME_HEIGHT: f32 = 24.0;
const HDR_LABEL_HEIGHT: f32 = 20.0;
const HEADER_TOTAL_HEIGHT: f32 = HDR_NAME_HEIGHT + HDR_LABEL_HEIGHT;
const ROW_HEIGHT: f32 = 22.0;

#[derive(Debug, Clone)]
pub enum GridAction {
    ShowStatsFor(usize),
    CopyText(String),
}

/// Render a dataset tab's full data viewer: active filter/sort bar + frozen two-tier header + 2D virtualized grid.
pub fn show_data_grid(
    ui: &mut Ui,
    tab: &mut DatasetTabState,
    pal: &AppPalette,
    is_active_tab: bool,
) -> Vec<GridAction> {
    let mut actions = Vec::new();

    if let Some(err) = tab.load_error.clone() {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("⚠ 加载数据集失败")
                        .size(16.0)
                        .strong()
                        .color(Color32::from_rgb(210, 60, 60)),
                );
                ui.add_space(6.0);
                ui.label(RichText::new(err).color(pal.text_muted));
                ui.add_space(10.0);
                if ui.button("重试加载").clicked() {
                    let ctx = ui.ctx().clone();
                    tab.start_loading(Some(ctx));
                }
            });
        });
        return actions;
    }

    let Some(table) = tab.table.clone() else {
        ui.centered_and_justified(|ui| {
            ui.horizontal(|ui| {
                ui.spinner();
                ui.label(
                    RichText::new(format!("正在高速解析 {} ...", tab.display_name))
                        .size(14.0)
                        .color(pal.text_muted),
                );
            });
        });
        return actions;
    };

    if tab.loading {
        ui.horizontal(|ui| {
            ui.spinner();
            let pct = if tab.load_progress > 0.0 {
                format!("{:.0}%", tab.load_progress * 100.0)
            } else {
                "流式读取中".to_string()
            };
            ui.label(
                RichText::new(format!(
                    "后台继续加载剩余行：已加载 {} 行 ({}) — 可立即浏览首屏数据",
                    tab.rows_loaded_so_far, pct
                ))
                .size(11.5)
                .color(pal.accent),
            );
        });
    }

    // Active Filter & Sort summary bar
    if !tab.query.is_empty() {
        let mut remove_filter_idx: Option<usize> = None;
        let mut remove_sort_col: Option<usize> = None;
        let mut flip_sort_col: Option<usize> = None;
        let mut clear_all = false;

        egui::Frame::NONE
            .fill(pal.bg_header_label)
            .inner_margin(egui::Margin::symmetric(6, 4))
            .show(ui, |ui| {
                ui.horizontal_wrapped(|ui| {
                    if !tab.query.filters.is_empty() {
                        ui.label(RichText::new("筛选:").size(11.5).strong().color(pal.text_muted));
                        for (i, cond) in tab.query.filters.iter().enumerate() {
                            if let Some(meta) = table.variables.get(cond.col_idx) {
                                let chip_text = format!("{}  ×", cond.op.display_chip(meta));
                                let btn = egui::Button::new(
                                    RichText::new(chip_text).size(11.2).color(pal.text_primary),
                                )
                                .fill(pal.bg_chip)
                                .stroke(Stroke::new(1.0, pal.accent))
                                .corner_radius(CornerRadius::same(10));
                                if ui.add(btn).on_hover_text("点击移除此筛选条件").clicked() {
                                    remove_filter_idx = Some(i);
                                }
                            }
                        }
                    }

                    if !tab.query.sorts.is_empty() {
                        ui.label(RichText::new("排序:").size(11.5).strong().color(pal.text_muted));
                        for (rank, sk) in tab.query.sorts.iter().enumerate() {
                            if let Some(meta) = table.variables.get(sk.col_idx) {
                                let arrow = if sk.ascending { "▲" } else { "▼" };
                                let label = format!("{}: {} {}", rank + 1, meta.name, arrow);
                                let resp = ui
                                    .add(
                                        egui::Button::new(
                                            RichText::new(label).size(11.2).color(pal.text_primary),
                                        )
                                        .fill(pal.bg_header_name)
                                        .corner_radius(CornerRadius::same(10)),
                                    )
                                    .on_hover_text("左键切换升降序，右键移除此排序");
                                if resp.clicked() {
                                    flip_sort_col = Some(sk.col_idx);
                                }
                                if resp.secondary_clicked() {
                                    remove_sort_col = Some(sk.col_idx);
                                }
                            }
                        }
                    }

                    if ui
                        .small_button(RichText::new("清除全部 (C)").color(pal.accent))
                        .clicked()
                    {
                        clear_all = true;
                    }
                });
            });

        if clear_all {
            tab.clear_all_filters_and_sorts();
        } else if let Some(idx) = remove_filter_idx {
            tab.query.filters.remove(idx);
            tab.recompute_view();
        } else if let Some(col) = remove_sort_col {
            tab.query.remove_sort(col);
            tab.recompute_view();
        } else if let Some(col) = flip_sort_col {
            if let Some(sk) = tab.query.sorts.iter_mut().find(|s| s.col_idx == col) {
                sk.ascending = !sk.ascending;
            }
            tab.recompute_view();
        }
    }

    let vis_cols = tab.visible_columns();
    if vis_cols.is_empty() {
        ui.centered_and_justified(|ui| {
            ui.vertical_centered(|ui| {
                ui.label(
                    RichText::new("当前已隐藏全部变量")
                        .size(15.0)
                        .strong()
                        .color(pal.text_muted),
                );
                ui.add_space(6.0);
                ui.label(
                    RichText::new("按 U 键全选恢复全部变量，或在右侧「变量」栏勾选需要展示的变量")
                        .size(12.0)
                        .color(pal.text_muted),
                );
                ui.add_space(10.0);
                if ui.button("恢复全部变量 (U)").clicked() {
                    tab.show_all_variables();
                }
            });
        });
        return actions;
    }

    let mut col_x_starts = Vec::with_capacity(vis_cols.len() + 1);
    let mut acc_x = 0.0f32;
    for &c in &vis_cols {
        col_x_starts.push(acc_x);
        acc_x += tab.column_widths.get(c).copied().unwrap_or(110.0);
    }
    col_x_starts.push(acc_x);
    let total_data_w = acc_x;
    let total_rows = tab.visible_rows.len();
    let total_data_h = (total_rows as f32) * ROW_HEIGHT;

    let avail = ui.available_rect_before_wrap();
    let viewport_w = (avail.width() - ROW_NUM_WIDTH).max(40.0);
    let viewport_h = (avail.height() - HEADER_TOTAL_HEIGHT).max(40.0);

    if let Some(target_col) = tab.scroll_to_col.take() {
        if let Some(vis_idx) = vis_cols.iter().position(|&c| c == target_col) {
            let cx0 = col_x_starts[vis_idx];
            let cx1 = col_x_starts[vis_idx + 1];
            let cur_x = tab.grid_scroll_offset.x;
            if cx0 < cur_x {
                tab.grid_scroll_offset.x = cx0;
            } else if cx1 > cur_x + viewport_w - 16.0 {
                tab.grid_scroll_offset.x = (cx1 - viewport_w + 24.0).max(0.0);
            }
        }
    }
    if let Some(target_row) = tab.scroll_to_row.take() {
        let ry0 = (target_row as f32) * ROW_HEIGHT;
        let ry1 = ry0 + ROW_HEIGHT;
        let cur_y = tab.grid_scroll_offset.y;
        if ry0 < cur_y {
            tab.grid_scroll_offset.y = ry0;
        } else if ry1 > cur_y + viewport_h - 18.0 {
            tab.grid_scroll_offset.y = (ry1 - viewport_h + 24.0).max(0.0);
        }
    }

    let (grid_rect, _) = ui.allocate_exact_size(avail.size(), Sense::hover());

    if is_active_tab {
        ui.painter().line_segment(
            [grid_rect.left_top(), grid_rect.right_top()],
            Stroke::new(2.0, pal.accent),
        );
    }

    // 1. Frozen Corner Header (#)
    let corner_rect = Rect::from_min_size(
        grid_rect.min,
        Vec2::new(ROW_NUM_WIDTH, HEADER_TOTAL_HEIGHT),
    );
    ui.painter().rect_filled(corner_rect, 0.0, pal.bg_header_name);
    ui.painter().line_segment(
        [
            Pos2::new(corner_rect.left(), corner_rect.top() + HDR_NAME_HEIGHT),
            Pos2::new(corner_rect.right(), corner_rect.top() + HDR_NAME_HEIGHT),
        ],
        Stroke::new(1.0, pal.border_grid),
    );
    ui.painter().line_segment(
        [corner_rect.right_top(), corner_rect.right_bottom()],
        Stroke::new(1.0, pal.border_header),
    );
    ui.painter().line_segment(
        [corner_rect.left_bottom(), corner_rect.right_bottom()],
        Stroke::new(1.0, pal.border_header),
    );
    ui.painter().text(
        Pos2::new(corner_rect.center().x, corner_rect.top() + HDR_NAME_HEIGHT * 0.5),
        Align2::CENTER_CENTER,
        "#",
        FontId::proportional(12.0),
        pal.text_muted,
    );
    ui.painter().text(
        Pos2::new(
            corner_rect.center().x,
            corner_rect.top() + HDR_NAME_HEIGHT + HDR_LABEL_HEIGHT * 0.5,
        ),
        Align2::CENTER_CENTER,
        "行号",
        FontId::proportional(10.5),
        pal.text_muted,
    );

    let corner_resp = ui.interact(corner_rect, Id::new(("corner_hdr", tab.id)), Sense::click());
    corner_resp.context_menu(|ui| {
        if ui.button("批量自适应所有可见列宽").clicked() {
            tab.auto_fit_all_visible_columns();
            ui.close_menu();
        }
        if ui.button("重置所有列宽").clicked() {
            tab.reset_all_column_widths();
            ui.close_menu();
        }
        ui.separator();
        if ui.button("全选所有变量 (U)").clicked() {
            tab.show_all_variables();
            ui.close_menu();
        }
        if ui.button("隐藏所有变量 (H)").clicked() {
            tab.hide_all_variables();
            ui.close_menu();
        }
        if ui.button("清除所有筛选与排序 (C)").clicked() {
            tab.clear_all_filters_and_sorts();
            ui.close_menu();
        }
    });

    // 2. Scrollable Body Area
    let body_clip_rect = Rect::from_min_max(
        Pos2::new(
            grid_rect.left() + ROW_NUM_WIDTH,
            grid_rect.top() + HEADER_TOTAL_HEIGHT,
        ),
        grid_rect.max,
    );

    let mut deferred_sort: Option<(usize, bool)> = None;
    let mut deferred_set_sort: Option<(usize, bool, bool)> = None;
    let mut deferred_remove_sort: Option<usize> = None;
    let mut deferred_reorder: Option<(usize, usize)> = None;
    let mut deferred_resize: Option<(usize, f32)> = None;
    let mut deferred_autofit_col: Option<usize> = None;
    let mut deferred_autofit_all = false;
    let mut deferred_reset_widths = false;
    let mut deferred_hide_col: Option<usize> = None;
    let mut deferred_move_col: Option<(usize, isize)> = None;
    let mut deferred_quick_filter: Option<(usize, FilterOp, bool)> = None;
    let mut deferred_open_filter_dlg: Option<FilterDialogState> = None;
    let mut deferred_clear_all = false;

    let current_search_cell: Option<(usize, usize)> = tab
        .search_current_idx
        .and_then(|i| tab.search_matches.get(i).copied());
    let active_search_lower: Option<String> = if !tab.search_executed_query.is_empty() {
        Some(tab.search_executed_query.to_lowercase())
    } else {
        None
    };

    let mut body_ui = ui.new_child(
        egui::UiBuilder::new()
            .max_rect(body_clip_rect)
            .layout(egui::Layout::top_down(Align::Min)),
    );

    let scroll_out = ScrollArea::both()
        .id_salt(("data_grid_scroll", tab.id))
        .scroll_offset(tab.grid_scroll_offset)
        .auto_shrink([false, false])
        .show_viewport(&mut body_ui, |ui, viewport| {
            let content_size = Vec2::new(
                total_data_w.max(body_clip_rect.width()),
                total_data_h.max(body_clip_rect.height()),
            );
            let (content_rect, _) = ui.allocate_exact_size(content_size, Sense::hover());

            if total_rows == 0 {
                ui.painter().text(
                    Pos2::new(
                        content_rect.left() + viewport.min.x + body_clip_rect.width() * 0.5,
                        content_rect.top() + viewport.min.y + 60.0,
                    ),
                    Align2::CENTER_CENTER,
                    "当前筛选条件下无匹配数据（按 C 键清除筛选）",
                    FontId::proportional(13.5),
                    pal.text_muted,
                );
                return;
            }

            let first_row = ((viewport.min.y / ROW_HEIGHT).floor().max(0.0) as usize).min(total_rows);
            let end_row =
                (((viewport.max.y / ROW_HEIGHT).ceil().max(0.0) as usize) + 1).min(total_rows);

            let first_col_vis = col_x_starts
                .partition_point(|&x| x <= viewport.min.x)
                .saturating_sub(1);
            let end_col_vis = col_x_starts
                .partition_point(|&x| x < viewport.max.x)
                .min(vis_cols.len());

            let painter = ui.painter().with_clip_rect(body_clip_rect);
            let pointer_pos = ui.ctx().pointer_latest_pos();
            let font_cell = FontId::proportional(12.2);

            for vis_r in first_row..end_row {
                let orig_row = tab.visible_rows[vis_r] as usize;
                let y0 = content_rect.top() + (vis_r as f32) * ROW_HEIGHT;
                let y1 = y0 + ROW_HEIGHT;

                let is_sel_row = tab.selected_cell.map(|(r, _)| r == vis_r).unwrap_or(false);
                let row_full_rect = Rect::from_min_max(
                    Pos2::new(content_rect.left() + viewport.min.x, y0),
                    Pos2::new(content_rect.left() + viewport.max.x, y1),
                );
                let is_hovered_row = pointer_pos
                    .map(|p| body_clip_rect.contains(p) && p.y >= y0 && p.y < y1)
                    .unwrap_or(false);

                let bg = if is_sel_row {
                    pal.bg_selected_row
                } else if is_hovered_row {
                    pal.bg_row_hover
                } else if vis_r % 2 == 0 {
                    pal.bg_row_even
                } else {
                    pal.bg_row_odd
                };
                painter.rect_filled(row_full_rect, 0.0, bg);

                for c_vis in first_col_vis..end_col_vis {
                    let orig_col = vis_cols[c_vis];
                    let x0 = content_rect.left() + col_x_starts[c_vis];
                    let x1 = content_rect.left() + col_x_starts[c_vis + 1];
                    let cell_rect = Rect::from_min_max(Pos2::new(x0, y0), Pos2::new(x1, y1));

                    let meta = &table.variables[orig_col];
                    let (text, is_missing, align_right) = match &table.columns[orig_col] {
                        ColumnData::Numeric(nc) => {
                            let tag = nc.missing_tag(orig_row);
                            if tag != MISSING_VALID {
                                (format_missing_tag(tag), true, true)
                            } else {
                                (
                                    format_numeric_value(nc.values[orig_row], meta),
                                    false,
                                    !meta.date_kind.is_temporal(),
                                )
                            }
                        }
                        ColumnData::String(sc) => {
                            let s = sc.get_str(orig_row);
                            (s.to_string(), s.is_empty(), false)
                        }
                    };

                    // Search highlight (current match vs other visible matches)
                    let is_cur_search = current_search_cell == Some((vis_r, orig_col));
                    let is_any_search = is_cur_search
                        || active_search_lower
                            .as_deref()
                            .map(|needle| !text.is_empty() && text.to_lowercase().contains(needle))
                            .unwrap_or(false);

                    let is_primary_sel = tab.selected_cell == Some((vis_r, orig_col));
                    let is_multi_sel = tab.selected_cells.contains(&(vis_r, orig_col));

                    if is_cur_search {
                        painter.rect_filled(cell_rect, 0.0, pal.bg_search_current);
                    } else if is_multi_sel || is_primary_sel {
                        painter.rect_filled(cell_rect, 0.0, pal.bg_selected_cell);
                    } else if is_any_search {
                        painter.rect_filled(cell_rect, 0.0, pal.bg_search_match);
                    }

                    if is_primary_sel {
                        painter.rect_stroke(
                            cell_rect.shrink(0.5),
                            0.0,
                            Stroke::new(1.6, pal.accent),
                            egui::StrokeKind::Inside,
                        );
                    } else if is_multi_sel {
                        painter.rect_stroke(
                            cell_rect.shrink(0.5),
                            0.0,
                            Stroke::new(1.0, pal.accent),
                            egui::StrokeKind::Inside,
                        );
                    }

                    if !text.is_empty() {
                        let cell_clip = painter.with_clip_rect(cell_rect.shrink2(Vec2::new(4.0, 0.0)));
                        let color = if is_missing {
                            pal.text_missing
                        } else {
                            pal.text_primary
                        };
                        if align_right {
                            cell_clip.text(
                                Pos2::new(cell_rect.right() - 6.0, cell_rect.center().y),
                                Align2::RIGHT_CENTER,
                                &text,
                                font_cell.clone(),
                                color,
                            );
                        } else {
                            cell_clip.text(
                                Pos2::new(cell_rect.left() + 6.0, cell_rect.center().y),
                                Align2::LEFT_CENTER,
                                &text,
                                font_cell.clone(),
                                color,
                            );
                        }
                    }

                    painter.line_segment(
                        [Pos2::new(x1, y0), Pos2::new(x1, y1)],
                        Stroke::new(1.0, pal.border_grid),
                    );
                }

                painter.line_segment(
                    [
                        Pos2::new(content_rect.left() + viewport.min.x, y1),
                        Pos2::new(content_rect.left() + viewport.max.x, y1),
                    ],
                    Stroke::new(1.0, pal.border_grid),
                );
            }

            let visible_body_rect = Rect::from_min_max(
                Pos2::new(
                    content_rect.left() + viewport.min.x,
                    content_rect.top() + viewport.min.y,
                ),
                Pos2::new(
                    content_rect.left() + viewport.max.x,
                    content_rect.top() + viewport.max.y,
                ),
            );
            let grid_resp = ui.interact(
                visible_body_rect,
                Id::new(("grid_body_interact", tab.id)),
                Sense::click(),
            );

            let locate_cell = |pos: Pos2| -> Option<(usize, usize)> {
                let rel_y = pos.y - content_rect.top();
                let rel_x = pos.x - content_rect.left();
                if rel_y < 0.0 || rel_x < 0.0 {
                    return None;
                }
                let vis_r = (rel_y / ROW_HEIGHT).floor() as usize;
                if vis_r >= total_rows {
                    return None;
                }
                let c_vis = col_x_starts.partition_point(|&x| x <= rel_x).checked_sub(1)?;
                let orig_col = *vis_cols.get(c_vis)?;
                Some((vis_r, orig_col))
            };

            if grid_resp.clicked() {
                if let Some(pos) = grid_resp.interact_pointer_pos() {
                    if let Some((vis_r, orig_col)) = locate_cell(pos) {
                        let (ctrl, shift) = ui.input(|i| {
                            (i.modifiers.command || i.modifiers.ctrl, i.modifiers.shift)
                        });
                        if shift {
                            tab.select_cell_range_to(vis_r, orig_col);
                        } else if ctrl {
                            tab.toggle_cell_selection(vis_r, orig_col);
                        } else {
                            tab.select_single_cell(vis_r, orig_col);
                        }
                    }
                }
            }

            if grid_resp.secondary_clicked() {
                if let Some(pos) = grid_resp.interact_pointer_pos() {
                    if let Some((vis_r, orig_col)) = locate_cell(pos) {
                        if !tab.selected_cells.contains(&(vis_r, orig_col)) {
                            tab.select_single_cell(vis_r, orig_col);
                        } else {
                            // Keep existing multi-cell selection intact for multi-value filtering!
                            tab.selected_cell = Some((vis_r, orig_col));
                            tab.selected_var = Some(orig_col);
                        }
                    }
                }
            }

            // Right-click cell context menu (supports single AND multi-cell selection filtering!)
            grid_resp.context_menu(|ui| {
                let Some((vis_r, orig_col)) = tab.selected_cell else {
                    ui.label("请先选中一个单元格");
                    return;
                };
                let Some(&orig_row) = tab.visible_rows.get(vis_r) else {
                    return;
                };
                let orig_row = orig_row as usize;
                let meta = &table.variables[orig_col];
                let formatted = table.format_cell(orig_row, orig_col);
                let preview = truncate_preview(&formatted, 28);

                // Check whether multiple distinct values are selected in this column
                let sel_orig_rows = tab.selected_orig_rows_for_col(orig_col);
                let multi_in_op = build_multi_cell_filter(&table, &sel_orig_rows, orig_col, false);
                let multi_not_in_op =
                    build_multi_cell_filter(&table, &sel_orig_rows, orig_col, true);
                let is_multi_distinct = matches!(
                    multi_in_op,
                    Some(FilterOp::StrIn(_)) | Some(FilterOp::NumIn { .. })
                );

                if is_multi_distinct {
                    if let Some(op) = &multi_in_op {
                        ui.label(
                            RichText::new(format!("已多选: {}", op.display_chip(meta)))
                                .strong()
                                .size(12.0)
                                .color(pal.accent),
                        );
                    }
                    ui.separator();
                    if let Some(op) = multi_in_op {
                        if ui.button("筛选: 属于已选多个值  (D)").clicked() {
                            deferred_quick_filter = Some((orig_col, op, true));
                            ui.close_menu();
                        }
                    }
                    if let Some(op) = multi_not_in_op {
                        if ui.button("筛选: 不属于已选多个值").clicked() {
                            deferred_quick_filter = Some((orig_col, op, false));
                            ui.close_menu();
                        }
                    }
                    ui.separator();
                } else {
                    ui.label(
                        RichText::new(format!(
                            "{} = {}",
                            meta.name,
                            if preview.is_empty() { "(空)" } else { &preview }
                        ))
                        .strong()
                        .size(12.0),
                    );
                    ui.separator();
                }

                match &table.columns[orig_col] {
                    ColumnData::String(sc) => {
                        let raw_s = sc.get_str(orig_row).to_string();
                        if ui
                            .button(format!(
                                "筛选: 等于 \"{}\"  (D)",
                                truncate_preview(&raw_s, 18)
                            ))
                            .clicked()
                        {
                            let op = if raw_s.is_empty() {
                                FilterOp::StrIsEmpty
                            } else {
                                FilterOp::StrEq(raw_s.clone())
                            };
                            deferred_quick_filter = Some((orig_col, op, true));
                            ui.close_menu();
                        }
                        if ui
                            .button(format!(
                                "筛选: 不等于 \"{}\"",
                                truncate_preview(&raw_s, 18)
                            ))
                            .clicked()
                        {
                            let op = if raw_s.is_empty() {
                                FilterOp::StrIsNotEmpty
                            } else {
                                FilterOp::StrNeq(raw_s.clone())
                            };
                            deferred_quick_filter = Some((orig_col, op, false));
                            ui.close_menu();
                        }
                        if ui.button("筛选: 属于多个值 (逗号分隔)...").clicked() {
                            deferred_open_filter_dlg = Some(FilterDialogState {
                                col_idx: orig_col,
                                op_kind: FilterDialogOpKind::StrIn,
                                input_primary: raw_s.clone(),
                                input_secondary: String::new(),
                                replace_same_col: true,
                                error_msg: None,
                                just_opened: true,
                            });
                            ui.close_menu();
                        }
                        if ui.button("筛选: 包含 (Contains)...").clicked() {
                            deferred_open_filter_dlg = Some(FilterDialogState {
                                col_idx: orig_col,
                                op_kind: FilterDialogOpKind::StrContains,
                                input_primary: raw_s.clone(),
                                input_secondary: String::new(),
                                replace_same_col: false,
                                error_msg: None,
                                just_opened: true,
                            });
                            ui.close_menu();
                        }
                        if ui.button("筛选: 不包含...").clicked() {
                            deferred_open_filter_dlg = Some(FilterDialogState {
                                col_idx: orig_col,
                                op_kind: FilterDialogOpKind::StrNotContains,
                                input_primary: raw_s.clone(),
                                input_secondary: String::new(),
                                replace_same_col: false,
                                error_msg: None,
                                just_opened: true,
                            });
                            ui.close_menu();
                        }
                        if ui.button("筛选: 为空").clicked() {
                            deferred_quick_filter = Some((orig_col, FilterOp::StrIsEmpty, false));
                            ui.close_menu();
                        }
                        if ui.button("筛选: 非空").clicked() {
                            deferred_quick_filter = Some((orig_col, FilterOp::StrIsNotEmpty, false));
                            ui.close_menu();
                        }
                    }
                    ColumnData::Numeric(nc) => {
                        let tag = nc.missing_tag(orig_row);
                        if tag == MISSING_VALID {
                            let val = nc.values[orig_row];
                            if ui.button(format!("筛选: 等于 {}  (D)", preview)).clicked() {
                                deferred_quick_filter = Some((orig_col, FilterOp::NumEq(val), true));
                                ui.close_menu();
                            }
                            if ui.button(format!("筛选: 不等于 {}", preview)).clicked() {
                                deferred_quick_filter =
                                    Some((orig_col, FilterOp::NumNeq(val), false));
                                ui.close_menu();
                            }
                            if ui.button(format!("筛选: 大于 > {}", preview)).clicked() {
                                deferred_quick_filter =
                                    Some((orig_col, FilterOp::NumGt(val), false));
                                ui.close_menu();
                            }
                            if ui.button(format!("筛选: 大于等于 >= {}", preview)).clicked() {
                                deferred_quick_filter =
                                    Some((orig_col, FilterOp::NumGte(val), false));
                                ui.close_menu();
                            }
                            if ui.button(format!("筛选: 小于 < {}", preview)).clicked() {
                                deferred_quick_filter =
                                    Some((orig_col, FilterOp::NumLt(val), false));
                                ui.close_menu();
                            }
                            if ui.button(format!("筛选: 小于等于 <= {}", preview)).clicked() {
                                deferred_quick_filter =
                                    Some((orig_col, FilterOp::NumLte(val), false));
                                ui.close_menu();
                            }
                        } else if let Some(op) =
                            build_cell_equals_filter(&table, orig_row, orig_col)
                        {
                            if ui.button(format!("筛选: 等于 {}  (D)", preview)).clicked() {
                                deferred_quick_filter = Some((orig_col, op, true));
                                ui.close_menu();
                            }
                        }
                        if ui.button("筛选: 属于多个数值 (逗号分隔)...").clicked() {
                            deferred_open_filter_dlg = Some(FilterDialogState {
                                col_idx: orig_col,
                                op_kind: FilterDialogOpKind::NumIn,
                                input_primary: formatted.clone(),
                                input_secondary: String::new(),
                                replace_same_col: true,
                                error_msg: None,
                                just_opened: true,
                            });
                            ui.close_menu();
                        }
                        if ui.button("筛选: 缺失值 (./.A-.Z)").clicked() {
                            deferred_quick_filter = Some((orig_col, FilterOp::NumIsMissing, false));
                            ui.close_menu();
                        }
                        if ui.button("筛选: 非缺失值").clicked() {
                            deferred_quick_filter =
                                Some((orig_col, FilterOp::NumIsNotMissing, false));
                            ui.close_menu();
                        }
                        if ui.button("自定义数值/日期筛选...").clicked() {
                            deferred_open_filter_dlg = Some(FilterDialogState {
                                col_idx: orig_col,
                                op_kind: FilterDialogOpKind::NumEq,
                                input_primary: if tag == MISSING_VALID {
                                    formatted.clone()
                                } else {
                                    String::new()
                                },
                                input_secondary: String::new(),
                                replace_same_col: false,
                                error_msg: None,
                                just_opened: true,
                            });
                            ui.close_menu();
                        }
                    }
                }

                ui.separator();
                if ui.button("查看该变量统计量  (F)").clicked() {
                    actions.push(GridAction::ShowStatsFor(orig_col));
                    ui.close_menu();
                }
                if ui.button("复制单元格内容  (Ctrl+C)").clicked() {
                    actions.push(GridAction::CopyText(formatted));
                    ui.close_menu();
                }
                if !tab.query.is_empty() && ui.button("清除所有筛选与排序  (C)").clicked() {
                    deferred_clear_all = true;
                    ui.close_menu();
                }
            });
        });

    let scroll_offset = scroll_out.state.offset;
    tab.grid_scroll_offset = scroll_offset;

    // 3. Frozen Left Row Numbers
    let row_num_clip = Rect::from_min_max(
        Pos2::new(grid_rect.left(), grid_rect.top() + HEADER_TOTAL_HEIGHT),
        Pos2::new(grid_rect.left() + ROW_NUM_WIDTH, grid_rect.bottom()),
    );
    let rn_painter = ui.painter().with_clip_rect(row_num_clip);
    rn_painter.rect_filled(row_num_clip, 0.0, pal.bg_header_label);
    let first_rn = ((scroll_offset.y / ROW_HEIGHT).floor().max(0.0) as usize).min(total_rows);
    let end_rn = (((scroll_offset.y + row_num_clip.height()) / ROW_HEIGHT)
        .ceil()
        .max(0.0) as usize
        + 1)
    .min(total_rows);
    let rn_font = FontId::monospace(11.0);
    for vis_r in first_rn..end_rn {
        let y0 = row_num_clip.top() + (vis_r as f32) * ROW_HEIGHT - scroll_offset.y;
        let y1 = y0 + ROW_HEIGHT;
        let r_rect = Rect::from_min_max(
            Pos2::new(row_num_clip.left(), y0),
            Pos2::new(row_num_clip.right(), y1),
        );
        if tab.selected_cell.map(|(r, _)| r == vis_r).unwrap_or(false) {
            rn_painter.rect_filled(r_rect, 0.0, pal.bg_selected_col_hdr);
        }
        let num_text = format!("{}", vis_r + 1);
        rn_painter.text(
            Pos2::new(r_rect.right() - 8.0, r_rect.center().y),
            Align2::RIGHT_CENTER,
            &num_text,
            rn_font.clone(),
            pal.text_muted,
        );
        rn_painter.line_segment(
            [Pos2::new(r_rect.left(), y1), Pos2::new(r_rect.right(), y1)],
            Stroke::new(1.0, pal.border_grid),
        );
    }
    rn_painter.line_segment(
        [row_num_clip.right_top(), row_num_clip.right_bottom()],
        Stroke::new(1.0, pal.border_header),
    );

    // 4. Frozen Two-Tier Column Headers
    let hdr_clip = Rect::from_min_max(
        Pos2::new(grid_rect.left() + ROW_NUM_WIDTH, grid_rect.top()),
        Pos2::new(grid_rect.right(), grid_rect.top() + HEADER_TOTAL_HEIGHT),
    );
    let hdr_painter = ui.painter().with_clip_rect(hdr_clip);
    hdr_painter.rect_filled(hdr_clip, 0.0, pal.bg_header_label);

    let first_hdr_col = col_x_starts
        .partition_point(|&x| x <= scroll_offset.x)
        .saturating_sub(1);
    let end_hdr_col = col_x_starts
        .partition_point(|&x| x < scroll_offset.x + hdr_clip.width())
        .min(vis_cols.len());

    let font_var_name = FontId::proportional(12.2);
    let font_var_label = FontId::proportional(10.8);

    for c_vis in first_hdr_col..end_hdr_col {
        let orig_col = vis_cols[c_vis];
        let meta = &table.variables[orig_col];
        let x0 = hdr_clip.left() + col_x_starts[c_vis] - scroll_offset.x;
        let x1 = hdr_clip.left() + col_x_starts[c_vis + 1] - scroll_offset.x;
        let full_hdr_rect = Rect::from_min_max(
            Pos2::new(x0, hdr_clip.top()),
            Pos2::new(x1, hdr_clip.bottom()),
        );
        let name_rect = Rect::from_min_max(
            Pos2::new(x0, hdr_clip.top()),
            Pos2::new(x1, hdr_clip.top() + HDR_NAME_HEIGHT),
        );
        let label_rect = Rect::from_min_max(
            Pos2::new(x0, hdr_clip.top() + HDR_NAME_HEIGHT),
            Pos2::new(x1, hdr_clip.bottom()),
        );

        let is_sel_col = tab.active_target_col() == Some(orig_col);
        let name_bg = if is_sel_col {
            pal.bg_selected_col_hdr
        } else {
            pal.bg_header_name
        };
        hdr_painter.rect_filled(name_rect, 0.0, name_bg);
        hdr_painter.rect_filled(label_rect, 0.0, pal.bg_header_label);

        let sort_info = tab.query.sort_rank(orig_col);
        let has_filter = tab.query.has_filter_on(orig_col);
        let mut right_reserved = 6.0f32;

        if let Some((rank, asc)) = sort_info {
            let badge = if tab.query.sorts.len() > 1 {
                format!("{}{}", if asc { "▲" } else { "▼" }, rank)
            } else if asc {
                "▲".to_string()
            } else {
                "▼".to_string()
            };
            hdr_painter
                .with_clip_rect(name_rect.intersect(hdr_clip))
                .text(
                    Pos2::new(x1 - 6.0, name_rect.center().y),
                    Align2::RIGHT_CENTER,
                    &badge,
                    FontId::proportional(10.5),
                    pal.accent,
                );
            right_reserved += 22.0;
        }
        if has_filter {
            let cx = x1 - right_reserved - 4.0;
            hdr_painter
                .with_clip_rect(name_rect.intersect(hdr_clip))
                .circle_filled(Pos2::new(cx, name_rect.center().y), 3.2, pal.accent);
            right_reserved += 12.0;
        }

        let name_text_clip = Rect::from_min_max(
            Pos2::new(x0 + 5.0, name_rect.top()),
            Pos2::new((x1 - right_reserved).max(x0 + 6.0), name_rect.bottom()),
        )
        .intersect(hdr_clip);
        hdr_painter.with_clip_rect(name_text_clip).text(
            Pos2::new(x0 + 6.0, name_rect.center().y),
            Align2::LEFT_CENTER,
            &meta.name,
            font_var_name.clone(),
            pal.text_primary,
        );

        let label_text_clip = label_rect.shrink2(Vec2::new(5.0, 0.0)).intersect(hdr_clip);
        let lbl = if meta.label.is_empty() {
            "-"
        } else {
            meta.label.as_str()
        };
        hdr_painter.with_clip_rect(label_text_clip).text(
            Pos2::new(x0 + 6.0, label_rect.center().y),
            Align2::LEFT_CENTER,
            lbl,
            font_var_label.clone(),
            pal.text_muted,
        );

        hdr_painter.line_segment(
            [name_rect.left_bottom(), name_rect.right_bottom()],
            Stroke::new(1.0, pal.border_grid),
        );
        hdr_painter.line_segment(
            [full_hdr_rect.right_top(), full_hdr_rect.right_bottom()],
            Stroke::new(1.0, pal.border_header),
        );

        let resize_rect = Rect::from_min_max(
            Pos2::new(x1 - 4.0, hdr_clip.top()),
            Pos2::new(x1 + 3.0, hdr_clip.bottom()),
        )
        .intersect(hdr_clip);
        let resize_resp = ui.interact(
            resize_rect,
            Id::new(("col_resize", tab.id, orig_col)),
            Sense::drag(),
        );
        if resize_resp.hovered() || resize_resp.dragged() {
            ui.ctx().set_cursor_icon(egui::CursorIcon::ResizeHorizontal);
        }
        if resize_resp.dragged() {
            let cur_w = tab.column_widths.get(orig_col).copied().unwrap_or(110.0);
            let new_w = (cur_w + resize_resp.drag_delta().x).clamp(45.0, 800.0);
            deferred_resize = Some((orig_col, new_w));
        }

        let interact_rect = Rect::from_min_max(
            Pos2::new(x0 + 2.0, hdr_clip.top()),
            Pos2::new((x1 - 4.0).max(x0 + 3.0), hdr_clip.bottom()),
        )
        .intersect(hdr_clip);
        let hdr_resp = ui.interact(
            interact_rect,
            Id::new(("col_hdr", tab.id, orig_col)),
            Sense::click_and_drag(),
        );

        if hdr_resp.clicked() {
            tab.selected_var = Some(orig_col);
            let shift = ui.input(|i| i.modifiers.shift);
            deferred_sort = Some((orig_col, shift));
        }

        if hdr_resp.drag_started() {
            tab.dragging_col = Some(orig_col);
        }

        if let Some(src_col) = tab.dragging_col {
            if src_col != orig_col && hdr_resp.hovered() {
                hdr_painter.line_segment(
                    [Pos2::new(x0, hdr_clip.top()), Pos2::new(x0, hdr_clip.bottom())],
                    Stroke::new(3.0, pal.accent),
                );
                if ui.input(|i| i.pointer.any_released()) {
                    deferred_reorder = Some((src_col, orig_col));
                    tab.dragging_col = None;
                }
            }
        }

        let fmt_display = if meta.format.is_empty() {
            "-"
        } else {
            &meta.format
        };
        hdr_resp.clone().on_hover_text(format!(
            "变量名: {}\n标签: {}\n类型: {} | Format: {} | 存储长度: {}\n点击排序 | Shift+点击叠加多列排序 | 拖动列头调整顺序 | 右键更多操作",
            meta.name,
            if meta.label.is_empty() { "(无)" } else { &meta.label },
            meta.var_type.short_label(),
            fmt_display,
            meta.storage_width
        ));

        hdr_resp.context_menu(|ui| {
            ui.label(RichText::new(format!("列: {}", meta.name)).strong());
            ui.separator();
            if ui.button("升序排序 (单列)").clicked() {
                deferred_set_sort = Some((orig_col, true, false));
                ui.close_menu();
            }
            if ui.button("降序排序 (单列)").clicked() {
                deferred_set_sort = Some((orig_col, false, false));
                ui.close_menu();
            }
            if ui.button("追加升序排序 (多列叠加)").clicked() {
                deferred_set_sort = Some((orig_col, true, true));
                ui.close_menu();
            }
            if ui.button("追加降序排序 (多列叠加)").clicked() {
                deferred_set_sort = Some((orig_col, false, true));
                ui.close_menu();
            }
            if tab.query.sort_rank(orig_col).is_some() && ui.button("取消此列排序").clicked() {
                deferred_remove_sort = Some(orig_col);
                ui.close_menu();
            }
            ui.separator();
            if ui.button("筛选此列...").clicked() {
                let op_kind = match meta.var_type {
                    VarType::String => FilterDialogOpKind::StrIn,
                    VarType::Numeric => FilterDialogOpKind::NumEq,
                };
                deferred_open_filter_dlg = Some(FilterDialogState {
                    col_idx: orig_col,
                    op_kind,
                    input_primary: String::new(),
                    input_secondary: String::new(),
                    replace_same_col: false,
                    error_msg: None,
                    just_opened: true,
                });
                ui.close_menu();
            }
            if ui.button("查看统计量  (F)").clicked() {
                tab.selected_var = Some(orig_col);
                actions.push(GridAction::ShowStatsFor(orig_col));
                ui.close_menu();
            }
            ui.separator();
            if ui.button("自适应当前列宽 (6~30字符)").clicked() {
                deferred_autofit_col = Some(orig_col);
                ui.close_menu();
            }
            if ui.button("批量自适应所有可见列宽").clicked() {
                deferred_autofit_all = true;
                ui.close_menu();
            }
            if ui.button("重置所有列宽").clicked() {
                deferred_reset_widths = true;
                ui.close_menu();
            }
            ui.separator();
            if ui.button("向左移动一列  (Alt+←)").clicked() {
                deferred_move_col = Some((orig_col, -1));
                ui.close_menu();
            }
            if ui.button("向右移动一列  (Alt+→)").clicked() {
                deferred_move_col = Some((orig_col, 1));
                ui.close_menu();
            }
            if ui.button("隐藏此变量").clicked() {
                deferred_hide_col = Some(orig_col);
                ui.close_menu();
            }
        });
    }

    hdr_painter.line_segment(
        [hdr_clip.left_bottom(), hdr_clip.right_bottom()],
        Stroke::new(1.2, pal.border_header),
    );

    if let Some(src_col) = tab.dragging_col {
        if ui.input(|i| i.pointer.any_released()) {
            tab.dragging_col = None;
        } else if let Some(pos) = ui.ctx().pointer_latest_pos() {
            if let Some(meta) = table.variables.get(src_col) {
                egui::Area::new(Id::new(("col_drag_badge", tab.id)))
                    .order(egui::Order::Tooltip)
                    .fixed_pos(pos + Vec2::new(10.0, 10.0))
                    .interactable(false)
                    .show(ui.ctx(), |ui| {
                        egui::Frame::popup(ui.style()).show(ui, |ui| {
                            ui.label(
                                RichText::new(format!("⇄ 移动列: {}", meta.name))
                                    .strong()
                                    .size(11.5),
                            );
                        });
                    });
            }
        }
    }

    if deferred_clear_all {
        tab.clear_all_filters_and_sorts();
    }
    if let Some((col, append)) = deferred_sort {
        tab.query.toggle_sort(col, append);
        tab.recompute_view();
    }
    if let Some((col, asc, append)) = deferred_set_sort {
        tab.query.set_sort_direction(col, asc, append);
        tab.recompute_view();
    }
    if let Some(col) = deferred_remove_sort {
        tab.query.remove_sort(col);
        tab.recompute_view();
    }
    if let Some((src, dst)) = deferred_reorder {
        tab.reorder_column(src, dst);
    }
    if let Some((col, new_w)) = deferred_resize {
        if col < tab.column_widths.len() {
            tab.column_widths[col] = new_w;
        }
    }
    if let Some(col) = deferred_autofit_col {
        tab.auto_fit_column(col);
    }
    if deferred_autofit_all {
        tab.auto_fit_all_visible_columns();
    }
    if deferred_reset_widths {
        tab.reset_all_column_widths();
    }
    if let Some(col) = deferred_hide_col {
        if col < tab.column_visible.len() {
            tab.column_visible[col] = false;
        }
    }
    if let Some((col, delta)) = deferred_move_col {
        tab.move_column_by(col, delta);
    }
    if let Some((col, op, replace_same)) = deferred_quick_filter {
        if replace_same {
            tab.query.set_quick_filter_for_column(col, op);
        } else {
            tab.query.add_filter(col, op);
        }
        tab.recompute_view();
    }
    if let Some(dlg) = deferred_open_filter_dlg {
        tab.filter_dialog = Some(dlg);
    }

    show_filter_dialog(ui.ctx(), tab);

    actions
}

fn show_filter_dialog(ctx: &egui::Context, tab: &mut DatasetTabState) {
    if tab.filter_dialog.is_none() {
        return;
    }
    let Some(table) = tab.table.clone() else {
        tab.filter_dialog = None;
        return;
    };

    let col_idx = tab.filter_dialog.as_ref().unwrap().col_idx;
    let Some(meta) = table.variables.get(col_idx) else {
        tab.filter_dialog = None;
        return;
    };

    let mut open = true;
    let mut should_apply = false;
    let mut should_cancel = false;

    egui::Window::new(format!("筛选变量: {}", meta.name))
        .id(Id::new(("filter_dialog", tab.id)))
        .collapsible(false)
        .resizable(false)
        .anchor(egui::Align2::CENTER_CENTER, Vec2::ZERO)
        .open(&mut open)
        .show(ctx, |ui| {
            let Some(dlg) = &mut tab.filter_dialog else {
                return;
            };
            if !meta.label.is_empty() {
                ui.label(RichText::new(&meta.label).size(11.5).weak());
            }
            ui.add_space(4.0);

            let ops: &[FilterDialogOpKind] = match meta.var_type {
                VarType::String => &[
                    FilterDialogOpKind::StrEq,
                    FilterDialogOpKind::StrNeq,
                    FilterDialogOpKind::StrIn,
                    FilterDialogOpKind::StrNotIn,
                    FilterDialogOpKind::StrContains,
                    FilterDialogOpKind::StrNotContains,
                ],
                VarType::Numeric => &[
                    FilterDialogOpKind::NumEq,
                    FilterDialogOpKind::NumNeq,
                    FilterDialogOpKind::NumIn,
                    FilterDialogOpKind::NumNotIn,
                    FilterDialogOpKind::NumGt,
                    FilterDialogOpKind::NumGte,
                    FilterDialogOpKind::NumLt,
                    FilterDialogOpKind::NumLte,
                    FilterDialogOpKind::NumBetween,
                ],
            };

            ui.horizontal(|ui| {
                ui.label("条件:");
                egui::ComboBox::from_id_salt(("filter_op_combo", tab.id))
                    .selected_text(dlg.op_kind.label())
                    .show_ui(ui, |ui| {
                        for &op in ops {
                            ui.selectable_value(&mut dlg.op_kind, op, op.label());
                        }
                    });
            });

            let hint = match dlg.op_kind {
                FilterDialogOpKind::StrIn | FilterDialogOpKind::StrNotIn => {
                    "多个值用逗号分隔，如: Asian, White"
                }
                FilterDialogOpKind::NumIn | FilterDialogOpKind::NumNotIn => {
                    "多个数值用逗号分隔，如: 20, 35, 50"
                }
                _ if meta.date_kind.is_temporal() => {
                    "输入数值或日期 (如 2025-01-01 / 01JAN2025)"
                }
                _ if meta.var_type == VarType::Numeric => "输入数值",
                _ => "输入匹配文本",
            };

            ui.horizontal(|ui| {
                ui.label(if dlg.op_kind == FilterDialogOpKind::NumBetween {
                    "起始:"
                } else {
                    "值:  "
                });
                let resp = ui.add(
                    egui::TextEdit::singleline(&mut dlg.input_primary)
                        .hint_text(hint)
                        .desired_width(260.0),
                );
                if dlg.just_opened {
                    resp.request_focus();
                    dlg.just_opened = false;
                }
            });

            if dlg.op_kind == FilterDialogOpKind::NumBetween {
                ui.horizontal(|ui| {
                    ui.label("结束:");
                    ui.add(
                        egui::TextEdit::singleline(&mut dlg.input_secondary)
                            .hint_text(hint)
                            .desired_width(260.0),
                    );
                });
            }

            if let Some(err) = &dlg.error_msg {
                ui.label(
                    RichText::new(err)
                        .color(Color32::from_rgb(210, 60, 60))
                        .size(11.5),
                );
            }

            ui.add_space(6.0);
            ui.horizontal(|ui| {
                if ui.button("应用筛选 (Enter)").clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Enter))
                {
                    should_apply = true;
                }
                if ui.button("取消 (Esc)").clicked()
                    || ui.input(|i| i.key_pressed(egui::Key::Escape))
                {
                    should_cancel = true;
                }
            });
        });

    if should_apply {
        tab.apply_filter_dialog();
    } else if should_cancel || !open {
        tab.filter_dialog = None;
    }
}

fn truncate_preview(s: &str, max_chars: usize) -> String {
    let mut chars = s.chars();
    let head: String = chars.by_ref().take(max_chars).collect();
    if chars.next().is_some() {
        format!("{}...", head)
    } else {
        head
    }
}
