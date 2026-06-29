//! `foreign-svg` — puente SVG ↔ capas vectoriales de tullpu.
//!
//! - [`importar_svg`] parsea un `.svg` (vía `usvg`, que resuelve transforms,
//!   estilos y unidades, aplanando todo a paths con coords absolutas y paint
//!   resuelto) y devuelve un [`ParamsVector`] por path — listo para colgar como
//!   `ClaseCapa::Vector` y rasterizar con `tullpu_ops::rasterizar_vector`.
//! - [`exportar_svg`] toma capas vectoriales y arma un documento SVG a mano: un
//!   `<path>` por capa con su `d`, `fill`/`fill-opacity`/`fill-rule` y
//!   `stroke`/`stroke-width`. Round-trip `exportar → importar` preserva la
//!   geometría (ver tests).
//!
//! Lo que **no** se importa (post-MVP, como en `foreign-psd`): gradientes y
//! patrones (un fill no-sólido se omite), texto, imágenes embebidas y clips.

#![forbid(unsafe_code)]

use tullpu_core::{ComandoPath, ParamsVector, ReglaRelleno};

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("svg inválido: {0}")]
    Parse(String),
}

/// Resultado de importar un SVG: las capas vectoriales y el tamaño del lienzo
/// que declara el SVG (su viewBox/size), por si el caller quiere dimensionar.
#[derive(Debug, Clone)]
pub struct SvgImportado {
    pub capas: Vec<ParamsVector>,
    pub width: u32,
    pub height: u32,
}

/// Importa un `.svg` a capas vectoriales nativas. Cada path del SVG (ya con su
/// transform absoluto aplicado y su paint resuelto) se convierte en un
/// [`ParamsVector`]. El orden de `capas` respeta el orden de pintado del SVG
/// (fondo → frente), igual que `Lienzo::capas`.
pub fn importar_svg(bytes: &[u8]) -> Result<SvgImportado, Error> {
    let opt = usvg::Options::default();
    let tree = usvg::Tree::from_data(bytes, &opt).map_err(|e| Error::Parse(e.to_string()))?;
    let size = tree.size();
    let mut capas = Vec::new();
    recorrer(tree.root(), &mut capas);
    Ok(SvgImportado {
        capas,
        width: size.width().ceil() as u32,
        height: size.height().ceil() as u32,
    })
}

fn recorrer(grupo: &usvg::Group, out: &mut Vec<ParamsVector>) {
    for nodo in grupo.children() {
        match nodo {
            usvg::Node::Group(g) => recorrer(g, out),
            usvg::Node::Path(p) => {
                if let Some(pv) = path_a_vector(p) {
                    out.push(pv);
                }
            }
            // Texto/imágenes no son geometría vectorial editable: se omiten.
            usvg::Node::Image(_) | usvg::Node::Text(_) => {}
        }
    }
}

fn path_a_vector(p: &usvg::Path) -> Option<ParamsVector> {
    // Geometría en coords absolutas: aplicamos el transform acumulado del nodo.
    let abs = p.abs_transform();
    let data = p.data().clone();
    let data = if abs.is_identity() {
        data
    } else {
        data.transform(abs)?
    };

    let comandos = segmentos_a_comandos(&data);
    if comandos.is_empty() {
        return None;
    }

    let (relleno, regla) = match p.fill() {
        Some(f) => (
            color_solido(f.paint(), f.opacity()),
            match f.rule() {
                usvg::FillRule::EvenOdd => ReglaRelleno::ParImpar,
                usvg::FillRule::NonZero => ReglaRelleno::NoCero,
            },
        ),
        None => (None, ReglaRelleno::NoCero),
    };
    let (trazo, ancho_trazo) = match p.stroke() {
        Some(s) => (color_solido(s.paint(), s.opacity()), s.width().get()),
        None => (None, 0.0),
    };

    // Path sin relleno ni trazo visible: nada que pintar.
    if relleno.is_none() && trazo.is_none() {
        return None;
    }

    Some(ParamsVector {
        comandos,
        relleno,
        regla,
        trazo,
        ancho_trazo,
    })
}

/// Convierte un `tiny_skia_path::Path` a comandos de tullpu. Las cuádricas se
/// elevan a cúbicas (tullpu sólo modela líneas y cúbicas).
fn segmentos_a_comandos(data: &usvg::tiny_skia_path::Path) -> Vec<ComandoPath> {
    use usvg::tiny_skia_path::PathSegment as S;
    let mut comandos = Vec::new();
    let mut cur = (0.0f32, 0.0f32);
    for seg in data.segments() {
        match seg {
            S::MoveTo(p) => {
                cur = (p.x, p.y);
                comandos.push(ComandoPath::MoverA { x: p.x, y: p.y });
            }
            S::LineTo(p) => {
                cur = (p.x, p.y);
                comandos.push(ComandoPath::LineaA { x: p.x, y: p.y });
            }
            S::QuadTo(c, p) => {
                // Elevación cuádrica→cúbica: c1 = p0 + 2/3(c-p0), c2 = p1 + 2/3(c-p1).
                let c1 = (
                    cur.0 + 2.0 / 3.0 * (c.x - cur.0),
                    cur.1 + 2.0 / 3.0 * (c.y - cur.1),
                );
                let c2 = (
                    p.x + 2.0 / 3.0 * (c.x - p.x),
                    p.y + 2.0 / 3.0 * (c.y - p.y),
                );
                cur = (p.x, p.y);
                comandos.push(ComandoPath::CurvaA {
                    c1x: c1.0, c1y: c1.1, c2x: c2.0, c2y: c2.1, x: p.x, y: p.y,
                });
            }
            S::CubicTo(c1, c2, p) => {
                cur = (p.x, p.y);
                comandos.push(ComandoPath::CurvaA {
                    c1x: c1.x, c1y: c1.y, c2x: c2.x, c2y: c2.y, x: p.x, y: p.y,
                });
            }
            S::Close => comandos.push(ComandoPath::Cerrar),
        }
    }
    comandos
}

/// Color RGBA8 si el paint es un color sólido; `None` para gradientes/patrones
/// (no soportados). La opacidad del fill/stroke se funde al canal alfa.
fn color_solido(paint: &usvg::Paint, opacidad: usvg::Opacity) -> Option<[u8; 4]> {
    match paint {
        usvg::Paint::Color(c) => {
            let a = (opacidad.get() * 255.0).round().clamp(0.0, 255.0) as u8;
            Some([c.red, c.green, c.blue, a])
        }
        _ => None,
    }
}

// =============================================================================
//  Export
// =============================================================================

/// Exporta capas vectoriales a un documento SVG `width×height`. Cada
/// [`ParamsVector`] se vuelca a un `<path d=...>` con su relleno/trazo. El orden
/// del slice es el orden de pintado (el primero queda al fondo).
pub fn exportar_svg(capas: &[ParamsVector], width: u32, height: u32) -> String {
    let mut s = String::new();
    s.push_str(&format!(
        "<svg xmlns=\"http://www.w3.org/2000/svg\" width=\"{width}\" height=\"{height}\" viewBox=\"0 0 {width} {height}\">\n"
    ));
    for pv in capas {
        s.push_str("  <path d=\"");
        s.push_str(&path_d(&pv.comandos));
        s.push('"');
        match pv.relleno {
            Some(c) => {
                s.push_str(&format!(" fill=\"{}\"", hex_rgb(c)));
                if c[3] != 255 {
                    s.push_str(&format!(" fill-opacity=\"{:.4}\"", c[3] as f32 / 255.0));
                }
                if matches!(pv.regla, ReglaRelleno::ParImpar) {
                    s.push_str(" fill-rule=\"evenodd\"");
                }
            }
            None => s.push_str(" fill=\"none\""),
        }
        if let (Some(c), true) = (pv.trazo, pv.ancho_trazo > 0.0) {
            s.push_str(&format!(" stroke=\"{}\"", hex_rgb(c)));
            if c[3] != 255 {
                s.push_str(&format!(" stroke-opacity=\"{:.4}\"", c[3] as f32 / 255.0));
            }
            s.push_str(&format!(" stroke-width=\"{}\"", fmt_f(pv.ancho_trazo)));
        }
        s.push_str("/>\n");
    }
    s.push_str("</svg>\n");
    s
}

fn path_d(comandos: &[ComandoPath]) -> String {
    let mut d = String::new();
    for (i, cmd) in comandos.iter().enumerate() {
        if i > 0 {
            d.push(' ');
        }
        match *cmd {
            ComandoPath::MoverA { x, y } => {
                d.push_str(&format!("M {} {}", fmt_f(x), fmt_f(y)))
            }
            ComandoPath::LineaA { x, y } => {
                d.push_str(&format!("L {} {}", fmt_f(x), fmt_f(y)))
            }
            ComandoPath::CurvaA { c1x, c1y, c2x, c2y, x, y } => d.push_str(&format!(
                "C {} {} {} {} {} {}",
                fmt_f(c1x), fmt_f(c1y), fmt_f(c2x), fmt_f(c2y), fmt_f(x), fmt_f(y)
            )),
            ComandoPath::Cerrar => d.push('Z'),
        }
    }
    d
}

fn hex_rgb(c: [u8; 4]) -> String {
    format!("#{:02x}{:02x}{:02x}", c[0], c[1], c[2])
}

/// Formatea un float sin ceros de cola innecesarios (`12.0` → `12`).
fn fmt_f(v: f32) -> String {
    if v.fract() == 0.0 {
        format!("{}", v as i64)
    } else {
        let s = format!("{v:.4}");
        s.trim_end_matches('0').trim_end_matches('.').to_string()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn round_trip_rectangulo_preserva_geometria() {
        let pv = ParamsVector::rectangulo(10.0, 20.0, 30.0, 40.0, [200, 30, 40, 255]);
        let svg = exportar_svg(std::slice::from_ref(&pv), 100, 100);
        let imp = importar_svg(svg.as_bytes()).expect("reparsear");
        assert_eq!(imp.capas.len(), 1, "una capa");
        let r = &imp.capas[0];
        assert_eq!(r.relleno, Some([200, 30, 40, 255]));
        // El rect exportado es M/L/L/L/Z; usvg lo reparsea como un path
        // equivalente. Comprobamos la bbox de los puntos.
        let (mut minx, mut miny, mut maxx, mut maxy) = (f32::MAX, f32::MAX, f32::MIN, f32::MIN);
        for c in &r.comandos {
            if let ComandoPath::MoverA { x, y } | ComandoPath::LineaA { x, y } = *c {
                minx = minx.min(x); miny = miny.min(y);
                maxx = maxx.max(x); maxy = maxy.max(y);
            }
        }
        assert!((minx - 10.0).abs() < 0.5 && (miny - 20.0).abs() < 0.5, "esquina {minx},{miny}");
        assert!((maxx - 40.0).abs() < 0.5 && (maxy - 60.0).abs() < 0.5, "extremo {maxx},{maxy}");
    }

    #[test]
    fn importa_circle_y_rect_de_svg_crudo() {
        let svg = r##"<svg xmlns="http://www.w3.org/2000/svg" width="100" height="100">
            <rect x="10" y="10" width="20" height="20" fill="#ff0000"/>
            <circle cx="60" cy="60" r="15" fill="#00ff00" fill-opacity="0.5"/>
        </svg>"##;
        let imp = importar_svg(svg.as_bytes()).unwrap();
        assert_eq!(imp.capas.len(), 2, "rect + circle");
        assert_eq!(imp.capas[0].relleno, Some([255, 0, 0, 255]));
        // El círculo translúcido: alfa ~128, verde.
        let c = imp.capas[1].relleno.unwrap();
        assert_eq!([c[0], c[1], c[2]], [0, 255, 0]);
        assert!((c[3] as i32 - 128).abs() <= 2, "alfa ~128, fue {}", c[3]);
    }

    #[test]
    fn stroke_se_exporta_e_importa() {
        let mut pv = ParamsVector::rectangulo(5.0, 5.0, 10.0, 10.0, [0, 0, 0, 255]);
        pv.trazo = Some([10, 20, 30, 255]);
        pv.ancho_trazo = 3.0;
        let svg = exportar_svg(std::slice::from_ref(&pv), 50, 50);
        assert!(svg.contains("stroke=\"#0a141e\""), "svg: {svg}");
        assert!(svg.contains("stroke-width=\"3\""), "svg: {svg}");
        let imp = importar_svg(svg.as_bytes()).unwrap();
        let r = &imp.capas[0];
        assert_eq!(r.trazo, Some([10, 20, 30, 255]));
        assert!((r.ancho_trazo - 3.0).abs() < 0.01);
    }
}
