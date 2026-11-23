use crate::state::{DatasetTabState, RightTab};
use crate::theme::AppPalette;
use egui::{Color32, RichText, ScrollArea, Sense, Ui};
use sas_engine::{format_numeric_value, format_plain_number, ColumnStats, FilterOp, VarType};

/// Render the Right Sidebar containing 3 tabs: Variables / Statistics / Shortcuts.
pub fn show_right_panel(
    ui: &mut Ui,
    active_tab: Option<&mut DatasetTabState>,
    current_right_tab: &mut RightTab,
    focus_var_search: &mut bool,
    pal: &AppPalette,
) {
    // Top tab selector bar
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
            RichText::new("在左侧文件管理器双击 .sas7bdat 或 .xpt 即可查看变量与统计信息")
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

    // Search box (matches both Variable Name AND Variable Label)
    ui.horizontal(|ui| {
        let resp = ui.add(
            egui::TextEdit::singleline(&mut tab.var_search)
                .hint_text("搜索变量名或标签... (Ctrl+F)")
                .desired_width(ui.available_width() - 24.0),
        );
        if *focus_var_search {
            resp.request_focus();
            *focus_var_search = false;
        }
        if !tab.var_search.is_empty() && ui.small_button("×").clicked() {
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

    // Quick selection toolbar
    ui.horizontal(|ui| {
        if ui
            .small_button("全选 (U)")
            .on_hover_text("显示全部变量")
            .clicked()
        {
            tab.show_all_variables();
        }
        if ui
            .small_button("清空 (H)")
            .on_hover_text("隐藏全部变量")
            .clicked()
        {
            tab.hide_all_variables();
        }
        if !search_lower.is_empty()
            && ui
                .small_button("仅选匹配")
                .on_hover_text("仅勾选当前搜索匹配到的变量")
                .clicked()
        {
            for v in &mut tab.column_visible {
                *v = false;
            }
            for &c in &matched_cols {
                if c < tab.column_visible.len() {
                    tab.column_visible[c] = true;
                }
            }
        }
        ui.with_layout(egui::Layout::right_to_left(egui::Align::Center), |ui| {
            ui.label(
                RichText::new(format!("{}/{}", checked_count, total_vars))
                    .size(11.5)
                    .color(pal.text_muted),
            );
        });
    });
    ui.separator();

    // Virtualized variable list (handles datasets with 1,000+ variables effortlessly!)
    let row_h = 38.0;
    let mut jump_to_col: Option<usize> = None;
    let mut stats_for_col: Option<usize> = None;

    ScrollArea::vertical()
        .id_salt(("var_list_scroll", tab.id))
        .auto_shrink([false, false])
        .show_rows(ui, row_h, matched_cols.len(), |ui, row_range| {
            for list_idx in row_range {
                let col_idx = matched_cols[list_idx];
                let Some(meta) = table.variables.get(col_idx) else {
                    continue;
                };
                let is_selected = tab.selected_var == Some(col_idx);

                let frame_fill = if is_selected {
                    pal.accent_soft
                } else if list_idx % 2 == 0 {
                    Color32::TRANSPARENT
                } else {
                    pal.bg_row_odd
                };

                let info_rect = egui::Frame::NONE
                    .fill(frame_fill)
                    .inner_margin(egui::Margin::symmetric(4, 3))
                    .show(ui, |ui| {
                        ui.set_min_height(row_h - 6.0);
                        ui.horizontal(|ui| {
                            if col_idx < tab.column_visible.len() {
                                ui.checkbox(&mut tab.column_visible[col_idx], "");
                            }

                            let info = ui.vertical(|ui| {
                                ui.set_min_width(ui.available_width());
                                ui.spacing_mut().item_spacing.y = 1.0;
                                // Line 1: Var Name + Type badge + Format
                                ui.horizontal(|ui| {
                                    ui.label(
                                        RichText::new(&meta.name)
                                            .strong()
                                            .size(12.2)
                                            .color(if is_selected {
                                                pal.accent
                                            } else {
                                                pal.text_primary
                                            }),
                                    );
                                    let (type_str, type_col) = match meta.var_type {
                                        VarType::Numeric => ("Num", pal.badge_num),
                                        VarType::String => ("Char", pal.badge_char),
                                    };
                                    ui.label(
                                        RichText::new(type_str)
                                            .size(10.0)
                                            .strong()
                                            .color(type_col),
                                    );
                                    if !meta.format.is_empty() {
                                        ui.label(
                                            RichText::new(&meta.format)
                                                .monospace()
                                                .size(10.2)
                                                .color(pal.text_muted),
                                        );
                                    }
                                });
                                // Line 2: Variable Label
                                let lbl = if meta.label.is_empty() {
                                    "(无标签)"
                                } else {
                                    &meta.label
                                };
                                ui.add(
                                    egui::Label::new(
                                        RichText::new(lbl).size(11.0).color(pal.text_muted),
                                    )
                                    .truncate(),
                                );
                            });
                            info.response.rect
                        })
                        .inner
                    })
                    .inner;

                let click_resp = ui.interact(
                    info_rect,
                    egui::Id::new(("var_row_click", tab.id, col_idx)),
                    Sense::click(),
                );
                if click_resp.clicked() {
                    tab.selected_var = Some(col_idx);
                    // Also clear row-specific cell selection if clicking a different var so `D` opens filter for this var
                    if let Some((_, sel_c)) = tab.selected_cell {
                        if sel_c != col_idx {
                            tab.selected_cell = None;
                        }
                    }
                }
                if click_resp.double_clicked() {
                    tab.selected_var = Some(col_idx);
                    if col_idx < tab.column_visible.len() {
                        tab.column_visible[col_idx] = true;
                    }
                    jump_to_col = Some(col_idx);
                }
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

    // Ensure stats are computed if a variable is targeted
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
                RichText::new("选中数据单元格或变量栏中的变量后\n按快捷键 F 即可立即显示统计量")
                    .color(pal.text_muted)
                    .size(11.5),
            );
        });
        return;
    };

    // Ensure cache is fresh with current filter_version
    if tab.stats_cache_key != Some((tab.filter_version, col_idx)) {
        tab.refresh_stats_for(col_idx);
    }

    let Some(meta) = table.variables.get(col_idx) else {
        return;
    };

    // Variable summary header
    egui::Frame::NONE
        .fill(pal.bg_header_name)
        .inner_margin(egui::Margin::same(8))
        .corner_radius(egui::CornerRadius::same(4))
        .show(ui, |ui| {
            ui.horizontal(|ui| {
                ui.label(RichText::new(&meta.name).strong().size(14.0).color(pal.accent));
                let (t_str, t_col) = match meta.var_type {
                    VarType::Numeric => ("Num", pal.badge_num),
                    VarType::String => ("Char", pal.badge_char),
                };
                ui.label(RichText::new(t_str).strong().size(11.0).color(t_col));
                if !meta.format.is_empty() {
                    ui.label(RichText::new(&meta.format).monospace().size(11.0).color(pal.text_muted));
                }
            });
            if !meta.label.is_empty() {
                ui.label(RichText::new(&meta.label).size(11.8).color(pal.text_primary));
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
            ui.label(RichText::new(scope_text).size(11.0).color(pal.text_muted));
        });

    ui.add_space(6.0);

    let Some(stats) = tab.cached_stats.clone() else {
        return;
    };

    let mut quick_filter_op: Option<FilterOp> = None;

    ScrollArea::vertical()
        .id_salt(("stats_scroll", tab.id))
        .auto_shrink([false, false])
        .show(ui, |ui| match &stats {
            ColumnStats::String(ss) => {
                egui::Grid::new(("str_summary_grid", tab.id))
                    .num_columns(2)
                    .spacing([14.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        stat_kv_row(ui, "总观测数 (Total)", &ss.total_rows.to_string(), pal);
                        stat_kv_row(ui, "非空数 (Valid)", &ss.valid_count.to_string(), pal);
                        stat_kv_row(ui, "空值数 (Blank)", &ss.missing_count.to_string(), pal);
                        stat_kv_row(ui, "唯一值数 (Distinct)", &ss.distinct_count.to_string(), pal);
                    });

                ui.add_space(8.0);
                ui.horizontal(|ui| {
                    ui.label(RichText::new("频数分布 (双击行快速筛选)").strong().size(12.2));
                    if ss.truncated {
                        ui.label(RichText::new("(前 500 项)").size(10.8).color(pal.text_muted));
                    }
                });
                ui.separator();

                render_freq_table(ui, &ss.frequencies, false, pal, &mut quick_filter_op);
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
                        Some(v) if meta.date_kind.is_temporal() => {
                            format!("{} ({})", format_numeric_value(v, meta), format_plain_number(v, None))
                        }
                        Some(v) => format_plain_number(v, None),
                        None => "-".to_string(),
                    }
                };

                egui::Grid::new(("num_summary_grid", tab.id))
                    .num_columns(2)
                    .spacing([14.0, 4.0])
                    .striped(true)
                    .show(ui, |ui| {
                        stat_kv_row(ui, "总观测数 (Total)", &ns.total_rows.to_string(), pal);
                        stat_kv_row(ui, "有效数 (N)", &ns.valid_count.to_string(), pal);
                        stat_kv_row(ui, "缺失数 (NMiss)", &ns.missing_count.to_string(), pal);
                        stat_kv_row(ui, "唯一值数 (Distinct)", &ns.distinct_count.to_string(), pal);
                        stat_kv_row(ui, "均值 (Mean)", &fmt_val(ns.mean), pal);
                        stat_kv_row(ui, "标准差 (SD)", &fmt_val(ns.std_dev), pal);
                        stat_kv_row(ui, "最小值 (Min)", &fmt_maybe_date(ns.min), pal);
                        stat_kv_row(ui, "25% 分位数 (Q1)", &fmt_maybe_date(ns.q1), pal);
                        stat_kv_row(ui, "中位数 (Median)", &fmt_maybe_date(ns.median), pal);
                        stat_kv_row(ui, "75% 分位数 (Q3)", &fmt_maybe_date(ns.q3), pal);
                        stat_kv_row(ui, "最大值 (Max)", &fmt_maybe_date(ns.max), pal);
                        stat_kv_row(ui, "总和 (Sum)", &fmt_val(ns.sum), pal);
                    });

                if !ns.tagged_missing_counts.is_empty() {
                    ui.add_space(8.0);
                    ui.label(RichText::new("缺失值明细").strong().size(12.0));
                    egui::Grid::new(("num_miss_grid", tab.id))
                        .num_columns(2)
                        .spacing([14.0, 3.0])
                        .show(ui, |ui| {
                            for (tag_str, cnt) in &ns.tagged_missing_counts {
                                let pct = if ns.total_rows > 0 {
                                    (*cnt as f64) * 100.0 / (ns.total_rows as f64)
                                } else {
                                    0.0
                                };
                                stat_kv_row(ui, tag_str, &format!("{} ({:.2}%)", cnt, pct), pal);
                            }
                        });
                }

                if let Some(freqs) = &ns.low_card_freqs {
                    ui.add_space(10.0);
                    ui.label(RichText::new("取值频数分布 (双击行快速筛选)").strong().size(12.2));
                    ui.separator();
                    render_freq_table(ui, freqs, true, pal, &mut quick_filter_op);
                }
            }
        });

    if let Some(op) = quick_filter_op {
        tab.query.set_quick_filter_for_column(col_idx, op);
        tab.recompute_view();
    }
}

fn stat_kv_row(ui: &mut Ui, key: &str, val: &str, pal: &AppPalette) {
    ui.label(RichText::new(key).size(12.0).color(pal.text_muted));
    ui.label(RichText::new(val).monospace().strong().size(12.0).color(pal.text_primary));
    ui.end_row();
}

fn render_freq_table(
    ui: &mut Ui,
    entries: &[sas_engine::FreqEntry],
    is_numeric: bool,
    pal: &AppPalette,
    quick_filter_op: &mut Option<FilterOp>,
) {
    let avail_w = ui.available_width();
    for (idx, entry) in entries.iter().enumerate() {
        let row_h = 22.0;
        let (rect, resp) = ui.allocate_exact_size(egui::vec2(avail_w, row_h), Sense::click());

        // Subtle zebra & percentage bar
        if idx % 2 == 1 {
            ui.painter().rect_filled(rect, 0.0, pal.bg_row_odd);
        }
        if resp.hovered() {
            ui.painter().rect_filled(rect, 0.0, pal.bg_row_hover);
        }

        let bar_w = (avail_w * ((entry.percent as f32) / 100.0)).clamp(1.0, avail_w);
        let bar_rect = egui::Rect::from_min_size(
            egui::pos2(rect.left(), rect.bottom() - 3.0),
            egui::vec2(bar_w, 2.5),
        );
        ui.painter().rect_filled(bar_rect, 1.0, pal.bar_fill);

        // Right-aligned count & percent
        let right_text = format!("{}  ({:.1}%)", entry.count, entry.percent);
        let right_w = 104.0;
        let val_clip = egui::Rect::from_min_max(
            rect.min,
            egui::pos2((rect.right() - right_w).max(rect.left() + 30.0), rect.bottom()),
        );
        ui.painter().with_clip_rect(val_clip).text(
            egui::pos2(rect.left() + 4.0, rect.center().y - 1.0),
            egui::Align2::LEFT_CENTER,
            &entry.value,
            egui::FontId::proportional(11.8),
            if entry.is_missing {
                pal.text_missing
            } else {
                pal.text_primary
            },
        );
        ui.painter().text(
            egui::pos2(rect.right() - 4.0, rect.center().y - 1.0),
            egui::Align2::RIGHT_CENTER,
            &right_text,
            egui::FontId::monospace(11.0),
            pal.text_muted,
        );

        resp.clone().on_hover_text(format!(
            "值: {}\n频数: {}\n占比: {:.2}%\n双击立即按此值筛选",
            entry.value, entry.count, entry.percent
        ));

        if resp.double_clicked() {
            if is_numeric {
                if entry.is_missing {
                    *quick_filter_op = Some(FilterOp::NumEqTaggedMissing(entry.missing_tag));
                } else if let Some(v) = entry.raw_numeric {
                    *quick_filter_op = Some(FilterOp::NumEq(v));
                }
            } else if entry.is_missing {
                *quick_filter_op = Some(FilterOp::StrIsEmpty);
            } else {
                *quick_filter_op = Some(FilterOp::StrEq(entry.value.clone()));
            }
        }
    }
}

fn show_shortcuts_tab(ui: &mut Ui, pal: &AppPalette) {
    ScrollArea::vertical()
        .auto_shrink([false, false])
        .show(ui, |ui| {
            shortcut_section(
                ui,
                "核心联动快捷键",
                &[
                    ("F", "在统计量栏显示当前选中单元格/变量的统计信息"),
                    ("D", "对选中单元格执行 = 快速筛选（不同列叠加，同列替换）"),
                    ("C", "清除当前数据集的所有筛选与排序条件"),
                    ("H", "清空右侧变量栏勾选（隐藏所有变量）"),
                    ("U", "全选右侧变量栏（恢复显示所有变量）"),
                ],
                pal,
            );

            ui.add_space(10.0);
            shortcut_section(
                ui,
                "列与排序操作",
                &[
                    ("Alt + ← / →", "将当前选中的变量列向左 / 向右移动位置"),
                    ("拖动列头", "自由拖拽调整变量列顺序"),
                    ("点击列头", "对该变量进行升序/降序/取消排序"),
                    ("Shift + 点击列头", "叠加多个变量进行排序"),
                    ("右键列头", "自适应列宽 / 批量自适应所有可见列宽 / 排序"),
                    ("右键单元格", "对字符串(=/!=/包含)或数值(=/!=/>/<等)快速筛选"),
                ],
                pal,
            );

            ui.add_space(10.0);
            shortcut_section(
                ui,
                "导航与界面控制",
                &[
                    ("方向键", "在数据单元格间快速移动焦点"),
                    ("PgUp / PgDn", "数据表格快速上下翻页"),
                    ("Ctrl/Cmd + C", "复制当前选中单元格内容（不与 C 键冲突）"),
                    ("Ctrl/Cmd + F", "聚焦右侧变量栏搜索框"),
                    ("Ctrl/Cmd + B", "收起 / 展开左侧文件管理器"),
                    ("Ctrl/Cmd + \\", "收起 / 展开最右侧信息栏"),
                    ("拖拽标签页", "将多个数据集上下 / 左右并排分屏展示"),
                ],
                pal,
            );

            ui.add_space(10.0);
            ui.label(
                RichText::new("提示：当光标位于搜索框、筛选输入框或重命名输入框中时，单字母快捷键（F/D/C/H/U）会自动暂停以防误触。")
                    .size(11.0)
                    .color(pal.text_muted),
            );
        });
}

fn shortcut_section(ui: &mut Ui, title: &str, items: &[(&str, &str)], pal: &AppPalette) {
    ui.label(RichText::new(title).strong().size(12.8).color(pal.accent));
    ui.add_space(3.0);
    egui::Grid::new(("sc_grid", title))
        .num_columns(2)
        .spacing([10.0, 6.0])
        .striped(true)
        .show(ui, |ui| {
            for (key, desc) in items {
                egui::Frame::NONE
                    .fill(pal.bg_header_name)
                    .corner_radius(egui::CornerRadius::same(4))
                    .inner_margin(egui::Margin::symmetric(6, 2))
                    .show(ui, |ui| {
                        ui.label(
                            RichText::new(*key)
                                .monospace()
                                .strong()
                                .size(11.2)
                                .color(pal.text_primary),
                        );
                    });
                ui.label(RichText::new(*desc).size(11.8).color(pal.text_primary));
                ui.end_row();
            }
        });
}
