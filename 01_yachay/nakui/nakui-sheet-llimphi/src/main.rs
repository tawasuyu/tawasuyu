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
    Copy,
    Cut,
    Paste,
    /// Entra a modo edición preservando el raw actual (F2).
    StartEditExisting,
    /// Entra a modo edición SUSTITUYENDO el raw por la tecla
    /// tipeada — comportamiento natural cuando empezás a escribir
    /// sobre una celda no-editando.
    StartEditWith(String),
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
    /// Origen del último copy/cut interno: `(raw, source_cell)`. Si
    /// al pegar el clipboard del sistema sigue conteniendo
    /// exactamente ese mismo raw, sabemos que es un paste "Nakui →
    /// Nakui" y aplicamos shift de fórmula. Si difiere (el user copió
    /// algo de otro lado), el paste es literal.
    clipboard_origin: Option<(String, CellRef)>,
    /// `true` cuando el usuario está editando la celda activa
    /// dentro de la grilla (F2 o tipeando una letra). El text-input
    /// se renderiza encima de la celda en vez del valor estático,
    /// y las flechas commitean+mueven en vez de navegar.
    editing: bool,
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
            clipboard_origin: None,
            editing: false,
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, _h: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::SelectCell(cr) => {
                // Click externo cierra una edición en curso aplicando
                // lo que había en la barra — feel Excel.
                if model.editing {
                    commit_bar(&mut model);
                }
                model.selected = cr;
                model.bar.set_text(model.wb.raw(cr).unwrap_or(""));
                model.status = Status::default();
                ensure_visible(&mut model);
            }
            Msg::Move(dir) => {
                if model.editing {
                    commit_bar(&mut model);
                    model.editing = false;
                }
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
                commit_bar(&mut model);
                model.editing = false;
            }
            Msg::Cancel => {
                // Esc revierte la barra al valor real de la celda y
                // sale de edición.
                model
                    .bar
                    .set_text(model.wb.raw(model.selected).unwrap_or(""));
                model.editing = false;
                model.status = Status::default();
            }
            Msg::StartEditExisting => {
                model.editing = true;
                // bar ya tiene el raw cargado por SelectCell; nada más
                // que hacer salvo confirmar el modo.
            }
            Msg::StartEditWith(first_char) => {
                model.editing = true;
                model.bar.set_text(first_char);
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
            Msg::Copy => {
                let raw = model.wb.raw(model.selected).unwrap_or("").to_string();
                match arboard::Clipboard::new()
                    .and_then(|mut cb| cb.set_text(raw.clone()))
                {
                    Ok(()) => {
                        model.clipboard_origin = Some((raw, model.selected));
                        model.status = Status {
                            text: format!("  ⧉ copiado: {}", model.selected),
                            kind: StatusKind::Info,
                        };
                    }
                    Err(e) => {
                        model.status = Status {
                            text: format!("  ✗ clipboard: {e}"),
                            kind: StatusKind::Error,
                        };
                    }
                }
            }
            Msg::Cut => {
                let raw = model.wb.raw(model.selected).unwrap_or("").to_string();
                match arboard::Clipboard::new()
                    .and_then(|mut cb| cb.set_text(raw.clone()))
                {
                    Ok(()) => {
                        model.clipboard_origin = Some((raw, model.selected));
                        // Cut = copy + clear de la fuente.
                        let _ = model.wb.clear_cell(model.selected);
                        model.bar.set_text("");
                        model.status = Status {
                            text: format!("  ✂ cortado: {}", model.selected),
                            kind: StatusKind::Info,
                        };
                    }
                    Err(e) => {
                        model.status = Status {
                            text: format!("  ✗ clipboard: {e}"),
                            kind: StatusKind::Error,
                        };
                    }
                }
            }
            Msg::Paste => {
                model.status = paste_into(&mut model.wb, model.selected, &model.clipboard_origin);
                // Tras pegar, recargo la barra de fórmula con el nuevo
                // raw de la celda destino.
                model.bar.set_text(model.wb.raw(model.selected).unwrap_or(""));
            }
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
            model.editing,
            &model.bar,
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
                match s.to_lowercase().as_str() {
                    "z" => {
                        return Some(if ev.modifiers.shift {
                            Msg::Redo
                        } else {
                            Msg::Undo
                        });
                    }
                    "y" => return Some(Msg::Redo),
                    "c" => return Some(Msg::Copy),
                    "x" => return Some(Msg::Cut),
                    "v" => return Some(Msg::Paste),
                    _ => {}
                }
            }
        }
        match &ev.key {
            Key::Named(NamedKey::Enter) => Some(Msg::Commit),
            Key::Named(NamedKey::Escape) => Some(Msg::Cancel),
            Key::Named(NamedKey::F2) => Some(Msg::StartEditExisting),
            // Flechas: si NO está editando, navegan SIEMPRE. Si está
            // editando, navegan SOLO si el caret está en el extremo
            // de la barra (Up/Down) o si Shift no se está usando
            // (Left/Right ya consideran el caret). Esto reproduce el
            // feel Excel: flechas sin Shift dentro de una celda en
            // edición commiteán y mueven; con Shift seleccionan
            // dentro del texto.
            Key::Named(NamedKey::ArrowUp) if !ev.modifiers.shift => Some(Msg::Move(Dir::Up)),
            Key::Named(NamedKey::ArrowDown) if !ev.modifiers.shift => Some(Msg::Move(Dir::Down)),
            Key::Named(NamedKey::ArrowLeft)
                if !ev.modifiers.shift
                    && (!model.editing || !text_caret_can_move_left(&model.bar)) =>
            {
                Some(Msg::Move(Dir::Left))
            }
            Key::Named(NamedKey::ArrowRight)
                if !ev.modifiers.shift
                    && (!model.editing || !text_caret_can_move_right(&model.bar)) =>
            {
                Some(Msg::Move(Dir::Right))
            }
            Key::Named(NamedKey::Tab) => Some(Msg::Move(Dir::Right)),
            _ => {
                // Si no está editando y llega una tecla productiva
                // (con texto sin modificadores), entra a edición
                // reemplazando el contenido — feel Excel: tipeás y
                // la celda muestra lo que estás tipeando.
                if !model.editing
                    && !ev.modifiers.alt
                    && !ev.modifiers.meta
                    && !ev.modifiers.ctrl
                {
                    if let Some(text) = ev.text.as_ref() {
                        if !text.is_empty()
                            && text.chars().all(|c| !c.is_control())
                        {
                            return Some(Msg::StartEditWith(text.clone()));
                        }
                    }
                }
                Some(Msg::FormulaKey(ev.clone()))
            }
        }
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

/// Aplica el contenido actual de la barra a la celda seleccionada
/// y actualiza el status. No toca `editing` — el caller decide qué
/// hacer con ese flag (Commit lo desactiva; Move lo desactiva tras
/// commit; SelectCell lo desactiva tras commit).
fn commit_bar(model: &mut Model) {
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

/// Paste con shift-de-fórmulas si la fuente coincide con
/// `clipboard_origin`. Si el clipboard del sistema cambió (el
/// usuario copió texto de otra app), pega literal.
fn paste_into(
    wb: &mut Workbook,
    dest: CellRef,
    origin: &Option<(String, CellRef)>,
) -> Status {
    let payload = match arboard::Clipboard::new().and_then(|mut cb| cb.get_text()) {
        Ok(t) => t,
        Err(e) => {
            return Status {
                text: format!("  ✗ clipboard vacío: {e}"),
                kind: StatusKind::Error,
            };
        }
    };
    // Caso 1: paste interno coherente con un copy/cut previo →
    // shift de fórmulas. La fuente y el raw deben coincidir
    // exactamente; si el user cambió la celda fuente entremedias,
    // el origin queda desactualizado y caemos al paste literal.
    if let Some((origin_raw, origin_cell)) = origin {
        if *origin_raw == payload {
            let drow = dest.row as i32 - origin_cell.row as i32;
            let dcol = dest.col as i32 - origin_cell.col as i32;
            let new_raw = shift_raw(&payload, drow, dcol);
            return match wb.set_cell(dest, &new_raw) {
                Ok(_) => Status {
                    text: format!("  ⇲ pegado en {dest} (shift {drow:+},{dcol:+})"),
                    kind: StatusKind::Info,
                },
                Err(e) => Status {
                    text: format!("  ✗ paste: {e}"),
                    kind: StatusKind::Error,
                },
            };
        }
    }
    // Caso 2: paste literal — clipboard de otra app o cambió de
    // contenido. Lo metemos tal cual.
    match wb.set_cell(dest, &payload) {
        Ok(_) => Status {
            text: format!("  ⇲ pegado en {dest}"),
            kind: StatusKind::Info,
        },
        Err(e) => Status {
            text: format!("  ✗ paste: {e}"),
            kind: StatusKind::Error,
        },
    }
}

/// Shifta el raw como lo haría un fill: parse → shift → render. Si
/// el raw no es una fórmula (no empieza con `=`) o no parsea, lo
/// devolvemos sin tocar — un literal numérico o texto no se shifta.
fn shift_raw(raw: &str, drow: i32, dcol: i32) -> String {
    let stripped = match raw.strip_prefix('=') {
        Some(s) => s,
        None => return raw.to_string(),
    };
    match nakui_sheet::formula::compile(stripped) {
        Ok(expr) => {
            let shifted = nakui_sheet::formula::shift(&expr, drow, dcol);
            format!("={}", nakui_sheet::formula::render(&shifted))
        }
        Err(_) => raw.to_string(),
    }
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
    editing: bool,
    bar: &TextInputState,
) -> View<Msg> {
    let mut rows: Vec<View<Msg>> = Vec::new();
    // Cabecera de columnas: muestra los labels A, B, C... empezando
    // desde la columna del viewport.
    rows.push(column_header_row(viewport_col));
    // Filas de datos. Cada r local mapea a row = viewport_row + r.
    for r in 0..VISIBLE_ROWS {
        let abs_row = viewport_row + r;
        rows.push(data_row(wb, selected, abs_row, viewport_col, editing, bar));
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

fn data_row(
    wb: &Workbook,
    selected: CellRef,
    row: u32,
    viewport_col: u32,
    editing: bool,
    bar: &TextInputState,
) -> View<Msg> {
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
        if editing && cr == selected {
            cells.push(editing_cell_view(bar));
        } else {
            cells.push(cell_view(wb, selected, cr));
        }
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
fn editing_cell_view(bar: &TextInputState) -> View<Msg> {
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
