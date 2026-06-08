//! `nakui-sheet-llimphi` — UI mínima estilo Excel sobre Llimphi.
//!
//! Capas:
//!   - Cabecera con el título + última cell editada + estado.
//!   - Barra de fórmula (text-input single-line) que muestra el `raw`
//!     de la celda seleccionada. Enter aplica al Workbook; Esc revierte.
//!   - Grilla con headers de columna (A, B, ...) y de fila (1, 2, ...).
//!     Click sobre una celda la selecciona; flechas la mueven.
//!   - Paneles inmovilizables (freeze panes): Ctrl+Shift+F ancla las
//!     filas por encima y columnas a la izquierda de la celda activa
//!     (toggle); se pintan siempre, el resto scrollea por detrás.
//!   - Tabla dinámica (pivot): Ctrl+Shift+P abre un overlay que agrupa
//!     las filas de la selección por una columna y agrega otra
//!     (SUMA/CONTAR/PROM/MÍN/MÁX). A/G/V/H ciclan función/grupo/valor/
//!     encabezado; Esc cierra.
//!
//! No re-implementa el flujo Excel completo de edición *dentro* de la
//! celda — toda la edición pasa por la barra. Eso simplifica el caret
//! y deja transparente la diferencia entre "valor mostrado" (en la
//! grilla) y "fórmula real" (en la barra), que es exactamente lo que
//! quieres ver para entender el motor.

#![forbid(unsafe_code)]

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, FlexDirection, Rect, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{
    App, Handle, Key, KeyEvent, KeyState, NamedKey, View, WheelDelta,
};
use llimphi_widget_context_menu::{
    context_menu_view, context_menu_view_ex, step_active, ContextMenuExtras, ContextMenuItem,
    ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_edit_menu::{self as editmenu, EditAction, EditFlags};
use llimphi_widget_menubar::{
    menubar_command_at, menubar_nav, menubar_overlay_animated, menubar_view, MenuBarSpec,
    DEFAULT_HEIGHT as MENU_H,
};
use llimphi_motion::{animate, motion, Tween};
use llimphi_clipboard::SystemClipboard;
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use nakui_sheet::{csv_io, CellFormat, CellRange, CellRef, ExportMode, SheetValue, Workbook};
// Motor de tabla dinámica (regla #2): `Agg`/`PivotState` y el cómputo viven
// en el core; acá sólo se construyen, rotan y pintan.
use nakui_sheet::pivot::{compute_pivot, pivot_col_label, Agg, PivotState};
use std::sync::Arc;

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
    /// Tinte sutil para celdas dentro del rango de selección
    /// (excepto la active, que es accent sólido). Brown-amber muy
    /// apagado — visible sobre el negro pero no rivaliza con la
    /// accent de la live cell.
    pub const SEL_RANGE_BG: Color = Color::from_rgba8(64, 44, 22, 255);
    /// Tinte de las celdas dentro de una banda inmovilizada (frozen
    /// pane). Azul-gris muy apagado — distinto del negro de las
    /// celdas normales para que el usuario vea de un vistazo qué
    /// filas/columnas quedaron ancladas, sin rivalizar con el accent.
    pub const FROZEN_BG: Color = Color::from_rgba8(20, 24, 34, 255);
    pub const ERROR: Color = Color::from_rgba8(232, 96, 96, 255);
    pub const ERROR_BG: Color = Color::from_rgba8(80, 24, 24, 255);
    pub const FG_PLACEHOLDER: Color = Color::from_rgba8(95, 100, 115, 255);
}

struct NakuiSheetApp;

#[derive(Clone)]
enum Msg {
    SelectCell(CellRef),
    Move(Dir),
    /// Como `Move`, pero NO colapsa el anchor — extiende el rango
    /// de selección desde el anchor actual. Lo dispara Shift+flecha.
    ExtendMove(Dir),
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
    /// Aplica un formato predefinido a la celda activa. Lo dispara
    /// Ctrl+Shift+1/4/5 — los atajos clásicos de Excel.
    ApplyFormat(CellFormat),
    /// Exporta la hoja entera a `./nakui-export.csv` (Ctrl+E).
    ExportCsv,
    /// Importa `./nakui-import.csv` a partir de A1 (Ctrl+I).
    ImportCsv,
    /// Borra el contenido de la celda activa (Delete / menú "Limpiar").
    ClearActive,
    /// Abre el menú contextual sobre la celda dada, en la posición de
    /// pantalla `(x, y)`. La selección se mueve a esa celda como
    /// efecto colateral — es lo que el usuario espera tras un
    /// right-click.
    OpenMenu { cell: CellRef, pos: (f32, f32) },
    /// Cierra el menú contextual sin elegir ninguna opción.
    CloseMenu,
    /// Mueve el item resaltado del menú (-1 = arriba, +1 = abajo).
    MenuStep(i32),
    /// Activa el item resaltado del menú actual (Enter).
    MenuActivateActive,
    /// Activa el item N-ésimo del menú (click directo).
    MenuPick(usize),
    /// Inmoviliza paneles tomando la celda activa como esquina: todas
    /// las filas por encima y las columnas a la izquierda quedan
    /// ancladas (igual que "Inmovilizar paneles" de Excel). En A1 es
    /// no-op. Lo dispara Ctrl+Shift+F y el menú contextual.
    FreezeAtSelection,
    /// Libera todos los paneles inmovilizados (vuelve a scroll total).
    Unfreeze,
    /// Abre la tabla dinámica (pivot) sobre la selección actual. Lo
    /// dispara Ctrl+Shift+P y el menú contextual.
    OpenPivot,
    /// Cierra el overlay de la tabla dinámica.
    ClosePivot,
    /// Rota la función de agregación del pivot (-1 / +1).
    PivotCycleAgg(i32),
    /// Mueve la columna de agrupación del pivot dentro del rango.
    PivotCycleGroup(i32),
    /// Mueve la columna de valor (agregada) del pivot dentro del rango.
    PivotCycleValue(i32),
    /// Conmuta si la primera fila del rango se trata como encabezado.
    PivotToggleHeader,
    /// Barra de menú principal: abrir/cerrar un menú raíz (`None` = cerrar).
    MenuBarOpen(Option<usize>),
    /// Comando elegido en el menú principal — se traduce al `Msg` real.
    MenuCommand(String),
    /// Right-click sobre la barra de fórmula → abre el menú de edición en
    /// `(x, y)` de ventana, operando sobre el `TextInputState` de la barra.
    EditMenuOpen(f32, f32),
    /// Acción elegida en el menú de edición de la barra de fórmula.
    EditMenuAction(EditAction),
    /// Cierra cualquier menú/overlay abierto (menú principal + edición).
    CloseMenus,
    /// Navegación por teclado en el dropdown del menú principal.
    MenuNav(i32),
    /// Ejecuta la fila activa del menú principal (Enter).
    MenuActivate,
    /// Tick de animación de los dropdowns (sólo re-render).
    MenuTick,
    /// Navegación por teclado en el menú de edición de la barra de fórmula.
    EditNav(i32),
    /// Ejecuta la fila activa del menú de edición (Enter).
    EditActivate,
}

#[derive(Clone, Copy)]
enum Dir {
    Up,
    Down,
    Left,
    Right,
}

// `Agg` y `PivotState` (el motor de tabla dinámica) viven en
// `nakui_sheet::pivot` (regla #2). Se importan vía `use nakui_sheet::pivot::*`
// más abajo; el frontend sólo conserva el render del overlay (ver `pivot.rs`).

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
    /// Estado del menú contextual abierto. `None` = sin menú.
    menu: Option<MenuState>,
    /// "Otro extremo" del rango de selección. La selección es el
    /// rectángulo `(anchor, selected)` normalizado. Con un click o
    /// flecha pelada, `anchor == selected` (selección de una sola
    /// celda). Shift+flecha mueve `selected` sin tocar `anchor`,
    /// extendiendo el rectángulo a lo Excel.
    anchor: CellRef,
    /// Cantidad de filas inmovilizadas (frozen panes). Las primeras
    /// `freeze_rows` filas (0..freeze_rows) se pintan SIEMPRE arriba,
    /// no importa el scroll. `0` = sin inmovilizar. Invariante:
    /// `viewport_row >= freeze_rows`.
    freeze_rows: u32,
    /// Cantidad de columnas inmovilizadas. Análogo a `freeze_rows`
    /// pero sobre el eje horizontal. Invariante: `viewport_col >=
    /// freeze_cols`.
    freeze_cols: u32,
    /// Tabla dinámica abierta como overlay. `None` = sin pivot.
    pivot: Option<PivotState>,
    /// Menú principal (barra superior): índice del menú raíz abierto.
    /// `None` = cerrado.
    menu_open: Option<usize>,
    /// Fila activa (teclado) del dropdown principal. `usize::MAX` = ninguna.
    menu_active: usize,
    /// Animación de aparición/swap del dropdown principal.
    menu_anim: Tween<f32>,
    /// Menú de edición contextual sobre la barra de fórmula: ancla
    /// `(x, y)` en coordenadas de ventana. `None` = cerrado.
    edit_menu: Option<(f32, f32)>,
    /// Fila activa (teclado) del menú de edición. `usize::MAX` = ninguna.
    edit_active: usize,
    /// Animación de aparición del menú de edición.
    edit_anim: Tween<f32>,
    /// Clipboard del sistema para las acciones del menú de edición de la
    /// barra de fórmula (cut/copy/paste de texto dentro del input). El
    /// copy/cut/paste de CELDAS sigue usando `arboard` aparte porque
    /// shifta fórmulas.
    clipboard: SystemClipboard,
}

/// Estado del menú contextual mientras está abierto.
#[derive(Clone)]
struct MenuState {
    /// Celda sobre la que se invocó. La selección ya se movió a
    /// esta celda al abrir el menú.
    cell: CellRef,
    /// Esquina top-left donde queremos renderizar el panel.
    pos: (f32, f32),
    /// Item resaltado por keyboard nav. `usize::MAX` = ninguno.
    active: usize,
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
            freeze_rows: 0,
            freeze_cols: 0,
            pivot: None,
            clipboard_origin: None,
            editing: false,
            menu: None,
            anchor: selected,
            menu_open: None,
            menu_active: usize::MAX,
            menu_anim: Tween::idle(1.0),
            edit_menu: None,
            edit_active: usize::MAX,
            edit_anim: Tween::idle(1.0),
            clipboard: SystemClipboard::new(),
        }
    }

    fn update(mut model: Self::Model, msg: Self::Msg, h: &Handle<Self::Msg>) -> Self::Model {
        match msg {
            Msg::SelectCell(cr) => {
                // Click externo cierra una edición en curso aplicando
                // lo que había en la barra — feel Excel.
                if model.editing {
                    commit_bar(&mut model);
                }
                model.selected = cr;
                model.anchor = cr;
                model.bar.set_text(model.wb.raw(cr).unwrap_or(""));
                model.status = selection_status(&model);
                ensure_visible(&mut model);
            }
            Msg::Move(dir) => {
                if model.editing {
                    commit_bar(&mut model);
                    model.editing = false;
                }
                let cr = move_cell(model.selected, dir);
                model.selected = cr;
                model.anchor = cr;
                model.bar.set_text(model.wb.raw(cr).unwrap_or(""));
                model.status = selection_status(&model);
                ensure_visible(&mut model);
            }
            Msg::ExtendMove(dir) => {
                // Shift+flecha: extiende sin tocar anchor. Si estaba
                // editando, salimos primero (mover ≠ editar).
                if model.editing {
                    commit_bar(&mut model);
                    model.editing = false;
                }
                let cr = move_cell(model.selected, dir);
                model.selected = cr;
                // bar mantiene el raw de la cell activa (la "live"
                // cell del rango), igual que Excel.
                model.bar.set_text(model.wb.raw(cr).unwrap_or(""));
                model.status = selection_status(&model);
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
            Msg::ExportCsv => {
                let path = std::path::Path::new("./nakui-export.csv");
                let result = std::fs::File::create(path)
                    .map_err(|e| format!("crear {path:?}: {e}"))
                    .and_then(|f| {
                        csv_io::export_csv(&model.wb, ExportMode::Raw, f)
                            .map_err(|e| format!("export: {e}"))
                    });
                model.status = match result {
                    Ok(()) => Status {
                        text: format!("  ⇪ exportado a {}", path.display()),
                        kind: StatusKind::Info,
                    },
                    Err(e) => Status {
                        text: format!("  ✗ export: {e}"),
                        kind: StatusKind::Error,
                    },
                };
            }
            Msg::ImportCsv => {
                let path = std::path::Path::new("./nakui-import.csv");
                let result = std::fs::File::open(path)
                    .map_err(|e| format!("abrir {path:?}: {e}"))
                    .and_then(|f| {
                        csv_io::import_csv(&mut model.wb, f)
                            .map_err(|e| format!("import: {e}"))
                    });
                model.status = match result {
                    Ok(n) => {
                        model.bar.set_text(model.wb.raw(model.selected).unwrap_or(""));
                        Status {
                            text: format!("  ⇩ importadas {n} celdas desde {}", path.display()),
                            kind: StatusKind::Info,
                        }
                    }
                    Err(e) => Status {
                        text: format!("  ✗ import: {e}"),
                        kind: StatusKind::Error,
                    },
                };
            }
            Msg::ApplyFormat(fmt) => match model.wb.set_format(model.selected, fmt.clone()) {
                Ok(_) => {
                    model.status = Status {
                        text: format!("  ▦ formato aplicado a {}", model.selected),
                        kind: StatusKind::Info,
                    };
                }
                Err(e) => {
                    model.status = Status {
                        text: format!("  ✗ formato: {e}"),
                        kind: StatusKind::Error,
                    };
                }
            },
            Msg::Scroll { drow, dcol } => {
                model.viewport_row =
                    apply_scroll_axis(model.viewport_row, drow);
                model.viewport_col =
                    apply_scroll_axis(model.viewport_col, dcol);
                // El viewport scrolleable nunca puede invadir la banda
                // inmovilizada — esas filas/columnas viven aparte.
                clamp_viewport_to_freeze(&mut model);
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
            Msg::ClearActive => {
                let cr = model.selected;
                match model.wb.clear_cell(cr) {
                    Ok(_) => {
                        model.bar.set_text("");
                        model.status = Status {
                            text: format!("  ␡ limpiada: {cr}"),
                            kind: StatusKind::Info,
                        };
                    }
                    Err(e) => {
                        model.status = Status {
                            text: format!("  ✗ limpiar: {e}"),
                            kind: StatusKind::Error,
                        };
                    }
                }
            }
            Msg::OpenMenu { cell, pos } => {
                if model.editing {
                    commit_bar(&mut model);
                    model.editing = false;
                }
                model.selected = cell;
                model.bar.set_text(model.wb.raw(cell).unwrap_or(""));
                model.menu = Some(MenuState {
                    cell,
                    pos,
                    active: usize::MAX,
                });
            }
            Msg::CloseMenu => {
                model.menu = None;
            }
            Msg::MenuStep(dir) => {
                if let Some(menu) = model.menu.as_mut() {
                    let items = menu_items(&model.wb, model.clipboard_origin.is_some(), model.freeze_rows > 0 || model.freeze_cols > 0);
                    menu.active = step_active(&items, menu.active, dir);
                }
            }
            Msg::MenuActivateActive => {
                if let Some(menu) = model.menu.clone() {
                    model.menu = None;
                    if menu.active != usize::MAX {
                        if let Some(inner) = menu_item_msg(menu.active) {
                            h.dispatch(inner);
                        }
                    }
                }
            }
            Msg::MenuPick(idx) => {
                model.menu = None;
                if let Some(inner) = menu_item_msg(idx) {
                    h.dispatch(inner);
                }
            }
            Msg::FreezeAtSelection => {
                // Inmovilizamos por encima/izquierda de la celda
                // activa. Dejamos siempre algunas ranuras de scroll
                // (no tiene sentido anclar toda la grilla visible).
                let max_fr = VISIBLE_ROWS.saturating_sub(3);
                let max_fc = VISIBLE_COLS.saturating_sub(2);
                model.freeze_rows = model.selected.row.min(max_fr);
                model.freeze_cols = model.selected.col.min(max_fc);
                clamp_viewport_to_freeze(&mut model);
                model.status = if model.freeze_rows == 0 && model.freeze_cols == 0 {
                    Status {
                        text: "  ❄ nada que inmovilizar (A1) — movete a la esquina deseada".into(),
                        kind: StatusKind::Info,
                    }
                } else {
                    Status {
                        text: format!(
                            "  ❄ paneles inmovilizados: {} fila(s) · {} columna(s)",
                            model.freeze_rows, model.freeze_cols
                        ),
                        kind: StatusKind::Info,
                    }
                };
            }
            Msg::Unfreeze => {
                model.freeze_rows = 0;
                model.freeze_cols = 0;
                model.status = Status {
                    text: "  ❄ paneles liberados".into(),
                    kind: StatusKind::Info,
                };
            }
            Msg::OpenPivot => {
                let r = selection_rect(&model);
                if r.cell_count() < 2 {
                    model.status = Status {
                        text: "  Σ pivot: seleccioná primero un rango (≥2 celdas)".into(),
                        kind: StatusKind::Error,
                    };
                } else {
                    if model.editing {
                        commit_bar(&mut model);
                        model.editing = false;
                    }
                    model.menu = None;
                    // Encabezado si el rango tiene más de una fila; agrupar
                    // por la primera columna y agregar la última (o la
                    // misma si el rango es de una sola columna).
                    let header_row = r.end.row > r.start.row;
                    let group_col = r.start.col;
                    let value_col = if r.end.col > r.start.col {
                        r.end.col
                    } else {
                        r.start.col
                    };
                    model.pivot = Some(PivotState {
                        source: r,
                        group_col,
                        value_col,
                        agg: Agg::Sum,
                        header_row,
                    });
                    model.status = Status::default();
                }
            }
            Msg::ClosePivot => {
                model.pivot = None;
            }
            Msg::PivotCycleAgg(dir) => {
                if let Some(p) = model.pivot.as_mut() {
                    p.agg = p.agg.cycle(dir);
                }
            }
            Msg::PivotCycleGroup(dir) => {
                if let Some(p) = model.pivot.as_mut() {
                    p.group_col = cycle_col(p.group_col, &p.source, dir);
                }
            }
            Msg::PivotCycleValue(dir) => {
                if let Some(p) = model.pivot.as_mut() {
                    p.value_col = cycle_col(p.value_col, &p.source, dir);
                }
            }
            Msg::PivotToggleHeader => {
                if let Some(p) = model.pivot.as_mut() {
                    p.header_row = !p.header_row;
                }
            }
            Msg::MenuBarOpen(idx) => {
                model.menu_open = idx;
                model.menu_active = usize::MAX;
                // Abrir el menú principal cierra cualquier otro overlay
                // local (menú de edición, menú de celda).
                model.edit_menu = None;
                if idx.is_some() {
                    model.menu_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                    animate(h, motion::FAST, || Msg::MenuTick);
                }
            }
            Msg::MenuNav(dir) => {
                if let Some(mi) = model.menu_open {
                    let menu = app_menu(&model);
                    model.menu_active = menubar_nav(&menu, mi, model.menu_active, dir);
                }
            }
            Msg::MenuActivate => {
                if let Some(mi) = model.menu_open {
                    let menu = app_menu(&model);
                    if let Some(cmd) = menubar_command_at(&menu, mi, model.menu_active) {
                        model.menu_open = None;
                        model.menu_active = usize::MAX;
                        if let Some(inner) = menubar_command_msg(&model, &cmd) {
                            h.dispatch(inner);
                        }
                    }
                }
            }
            Msg::MenuTick => {}
            Msg::MenuCommand(cmd) => {
                model.menu_open = None;
                model.menu_active = usize::MAX;
                if let Some(inner) = menubar_command_msg(&model, &cmd) {
                    h.dispatch(inner);
                }
            }
            Msg::EditMenuOpen(x, y) => {
                // Sólo tiene sentido el menú de edición sobre la barra de
                // fórmula. Lo anclamos en la posición de ventana del click.
                model.menu_open = None;
                model.menu = None;
                model.edit_menu = Some((x, y));
                model.edit_active = usize::MAX;
                model.edit_anim = Tween::new(0.0, 1.0, motion::FAST, motion::ease_out_cubic);
                animate(h, motion::FAST, || Msg::MenuTick);
            }
            Msg::EditNav(dir) => {
                let flags = EditFlags::from_editor(model.bar.editor(), model.bar.is_masked());
                model.edit_active = editmenu::edit_menu_step(flags, model.edit_active, dir);
            }
            Msg::EditActivate => {
                let flags = EditFlags::from_editor(model.bar.editor(), model.bar.is_masked());
                if let Some(action) = editmenu::edit_menu_action_at(flags, model.edit_active) {
                    return NakuiSheetApp::update(model, Msg::EditMenuAction(action), h);
                }
            }
            Msg::EditMenuAction(action) => {
                model.edit_menu = None;
                model.edit_active = usize::MAX;
                let _ = editmenu::apply(model.bar.editor_mut(), action, &mut model.clipboard);
                // Si el menú de edición tocó el texto de la barra estando
                // en modo edición, lo dejamos vivo — el commit pasa con
                // Enter como siempre. No tocamos el Workbook acá.
            }
            Msg::CloseMenus => {
                model.menu_open = None;
                model.menu_active = usize::MAX;
                model.edit_menu = None;
                model.edit_active = usize::MAX;
            }
        }
        model
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        // 1) Menú de edición sobre la barra de fórmula: máxima prioridad
        //    (es lo que el usuario acaba de invocar con right-click).
        if let Some((x, y)) = model.edit_menu {
            let flags = EditFlags::from_editor(model.bar.editor(), model.bar.is_masked());
            let (w, h) = Self::initial_size();
            let mut spec = editmenu::edit_context_menu(
                (x, y),
                (w as f32, h as f32),
                &model.theme,
                flags,
                Msg::EditMenuAction,
                Msg::CloseMenus,
            );
            spec.active = model.edit_active;
            return Some(context_menu_view_ex(
                spec,
                ContextMenuExtras {
                    appear: model.edit_anim.value(),
                    ..Default::default()
                },
            ));
        }
        // 2) Dropdown del menú principal (barra superior).
        if model.menu_open.is_some() {
            let menu = app_menu(model);
            return menubar_overlay_animated(
                &menubar_spec(&menu, model, &model.theme),
                model.menu_active,
                model.menu_anim.value(),
            );
        }
        // 3) El pivot: modal de pantalla completa.
        if let Some(pivot) = model.pivot.as_ref() {
            return Some(pivot_overlay_view(&model.wb, pivot));
        }
        // 4) Menú contextual de celda (el que ya existía).
        let menu = model.menu.as_ref()?;
        let items = menu_items(&model.wb, model.clipboard_origin.is_some(), model.freeze_rows > 0 || model.freeze_cols > 0);
        let mut palette = ContextMenuPalette::from_theme(&model.theme);
        // El theme dark-sheet vive en `palette` (módulo local). El
        // accent es naranja tawasuyu; eso ya viene del theme. Aclaramos
        // los slots para que el menú pegue con el panel negro y la
        // grilla sutil:
        palette.bg_panel = self::palette::BG_PANEL;
        palette.fg_text = self::palette::FG_TEXT;
        palette.fg_active = self::palette::ACCENT_FG;
        palette.bg_active = self::palette::ACCENT;
        palette.fg_shortcut = self::palette::FG_MUTED;
        palette.fg_disabled = self::palette::FG_PLACEHOLDER;
        palette.fg_header = self::palette::FG_MUTED;
        palette.border = self::palette::GRID_LINE;
        palette.separator = self::palette::GRID_LINE;
        palette.accent = self::palette::ACCENT;
        // Scrim casi imperceptible — apenas un velo. La idea es no
        // ocultar la hoja; el menú flota y la grilla sigue viéndose
        // detrás, sólo un poco amortiguada.
        palette.scrim = Color::from_rgba8(0, 0, 0, 90);

        let header = Some(menu.cell.to_string());
        let viewport_w = (VISIBLE_COLS as f32 * CELL_W) + ROW_HEADER_W;
        let viewport_h = TOP_HEADER_H
            + FORMULA_BAR_H
            + (VISIBLE_ROWS as f32 * CELL_H)
            + STATUS_H
            + CELL_H /* header de columnas */;
        // Anclaje: esquina inferior izquierda de la celda invocadora.
        // Si la celda está fuera del viewport (raro porque el menú
        // se invoca por click sobre una celda visible), el clamping
        // del widget la trae al borde más cercano.
        let col_local = screen_col_index(model, menu.cell.col) as f32;
        let row_local = screen_row_index(model, menu.cell.row) as f32;
        let anchor_x = ROW_HEADER_W + col_local * CELL_W + 6.0;
        // Y: top-of-window + barra título + barra fórmula + header de
        // columnas + filas previas + altura de la propia celda → menú
        // aparece JUSTO debajo de la celda, sin solaparla.
        let anchor_y = TOP_HEADER_H
            + FORMULA_BAR_H
            + CELL_H /* header de columnas */
            + row_local * CELL_H
            + CELL_H;
        let _ = menu.pos;

        let spec = ContextMenuSpec {
            anchor: (anchor_x, anchor_y),
            viewport: (viewport_w, viewport_h),
            header,
            items,
            active: menu.active,
            on_pick: Arc::new(|i| Msg::MenuPick(i)),
            on_dismiss: Msg::CloseMenu,
            palette,
        };
        Some(context_menu_view(spec))
    }

    fn view(model: &Self::Model) -> View<Self::Msg> {
        let t = &model.theme;
        let menu = app_menu(model);
        let menubar = menubar_view(&menubar_spec(&menu, model, t));
        let title_bar =
            title_bar_view(model.selected, model.freeze_rows, model.freeze_cols);
        let formula_bar = formula_bar_view(t, &model.bar, model.selected);
        let grid = grid_view(
            &model.wb,
            model.selected,
            model.viewport_row,
            model.viewport_col,
            model.editing,
            &model.bar,
            model,
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
        .children(vec![menubar, title_bar, formula_bar, grid, status])
    }

    fn on_key(model: &Self::Model, ev: &KeyEvent) -> Option<Self::Msg> {
        if ev.state != KeyState::Pressed {
            return None;
        }
        // Menú principal abierto: ←/→ cambian de menú raíz (con wrap),
        // ↑/↓ navegan la fila, Enter ejecuta, Esc cierra. Cualquier otra
        // tecla cierra (feel estándar). No cae a navegación de grilla.
        if let Some(mi) = model.menu_open {
            let n = app_menu(model).menus.len().max(1);
            return Some(match &ev.key {
                Key::Named(NamedKey::Escape) => Msg::CloseMenus,
                Key::Named(NamedKey::ArrowLeft) => Msg::MenuBarOpen(Some((mi + n - 1) % n)),
                Key::Named(NamedKey::ArrowRight) => Msg::MenuBarOpen(Some((mi + 1) % n)),
                Key::Named(NamedKey::ArrowDown) => Msg::MenuNav(1),
                Key::Named(NamedKey::ArrowUp) => Msg::MenuNav(-1),
                Key::Named(NamedKey::Enter) => Msg::MenuActivate,
                _ => Msg::CloseMenus,
            });
        }
        // Menú de edición de la barra de fórmula abierto: ↑/↓ navegan,
        // Enter ejecuta, Esc cierra.
        if model.edit_menu.is_some() {
            return Some(match &ev.key {
                Key::Named(NamedKey::Escape) => Msg::CloseMenus,
                Key::Named(NamedKey::ArrowDown) => Msg::EditNav(1),
                Key::Named(NamedKey::ArrowUp) => Msg::EditNav(-1),
                Key::Named(NamedKey::Enter) => Msg::EditActivate,
                _ => Msg::CloseMenus,
            });
        }
        // Si el pivot está abierto, las teclas controlan el modal:
        // Esc cierra, ←/→ rotan la función, A/G/V ciclan
        // función/grupo/valor, H conmuta encabezado.
        if model.pivot.is_some() {
            return match &ev.key {
                Key::Named(NamedKey::Escape) => Some(Msg::ClosePivot),
                Key::Named(NamedKey::ArrowLeft) => Some(Msg::PivotCycleAgg(-1)),
                Key::Named(NamedKey::ArrowRight) => Some(Msg::PivotCycleAgg(1)),
                Key::Character(s) => match s.to_lowercase().as_str() {
                    "a" => Some(Msg::PivotCycleAgg(1)),
                    "g" => Some(Msg::PivotCycleGroup(1)),
                    "v" => Some(Msg::PivotCycleValue(1)),
                    "h" => Some(Msg::PivotToggleHeader),
                    _ => None,
                },
                _ => None,
            };
        }
        // Si el menú contextual está abierto, todas las teclas se
        // interpretan en su contexto. Flechas mueven, Enter activa,
        // Esc cierra. Cualquier otra tecla cierra también — feel
        // estándar de menús.
        if model.menu.is_some() {
            return match &ev.key {
                Key::Named(NamedKey::ArrowUp) => Some(Msg::MenuStep(-1)),
                Key::Named(NamedKey::ArrowDown) => Some(Msg::MenuStep(1)),
                Key::Named(NamedKey::Enter) => Some(Msg::MenuActivateActive),
                Key::Named(NamedKey::Escape) => Some(Msg::CloseMenu),
                _ => Some(Msg::CloseMenu),
            };
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
                    "e" => return Some(Msg::ExportCsv),
                    "i" => return Some(Msg::ImportCsv),
                    _ => {}
                }
                // Atajos Ctrl+Shift+N de formato. En distintos
                // layouts el caracter producido por la tecla "1"
                // con shift puede ser "!", "¡", etc. — chequeamos
                // contra ambos.
                if ev.modifiers.shift {
                    let lower = s.to_lowercase();
                    // Ctrl+Shift+F: toggle de inmovilizar paneles. Si ya
                    // hay banda anclada la libera; si no, ancla en la
                    // celda activa — feel "Inmovilizar/Movilizar" de Excel.
                    if lower == "f" {
                        return Some(
                            if model.freeze_rows > 0 || model.freeze_cols > 0 {
                                Msg::Unfreeze
                            } else {
                                Msg::FreezeAtSelection
                            },
                        );
                    }
                    // Ctrl+Shift+P: tabla dinámica sobre la selección.
                    if lower == "p" {
                        return Some(Msg::OpenPivot);
                    }
                    if lower == "1" || lower == "!" {
                        return Some(Msg::ApplyFormat(CellFormat::Number {
                            decimals: 2,
                        }));
                    }
                    if lower == "4" || lower == "$" {
                        return Some(Msg::ApplyFormat(CellFormat::Currency {
                            symbol: "$".into(),
                            decimals: 2,
                        }));
                    }
                    if lower == "5" || lower == "%" {
                        return Some(Msg::ApplyFormat(CellFormat::Percent {
                            decimals: 0,
                        }));
                    }
                    if lower == "0" || lower == ")" {
                        // Ctrl+Shift+0: vuelve a General (sin formato).
                        return Some(Msg::ApplyFormat(CellFormat::General));
                    }
                }
            }
        }
        match &ev.key {
            Key::Named(NamedKey::Enter) => Some(Msg::Commit),
            Key::Named(NamedKey::Escape) => Some(Msg::Cancel),
            Key::Named(NamedKey::F2) => Some(Msg::StartEditExisting),
            // Delete: limpia el contenido de la celda activa cuando NO
            // se está editando. (Adentro de la barra ya sirve para
            // borrar carácter por carácter — viaja por FormulaKey.)
            Key::Named(NamedKey::Delete) if !model.editing => Some(Msg::ClearActive),
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
            // Shift+flechas: extienden la selección. Solo aplica
            // FUERA de edición — dentro de la barra, Shift+flecha
            // sigue siendo selección de texto (cae al FormulaKey).
            Key::Named(NamedKey::ArrowUp) if ev.modifiers.shift && !model.editing => {
                Some(Msg::ExtendMove(Dir::Up))
            }
            Key::Named(NamedKey::ArrowDown) if ev.modifiers.shift && !model.editing => {
                Some(Msg::ExtendMove(Dir::Down))
            }
            Key::Named(NamedKey::ArrowLeft) if ev.modifiers.shift && !model.editing => {
                Some(Msg::ExtendMove(Dir::Left))
            }
            Key::Named(NamedKey::ArrowRight) if ev.modifiers.shift && !model.editing => {
                Some(Msg::ExtendMove(Dir::Right))
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

/// Arma el `MenuBarSpec` compartido por `menubar_view` y `menubar_overlay`.
fn menubar_spec<'a>(
    menu: &'a app_bus::AppMenu,
    model: &Model,
    theme: &'a Theme,
) -> MenuBarSpec<'a, Msg> {
    let (w, h) = NakuiSheetApp::initial_size();
    MenuBarSpec {
        menu,
        open: model.menu_open,
        theme,
        viewport: (w as f32, h as f32),
        height: MENU_H,
        on_open: Arc::new(Msg::MenuBarOpen),
        on_command: Arc::new(|c: &str| Msg::MenuCommand(c.to_string())),
    }
}

// --- Submódulos del bin: lógica de selección/scroll, pivot y vistas.
// Tipos+consts viven en root (campos privados visibles a descendientes).
// Free-fns pub(crate) re-exportadas para que impl App las llame bare. ---
mod logic;
mod pivot;
mod view;

use logic::*;
use pivot::*;
use view::*;

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
    rimay_localize::init();
    let cfg = wawa_config::WawaConfig::load();
    let _ = rimay_localize::set_locale(&cfg.lang);
    llimphi_ui::run::<NakuiSheetApp>();
}
