//! `nakui-sheet-llimphi` — UI mínima estilo Excel sobre Llimphi.
//!
//! Capas:
//!   - Cabecera con el título + última cell editada + estado.
//!   - Barra de fórmula (text-input single-line) que muestra el `raw`
//!     de la celda seleccionada. Enter aplica al Workbook; Esc revierte.
//!   - Grilla con headers de columna (A, B, ...) y de fila (1, 2, ...).
//!     Click sobre una celda la selecciona; flechas la mueven.
//!
//! No re-implementa el flujo Excel completo de edición *dentro* de la
//! celda — toda la edición pasa por la barra. Eso simplifica el caret
//! y deja transparente la diferencia entre "valor mostrado" (en la
//! grilla) y "fórmula real" (en la barra), que es exactamente lo que
//! quieres ver para entender el motor.

#![forbid(unsafe_code)]

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Rect, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{
    App, Handle, Key, KeyEvent, KeyState, NamedKey, View, WheelDelta,
};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use nakui_sheet::{CellRef, SheetValue, Workbook};

const VISIBLE_COLS: u32 = 8;
const VISIBLE_ROWS: u32 = 18;
const CELL_W: f32 = 110.0;
const CELL_H: f32 = 24.0;
const ROW_HEADER_W: f32 = 40.0;
const FORMULA_BAR_H: f32 = 36.0;
const TOP_HEADER_H: f32 = 30.0;
const STATUS_H: f32 = 24.0;

struct NakuiSheetApp;

#[derive(Clone)]
enum Msg {
    SelectCell(CellRef),
    Move(Dir),
    FormulaKey(KeyEvent),
    Commit,
    Cancel,
}

#[derive(Clone, Copy)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

struct Model {
    wb: Workbook,
    selected: CellRef,
    /// Texto vivo en la barra de fórmula. Se carga desde `wb.raw(selected)`
    /// cada vez que cambia la selección, y se aplica con Enter.
    bar: TextInputState,
    /// Mensaje en la barra de estado (último error o info). Vacío = ok.
    status: Status,
    theme: Theme,
}

#[derive(Default, Clone)]
struct Status {
    text: String,
    kind: StatusKind,
}

#[derive(Default, Clone, Copy, PartialEq)]
enum StatusKind {
    #[default]
    Info,
    Error,
}

impl App for NakuiSheetApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "Nakui Sheet"
    }

    fn initial_size() -> (u32, u32) {
        (1100, 640)
    }

    fn init(_h: &Handle<Self::Msg>) -> Self::Model {
        let mut wb = Workbook::new();
        seed(&mut wb);
        let selected = CellRef::new(0, 0);
        let mut bar = TextInputState::new();
        bar.set_text(wb.raw(selected).unwrap_or(""));
        Model {
            wb,
            selected,
            bar,
            status: Status::default(),
            theme: Theme::dark(),
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _h: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::SelectCell(cr) => {
                model.selected = cr;
                model.bar.set_text(model.wb.raw(cr).unwrap_or(""));
                model.status = Status::default();
            }
            Msg::Move(dir) => {
                let cr = move_cell(model.selected, dir);
                model.selected = cr;
                model.bar.set_text(model.wb.raw(cr).unwrap_or(""));
                model.status = Status::default();
            }
            Msg::FormulaKey(ev) => {
                model.bar.apply_key(&ev);
            }
            Msg::Commit => {
                let raw = model.bar.text();
                match model.wb.set_cell(model.selected, &raw) {
                    Ok(report) => {
                        model.status = Status {
                            text: format!(
                                "  {} celda(s) recomputada(s)  ·  WAL: {} eventos",
                                report.changed.len(),
                                model.wb.events().len()
                            ),
                            kind: StatusKind::Info,
                        };
                    }
                    Err(e) => {
                        model.status = Status {
                            text: format!("  ✗ {e}"),
                            kind: StatusKind::Error,
                        };
                    }
                }
            }
            Msg::Cancel => {
                // Esc revierte la barra al valor real de la celda.
                model
                    .bar
                    .set_text(model.wb.raw(model.selected).unwrap_or(""));
                model.status = Status::default();
            }
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let t = &model.theme;
        let title_bar = title_bar_view(t, model.selected);
        let formula_bar = formula_bar_view(t, &model.bar, model.selected);
        let grid = grid_view(t, &model.wb, model.selected);
        let status = status_bar_view(t, &model.status);

        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .fill(t.bg_app)
        .children(vec![title_bar, formula_bar, grid, status])
    }

    fn on_key(model: &Self::Model, ev: &KeyEvent) -> Option<Self::Msg> {
        if ev.state != KeyState::Pressed {
            return None;
        }
        match &ev.key {
            Key::Named(NamedKey::Enter) => Some(Msg::Commit),
            Key::Named(NamedKey::Escape) => Some(Msg::Cancel),
            // Flechas sin Shift navegan el grid; con Shift las cede al
            // text-input para edición/selección dentro de la barra.
            Key::Named(NamedKey::ArrowUp) if !ev.modifiers.shift => Some(Msg::Move(Dir::Up)),
            Key::Named(NamedKey::ArrowDown) if !ev.modifiers.shift => Some(Msg::Move(Dir::Down)),
            Key::Named(NamedKey::ArrowLeft) if !ev.modifiers.shift && !text_caret_can_move_left(&model.bar) => {
                Some(Msg::Move(Dir::Left))
            }
            Key::Named(NamedKey::ArrowRight) if !ev.modifiers.shift && !text_caret_can_move_right(&model.bar) => {
                Some(Msg::Move(Dir::Right))
            }
            Key::Named(NamedKey::Tab) => Some(Msg::Move(Dir::Right)),
            _ => Some(Msg::FormulaKey(ev.clone())),
        }
        // Nota: la heurística de flechas izq/der intenta mantener
        // "feel Excel": si el caret de la barra puede moverse dentro
        // del texto, la flecha edita; si está en el extremo, navega
        // la celda. Up/Down siempre navegan (no hay multilínea).
    }

    fn on_wheel(
        _model: &Self::Model,
        _delta: WheelDelta,
        _cursor: (f32, f32),
        _modifiers: llimphi_ui::Modifiers,
    ) -> Option<Self::Msg> {
        None
    }
}

fn text_caret_can_move_left(bar: &TextInputState) -> bool {
    bar.editor().cursor.caret.col > 0
}

fn text_caret_can_move_right(bar: &TextInputState) -> bool {
    let line = bar.editor().cursor.caret.line;
    let len = bar.editor().buffer.line_len_chars(line);
    bar.editor().cursor.caret.col < len
}

fn move_cell(cr: CellRef, dir: Dir) -> CellRef {
    let col = cr.col;
    let row = cr.row;
    match dir {
        Dir::Up => CellRef::new(col, row.saturating_sub(1)),
        Dir::Down => CellRef::new(col, row.saturating_add(1).min(VISIBLE_ROWS - 1)),
        Dir::Left => CellRef::new(col.saturating_sub(1), row),
        Dir::Right => CellRef::new(col.saturating_add(1).min(VISIBLE_COLS - 1), row),
    }
}

fn title_bar_view(t: &Theme, selected: CellRef) -> View<Msg> {
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
    .fill(t.bg_panel)
    .children(vec![View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        format!("nakui-sheet  ·  celda activa: {selected}"),
        13.0,
        t.fg_text,
        Alignment::Start,
    )])
}

fn formula_bar_view(t: &Theme, bar: &TextInputState, selected: CellRef) -> View<Msg> {
    let palette = TextInputPalette::from_theme(t);
    let label = View::new(Style {
        size: Size {
            width: length(60.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(t.bg_panel_alt)
    .text_aligned(selected.to_string(), 13.0, t.fg_text, Alignment::Center);

    let input = View::new(Style {
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
        flex_grow: 1.0,
        ..Default::default()
    })
    .children(vec![text_input_view(
        bar,
        "ingresa fórmula o valor",
        true,
        &palette,
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
    .fill(t.bg_app)
    .children(vec![label, input])
}

fn grid_view(t: &Theme, wb: &Workbook, selected: CellRef) -> View<Msg> {
    let mut rows: Vec<View<Msg>> = Vec::new();
    // Cabecera de columnas.
    rows.push(column_header_row(t));
    // Filas de datos.
    for r in 0..VISIBLE_ROWS {
        rows.push(data_row(t, wb, selected, r));
    }
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        flex_direction: FlexDirection::Column,
        flex_grow: 1.0,
        ..Default::default()
    })
    .fill(t.bg_app)
    .children(rows)
}

fn column_header_row(t: &Theme) -> View<Msg> {
    let mut cells: Vec<View<Msg>> = Vec::new();
    // Esquina vacía.
    cells.push(
        View::new(Style {
            size: Size {
                width: length(ROW_HEADER_W),
                height: length(CELL_H),
            },
            ..Default::default()
        })
        .fill(t.bg_panel_alt),
    );
    for c in 0..VISIBLE_COLS {
        cells.push(
            View::new(Style {
                size: Size {
                    width: length(CELL_W),
                    height: length(CELL_H),
                },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .fill(t.bg_panel)
            .text_aligned(CellRef::col_label(c), 12.0, t.fg_muted, Alignment::Center),
        );
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

fn data_row(t: &Theme, wb: &Workbook, selected: CellRef, row: u32) -> View<Msg> {
    let mut cells: Vec<View<Msg>> = Vec::new();
    // Cabecera de fila.
    cells.push(
        View::new(Style {
            size: Size {
                width: length(ROW_HEADER_W),
                height: length(CELL_H),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .fill(t.bg_panel)
        .text_aligned(format!("{}", row + 1), 12.0, t.fg_muted, Alignment::Center),
    );
    for c in 0..VISIBLE_COLS {
        let cr = CellRef::new(c, row);
        cells.push(cell_view(t, wb, selected, cr));
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

fn cell_view(t: &Theme, wb: &Workbook, selected: CellRef, cr: CellRef) -> View<Msg> {
    let is_sel = cr == selected;
    let value = wb.value(cr);
    let display = match &value {
        SheetValue::Empty => String::new(),
        _ => value.to_display_string(),
    };
    let is_error = matches!(value, SheetValue::Error(_));
    let is_text = matches!(value, SheetValue::Text(_));

    let bg = if is_sel { t.accent } else { t.bg_app };
    let fg = if is_sel {
        // Sobre accent queremos texto contrastante; usamos fg_text del
        // tema asumiendo dark + accent legible (los temas current lo son).
        t.fg_text
    } else if is_error {
        Color::from_rgba8(220, 90, 90, 255)
    } else {
        t.fg_text
    };
    let alignment = if is_text {
        Alignment::Start
    } else {
        Alignment::End
    };

    View::new(Style {
        size: Size {
            width: length(CELL_W),
            height: length(CELL_H),
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
    .hover_fill(t.bg_panel)
    .text_aligned(display, 12.5, fg, alignment)
    .on_click(Msg::SelectCell(cr))
}

fn status_bar_view(t: &Theme, status: &Status) -> View<Msg> {
    let bg = match status.kind {
        StatusKind::Info => t.bg_panel,
        StatusKind::Error => Color::from_rgba8(120, 40, 40, 255),
    };
    let fg = match status.kind {
        StatusKind::Info => t.fg_muted,
        StatusKind::Error => Color::from_rgba8(255, 220, 220, 255),
    };
    View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(STATUS_H),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(bg)
    .text_aligned(status.text.clone(), 12.0, fg, Alignment::Start)
}

fn seed(wb: &mut Workbook) {
    let rows = [
        ("A1", "Concepto"), ("B1", "Cant"), ("C1", "Unit"), ("D1", "Subtotal"), ("E1", "IVA"), ("F1", "TOTAL"),
        ("A2", "Café"),     ("B2", "5"),    ("C2", "20"),  ("D2", "=B2*C2"),    ("E2", "=D2*16%"), ("F2", "=SUM(D2:E5)"),
        ("A3", "Té"),       ("B3", "3"),    ("C3", "15"),  ("D3", "=B3*C3"),    ("E3", "=D3*16%"),
        ("A4", "Azúcar"),   ("B4", "2"),    ("C4", "10"),  ("D4", "=B4*C4"),    ("E4", "=D4*16%"),
    ];
    for (cell, raw) in rows {
        let _ = wb.set_cell(cell.parse().unwrap(), raw);
    }
    // Invariante declarado de fábrica para que el demo lo enseñe a la
    // primera edición que lo viole.
    let _ = wb.add_invariant("tope_total", "=F2<=500");
}

fn main() {
    llimphi_ui::run::<NakuiSheetApp>();
}
