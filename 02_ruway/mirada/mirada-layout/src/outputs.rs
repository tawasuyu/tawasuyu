//! Disposición de outputs físicos en el espacio compuesto.
//!
//! Mientras [`layout`](crate::layout) decide *dónde va cada ventana dentro de
//! un output*, este módulo decide *dónde va cada output dentro del escritorio
//! global*. Es el cálculo que un compositor multi-monitor necesita en cuanto
//! tiene más de un scanout: a cada monitor, de dimensiones propias, hay que
//! asignarle un origen `(x, y)` en coordenadas globales para que las ventanas
//! que viajan de uno a otro lo hagan por un plano continuo.
//!
//! Es pura geometría — sin `std`, sin dependencia del kernel ni del driver de
//! GPU—. El consumidor (en wawa: `kernel/src/pantallas.rs` alimentado por el
//! driver virtio-gpu cuando enumere scanouts) traduce cada [`Rect`] devuelto a
//! su tipo nativo de región y lo registra. El día que la enumeración de
//! scanouts esté disponible, esta función es la única pieza de matemática que
//! hace falta — y ya está probada.

use alloc::vec::Vec;

use crate::geometry::Rect;

/// Cómo se reparten los outputs en el espacio global.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Disposicion {
    /// En fila, de izquierda a derecha, alineados arriba (`y = 0`). El ancho
    /// global es la suma de anchos; el alto, el del más alto. Es el arreglo
    /// por defecto de la mayoría de escritorios de doble monitor.
    Horizontal,
    /// En columna, de arriba a abajo, alineados a la izquierda (`x = 0`). El
    /// alto global es la suma de altos; el ancho, el del más ancho.
    Vertical,
}

/// Dispone `tamanos` (cada uno `(ancho, alto)` en píxeles) según `modo` y
/// devuelve un [`Rect`] por output con su origen global ya calculado, en el
/// mismo orden de entrada. El primero queda anclado en `(0, 0)` — es el
/// primario—. Tamaños no positivos se respetan tal cual (el llamante decide si
/// filtrar outputs apagados antes de llamar).
///
/// Ejemplo (dos monitores 1920×1080 + 1280×1024 en fila):
/// `[(1920,1080),(1280,1024)]` → `[Rect{0,0,1920,1080}, Rect{1920,0,1280,1024}]`.
pub fn disponer(tamanos: &[(i32, i32)], modo: Disposicion) -> Vec<Rect> {
    let mut rects = Vec::with_capacity(tamanos.len());
    let mut avance = 0;
    for &(w, h) in tamanos {
        let rect = match modo {
            Disposicion::Horizontal => Rect::new(avance, 0, w, h),
            Disposicion::Vertical => Rect::new(0, avance, w, h),
        };
        rects.push(rect);
        avance += match modo {
            Disposicion::Horizontal => w.max(0),
            Disposicion::Vertical => h.max(0),
        };
    }
    rects
}

/// El rectángulo que envuelve a todos los outputs dispuestos: el tamaño del
/// escritorio compuesto. Útil para dimensionar un framebuffer global o validar
/// que el espacio cabe. Vacío (`0×0` en el origen) si no hay outputs.
pub fn envolvente(rects: &[Rect]) -> Rect {
    if rects.is_empty() {
        return Rect::new(0, 0, 0, 0);
    }
    let mut max_x = 0;
    let mut max_y = 0;
    for r in rects {
        max_x = max_x.max(r.x + r.w.max(0));
        max_y = max_y.max(r.y + r.h.max(0));
    }
    Rect::new(0, 0, max_x, max_y)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn horizontal_encadena_origenes_por_ancho() {
        let r = disponer(&[(1920, 1080), (1280, 1024)], Disposicion::Horizontal);
        assert_eq!(r[0], Rect::new(0, 0, 1920, 1080));
        assert_eq!(r[1], Rect::new(1920, 0, 1280, 1024));
    }

    #[test]
    fn vertical_apila_por_alto() {
        let r = disponer(&[(800, 600), (800, 480)], Disposicion::Vertical);
        assert_eq!(r[0], Rect::new(0, 0, 800, 600));
        assert_eq!(r[1], Rect::new(0, 600, 800, 480));
    }

    #[test]
    fn primario_unico_queda_en_origen() {
        let r = disponer(&[(1024, 768)], Disposicion::Horizontal);
        assert_eq!(r, [Rect::new(0, 0, 1024, 768)]);
    }

    #[test]
    fn sin_outputs_da_vec_vacio() {
        assert!(disponer(&[], Disposicion::Horizontal).is_empty());
        assert_eq!(envolvente(&[]), Rect::new(0, 0, 0, 0));
    }

    #[test]
    fn envolvente_horizontal_suma_ancho_y_toma_alto_maximo() {
        let r = disponer(&[(1920, 1080), (1280, 1024)], Disposicion::Horizontal);
        // ancho = 1920 + 1280; alto = max(1080, 1024).
        assert_eq!(envolvente(&r), Rect::new(0, 0, 3200, 1080));
    }

    #[test]
    fn envolvente_vertical_suma_alto_y_toma_ancho_maximo() {
        let r = disponer(&[(800, 600), (1024, 480)], Disposicion::Vertical);
        assert_eq!(envolvente(&r), Rect::new(0, 0, 1024, 1080));
    }

    #[test]
    fn tamano_no_positivo_no_avanza_el_cursor() {
        // Un output "apagado" (0 de ancho) no desplaza al siguiente.
        let r = disponer(&[(0, 0), (640, 480)], Disposicion::Horizontal);
        assert_eq!(r[1], Rect::new(0, 0, 640, 480));
    }
}
