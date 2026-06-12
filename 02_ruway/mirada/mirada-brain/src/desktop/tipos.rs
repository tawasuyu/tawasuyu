//! Tipos de datos básicos del escritorio: identidad de ventana y salida física.

use mirada_layout::Rect;
use mirada_protocol::OutputId;

/// Lo que el Cerebro sabe de una ventana: su identidad de aplicación.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct WindowInfo {
    pub app_id: String,
    pub title: String,
}

/// Una salida física y el escritorio virtual que muestra ahora mismo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Output {
    pub id: OutputId,
    /// Rectángulo en el espacio global — las salidas van en fila horizontal.
    pub rect: Rect,
    /// Zonas exclusivas reservadas por el marco (`pata`/shell), en px desde
    /// cada borde: `(top, bottom, left, right)`. El teselado las esquiva.
    pub reserved: (i32, i32, i32, i32),
    /// Índice del escritorio que esta salida muestra.
    pub workspace: usize,
}

impl Output {
    /// El área teselable: el rect global menos las zonas reservadas. Es lo que
    /// se le pasa al motor de layout, así que las barras de cualquier borde
    /// quedan libres de ventanas.
    pub fn work_rect(&self) -> Rect {
        let (top, bottom, left, right) = self.reserved;
        Rect::new(
            self.rect.x + left,
            self.rect.y + top,
            (self.rect.w - left - right).max(1),
            (self.rect.h - top - bottom).max(1),
        )
    }
}
