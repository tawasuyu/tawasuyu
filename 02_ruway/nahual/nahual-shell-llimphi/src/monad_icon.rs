//! Ícono por **Mónada**, según su naturaleza y su contexto propio.
//!
//! La naturaleza de una Mónada es su [`Lens`] (Grid/Code/Gallery/Database/
//! Markdown/Tree), que cruza la frontera `Source` como `mime_hint =
//! "monada/<lente>"`. Antes todas las Mónadas se pintaban como `Icon::Folder`
//! genérico. Acá cada una es un [`IconSpec`] de tullpu donde:
//!
//! - la **forma/emblema** refleja la *naturaleza* (el lente), y
//! - el **color de la baldosa** se deriva del *id* — su contexto propio —, así
//!   dos Mónadas del mismo lente se distinguen a ojo (estilo identicon).
//!
//! Es exactamente el caso para el que se pensó el generador de íconos: la
//! naturaleza varía por contexto, y el ícono lo refleja sin glifos de fuente.

use nahual_source_core::Lens;
use tullpu_icon_core::{Capa, Color, Forma, IconSpec};

/// Paleta de baldosas (saturadas pero sobrias) para el color derivado del id.
const PALETA: [[u8; 4]; 8] = [
    [229, 91, 122, 255], // rosa
    [74, 134, 232, 255], // azul
    [59, 178, 115, 255], // verde
    [240, 168, 48, 255], // ámbar
    [155, 89, 182, 255], // violeta
    [43, 179, 192, 255], // teal
    [231, 110, 80, 255], // terracota
    [99, 110, 250, 255], // índigo
];

/// FNV-1a 64 — determinista entre máquinas (no usa el RandomState del proceso).
fn fnv1a(s: &str) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for b in s.as_bytes() {
        h ^= *b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Lente (naturaleza) desde el `mime_hint` que cruza la frontera `Source`.
/// `None`/desconocido ⇒ `Grid` (genérico), como hace `lens_mime` al revés.
pub fn lens_de_hint(hint: Option<&str>) -> Lens {
    match hint {
        Some("monada/gallery") => Lens::Gallery,
        Some("monada/code") => Lens::Code,
        Some("monada/database") => Lens::Database,
        Some("monada/markdown") => Lens::Markdown,
        Some("monada/tree") => Lens::Tree,
        _ => Lens::Grid,
    }
}

/// `true` si el nodo es una Mónada (id con prefijo `m:` o mime-hint `monada/…`).
pub fn es_monada(id: &str, mime_hint: Option<&str>) -> bool {
    id.starts_with("m:") || mime_hint.is_some_and(|h| h.starts_with("monada/"))
}

fn blanco() -> Color {
    Color::Rgba([255, 255, 255, 255])
}

/// Construye el `IconSpec` de una Mónada: baldosa redondeada con color derivado
/// de `id` + emblema blanco según `lens`. Determinista (mismo id+lens → mismo
/// ícono, en toda máquina).
pub fn monada_icon_spec(lens: Lens, id: &str) -> IconSpec {
    let bg = PALETA[(fnv1a(id) % PALETA.len() as u64) as usize];
    let mut capas = vec![Capa::rellena(
        Forma::RectRedondeado { x: 2.0, y: 2.0, w: 20.0, h: 20.0, r: 5.0 },
        Color::Rgba(bg),
    )];

    match lens {
        // Galería: sol + montaña (una foto).
        Lens::Gallery => {
            capas.push(Capa::rellena(Forma::Circulo { cx: 9.0, cy: 9.5, r: 2.0 }, blanco()));
            capas.push(Capa::rellena(
                Forma::PoligonoRegular { cx: 13.5, cy: 15.0, r: 5.0, lados: 3 },
                blanco(),
            ));
        }
        // Código: < >
        Lens::Code => {
            capas.push(Capa::trazada(Forma::polilinea(&[(11.0, 8.0), (7.0, 12.0), (11.0, 16.0)]), blanco(), 2.0));
            capas.push(Capa::trazada(Forma::polilinea(&[(13.0, 8.0), (17.0, 12.0), (13.0, 16.0)]), blanco(), 2.0));
        }
        // Datos: cilindro (tapa + cuerpo + base).
        Lens::Database => {
            capas.push(Capa::trazada(Forma::Elipse { cx: 12.0, cy: 8.0, rx: 5.5, ry: 2.2 }, blanco(), 1.6));
            capas.push(Capa::trazada(Forma::polilinea(&[(6.5, 8.0), (6.5, 16.0)]), blanco(), 1.6));
            capas.push(Capa::trazada(Forma::polilinea(&[(17.5, 8.0), (17.5, 16.0)]), blanco(), 1.6));
            capas.push(Capa::trazada(Forma::Elipse { cx: 12.0, cy: 16.0, rx: 5.5, ry: 2.2 }, blanco(), 1.6));
        }
        // Markdown: documento con renglones.
        Lens::Markdown => {
            capas.push(Capa::trazada(
                Forma::RectRedondeado { x: 6.0, y: 5.0, w: 12.0, h: 14.0, r: 1.5 },
                blanco(),
                1.6,
            ));
            for y in [9.0_f32, 12.0, 15.0] {
                capas.push(Capa::trazada(Forma::polilinea(&[(8.5, y), (15.5, y)]), blanco(), 1.3));
            }
        }
        // Árbol: nodos + enlaces.
        Lens::Tree => {
            capas.push(Capa::trazada(Forma::polilinea(&[(12.0, 7.0), (7.0, 14.5)]), blanco(), 1.4));
            capas.push(Capa::trazada(Forma::polilinea(&[(12.0, 7.0), (17.0, 14.5)]), blanco(), 1.4));
            capas.push(Capa::rellena(Forma::Circulo { cx: 12.0, cy: 6.5, r: 1.9 }, blanco()));
            capas.push(Capa::rellena(Forma::Circulo { cx: 7.0, cy: 16.0, r: 1.9 }, blanco()));
            capas.push(Capa::rellena(Forma::Circulo { cx: 17.0, cy: 16.0, r: 1.9 }, blanco()));
        }
        // Genérico (Grid): cuatro celdas.
        Lens::Grid => {
            for (x, y) in [(7.0, 7.0), (13.0, 7.0), (7.0, 13.0), (13.0, 13.0)] {
                capas.push(Capa::rellena(
                    Forma::RectRedondeado { x, y, w: 4.0, h: 4.0, r: 1.0 },
                    blanco(),
                ));
            }
        }
    }
    IconSpec::nuevo(id, capas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tullpu_icon_core::ColorFijo;

    #[test]
    fn cada_lente_produce_emblema_no_vacio() {
        for lens in [Lens::Grid, Lens::Code, Lens::Gallery, Lens::Database, Lens::Markdown, Lens::Tree] {
            let spec = monada_icon_spec(lens, "m:01HZX");
            // Baldosa + al menos un emblema.
            assert!(spec.capas.len() >= 2, "{lens:?} sin emblema");
            let r = ColorFijo::nuevo([0, 0, 0, 255]);
            assert!(!spec.compilar(&r).is_empty());
        }
    }

    #[test]
    fn color_depende_del_id_no_del_lente() {
        // Mismo lente, distinto id → baldosa de distinto color (su contexto propio).
        let a = monada_icon_spec(Lens::Code, "m:aaaa");
        let b = monada_icon_spec(Lens::Code, "m:zzzz");
        assert_ne!(a.capas[0], b.capas[0], "el color de baldosa debería variar por id");
    }

    #[test]
    fn deteccion_y_parseo() {
        assert!(es_monada("m:01HZX", None));
        assert!(es_monada("x", Some("monada/code")));
        assert!(!es_monada("/home/foo", Some("text/plain")));
        assert_eq!(lens_de_hint(Some("monada/gallery")), Lens::Gallery);
        assert_eq!(lens_de_hint(None), Lens::Grid);
    }
}
