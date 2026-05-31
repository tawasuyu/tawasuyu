use super::*;

/// Mantiene la celda seleccionada dentro del viewport con un margen
/// de seguridad. Si la celda salió por arriba/izquierda, el viewport
/// se acerca; si salió por abajo/derecha, el viewport avanza lo
/// justo para volver a verla más el margen. Las celdas que caen
/// dentro de una banda inmovilizada están siempre a la vista, así que
/// no fuerzan ningún scroll en ese eje.
pub(crate) fn ensure_visible(model: &mut Model) {
    let sel = model.selected;
    // Vertical — el área scrolleable tiene `VISIBLE_ROWS - freeze_rows`
    // ranuras y arranca en `viewport_row` (>= freeze_rows).
    if sel.row >= model.freeze_rows {
        let scroll_rows = VISIBLE_ROWS.saturating_sub(model.freeze_rows).max(1);
        let margin = SCROLL_MARGIN_ROWS.min(scroll_rows.saturating_sub(1));
        let v_top = model.viewport_row;
        let v_bot = model.viewport_row + scroll_rows;
        if sel.row < v_top + margin {
            model.viewport_row =
                sel.row.saturating_sub(margin).max(model.freeze_rows);
        } else if sel.row + margin >= v_bot {
            model.viewport_row = (sel.row + margin + 1)
                .saturating_sub(scroll_rows)
                .max(model.freeze_rows);
        }
    }
    // Horizontal — análogo.
    if sel.col >= model.freeze_cols {
        let scroll_cols = VISIBLE_COLS.saturating_sub(model.freeze_cols).max(1);
        let margin = SCROLL_MARGIN_COLS.min(scroll_cols.saturating_sub(1));
        let h_left = model.viewport_col;
        let h_right = model.viewport_col + scroll_cols;
        if sel.col < h_left + margin {
            model.viewport_col =
                sel.col.saturating_sub(margin).max(model.freeze_cols);
        } else if sel.col + margin >= h_right {
            model.viewport_col = (sel.col + margin + 1)
                .saturating_sub(scroll_cols)
                .max(model.freeze_cols);
        }
    }
}

/// Construye la lista de items del menú contextual de una celda. El
/// orden de items aquí es el contrato implícito de
/// `activate_menu_item` — si reordenás, asegurate de mover el match.
pub(crate) fn menu_items(
    wb: &Workbook,
    has_clipboard: bool,
    frozen: bool,
) -> Vec<ContextMenuItem> {
    let can_undo = wb.events().len() > 0; // approximation; el Workbook expone applied_count
    let _ = can_undo;
    vec![
        ContextMenuItem::action("Copiar").with_shortcut("Ctrl+C"),       // 0
        ContextMenuItem::action("Cortar").with_shortcut("Ctrl+X"),       // 1
        if has_clipboard {
            ContextMenuItem::action("Pegar").with_shortcut("Ctrl+V")
        } else {
            ContextMenuItem::action("Pegar")
                .with_shortcut("Ctrl+V")
                .disabled()
        },                                                                // 2
        ContextMenuItem::separator(),                                    // 3
        ContextMenuItem::action("Limpiar")
            .with_shortcut("Del")
            .destructive(),                                              // 4
        ContextMenuItem::separator(),                                    // 5
        ContextMenuItem::action("Formato: Número").with_shortcut("Ctrl+!"), // 6
        ContextMenuItem::action("Formato: Moneda  $").with_shortcut("Ctrl+$"), // 7
        ContextMenuItem::action("Formato: Porcentaje").with_shortcut("Ctrl+%"), // 8
        ContextMenuItem::action("Formato: General").with_shortcut("Ctrl+)"), // 9
        ContextMenuItem::separator(),                                    // 10
        if wb.can_undo() {
            ContextMenuItem::action("Deshacer").with_shortcut("Ctrl+Z")
        } else {
            ContextMenuItem::action("Deshacer")
                .with_shortcut("Ctrl+Z")
                .disabled()
        },                                                                // 11
        if wb.can_redo() {
            ContextMenuItem::action("Rehacer").with_shortcut("Ctrl+Y")
        } else {
            ContextMenuItem::action("Rehacer")
                .with_shortcut("Ctrl+Y")
                .disabled()
        },                                                                // 12
        ContextMenuItem::separator(),                                    // 13
        ContextMenuItem::action("Inmovilizar paneles aquí")
            .with_shortcut("Ctrl+Shift+F"),                              // 14
        if frozen {
            ContextMenuItem::action("Liberar paneles")
        } else {
            ContextMenuItem::action("Liberar paneles").disabled()
        },                                                                // 15
        ContextMenuItem::separator(),                                    // 16
        ContextMenuItem::action("Tabla dinámica…").with_shortcut("Ctrl+Shift+P"), // 17
    ]
}

/// Traduce un índice del menú a su Msg-equivalente. `None` para
/// separators o índices sin acción. Es la fuente de verdad para qué
/// hace cada fila del menú.
pub(crate) fn menu_item_msg(idx: usize) -> Option<Msg> {
    match idx {
        0 => Some(Msg::Copy),
        1 => Some(Msg::Cut),
        2 => Some(Msg::Paste),
        4 => Some(Msg::ClearActive),
        6 => Some(Msg::ApplyFormat(CellFormat::Number { decimals: 2 })),
        7 => Some(Msg::ApplyFormat(CellFormat::Currency {
            symbol: "$".into(),
            decimals: 2,
        })),
        8 => Some(Msg::ApplyFormat(CellFormat::Percent { decimals: 0 })),
        9 => Some(Msg::ApplyFormat(CellFormat::General)),
        11 => Some(Msg::Undo),
        12 => Some(Msg::Redo),
        14 => Some(Msg::FreezeAtSelection),
        15 => Some(Msg::Unfreeze),
        17 => Some(Msg::OpenPivot),
        _ => None,
    }
}

pub(crate) fn title_bar_view(selected: CellRef, freeze_rows: u32, freeze_cols: u32) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(TOP_HEADER_H),
        },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(palette::BG_PANEL)
    .children(vec![View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        if freeze_rows == 0 && freeze_cols == 0 {
            format!("nakui-sheet  ·  celda activa: {selected}")
        } else {
            format!(
                "nakui-sheet  ·  celda activa: {selected}  ·  ❄ {freeze_rows}×{freeze_cols}"
            )
        },
        13.0,
        palette::FG_TEXT,
        Alignment::Start,
    )])
}

pub(crate) fn formula_bar_view(t: &Theme, bar: &TextInputState, selected: CellRef) -> View<Msg> {
    let input_palette = TextInputPalette::from_theme(t);
    // Box pequeño tipo "Name Box" de Excel: muestra la cell activa
    // con fondo accent translúcido para que sea inconfundible.
    let label = View::new(Style {
        size: Size {
            width: length(70.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(palette::BG_PANEL_ALT)
    .text_aligned(
        selected.to_string(),
        13.0,
        palette::ACCENT,
        Alignment::Center,
    );

    // Offsets de ventana del origen top-left de este wrapper de input:
    // a su izquierda viene el label (70px) y el wrapper agrega 8px de
    // padding izquierdo; arriba vienen menubar + título + el padding
    // superior (4px) de la barra de fórmula. `on_right_click_at` da
    // coords locales al rect del nodo, así que sumamos ese origen para
    // anclar el menú de edición en coordenadas de ventana.
    const INPUT_ORIGIN_X: f32 = 70.0 + 8.0;
    let input_origin_y = MENU_H + TOP_HEADER_H + 4.0;
    let input = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        flex_grow: 1.0,
        ..Default::default()
    })
    .on_right_click_at(move |lx, ly, _w, _h| {
        Some(Msg::EditMenuOpen(INPUT_ORIGIN_X + lx, input_origin_y + ly))
    })
    .children(vec![text_input_view(
        bar,
        "ingresa fórmula o valor",
        true,
        &input_palette,
        Msg::SelectCell(selected),
    )]);

    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(FORMULA_BAR_H),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(4.0_f32),
            bottom: length(4.0_f32),
        },
        ..Default::default()
    })
    .fill(palette::BG_APP)
    .children(vec![label, input])
}

pub(crate) fn grid_view(
    wb: &Workbook,
    selected: CellRef,
    viewport_row: u32,
    viewport_col: u32,
    editing: bool,
    bar: &TextInputState,
    model: &Model,
) -> View<Msg> {
    let mut rows: Vec<View<Msg>> = Vec::new();
    let freeze_rows = model.freeze_rows;
    let freeze_cols = model.freeze_cols;
    // Cabecera de columnas: corner + columnas inmovilizadas + columnas
    // scrolleables a partir del viewport.
    rows.push(column_header_row(viewport_col, freeze_cols));
    // Banda de filas inmovilizadas (0..freeze_rows): siempre arriba.
    for abs_row in 0..freeze_rows {
        rows.push(data_row(
            wb,
            selected,
            abs_row,
            viewport_col,
            freeze_cols,
            editing,
            bar,
            model,
        ));
    }
    // Filas scrolleables. Cada r local mapea a row = viewport_row + r,
    // y `viewport_row >= freeze_rows` por invariante, así que no se
    // pisan con la banda inmovilizada.
    let scroll_rows = VISIBLE_ROWS.saturating_sub(freeze_rows);
    for r in 0..scroll_rows {
        let abs_row = viewport_row + r;
        rows.push(data_row(
            wb,
            selected,
            abs_row,
            viewport_col,
            freeze_cols,
            editing,
            bar,
            model,
        ));
    }
    // El contenedor de la grilla se pinta con el color de las líneas
    // — los bordes inferior/derecho de cada celda dejan ver este
    // fondo, lo cual crea la cuadrícula sin overdrawing. El borde
    // superior+izquierdo del grid surge automáticamente porque la
    // primera fila/columna apoya contra este fondo.
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        padding: Rect {
            left: length(1.0_f32),
            right: length(0.0_f32),
            top: length(1.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(palette::GRID_LINE)
    .children(rows)
}

/// Wrap genérico para una celda de la grilla: rect padre del color
/// de las líneas con padding right+bottom = 1px que deja ver la
/// línea; hijo del color de fondo de la celda. Cada celda "lleva
/// puesto" su borde inferior+derecho — el superior y el izquierdo
/// del grid los aporta el contenedor exterior.
pub(crate) fn bordered_cell(
    width_px: f32,
    height_px: f32,
    bg: Color,
    hover: Option<Color>,
    fg: Color,
    text: String,
    text_align: Alignment,
    on_click: Option<Msg>,
) -> View<Msg> {
    let mut inner = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .text_aligned(text, 12.5, fg, text_align);
    if let Some(h) = hover {
        inner = inner.hover_fill(h);
    }
    if let Some(msg) = on_click {
        inner = inner.on_click(msg);
    }
    View::new(Style {
        size: Size {
            width: length(width_px),
            height: length(height_px),
        },
        padding: Rect {
            left: length(0.0_f32),
            right: length(1.0_f32),
            top: length(0.0_f32),
            bottom: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette::GRID_LINE)
    .children(vec![inner])
}

pub(crate) fn column_header_row(viewport_col: u32, freeze_cols: u32) -> View<Msg> {
    let mut cells: Vec<View<Msg>> = Vec::new();
    // Esquina vacía — más oscura para anclar visualmente la grilla.
    cells.push(bordered_cell(
        ROW_HEADER_W,
        CELL_H,
        palette::BG_HEADER,
        None,
        palette::FG_HEADER,
        String::new(),
        Alignment::Center,
        None,
    ));
    // Una closure para no duplicar el header de columna. Las columnas
    // inmovilizadas se rotulan en accent para señalar el anclaje.
    let push_header = |cells: &mut Vec<View<Msg>>, abs_col: u32, frozen: bool| {
        cells.push(bordered_cell(
            CELL_W,
            CELL_H,
            palette::BG_HEADER,
            None,
            if frozen {
                palette::ACCENT
            } else {
                palette::FG_HEADER
            },
            CellRef::col_label(abs_col),
            Alignment::Center,
            None,
        ));
    };
    for abs_col in 0..freeze_cols {
        push_header(&mut cells, abs_col, true);
    }
    let scroll_cols = VISIBLE_COLS.saturating_sub(freeze_cols);
    for c in 0..scroll_cols {
        let abs_col = viewport_col + c;
        push_header(&mut cells, abs_col, false);
    }
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(CELL_H),
        },
        ..Default::default()
    })
    .children(cells)
}

pub(crate) fn data_row(
    wb: &Workbook,
    selected: CellRef,
    row: u32,
    viewport_col: u32,
    freeze_cols: u32,
    editing: bool,
    bar: &TextInputState,
    model: &Model,
) -> View<Msg> {
    let is_active_row = row == selected.row;
    let is_frozen_row = row < model.freeze_rows;
    let mut cells: Vec<View<Msg>> = Vec::new();
    // Cabecera de fila — accent suave si la fila contiene la celda
    // activa o si está inmovilizada.
    let header_bg = if is_active_row {
        palette::BG_PANEL_ALT
    } else {
        palette::BG_HEADER
    };
    let header_fg = if is_active_row || is_frozen_row {
        palette::ACCENT
    } else {
        palette::FG_HEADER
    };
    cells.push(bordered_cell(
        ROW_HEADER_W,
        CELL_H,
        header_bg,
        None,
        header_fg,
        format!("{}", row + 1),
        Alignment::Center,
        None,
    ));
    let push_cell = |cells: &mut Vec<View<Msg>>, abs_col: u32| {
        let cr = CellRef::new(abs_col, row);
        if editing && cr == selected {
            cells.push(editing_cell_view(bar));
        } else {
            cells.push(cell_view(wb, selected, cr, model));
        }
    };
    for abs_col in 0..freeze_cols {
        push_cell(&mut cells, abs_col);
    }
    let scroll_cols = VISIBLE_COLS.saturating_sub(freeze_cols);
    for c in 0..scroll_cols {
        let abs_col = viewport_col + c;
        push_cell(&mut cells, abs_col);
    }
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(CELL_H),
        },
        ..Default::default()
    })
    .children(cells)
}

/// Celda en modo edición: muestra el contenido del text-input
/// directamente, con un borde accent para que el usuario vea
/// claramente que está tipeando ahí (y no solo en la barra).
pub(crate) fn editing_cell_view(bar: &TextInputState) -> View<Msg> {
    let text = bar.text();
    let inner = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(palette::BG_PANEL_ALT)
    .text_aligned(text, 12.5, palette::FG_TEXT, Alignment::Start);

    // Padre del color accent para que la celda tenga un borde
    // distinguible (los 1px de padding right+bottom siguen
    // marcando la grilla, pero ahora ese borde es accent).
    View::new(Style {
        size: Size {
            width: length(CELL_W),
            height: length(CELL_H),
        },
        padding: Rect {
            left: length(1.0_f32),
            right: length(1.0_f32),
            top: length(1.0_f32),
            bottom: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(palette::ACCENT)
    .children(vec![inner])
}

pub(crate) fn cell_view(wb: &Workbook, selected: CellRef, cr: CellRef, model: &Model) -> View<Msg> {
    let is_sel = cr == selected;
    // `in_sel_range` cubre todas las celdas del rango activo
    // EXCEPTO la "live cell" (active). Excel pinta el rango con un
    // tinte sutil y deja la active sólida en accent — eso es lo
    // que reproducimos aquí.
    let in_sel_range = !is_sel && cell_in_selection(model, cr);
    let value = wb.value(cr);
    let display = match &value {
        SheetValue::Empty => String::new(),
        // El display respeta el formato configurado en la celda
        // (Number/Currency/Percent/General). Los no-numéricos
        // ignoran el formato a propósito.
        _ => wb.formatted(cr),
    };
    let is_error = matches!(value, SheetValue::Error(_));
    let is_text = matches!(value, SheetValue::Text(_));

    let is_frozen = cr.row < model.freeze_rows || cr.col < model.freeze_cols;
    let bg = if is_sel {
        palette::ACCENT
    } else if is_error {
        palette::ERROR_BG
    } else if in_sel_range {
        palette::SEL_RANGE_BG
    } else if is_frozen {
        palette::FROZEN_BG
    } else {
        palette::BG_CELL
    };
    let fg = if is_sel {
        palette::ACCENT_FG
    } else if is_error {
        palette::ERROR
    } else {
        palette::FG_TEXT
    };
    let alignment = if is_text {
        Alignment::Start
    } else {
        Alignment::End
    };

    // Right-click sobre la celda abre el menú contextual. El cálculo
    // de la posición de anclaje del panel lo hace `view_overlay`
    // mirroreando la matemática de `grid_view` desde la cell y el
    // viewport — `on_right_click_at` da local_x/local_y, pero no la
    // posición global. Pasamos la pos local en el Msg por si más
    // adelante queremos posicionar exactamente bajo el cursor.
    let cell = bordered_cell(
        CELL_W,
        CELL_H,
        bg,
        if is_sel { None } else { Some(palette::BG_CELL_HOVER) },
        fg,
        display,
        alignment,
        Some(Msg::SelectCell(cr)),
    );
    cell.on_right_click_at(move |lx, ly, _, _| {
        Some(Msg::OpenMenu {
            cell: cr,
            pos: (lx, ly),
        })
    })
}

pub(crate) fn status_bar_view(status: &Status) -> View<Msg> {
    let (bg, fg) = match status.kind {
        StatusKind::Info => (palette::BG_PANEL, palette::FG_MUTED),
        StatusKind::Error => (palette::ERROR_BG, palette::ERROR),
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(STATUS_H),
        },
        padding: Rect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .text_aligned(status.text.clone(), 12.0, fg, Alignment::Start)
}

/// Construye el menú principal (barra superior). Archivo / Editar /
/// Ver / Ayuda. El submenú "Editar" refleja en gris el estado real de
/// la barra de fórmula (input focuseado) y del Workbook.
pub(crate) fn app_menu(model: &Model) -> app_bus::AppMenu {
    use app_bus::{AppMenu, Menu, MenuItem};

    let ed = model.bar.editor();
    let has_sel = ed.has_selection();
    let has_text = !ed.is_empty();
    let can_undo_wb = model.wb.can_undo();
    let can_redo_wb = model.wb.can_redo();
    let has_clip = model.clipboard_origin.is_some();
    let frozen = model.freeze_rows > 0 || model.freeze_cols > 0;

    // --- Editar: undo/redo del Workbook + cut/copy/paste de celda + edición
    //     in-situ del texto de la barra (cut/copy/paste/seleccionar todo).
    let mut undo = MenuItem::new("Deshacer", "edit.undo").shortcut("Ctrl+Z");
    if !can_undo_wb { undo = undo.disabled(); }
    let mut redo = MenuItem::new("Rehacer", "edit.redo").shortcut("Ctrl+Y");
    if !can_redo_wb { redo = redo.disabled(); }
    let cell_cut = MenuItem::new("Cortar celda", "cell.cut").shortcut("Ctrl+X").separated();
    let cell_copy = MenuItem::new("Copiar celda", "cell.copy").shortcut("Ctrl+C");
    let mut cell_paste = MenuItem::new("Pegar celda", "cell.paste").shortcut("Ctrl+V");
    if !has_clip { cell_paste = cell_paste.disabled(); }
    let cell_clear = MenuItem::new("Limpiar celda", "cell.clear").shortcut("Del");
    // Edición del texto de la barra (input focuseado).
    let mut bar_cut = MenuItem::new("Cortar texto", "bar.cut").separated();
    let mut bar_copy = MenuItem::new("Copiar texto", "bar.copy");
    if !has_sel { bar_cut = bar_cut.disabled(); bar_copy = bar_copy.disabled(); }
    let bar_paste = MenuItem::new("Pegar texto", "bar.paste");
    let mut bar_sel_all = MenuItem::new("Seleccionar todo (texto)", "bar.selectall");
    if !has_text { bar_sel_all = bar_sel_all.disabled(); }

    // --- Ver: tema + formatos + inmovilizar + tabla dinámica.
    let mut unfreeze = MenuItem::new("Liberar paneles", "view.unfreeze");
    if !frozen { unfreeze = unfreeze.disabled(); }

    AppMenu::new()
        .menu(
            Menu::new("Archivo")
                .item(MenuItem::new("Importar CSV", "file.import").shortcut("Ctrl+I"))
                .item(MenuItem::new("Exportar CSV", "file.export").shortcut("Ctrl+E")),
        )
        .menu(
            Menu::new("Editar")
                .item(undo)
                .item(redo)
                .item(cell_cut)
                .item(cell_copy)
                .item(cell_paste)
                .item(cell_clear)
                .item(bar_cut)
                .item(bar_copy)
                .item(bar_paste)
                .item(bar_sel_all),
        )
        .menu(
            Menu::new("Ver")
                .item(MenuItem::new("Formato: Número", "fmt.number").shortcut("Ctrl+!"))
                .item(MenuItem::new("Formato: Moneda $", "fmt.currency").shortcut("Ctrl+$"))
                .item(MenuItem::new("Formato: Porcentaje", "fmt.percent").shortcut("Ctrl+%"))
                .item(MenuItem::new("Formato: General", "fmt.general").shortcut("Ctrl+)"))
                .item(MenuItem::new("Inmovilizar paneles aquí", "view.freeze").shortcut("Ctrl+Shift+F").separated())
                .item(unfreeze)
                .item(MenuItem::new("Tabla dinámica…", "view.pivot").shortcut("Ctrl+Shift+P").separated()),
        )
        .menu(
            Menu::new("Ayuda")
                .item(MenuItem::new("Acerca de Nakui Sheet", "help.about")),
        )
}

/// Traduce un comando del menú principal al `Msg` real de la planilla.
/// `None` para entradas informativas sin acción cableada.
pub(crate) fn menubar_command_msg(model: &Model, command: &str) -> Option<Msg> {
    match command {
        "file.import" => Some(Msg::ImportCsv),
        "file.export" => Some(Msg::ExportCsv),
        "edit.undo" => Some(Msg::Undo),
        "edit.redo" => Some(Msg::Redo),
        "cell.cut" => Some(Msg::Cut),
        "cell.copy" => Some(Msg::Copy),
        "cell.paste" => Some(Msg::Paste),
        "cell.clear" => Some(Msg::ClearActive),
        "bar.cut" => Some(Msg::EditMenuAction(EditAction::Cut)),
        "bar.copy" => Some(Msg::EditMenuAction(EditAction::Copy)),
        "bar.paste" => Some(Msg::EditMenuAction(EditAction::Paste)),
        "bar.selectall" => Some(Msg::EditMenuAction(EditAction::SelectAll)),
        "fmt.number" => Some(Msg::ApplyFormat(CellFormat::Number { decimals: 2 })),
        "fmt.currency" => Some(Msg::ApplyFormat(CellFormat::Currency {
            symbol: "$".into(),
            decimals: 2,
        })),
        "fmt.percent" => Some(Msg::ApplyFormat(CellFormat::Percent { decimals: 0 })),
        "fmt.general" => Some(Msg::ApplyFormat(CellFormat::General)),
        "view.freeze" => Some(Msg::FreezeAtSelection),
        "view.unfreeze" => Some(Msg::Unfreeze),
        "view.pivot" => Some(Msg::OpenPivot),
        "help.about" => {
            let _ = model;
            None
        }
        _ => None,
    }
}

/// Theme custom: `Theme::dark()` con overrides para que `text-input`
/// (que se construye desde un Theme) use nuestra paleta dark-sheet.
pub(crate) fn dark_sheet_theme() -> Theme {
    let mut t = Theme::dark();
    t.bg_app = palette::BG_APP;
    t.bg_panel = palette::BG_PANEL;
    t.bg_panel_alt = palette::BG_PANEL_ALT;
    t.bg_input = palette::BG_CELL;
    t.bg_input_focus = palette::BG_PANEL_ALT;
    t.fg_text = palette::FG_TEXT;
    t.fg_muted = palette::FG_MUTED;
    t.fg_placeholder = palette::FG_PLACEHOLDER;
    t.border = palette::GRID_LINE;
    t.border_focus = palette::ACCENT;
    t.accent = palette::ACCENT;
    t
}

