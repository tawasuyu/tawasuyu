//! Modos de teselado — cómo se reparte la pantalla entre ventanas.

use alloc::{vec, vec::Vec};

#[cfg(feature = "serde")]
use serde::{Deserialize, Serialize};

use crate::geometry::{split, Rect};

/// Estrategia de teselado.
///
/// Las variantes nuevas se añaden **al final** para no mover los índices
/// con que `postcard` las serializa en el API de control.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum LayoutMode {
    /// Una ventana maestra a la izquierda; el resto apiladas a la derecha.
    MasterStack,
    /// Todas a pantalla completa, superpuestas — sólo se ve la enfocada.
    Monocle,
    /// Rejilla uniforme.
    Grid,
    /// Columnas verticales de igual ancho.
    Columns,
    /// Filas horizontales de igual alto.
    Rows,
    /// Ventana maestra centrada; el resto en columnas a ambos lados.
    /// Pensado para monitores anchos.
    CenteredMaster,
    /// Espiral de Fibonacci: cada ventana parte por la mitad el espacio
    /// que queda, alternando el sentido del corte.
    Spiral,
}

/// Una zona como fracciones `0..=1` de una pantalla: esquina `(x, y)` y tamaño
/// `(w, h)`. Es un **blanco de arrastre** (drag-to-zone): el compositor la pinta
/// mientras se arrastra una ventana y, al soltar encima, la ancla a este rect.
/// El nombre vive en la config; el motor sólo da la geometría ([`Self::to_rect`]).
#[derive(Debug, Clone, Copy, PartialEq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct ZoneFrac {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl ZoneFrac {
    /// Escala la zona (fracciones `0..=1`) a un rect absoluto dentro de
    /// `screen`, acotándola para que no se salga. Pura (`no_std`).
    pub fn to_rect(self, screen: Rect) -> Rect {
        let fx = self.x.clamp(0.0, 1.0);
        let fy = self.y.clamp(0.0, 1.0);
        // `clamp` (núcleo) en vez de `min` (que vive en `std`): este crate es no_std.
        let fw = self.w.clamp(0.0, 1.0 - fx);
        let fh = self.h.clamp(0.0, 1.0 - fy);
        // `libm` (no `f32::round`, que vive en `std`): este crate es `no_std`.
        let x = screen.x + libm::roundf(fx * screen.w as f32) as i32;
        let y = screen.y + libm::roundf(fy * screen.h as f32) as i32;
        let w = (libm::roundf(fw * screen.w as f32) as i32).max(1);
        let h = (libm::roundf(fh * screen.h as f32) as i32).max(1);
        Rect::new(x, y, w, h)
    }
}

/// Cómo se coloca la imagen del wallpaper dentro de la salida. Es **geometría
/// pura**: el compositor decide qué hacer con el rect que devuelve [`wallpaper_dst_rect`]
/// y los píxeles que quedan fuera de él (típicamente negro).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
#[cfg_attr(feature = "serde", serde(rename_all = "kebab-case"))]
pub enum WallpaperFit {
    /// Deformar la imagen para que cubra exactamente la salida. Sin barras
    /// negras, puede distorsionar la relación de aspecto. Es el default.
    Stretch,
    /// Encajar la imagen entera dentro de la salida (contain): se respeta el
    /// aspecto y la dimensión más restrictiva toca el borde; el resto queda
    /// negro (letterbox/pillarbox).
    Fit,
    /// Cubrir la salida (cover): se respeta el aspecto y la dimensión más
    /// laxa toca el borde; el resto de la imagen sobresale y se recorta.
    Fill,
    /// Pegar la imagen en su tamaño nativo, centrada. Si es más chica queda
    /// rodeada de negro; si es más grande se recorta.
    Center,
    /// Repetir la imagen en su tamaño nativo, tilada desde la esquina
    /// superior-izquierda hasta cubrir la salida.
    Tile,
}

impl Default for WallpaperFit {
    fn default() -> Self {
        WallpaperFit::Stretch
    }
}

impl WallpaperFit {
    /// Identificador kebab-case del modo (`"stretch"`, `"fit"`, `"fill"`,
    /// `"center"`, `"tile"`) para serializarlo en config de texto.
    pub fn slug(self) -> &'static str {
        match self {
            WallpaperFit::Stretch => "stretch",
            WallpaperFit::Fit => "fit",
            WallpaperFit::Fill => "fill",
            WallpaperFit::Center => "center",
            WallpaperFit::Tile => "tile",
        }
    }

    /// Parsea un slug; `None` si no coincide con ninguno conocido.
    pub fn from_slug(slug: &str) -> Option<Self> {
        Some(match slug {
            "stretch" => WallpaperFit::Stretch,
            "fit" => WallpaperFit::Fit,
            "fill" => WallpaperFit::Fill,
            "center" => WallpaperFit::Center,
            "tile" => WallpaperFit::Tile,
            _ => return None,
        })
    }
}

/// Para `Stretch`/`Fit`/`Fill`/`Center`, devuelve el rect `(x, y, w, h)` —en
/// coordenadas de la salida— donde se pinta la imagen escalada. El consumidor
/// rellena el resto con un color de fondo (típicamente negro). Para `Center`
/// la imagen va a su tamaño nativo (puede salirse del destino, se clipea).
/// Para [`WallpaperFit::Tile`] devuelve `(0, 0, sw, sh)` — el consumidor lo
/// tila a mano.
///
/// Pura (`no_std`): aritmética entera salvo el escalado proporcional, que usa
/// `libm` para no depender de `std`.
pub fn wallpaper_dst_rect(
    fit: WallpaperFit,
    src_w: i32,
    src_h: i32,
    dst_w: i32,
    dst_h: i32,
) -> (i32, i32, i32, i32) {
    let sw = src_w.max(1);
    let sh = src_h.max(1);
    let dw = dst_w.max(0);
    let dh = dst_h.max(0);
    match fit {
        WallpaperFit::Stretch => (0, 0, dw, dh),
        WallpaperFit::Tile => (0, 0, sw, sh),
        WallpaperFit::Center => {
            let x = (dw - sw) / 2;
            let y = (dh - sh) / 2;
            (x, y, sw, sh)
        }
        WallpaperFit::Fit | WallpaperFit::Fill => {
            // `src_wider`: la imagen es más ancha que el destino (mismo signo
            // que `sw/sh > dw/dh`, sin floats: `sw*dh > sh*dw`).
            let src_wider = (sw as i64) * (dh as i64) > (sh as i64) * (dw as i64);
            // Fit (contain): la dimensión más restrictiva toca el borde, la
            // imagen entra entera → si la imagen es más ancha, igualar ancho.
            // Fill (cover): la dimensión más laxa toca el borde, la imagen
            // sobresale por la otra → si la imagen es más ancha, igualar alto.
            let match_width = match fit {
                WallpaperFit::Fit => src_wider,
                WallpaperFit::Fill => !src_wider,
                _ => unreachable!(),
            };
            let (scaled_w, scaled_h) = if match_width {
                let h = libm::roundf(dw as f32 * sh as f32 / sw as f32) as i32;
                (dw, h.max(1))
            } else {
                let w = libm::roundf(dh as f32 * sw as f32 / sh as f32) as i32;
                (w.max(1), dh)
            };
            let x = (dw - scaled_w) / 2;
            let y = (dh - scaled_h) / 2;
            (x, y, scaled_w, scaled_h)
        }
    }
}

impl LayoutMode {
    /// Todos los modos, en el orden del ciclo de `CycleLayout`.
    pub const ALL: [LayoutMode; 7] = [
        LayoutMode::MasterStack,
        LayoutMode::CenteredMaster,
        LayoutMode::Spiral,
        LayoutMode::Grid,
        LayoutMode::Columns,
        LayoutMode::Rows,
        LayoutMode::Monocle,
    ];

    /// El siguiente modo en el ciclo (envuelve al llegar al final).
    pub fn next(self) -> LayoutMode {
        let i = Self::ALL.iter().position(|&m| m == self).unwrap_or(0);
        Self::ALL[(i + 1) % Self::ALL.len()]
    }
}

/// Parámetros del teselado.
#[derive(Debug, Clone, Copy)]
#[cfg_attr(feature = "serde", derive(Serialize, Deserialize))]
pub struct LayoutParams {
    pub mode: LayoutMode,
    /// Fracción del ancho para la ventana maestra en `MasterStack` y
    /// `CenteredMaster` (se acota a `0.05..=0.95`).
    pub master_ratio: f32,
    /// Cuántas ventanas van en el área maestra (`nmaster`); al menos 1.
    pub master_count: usize,
    /// Margen en píxeles alrededor de cada ventana.
    pub gap: i32,
}

impl Default for LayoutParams {
    fn default() -> Self {
        Self {
            mode: LayoutMode::MasterStack,
            master_ratio: 0.6,
            master_count: 1,
            gap: 8,
        }
    }
}

/// Calcula el rectángulo de cada una de las `count` ventanas dentro de
/// `screen`. El vector resultante tiene exactamente `count` elementos,
/// en el mismo orden que las ventanas.
pub fn tile(screen: Rect, count: usize, params: &LayoutParams) -> Vec<Rect> {
    if count == 0 {
        return Vec::new();
    }
    let cells = match params.mode {
        LayoutMode::Monocle => vec![screen; count],
        LayoutMode::Columns => columns(screen, count),
        LayoutMode::Rows => rows(screen, count),
        LayoutMode::Grid => grid(screen, count),
        LayoutMode::MasterStack => {
            master_stack(screen, count, params.master_ratio, params.master_count)
        }
        LayoutMode::CenteredMaster => {
            centered_master(screen, count, params.master_ratio, params.master_count)
        }
        LayoutMode::Spiral => spiral(screen, count),
    };
    // El margen se aplica al final, uniforme para todos los modos. *Smart
    // gaps*: una sola ventana va a sangre, sin margen desperdiciado.
    let gap = if count == 1 { 0 } else { params.gap };
    cells.into_iter().map(|c| c.inset(gap)).collect()
}

/// Columnas verticales de igual ancho.
fn columns(screen: Rect, count: usize) -> Vec<Rect> {
    split(screen.w, count)
        .into_iter()
        .map(|(off, w)| Rect::new(screen.x + off, screen.y, w, screen.h))
        .collect()
}

/// Rejilla `cols × rows` lo más cuadrada posible.
fn grid(screen: Rect, count: usize) -> Vec<Rect> {
    // `libm` en vez de los métodos de `f64`: `sqrt`/`ceil` viven en
    // `std`, no en `core` — y este crate es `no_std`.
    let cols = libm::ceil(libm::sqrt(count as f64)) as usize;
    let rows = count.div_ceil(cols);
    let col_parts = split(screen.w, cols);
    let row_parts = split(screen.h, rows);
    (0..count)
        .map(|i| {
            let (cx, cw) = col_parts[i % cols];
            let (ry, rh) = row_parts[i / cols];
            Rect::new(screen.x + cx, screen.y + ry, cw, rh)
        })
        .collect()
}

/// Filas horizontales de igual alto.
fn rows(screen: Rect, count: usize) -> Vec<Rect> {
    split(screen.h, count)
        .into_iter()
        .map(|(off, h)| Rect::new(screen.x, screen.y + off, screen.w, h))
        .collect()
}

/// Espiral de Fibonacci: cada ventana se queda con la mitad del espacio
/// libre y la siguiente recurre en la otra mitad, alternando el corte.
/// La última ventana llena todo lo que sobra.
fn spiral(screen: Rect, count: usize) -> Vec<Rect> {
    let mut out = Vec::with_capacity(count);
    let mut area = screen;
    let mut horizontal = true;
    for _ in 1..count {
        if horizontal {
            let p = split(area.w, 2);
            out.push(Rect::new(area.x, area.y, p[0].1, area.h));
            area = Rect::new(area.x + p[1].0, area.y, p[1].1, area.h);
        } else {
            let p = split(area.h, 2);
            out.push(Rect::new(area.x, area.y, area.w, p[0].1));
            area = Rect::new(area.x, area.y + p[1].0, area.w, p[1].1);
        }
        horizontal = !horizontal;
    }
    out.push(area);
    out
}

/// `master_count` ventanas maestras centradas + el resto repartido en
/// columnas a ambos lados.
fn centered_master(screen: Rect, count: usize, ratio: f32, master_count: usize) -> Vec<Rect> {
    let m = master_count.clamp(1, count);
    let stack = count - m;
    // Centrar sólo tiene sentido con al menos una ventana por lado.
    if stack < 2 {
        return master_stack(screen, count, ratio, master_count);
    }
    let ratio = ratio.clamp(0.05, 0.95);
    let master_w = libm::roundf(screen.w as f32 * ratio) as i32;
    let sides = split(screen.w - master_w, 2);
    let (left_w, right_w) = (sides[0].1, sides[1].1);
    let left_n = stack / 2;
    let right_n = stack - left_n;

    let mut out = Vec::with_capacity(count);
    // Las maestras, apiladas en la columna central — orden de teselado.
    for (off, h) in split(screen.h, m) {
        out.push(Rect::new(screen.x + left_w, screen.y + off, master_w, h));
    }
    for (off, h) in split(screen.h, left_n) {
        out.push(Rect::new(screen.x, screen.y + off, left_w, h));
    }
    for (off, h) in split(screen.h, right_n) {
        out.push(Rect::new(screen.x + left_w + master_w, screen.y + off, right_w, h));
    }
    out
}

/// `master_count` ventanas maestras a la izquierda + el resto en pila a
/// la derecha. Sin pila, las maestras llenan toda la pantalla.
fn master_stack(screen: Rect, count: usize, ratio: f32, master_count: usize) -> Vec<Rect> {
    let m = master_count.clamp(1, count);
    let stack = count - m;
    if stack == 0 {
        return split(screen.h, m)
            .into_iter()
            .map(|(off, h)| Rect::new(screen.x, screen.y + off, screen.w, h))
            .collect();
    }
    let ratio = ratio.clamp(0.05, 0.95);
    let master_w = libm::roundf(screen.w as f32 * ratio) as i32;
    let stack_x = screen.x + master_w;
    let stack_w = screen.w - master_w;

    let mut out = Vec::with_capacity(count);
    for (off, h) in split(screen.h, m) {
        out.push(Rect::new(screen.x, screen.y + off, master_w, h));
    }
    for (off, h) in split(screen.h, stack) {
        out.push(Rect::new(stack_x, screen.y + off, stack_w, h));
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    const SCREEN: Rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };

    fn params(mode: LayoutMode) -> LayoutParams {
        LayoutParams { mode, gap: 0, ..LayoutParams::default() }
    }

    #[test]
    fn empty_count_yields_no_rects() {
        assert!(tile(SCREEN, 0, &params(LayoutMode::Grid)).is_empty());
    }

    #[test]
    fn zone_frac_escala_a_pixeles() {
        let screen = Rect::new(0, 0, 1000, 800);
        let z = ZoneFrac { x: 0.5, y: 0.0, w: 0.5, h: 0.5 };
        assert_eq!(z.to_rect(screen), Rect::new(500, 0, 500, 400));
    }

    #[test]
    fn zone_frac_fuera_de_rango_se_acota_a_la_pantalla() {
        let screen = Rect::new(0, 0, 1000, 800);
        // x=0.8 + w=0.5 se pasa: w se recorta a 0.2.
        let z = ZoneFrac { x: 0.8, y: 0.0, w: 0.5, h: 1.0 };
        assert_eq!(z.to_rect(screen), Rect::new(800, 0, 200, 800));
    }

    #[test]
    fn tile_count_matches_window_count() {
        for mode in LayoutMode::ALL {
            for n in 1..=9 {
                assert_eq!(tile(SCREEN, n, &params(mode)).len(), n, "modo {mode:?}");
            }
        }
    }

    #[test]
    fn rows_partition_the_height_exactly() {
        let rects = tile(SCREEN, 3, &params(LayoutMode::Rows));
        assert_eq!(rects.iter().map(|r| r.h).sum::<i32>(), 1080);
        assert!(rects.iter().all(|r| r.w == 1920));
    }

    #[test]
    fn spiral_tiles_cover_the_screen_without_overlap() {
        for n in 1..=9 {
            let total: i64 = tile(SCREEN, n, &params(LayoutMode::Spiral))
                .iter()
                .map(|r| r.area())
                .sum();
            assert_eq!(total, SCREEN.area(), "espiral con {n} ventanas");
        }
    }

    #[test]
    fn centered_master_centers_the_master_and_covers_the_screen() {
        let rects = tile(SCREEN, 5, &params(LayoutMode::CenteredMaster));
        let master = rects[0];
        // Hueco a la izquierda y a la derecha de la maestra: iguales ±1px.
        let left = master.x - SCREEN.x;
        let right = (SCREEN.x + SCREEN.w) - (master.x + master.w);
        assert!((left - right).abs() <= 1, "maestra no centrada: {left} vs {right}");
        let total: i64 = rects.iter().map(|r| r.area()).sum();
        assert_eq!(total, SCREEN.area());
    }

    #[test]
    fn layout_mode_next_cycles_through_every_mode() {
        let mut visited: Vec<LayoutMode> = Vec::new();
        let mut m = LayoutMode::MasterStack;
        for _ in 0..LayoutMode::ALL.len() {
            assert!(!visited.contains(&m), "modo repetido en el ciclo: {m:?}");
            visited.push(m);
            m = m.next();
        }
        // Tras una vuelta completa, de vuelta al inicio.
        assert_eq!(m, LayoutMode::MasterStack);
        for mode in LayoutMode::ALL {
            assert!(visited.contains(&mode), "el ciclo no pasa por {mode:?}");
        }
    }

    #[test]
    fn monocle_gives_every_window_the_full_screen() {
        for r in tile(SCREEN, 4, &params(LayoutMode::Monocle)) {
            assert_eq!(r, SCREEN);
        }
    }

    #[test]
    fn columns_partition_the_width_exactly() {
        let rects = tile(SCREEN, 3, &params(LayoutMode::Columns));
        assert_eq!(rects.iter().map(|r| r.w).sum::<i32>(), 1920);
        // Todas ocupan el alto completo.
        assert!(rects.iter().all(|r| r.h == 1080));
    }

    #[test]
    fn master_stack_master_takes_its_ratio() {
        let rects = tile(SCREEN, 3, &params(LayoutMode::MasterStack));
        // 60% de 1920 = 1152.
        assert_eq!(rects[0].w, 1152);
        // Las dos de la pila comparten el resto del ancho y el alto.
        assert_eq!(rects[1].w, 1920 - 1152);
        assert_eq!(rects[1].h + rects[2].h, 1080);
    }

    #[test]
    fn master_stack_single_window_fills_screen() {
        let rects = tile(SCREEN, 1, &params(LayoutMode::MasterStack));
        assert_eq!(rects[0], SCREEN);
    }

    #[test]
    fn grid_tiles_cover_the_screen_without_overlap() {
        // 4 ventanas → rejilla 2×2, cada una un cuarto.
        let rects = tile(SCREEN, 4, &params(LayoutMode::Grid));
        let total: i64 = rects.iter().map(|r| r.area()).sum();
        assert_eq!(total, SCREEN.area());
    }

    #[test]
    fn gap_shrinks_every_window() {
        let p = LayoutParams { mode: LayoutMode::Columns, gap: 10, ..LayoutParams::default() };
        for r in tile(SCREEN, 2, &p) {
            // Cada celda de 960 de ancho se encoge 20 (10 por lado).
            assert_eq!(r.w, 960 - 20);
            assert_eq!(r.h, 1080 - 20);
        }
    }

    #[test]
    fn nmaster_keeps_n_windows_in_the_master_column() {
        let p = LayoutParams {
            mode: LayoutMode::MasterStack,
            master_count: 2,
            gap: 0,
            ..LayoutParams::default()
        };
        let rects = tile(SCREEN, 4, &p);
        // Dos maestras comparten el ancho maestro (60% de 1920 = 1152).
        assert_eq!(rects[0].w, 1152);
        assert_eq!(rects[1].w, 1152);
        // Dos de pila comparten el resto.
        assert_eq!(rects[2].w, 1920 - 1152);
        assert_eq!(rects[3].w, 1920 - 1152);
        // Las dos maestras parten la altura entre ellas.
        assert_eq!(rects[0].h + rects[1].h, 1080);
    }

    #[test]
    fn nmaster_above_window_count_makes_every_window_a_master() {
        let p = LayoutParams {
            mode: LayoutMode::MasterStack,
            master_count: 9,
            gap: 0,
            ..LayoutParams::default()
        };
        let rects = tile(SCREEN, 3, &p);
        // Sin pila: las tres ocupan el ancho completo.
        assert!(rects.iter().all(|r| r.w == 1920));
        assert_eq!(rects.iter().map(|r| r.h).sum::<i32>(), 1080);
    }

    #[test]
    fn smart_gaps_drop_the_margin_for_a_single_window() {
        let p = LayoutParams { mode: LayoutMode::MasterStack, gap: 20, ..LayoutParams::default() };
        // Una sola ventana: a sangre, sin margen.
        assert_eq!(tile(SCREEN, 1, &p)[0], SCREEN);
        // Con dos, el margen vuelve.
        assert!(tile(SCREEN, 2, &p)[0].w < SCREEN.w);
    }

    #[test]
    fn layout_is_deterministic() {
        let p = params(LayoutMode::Grid);
        assert_eq!(tile(SCREEN, 7, &p), tile(SCREEN, 7, &p));
    }

    // --- wallpaper_dst_rect ---------------------------------------------

    #[test]
    fn wallpaper_stretch_cubre_toda_la_salida() {
        assert_eq!(
            wallpaper_dst_rect(WallpaperFit::Stretch, 800, 600, 1920, 1080),
            (0, 0, 1920, 1080),
        );
    }

    #[test]
    fn wallpaper_center_pega_la_imagen_a_su_tamano() {
        // Imagen más chica → queda centrada con padding.
        let r = wallpaper_dst_rect(WallpaperFit::Center, 800, 600, 1920, 1080);
        assert_eq!(r, ((1920 - 800) / 2, (1080 - 600) / 2, 800, 600));
        // Imagen más grande → offset negativo (se clipea).
        let r = wallpaper_dst_rect(WallpaperFit::Center, 4000, 3000, 1920, 1080);
        assert_eq!(r, ((1920 - 4000) / 2, (1080 - 3000) / 2, 4000, 3000));
    }

    #[test]
    fn wallpaper_fit_respeta_aspecto_y_no_sobresale() {
        // Imagen 4:3 (más cuadrada) en pantalla 16:9 (más ancha) → fit toca el
        // alto (la imagen es más alta-proporcional que la pantalla, así que la
        // dimensión más restrictiva es el ancho-virtual; el alto llena 1080 y
        // el ancho queda con pillarbox).
        let (x, y, w, h) = wallpaper_dst_rect(WallpaperFit::Fit, 800, 600, 1920, 1080);
        assert!(w <= 1920 && h <= 1080);
        assert_eq!(h, 1080);
        assert_eq!(w, 1440); // 1080 * 800 / 600
        assert_eq!(y, 0);
        assert_eq!(x, (1920 - 1440) / 2);

        // Imagen 16:9 panorámica en pantalla 4:3 → letterbox arriba/abajo.
        let (x, y, w, h) = wallpaper_dst_rect(WallpaperFit::Fit, 1600, 900, 1024, 768);
        assert_eq!(w, 1024);
        assert_eq!(h, 576); // 1024 * 9 / 16
        assert_eq!(x, 0);
        assert_eq!(y, (768 - 576) / 2);
    }

    #[test]
    fn wallpaper_fill_cubre_y_recorta_los_bordes() {
        // 4:3 imagen en 16:9 pantalla → fill llena el ancho, sobra arriba/abajo
        // (offset y negativo).
        let (x, y, w, h) = wallpaper_dst_rect(WallpaperFit::Fill, 800, 600, 1920, 1080);
        assert_eq!(w, 1920);
        assert_eq!(h, 1440); // 1920 * 600 / 800
        assert_eq!(x, 0);
        assert!(y < 0, "fill debe sobresalir en Y, no quedar dentro");

        // 16:9 imagen en 4:3 pantalla → fill llena el alto, sobra a los lados.
        let (x, y, w, h) = wallpaper_dst_rect(WallpaperFit::Fill, 1600, 900, 1024, 768);
        assert_eq!(h, 768);
        // 768 * 1600 / 900 = 1365.33 → 1365.
        assert!(w >= 1024);
        assert!(x < 0);
        assert_eq!(y, 0);
    }

    #[test]
    fn wallpaper_tile_devuelve_el_tamano_nativo() {
        assert_eq!(
            wallpaper_dst_rect(WallpaperFit::Tile, 128, 128, 1920, 1080),
            (0, 0, 128, 128),
        );
    }

    #[test]
    fn wallpaper_aspecto_igual_no_distorsiona_ni_recorta() {
        // Si la imagen ya tiene el mismo aspecto, fit y fill coinciden con
        // stretch (cubre todo, sin offset).
        let r_fit = wallpaper_dst_rect(WallpaperFit::Fit, 1600, 900, 1920, 1080);
        let r_fill = wallpaper_dst_rect(WallpaperFit::Fill, 1600, 900, 1920, 1080);
        assert_eq!(r_fit, (0, 0, 1920, 1080));
        assert_eq!(r_fill, (0, 0, 1920, 1080));
    }

    #[test]
    fn wallpaper_dimensiones_degeneradas_no_panican() {
        // No se exige nada de los valores devueltos: que no panique.
        let _ = wallpaper_dst_rect(WallpaperFit::Fit, 0, 0, 1920, 1080);
        let _ = wallpaper_dst_rect(WallpaperFit::Fill, 100, 100, 0, 0);
        let _ = wallpaper_dst_rect(WallpaperFit::Center, 100, 100, 0, 0);
    }
}
