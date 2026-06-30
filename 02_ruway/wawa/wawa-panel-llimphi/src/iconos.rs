//! Íconos vectoriales de los **dientes** del panel.
//!
//! Antes los dientes pintaban un glifo unicode (emoji) con el motor de texto:
//! en hardware sin esas fuentes salía notdef ("pila de líneas horizontales") y
//! en el resto, muchos caían al glifo monocromo y soso de DejaVu. Acá cada
//! diente es un [`IconSpec`] que el puente `tullpu-icon-llimphi` pinta como
//! **vectores** — determinista en toda máquina y con acentos de color.
//!
//! Convención: forma principal en [`Color::Corriente`] (la pinta el rail según
//! activo/inactivo, así sigue el theme) + un acento de color fijo para darles
//! vida. Grilla 24×24.

use tullpu_icon_core::{Capa, Color, Forma, IconSpec};
use tullpu_core::ComandoPath;

fn rgb(r: u8, g: u8, b: u8) -> Color {
    Color::Rgba([r, g, b, 255])
}

/// `Color::Corriente` (currentColor). Como `Color` no es `Copy`, lo emitimos
/// fresco en cada uso.
fn corr() -> Color {
    Color::Corriente
}

/// Polilínea trazada en un color (para tapas de sobre, manecillas, barras).
fn polilinea(pts: &[(f32, f32)], color: Color, ancho: f32) -> Capa {
    let mut comandos = Vec::with_capacity(pts.len());
    for (i, &(x, y)) in pts.iter().enumerate() {
        comandos.push(if i == 0 {
            ComandoPath::MoverA { x, y }
        } else {
            ComandoPath::LineaA { x, y }
        });
    }
    Capa::trazada(Forma::Path { comandos }, color, ancho)
}

const W: f32 = 1.8;

/// Devuelve el `IconSpec` del diente cuyo título es `title`. Para títulos
/// desconocidos cae a un punto vectorial (nunca a texto).
pub fn spec_diente(title: &str) -> IconSpec {
    let capas = match title {
        // Vista — un ojo (apariencia).
        "Vista" => vec![
            Capa::trazada(Forma::Elipse { cx: 12.0, cy: 12.0, rx: 9.0, ry: 5.5 }, corr(), W),
            Capa::rellena(Forma::Circulo { cx: 12.0, cy: 12.0, r: 2.8 }, rgb(43, 179, 192)),
        ],
        // Themes — tres muestras de color superpuestas.
        "Themes" => vec![
            Capa::rellena(Forma::Circulo { cx: 9.0, cy: 10.0, r: 3.4 }, rgb(229, 85, 106)),
            Capa::rellena(Forma::Circulo { cx: 15.0, cy: 10.0, r: 3.4 }, rgb(59, 178, 115)),
            Capa::rellena(Forma::Circulo { cx: 12.0, cy: 15.0, r: 3.4 }, rgb(74, 134, 232)),
        ],
        // Atajos — un teclado.
        "Atajos" => vec![
            Capa::trazada(Forma::RectRedondeado { x: 3.0, y: 7.0, w: 18.0, h: 10.0, r: 2.5 }, corr(), W),
            Capa::rellena(Forma::Rect { x: 6.0, y: 10.0, w: 2.4, h: 2.4 }, rgb(240, 168, 48)),
            Capa::rellena(Forma::Rect { x: 10.8, y: 10.0, w: 2.4, h: 2.4 }, rgb(240, 168, 48)),
            Capa::rellena(Forma::Rect { x: 15.6, y: 10.0, w: 2.4, h: 2.4 }, rgb(240, 168, 48)),
            Capa::rellena(Forma::Rect { x: 8.0, y: 14.0, w: 8.0, h: 1.8 }, rgb(240, 168, 48)),
        ],
        // Animaciones — una chispa de 4 puntas.
        "Animaciones" => vec![
            Capa::rellena(
                Forma::Estrella { cx: 12.0, cy: 12.0, r_ext: 8.5, r_int: 2.8, puntas: 4 },
                rgb(245, 197, 66),
            ),
        ],
        // Pata — sliders de mezclador.
        "Pata" => vec![
            polilinea(&[(4.0, 8.0), (20.0, 8.0)], corr(), W),
            polilinea(&[(4.0, 12.0), (20.0, 12.0)], corr(), W),
            polilinea(&[(4.0, 16.0), (20.0, 16.0)], corr(), W),
            Capa::rellena(Forma::Circulo { cx: 9.0, cy: 8.0, r: 2.2 }, rgb(155, 89, 182)),
            Capa::rellena(Forma::Circulo { cx: 15.0, cy: 12.0, r: 2.2 }, rgb(155, 89, 182)),
            Capa::rellena(Forma::Circulo { cx: 11.0, cy: 16.0, r: 2.2 }, rgb(155, 89, 182)),
        ],
        // Inicio — símbolo de encendido.
        "Inicio" => vec![
            Capa::trazada(Forma::Circulo { cx: 12.0, cy: 13.0, r: 7.5 }, corr(), W),
            polilinea(&[(12.0, 4.0), (12.0, 11.0)], rgb(229, 85, 106), 2.2),
        ],
        // Sistema — engranaje (octágono + núcleo).
        "Sistema" => vec![
            Capa::trazada(Forma::PoligonoRegular { cx: 12.0, cy: 12.0, r: 8.0, lados: 8 }, corr(), W),
            Capa::rellena(Forma::Circulo { cx: 12.0, cy: 12.0, r: 3.0 }, rgb(74, 134, 232)),
        ],
        // Acerca — "i" de información.
        "Acerca" => vec![
            Capa::trazada(Forma::Circulo { cx: 12.0, cy: 12.0, r: 9.0 }, corr(), W),
            Capa::rellena(Forma::Circulo { cx: 12.0, cy: 7.6, r: 1.4 }, rgb(43, 179, 192)),
            polilinea(&[(12.0, 11.0), (12.0, 16.5)], rgb(43, 179, 192), 2.2),
        ],
        // Correo — sobre.
        "Correo" => vec![
            Capa::trazada(Forma::RectRedondeado { x: 3.5, y: 6.5, w: 17.0, h: 11.0, r: 1.5 }, corr(), W),
            polilinea(&[(3.5, 7.5), (12.0, 13.0), (20.5, 7.5)], rgb(229, 85, 106), W),
        ],
        // Contextos — reloj (pacha).
        "Contextos" => vec![
            Capa::trazada(Forma::Circulo { cx: 12.0, cy: 12.0, r: 9.0 }, corr(), W),
            polilinea(&[(12.0, 12.0), (12.0, 6.5)], rgb(240, 168, 48), W),
            polilinea(&[(12.0, 12.0), (16.0, 13.5)], rgb(240, 168, 48), W),
        ],
        // Desconocido — punto vectorial (jamás texto).
        _ => vec![Capa::rellena(Forma::Circulo { cx: 12.0, cy: 12.0, r: 3.0 }, corr())],
    };
    IconSpec::nuevo(title, capas)
}

#[cfg(test)]
mod tests {
    use super::*;
    use tullpu_icon_core::ColorFijo;

    #[test]
    fn todos_los_dientes_compilan_no_vacios() {
        let titulos = [
            "Vista", "Themes", "Atajos", "Animaciones", "Pata", "Inicio", "Sistema", "Acerca",
            "Correo", "Contextos",
        ];
        let r = ColorFijo::nuevo([200, 200, 200, 255]);
        for t in titulos {
            let spec = spec_diente(t);
            let capas = spec.compilar(&r);
            assert!(!capas.is_empty(), "{t} sin capas");
            // Toda capa debe terminar con relleno o trazo (algo que pintar).
            for pv in &capas {
                assert!(
                    pv.relleno.is_some() || pv.gradiente.is_some() || (pv.trazo.is_some() && pv.ancho_trazo > 0.0),
                    "{t}: capa sin pintura"
                );
            }
        }
    }

    #[test]
    fn desconocido_cae_a_punto_no_a_texto() {
        let spec = spec_diente("???");
        assert_eq!(spec.capas.len(), 1);
    }
}
