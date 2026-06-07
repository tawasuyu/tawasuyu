//! `llimphi-svg` — puente fino entre `vello_svg` y Llimphi.
//!
//! `llimphi-icons` cubre el set canónico de ~50 íconos a mano (BezPath en
//! grid 24×24). Para lo demás — íconos de apps `.desktop` arbitrarias,
//! logotipos, assets de marca, exports vectoriales de pineal/cosmos — hace
//! falta cargar SVG real. Este crate es el puente: parsea una vez con
//! `vello_svg` y stampea la `vello::Scene` resultante en cualquier `View`
//! escalándola al rect del nodo.
//!
//! ## Uso
//!
//! ```ignore
//! use llimphi_svg::SvgAsset;
//!
//! // Parsea UNA vez (al cargar la app o el ícono):
//! let svg = SvgAsset::from_str(include_str!("logo.svg")).expect("logo válido");
//!
//! // Pintalo en un View tantas veces como quieras (escala al rect):
//! View::new(style).children(vec![svg.view::<Msg>()])
//! ```
//!
//! El parse cuesta — el `view()` no. Si vas a stampear el mismo SVG en muchos
//! nodos (lista de apps con el mismo ícono fallback), parseá una sola vez y
//! cloneá el `SvgAsset` (es barato: `Arc` internamente).
//!
//! ## Por qué no parsear en cada paint
//!
//! `vello_svg::render` corre el parser de `usvg` (~ms por SVG no trivial). En
//! una lista con 80 íconos `.desktop`, parsear en cada frame mata el thread
//! de UI. La regla: **el asset se parsea una vez**, la `Scene` resultante se
//! retiene en memoria y se stampea con `scene.append(&inner, Some(xf))` —
//! cuesta lo mismo que dibujar el resto del UI.
//!
//! ## Errores
//!
//! `SvgAsset::from_str` devuelve `Result<Self, SvgError>` — si el XML está
//! corrupto o usa features que `usvg` no soporta. Las apps típicas tratan el
//! error como "fallback a glyph genérico" — no rompen.

#![forbid(unsafe_code)]

use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Position;
use llimphi_ui::llimphi_raster::kurbo::Affine;
use llimphi_ui::llimphi_raster::vello::Scene;
use llimphi_ui::{PaintRect, View};

/// Asset SVG parseado y listo para stampear. Internamente guarda la
/// `vello::Scene` ya construida + el viewBox del SVG (para escalar al rect del
/// nodo manteniendo aspecto). Cloneable barato (`Arc`) — un mismo asset en una
/// lista de 80 íconos no replica memoria.
#[derive(Clone)]
pub struct SvgAsset {
    inner: Arc<Inner>,
}

struct Inner {
    scene: Scene,
    /// Tamaño del viewBox en unidades del SVG (px nominales). Lo necesitamos
    /// para escalar el `scene.append(...)` al rect del nodo destino.
    vb_w: f64,
    vb_h: f64,
}

/// Errores al parsear un SVG. Hoy es un wrap del tipo de `vello_svg::Error` —
/// las apps típicas lo tratan como "fallback" y no inspeccionan la variante.
#[derive(Debug)]
pub struct SvgError(pub String);

impl std::fmt::Display for SvgError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "svg: {}", self.0)
    }
}

impl std::error::Error for SvgError {}

impl SvgAsset {
    /// Parsea un SVG desde su representación XML como string. Devuelve un asset
    /// inmutable + cloneable. **Hace el parseo completo** (usvg + render a una
    /// `vello::Scene`); pensado para llamarse UNA vez por asset, no por frame.
    pub fn from_str(svg: &str) -> Result<Self, SvgError> {
        let scene = vello_svg::render(svg).map_err(|e| SvgError(e.to_string()))?;
        // El size del viewBox sale del propio SVG: lo recuperamos con un parse
        // mínimo de usvg para no acoplar este crate a la geometría del SVG por
        // fuera de vello_svg.
        let opt = vello_svg::usvg::Options::default();
        let tree = vello_svg::usvg::Tree::from_str(svg, &opt).map_err(|e| SvgError(e.to_string()))?;
        let size = tree.size();
        Ok(Self {
            inner: Arc::new(Inner {
                scene,
                vb_w: size.width() as f64,
                vb_h: size.height() as f64,
            }),
        })
    }

    /// Parsea un SVG desde sus bytes UTF-8 crudos (lo que devuelve
    /// `include_bytes!("…svg")` o `std::fs::read`).
    pub fn from_bytes(svg: &[u8]) -> Result<Self, SvgError> {
        let s = std::str::from_utf8(svg).map_err(|e| SvgError(e.to_string()))?;
        Self::from_str(s)
    }

    /// Tamaño del viewBox del SVG en px nominales. Útil cuando el caller quiere
    /// dimensionar el rect destino preservando el aspect ratio en vez de dejar
    /// que el `view()` lo escale al máximo lado.
    pub fn size(&self) -> (f64, f64) {
        (self.inner.vb_w, self.inner.vb_h)
    }

    /// Pinta el SVG sobre `scene`, ajustado al `rect` indicado. Escala
    /// uniforme al mínimo lado y **centra** dentro del rect (preserva
    /// aspect ratio). Útil cuando el caller compone varios assets en un
    /// `paint_with` propio sin pasar por `view()`.
    pub fn paint(&self, scene: &mut Scene, rect: PaintRect) {
        let side_w = rect.w as f64;
        let side_h = rect.h as f64;
        if side_w <= 0.0 || side_h <= 0.0 || self.inner.vb_w <= 0.0 || self.inner.vb_h <= 0.0 {
            return;
        }
        let s = (side_w / self.inner.vb_w).min(side_h / self.inner.vb_h);
        let used_w = self.inner.vb_w * s;
        let used_h = self.inner.vb_h * s;
        let tx = rect.x as f64 + (side_w - used_w) * 0.5;
        let ty = rect.y as f64 + (side_h - used_h) * 0.5;
        let xform = Affine::translate((tx, ty)) * Affine::scale(s);
        scene.append(&self.inner.scene, Some(xform));
    }

    /// Construye un `View` posicionado en absoluto, que ocupa todo el rect del
    /// padre y pinta el SVG centrado + escalado al mínimo lado. Es el equivalente
    /// de `icon_view` para SVG arbitrarios. Genérico sobre `Msg` igual que los
    /// widgets — el `View` no tiene handlers; la app los pone en el padre.
    pub fn view<Msg>(&self) -> View<Msg> {
        let asset = self.clone();
        View::new(Style {
            position: Position::Absolute,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            ..Default::default()
        })
        .paint_with(move |scene, _ts, rect| {
            asset.paint(scene, rect);
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Un SVG mínimo: una caja roja de 24×24 con un círculo blanco al centro.
    const SVG_OK: &str = r##"<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 24 24" width="24" height="24">
        <rect width="24" height="24" fill="#cc0000"/>
        <circle cx="12" cy="12" r="6" fill="#ffffff"/>
    </svg>"##;

    #[test]
    fn from_str_parsea_ok() {
        let a = SvgAsset::from_str(SVG_OK).expect("parsea");
        let (w, h) = a.size();
        assert!((w - 24.0).abs() < 1e-6 && (h - 24.0).abs() < 1e-6);
    }

    #[test]
    fn svg_inválido_da_error() {
        let bad = "<no es svg>";
        assert!(SvgAsset::from_str(bad).is_err());
    }

    #[test]
    fn asset_es_cloneable_barato() {
        let a = SvgAsset::from_str(SVG_OK).expect("parsea");
        let b = a.clone();
        assert_eq!(a.size(), b.size());
    }

    #[test]
    fn paint_no_panica_con_rect_cero() {
        let a = SvgAsset::from_str(SVG_OK).expect("parsea");
        let mut s = Scene::new();
        a.paint(&mut s, PaintRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 });
        a.paint(&mut s, PaintRect { x: 0.0, y: 0.0, w: 10.0, h: 0.0 });
    }
}
