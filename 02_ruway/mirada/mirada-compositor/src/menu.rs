//! Menú raíz estilo openbox: un pop-up que aparece al click derecho sobre el
//! fondo del escritorio (no sobre una ventana) y lista comandos del usuario.
//!
//! Sólo geometría e hit-testing puros — el render y el input viven en
//! [`crate::drm_backend`]. Las entradas (`(label, command)`) salen de la config
//! ([`mirada_brain::Config::menu`]).

/// Alto de cada fila del menú, en píxeles.
pub const ITEM_H: i32 = 26;
/// Ancho del menú, en píxeles.
pub const MENU_W: i32 = 210;
/// Relleno vertical arriba y abajo de la lista, en píxeles.
pub const PAD: i32 = 5;

/// Un menú raíz abierto: su esquina superior-izquierda y las entradas.
pub struct RootMenu {
    pub x: i32,
    pub y: i32,
    /// `(etiqueta, comando)` por fila.
    pub entries: Vec<(String, String)>,
}

impl RootMenu {
    /// Abre el menú con su esquina en `(px, py)`, acotada para que quepa entero
    /// dentro de una salida de `(out_w, out_h)` (se corre hacia adentro si el
    /// click fue cerca del borde).
    pub fn open(px: i32, py: i32, entries: Vec<(String, String)>, out_w: i32, out_h: i32) -> Self {
        let h = Self::height_for(entries.len());
        let x = px.min((out_w - MENU_W).max(0)).max(0);
        let y = py.min((out_h - h).max(0)).max(0);
        Self { x, y, entries }
    }

    /// Alto total del menú para `n` entradas.
    pub fn height_for(n: usize) -> i32 {
        n as i32 * ITEM_H + 2 * PAD
    }

    /// Alto total de este menú.
    pub fn height(&self) -> i32 {
        Self::height_for(self.entries.len())
    }

    /// El rect `(x, y, w, h)` de la fila `i`.
    pub fn item_rect(&self, i: usize) -> (i32, i32, i32, i32) {
        (self.x, self.y + PAD + i as i32 * ITEM_H, MENU_W, ITEM_H)
    }

    /// Índice de la fila bajo `(px, py)`, o `None` si el punto cae fuera del
    /// menú o en su relleno.
    pub fn hit(&self, px: i32, py: i32) -> Option<usize> {
        if px < self.x || px >= self.x + MENU_W {
            return None;
        }
        let rel = py - (self.y + PAD);
        if rel < 0 || rel >= self.entries.len() as i32 * ITEM_H {
            return None;
        }
        Some((rel / ITEM_H) as usize)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn menu() -> RootMenu {
        RootMenu {
            x: 100,
            y: 100,
            entries: vec![
                ("Terminal".into(), "kitty".into()),
                ("Navegador".into(), "firefox".into()),
                ("Archivos".into(), "nada".into()),
            ],
        }
    }

    #[test]
    fn hit_devuelve_la_fila_correcta() {
        let m = menu();
        // Primera fila: justo después del PAD.
        assert_eq!(m.hit(150, 100 + PAD + 1), Some(0));
        // Segunda fila.
        assert_eq!(m.hit(150, 100 + PAD + ITEM_H + 1), Some(1));
        // Tercera fila (última).
        assert_eq!(m.hit(150, 100 + PAD + 2 * ITEM_H + 1), Some(2));
    }

    #[test]
    fn hit_fuera_del_menu_es_none() {
        let m = menu();
        assert_eq!(m.hit(50, 110), None); // a la izquierda
        assert_eq!(m.hit(400, 110), None); // a la derecha
        assert_eq!(m.hit(150, 50), None); // por encima
        // Por debajo de la última fila.
        assert_eq!(m.hit(150, 100 + PAD + 3 * ITEM_H + 1), None);
    }

    #[test]
    fn open_acota_para_caber_en_la_salida() {
        let entries = vec![("a".into(), "a".into()), ("b".into(), "b".into())];
        // Click cerca del borde inferior-derecho de una salida 800x600.
        let m = RootMenu::open(790, 595, entries, 800, 600);
        assert!(m.x + MENU_W <= 800, "se sale por la derecha: x={}", m.x);
        assert!(m.y + m.height() <= 600, "se sale por abajo: y={}", m.y);
    }

    #[test]
    fn open_no_va_a_negativo_en_salida_minuscula() {
        let entries = vec![("a".into(), "a".into())];
        let m = RootMenu::open(5, 5, entries, 10, 10);
        assert!(m.x >= 0 && m.y >= 0);
    }
}
