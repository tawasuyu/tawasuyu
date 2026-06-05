use super::*;

pub(crate) fn header_bar(model: &Model, theme: &Theme) -> View<Msg> {
    // Section 1: brand pill (nada con bg accent).
    let brand = View::new(Style {
        size: Size { width: length(108.0_f32), height: length(22.0_f32) },
        padding: Rect {
            left: length(10.0_f32), right: length(10.0_f32),
            top: length(0.0_f32), bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.accent)
    .radius(4.0)
    .text_aligned("nada".to_string(), 11.0, theme.bg_app, Alignment::Center);

    // Section 2: breadcrumb root - active-file (ocupa el centro).
    let crumb_text = match model.active_tab() {
        Some(tab) => {
            let rel = relative_to(&model.root, &tab.path);
            let dirty = if tab.dirty { "  ●" } else { "" };
            format!("{}  ›  {}{}", model.root.display(), rel, dirty)
        }
        None => format!("{}", model.root.display()),
    };
    let breadcrumb = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(12.0_f32), right: length(12.0_f32),
            top: length(0.0_f32), bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(crumb_text, 11.5, theme.fg_text, Alignment::Start);

    // Section 3: hint con shortcuts mas usados.
    let hint = View::new(Style {
        size: Size { width: length(360.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(0.0_f32), right: length(12.0_f32),
            top: length(0.0_f32), bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(
        rimay_localize::t("edit-header-hint"),
        10.5, theme.fg_muted, Alignment::End,
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(HEADER_H) },
        padding: Rect {
            left: length(8.0_f32), right: length(8.0_f32),
            top: length(6.0_f32), bottom: length(6.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![brand, breadcrumb, hint])
}

/// Linea fina accent-tinted que separa header del body, body del status.
pub(crate) fn separator_line(theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(SEP_H) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.border)
}

/// Status bar al pie estilo VS Code: tres secciones (status mensaje a la
/// izquierda, cursor + lang al centro, lsp + bookmarks + tabs a la derecha).
pub(crate) fn status_bar(model: &Model, theme: &Theme) -> View<Msg> {
    // --- left: status text ---
    let status_text = if model.status.is_empty() {
        "✓ ready".to_string()
    } else {
        model.status.clone()
    };
    let left = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(10.0_f32), right: length(8.0_f32),
            top: length(0.0_f32), bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(status_text, 10.5, theme.fg_text, Alignment::Start);

    // --- center: cursor pos + lang ---
    let center_text = match model.active_tab() {
        Some(tab) => {
            let lang = lang_label(&tab.path);
            let line = tab.editor.cursor.caret.line + 1;
            let col = tab.editor.cursor.caret.col + 1;
            rimay_localize::t_args(
                "edit-status-position",
                &[
                    ("line", line.to_string().into()),
                    ("col", col.to_string().into()),
                    ("lang", lang.into()),
                ],
            )
        }
        None => "".to_string(),
    };
    let center = View::new(Style {
        size: Size { width: length(220.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(0.0_f32), right: length(0.0_f32),
            top: length(0.0_f32), bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(center_text, 10.5, theme.fg_muted, Alignment::Center);

    // --- right: lsp + bookmarks + tabs ---
    let lsp_label = model.lsp_label.clone();
    let bm = model.bookmarks.marks.len();
    let bm_label = if bm > 0 { format!("★ {bm}") } else { "".to_string() };
    let tabs_label = if model.tabs.is_empty() {
        "".to_string()
    } else if model.tabs.len() == 1 {
        "1 tab".to_string()
    } else {
        format!("{} tabs", model.tabs.len())
    };
    let git_label = git_summary(&model.git_status);
    let right_text = [tabs_label, bm_label, git_label, lsp_label]
        .into_iter()
        .filter(|s| !s.is_empty())
        .collect::<Vec<_>>()
        .join("  ·  ");
    let right = View::new(Style {
        size: Size { width: length(360.0_f32), height: percent(1.0_f32) },
        padding: Rect {
            left: length(0.0_f32), right: length(10.0_f32),
            top: length(0.0_f32), bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text_aligned(right_text, 10.5, theme.fg_muted, Alignment::End);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(STATUS_H) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![left, center, right])
}

/// Etiqueta corta del lenguaje a partir del path. Convive con
/// language_for_path pero no devuelve el enum del editor — solo
/// texto humano para la status bar.
pub(crate) fn lang_label(path: &Path) -> &'static str {
    match path.extension().and_then(|s| s.to_str()) {
        Some("rs") => "Rust",
        Some("py") => "Python",
        Some("js") | Some("mjs") => "JS",
        Some("ts") => "TS",
        Some("tsx") => "TSX",
        Some("go") => "Go",
        Some("toml") => "TOML",
        Some("md") => "Markdown",
        Some("json") => "JSON",
        Some("yaml") | Some("yml") => "YAML",
        Some("sh") => "Shell",
        Some("html") => "HTML",
        Some("css") => "CSS",
        _ => "Text",
    }
}

pub(crate) fn body_view(model: &Model, theme: &Theme) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(vec![tree_panel(model, theme), right_panel(model, theme)])
}

/// Columna derecha: editor arriba; si el terminal está abierto, va
/// como panel inferior fijo de 220px (estilo VS Code).
pub(crate) fn right_panel(model: &Model, theme: &Theme) -> View<Msg> {
    let editor = editor_panel(model, theme);
    // Si el minimap esta abierto, el editor y el minimap conviven en
    // un Row para que el minimap quede pegado a la derecha (estilo VS Code).
    let editor_row: View<Msg> = match model.minimap.as_ref() {
        Some(mm_state) => {
            let (lines, vp_start, vp_end, caret_line) = minimap_snapshot_data(model);
            let snap = MiniMapSnapshot {
                lines: &lines,
                viewport_start: vp_start,
                viewport_end: vp_end,
                caret_line,
            };
            let palette = MiniMapPalette::from_theme(theme);
            let mm_view = minimap::view(mm_state, &snap, &palette, Msg::MiniMap);
            View::new(Style {
                flex_direction: FlexDirection::Row,
                flex_grow: 1.0,
                size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
                ..Default::default()
            })
            .children(vec![editor, mm_view])
        }
        None => editor,
    };
    let mut children = vec![editor_row];
    if let Some(state) = model.fif.as_ref() {
        let palette = FifPalette::from_theme(theme);
        children.push(fif::view_results_bar(
            state,
            &model.all_files,
            &model.root,
            &palette,
            Msg::Fif,
        ));
    }
    if let Some(state) = model.term.as_ref() {
        children.push(term::view(
            state,
            &ShumaTermPalette::from_theme(theme),
            TERM_PANEL_H,
            Msg::Term,
        ));
    }
    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .children(children)
}

pub(crate) fn tree_panel(model: &Model, theme: &Theme) -> View<Msg> {
    let rows: Vec<TreeRow<Msg>> = model
        .nodes
        .iter()
        .enumerate()
        .map(|(i, n)| TreeRow {
            label: row_label_with_git(n, &model.git_status),
            depth: n.depth,
            has_children: n.is_dir,
            expanded: n.expanded,
            selected: model.selected == Some(i),
            on_toggle: Msg::ToggleNode(i),
            on_select: Msg::SelectNode(i),
            icon: None,
            on_context: None,
            editor: None,
        })
        .collect();

    let spec = TreeSpec {
        rows,
        row_height: TREE_ROW_H,
        indent_px: TREE_INDENT,
        palette: TreePalette::from_theme(theme),
        guides: false,
    };

    // El árbol scrollea: viewport clipeado de alto = panel, contenido =
    // una fila por nodo. Rueda (cursor encima) + barra arrastrable, sin
    // tocar el on_wheel global (que sigue scrolleando el editor).
    let scroller = llimphi_widget_scroll::scroll_y(
        model.tree_scroll,
        model.tree_content_h(),
        model.tree_viewport_h(),
        tree_view(spec),
        Msg::TreeScroll,
        &llimphi_widget_scroll::ScrollPalette::from_theme(theme),
    );

    View::new(Style {
        size: Size { width: length(TREE_WIDTH), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![scroller])
}

pub(crate) fn editor_panel(model: &Model, theme: &Theme) -> View<Msg> {
    let inner = active_editor_content(model, theme);
    if model.tabs.is_empty() {
        // Sin tabs todavía: solo placeholder, sin tab strip.
        return View::new(Style {
            flex_direction: FlexDirection::Column,
            flex_grow: 1.0,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![inner]);
    }
    let labels: Vec<String> = model
        .tabs
        .iter()
        .map(|t| {
            let name = t.path.file_name().and_then(|s| s.to_str()).unwrap_or("?");
            let gmark = model
                .git_status
                .get(&t.path)
                .map(|c| format!("{c} "))
                .unwrap_or_default();
            if t.dirty {
                format!("● {gmark}{name}")
            } else {
                format!("{gmark}{name}")
            }
        })
        .collect();
    let active = model.active.unwrap_or(0);
    tabs_view(TabsSpec {
        labels,
        active,
        on_select: Msg::ActivateTab,
        content: inner,
        tab_height: TAB_STRIP_H,
        palette: TabsPalette::from_theme(theme),
        tab_width: None,
    })
}

/// Contenido del tab activo: bars (find/completions/hover/etc.) + editor.
/// Si no hay tab activo, devuelve el placeholder.
pub(crate) fn active_editor_content(model: &Model, theme: &Theme) -> View<Msg> {
    let mut children: Vec<View<Msg>> = Vec::new();
    if let Some(p) = model.palette.as_ref() {
        let pal = PalettePalette::from_theme(theme);
        children.push(palette::view(p, &model.palette_commands, &pal, Msg::Palette));
    }
    if let Some(o) = model.outline.as_ref() {
        let pal = OutlinePalette::from_theme(theme);
        children.push(outline::view(o, &model.outline_symbols, &pal, Msg::Outline));
    }
    if let Some(d) = model.diff.as_ref() {
        let pal = DiffPalette::from_theme(theme);
        children.push(diff::view(d, &pal, DIFF_PANEL_H, Msg::Diff));
    }
    if model.bookmarks.overlay.is_some() {
        let pal = BookmarksPalette::from_theme(theme);
        children.push(bookmarks::view(&model.bookmarks, &model.root, &pal, Msg::Bookmarks));
    }
    if let Some(p) = model.picker.as_ref() {
        let palette = PickerPalette::from_theme(theme);
        let ordered = files_with_recents_first(&model.recent_files, &model.all_files);
        children.push(picker::view(p, &ordered, &model.root, &palette, Msg::Picker));
    }
    if let Some(f) = model.fif.as_ref().filter(|s| s.dialog_open) {
        let palette = FifPalette::from_theme(theme);
        children.push(fif::view_dialog(f, &palette, Msg::Fif));
    }
    if let Some(find) = model.find.as_ref() {
        children.push(find_bar(find, theme));
    }
    if let Some(bar) = model.completions.as_ref() {
        children.push(completions_bar_view(bar, theme));
    }
    if let Some(hp) = model.hover.as_ref() {
        children.push(hover_view(hp, theme));
    }
    if let Some(bar) = model.sig_help.as_ref() {
        children.push(sig_help_view(bar, theme));
    }
    if let Some(rb) = model.references.as_ref() {
        children.push(references_view(rb, &model.root, theme));
    }
    if let Some(rn) = model.rename.as_ref() {
        children.push(rename_view(rn, theme));
    }
    if let Some(sa) = model.save_as.as_ref() {
        children.push(save_as_view(sa, theme));
    }
    let editor_view = match model.active_tab() {
        None => empty_editor_placeholder(theme),
        Some(tab) => {
            let language = language_for_path(&tab.path);
            let palette = EditorPalette::from_theme(theme);
            let metrics = EditorMetrics::for_font_size(13.0);
            let matches: Vec<(usize, usize)> = model
                .find
                .as_ref()
                .filter(|f| !f.state.query.is_empty())
                .map(|f| all_matches(&tab.editor.buffer, &f.state))
                .unwrap_or_default();
            text_editor_view_full(
                &tab.editor,
                &palette,
                metrics,
                EDITOR_VISIBLE_LINES,
                language,
                &matches,
                |ev| Some(Msg::EditorPointer(ev)),
            )
        }
    };
    children.push(editor_view);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(children)
}

pub(crate) const FIND_BAR_H: f32 = 32.0;
pub(crate) const COMPLETIONS_BAR_H: f32 = 120.0;
pub(crate) const COMPLETIONS_ROW_H: f32 = 22.0;
pub(crate) const COMPLETIONS_MAX_ITEMS_VISIBLE: usize = 5;

pub(crate) const HOVER_BAR_H: f32 = 96.0;
pub(crate) const SIG_HELP_BAR_H: f32 = 56.0;
pub(crate) const REFS_BAR_H: f32 = 160.0;
pub(crate) const RENAME_BAR_H: f32 = 56.0;

pub(crate) fn save_as_view(sa: &SaveAsBar, theme: &Theme) -> View<Msg> {
    let tp = TextInputPalette::from_theme(theme);
    let header = "save as · ruta completa · Enter guarda · Esc cancela".to_string();
    let header_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        padding: Rect {
            left: length(8.0_f32), right: length(8.0_f32),
            top: length(0.0_f32), bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(header, 10.0, theme.fg_muted, Alignment::Start);

    let input_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(RENAME_BAR_H - 18.0) },
        padding: Rect {
            left: length(6.0_f32), right: length(6.0_f32),
            top: length(2.0_f32), bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![text_input_view(
        &sa.input,
        "/ruta/al/archivo.ext",
        true,
        &tp,
        Msg::SaveAsOpen,
    )]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(RENAME_BAR_H) },
        ..Default::default()
    })
    .children(vec![header_view, input_view])
}

pub(crate) fn rename_view(rb: &RenameBar, theme: &Theme) -> View<Msg> {
    let tp = TextInputPalette::from_theme(theme);
    let header = if rb.waiting {
        format!("rename @ {}:{} · esperando LSP…", rb.anchor.0 + 1, rb.anchor.1)
    } else {
        format!("rename @ {}:{} · Enter aplica · Esc cancela", rb.anchor.0 + 1, rb.anchor.1)
    };
    let header_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        padding: Rect {
            left: length(8.0_f32), right: length(8.0_f32),
            top: length(0.0_f32), bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(header, 10.0, theme.fg_muted, Alignment::Start);

    let input_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(RENAME_BAR_H - 18.0) },
        padding: Rect {
            left: length(6.0_f32), right: length(6.0_f32),
            top: length(2.0_f32), bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![text_input_view(
        &rb.input,
        "nuevo nombre",
        true,
        &tp,
        Msg::RenameOpen,
    )]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(RENAME_BAR_H) },
        ..Default::default()
    })
    .children(vec![header_view, input_view])
}
pub(crate) const REFS_ROW_H: f32 = 20.0;
pub(crate) const REFS_MAX_VISIBLE: usize = 7;

pub(crate) fn references_view(bar: &ReferencesBar, root: &Path, theme: &Theme) -> View<Msg> {
    let header = if bar.items.is_empty() {
        format!(
            "references @ {}:{} · esperando LSP…",
            bar.anchor.0 + 1, bar.anchor.1,
        )
    } else {
        format!(
            "references @ {}:{} · {} / {} · ↓↑ navega · Enter abre · Esc cierra",
            bar.anchor.0 + 1, bar.anchor.1,
            bar.selected + 1, bar.items.len(),
        )
    };
    let mut rows: Vec<View<Msg>> = Vec::new();
    rows.push(
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
            padding: Rect {
                left: length(8.0_f32), right: length(8.0_f32),
                top: length(0.0_f32), bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(theme.bg_panel_alt)
        .text_aligned(header, 10.0, theme.fg_muted, Alignment::Start),
    );
    let visible_start = bar.selected.saturating_sub(REFS_MAX_VISIBLE.saturating_sub(1));
    let visible_end = (visible_start + REFS_MAX_VISIBLE).min(bar.items.len());
    for i in visible_start..visible_end {
        let loc = &bar.items[i];
        let selected = i == bar.selected;
        let bg = if selected { theme.bg_selected } else { theme.bg_panel };
        let label = format!("{}:{}", relative_to(root, &loc.path), loc.line + 1);
        rows.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(REFS_ROW_H) },
                padding: Rect {
                    left: length(10.0_f32), right: length(8.0_f32),
                    top: length(0.0_f32), bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .fill(bg)
            .text_aligned(label, 11.0, theme.fg_text, Alignment::Start),
        );
    }
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(REFS_BAR_H) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(rows)
}

pub(crate) fn sig_help_view(bar: &SignatureHelpBar, theme: &Theme) -> View<Msg> {
    let header = format!(
        "signatureHelp @ {}:{} · Esc cierra",
        bar.anchor.0 + 1,
        bar.anchor.1,
    );
    let body_text = match bar.info.as_ref() {
        None => "esperando LSP…".to_string(),
        Some(info) => {
            let active = info
                .param_labels
                .get(info.active_param)
                .map(|s| format!(" · activo: «{s}»"))
                .unwrap_or_default();
            format!("{}{active}", info.label)
        }
    };
    let header_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(header, 10.0, theme.fg_muted, Alignment::Start);
    let body_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(SIG_HELP_BAR_H - 18.0) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(body_text, 12.0, theme.fg_text, Alignment::Start);
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(SIG_HELP_BAR_H) },
        ..Default::default()
    })
    .children(vec![header_view, body_view])
}

pub(crate) fn hover_view(hp: &HoverPopup, theme: &Theme) -> View<Msg> {
    let header = format!(
        "hover @ {}:{} · Esc cierra",
        hp.anchor.0 + 1,
        hp.anchor.1,
    );
    let body_text = match hp.info.as_ref() {
        None => "esperando LSP…".to_string(),
        Some(info) => truncate_hover(&info.contents, 600),
    };

    let header_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(header, 10.0, theme.fg_muted, Alignment::Start);

    let body_view = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(HOVER_BAR_H - 18.0) },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .text_aligned(body_text, 11.0, theme.fg_text, Alignment::Start)
    // La caja del hover tiene alto fijo (~78px): clampamos a las líneas que
    // caben terminando en «…» en vez de confiar en el recorte de la caja.
    .ellipsis(5);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(HOVER_BAR_H) },
        ..Default::default()
    })
    .children(vec![header_view, body_view])
}

pub(crate) fn truncate_hover(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let cut: String = s.chars().take(max - 1).collect();
        format!("{cut}…")
    }
}

pub(crate) fn completions_bar_view(bar: &CompletionsBar, theme: &Theme) -> View<Msg> {
    let filtered = bar.filtered_indices();
    let mut rows: Vec<View<Msg>> = Vec::with_capacity(COMPLETIONS_MAX_ITEMS_VISIBLE);
    let filter_label = if bar.filter.is_empty() {
        String::new()
    } else {
        format!(" filtro «{}»", bar.filter)
    };
    let header = if bar.items.is_empty() {
        format!(
            "completions @ {}:{}{} · esperando LSP…",
            bar.anchor.0 + 1, bar.anchor.1, filter_label,
        )
    } else if filtered.is_empty() {
        format!(
            "completions @ {}:{}{} · sin matches",
            bar.anchor.0 + 1, bar.anchor.1, filter_label,
        )
    } else {
        format!(
            "completions @ {}:{}{} · {} / {} · Tab/Enter aplica · Esc cierra",
            bar.anchor.0 + 1,
            bar.anchor.1,
            filter_label,
            bar.selected + 1,
            filtered.len(),
        )
    };
    rows.push(
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: length(18.0_f32) },
            padding: Rect {
                left: length(8.0_f32),
                right: length(8.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(theme.bg_panel_alt)
        .text_aligned(header, 10.0, theme.fg_muted, Alignment::Start),
    );

    let visible_start = bar
        .selected
        .saturating_sub(COMPLETIONS_MAX_ITEMS_VISIBLE.saturating_sub(1));
    let visible_end = (visible_start + COMPLETIONS_MAX_ITEMS_VISIBLE).min(filtered.len());
    for vi in visible_start..visible_end {
        let item_idx = filtered[vi];
        let item = &bar.items[item_idx];
        let selected = vi == bar.selected;
        let bg = if selected { theme.bg_selected } else { theme.bg_panel };
        let kind = item.kind.as_deref().unwrap_or("?");
        let detail = item.detail.as_deref().unwrap_or("");
        let label = format!("[{kind:>5}] {}  {}", item.label, detail);
        rows.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(COMPLETIONS_ROW_H) },
                padding: Rect {
                    left: length(10.0_f32),
                    right: length(8.0_f32),
                    top: length(0.0_f32),
                    bottom: length(0.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .fill(bg)
            .text_aligned(label, 11.0, theme.fg_text, Alignment::Start),
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(COMPLETIONS_BAR_H) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(rows)
}

pub(crate) fn find_bar(find: &FindBarState, theme: &Theme) -> View<Msg> {
    let tp = TextInputPalette::from_theme(theme);
    let input = text_input_view(&find.input, "buscar… (Enter / Ctrl+G siguiente · Shift inverso · Esc cierra)", true, &tp, Msg::FindOpen);
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(FIND_BAR_H) },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(2.0_f32),
            bottom: length(2.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(vec![input])
}

pub(crate) fn empty_editor_placeholder(theme: &Theme) -> View<Msg> {
    let title = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(36.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned("nada".to_string(), 22.0, theme.fg_text, Alignment::Center);

    let subtitle = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(rimay_localize::t("nada-tagline"), 12.0, theme.fg_muted, Alignment::Center);

    fn row(theme: &Theme, key: &str, action: &str) -> View<Msg> {
        let key_v = View::new(Style {
            size: Size { width: length(180.0_f32), height: length(22.0_f32) },
            padding: Rect { left: length(10.0_f32), right: length(10.0_f32), top: length(2.0_f32), bottom: length(2.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .fill(theme.bg_panel_alt)
        .radius(3.0)
        .text_aligned(key.to_string(), 11.0, theme.fg_text, Alignment::Center);
        let action_v = View::new(Style {
            size: Size { width: length(220.0_f32), height: length(22.0_f32) },
            padding: Rect { left: length(12.0_f32), right: length(0.0_f32), top: length(0.0_f32), bottom: length(0.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(action.to_string(), 11.5, theme.fg_muted, Alignment::Start);
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: length(420.0_f32), height: length(26.0_f32) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .children(vec![key_v, action_v])
    }

    let card_children = vec![
        title,
        subtitle,
        View::new(Style { size: Size { width: percent(1.0_f32), height: length(20.0_f32) }, ..Default::default() }),
        row(theme, "Ctrl+P", "Abrir archivo (fuzzy file picker)"),
        row(theme, "Ctrl+Shift+P", "Command Palette"),
        row(theme, "Ctrl+Shift+F", "Find in Files"),
        row(theme, "Ctrl+`", "Abrir terminal integrado"),
        row(theme, "Ctrl+Shift+O", "Symbol Outline"),
        row(theme, "Ctrl+Shift+M", "Toggle Mini-Map"),
        row(theme, "Ctrl+Alt+B", "Toggle Bookmark"),
    ];
    let body_card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(460.0_f32), height: length(320.0_f32) },
        padding: Rect { left: length(20.0_f32), right: length(20.0_f32), top: length(24.0_f32), bottom: length(24.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(8.0)
    .children(card_children);

    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Column,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_app)
    .children(vec![body_card])
}
