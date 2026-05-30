//! Deck core — máquinas de estado agnósticas para presentar, en dos modos:
//!
//! - **Lineal** ([`DeckState`], este archivo): strip horizontal de páginas con
//!   drag/snap. Dados los eventos crudos de pointer (coords + viewport width
//!   + n_pages) decide cuándo arrancar drag horizontal, cuánto trasladar el
//!   strip y a qué página snapear al soltar.
//! - **Espacial** ([`recorrido`], tipo Prezi): lienzo 2D infinito con marcos en
//!   coordenadas de mundo y una ruta de cámara ([`camara::Camara`]) que vuela
//!   entre ellos con zoom/pan/giro. El modo lineal es su caso degenerado.
//!
//! Todo agnóstico: sin DOM, sin wasm-bindgen, sin render.

pub mod camara;
pub mod recorrido;

pub use camara::{Camara, Ease, Rect, FIT_MARGEN, ZOOM_MAX, ZOOM_MIN};
pub use recorrido::{
    ContenidoMarco, Marco, MarcoId, Recorrido, RecorridoState, DURACION_PASO_S,
};

/// Umbral en pixels para confirmar gesto horizontal vs vertical.
pub const DRAG_DECISION_PX: f64 = 8.0;
/// Cuán más horizontal que vertical debe ser el delta para considerarse "swipe".
pub const HORIZONTAL_BIAS: f64 = 1.3;

#[derive(Clone, Debug, Default)]
pub struct DeckState {
    pub current_index: usize,
    pointer_start: Option<(f64, f64, i32)>,
    drag_active: bool,
    drag_start_offset: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub enum DragOutcome {
    /// Aún no hay decisión — esperar más movimiento.
    Idle,
    /// Empezar drag horizontal: el host debe capturar el pointer.
    StartHorizontal { pointer_id: i32 },
    /// Movimiento vertical predominante — host debe ceder al scroll nativo.
    CancelVertical,
    /// Drag en curso — host debe trasladar el strip a este offset.
    DragOffset(f64),
}

#[derive(Clone, Debug, PartialEq)]
pub struct SnapResult {
    pub target_index: usize,
    pub offset_px: f64,
    pub changed: bool,
}

impl DeckState {
    pub fn new() -> Self { Self::default() }

    /// Marca el inicio de un gesto. `viewport_width` se usa para anclar el
    /// drag_start_offset a la página visible actual.
    pub fn pointer_down(&mut self, x: f64, y: f64, pointer_id: i32, viewport_width: f64) {
        self.pointer_start = Some((x, y, pointer_id));
        self.drag_active = false;
        self.drag_start_offset = -(self.current_index as f64) * viewport_width;
    }

    /// Procesa un movimiento. Devuelve la acción que el host debe ejecutar.
    pub fn pointer_move(&mut self, x: f64, y: f64) -> DragOutcome {
        let Some((sx, sy, pid)) = self.pointer_start else {
            return DragOutcome::Idle;
        };
        let dx = x - sx;
        let dy = y - sy;
        if !self.drag_active {
            let abs_dx = dx.abs();
            let abs_dy = dy.abs();
            if abs_dx > DRAG_DECISION_PX && abs_dx > abs_dy * HORIZONTAL_BIAS {
                self.drag_active = true;
                return DragOutcome::StartHorizontal { pointer_id: pid };
            } else if abs_dy > DRAG_DECISION_PX {
                self.pointer_start = None;
                return DragOutcome::CancelVertical;
            } else {
                return DragOutcome::Idle;
            }
        }
        DragOutcome::DragOffset(self.drag_start_offset + dx)
    }

    /// Finaliza el gesto. Si había drag activo, calcula la página snap y
    /// actualiza `current_index`. `current_offset` viene del estado real
    /// del strip (el host lee CSS transform / variable).
    pub fn pointer_end(
        &mut self,
        current_offset: f64,
        viewport_width: f64,
        n_pages: usize,
    ) -> Option<SnapResult> {
        let was_active = self.drag_active;
        self.drag_active = false;
        self.pointer_start = None;
        if !was_active || viewport_width <= 0.0 || n_pages == 0 {
            return None;
        }
        let raw = -current_offset / viewport_width;
        let target = (raw.round().max(0.0) as usize).min(n_pages - 1);
        let offset_px = -(target as f64) * viewport_width;
        let changed = self.current_index != target;
        self.current_index = target;
        Some(SnapResult { target_index: target, offset_px, changed })
    }

    /// Salto programático (click en tabs externos). Devuelve el offset
    /// resultante para que el host lo aplique al strip.
    pub fn goto(&mut self, index: usize, viewport_width: f64) -> SnapResult {
        let offset_px = -(index as f64) * viewport_width;
        let changed = self.current_index != index;
        self.current_index = index;
        SnapResult { target_index: index, offset_px, changed }
    }

    /// Reposiciona tras un resize. Devuelve el offset que el host debe
    /// aplicar sin animación.
    pub fn reposition(&self, viewport_width: f64) -> f64 {
        -(self.current_index as f64) * viewport_width
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn vertical_drag_is_cancelled() {
        let mut s = DeckState::new();
        s.pointer_down(100.0, 100.0, 1, 800.0);
        // Movimiento vertical mayor que el umbral.
        let r = s.pointer_move(100.0, 120.0);
        assert_eq!(r, DragOutcome::CancelVertical);
    }

    #[test]
    fn horizontal_drag_starts_after_threshold() {
        let mut s = DeckState::new();
        s.pointer_down(100.0, 100.0, 7, 800.0);
        // Justo por debajo del umbral → Idle.
        assert_eq!(s.pointer_move(105.0, 100.0), DragOutcome::Idle);
        // Sobre el umbral con bias horizontal → Start.
        let r = s.pointer_move(120.0, 100.0);
        assert_eq!(r, DragOutcome::StartHorizontal { pointer_id: 7 });
    }

    #[test]
    fn snap_rounds_to_nearest_page() {
        let mut s = DeckState::new();
        s.current_index = 1;
        s.pointer_down(0.0, 0.0, 1, 1000.0); // drag_start_offset = -1000
        // Forzar drag activo
        s.pointer_move(20.0, 0.0);
        // Offset actual = -1000 + 20 = -980 → target round(980/1000) = 1, sin cambio
        let r = s.pointer_end(-980.0, 1000.0, 3).unwrap();
        assert_eq!(r.target_index, 1);
        assert!(!r.changed);
        // Mover lo suficiente para snapear a página 0
        s.pointer_down(0.0, 0.0, 1, 1000.0);
        s.pointer_move(600.0, 0.0);
        let r = s.pointer_end(-400.0, 1000.0, 3).unwrap();
        assert_eq!(r.target_index, 0);
        assert!(r.changed);
    }

    #[test]
    fn snap_clamps_to_bounds() {
        let mut s = DeckState::new();
        s.current_index = 2;
        s.pointer_down(0.0, 0.0, 1, 500.0);
        s.pointer_move(50.0, 0.0);
        // Offset muy a la izquierda → debería clamp a n_pages-1
        let r = s.pointer_end(-9999.0, 500.0, 3).unwrap();
        assert_eq!(r.target_index, 2);
    }

    #[test]
    fn goto_updates_index_and_offset() {
        let mut s = DeckState::new();
        let r = s.goto(2, 800.0);
        assert_eq!(r.target_index, 2);
        assert_eq!(r.offset_px, -1600.0);
        assert!(r.changed);
        // segundo goto al mismo índice → changed=false
        let r = s.goto(2, 800.0);
        assert!(!r.changed);
    }
}
