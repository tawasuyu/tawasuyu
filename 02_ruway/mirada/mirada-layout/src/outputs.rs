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

/// Factor de escala HiDPI expresado en 120-avos, la convención de
/// `wp_fractional_scale` de Wayland: `120` = 100 %, `180` = 150 %, `240` = 200 %.
/// Mantener la escala como entero sobre 120 deja toda la matemática en `i32`
/// —sin `f32`, apto para el kernel de wawa— y casa exacto con el protocolo.
pub const ESCALA_100: i32 = 120;

/// Un output físico con su factor de escala HiDPI. `ancho`/`alto` son los
/// píxeles reales del scanout; `escala_120`, cuántos 120-avos de aumento aplica
/// el cliente (ver [`ESCALA_100`]).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Salida {
    pub ancho: i32,
    pub alto: i32,
    pub escala_120: i32,
}

impl Salida {
    /// Una salida a 100 % (`escala_120 = ESCALA_100`).
    pub fn new(ancho: i32, alto: i32) -> Self {
        Self {
            ancho,
            alto,
            escala_120: ESCALA_100,
        }
    }

    /// La misma salida con otra escala.
    pub fn con_escala(self, escala_120: i32) -> Self {
        Self { escala_120, ..self }
    }

    /// Tamaño lógico `(ancho, alto)` del output: los píxeles físicos divididos
    /// por la escala. Un 4K (3840×2160) a 200 % mide 1920×1080 lógicos. Una
    /// escala no positiva se trata como 100 % (sin escalar). La división trunca
    /// —el píxel lógico parcial no existe—.
    pub fn logico(&self) -> (i32, i32) {
        let escala = if self.escala_120 > 0 {
            self.escala_120
        } else {
            ESCALA_100
        };
        let w = self.ancho.max(0) as i64 * ESCALA_100 as i64 / escala as i64;
        let h = self.alto.max(0) as i64 * ESCALA_100 as i64 / escala as i64;
        (w as i32, h as i32)
    }
}

/// Como [`disponer`], pero en **coordenadas lógicas**: cada output aporta su
/// tamaño *lógico* (físico ÷ escala) al encadenado. Es lo que un compositor
/// multi-DPI necesita: las ventanas viajan por un plano lógico continuo, y cada
/// output traduce de vuelta a físico con su propia escala al componer, así un
/// monitor 1× junto a uno 2× no abre un salto en el escritorio. Mismo orden de
/// entrada; el primero queda anclado en `(0, 0)`.
///
/// Ejemplo (un 1080p a 100 % junto a un 4K a 200 %): ambos miden 1920×1080
/// lógicos, así que quedan `[Rect{0,0,1920,1080}, Rect{1920,0,1920,1080}]` —el
/// 4K, con el doble de píxeles físicos, ocupa el mismo ancho lógico—.
pub fn disponer_logico(salidas: &[Salida], modo: Disposicion) -> Vec<Rect> {
    let tamanos: Vec<(i32, i32)> = salidas.iter().map(Salida::logico).collect();
    disponer(&tamanos, modo)
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

    #[test]
    fn logico_divide_los_fisicos_por_la_escala() {
        // 4K a 200 % mide 1920×1080 lógicos.
        assert_eq!(Salida::new(3840, 2160).con_escala(240).logico(), (1920, 1080));
        // 150 %: 2560×1440 → 1706×960 (trunca el píxel parcial).
        assert_eq!(Salida::new(2560, 1440).con_escala(180).logico(), (1706, 960));
    }

    #[test]
    fn escala_100_equivale_a_los_fisicos() {
        let s = Salida::new(1920, 1080);
        assert_eq!(s.escala_120, ESCALA_100);
        assert_eq!(s.logico(), (1920, 1080));
    }

    #[test]
    fn escala_no_positiva_se_trata_como_100() {
        // Un output que aún no reportó escala (0) no se encoge a infinito.
        assert_eq!(Salida::new(1280, 720).con_escala(0).logico(), (1280, 720));
        assert_eq!(Salida::new(1280, 720).con_escala(-5).logico(), (1280, 720));
    }

    #[test]
    fn disponer_logico_continua_el_plano_entre_dpis_distintas() {
        // Un 1080p@100 % junto a un 4K@200 %: ambos 1920×1080 lógicos, plano
        // continuo (el segundo arranca justo donde acaba el primero).
        let salidas = [
            Salida::new(1920, 1080),
            Salida::new(3840, 2160).con_escala(240),
        ];
        let r = disponer_logico(&salidas, Disposicion::Horizontal);
        assert_eq!(r[0], Rect::new(0, 0, 1920, 1080));
        assert_eq!(r[1], Rect::new(1920, 0, 1920, 1080));
        // El escritorio lógico mide 3840×1080 pese a los 7680 px físicos.
        assert_eq!(envolvente(&r), Rect::new(0, 0, 3840, 1080));
    }

    #[test]
    fn disponer_logico_sin_escala_coincide_con_disponer() {
        let salidas = [Salida::new(1920, 1080), Salida::new(1280, 1024)];
        let logico = disponer_logico(&salidas, Disposicion::Horizontal);
        let fisico = disponer(&[(1920, 1080), (1280, 1024)], Disposicion::Horizontal);
        assert_eq!(logico, fisico);
    }
}
