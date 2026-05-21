//! Modos de teselado — cómo se reparte la pantalla entre ventanas.

use serde::{Deserialize, Serialize};

use crate::geometry::{split, Rect};

/// Estrategia de teselado.
///
/// Las variantes nuevas se añaden **al final** para no mover los índices
/// con que `postcard` las serializa en el API de control.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
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
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
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
    let cols = (count as f64).sqrt().ceil() as usize;
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
    let master_w = (screen.w as f32 * ratio).round() as i32;
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
    let master_w = (screen.w as f32 * ratio).round() as i32;
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
}
