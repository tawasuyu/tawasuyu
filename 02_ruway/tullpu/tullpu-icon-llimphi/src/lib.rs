//! `tullpu-icon-llimphi` — el puente que hace que un [`IconSpec`] se pinte como
//! **vectores** en una UI Llimphi.
//!
//! Compila el spec a `Vec<ParamsVector>` (vía `tullpu-icon-core`) y pinta cada
//! capa directamente en el `Scene` de vello: relleno sólido, trazo y gradiente,
//! **multicolor**. A diferencia de pintar un glifo unicode con el motor de texto
//! (`text_aligned`), esto es **determinista en toda máquina** — no depende de
//! qué fuentes tenga el sistema, así que no hay "tofu"/notdef en hardware viejo.
//!
//! Es el análogo de `llimphi_icons::icon_view`, pero para íconos *generados*
//! (paramétricos o de la IA) y con color por capa en vez de un único trazo.
//!
//! ```ignore
//! // En un make_icon de dock_rail_view, o cualquier paint_with:
//! tullpu_icon_llimphi::spec_view(mi_spec, palette.fg_text)
//! ```

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{percent, Size, Style},
    Position,
};
use llimphi_ui::llimphi_raster::kurbo::{Affine, BezPath, Cap, Join, Point, Stroke};
use llimphi_ui::llimphi_raster::peniko::{Color, Fill, Gradient};
use llimphi_ui::llimphi_raster::vello::Scene;
use llimphi_ui::{PaintRect, View};
use tullpu_core::{ComandoPath, Gradiente, ParamsVector, ReglaRelleno};
use tullpu_icon_core::{ColorFijo, IconSpec, ResolverColor};

/// `peniko::Color` → RGBA8 (para alimentar el resolver del core).
fn a_rgba8(c: Color) -> [u8; 4] {
    let [r, g, b, a] = c.components;
    let q = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u8;
    [q(r), q(g), q(b), q(a)]
}

/// RGBA8 → `peniko::Color`.
fn de_rgba8(c: [u8; 4]) -> Color {
    Color::from_rgba8(c[0], c[1], c[2], c[3])
}

/// Traduce la lista de comandos de tullpu a un `BezPath` de kurbo. Es una
/// correspondencia 1:1 (ambos son listas de órdenes de path con cúbicas).
fn a_bezpath(comandos: &[ComandoPath]) -> BezPath {
    let mut p = BezPath::new();
    for c in comandos {
        match *c {
            ComandoPath::MoverA { x, y } => p.move_to((x as f64, y as f64)),
            ComandoPath::LineaA { x, y } => p.line_to((x as f64, y as f64)),
            ComandoPath::CurvaA { c1x, c1y, c2x, c2y, x, y } => p.curve_to(
                (c1x as f64, c1y as f64),
                (c2x as f64, c2y as f64),
                (x as f64, y as f64),
            ),
            ComandoPath::Cerrar => p.close_path(),
        }
    }
    p
}

fn a_gradiente(g: &Gradiente) -> Gradient {
    match g {
        Gradiente::Lineal { x1, y1, x2, y2, paradas } => {
            let stops: Vec<(f32, Color)> = paradas.iter().map(|(o, c)| (*o, de_rgba8(*c))).collect();
            Gradient::new_linear(Point::new(*x1 as f64, *y1 as f64), Point::new(*x2 as f64, *y2 as f64))
                .with_stops(stops.as_slice())
        }
        Gradiente::Radial { cx, cy, r, paradas } => {
            let stops: Vec<(f32, Color)> = paradas.iter().map(|(o, c)| (*o, de_rgba8(*c))).collect();
            Gradient::new_radial(Point::new(*cx as f64, *cy as f64), *r).with_stops(stops.as_slice())
        }
    }
}

/// Pinta una capa ya compilada. El `xform` mapea coords-grilla → rect destino;
/// el gradiente comparte ese mismo `xform` (brush sin transform propio), igual
/// que la forma, así que queda alineado.
fn pintar_capa(scene: &mut Scene, xform: Affine, pv: &ParamsVector) {
    let path = a_bezpath(&pv.comandos);
    let regla = match pv.regla {
        ReglaRelleno::ParImpar => Fill::EvenOdd,
        ReglaRelleno::NoCero => Fill::NonZero,
    };
    if let Some(g) = &pv.gradiente {
        scene.fill(regla, xform, &a_gradiente(g), None, &path);
    } else if let Some(c) = pv.relleno {
        scene.fill(regla, xform, &de_rgba8(c), None, &path);
    }
    if let (Some(c), true) = (pv.trazo, pv.ancho_trazo > 0.0) {
        let stroke = Stroke::new(pv.ancho_trazo as f64)
            .with_join(Join::Round)
            .with_caps(Cap::Round);
        scene.stroke(&stroke, xform, de_rgba8(c), None, &path);
    }
}

/// Afín que centra y escala la grilla `lienzo` dentro de `rect` (lado menor),
/// idéntica a la de `llimphi_icons::paint_icon`.
fn xform_para(rect: PaintRect, lienzo: f32) -> Option<Affine> {
    let side = rect.w.min(rect.h) as f64;
    if side <= 0.0 {
        return None;
    }
    let scale = side / (lienzo.max(1.0) as f64);
    let tx = rect.x as f64 + (rect.w as f64 - side) * 0.5;
    let ty = rect.y as f64 + (rect.h as f64 - side) * 0.5;
    Some(Affine::translate((tx, ty)) * Affine::scale(scale))
}

/// Pintor crudo: stampea un [`IconSpec`] en `scene` dentro de `rect`. `corriente`
/// resuelve los `Color::Corriente`/marca sin resolver del spec (currentColor).
/// Útil para componer varios íconos en un mismo `paint_with`.
pub fn pintar_spec(scene: &mut Scene, rect: PaintRect, spec: &IconSpec, corriente: Color) {
    let Some(xform) = xform_para(rect, spec.lienzo) else { return };
    let resolver = ColorFijo::nuevo(a_rgba8(corriente));
    for pv in spec.compilar(&resolver) {
        pintar_capa(scene, xform, &pv);
    }
}

/// Igual que [`pintar_spec`] pero con un [`ResolverColor`] propio — para mapear
/// `Color::Marca("cosmos")` a su color de marca real (catálogo del consumidor).
pub fn pintar_spec_con<R: ResolverColor>(scene: &mut Scene, rect: PaintRect, spec: &IconSpec, resolver: &R) {
    let Some(xform) = xform_para(rect, spec.lienzo) else { return };
    for pv in spec.compilar(resolver) {
        pintar_capa(scene, xform, &pv);
    }
}

fn estilo_icono() -> Style {
    Style {
        position: Position::Absolute,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        ..Default::default()
    }
}

/// `View` que pinta el `IconSpec` ocupando todo su rect (análogo a
/// `llimphi_icons::icon_view`). `corriente` = color para los `Corriente`.
pub fn spec_view<Msg: Clone + 'static>(spec: IconSpec, corriente: Color) -> View<Msg> {
    View::new(estilo_icono()).paint_with(move |scene, _ts, rect| {
        pintar_spec(scene, rect, &spec, corriente);
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use tullpu_icon_core::{Capa, Color as IcColor, Forma};

    #[test]
    fn comandos_a_bezpath_cuenta_elementos() {
        // Un círculo: MoverA + 4 cúbicas + Cerrar = 6 elementos en kurbo.
        let spec = IconSpec::nuevo(
            "c",
            vec![Capa::rellena(Forma::Circulo { cx: 12.0, cy: 12.0, r: 8.0 }, IcColor::Corriente)],
        );
        let capas = spec.compilar(&ColorFijo::nuevo([0, 0, 0, 255]));
        let bez = a_bezpath(&capas[0].comandos);
        assert_eq!(bez.elements().len(), 6);
    }

    #[test]
    fn pinta_sin_panic_en_scene() {
        // El render real no se puede "ver" en test, pero sí certificar que el
        // pipeline corre sin panics y emite al Scene (estructural: es vector).
        let spec = IconSpec::nuevo(
            "insignia",
            vec![
                Capa::rellena(Forma::RectRedondeado { x: 3.0, y: 3.0, w: 18.0, h: 18.0, r: 5.0 }, IcColor::Rgba([229, 91, 122, 255])),
                Capa::trazada(Forma::Estrella { cx: 12.0, cy: 12.0, r_ext: 6.0, r_int: 2.6, puntas: 5 }, IcColor::Corriente, 1.5),
            ],
        );
        let mut scene = Scene::new();
        let rect = PaintRect { x: 0.0, y: 0.0, w: 32.0, h: 32.0 };
        pintar_spec(&mut scene, rect, &spec, Color::from_rgba8(255, 255, 255, 255));
        // Si llegamos acá sin panic, el puente compiló geometría a vello.
    }

    #[test]
    fn rect_degenerado_no_pinta() {
        let spec = IconSpec::nuevo("x", vec![Capa::rellena(Forma::Circulo { cx: 12.0, cy: 12.0, r: 8.0 }, IcColor::Corriente)]);
        let mut scene = Scene::new();
        let rect = PaintRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 };
        pintar_spec(&mut scene, rect, &spec, Color::from_rgba8(0, 0, 0, 255));
        // No debe panic-ear con lado 0.
    }
}
