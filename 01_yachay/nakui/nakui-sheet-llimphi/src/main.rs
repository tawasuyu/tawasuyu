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

const VISIBLE_COLS: u32 = 12;
const VISIBLE_ROWS: u32 = 25;
const CELL_W: f32 = 110.0;
const CELL_H: f32 = 24.0;
const ROW_HEADER_W: f32 = 52.0;
const FORMULA_BAR_H: f32 = 36.0;
const TOP_HEADER_H: f32 = 30.0;
const STATUS_H: f32 = 24.0;
/// Cuánto avanza el viewport por cada "línea" de wheel. Las apps
/// modernas tienden a 3 líneas por tick (mismo factor que GTK/macOS).
const WHEEL_LINES: f32 = 3.0;
/// Margen de seguridad: cuando la selección se acerca al borde
/// visible, ajustamos el viewport para que siempre quede al menos
/// una celda de "respiración" alrededor.
const SCROLL_MARGIN_ROWS: u32 = 1;
const SCROLL_MARGIN_COLS: u32 = 1;

/// Paleta dark-sheet — fondo casi negro con cuadrícula sutil. Los
/// colores se eligen para que la grilla se vea NÍTIDA pero no
/// agresiva: las líneas de borde son 1px en gris oscuro,
/// suficientemente claras para guiar el ojo, suficientemente
/// apagadas para no competir con los valores de las celdas.
mod palette {
    use llimphi_ui::llimphi_raster::peniko::Color;

    pub const BG_APP: Color = Color::from_rgba8(8, 8, 10, 255);
    pub const BG_PANEL: Color = Color::from_rgba8(18, 18, 22, 255);
    pub const BG_PANEL_ALT: Color = Color::from_rgba8(24, 24, 28, 255);
    pub const BG_CELL: Color = Color::from_rgba8(12, 12, 14, 255);
    pub const BG_CELL_HOVER: Color = Color::from_rgba8(22, 22, 28, 255);
    pub const BG_HEADER: Color = Color::from_rgba8(28, 28, 34, 255);
    pub const GRID_LINE: Color = Color::from_rgba8(42, 42, 50, 255);
    pub const FG_TEXT: Color = Color::from_rgba8(232, 232, 235, 255);
    pub const FG_MUTED: Color = Color::from_rgba8(135, 138, 150, 255);
    pub const FG_HEADER: Color = Color::from_rgba8(170, 175, 188, 255);
    pub const ACCENT: Color = Color::from_rgba8(255, 140, 32, 255);
    pub const ACCENT_FG: Color = Color::from_rgba8(20, 14, 6, 255);
    pub const ERROR: Color = Color::from_rgba8(232, 96, 96, 255);
    pub const ERROR_BG: Color = Color::from_rgba8(80, 24, 24, 255);
    pub const FG_PLACEHOLDER: Color = Color::from_rgba8(95, 100, 115, 255);
}

struct NakuiSheetApp;

#[derive(Clone)]
enum Msg {
    SelectCell(CellRef),
    Move(Dir),
    FormulaKey(KeyEvent),
    Commit,
    Cancel,
    /// Desplaza el viewport. Filas positivas = scroll hacia abajo
    /// (la celda B5 sube en pantalla, llegan al fondo nuevas filas).
    Scroll { drow: i32, dcol: i32 },
    Undo,
    Redo,
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
    /// Esquina superior izquierda del viewport visible. El render
    /// pinta `VISIBLE_ROWS × VISIBLE_COLS` celdas a partir de aquí.
    viewport_row: u32,
    viewport_col: u32,
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
            theme: dark_sheet_theme(),
            viewport_row: 0,
            viewport_col: 0,
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _h: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::SelectCell(cr) => {
                model.selected = cr;
                model.bar.set_text(model.wb.raw(cr).unwrap_or(""));
                model.status = Status::default();
                ensure_visible(&mut model);
            }
            Msg::Move(dir) => {
                let cr = move_cell(model.selected, dir);
                model.selected = cr;
                model.bar.set_text(model.wb.raw(cr).unwrap_or(""));
                model.status = Status::default();
                ensure_visible(&mut model);
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
            Msg::Scroll { drow, dcol } => {
                model.viewport_row =
                    apply_scroll_axis(model.viewport_row, drow);
                model.viewport_col =
                    apply_scroll_axis(model.viewport_col, dcol);
            }
            Msg::Undo => match model.wb.undo() {
                Ok(Some(_)) => {
                    model.bar.set_text(model.wb.raw(model.selected).unwrap_or(""));
                    model.status = Status {
                        text: format!(
                            "  ↶ undo  ·  applied {} / {} eventos",
                            applied_count(&model.wb),
                            model.wb.events().len()
                        ),
                        kind: StatusKind::Info,
                    };
                }
                Ok(None) => {
                    model.status = Status {
                        text: "  nada que deshacer".into(),
                        kind: StatusKind::Info,
                    };
                }
                Err(e) => {
                    model.status = Status {
                        text: format!("  ✗ undo: {e}"),
                        kind: StatusKind::Error,
                    };
                }
            },
            Msg::Redo => match model.wb.redo() {
                Ok(Some(_)) => {
                    model.bar.set_text(model.wb.raw(model.selected).unwrap_or(""));
                    model.status = Status {
                        text: format!(
                            "  ↷ redo  ·  applied {} / {} eventos",
                            applied_count(&model.wb),
                            model.wb.events().len()
                        ),
                        kind: StatusKind::Info,
                    };
                }
                Ok(None) => {
                    model.status = Status {
                        text: "  nada que rehacer".into(),
                        kind: StatusKind::Info,
                    };
                }
                Err(e) => {
                    model.status = Status {
                        text: format!("  ✗ redo: {e}"),
                        kind: StatusKind::Error,
                    };
                }
            },
        }
        model
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let t = &model.theme;
        let title_bar = title_bar_view(model.selected);
        let formula_bar = formula_bar_view(t, &model.bar, model.selected);
        let grid = grid_view(
            &model.wb,
            model.selected,
            model.viewport_row,
            model.viewport_col,
        );
        let status = status_bar_view(&model.status);

        View::new(Style {
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            flex_direction: FlexDirection::Column,
            ..Default::default()
        })
        .fill(palette::BG_APP)
        .children(vec![title_bar, formula_bar, grid, status])
    }

    fn on_key(model: &Self::Model, ev: &KeyEvent) -> Option<Self::Msg> {
        if ev.state != KeyState::Pressed {
            return None;
        }
        // Atajos con Ctrl: undo/redo. Tienen prioridad sobre cualquier
        // otra interpretación de la tecla.
        if ev.modifiers.ctrl {
            if let Key::Character(s) = &ev.key {
                let lc = s.to_lowercase();
                if lc == "z" {
                    return Some(if ev.modifiers.shift {
                        Msg::Redo
                    } else {
                        Msg::Undo
                    });
                }
                if lc == "y" {
                    return Some(Msg::Redo);
                }
            }
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
        delta: WheelDelta,
        _cursor: (f32, f32),
        modifiers: llimphi_ui::Modifiers,
    ) -> Option<Self::Msg> {
        // Convención CSS de llimphi: delta.y positivo = scroll hacia
        // abajo. Multiplico por WHEEL_LINES para que cada tick mueva
        // varias filas — comportamiento esperado en apps de tabla.
        let drow = (delta.y * WHEEL_LINES).round() as i32;
        let dcol = (delta.x * WHEEL_LINES).round() as i32;
        // Shift+wheel convierte el scroll vertical en horizontal —
        // mismo gesto que GTK/Excel.
        let (drow, dcol) = if modifiers.shift {
            (0, drow.max(dcol))
        } else {
            (drow, dcol)
        };
        if drow == 0 && dcol == 0 {
            None
        } else {
            Some(Msg::Scroll { drow, dcol })
        }
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
    // Sin clamp a VISIBLE_* — la hoja es virtualmente ilimitada;
    // el viewport sigue a la selección vía `ensure_visible`.
    match dir {
        Dir::Up => CellRef::new(col, row.saturating_sub(1)),
        Dir::Down => CellRef::new(col, row.saturating_add(1)),
        Dir::Left => CellRef::new(col.saturating_sub(1), row),
        Dir::Right => CellRef::new(col.saturating_add(1), row),
    }
}

fn applied_count(wb: &Workbook) -> usize {
    wb.applied_count()
}

fn apply_scroll_axis(viewport: u32, delta: i32) -> u32 {
    if delta >= 0 {
        viewport.saturating_add(delta as u32)
    } else {
        viewport.saturating_sub((-delta) as u32)
    }
}

/// Mantiene la celda seleccionada dentro del viewport con un margen
/// de seguridad. Si la celda salió por arriba/izquierda, el viewport
/// se acerca; si salió por abajo/derecha, el viewport avanza lo
/// justo para volver a verla más el margen.
fn ensure_visible(model: &mut Model) {
    let sel = model.selected;
    // Vertical
    let v_top = model.viewport_row;
    let v_bot = model.viewport_row + VISIBLE_ROWS;
    if sel.row < v_top + SCROLL_MARGIN_ROWS {
        model.viewport_row = sel.row.saturating_sub(SCROLL_MARGIN_ROWS);
    } else if sel.row + SCROLL_MARGIN_ROWS >= v_bot {
        model.viewport_row = sel.row + SCROLL_MARGIN_ROWS + 1 - VISIBLE_ROWS;
    }
    // Horizontal
    let h_left = model.viewport_col;
    let h_right = model.viewport_col + VISIBLE_COLS;
    if sel.col < h_left + SCROLL_MARGIN_COLS {
        model.viewport_col = sel.col.saturating_sub(SCROLL_MARGIN_COLS);
    } else if sel.col + SCROLL_MARGIN_COLS >= h_right {
        model.viewport_col = sel.col + SCROLL_MARGIN_COLS + 1 - VISIBLE_COLS;
    }
}

fn title_bar_view(selected: CellRef) -> View<Msg> {
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
        format!("nakui-sheet  ·  celda activa: {selected}"),
        13.0,
        palette::FG_TEXT,
        Alignment::Start,
    )])
}

fn formula_bar_view(t: &Theme, bar: &TextInputState, selected: CellRef) -> View<Msg> {
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

fn grid_view(
    wb: &Workbook,
    selected: CellRef,
    viewport_row: u32,
    viewport_col: u32,
) -> View<Msg> {
    let mut rows: Vec<View<Msg>> = Vec::new();
    // Cabecera de columnas: muestra los labels A, B, C... empezando
    // desde la columna del viewport.
    rows.push(column_header_row(viewport_col));
    // Filas de datos. Cada r local mapea a row = viewport_row + r.
    for r in 0..VISIBLE_ROWS {
        let abs_row = viewport_row + r;
        rows.push(data_row(wb, selected, abs_row, viewport_col));
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
fn bordered_cell(
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

fn column_header_row(viewport_col: u32) -> View<Msg> {
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
    for c in 0..VISIBLE_COLS {
        let abs_col = viewport_col + c;
        cells.push(bordered_cell(
            CELL_W,
            CELL_H,
            palette::BG_HEADER,
            None,
            palette::FG_HEADER,
            CellRef::col_label(abs_col),
            Alignment::Center,
            None,
        ));
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

fn data_row(wb: &Workbook, selected: CellRef, row: u32, viewport_col: u32) -> View<Msg> {
    let is_active_row = row == selected.row;
    let mut cells: Vec<View<Msg>> = Vec::new();
    // Cabecera de fila — accent suave si la fila contiene la celda activa.
    let header_bg = if is_active_row {
        palette::BG_PANEL_ALT
    } else {
        palette::BG_HEADER
    };
    let header_fg = if is_active_row {
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
    for c in 0..VISIBLE_COLS {
        let abs_col = viewport_col + c;
        let cr = CellRef::new(abs_col, row);
        cells.push(cell_view(wb, selected, cr));
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

fn cell_view(wb: &Workbook, selected: CellRef, cr: CellRef) -> View<Msg> {
    let is_sel = cr == selected;
    let value = wb.value(cr);
    let display = match &value {
        SheetValue::Empty => String::new(),
        _ => value.to_display_string(),
    };
    let is_error = matches!(value, SheetValue::Error(_));
    let is_text = matches!(value, SheetValue::Text(_));

    let bg = if is_sel {
        palette::ACCENT
    } else if is_error {
        palette::ERROR_BG
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

    bordered_cell(
        CELL_W,
        CELL_H,
        bg,
        if is_sel { None } else { Some(palette::BG_CELL_HOVER) },
        fg,
        display,
        alignment,
        Some(Msg::SelectCell(cr)),
    )
}

fn status_bar_view(status: &Status) -> View<Msg> {
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

/// Theme custom: `Theme::dark()` con overrides para que `text-input`
/// (que se construye desde un Theme) use nuestra paleta dark-sheet.
fn dark_sheet_theme() -> Theme {
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
