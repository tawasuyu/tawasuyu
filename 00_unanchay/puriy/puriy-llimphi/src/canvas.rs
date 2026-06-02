//! Canvas 2D: painter sobre vello + helpers de estilo/sombra/gradiente/patrón
//! + refresco de frames desde el runtime JS. Extraído de `lib.rs` (regla #1: un
//! archivo no debería pasar ~2k LOC). La superficie pública al resto del chrome
//! es `CanvasFrame`, `refresh_canvas_frames` y `render_canvas`; el resto es
//! privado al módulo (algunos `pub(crate)` sólo para los tests de `lib.rs`).
//!
//! Historia de fases: 7.196 (cableado a vello), 7.197 (gradientes/clip/dash),
//! 7.197b (drawImage), 7.198 (createPattern), 7.199 (sombras).

use llimphi_layout::taffy::prelude::{length, Size, Style};
use llimphi_raster::kurbo::{Affine, Cap, Join, Point, Rect as KurboRect, RoundedRect, Stroke};
use llimphi_raster::peniko::{
    BlendMode, Blob, Brush, Color, ColorStop, ColorStops, Compose, Extend, Fill, Gradient,
    GradientKind, Image as PenikoImage, ImageFormat, Mix,
};
use llimphi_ui::View;

use super::{Msg, TabState};

/// Frame de un `<canvas>` 2D recolectado del runtime JS (Fase 7.196).
/// Espejo de lo que devuelve `__puriy_collect_canvas()`: el id del elemento,
/// su tamaño intrínseco y la lista de comandos de dibujo (cada uno un array
/// `[op, ...args]`, con un snapshot de estilo apendido en los que pintan).
#[derive(serde::Deserialize, Clone, Debug, Default)]
pub(crate) struct CanvasFrame {
    pub(crate) id: String,
    #[serde(default)]
    pub(crate) width: f32,
    #[serde(default)]
    pub(crate) height: f32,
    #[serde(default)]
    pub(crate) cmds: Vec<Vec<serde_json::Value>>,
}

/// Refresca `t.canvas_frames` evaluando `__puriy_collect_canvas()` en el
/// runtime y parseando el JSON. Llamado tras correr scripts, en cada tick y
/// tras dispatchear eventos — cualquier momento en que el JS pudo dibujar.
/// Barato cuando no hay canvas: `canvas_json` devuelve `None` (un `eval` mini).
pub(crate) fn refresh_canvas_frames(t: &mut TabState) {
    let Some(rt) = t.js.as_mut() else { return };
    match rt.canvas_json() {
        Some(json) => match serde_json::from_str::<Vec<CanvasFrame>>(&json) {
            Ok(frames) => {
                t.canvas_frames.clear();
                for f in frames {
                    t.canvas_frames.insert(f.id.clone(), f);
                }
            }
            Err(_) => t.canvas_frames.clear(),
        },
        None => t.canvas_frames.clear(),
    }
    decode_canvas_images(t);
}

/// Decodifica (una vez) las imágenes referenciadas por comandos `drawImage`
/// de los frames de canvas, resolviendo cada `src` crudo contra `t.url` vía el
/// cache de imágenes del engine. El resultado (o `None` si falla) queda en
/// `t.canvas_images` para que el painter lo busque sin re-decodificar cada
/// frame. Fase 7.197b.
pub(crate) fn decode_canvas_images(t: &mut TabState) {
    // Recolecta los src referenciados (drawImage + patrones createPattern),
    // deduplicados — préstamo separado de `canvas_frames` (inmutable) y
    // `canvas_images` (mutable). Luego filtra los ya decodificados.
    let mut candidatos: Vec<String> = Vec::new();
    for frame in t.canvas_frames.values() {
        for cmd in &frame.cmds {
            // drawImage: el src va en el arg 1.
            if cmd.first().and_then(|v| v.as_str()) == Some("drawImage") {
                if let Some(src) = cmd.get(1).and_then(|v| v.as_str()) {
                    if !src.is_empty() && !candidatos.iter().any(|s| s == src) {
                        candidatos.push(src.to_string());
                    }
                }
            }
            // createPattern: descriptores {_pattern,src,rep} anidados en los
            // snapshots de estilo (fill/stroke/rect) — escaneo recursivo.
            for v in cmd {
                collect_pattern_srcs(v, &mut candidatos);
            }
        }
    }
    let nuevos: Vec<String> = candidatos
        .into_iter()
        .filter(|s| !t.canvas_images.contains_key(s))
        .collect();
    if nuevos.is_empty() {
        return;
    }
    let base = url::Url::parse(&t.url).ok();
    for src in nuevos {
        let img = puriy_engine::fetch_image_src(base.as_ref(), &src).map(|d| {
            let blob = Blob::from(d.rgba);
            PenikoImage::new(blob, ImageFormat::Rgba8, d.width, d.height)
        });
        t.canvas_images.insert(src, img);
    }
}

/// Extrae un `f64` de un valor JSON (default 0.0 si no es número).
fn cnum(v: Option<&serde_json::Value>) -> f64 {
    v.and_then(|x| x.as_f64()).unwrap_or(0.0)
}

/// Resuelve `fillStyle`/`strokeStyle` (string color CSS o objeto
/// `CanvasGradient`) a un color sólido peniko, multiplicando el alpha por
/// `ga` (globalAlpha). Los gradientes se degradan al color de su último
/// stop (MVP — sin gradiente real todavía). Default negro opaco.
pub(crate) fn canvas_color(v: Option<&serde_json::Value>, ga: f64) -> Color {
    let base = match v {
        Some(serde_json::Value::String(s)) => puriy_engine::parse_color(s),
        Some(serde_json::Value::Object(o)) => {
            // CanvasGradient: { _kind, _coords, _stops: [[offset, color], ...] }
            o.get("_stops")
                .and_then(|s| s.as_array())
                .and_then(|arr| arr.last())
                .and_then(|stop| stop.as_array())
                .and_then(|pair| pair.get(1))
                .and_then(|c| c.as_str())
                .and_then(puriy_engine::parse_color)
        }
        _ => None,
    }
    .unwrap_or(puriy_engine::Color { r: 0, g: 0, b: 0, a: 255 });
    let a = ((base.a as f64) * ga).clamp(0.0, 255.0) as u8;
    Color::from_rgba8(base.r, base.g, base.b, a)
}

/// Lee el estado de sombra del snapshot (`sc`/`sb`/`sox`/`soy`) y lo resuelve a
/// `(color, blur, offX, offY)`, multiplicando el alpha del color por `ga`
/// (globalAlpha). Devuelve `None` si la sombra está inactiva: sin campo `sc`,
/// color totalmente transparente, o blur 0 y ambos offsets en 0. Fase 7.199.
pub(crate) fn canvas_shadow(st: Option<&serde_json::Value>, ga: f64) -> Option<(Color, f64, f64, f64)> {
    let col = style_str(st, "sc").and_then(|s| puriy_engine::parse_color(&s))?;
    if col.a == 0 {
        return None;
    }
    let blur = style_field(st, "sb").unwrap_or(0.0).max(0.0);
    let ox = style_field(st, "sox").unwrap_or(0.0);
    let oy = style_field(st, "soy").unwrap_or(0.0);
    if blur <= 0.0 && ox == 0.0 && oy == 0.0 {
        return None;
    }
    let a = ((col.a as f64) * ga).clamp(0.0, 255.0) as u8;
    Some((Color::from_rgba8(col.r, col.g, col.b, a), blur, ox, oy))
}

/// Resuelve `globalCompositeOperation` (campo `gco` del snapshot) a un
/// `peniko::BlendMode` de vello. Devuelve `None` para `source-over` (el default
/// — no hace falta capa de blend) o un modo desconocido. Los modos de *mezcla*
/// (multiply/screen/overlay/…) mapean a `Mix` (compose SrcOver); los Porter-Duff
/// (lighter/copy/destination-out/…) a `Compose` (mix Normal). El chrome envuelve
/// el dibujo de la op en `push_layer(blend, base, todo_el_canvas)` para que la
/// composición se evalúe contra el backdrop del canvas entero. Fase 7.200.
pub(crate) fn canvas_composite(st: Option<&serde_json::Value>) -> Option<BlendMode> {
    let gco = style_str(st, "gco")?;
    let bm: BlendMode = match gco.as_str() {
        "multiply" => Mix::Multiply.into(),
        "screen" => Mix::Screen.into(),
        "overlay" => Mix::Overlay.into(),
        "darken" => Mix::Darken.into(),
        "lighten" => Mix::Lighten.into(),
        "color-dodge" => Mix::ColorDodge.into(),
        "color-burn" => Mix::ColorBurn.into(),
        "hard-light" => Mix::HardLight.into(),
        "soft-light" => Mix::SoftLight.into(),
        "difference" => Mix::Difference.into(),
        "exclusion" => Mix::Exclusion.into(),
        "hue" => Mix::Hue.into(),
        "saturation" => Mix::Saturation.into(),
        "color" => Mix::Color.into(),
        "luminosity" => Mix::Luminosity.into(),
        "lighter" => Compose::Plus.into(),
        "plus-lighter" => Compose::PlusLighter.into(),
        "copy" => Compose::Copy.into(),
        "destination-over" => Compose::DestOver.into(),
        "source-in" => Compose::SrcIn.into(),
        "destination-in" => Compose::DestIn.into(),
        "source-out" => Compose::SrcOut.into(),
        "destination-out" => Compose::DestOut.into(),
        "source-atop" => Compose::SrcAtop.into(),
        "destination-atop" => Compose::DestAtop.into(),
        "xor" => Compose::Xor.into(),
        // "source-over" (default) y desconocidos → sin capa.
        _ => return None,
    };
    Some(bm)
}

/// Recolecta (recursivamente, deduplicando contra `out`) los `src` de los
/// descriptores de patrón `{_pattern:true, src, rep}` que `createPattern`
/// inyecta en los snapshots de estilo. Se reusa el mismo mapa de imágenes
/// decodificadas que `drawImage` (Fase 7.198).
fn collect_pattern_srcs(v: &serde_json::Value, out: &mut Vec<String>) {
    match v {
        serde_json::Value::Object(o) => {
            if o.get("_pattern").and_then(|p| p.as_bool()) == Some(true) {
                if let Some(src) = o.get("src").and_then(|s| s.as_str()) {
                    if !src.is_empty() && !out.iter().any(|s2| s2 == src) {
                        out.push(src.to_string());
                    }
                }
            }
            for val in o.values() {
                collect_pattern_srcs(val, out);
            }
        }
        serde_json::Value::Array(a) => {
            for val in a {
                collect_pattern_srcs(val, out);
            }
        }
        _ => {}
    }
}

/// Construye un `Brush::Image` desde un descriptor de patrón
/// `{_pattern, src, rep}` (Fase 7.198). `rep` mapea a los modos de extensión
/// de peniko: `repeat`→Repeat/Repeat, `repeat-x`→Repeat/Pad, `repeat-y`→
/// Pad/Repeat, `no-repeat`→Pad/Pad. `ga` (globalAlpha) se aplica como
/// multiplicador de alpha de la imagen. Devuelve `None` si falta el src o la
/// imagen no está decodificada (el caller cae a color sólido). El patrón se
/// ancla al origen del espacio de usuario del canvas (el caller pinta con
/// `transform = base*cur`, `brush_transform = None`).
fn build_canvas_pattern(
    o: &serde_json::Map<String, serde_json::Value>,
    ga: f64,
    images: &std::collections::HashMap<String, PenikoImage>,
) -> Option<Brush> {
    let src = o.get("src")?.as_str()?;
    if src.is_empty() {
        return None;
    }
    let img = images.get(src)?;
    let (xe, ye) = match o.get("rep").and_then(|r| r.as_str()).unwrap_or("repeat") {
        "repeat-x" => (Extend::Repeat, Extend::Pad),
        "repeat-y" => (Extend::Pad, Extend::Repeat),
        "no-repeat" => (Extend::Pad, Extend::Pad),
        _ => (Extend::Repeat, Extend::Repeat),
    };
    let img = img
        .clone()
        .with_x_extend(xe)
        .with_y_extend(ye)
        .with_alpha(ga.clamp(0.0, 1.0) as f32);
    Some(Brush::Image(img))
}

/// Resuelve `fillStyle`/`strokeStyle` a un `peniko::Brush`: un descriptor de
/// patrón (`createPattern`) → `Brush::Image` tileado (Fase 7.198); un objeto
/// `CanvasGradient` con stops válidos → gradiente REAL (linear/radial/conic→
/// sweep); si no, degrada a color sólido (vía `canvas_color`). `ga`
/// (globalAlpha) multiplica el alpha de cada stop / la imagen. Las coordenadas
/// del gradiente/patrón quedan en el espacio de usuario del canvas: el caller
/// lo pinta con `transform = base*cur` y `brush_transform = None`, así se
/// posiciona en ese mismo espacio (Fase 7.197/7.198).
pub(crate) fn canvas_brush(
    v: Option<&serde_json::Value>,
    ga: f64,
    images: &std::collections::HashMap<String, PenikoImage>,
) -> Brush {
    if let Some(serde_json::Value::Object(o)) = v {
        if o.get("_pattern").and_then(|p| p.as_bool()) == Some(true) {
            if let Some(b) = build_canvas_pattern(o, ga, images) {
                return b;
            }
        }
        if let Some(g) = build_canvas_gradient(o, ga) {
            return Brush::Gradient(g);
        }
    }
    Brush::Solid(canvas_color(v, ga))
}

/// Construye un `peniko::Gradient` desde el objeto JS `CanvasGradient`
/// (`{_kind, _coords, _stops:[[offset,color],...]}`). Devuelve `None` si el
/// tipo es desconocido, faltan stops (<2) o algún color no parsea — el caller
/// cae a color sólido. `linear`: coords `[x0,y0,x1,y1]`; `radial`:
/// `[x0,y0,r0,x1,y1,r1]`; `conic`: `[angle,x,y]` (→ `Sweep`, orientación
/// aproximada — peniko mide CCW desde +x, canvas CW desde arriba).
fn build_canvas_gradient(
    o: &serde_json::Map<String, serde_json::Value>,
    ga: f64,
) -> Option<Gradient> {
    let kind = o.get("_kind")?.as_str()?;
    let coords: Vec<f64> = o
        .get("_coords")?
        .as_array()?
        .iter()
        .map(|v| v.as_f64().unwrap_or(0.0))
        .collect();
    let c = |i: usize| coords.get(i).copied().unwrap_or(0.0);
    let stops_json = o.get("_stops")?.as_array()?;
    if stops_json.len() < 2 {
        return None;
    }
    let mut stops: Vec<ColorStop> = Vec::with_capacity(stops_json.len());
    for s in stops_json {
        let pair = s.as_array()?;
        let off = pair.first()?.as_f64()? as f32;
        let col = pair.get(1)?.as_str().and_then(puriy_engine::parse_color)?;
        let a = ((col.a as f64) * ga).clamp(0.0, 255.0) as u8;
        stops.push(ColorStop::from((off, Color::from_rgba8(col.r, col.g, col.b, a))));
    }
    let kind = match kind {
        "linear" => GradientKind::Linear {
            start: Point::new(c(0), c(1)),
            end: Point::new(c(2), c(3)),
        },
        "radial" => GradientKind::Radial {
            start_center: Point::new(c(0), c(1)),
            start_radius: c(2) as f32,
            end_center: Point::new(c(3), c(4)),
            end_radius: c(5) as f32,
        },
        "conic" => {
            let ang = c(0) as f32;
            GradientKind::Sweep {
                center: Point::new(c(1), c(2)),
                start_angle: ang,
                end_angle: ang + std::f32::consts::TAU,
            }
        }
        _ => return None,
    };
    Some(Gradient {
        kind,
        stops: ColorStops(stops.into()),
        ..Default::default()
    })
}

/// Arma un `kurbo::Stroke` desde el snapshot de estilo: ancho `lw`, `lineCap`
/// (`lc`), `lineJoin` (`lj`) y `setLineDash`/`lineDashOffset` (`ld`/`ldo`). Un
/// patrón de dash de longitud impar se duplica (semántica de canvas). Fase 7.197.
pub(crate) fn canvas_stroke(st: Option<&serde_json::Value>, lw: f64) -> Stroke {
    let cap = match style_str(st, "lc").as_deref() {
        Some("round") => Cap::Round,
        Some("square") => Cap::Square,
        _ => Cap::Butt,
    };
    let join = match style_str(st, "lj").as_deref() {
        Some("round") => Join::Round,
        Some("bevel") => Join::Bevel,
        _ => Join::Miter,
    };
    let mut stroke = Stroke::new(lw).with_caps(cap).with_join(join);
    let mut pattern: Vec<f64> = st
        .and_then(|v| v.as_object())
        .and_then(|o| o.get("ld"))
        .and_then(|d| d.as_array())
        .map(|arr| arr.iter().filter_map(|x| x.as_f64()).collect())
        .unwrap_or_default();
    if pattern.iter().any(|d| *d > 0.0) {
        if pattern.len() % 2 == 1 {
            let dup = pattern.clone();
            pattern.extend(dup);
        }
        let offset = style_field(st, "ldo").unwrap_or(0.0);
        stroke = stroke.with_dashes(offset, pattern);
    }
    stroke
}

/// Px de fuente parseados de un string CSS `font` tipo `"16px sans-serif"`.
pub(crate) fn canvas_font_px(font: Option<&str>) -> f32 {
    let f = font.unwrap_or("10px sans-serif");
    // Busca "<num>px".
    if let Some(idx) = f.find("px") {
        let start = f[..idx]
            .rfind(|c: char| !(c.is_ascii_digit() || c == '.'))
            .map(|i| i + 1)
            .unwrap_or(0);
        if let Ok(v) = f[start..idx].parse::<f32>() {
            if v > 0.0 {
                return v;
            }
        }
    }
    10.0
}

/// Renderiza un `<canvas>` 2D: un View del tamaño intrínseco (escalado por
/// zoom) cuyo `paint_with` interpreta el log de comandos del frame con vello.
/// Si no hay frame (el script aún no pidió contexto / dibujó), devuelve el
/// View vacío (rect transparente). Fase 7.196.
pub(crate) fn render_canvas(
    frame: Option<&CanvasFrame>,
    images: &std::collections::HashMap<String, Option<PenikoImage>>,
    intrinsic_w: f32,
    intrinsic_h: f32,
    zoom: f32,
) -> View<Msg> {
    // El View se muestra al tamaño del box (atributos width/height del engine,
    // escalado por zoom). El espacio de COORDENADAS de los comandos es el
    // tamaño del buffer de dibujo (`frame.width/height`, que un script pudo
    // cambiar vía `canvas.width = N`); si no hay frame, cae al intrínseco.
    let w = intrinsic_w * zoom;
    let h = intrinsic_h * zoom;
    let cmds: Vec<Vec<serde_json::Value>> = frame.map(|f| f.cmds.clone()).unwrap_or_default();
    let iw = frame
        .map(|f| f.width)
        .filter(|v| *v > 0.0)
        .unwrap_or(intrinsic_w)
        .max(1.0) as f64;
    let ih = frame
        .map(|f| f.height)
        .filter(|v| *v > 0.0)
        .unwrap_or(intrinsic_h)
        .max(1.0) as f64;
    // Sólo las imágenes que ESTE frame referencia (decodificadas), clonadas
    // al closure (peniko::Image es Arc-backed → clon barato).
    let mut frame_images: std::collections::HashMap<String, PenikoImage> =
        std::collections::HashMap::new();
    let mut refs: Vec<String> = Vec::new();
    for cmd in &cmds {
        if cmd.first().and_then(|v| v.as_str()) == Some("drawImage") {
            if let Some(src) = cmd.get(1).and_then(|v| v.as_str()) {
                if !src.is_empty() && !refs.iter().any(|s| s == src) {
                    refs.push(src.to_string());
                }
            }
        }
        // Patrones (createPattern): src anidado en los snapshots de estilo.
        for v in cmd {
            collect_pattern_srcs(v, &mut refs);
        }
    }
    for src in refs {
        if !frame_images.contains_key(&src) {
            if let Some(Some(img)) = images.get(&src) {
                frame_images.insert(src, img.clone());
            }
        }
    }
    View::new(Style {
        size: Size { width: length(w), height: length(h) },
        ..Default::default()
    })
    .paint_with(move |scene, ts, rect| {
        paint_canvas_cmds(scene, ts, rect, &cmds, iw, ih, &frame_images);
    })
}

/// Interpreta el log de comandos 2D contra `scene` (vello), mapeando el
/// espacio de usuario del canvas (0..iw, 0..ih) al `rect` de pantalla. MVP:
/// soporta fill/stroke de paths (move/line/bezier/quad/arc/ellipse/rect/
/// roundRect/closePath), fillRect/strokeRect, fillText/strokeText, los
/// transforms (save/restore/translate/scale/rotate/transform/setTransform/
/// resetTransform/beginPath), globalAlpha, gradientes REALES (linear/radial/
/// conic→sweep) en fill/stroke/rect, `clip` (recorte por path, balanceado con
/// save/restore), line dash/cap/join (Fase 7.197), `drawImage` de imágenes
/// decodificadas (Fase 7.197b, vía el mapa `images` keyeado por `src`),
/// patrones `createPattern` (Fase 7.198), sombras `shadow*` (Fase 7.199:
/// `fillRect` con blur gaussiano real, el resto silueta desplazada) y
/// `globalCompositeOperation` (Fase 7.200: blend modes de vello vía
/// `push_layer`, en fill/stroke/rect/text — ver `canvas_composite`).
/// Limitaciones: putImageData/getImageData (sin buffer CPU), clearRect parcial,
/// y `globalCompositeOperation`/`globalAlpha`/sombra sobre drawImage quedan
/// fuera (su comando no lleva snapshot); el texto con gradiente degrada a color
/// sólido (el typesetter sólo toma `Color`).
pub(crate) fn paint_canvas_cmds(
    scene: &mut llimphi_raster::vello::Scene,
    ts: &mut llimphi_ui::llimphi_text::Typesetter,
    rect: llimphi_ui::PaintRect,
    cmds: &[Vec<serde_json::Value>],
    iw: f64,
    ih: f64,
    images: &std::collections::HashMap<String, PenikoImage>,
) {
    use llimphi_raster::kurbo::{BezPath, Shape};

    // base: espacio de usuario del canvas → rect de pantalla.
    let sx = rect.w as f64 / iw;
    let sy = rect.h as f64 / ih;
    let base = Affine::translate((rect.x as f64, rect.y as f64))
        * Affine::scale_non_uniform(sx, sy);
    // Rect del canvas entero (espacio usuario) — clip de las capas de blend de
    // `globalCompositeOperation`, para que la composición se evalúe contra el
    // backdrop de todo el canvas (Fase 7.200).
    let whole = KurboRect::new(0.0, 0.0, iw, ih);

    let mut cur = Affine::IDENTITY; // transform actual del canvas (espacio usuario)
    let mut tstack: Vec<Affine> = Vec::new();
    let mut path = BezPath::new();
    // Clips abiertos (push_layer) por nivel de save; `base_clips` los que se
    // abrieron fuera de cualquier save. Se balancean en restore/reset y al
    // terminar para no dejar layers colgando en la escena (Fase 7.197).
    let mut clip_stack: Vec<u32> = Vec::new();
    let mut base_clips: u32 = 0;
    let pop_clips = |scene: &mut llimphi_raster::vello::Scene,
                         clip_stack: &mut Vec<u32>,
                         base_clips: &mut u32| {
        while let Some(n) = clip_stack.pop() {
            for _ in 0..n {
                scene.pop_layer();
            }
        }
        for _ in 0..*base_clips {
            scene.pop_layer();
        }
        *base_clips = 0;
    };

    for cmd in cmds {
        let Some(op) = cmd.first().and_then(|v| v.as_str()) else { continue };
        let a = |i: usize| cnum(cmd.get(i));
        match op {
            "save" => {
                tstack.push(cur);
                clip_stack.push(0);
            }
            "restore" => {
                if let Some(t) = tstack.pop() {
                    cur = t;
                }
                if let Some(n) = clip_stack.pop() {
                    for _ in 0..n {
                        scene.pop_layer();
                    }
                }
            }
            "translate" => cur *= Affine::translate((a(1), a(2))),
            "scale" => cur *= Affine::scale_non_uniform(a(1), a(2)),
            "rotate" => cur *= Affine::rotate(a(1)),
            "transform" => {
                cur *= Affine::new([a(1), a(2), a(3), a(4), a(5), a(6)]);
            }
            "setTransform" => {
                cur = Affine::new([a(1), a(2), a(3), a(4), a(5), a(6)]);
            }
            "resetTransform" | "reset" => {
                cur = Affine::IDENTITY;
                if op == "reset" {
                    path = BezPath::new();
                    tstack.clear();
                    pop_clips(scene, &mut clip_stack, &mut base_clips);
                }
            }
            "beginPath" => path = BezPath::new(),
            "closePath" => path.close_path(),
            "moveTo" => path.move_to((a(1), a(2))),
            "lineTo" => path.line_to((a(1), a(2))),
            "bezierCurveTo" => path.curve_to((a(1), a(2)), (a(3), a(4)), (a(5), a(6))),
            "quadraticCurveTo" => path.quad_to((a(1), a(2)), (a(3), a(4))),
            "rect" => {
                let (x, y, w, h) = (a(1), a(2), a(3), a(4));
                path.move_to((x, y));
                path.line_to((x + w, y));
                path.line_to((x + w, y + h));
                path.line_to((x, y + h));
                path.close_path();
            }
            "roundRect" => {
                // MVP: radio uniforme (primer valor) si lo hay, sino 0.
                let (x, y, w, h) = (a(1), a(2), a(3), a(4));
                let r = cmd.get(5).and_then(|v| v.as_f64()).unwrap_or(0.0);
                let rr = RoundedRect::new(x, y, x + w, y + h, r);
                path.extend(rr.path_elements(0.1));
            }
            "arc" => {
                // arc(x, y, r, start, end, ccw=false)
                let (cx, cy, r, start, end) = (a(1), a(2), a(3), a(4), a(5));
                let ccw = cmd.get(6).and_then(|v| v.as_bool()).unwrap_or(false);
                append_arc(&mut path, cx, cy, r, r, 0.0, start, end, ccw);
            }
            "ellipse" => {
                // ellipse(x, y, rx, ry, rotation, start, end, ccw=false)
                let (cx, cy, rx, ry, rot, start, end) =
                    (a(1), a(2), a(3), a(4), a(5), a(6), a(7));
                let ccw = cmd.get(8).and_then(|v| v.as_bool()).unwrap_or(false);
                append_arc(&mut path, cx, cy, rx, ry, rot, start, end, ccw);
            }
            "arcTo" => {
                // MVP: línea al primer punto de control (aproximación).
                path.line_to((a(1), a(2)));
            }
            "fill" => {
                let st = cmd.get(1);
                let ga = style_field(st, "ga").unwrap_or(1.0);
                let comp = canvas_composite(st);
                if let Some(bm) = comp {
                    scene.push_layer(bm, 1.0, base, &whole);
                }
                if let Some((col, _b, ox, oy)) = canvas_shadow(st, ga) {
                    let sxf = base * cur * Affine::translate((ox, oy));
                    scene.fill(Fill::NonZero, sxf, &Brush::Solid(col), None, &path);
                }
                let brush = canvas_brush(style_color(st, "f").as_ref(), ga, images);
                scene.fill(Fill::NonZero, base * cur, &brush, None, &path);
                if comp.is_some() {
                    scene.pop_layer();
                }
            }
            "stroke" => {
                let st = cmd.get(1);
                let ga = style_field(st, "ga").unwrap_or(1.0);
                let lw = style_field(st, "lw").unwrap_or(1.0).max(0.01);
                let stroke = canvas_stroke(st, lw);
                let comp = canvas_composite(st);
                if let Some(bm) = comp {
                    scene.push_layer(bm, 1.0, base, &whole);
                }
                if let Some((col, _b, ox, oy)) = canvas_shadow(st, ga) {
                    let sxf = base * cur * Affine::translate((ox, oy));
                    scene.stroke(&stroke, sxf, &Brush::Solid(col), None, &path);
                }
                let brush = canvas_brush(style_color(st, "s").as_ref(), ga, images);
                scene.stroke(&stroke, base * cur, &brush, None, &path);
                if comp.is_some() {
                    scene.pop_layer();
                }
            }
            "clip" => {
                // Recorta el dibujo posterior al path actual. La capa se cierra
                // en el `restore` que cierra el `save` correspondiente (o al
                // terminar el frame si no hubo save).
                scene.push_layer(Mix::Clip, 1.0, base * cur, &path);
                match clip_stack.last_mut() {
                    Some(top) => *top += 1,
                    None => base_clips += 1,
                }
            }
            "fillRect" => {
                // ['fillRect', x, y, w, h, fillStyle, snapshot]
                let ga = style_field(cmd.get(6), "ga").unwrap_or(1.0);
                let r = KurboRect::new(a(1), a(2), a(1) + a(3), a(2) + a(4));
                let comp = canvas_composite(cmd.get(6));
                if let Some(bm) = comp {
                    scene.push_layer(bm, 1.0, base, &whole);
                }
                // Sombra REAL blureada (gaussiana) — el caso estrella (cards,
                // botones): rect desplazado por (ox,oy), std_dev = blur/2.
                if let Some((col, blur, ox, oy)) = canvas_shadow(cmd.get(6), ga) {
                    let sr = KurboRect::new(a(1) + ox, a(2) + oy, a(1) + a(3) + ox, a(2) + a(4) + oy);
                    scene.draw_blurred_rounded_rect(base * cur, sr, col, 0.0, (blur * 0.5).max(0.0));
                }
                let brush = canvas_brush(cmd.get(5), ga, images);
                scene.fill(Fill::NonZero, base * cur, &brush, None, &r);
                if comp.is_some() {
                    scene.pop_layer();
                }
            }
            "strokeRect" => {
                let st = cmd.get(6);
                let ga = style_field(st, "ga").unwrap_or(1.0);
                let lw = style_field(st, "lw").unwrap_or(1.0).max(0.01);
                let stroke = canvas_stroke(st, lw);
                let r = KurboRect::new(a(1), a(2), a(1) + a(3), a(2) + a(4));
                let comp = canvas_composite(st);
                if let Some(bm) = comp {
                    scene.push_layer(bm, 1.0, base, &whole);
                }
                if let Some((col, _b, ox, oy)) = canvas_shadow(st, ga) {
                    let sxf = base * cur * Affine::translate((ox, oy));
                    scene.stroke(&stroke, sxf, &Brush::Solid(col), None, &r);
                }
                let brush = canvas_brush(cmd.get(5), ga, images);
                scene.stroke(&stroke, base * cur, &brush, None, &r);
                if comp.is_some() {
                    scene.pop_layer();
                }
            }
            "fillText" | "strokeText" => {
                // ['fillText', text, x, y, maxWidth, snapshot]
                let text = cmd.get(1).and_then(|v| v.as_str()).unwrap_or("");
                if text.is_empty() {
                    continue;
                }
                let (x, y) = (a(2), a(3));
                let st = cmd.get(5);
                let ga = style_field(st, "ga").unwrap_or(1.0);
                let comp = canvas_composite(st);
                if let Some(bm) = comp {
                    scene.push_layer(bm, 1.0, base, &whole);
                }
                let key = if op == "fillText" { "f" } else { "s" };
                let color = canvas_color(style_color(st, key).as_ref(), ga);
                let px = canvas_font_px(style_str(st, "fnt").as_deref());
                let layout = ts.layout(
                    text,
                    px,
                    None,
                    llimphi_ui::llimphi_text::Alignment::Start,
                    1.0,
                    false,
                    None,
                );
                // textAlign: ajusta x. Baseline alphabetic ⇒ subimos ~0.8em.
                let tw = layout.width() as f64;
                let align = style_str(st, "ta").unwrap_or_default();
                let dx = match align.as_str() {
                    "center" => -tw / 2.0,
                    "right" | "end" => -tw,
                    _ => 0.0,
                };
                let ascent = (px as f64) * 0.8;
                // Sombra de texto: los glifos reales en color de sombra,
                // desplazados (blur crisp — el typesetter sólo toma Color).
                if let Some((scol, _b, ox, oy)) = canvas_shadow(st, ga) {
                    let sxf = base * cur * Affine::translate((x + dx + ox, y - ascent + oy));
                    llimphi_ui::llimphi_text::draw_layout_xf(scene, &layout, scol, sxf);
                }
                let xf = base * cur * Affine::translate((x + dx, y - ascent));
                llimphi_ui::llimphi_text::draw_layout_xf(scene, &layout, color, xf);
                if comp.is_some() {
                    scene.pop_layer();
                }
            }
            "drawImage" => {
                // ['drawImage', src, ...nums] — nums: 2 (dx,dy), 4 (dx,dy,dw,dh)
                // u 8 (sx,sy,sw,sh,dx,dy,dw,dh). El source rect default es la
                // imagen entera (W×H decodificados).
                let src = cmd.get(1).and_then(|v| v.as_str()).unwrap_or("");
                let Some(img) = images.get(src) else { continue };
                let (iw_i, ih_i) = (img.width as f64, img.height as f64);
                let nums: Vec<f64> =
                    cmd[2.min(cmd.len())..].iter().map(|v| v.as_f64().unwrap_or(0.0)).collect();
                let (sx, sy, sw, sh, dx, dy, dw, dh) = match nums.len() {
                    2 => (0.0, 0.0, iw_i, ih_i, nums[0], nums[1], iw_i, ih_i),
                    4 => (0.0, 0.0, iw_i, ih_i, nums[0], nums[1], nums[2], nums[3]),
                    8 => (nums[0], nums[1], nums[2], nums[3], nums[4], nums[5], nums[6], nums[7]),
                    _ => continue,
                };
                if sw <= 0.0 || sh <= 0.0 || dw == 0.0 || dh == 0.0 {
                    continue;
                }
                // Recorta al dest rect (necesario cuando el source es un
                // sub-rect: draw_image pinta la imagen ENTERA, el clip la acota).
                let dest = KurboRect::new(dx, dy, dx + dw, dy + dh);
                scene.push_layer(Mix::Clip, 1.0, base * cur, &dest);
                // Mapea espacio-pixel de la imagen → dest: el source (sx,sy)
                // cae en (dx,dy) y (sx+sw,sy+sh) en (dx+dw,dy+dh).
                let m = Affine::translate((dx, dy))
                    * Affine::scale_non_uniform(dw / sw, dh / sh)
                    * Affine::translate((-sx, -sy));
                scene.draw_image(img, base * cur * m);
                scene.pop_layer();
            }
            // clearRect parcial / putImageData: no-op (el MVP no tiene buffer
            // persistente; globalAlpha no se aplica a drawImage).
            _ => {}
        }
    }
    // Cierra cualquier clip que quedó abierto (clip sin restore al final del
    // frame) — la escena debe quedar balanceada.
    pop_clips(scene, &mut clip_stack, &mut base_clips);
}

/// Apendea un arco/elipse al path (espacio usuario), manejando la dirección
/// (clockwise por default en canvas, que con y-abajo es sweep positivo).
/// Hace `move_to`/`line_to` al punto de inicio según haya o no subpath.
fn append_arc(
    path: &mut llimphi_raster::kurbo::BezPath,
    cx: f64,
    cy: f64,
    rx: f64,
    ry: f64,
    rot: f64,
    start: f64,
    end: f64,
    ccw: bool,
) {
    use llimphi_raster::kurbo::{Arc as KArc, PathEl, Point as KPoint};
    use std::f64::consts::TAU;
    let mut sweep = end - start;
    if !ccw {
        if sweep < 0.0 {
            sweep = sweep.rem_euclid(TAU);
        }
        if sweep == 0.0 && end != start {
            sweep = TAU;
        }
    } else {
        if sweep > 0.0 {
            sweep = -((-sweep).rem_euclid(TAU));
        }
        if sweep == 0.0 && end != start {
            sweep = -TAU;
        }
    }
    // Punto de inicio del arco (con rotación de elipse).
    let (cs, sn) = (rot.cos(), rot.sin());
    let lx = rx * start.cos();
    let ly = ry * start.sin();
    let sx = cx + lx * cs - ly * sn;
    let sy = cy + lx * sn + ly * cs;
    let start_pt = KPoint::new(sx, sy);
    let empty = path.elements().is_empty();
    if empty {
        path.move_to(start_pt);
    } else {
        path.line_to(start_pt);
    }
    let arc = KArc::new((cx, cy), (rx, ry), start, sweep, rot);
    for el in arc.append_iter(0.1) {
        // append_iter continúa desde el punto actual (no emite MoveTo).
        if !matches!(el, PathEl::MoveTo(_)) {
            path.push(el);
        }
    }
}

/// Lee un campo numérico (`lw`, `ga`) del snapshot de estilo (objeto JSON).
fn style_field(st: Option<&serde_json::Value>, key: &str) -> Option<f64> {
    st.and_then(|v| v.as_object())
        .and_then(|o| o.get(key))
        .and_then(|x| x.as_f64())
}

/// Lee un campo string (`fnt`, `ta`) del snapshot de estilo.
fn style_str(st: Option<&serde_json::Value>, key: &str) -> Option<String> {
    st.and_then(|v| v.as_object())
        .and_then(|o| o.get(key))
        .and_then(|x| x.as_str())
        .map(|s| s.to_string())
}

/// Lee `fillStyle`/`strokeStyle` (`f`/`s`) del snapshot — puede ser string
/// (color) u objeto (gradiente); devuelve el `Value` para `canvas_color`.
fn style_color(st: Option<&serde_json::Value>, key: &str) -> Option<serde_json::Value> {
    st.and_then(|v| v.as_object())
        .and_then(|o| o.get(key))
        .cloned()
}
