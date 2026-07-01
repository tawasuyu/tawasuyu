//! `llimphi-widget-calendar` — vista mensual del calendario.
//!
//! Una grilla 7 (columnas = días de la semana) × 6 (filas = semanas)
//! mostrando los días del mes en foco, con header de mes/año + flechas de
//! navegación, fila de iniciales de día, día seleccionado y día actual
//! resaltados. El widget **no tiene estado propio**: el mes en foco, la
//! selección y la fecha "hoy" viven en el `Model` del caller; el widget
//! emite `Msg`s para navegar y para seleccionar.
//!
//! Base del **date-picker** (un `field` + un overlay que muestra este
//! calendar) y útil por sí solo para agendas, planning, ERP. La lógica
//! del calendario (días del mes, primer día de la semana, grilla 6×7) es
//! pública y testeable: ver [`days_in_month`], [`first_weekday`],
//! [`month_grid`].

#![forbid(unsafe_code)]

use chrono::{Datelike, NaiveDate, Weekday};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, FlexDirection, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::View;
use llimphi_theme::Theme;

/// Día de la semana con el que arranca la grilla. La mayor parte del mundo
/// usa `Monday`; EE. UU. usa `Sunday`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WeekStart {
    Monday,
    Sunday,
}

impl Default for WeekStart {
    fn default() -> Self {
        Self::Monday
    }
}

impl WeekStart {
    /// Posición (0..7) del `weekday` empezando por este `WeekStart`. Con
    /// `Monday`: `Mon=0, Tue=1, …, Sun=6`. Con `Sunday`: `Sun=0, Mon=1,
    /// …, Sat=6`.
    pub fn index(self, w: Weekday) -> u32 {
        let mon0 = w.num_days_from_monday();
        match self {
            Self::Monday => mon0,
            // Domingo (mon0=6) pasa a 0, lunes (0) a 1, …
            Self::Sunday => (mon0 + 1) % 7,
        }
    }

    /// Iniciales de los siete días en el orden que dicta este `WeekStart`,
    /// con `locale = "es"` (L/M/M/J/V/S/D para Monday-start). Suficiente
    /// para v1; un mapa por `Theme.locale` se agrega cuando alguna app lo
    /// pida.
    pub fn weekday_initials_es(self) -> [&'static str; 7] {
        match self {
            Self::Monday => ["L", "M", "M", "J", "V", "S", "D"],
            Self::Sunday => ["D", "L", "M", "M", "J", "V", "S"],
        }
    }
}

/// Cantidad de días del mes `month` (1..=12) del año `year`. Devuelve `0`
/// si `month` está fuera de rango (defensa contra `u32` arbitrario; un
/// caller correcto pasa 1..=12).
pub fn days_in_month(year: i32, month: u32) -> u32 {
    if !(1..=12).contains(&month) {
        return 0;
    }
    // Trick estándar: primer día del mes siguiente menos un día. Año
    // siguiente si month == 12. `from_ymd_opt` retorna `Some` siempre que
    // (year, month, day) sean válidos — para day=1 con month 1..=12 lo es.
    let (y2, m2) = if month == 12 {
        (year + 1, 1)
    } else {
        (year, month + 1)
    };
    let first_next = NaiveDate::from_ymd_opt(y2, m2, 1).expect("primer día válido");
    let first_this = NaiveDate::from_ymd_opt(year, month, 1).expect("primer día válido");
    first_next.signed_duration_since(first_this).num_days() as u32
}

/// Día de la semana del **primer día** del mes (1 del mes). Sólo útil para
/// quien quiera el `Weekday` crudo; la grilla usa [`first_weekday_index`].
pub fn first_weekday(year: i32, month: u32) -> Option<Weekday> {
    NaiveDate::from_ymd_opt(year, month, 1).map(|d| d.weekday())
}

/// Índice de columna (0..7) del primer día del mes según `WeekStart`. Es
/// la cantidad de celdas vacías ANTES del 1 en la primera fila de la
/// grilla. Devuelve `0` si el mes es inválido.
pub fn first_weekday_index(year: i32, month: u32, start: WeekStart) -> u32 {
    first_weekday(year, month).map(|w| start.index(w)).unwrap_or(0)
}

/// Grilla mensual de **6 filas × 7 columnas** con los días del mes. Las
/// celdas que caen antes del primer día o después del último día del mes
/// son `None`. Siempre seis filas: para meses cortos (28 días arrancando
/// en domingo bajo Monday-start, p. ej. febrero 2021) la última fila queda
/// vacía; mantener seis filas estables evita reflow al navegar entre
/// meses (toda la grilla mide igual). Si `month` es inválido devuelve seis
/// filas de siete `None`s.
pub fn month_grid(year: i32, month: u32, start: WeekStart) -> [[Option<u32>; 7]; 6] {
    let mut grid: [[Option<u32>; 7]; 6] = [[None; 7]; 6];
    let dim = days_in_month(year, month);
    if dim == 0 {
        return grid;
    }
    let offset = first_weekday_index(year, month, start) as usize;
    for day in 1..=dim {
        let cell = offset + (day as usize) - 1;
        let r = cell / 7;
        let c = cell % 7;
        if r < 6 {
            grid[r][c] = Some(day);
        }
        // Si r == 6 (no debería pasar para meses estándar: 31+6=37<42),
        // se ignora; la grilla de 6×7 = 42 celdas alcanza para el peor caso
        // (31 días con offset = 6 → última celda 36, fila 5).
    }
    grid
}

/// Avanza `(year, month)` un mes adelante (`delta = +1`) o atrás (`-1`),
/// wrappeando años. Helper para los botones `<`/`>` del header.
pub fn shift_month(year: i32, month: u32, delta: i32) -> (i32, u32) {
    let total = (year * 12 + month as i32 - 1) + delta;
    let y = total.div_euclid(12);
    let m = (total.rem_euclid(12) + 1) as u32;
    (y, m)
}

/// Paleta del calendario.
#[derive(Debug, Clone, Copy)]
pub struct CalendarPalette {
    /// Fondo del panel.
    pub bg: Color,
    /// Texto normal de los días del mes en foco.
    pub fg: Color,
    /// Texto de las iniciales de día / mes/año del header (atenuado).
    pub fg_muted: Color,
    /// Hover sobre una celda de día.
    pub hover_bg: Color,
    /// Fondo de la celda del día seleccionado.
    pub selected_bg: Color,
    /// Texto del día seleccionado (contraste contra `selected_bg`).
    pub selected_fg: Color,
    /// Color del borde de la celda "hoy" (sólo borde — el fondo sigue al
    /// hover/selected normal).
    pub today_border: Color,
}

impl CalendarPalette {
    pub fn from_theme(t: &Theme) -> Self {
        Self {
            bg: t.bg_panel,
            fg: t.fg_text,
            fg_muted: t.fg_muted,
            hover_bg: t.bg_row_hover,
            selected_bg: t.accent,
            // El texto sobre `accent` se elige por contraste — el theme no
            // expone un `fg_on_accent` dedicado, así que tomamos `bg_app`
            // (oscuro en dark theme, claro en light theme) que es el
            // inverso natural del `fg_text`.
            selected_fg: t.bg_app,
            today_border: t.accent,
        }
    }
}

impl Default for CalendarPalette {
    fn default() -> Self {
        Self::from_theme(&Theme::dark())
    }
}

/// Especificación del calendario para una vista mensual.
pub struct CalendarSpec<Msg> {
    /// Año del mes en foco.
    pub view_year: i32,
    /// Mes en foco (1..=12).
    pub view_month: u32,
    /// Día seleccionado (opcional; si cae en otro mes no se resalta).
    pub selected: Option<NaiveDate>,
    /// Fecha "hoy" (opcional; si cae en otro mes no se resalta). El caller
    /// la inyecta — el widget no toca el reloj para mantenerse puro y
    /// testeable.
    pub today: Option<NaiveDate>,
    /// Primer día de la semana en la grilla.
    pub week_start: WeekStart,
    pub palette: CalendarPalette,
    /// `Msg` para "seleccioné el día X (NaiveDate del mes en foco)".
    pub on_select: std::sync::Arc<dyn Fn(NaiveDate) -> Msg + Send + Sync>,
    /// `Msg` para "muévete al mes (year, month)" — disparado por `<` / `>`.
    pub on_view_change: std::sync::Arc<dyn Fn(i32, u32) -> Msg + Send + Sync>,
}

/// Nombre del mes en español (1..=12). Para v1; un map por `Theme.locale`
/// se agrega cuando alguna app lo pida.
fn month_name_es(m: u32) -> &'static str {
    match m {
        1 => "enero",
        2 => "febrero",
        3 => "marzo",
        4 => "abril",
        5 => "mayo",
        6 => "junio",
        7 => "julio",
        8 => "agosto",
        9 => "septiembre",
        10 => "octubre",
        11 => "noviembre",
        12 => "diciembre",
        _ => "—",
    }
}

const HEADER_H: f32 = 32.0;
const ROW_H: f32 = 32.0;
const CELL_W: f32 = 32.0;
const PAD: f32 = 8.0;

/// Vista del calendario. Devuelve un `View<Msg>` autocontenido (panel +
/// header + week labels + grilla 6×7), de ancho fijo `7 * CELL_W + 2 * PAD`.
pub fn calendar_view<Msg>(spec: CalendarSpec<Msg>) -> View<Msg>
where
    Msg: Clone + 'static,
{
    let CalendarSpec {
        view_year,
        view_month,
        selected,
        today,
        week_start,
        palette,
        on_select,
        on_view_change,
    } = spec;

    // Header: < Mes Año >
    let prev = {
        let f = on_view_change.clone();
        let (py, pm) = shift_month(view_year, view_month, -1);
        View::<Msg>::new(Style {
            size: Size { width: length(CELL_W), height: length(HEADER_H) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text("‹", 18.0, palette.fg_muted)
        .hover_fill(palette.hover_bg)
        .radius(6.0)
        .cursor(llimphi_ui::Cursor::Pointer)
        .on_click_at({
            let f = f.clone();
            move |_, _, _, _| Some(f(py, pm))
        })
    };
    let next = {
        let f = on_view_change.clone();
        let (ny, nm) = shift_month(view_year, view_month, 1);
        View::<Msg>::new(Style {
            size: Size { width: length(CELL_W), height: length(HEADER_H) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text("›", 18.0, palette.fg_muted)
        .hover_fill(palette.hover_bg)
        .radius(6.0)
        .cursor(llimphi_ui::Cursor::Pointer)
        .on_click_at(move |_, _, _, _| Some(f(ny, nm)))
    };
    let title = format!("{} {}", month_name_es(view_month), view_year);
    let title_box = View::<Msg>::new(Style {
        size: Size { width: percent(1.0_f32), height: length(HEADER_H) },
        flex_grow: 1.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text(title, 14.0, palette.fg);
    let header = View::<Msg>::new(Style {
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        size: Size { width: percent(1.0_f32), height: length(HEADER_H) },
        ..Default::default()
    })
    .children(vec![prev, title_box, next]);

    // Week labels (L M M J V S D o D L M M J V S).
    let mut week_cols: Vec<View<Msg>> = Vec::with_capacity(7);
    for label in week_start.weekday_initials_es() {
        week_cols.push(
            View::<Msg>::new(Style {
                size: Size { width: length(CELL_W), height: length(ROW_H * 0.75) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .text(label, 11.0, palette.fg_muted),
        );
    }
    let week_row = View::<Msg>::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: length(CELL_W * 7.0),
            height: length(ROW_H * 0.75),
        },
        ..Default::default()
    })
    .children(week_cols);

    // Grilla 6×7.
    let grid = month_grid(view_year, view_month, week_start);
    let mut grid_rows: Vec<View<Msg>> = Vec::with_capacity(6);
    for row in grid.iter() {
        let mut cells: Vec<View<Msg>> = Vec::with_capacity(7);
        for cell in row.iter() {
            let view: View<Msg> = match *cell {
                None => View::<Msg>::new(Style {
                    size: Size { width: length(CELL_W), height: length(ROW_H) },
                    ..Default::default()
                }),
                Some(day) => {
                    let date = NaiveDate::from_ymd_opt(view_year, view_month, day)
                        .expect("día válido del mes en foco");
                    let is_sel = selected == Some(date);
                    let is_today = today == Some(date);
                    let fg = if is_sel { palette.selected_fg } else { palette.fg };
                    let mut v = View::<Msg>::new(Style {
                        size: Size { width: length(CELL_W), height: length(ROW_H) },
                        align_items: Some(AlignItems::Center),
                        justify_content: Some(JustifyContent::Center),
                        ..Default::default()
                    })
                    .text(day.to_string(), 13.0, fg)
                    .radius(6.0)
                    .cursor(llimphi_ui::Cursor::Pointer);
                    if is_sel {
                        v = v.fill(palette.selected_bg);
                    } else {
                        v = v.hover_fill(palette.hover_bg);
                    }
                    if is_today {
                        v = v.border(1.0, palette.today_border);
                    }
                    let f = on_select.clone();
                    v = v.on_click_at(move |_, _, _, _| Some(f(date)));
                    v
                }
            };
            cells.push(view);
        }
        grid_rows.push(
            View::<Msg>::new(Style {
                flex_direction: FlexDirection::Row,
                size: Size { width: length(CELL_W * 7.0), height: length(ROW_H) },
                ..Default::default()
            })
            .children(cells),
        );
    }
    let grid_box = View::<Msg>::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(CELL_W * 7.0),
            height: length(ROW_H * 6.0),
        },
        ..Default::default()
    })
    .children(grid_rows);

    let total_w = CELL_W * 7.0 + PAD * 2.0;
    let total_h = HEADER_H + ROW_H * 0.75 + ROW_H * 6.0 + PAD * 2.0;
    View::<Msg>::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(total_w), height: length(total_h) },
        padding: llimphi_ui::llimphi_layout::taffy::Rect {
            top: length(PAD),
            bottom: length(PAD),
            left: length(PAD),
            right: length(PAD),
        },
        ..Default::default()
    })
    .fill(palette.bg)
    .radius(8.0)
    .children(vec![header, week_row, grid_box])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn days_in_month_meses_estandar() {
        assert_eq!(days_in_month(2026, 1), 31);
        assert_eq!(days_in_month(2026, 2), 28);
        assert_eq!(days_in_month(2024, 2), 29); // bisiesto
        assert_eq!(days_in_month(2026, 4), 30);
        assert_eq!(days_in_month(2026, 12), 31);
        // Fuera de rango → 0.
        assert_eq!(days_in_month(2026, 0), 0);
        assert_eq!(days_in_month(2026, 13), 0);
    }

    #[test]
    fn first_weekday_index_monday_y_sunday_start() {
        // 1 de enero 2026 cae JUEVES.
        // Con Monday-start: Mon=0, Tue=1, Wed=2, Thu=3 → index = 3.
        assert_eq!(first_weekday_index(2026, 1, WeekStart::Monday), 3);
        // Con Sunday-start: Sun=0, Mon=1, ..., Thu=4.
        assert_eq!(first_weekday_index(2026, 1, WeekStart::Sunday), 4);
        // 1 de febrero 2026 cae DOMINGO.
        // Monday-start: Sun=6.
        assert_eq!(first_weekday_index(2026, 2, WeekStart::Monday), 6);
        // Sunday-start: Sun=0.
        assert_eq!(first_weekday_index(2026, 2, WeekStart::Sunday), 0);
    }

    #[test]
    fn month_grid_enero_2026_monday_start() {
        // Enero 2026: 31 días, primer día jueves (index 3 con Monday-start).
        let g = month_grid(2026, 1, WeekStart::Monday);
        // Tres celdas vacías al inicio.
        assert_eq!(g[0][0], None);
        assert_eq!(g[0][1], None);
        assert_eq!(g[0][2], None);
        assert_eq!(g[0][3], Some(1)); // jueves 1
        assert_eq!(g[0][6], Some(4)); // domingo 4
        assert_eq!(g[1][0], Some(5)); // lunes 5
        // Último día: 31. (31 + 3 - 1) / 7 = 33/7 = 4, % 7 = 5 ⇒ fila 4, col 5.
        assert_eq!(g[4][5], Some(31));
        // Fila 5 (sexta) entera vacía.
        for c in g[5].iter() {
            assert_eq!(*c, None);
        }
    }

    #[test]
    fn month_grid_febrero_2026_28_dias_domingo_inicio() {
        // Febrero 2026: 28 días, primer día domingo (index 6 con Monday-start).
        let g = month_grid(2026, 2, WeekStart::Monday);
        // Primera fila: seis vacías + Some(1) en col 6 (domingo).
        for c in 0..6 {
            assert_eq!(g[0][c], None);
        }
        assert_eq!(g[0][6], Some(1));
        assert_eq!(g[1][0], Some(2)); // lunes 2
        // 28: (28+6-1)/7 = 33/7 = 4, %7 = 5 ⇒ fila 4 col 5.
        assert_eq!(g[4][5], Some(28));
        // Fila 5 entera vacía.
        for c in g[5].iter() {
            assert_eq!(*c, None);
        }
    }

    #[test]
    fn shift_month_wrappea_anios() {
        // Enero → diciembre del año previo.
        assert_eq!(shift_month(2026, 1, -1), (2025, 12));
        // Diciembre → enero del año siguiente.
        assert_eq!(shift_month(2026, 12, 1), (2027, 1));
        // Salto de varios meses.
        assert_eq!(shift_month(2026, 6, -8), (2025, 10));
        assert_eq!(shift_month(2026, 6, 18), (2027, 12));
    }

    #[test]
    fn week_start_index_consistente() {
        // Monday-start.
        assert_eq!(WeekStart::Monday.index(Weekday::Mon), 0);
        assert_eq!(WeekStart::Monday.index(Weekday::Sun), 6);
        // Sunday-start.
        assert_eq!(WeekStart::Sunday.index(Weekday::Sun), 0);
        assert_eq!(WeekStart::Sunday.index(Weekday::Mon), 1);
        assert_eq!(WeekStart::Sunday.index(Weekday::Sat), 6);
    }
}
