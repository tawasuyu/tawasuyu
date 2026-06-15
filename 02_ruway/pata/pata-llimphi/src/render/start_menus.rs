//! Variantes alternativas del menú de inicio.
//!
//! Conviven con el menú classic (panel a la izquierda con buscador + lista
//! filtrable, definido en [`super`]). El usuario alterna estilos con
//! click-derecho sobre el botón de inicio.
//!
//! - [`start_menu_xp_overlay`] — réplica sobria del menú de Windows XP:
//!   banda azul superior con avatar + nombre, dos columnas (pinned ⟂
//!   "todos los programas"), franja inferior con dos acciones.
//! - [`start_menu_gnome_overlay`] — overlay full-screen estilo GNOME
//!   Activities: scrim oscuro, buscador grande arriba, grid central de
//!   tiles 96×96 con label.

use app_bus::AppEntry;
use llimphi_theme::{elevation, motion, radius, Theme};
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, percent, Dimension, FlexDirection, Size, Style},
    AlignItems, JustifyContent, Position, Rect as TaffyRect,
};
use llimphi_ui::llimphi_raster::peniko::{color::AlphaColor, Color};
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::{Shadow, View};
use llimphi_widget_scroll::{clamp_offset, scroll_y, ScrollPalette};

use crate::Msg;

use super::menu_filtered;

/// Los cuatro cuadrantes del workspace. Una app cuya categoría sea uno de
/// ellos es de la **suite tawasuyu** (la sembró `default_entries`), no una
/// `.desktop` ajena del sistema. Sirve para separar lo propio (curado, con
/// glifo) de lo descubierto (basura variada del sistema).
pub(super) const CUADRANTES: [&str; 4] = ["unanchay", "yachay", "ruway", "ukupacha"];

/// `true` si la app pertenece a la suite (categoría = cuadrante).
pub(super) fn es_suite(a: &AppEntry) -> bool {
    a.category
        .as_deref()
        .is_some_and(|c| CUADRANTES.contains(&c))
}

/// El glifo de la app, o —si no tiene uno corto— su inicial en mayúscula
/// como chip. Mucho mejor que la flechita `▸` genérica que dejaba todas las
/// `.desktop` del sistema (sin glifo) indistinguibles.
pub(super) fn icono_o_inicial(a: &AppEntry) -> String {
    if let Some(g) = a.icon.as_deref().filter(|s| s.chars().count() <= 2) {
        // Algunos glifos (emoji, dingbats que la fuente del sistema no trae)
        // salen como tofu / «NO GLYPH». Para esos caemos a la inicial.
        if g.chars().all(glifo_renderiza) {
            return g.to_string();
        }
    }
    inicial(&a.label)
}

/// La inicial alfanumérica de un rótulo, en mayúscula (chip de fallback).
fn inicial(label: &str) -> String {
    label
        .chars()
        .find(|c| c.is_alphanumeric())
        .map(|c| c.to_uppercase().to_string())
        .unwrap_or_else(|| "•".to_string())
}

/// `true` si el carácter está en la fuente del sistema (DejaVu). Heurística:
/// excluye emoji (plano suplementario) y los dingbats confirmados ausentes que
/// salían como tofu en el menú (lápiz/pluma ✎✒, cuadrados rellenos ▤▦, mapa 🗺).
fn glifo_renderiza(c: char) -> bool {
    let u = c as u32;
    if u >= 0x1F000 {
        return false; // emoji / símbolos suplementarios
    }
    !matches!(u, 0x270E | 0x2712 | 0x25A4 | 0x25A6)
}

// =====================================================================
// Estilo XP — banda azul, dos columnas, footer rojo
// =====================================================================

/// Ancho del panel XP. Tomado del Bliss original: ~380 px (más esbelto
/// que el Classic Win10 pero más ancho que el Classic Win95).
const XP_W: f32 = 420.0;
/// Alto del banner superior con avatar + nombre.
const XP_HEADER_H: f32 = 60.0;
/// Alto del footer "Cerrar sesión / Apagar".
const XP_FOOTER_H: f32 = 44.0;
/// Alto de cada fila de app.
const XP_ROW_H: f32 = 30.0;

/// Overlay del menú XP. Pintado encima del rect del frame, abajo de la
/// barra. El scrim cierra al click. La animación de entrada es
/// `animated_inout` con `motion::FAST` aplicada al panel.
/// El **cuerpo** del menú estilo Windows XP: tarjeta con banda de usuario, dos
/// columnas (pin de inicio + todos los programas) y footer. Es el control
/// reutilizable — el *chrome* (scrim/posición en winit, barra en layer-shell)
/// lo pone el caller. `avail_h` es el alto disponible para la tarjeta.
pub(super) fn xp_body(
    apps: &[AppEntry],
    query: &str,
    offset: f32,
    avail_h: f32,
    theme: &Theme,
) -> View<Msg> {
    let panel_h = avail_h.clamp(420.0, 720.0);
    let panel_w = XP_W;
    let cols_h = panel_h - XP_HEADER_H - XP_FOOTER_H;

    let header = xp_header(theme);
    // Dos columnas: pinned = la suite tawasuyu (categoría = cuadrante) como
    // favoritos curados; programs = el resto (apps del sistema), scrolleable y
    // filtrable. Antes pinned era el head alfabético → se llenaba de basura
    // del sistema (Avahi, lstopo…) en vez de las apps propias.
    let matches = menu_filtered(apps, query);
    let (pinned, programs): (Vec<&AppEntry>, Vec<&AppEntry>) =
        matches.iter().copied().partition(|a| es_suite(a));
    let pinned_col = xp_column("Pin de inicio", pinned, cols_h, theme, true);
    let programs_col =
        xp_column_scrolling("Todos los programas", programs, offset, cols_h, theme);
    let columns = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(cols_h),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .children(vec![pinned_col, programs_col]);
    let footer = xp_footer(theme);

    let (sh_a, sh_blur, sh_dy) = elevation::E4;
    let shadow = Shadow {
        color: Color::from_rgba8(0, 0, 0, sh_a),
        blur: sh_blur,
        dx: 0.0,
        dy: sh_dy,
        spread: 0.0,
    };
    // La tarjeta va EN FLUJO (sin position absoluta): así el caller la coloca —
    // en un scrim (winit) o sobre la barra (layer-shell).
    View::new(Style {
        size: Size {
            width: length(panel_w),
            height: length(panel_h),
        },
        flex_direction: FlexDirection::Column,
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(Color::from_rgba8(245, 246, 250, 255))
    .radius(radius::LG)
    .shadow(shadow)
    .clip(true)
    .animated_inout(0xC5_AA_5E_47_u64, motion::FAST)
    .children(vec![header, columns, footer])
}

/// Overlay XP para el path **winit** (`view_overlay`): scrim full-screen con la
/// tarjeta [`xp_body`] anclada arriba a la izquierda.
pub fn start_menu_xp_overlay(
    apps: &[AppEntry],
    query: &str,
    offset: f32,
    bar_h: f32,
    screen: (f32, f32),
    theme: &Theme,
) -> View<Msg> {
    let body = xp_body(apps, query, offset, (screen.1 - bar_h) * 0.84, theme);
    View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(bar_h),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        padding: TaffyRect {
            left: length(8.0_f32),
            right: length(0.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(0, 0, 0, 110))
    .on_click(Msg::StartToggle)
    .children(vec![body])
}

/// Banda azul superior con avatar circular + nombre del usuario.
fn xp_header(theme: &Theme) -> View<Msg> {
    // El gradiente icónico XP: azul medio → azul más oscuro abajo.
    use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, Rect as KurboRect};
    use llimphi_ui::llimphi_raster::peniko::{Fill, Gradient};
    let _ = theme;

    let avatar = View::new(Style {
        size: Size { width: length(40.0_f32), height: length(40.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(245, 247, 252, 255))
    .radius(20.0)
    .text_aligned(
        usuario_inicial(),
        18.0,
        Color::from_rgba8(36, 64, 140, 255),
        Alignment::Center,
    )
    .bold();

    let nombre = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: auto(), height: percent(1.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        usuario_legible(),
        15.0,
        Color::from_rgba8(255, 255, 255, 255),
        Alignment::Start,
    )
    .bold();

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(XP_HEADER_H),
        },
        align_items: Some(AlignItems::Center),
        padding: TaffyRect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size { width: length(12.0_f32), height: length(0.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let x0 = rect.x as f64;
        let y0 = rect.y as f64;
        let x1 = (rect.x + rect.w) as f64;
        let y1 = (rect.y + rect.h) as f64;
        let r = KurboRect::new(x0, y0, x1, y1);
        // Verde-azul XP: arriba claro, abajo más profundo.
        let g = Gradient::new_linear(Point::new(x0, y0), Point::new(x0, y1)).with_stops(
            [
                Color::from_rgba8(52, 102, 196, 255),
                Color::from_rgba8(28, 60, 144, 255),
            ]
            .as_slice(),
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &r);
    })
    .children(vec![avatar, nombre])
}

/// Columna (pinned o programs) — título + lista de filas. `bordered`
/// agrega la línea vertical XP-style a la derecha (separa columnas).
fn xp_column(
    title: &str,
    apps: Vec<&AppEntry>,
    col_h: f32,
    theme: &Theme,
    bordered: bool,
) -> View<Msg> {
    let title_v = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: TaffyRect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text(title.to_string(), 11.0, Color::from_rgba8(96, 110, 132, 255));

    let rows: Vec<View<Msg>> = apps.iter().map(|a| xp_app_row(a, theme)).collect();

    let mut col_style = Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(0.5_f32),
            height: length(col_h),
        },
        padding: TaffyRect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    };
    if bordered {
        col_style.border = TaffyRect {
            left: length(0.0_f32),
            right: length(1.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        };
    }

    let mut children = vec![title_v];
    children.extend(rows);
    View::new(col_style).children(children)
}

/// La columna derecha — con scroll y filtro por query.
fn xp_column_scrolling(
    title: &str,
    apps: Vec<&AppEntry>,
    offset: f32,
    col_h: f32,
    theme: &Theme,
) -> View<Msg> {
    let title_v = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(20.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: TaffyRect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        ..Default::default()
    })
    .text(title.to_string(), 11.0, Color::from_rgba8(96, 110, 132, 255));

    let rows: Vec<View<Msg>> = apps.iter().map(|a| xp_app_row(a, theme)).collect();
    let n = rows.len() as f32;
    let inner = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        gap: Size { width: length(0.0_f32), height: length(2.0_f32) },
        ..Default::default()
    })
    .children(rows);

    let viewport_h = (col_h - 20.0).max(XP_ROW_H);
    let content_len = n * (XP_ROW_H + 2.0);
    let scroll = scroll_y(
        clamp_offset(offset, content_len, viewport_h),
        content_len,
        viewport_h,
        inner,
        Msg::StartScroll,
        &ScrollPalette::from_theme(theme),
    );

    let scroll_wrap = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(viewport_h),
        },
        ..Default::default()
    })
    .children(vec![scroll]);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(0.5_f32),
            height: length(col_h),
        },
        padding: TaffyRect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(Color::from_rgba8(235, 238, 246, 255))
    .children(vec![title_v, scroll_wrap])
}

fn xp_app_row(a: &AppEntry, theme: &Theme) -> View<Msg> {
    let icono = icono_o_inicial(a);
    let badge = View::new(Style {
        size: Size { width: length(22.0_f32), height: length(XP_ROW_H) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(icono, 14.0, Color::from_rgba8(36, 64, 140, 255), Alignment::Center);
    let nombre = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: auto(), height: length(XP_ROW_H) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text_aligned(
        a.label.clone(),
        12.5,
        Color::from_rgba8(20, 22, 40, 255),
        Alignment::Start,
    );

    let _ = theme;

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(XP_ROW_H),
        },
        padding: TaffyRect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .hover_fill(Color::from_rgba8(28, 60, 144, 32))
    .on_click(Msg::LaunchApp(a.id.clone()))
    .children(vec![badge, nombre])
}

/// Pie del menú XP: dos acciones (cerrar sesión / apagar).
fn xp_footer(_theme: &Theme) -> View<Msg> {
    use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, Rect as KurboRect};
    use llimphi_ui::llimphi_raster::peniko::{Fill, Gradient};

    let btn = |label: &str, glyph: &str, fg: Color, on_click: Msg| -> View<Msg> {
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size {
                width: auto(),
                height: length(XP_FOOTER_H - 12.0),
            },
            align_items: Some(AlignItems::Center),
            padding: TaffyRect {
                left: length(10.0_f32),
                right: length(10.0_f32),
                top: length(0.0_f32),
                bottom: length(0.0_f32),
            },
            gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
            ..Default::default()
        })
        .radius(4.0)
        .hover_fill(Color::from_rgba8(255, 255, 255, 28))
        .on_click(on_click)
        .children(vec![
            View::new(Style {
                size: Size { width: length(20.0_f32), height: length(20.0_f32) },
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .text(glyph.to_string(), 14.0, fg),
            View::new(Style {
                size: Size { width: auto(), height: length(20.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text(label.to_string(), 12.0, fg),
        ])
    };

    let logout = btn(
        "Cerrar sesión",
        "↩",
        Color::from_rgba8(255, 255, 255, 255),
        Msg::Quit,
    );
    let shutdown = btn(
        "Apagar",
        "⏻",
        Color::from_rgba8(255, 230, 230, 255),
        Msg::Spawn("systemctl poweroff".to_string()),
    );

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: percent(1.0_f32),
            height: length(XP_FOOTER_H),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexEnd),
        padding: TaffyRect {
            left: length(10.0_f32),
            right: length(10.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let x0 = rect.x as f64;
        let y0 = rect.y as f64;
        let x1 = (rect.x + rect.w) as f64;
        let y1 = (rect.y + rect.h) as f64;
        let r = KurboRect::new(x0, y0, x1, y1);
        // Banda verde apagada típica del XP "Turn Off Computer".
        let g = Gradient::new_linear(Point::new(x0, y0), Point::new(x0, y1)).with_stops(
            [
                Color::from_rgba8(118, 145, 197, 255),
                Color::from_rgba8(60, 88, 168, 255),
            ]
            .as_slice(),
        );
        scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &r);
    })
    .children(vec![logout, shutdown])
}

fn usuario_legible() -> String {
    std::env::var("USER")
        .unwrap_or_else(|_| "tawasuyu".into())
}

fn usuario_inicial() -> String {
    usuario_legible()
        .chars()
        .next()
        .unwrap_or('G')
        .to_uppercase()
        .to_string()
}

// =====================================================================
// Estilo GNOME — overlay full-screen + grid de tiles
// =====================================================================

const GNOME_TILE_SIZE: f32 = 96.0;
const GNOME_TILE_GAP: f32 = 18.0;
const GNOME_SEARCH_H: f32 = 56.0;
const GNOME_SEARCH_W: f32 = 540.0;
const GNOME_LABEL_H: f32 = 28.0;

/// Overlay del menú GNOME — full-screen, scrim oscuro, search arriba,
/// grid centrado de tiles.
/// El **cuerpo** del menú estilo grilla (GNOME Activities / KDE Kickoff):
/// buscador arriba + grilla de tiles que fluye al ancho del contenedor. Control
/// reutilizable; el caller pone el chrome (scrim en winit, barra en layer).
/// `columns_hint` acota cuántos tiles mostrar (`columns*6`, o 36 si es 0).
pub(super) fn gnome_body(apps: &[AppEntry], query: &str, columns_hint: u32, theme: &Theme) -> View<Msg> {
    let matches = menu_filtered(apps, query);
    let search = gnome_search(query, matches.len(), theme);
    let cap = if columns_hint > 0 {
        (columns_hint as usize) * 6
    } else {
        36
    };
    let tiles: Vec<View<Msg>> = matches.iter().take(cap).map(|a| gnome_tile(a)).collect();
    let grid = llimphi_widget_wrap::wrap_view(
        tiles,
        llimphi_widget_wrap::WrapAxis::Row,
        GNOME_TILE_GAP,
        GNOME_TILE_GAP,
    );
    // El bloque (search + grid) llena el ancho del contenedor; la grilla fluye.
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        align_items: Some(AlignItems::Center),
        gap: Size {
            width: length(0.0_f32),
            height: length(28.0_f32),
        },
        ..Default::default()
    })
    .children(vec![search, grid])
    .animated_enter(0xA0_91_E0_03_u64, motion::SLOW)
}

/// Overlay grilla para el path **winit**: scrim full-screen con [`gnome_body`]
/// centrado arriba.
pub fn start_menu_gnome_overlay(
    apps: &[AppEntry],
    query: &str,
    bar_h: f32,
    screen: (f32, f32),
    theme: &Theme,
) -> View<Msg> {
    let usable_w = screen.0 - 80.0;
    let tile_full = GNOME_TILE_SIZE + GNOME_TILE_GAP;
    let cols = ((usable_w / tile_full).floor() as u32).max(3);
    let content = gnome_body(apps, query, cols, theme);

    let centered = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexStart),
        padding: TaffyRect {
            left: length(40.0_f32),
            right: length(40.0_f32),
            top: length(80.0_f32),
            bottom: length(40.0_f32),
        },
        ..Default::default()
    })
    .children(vec![content]);

    View::new(Style {
        position: Position::Absolute,
        inset: TaffyRect {
            left: length(0.0_f32),
            top: length(bar_h),
            right: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        // Scrim oscuro + leve tinte hacia el accent, estilo GNOME shell.
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Rect as KurboRect};
        use llimphi_ui::llimphi_raster::peniko::Fill;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let x0 = rect.x as f64;
        let y0 = rect.y as f64;
        let x1 = (rect.x + rect.w) as f64;
        let y1 = (rect.y + rect.h) as f64;
        let r = KurboRect::new(x0, y0, x1, y1);
        let scrim: Color = AlphaColor::new([0.04, 0.05, 0.10, 0.86]);
        scene.fill(Fill::NonZero, Affine::IDENTITY, scrim, None, &r);
    })
    .on_click(Msg::StartToggle)
    .children(vec![centered])
}

fn gnome_search(query: &str, n_matches: usize, theme: &Theme) -> View<Msg> {
    let texto = if query.is_empty() {
        "Escribí para buscar…".to_string()
    } else {
        query.to_string()
    };
    let fg = if query.is_empty() {
        Color::from_rgba8(160, 170, 190, 255)
    } else {
        Color::from_rgba8(245, 246, 250, 255)
    };
    let conteo = format!("{} resultados", n_matches);

    let _ = theme;
    let buscador = View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size {
            width: length(GNOME_SEARCH_W),
            height: length(GNOME_SEARCH_H),
        },
        align_items: Some(AlignItems::Center),
        padding: TaffyRect {
            left: length(18.0_f32),
            right: length(18.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        gap: Size { width: length(12.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .fill(Color::from_rgba8(0, 0, 0, 110))
    .radius(28.0)
    .border(1.0, Color::from_rgba8(255, 255, 255, 40))
    .children(vec![
        View::new(Style {
            size: Size { width: length(20.0_f32), height: length(GNOME_SEARCH_H) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text(
            "⌕".to_string(),
            18.0,
            Color::from_rgba8(220, 230, 250, 255),
        ),
        View::new(Style {
            flex_grow: 1.0,
            size: Size { width: auto(), height: length(GNOME_SEARCH_H) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(texto, 16.0, fg, Alignment::Start),
        View::new(Style {
            size: Size { width: auto(), height: length(GNOME_SEARCH_H) },
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text(conteo, 11.0, Color::from_rgba8(160, 170, 190, 255)),
    ]);

    buscador
}

fn gnome_tile(a: &AppEntry) -> View<Msg> {
    // Inicial-chip cuando no hay glifo renderizable (antes «▸» para todas las
    // .desktop sin ícono → un mar de triángulos idénticos, y «NO GLYPH» para
    // los glifos de la suite ausentes en la fuente).
    let glyph = icono_o_inicial(a);
    let label = a.label.clone();
    let id = a.id.clone();

    let icon_box = View::new(Style {
        size: Size {
            width: length(GNOME_TILE_SIZE),
            height: length(GNOME_TILE_SIZE),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(255, 255, 255, 18))
    .radius(20.0)
    .border(1.0, Color::from_rgba8(255, 255, 255, 30))
    .hover_fill(Color::from_rgba8(255, 255, 255, 48))
    .text_aligned(
        glyph,
        46.0,
        Color::from_rgba8(245, 246, 250, 255),
        Alignment::Center,
    );

    let label_v = View::new(Style {
        size: Size {
            width: length(GNOME_TILE_SIZE + 16.0),
            height: length(GNOME_LABEL_H),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .text_aligned(
        label,
        12.0,
        Color::from_rgba8(238, 242, 252, 255),
        Alignment::Center,
    )
    .ellipsis(1);

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(GNOME_TILE_SIZE + 16.0),
            height: Dimension::auto(),
        },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(0.0_f32), height: length(6.0_f32) },
        ..Default::default()
    })
    .on_click(Msg::LaunchApp(id))
    .cursor(llimphi_ui::Cursor::Pointer)
    .children(vec![icon_box, label_v])
}

/// El **Program Manager** estilo Windows 3.1: una "ventana" gris Motif con
/// barra de título azul y una grilla de íconos de apps (lanzables al click).
/// Reutiliza el tile del menú grilla. Es persistente (no es un popup): lo monta
/// la vista `windows-3.1` como widget de una barra. El dato son las apps del
/// registro (vía `BarData::apps`).
pub(super) fn program_manager_view(apps: &[AppEntry], theme: &Theme) -> View<Msg> {
    let _ = theme;
    let azul = Color::from_rgba8(0, 0, 130, 255);
    let gris = Color::from_rgba8(196, 196, 196, 255);
    let blanco = Color::from_rgba8(255, 255, 255, 255);

    // Barra de título azul (la marca de Win3.1).
    let titulo = View::new(Style {
        size: Size {
            width: percent(1.0_f32),
            height: length(22.0_f32),
        },
        align_items: Some(AlignItems::Center),
        padding: TaffyRect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(azul)
    .text("Administrador de programas".to_string(), 12.0, blanco);

    // Grilla de íconos (grupos de programas).
    let tiles: Vec<View<Msg>> = apps.iter().take(24).map(gnome_tile).collect();
    let grid = llimphi_widget_wrap::wrap_view(
        tiles,
        llimphi_widget_wrap::WrapAxis::Row,
        GNOME_TILE_GAP,
        GNOME_TILE_GAP,
    );
    let cuerpo = View::new(Style {
        flex_grow: 1.0,
        size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        padding: TaffyRect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(12.0_f32),
            bottom: length(12.0_f32),
        },
        ..Default::default()
    })
    .fill(gris)
    .children(vec![grid]);

    // La "ventana": columna gris con borde sutil, tamaño fijo (no flota: es un
    // panel persistente que la barra de la vista win3.1 centra).
    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: length(660.0_f32),
            height: length(380.0_f32),
        },
        ..Default::default()
    })
    .fill(gris)
    .radius(2.0)
    .clip(true)
    .children(vec![titulo, cuerpo])
}
