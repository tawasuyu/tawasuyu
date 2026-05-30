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
use llimphi_widget_context_menu::{
    context_menu_view, step_active, ContextMenuItem, ContextMenuPalette, ContextMenuSpec,
};
use llimphi_widget_text_input::{text_input_view, TextInputPalette, TextInputState};
use nakui_sheet::{csv_io, CellFormat, CellRange, CellRef, ExportMode, SheetValue, Workbook};
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
            clipboard_origin: None,
            editing: false,
            menu: None,
            anchor: selected,
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
        }
        model
    }

    fn view_overlay(model: &Self::Model) -> Option<View<Self::Msg>> {
        let menu = model.menu.as_ref()?;
        let items = menu_items(&model.wb, model.clipboard_origin.is_some(), model.freeze_rows > 0 || model.freeze_cols > 0);
        let mut palette = ContextMenuPalette::from_theme(&model.theme);
        // El theme dark-sheet vive en `palette` (módulo local). El
        // accent es naranja gioser; eso ya viene del theme. Aclaramos
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
        .children(vec![title_bar, formula_bar, grid, status])
    }

    fn on_key(model: &Self::Model, ev: &KeyEvent) -> Option<Self::Msg> {
        if ev.state != KeyState::Pressed {
            return None;
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

/// Rectángulo de selección actual normalizado (top-left + bottom-right).
fn selection_rect(model: &Model) -> CellRange {
    CellRange::new(model.anchor, model.selected)
}

fn selection_is_single(model: &Model) -> bool {
    model.anchor == model.selected
}

/// Status descriptivo de la selección: una sola celda → vacío
/// (volvemos al estado neutro); un rango → "Sel: A1:C5 · 15 celdas
/// · suma 234.5" si hay numéricos.
fn selection_status(model: &Model) -> Status {
    if selection_is_single(model) {
        return Status::default();
    }
    let r = selection_rect(model);
    let count = r.cell_count();
    let mut sum = rust_decimal::Decimal::ZERO;
    let mut num_count = 0u32;
    for cr in r.iter() {
        if let SheetValue::Number(n) = model.wb.value(cr) {
            sum += n;
            num_count += 1;
        }
    }
    let text = if num_count == 0 {
        format!("  Sel: {} · {} celdas", r, count)
    } else {
        let avg = sum / rust_decimal::Decimal::from(num_count as i64);
        format!(
            "  Sel: {} · {} celdas · suma {} · prom {}",
            r,
            count,
            sum.normalize(),
            avg.normalize()
        )
    };
    Status {
        text,
        kind: StatusKind::Info,
    }
}

fn cell_in_selection(model: &Model, cr: CellRef) -> bool {
    if selection_is_single(model) {
        cr == model.selected
    } else {
        let r = selection_rect(model);
        cr.col >= r.start.col
            && cr.col <= r.end.col
            && cr.row >= r.start.row
            && cr.row <= r.end.row
    }
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

/// Índice de columna *en pantalla* (0 = primera columna tras el row
/// header) de una columna absoluta, teniendo en cuenta la banda
/// inmovilizada. Las columnas frozen ocupan las primeras `freeze_cols`
/// ranuras; el resto se mide desde el viewport scrolleable.
fn screen_col_index(model: &Model, col: u32) -> u32 {
    if col < model.freeze_cols {
        col
    } else {
        model.freeze_cols + col.saturating_sub(model.viewport_col)
    }
}

/// Análogo a [`screen_col_index`] sobre el eje de filas.
fn screen_row_index(model: &Model, row: u32) -> u32 {
    if row < model.freeze_rows {
        row
    } else {
        model.freeze_rows + row.saturating_sub(model.viewport_row)
    }
}

/// Empuja el viewport scrolleable de vuelta a respetar la banda
/// inmovilizada. Las filas/columnas `< freeze_*` se pintan aparte y
/// SIEMPRE; el área que scrollea arranca recién en `freeze_*`.
fn clamp_viewport_to_freeze(model: &mut Model) {
    model.viewport_row = model.viewport_row.max(model.freeze_rows);
    model.viewport_col = model.viewport_col.max(model.freeze_cols);
}

/// Mantiene la celda seleccionada dentro del viewport con un margen
/// de seguridad. Si la celda salió por arriba/izquierda, el viewport
/// se acerca; si salió por abajo/derecha, el viewport avanza lo
/// justo para volver a verla más el margen. Las celdas que caen
/// dentro de una banda inmovilizada están siempre a la vista, así que
/// no fuerzan ningún scroll en ese eje.
fn ensure_visible(model: &mut Model) {
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
fn menu_items(
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
    ]
}

/// Traduce un índice del menú a su Msg-equivalente. `None` para
/// separators o índices sin acción. Es la fuente de verdad para qué
/// hace cada fila del menú.
fn menu_item_msg(idx: usize) -> Option<Msg> {
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
        _ => None,
    }
}

fn title_bar_view(selected: CellRef, freeze_rows: u32, freeze_cols: u32) -> View<Msg> {
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

fn column_header_row(viewport_col: u32, freeze_cols: u32) -> View<Msg> {
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

fn data_row(
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

fn cell_view(wb: &Workbook, selected: CellRef, cr: CellRef, model: &Model) -> View<Msg> {
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
