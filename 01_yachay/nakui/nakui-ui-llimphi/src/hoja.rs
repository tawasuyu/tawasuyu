//! Área **Hoja** — vista tipo Excel embebida en el shell de Nakui.
//!
//! Reusa el motor real de hojas (`nakui-sheet` → `yupay`): un `Workbook`
//! con fórmulas, recálculo reactivo y undo/redo. Acá vive sólo la
//! *presentación* dentro del shell unificado (la app dedicada
//! `nakui-sheet-llimphi` sigue siendo la referencia completa con pivot y
//! freeze panes). La grilla se arma con `View`s anidados —una celda por
//! nodo— para que el hit-testing del click sea directo; las líneas de la
//! cuadrícula salen "gratis" del fill del contenedor visto por el gap de
//! 1px entre celdas.

use super::*;
use nakui_sheet::{CellRef, Workbook};

/// Columnas/filas visibles del viewport.
pub(crate) const HOJA_COLS: u32 = 14;
pub(crate) const HOJA_ROWS: u32 = 28;
const CELL_W: f32 = 104.0;
const CELL_H: f32 = 24.0;
const ROW_HDR_W: f32 = 48.0;

/// Estado vivo de la hoja activa. Vive en el `Model` porque la barra de
/// fórmula mantiene su `TextInputState` (cursor + buffer) entre frames.
pub(crate) struct SheetView {
    pub wb: Workbook,
    /// Celda activa (`col`, `row`, 0-indexados).
    pub sel: CellRef,
    /// Buffer vivo de la barra de fórmula; se carga del `raw` de la celda
    /// al cambiar de selección y se aplica con Enter.
    pub bar: TextInputState,
    /// `true` mientras se teclea sobre la celda (Enter aplica, Esc revierte).
    pub editing: bool,
    /// Esquina superior izquierda del viewport visible.
    pub viewport_col: u32,
    pub viewport_row: u32,
    /// Último mensaje de estado (vacío = ok).
    pub status: String,
}

impl SheetView {
    pub(crate) fn new() -> Self {
        let mut wb = Workbook::new();
        seed(&mut wb);
        let sel = CellRef::new(0, 0);
        let mut bar = TextInputState::new();
        bar.set_text(wb.raw(sel).unwrap_or(""));
        Self {
            wb,
            sel,
            bar,
            editing: false,
            viewport_col: 0,
            viewport_row: 0,
            status: String::new(),
        }
    }

    fn reload_bar(&mut self) {
        self.bar.set_text(self.wb.raw(self.sel).unwrap_or(""));
    }

    /// Aplica el buffer de la barra a la celda activa.
    pub(crate) fn commit(&mut self) {
        let raw = self.bar.text().to_string();
        self.status = match self.wb.set_cell(self.sel, &raw) {
            Ok(_) => String::new(),
            Err(e) => format!("✗ {e}"),
        };
        self.editing = false;
    }

    /// Selecciona una celda concreta (click). Aplica una edición en curso.
    pub(crate) fn select(&mut self, col: u32, row: u32) {
        if self.editing {
            self.commit();
        }
        self.sel = CellRef::new(col, row);
        self.reload_bar();
        self.ensure_visible();
    }

    /// Mueve la selección por delta, con clamp a coordenadas no-negativas.
    pub(crate) fn move_by(&mut self, dcol: i32, drow: i32) {
        if self.editing {
            self.commit();
        }
        let c = (self.sel.col as i32 + dcol).max(0) as u32;
        let r = (self.sel.row as i32 + drow).max(0) as u32;
        self.sel = CellRef::new(c, r);
        self.reload_bar();
        self.ensure_visible();
    }

    pub(crate) fn cancel(&mut self) {
        self.reload_bar();
        self.editing = false;
        self.status.clear();
    }

    pub(crate) fn clear_active(&mut self) {
        self.status = match self.wb.clear_cell(self.sel) {
            Ok(_) => String::new(),
            Err(e) => format!("✗ {e}"),
        };
        self.bar.set_text("");
    }

    pub(crate) fn undo(&mut self) {
        if let Ok(Some(_)) = self.wb.undo() {
            self.reload_bar();
            self.status = "↶ deshacer".into();
        }
    }

    pub(crate) fn redo(&mut self) {
        if let Ok(Some(_)) = self.wb.redo() {
            self.reload_bar();
            self.status = "↷ rehacer".into();
        }
    }

    /// Ajusta el viewport para que la selección quede siempre visible.
    fn ensure_visible(&mut self) {
        if self.sel.col < self.viewport_col {
            self.viewport_col = self.sel.col;
        } else if self.sel.col >= self.viewport_col + HOJA_COLS {
            self.viewport_col = self.sel.col + 1 - HOJA_COLS;
        }
        if self.sel.row < self.viewport_row {
            self.viewport_row = self.sel.row;
        } else if self.sel.row >= self.viewport_row + HOJA_ROWS {
            self.viewport_row = self.sel.row + 1 - HOJA_ROWS;
        }
    }

    pub(crate) fn scroll(&mut self, dcol: i32, drow: i32) {
        self.viewport_col = (self.viewport_col as i32 + dcol).max(0) as u32;
        self.viewport_row = (self.viewport_row as i32 + drow).max(0) as u32;
    }
}

/// Datos de ejemplo: una factura con fórmulas vivas para que la hoja se
/// vea con contenido al primer arranque.
fn seed(wb: &mut Workbook) {
    let rows = [
        ("A1", "Concepto"), ("B1", "Cant"), ("C1", "Unit"), ("D1", "Subtotal"), ("E1", "IVA"),
        ("A2", "Café"),     ("B2", "5"),    ("C2", "20"),   ("D2", "=B2*C2"),   ("E2", "=D2*16%"),
        ("A3", "Té"),       ("B3", "3"),    ("C3", "15"),   ("D3", "=B3*C3"),   ("E3", "=D3*16%"),
        ("A4", "Azúcar"),   ("B4", "2"),    ("C4", "10"),   ("D4", "=B4*C4"),   ("E4", "=D4*16%"),
        ("A6", "TOTAL"),    ("D6", "=SUM(D2:D4)"), ("E6", "=SUM(E2:E4)"),
    ];
    for (cell, raw) in rows {
        if let Ok(cr) = cell.parse::<CellRef>() {
            let _ = wb.set_cell(cr, raw);
        }
    }
}

/// Construye el área de la hoja: barra de fórmula + grilla.
pub(crate) fn build_hoja(model: &Model, theme: &Theme) -> View<Msg> {
    let s = &model.sheet;
    let formula = formula_bar(s, theme);
    let grid = grid(s, theme);
    let status = text_line(
        if s.status.is_empty() {
            format!("{}   ·   hoja viva — Enter aplica, Esc revierte, ↑↓←→ navega", s.sel)
        } else {
            s.status.clone()
        },
        11.0,
        theme.fg_muted,
    );

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_grow: 1.0,
        gap: Size {
            width: length(0.0_f32),
            height: length(8.0_f32),
        },
        ..Default::default()
    })
    .children(vec![formula, grid, status])
}

/// Barra de fórmula: etiqueta de la celda activa + input con su `raw`.
fn formula_bar(s: &SheetView, theme: &Theme) -> View<Msg> {
    let label = View::new(Style {
        size: Size {
            width: length(56.0_f32),
            height: length(30.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(5.0)
    .text_aligned(s.sel.to_string(), 12.5, theme.accent, Alignment::Center);

    let input = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(30.0_f32),
        },
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![text_input_view(
        &s.bar,
        "fx — escribí un valor o =fórmula",
        true,
        &TextInputPalette::from_theme(theme),
        Msg::HojaFocusBar,
    )]);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(30.0_f32),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(8.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(vec![label, input])
}

/// La grilla. El contenedor se pinta con el color de borde; el gap de 1px
/// entre filas y celdas deja ver esa línea como cuadrícula.
fn grid(s: &SheetView, theme: &Theme) -> View<Msg> {
    let mut rows: Vec<View<Msg>> = Vec::with_capacity(HOJA_ROWS as usize + 1);

    // Fila de encabezados: esquina + etiquetas de columna.
    let mut header: Vec<View<Msg>> = vec![corner_cell(theme)];
    for dc in 0..HOJA_COLS {
        let col = s.viewport_col + dc;
        header.push(header_cell(CellRef::col_label(col), CELL_W, theme));
    }
    rows.push(grid_row(header));

    for dr in 0..HOJA_ROWS {
        let row = s.viewport_row + dr;
        let mut cells: Vec<View<Msg>> = vec![header_cell((row + 1).to_string(), ROW_HDR_W, theme)];
        for dc in 0..HOJA_COLS {
            let col = s.viewport_col + dc;
            cells.push(data_cell(s, col, row, theme));
        }
        rows.push(grid_row(cells));
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: auto(),
            height: auto(),
        },
        flex_grow: 1.0,
        gap: Size {
            width: length(0.0_f32),
            height: length(1.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.border)
    .children(rows)
}

fn grid_row(cells: Vec<View<Msg>>) -> View<Msg> {
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: auto(),
            height: length(CELL_H),
        },
        flex_shrink: 0.0,
        gap: Size {
            width: length(1.0_f32),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    .children(cells)
}

fn corner_cell(theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(ROW_HDR_W),
            height: length(CELL_H),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
}

fn header_cell(label: String, width: f32, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size {
            width: length(width),
            height: length(CELL_H),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .text_aligned(label, 11.0, theme.fg_muted, Alignment::Center)
}

fn data_cell(s: &SheetView, col: u32, row: u32, theme: &Theme) -> View<Msg> {
    let cr = CellRef::new(col, row);
    let selected = s.sel == cr;

    // Edición in-cell: la celda activa en modo edición monta un text-input
    // real (caret + foco) sobre el buffer de la barra; las teclas ya viajan
    // por `HojaFormulaKey`. Fuera de edición, valor calculado estático.
    if selected && s.editing {
        let mut pal = TextInputPalette::from_theme(theme);
        pal.bg = theme.bg_input_focus;
        pal.border = theme.accent;
        pal.border_focus = theme.accent;
        return View::new(Style {
            size: Size {
                width: length(CELL_W),
                height: length(CELL_H),
            },
            flex_shrink: 0.0,
            ..Default::default()
        })
        .children(vec![text_input_view(
            &s.bar,
            "",
            true,
            &pal,
            Msg::HojaFocusBar,
        )]);
    }

    let display = s.wb.formatted(cr);
    let (bg, fg) = if selected {
        (theme.accent, theme.bg_app)
    } else {
        (theme.bg_panel, theme.fg_text)
    };

    View::new(Style {
        size: Size {
            width: length(CELL_W),
            height: length(CELL_H),
        },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(bg)
    .hover_fill(if selected { bg } else { theme.bg_row_hover })
    .text_aligned(display, 11.5, fg, Alignment::Start)
    // Click en la celda activa entra a edición in-cell; en otra, la selecciona.
    .on_click(if selected {
        Msg::HojaEditStart
    } else {
        Msg::HojaSelectCell { col, row }
    })
}

/// Teclado de la hoja (sólo cuando el área Hoja está activa y ningún menú
/// está abierto). Devuelve el `Msg` a despachar.
pub(crate) fn on_key(s: &SheetView, ev: &KeyEvent) -> Option<Msg> {
    if ev.state != KeyState::Pressed {
        // El release viaja a la barra para no perder eventos del input.
        return Some(Msg::HojaFormulaKey(ev.clone()));
    }
    if ev.modifiers.ctrl {
        if let Key::Character(c) = &ev.key {
            match c.to_lowercase().as_str() {
                "z" => return Some(if ev.modifiers.shift { Msg::HojaRedo } else { Msg::HojaUndo }),
                "y" => return Some(Msg::HojaRedo),
                "e" => return Some(Msg::HojaExportCsv),
                _ => {}
            }
        }
    }
    match &ev.key {
        Key::Named(NamedKey::Enter) => Some(Msg::HojaCommit),
        Key::Named(NamedKey::Escape) => Some(Msg::HojaCancel),
        Key::Named(NamedKey::F2) if !s.editing => Some(Msg::HojaEditStart),
        Key::Named(NamedKey::Delete) if !s.editing => Some(Msg::HojaClear),
        Key::Named(NamedKey::ArrowUp) if !s.editing => Some(Msg::HojaMove { dcol: 0, drow: -1 }),
        Key::Named(NamedKey::ArrowDown) if !s.editing => Some(Msg::HojaMove { dcol: 0, drow: 1 }),
        Key::Named(NamedKey::ArrowLeft) if !s.editing => Some(Msg::HojaMove { dcol: -1, drow: 0 }),
        Key::Named(NamedKey::ArrowRight) if !s.editing => Some(Msg::HojaMove { dcol: 1, drow: 0 }),
        Key::Named(NamedKey::Tab) => Some(Msg::HojaMove { dcol: 1, drow: 0 }),
        _ => {
            // Una tecla productiva sin modificadores entra a edición.
            if !s.editing && !ev.modifiers.alt && !ev.modifiers.meta && !ev.modifiers.ctrl {
                if let Some(text) = ev.text.as_ref() {
                    if !text.is_empty() && text.chars().all(|c| !c.is_control()) {
                        return Some(Msg::HojaEditWith(text.clone()));
                    }
                }
            }
            Some(Msg::HojaFormulaKey(ev.clone()))
        }
    }
}

/// Inspector de la hoja: la celda activa, su raw y su valor calculado.
pub(crate) fn inspector(s: &SheetView, theme: &Theme) -> Vec<View<Msg>> {
    let raw = s.wb.raw(s.sel).unwrap_or("(vacía)").to_string();
    vec![
        text_line(format!("Celda  {}", s.sel), 13.0, theme.fg_text),
        text_line(format!("raw:  {raw}"), 11.5, theme.fg_muted),
        text_line(format!("valor:  {}", s.wb.formatted(s.sel)), 11.5, theme.fg_muted),
    ]
}
