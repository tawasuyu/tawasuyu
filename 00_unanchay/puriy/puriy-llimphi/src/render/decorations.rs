use super::*;

/// Una capa de background extra ya lista para pintar dentro del closure de
/// `paint_with` (la imagen ya envuelta en `peniko::Image`, o el gradiente
/// clonado). Preparada fuera del closure para no capturar `BoxNode`.
pub(crate) enum PreparedBgLayer {
    Image {
        img: PenikoImage,
        iw: f64,
        ih: f64,
        size: puriy_engine::style::BackgroundSize,
        position: puriy_engine::style::BackgroundPosition,
        repeat: puriy_engine::style::BackgroundRepeat,
    },
    Gradient(puriy_engine::style::LinearGradient),
}

/// Pinta las capas de background EXTRA (debajo de la capa 0) dentro de `rect`.
/// CSS pinta la primera capa de la lista arriba, así que estas van debajo y se
/// pintan en orden inverso (la última de la lista, la más al fondo, primero).
/// Cada capa es imagen (vía `paint_background_image`) o gradiente lineal.
pub(crate) fn paint_extra_bg_layers(
    scene: &mut llimphi_raster::vello::Scene,
    rect: llimphi_ui::PaintRect,
    radius: f64,
    layers: &[PreparedBgLayer],
    alpha_mul: f32,
) {
    for layer in layers.iter().rev() {
        match layer {
            PreparedBgLayer::Gradient(g) => {
                if let Some(brush) = build_linear_gradient_brush(g, rect, alpha_mul) {
                    let r = RoundedRect::new(
                        rect.x as f64,
                        rect.y as f64,
                        (rect.x + rect.w) as f64,
                        (rect.y + rect.h) as f64,
                        radius,
                    );
                    scene.fill(Fill::NonZero, Affine::IDENTITY, &brush, None, &r);
                }
            }
            PreparedBgLayer::Image { img, iw, ih, size, position, repeat } => {
                if *iw > 0.0 && *ih > 0.0 {
                    // Las capas extra usan border-box para origin y clip (los
                    // box-values del shorthand sólo viajan a la capa 0).
                    paint_background_image(
                        scene, rect, rect, radius, img, *iw, *ih, *size, *position, *repeat,
                    );
                }
            }
        }
    }
}

/// Pinta `background-image` dentro de `area` resolviendo `background-size`,
/// `background-position` y `background-repeat` (Fase 7.204). `area` es el área
/// de posicionamiento (`background-origin`, Fase 7.207): contra ella se calculan
/// `cover`/`contain`, los `%` y el origen del tiling. El pintado se recorta a
/// `clip_rect`/`clip_radius` (`background-clip`) con un clip layer. `iw`/`ih`
/// son las dimensiones naturales de la imagen (> 0, garantizado por el caller).
/// Asume transform identidad (los radios ya vienen escalados).
#[allow(clippy::too_many_arguments)]
pub(crate) fn paint_background_image(
    scene: &mut llimphi_raster::vello::Scene,
    area: llimphi_ui::PaintRect,
    clip_rect: llimphi_ui::PaintRect,
    clip_radius: f64,
    img: &PenikoImage,
    iw: f64,
    ih: f64,
    size: BackgroundSize,
    position: BackgroundPosition,
    repeat: BackgroundRepeat,
) {
    let rect = area;
    let rw = rect.w as f64;
    let rh = rect.h as f64;
    if rw <= 0.0 || rh <= 0.0 {
        return;
    }
    // 1) Tamaño del tile (tw, th) en px.
    let resolve = |lv: LengthVal, basis: f64| -> Option<f64> {
        match lv {
            LengthVal::Px(n) => Some(n as f64),
            LengthVal::Pct(p) => Some(basis * p as f64 / 100.0),
            LengthVal::Auto => None,
        }
    };
    let (tw, th) = match size {
        BackgroundSize::Auto => (iw, ih),
        BackgroundSize::Cover => {
            let s = (rw / iw).max(rh / ih);
            (iw * s, ih * s)
        }
        BackgroundSize::Contain => {
            let s = (rw / iw).min(rh / ih);
            (iw * s, ih * s)
        }
        BackgroundSize::Explicit { x, y } => match (resolve(x, rw), resolve(y, rh)) {
            (Some(w), Some(h)) => (w, h),
            (Some(w), None) => (w, w * ih / iw), // alto por aspecto
            (None, Some(h)) => (h * iw / ih, h), // ancho por aspecto
            (None, None) => (iw, ih),            // ambos auto = natural
        },
    };
    if tw <= 0.5 || th <= 0.5 {
        return;
    }
    // 2) Offset del primer tile (relativo al origen del rect). Para %, el
    //    punto p% de la imagen se alinea con el p% del box (semántica CSS).
    let pos_off = |lv: LengthVal, basis: f64, tile: f64| -> f64 {
        match lv {
            LengthVal::Px(n) => n as f64,
            LengthVal::Pct(p) => (basis - tile) * p as f64 / 100.0,
            LengthVal::Auto => 0.0,
        }
    };
    let ox = pos_off(position.x, rw, tw);
    let oy = pos_off(position.y, rh, th);
    // 3) Ejes a tilear + posiciones de inicio cubriendo [0, span].
    let (rep_x, rep_y) = match repeat {
        BackgroundRepeat::Repeat => (true, true),
        BackgroundRepeat::RepeatX => (true, false),
        BackgroundRepeat::RepeatY => (false, true),
        BackgroundRepeat::NoRepeat => (false, false),
    };
    let axis_positions = |off: f64, tile: f64, span: f64, rep: bool| -> Vec<f64> {
        if !rep {
            return vec![off];
        }
        let mut start = off;
        while start > 0.0 {
            start -= tile;
        }
        let mut v = Vec::new();
        let mut p = start;
        // Cap defensivo (tiles diminutos no deben colgar el render).
        while p < span && v.len() < 4096 {
            v.push(p);
            p += tile;
        }
        v
    };
    let xs = axis_positions(ox, tw, rw, rep_x);
    let ys = axis_positions(oy, th, rh, rep_y);
    // 4) Clip a la caja de `background-clip` (borde redondeado) y dibujo de
    //    cada tile. El tiling se ancla a `area` (origin); el recorte a clip_rect.
    let clip = RoundedRect::new(
        clip_rect.x as f64,
        clip_rect.y as f64,
        (clip_rect.x + clip_rect.w) as f64,
        (clip_rect.y + clip_rect.h) as f64,
        clip_radius,
    );
    scene.push_layer(llimphi_raster::peniko::Fill::NonZero, llimphi_raster::peniko::BlendMode::default(), 1.0, Affine::IDENTITY, &clip);
    let scale = Affine::scale_non_uniform(tw / iw, th / ih);
    for &x in &xs {
        for &y in &ys {
            let tf = Affine::translate((rect.x as f64 + x, rect.y as f64 + y)) * scale;
            scene.draw_image(img, tf);
        }
    }
    scene.pop_layer();
}

/// Aplica `border-radius` y dibuja, en una sola pasada de `paint_with`,
/// la sombra (si la hay) y el contorno del border (si lo hay). Vello
/// pinta el callback entre el `fill` y la `image`/`text` del view, así
/// que la sombra cae detrás del contenido pero encima del fondo del
/// parent. Aproximación: sin gaussian blur — el `blur_px` se mapea
/// Patrón de dashes (en px) para un `border-style` dado y grosor `w`.
/// `Solid`/`Double` → `None` (se pintan distinto). `Dotted` = cuadros del
/// tamaño del grosor; `Dashed` = trazos de 3× separados por 2×.
fn border_dash_pattern(style: BorderLineStyle, w: f64) -> Option<[f64; 2]> {
    match style {
        BorderLineStyle::Dotted => Some([w.max(0.5), w.max(0.5) * 1.5]),
        BorderLineStyle::Dashed => Some([w.max(0.5) * 3.0, w.max(0.5) * 2.0]),
        BorderLineStyle::Solid
        | BorderLineStyle::Double
        | BorderLineStyle::Groove
        | BorderLineStyle::Ridge
        | BorderLineStyle::Inset
        | BorderLineStyle::Outset => None,
    }
}

/// Cuáles `border-style` requieren render per-lado por usar dos
/// tonalidades del color base (top+left vs bottom+right). Cuando es
/// `true`, el camino uniforme no aplica.
fn border_style_is_3d(style: BorderLineStyle) -> bool {
    matches!(
        style,
        BorderLineStyle::Groove
            | BorderLineStyle::Ridge
            | BorderLineStyle::Inset
            | BorderLineStyle::Outset
    )
}

/// Mezcla el color base con blanco/negro para producir las dos
/// tonalidades del 3D. `factor` ∈ [-1, 1]: -1 ⇒ negro puro, 0 ⇒ base,
/// +1 ⇒ blanco puro. Mantiene alfa.
fn shade(c: puriy_engine::Color, factor: f32) -> puriy_engine::Color {
    let mix = |ch: u8| -> u8 {
        let v = ch as f32;
        let t = if factor >= 0.0 {
            v + (255.0 - v) * factor
        } else {
            v + v * factor
        };
        t.clamp(0.0, 255.0) as u8
    };
    puriy_engine::Color { r: mix(c.r), g: mix(c.g), b: mix(c.b), a: c.a }
}

/// Color a usar en cada par de lados (top/left, bottom/right) según
/// la variante 3D. Sigue la convención de browsers — `groove`/`inset`
/// hunden (top+left oscuro), `ridge`/`outset` elevan (top+left claro).
fn border_3d_colors(
    style: BorderLineStyle,
    base: puriy_engine::Color,
) -> (puriy_engine::Color, puriy_engine::Color) {
    let dark = shade(base, -0.4);
    let light = shade(base, 0.4);
    match style {
        BorderLineStyle::Groove | BorderLineStyle::Inset => (dark, light),
        BorderLineStyle::Ridge | BorderLineStyle::Outset => (light, dark),
        _ => (base, base),
    }
}

/// Datos de `text-decoration` capturados para el closure de paint (evita
/// referenciar el `BoxNode` dentro de él). Todo ya escalado por zoom.
#[derive(Clone, Copy)]
struct DecoSpec {
    line: TextDecorationLine,
    color: puriy_engine::Color,
    font_size: f32,
    style: TextDecorationStyle,
    /// `text-decoration-thickness` en px (ya × zoom). `None` = auto.
    thickness: Option<f64>,
    /// `text-underline-offset` en px (ya × zoom). `None` = auto.
    underline_offset: Option<f64>,
}

/// como expansión adicional del rect con alpha proporcional, lo cual
/// da una sombra "dura" pero proporcionada.
pub(crate) fn apply_decorations(mut view: View<Msg>, b: &BoxNode, zoom: f32) -> View<Msg> {
    let z = zoom;
    // Radio del clip del view: usamos el máximo de las 4 esquinas (Llimphi
    // `View::radius` toma un escalar). Cuando las 4 esquinas son iguales
    // el resultado es exacto; cuando difieren, el clip queda con la
    // esquina más redonda — el border per-side dibujado abajo seguirá
    // marcando las corners individuales.
    let radii = b.border_radii;
    let radius_max =
        radii.top_left.max(radii.top_right).max(radii.bottom_right).max(radii.bottom_left);
    if radius_max > 0.0 {
        view = view.radius((radius_max * z) as f64);
    }
    let radius = (radius_max * z) as f64;
    let shadows: Vec<BoxShadow> = b
        .box_shadows
        .iter()
        .map(|s| BoxShadow {
            offset_x: s.offset_x * z,
            offset_y: s.offset_y * z,
            blur_px: s.blur_px * z,
            spread_px: s.spread_px * z,
            color: s.color,
            inset: s.inset,
        })
        .collect();
    let alpha_mul = b.opacity.clamp(0.0, 1.0);
    // Border uniforme = los 4 lados con mismo width y color. Lo
    // dibujamos como RoundedRect stroke para que las corners radius
    // queden suaves. Si los lados difieren, pintamos cada uno como
    // segmento independiente (Border::Sides) — las corners en ese caso
    // van en chaflán cuadrado, que matchea el look estándar de browsers
    // cuando se mezclan widths/colors por lado.
    let bw = b.border_widths;
    let bc = b.border_colors;
    let border_style = b.border_style;
    // Los estilos 3D (`groove`/`ridge`/`inset`/`outset`) NUNCA usan el
    // camino uniforme porque cada par de lados pinta una tonalidad
    // distinta — fuerza el camino per-lado.
    let force_per_side = border_style_is_3d(border_style);
    let uniform_border = if !force_per_side
        && bw.top == bw.right
        && bw.right == bw.bottom
        && bw.bottom == bw.left
        && bc.top == bc.right
        && bc.right == bc.bottom
        && bc.bottom == bc.left
        && bw.top > 0.0
    {
        bc.top.map(|c| (c, bw.top * z))
    } else {
        None
    };
    let per_side_border = if uniform_border.is_none() {
        let s_top = bc.top.filter(|_| bw.top > 0.0).map(|c| (c, bw.top * z));
        let s_right = bc.right.filter(|_| bw.right > 0.0).map(|c| (c, bw.right * z));
        let s_bottom = bc.bottom.filter(|_| bw.bottom > 0.0).map(|c| (c, bw.bottom * z));
        let s_left = bc.left.filter(|_| bw.left > 0.0).map(|c| (c, bw.left * z));
        if s_top.is_some() || s_right.is_some() || s_bottom.is_some() || s_left.is_some() {
            Some((s_top, s_right, s_bottom, s_left))
        } else {
            None
        }
    } else {
        None
    };
    // outline se pinta fuera del border + offset, sin afectar layout. Si
    // `style_active` es false (none/hidden) o falta color, no pinta.
    let outline = if b.outline.style_active
        && b.outline.width > 0.0
        && b.outline.color.is_some()
    {
        Some((
            b.outline.color.unwrap(),
            b.outline.width * z,
            b.outline.offset * z,
            b.outline.style,
        ))
    } else {
        None
    };
    // text-decoration sólo tiene efecto visual sobre hojas de texto. En
    // un nodo container, la línea ya la pinta cada hoja descendiente.
    let deco = if b.text.is_some() && b.text_decoration != TextDecorationLine::None {
        // `text-decoration-color` default = currentColor (sigue al texto).
        let dc = b.text_decoration_color.unwrap_or(b.color);
        Some(DecoSpec {
            line: b.text_decoration,
            color: dc,
            font_size: b.font_size * z,
            style: b.text_decoration_style,
            thickness: b.text_decoration_thickness.map(|t| (t * z) as f64),
            underline_offset: b.text_underline_offset.map(|o| (o * z) as f64),
        })
    } else {
        None
    };
    let gradient = b.background_gradient.clone();
    // `background-image: url(...)`: si el engine pudo descargarla, la
    // envolvemos en peniko::Image para que el closure de paint_with la
    // escale/posicione/tile dentro del rect según `background-size`,
    // `background-position` y `background-repeat` (Fase 7.204).
    let bg_image = b.background_image.as_ref().map(|img| {
        let blob = Blob::from(img.rgba.clone());
        let peniko = PenikoImage::new(ImageData { data: blob, format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: img.width, height: img.height });
        (peniko, img.width as f64, img.height as f64)
    });
    let bg_size = b.background_size;
    let bg_position = b.background_position;
    let bg_repeat = b.background_repeat;
    // `background-origin` / `background-clip` (Fase 7.207). Insets (escalados)
    // del border-box hacia adentro: padding-box descuenta el border; content-box
    // border + padding. Aplican a imagen y gradiente de la capa 0; el color
    // sólido sigue al border-box (lo pinta el `View::fill`, fuera del closure).
    let pad = b.padding;
    let pb_ins = (bw.left * z, bw.top * z, bw.right * z, bw.bottom * z);
    let cb_ins = (
        (bw.left + pad.left) * z,
        (bw.top + pad.top) * z,
        (bw.right + pad.right) * z,
        (bw.bottom + pad.bottom) * z,
    );
    let origin_ins = match b.background_origin {
        puriy_engine::style::BackgroundOrigin::BorderBox => (0.0, 0.0, 0.0, 0.0),
        puriy_engine::style::BackgroundOrigin::PaddingBox => pb_ins,
        puriy_engine::style::BackgroundOrigin::ContentBox => cb_ins,
    };
    let clip_ins = match b.background_clip {
        puriy_engine::style::BackgroundClip::PaddingBox => pb_ins,
        puriy_engine::style::BackgroundClip::ContentBox => cb_ins,
        // BorderBox y Text no insetean el rect (Text ni siquiera pinta el
        // fondo como rect — lo rellena en las glifos de la hoja de texto).
        _ => (0.0, 0.0, 0.0, 0.0),
    };
    // `background-clip: text` (Fase 7.208): el elemento estilado NO pinta su
    // gradiente/imagen como rect — el relleno va a las glifos de su texto hijo
    // (propagado en build a la hoja). Acá lo suprimimos en el rect.
    let clip_is_text =
        matches!(b.background_clip, puriy_engine::style::BackgroundClip::Text);
    // Capas de background EXTRA (debajo de la capa 0). Cada una es imagen
    // (raster ya decodificado) o gradiente. Se preparan acá para no capturar
    // el BoxNode dentro del closure.
    let extra_layers: Vec<PreparedBgLayer> = b
        .background_extra_layers
        .iter()
        .filter_map(|l| {
            if let Some(img) = &l.image {
                let blob = Blob::from(img.rgba.clone());
                let peniko = PenikoImage::new(ImageData { data: blob, format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: img.width, height: img.height });
                Some(PreparedBgLayer::Image {
                    img: peniko,
                    iw: img.width as f64,
                    ih: img.height as f64,
                    size: l.size,
                    position: l.position,
                    repeat: l.repeat,
                })
            } else {
                l.gradient.clone().map(PreparedBgLayer::Gradient)
            }
        })
        .collect();
    if shadows.is_empty()
        && uniform_border.is_none()
        && per_side_border.is_none()
        && deco.is_none()
        && outline.is_none()
        && gradient.is_none()
        && bg_image.is_none()
        && extra_layers.is_empty()
    {
        return view;
    }
    view.paint_with(move |scene, _typesetter, rect| {
        // Capas de background EXTRA, debajo de la capa 0.
        paint_extra_bg_layers(scene, rect, radius, &extra_layers, alpha_mul);
        // Cajas de `background-origin` (área de posicionamiento) y
        // `background-clip` (recorte) de la capa 0. Insetean el border-box.
        let inset = |ins: (f32, f32, f32, f32)| llimphi_ui::PaintRect {
            x: rect.x + ins.0,
            y: rect.y + ins.1,
            w: (rect.w - ins.0 - ins.2).max(0.0),
            h: (rect.h - ins.1 - ins.3).max(0.0),
        };
        let origin_rect = inset(origin_ins);
        let clip_rect = inset(clip_ins);
        // El radio interno se encoge con el inset (esquina top-left como repr.).
        let clip_radius = (radius - clip_ins.0.max(clip_ins.1) as f64).max(0.0);
        // linear-gradient: se dimensiona contra el origin box y se recorta al
        // clip box. peniko interpreta `Linear { start, end }` como las dos
        // puntas — `build_linear_gradient_brush` cruza el rect dado.
        if let Some(g) = &gradient {
            if !clip_is_text {
                if let Some(brush) = build_linear_gradient_brush(g, origin_rect, alpha_mul) {
                    let r = RoundedRect::new(
                        clip_rect.x as f64,
                        clip_rect.y as f64,
                        (clip_rect.x + clip_rect.w) as f64,
                        (clip_rect.y + clip_rect.h) as f64,
                        clip_radius,
                    );
                    scene.fill(Fill::NonZero, Affine::IDENTITY, &brush, None, &r);
                }
            }
        }
        // background-image: resuelve tamaño/posición/repeat (Fase 7.204) contra
        // el origin box y tilea, recortado al clip box (Fase 7.207).
        if let Some((img, iw, ih)) = &bg_image {
            if *iw > 0.0 && *ih > 0.0 && !clip_is_text {
                paint_background_image(
                    scene, origin_rect, clip_rect, clip_radius, img, *iw, *ih, bg_size,
                    bg_position, bg_repeat,
                );
            }
        }
        // Sombras: CSS pinta back-to-front (la PRIMERA listada queda
        // ENCIMA). Iteramos en reversa para que el orden visual
        // coincida. Las `outset` se pintan detrás del fondo; las
        // `inset` se pintan ENCIMA del fondo (recortadas al box) más
        // arriba — acá sólo despachamos las outset; las inset las
        // tomamos en otra pasada.
        for BoxShadow { offset_x, offset_y, blur_px, spread_px, color, inset } in
            shadows.iter().rev()
        {
            if *inset {
                continue;
            }
            let extra = (blur_px + spread_px) as f64;
            let half_alpha = if *blur_px > 0.0 { 0.55 } else { 0.85 };
            let sc = Color::from_rgba8(
                color.r,
                color.g,
                color.b,
                (color.a as f64 * half_alpha) as u8,
            );
            let r = RoundedRect::new(
                (rect.x + offset_x) as f64 - extra,
                (rect.y + offset_y) as f64 - extra,
                (rect.x + rect.w + offset_x) as f64 + extra,
                (rect.y + rect.h + offset_y) as f64 + extra,
                (radius + extra).max(0.0),
            );
            scene.fill(Fill::NonZero, Affine::IDENTITY, sc, None, &r);
        }
        // Sombras `inset`: aproximación. Cada sombra inset se pinta
        // como un stroke por dentro del box rect, con grosor = blur +
        // spread + max(|offset|). El offset desplaza el rect del
        // stroke en su misma dirección, dando el sesgo lateral del
        // espec (lado contrario al offset queda más oscuro). Blur
        // real (gaussiano) sigue pendiente — alfa rebajada cuando hay
        // blur, igual que outset.
        for BoxShadow { offset_x, offset_y, blur_px, spread_px, color, inset } in
            shadows.iter().rev()
        {
            if !*inset {
                continue;
            }
            let off_max = offset_x.abs().max(offset_y.abs());
            let sw = ((*blur_px + *spread_px + off_max) as f64).max(1.0);
            let half_alpha = if *blur_px > 0.0 { 0.55 } else { 0.85 };
            let sc = Color::from_rgba8(
                color.r,
                color.g,
                color.b,
                (color.a as f64 * half_alpha) as u8,
            );
            let half = sw * 0.5;
            let r = RoundedRect::new(
                rect.x as f64 + half + *offset_x as f64,
                rect.y as f64 + half + *offset_y as f64,
                (rect.x + rect.w) as f64 - half + *offset_x as f64,
                (rect.y + rect.h) as f64 - half + *offset_y as f64,
                (radius - half).max(0.0),
            );
            scene.stroke(&Stroke::new(sw), Affine::IDENTITY, sc, None, &r);
        }
        if let Some((bc, w)) = uniform_border {
            let a = (bc.a as f32 * alpha_mul) as u8;
            let color = Color::from_rgba8(bc.r, bc.g, bc.b, a);
            let mk_rect = |inset: f64, sw: f64| {
                let half = sw * 0.5 + inset;
                RoundedRect::new(
                    rect.x as f64 + half,
                    rect.y as f64 + half,
                    (rect.x + rect.w) as f64 - half,
                    (rect.y + rect.h) as f64 - half,
                    (radius - half).max(0.0),
                )
            };
            if let BorderLineStyle::Double = border_style {
                // Dos líneas de ~1/3 del grosor, separadas por otro 1/3.
                let sw = (w as f64 / 3.0).max(1.0);
                scene.stroke(&Stroke::new(sw), Affine::IDENTITY, color, None, &mk_rect(0.0, sw));
                scene.stroke(
                    &Stroke::new(sw), Affine::IDENTITY, color, None,
                    &mk_rect(w as f64 - sw, sw),
                );
            } else {
                let mut stroke = Stroke::new(w as f64);
                if let Some(p) = border_dash_pattern(border_style, w as f64) {
                    stroke = stroke.with_dashes(0.0, p);
                }
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &mk_rect(0.0, w as f64));
            }
        }
        if let Some((s_top, s_right, s_bottom, s_left)) = per_side_border {
            // Per-side: pintamos cada lado como una línea recta del color
            // y grosor correspondientes. Corners en chaflán cuadrado —
            // matchea el look de browsers cuando border-{top,right,...}
            // difieren entre sí.
            let x0 = rect.x as f64;
            let y0 = rect.y as f64;
            let x1 = x0 + rect.w as f64;
            let y1 = y0 + rect.h as f64;
            // Stroke por lado con el patrón de `border-style`. `double` cae
            // a sólido en el modo per-lado (combo raro, no vale el coste).
            let side_stroke = |w: f64| {
                let mut s = Stroke::new(w);
                if let Some(p) = border_dash_pattern(border_style, w) {
                    s = s.with_dashes(0.0, p);
                }
                s
            };
            // Para `groove`/`ridge`/`inset`/`outset` cada par de lados usa
            // una tonalidad distinta del color base (top+left vs
            // bottom+right). Para el resto la tonalidad es la base tal cual.
            let tone_tl = |c: puriy_engine::Color| -> puriy_engine::Color {
                if border_style_is_3d(border_style) {
                    border_3d_colors(border_style, c).0
                } else {
                    c
                }
            };
            let tone_br = |c: puriy_engine::Color| -> puriy_engine::Color {
                if border_style_is_3d(border_style) {
                    border_3d_colors(border_style, c).1
                } else {
                    c
                }
            };
            // Cada lado se inseta por w/2 para que el trazo caiga dentro
            // del rect del nodo (vello pinta centrado al path). Pintamos
            // inline (sin closure) para evitar capturas raras del scene.
            if let Some((c, w)) = s_top {
                let c = tone_tl(c);
                let h = (w as f64) * 0.5;
                let a = (c.a as f32 * alpha_mul) as u8;
                let color = Color::from_rgba8(c.r, c.g, c.b, a);
                let line = Line::new((x0, y0 + h), (x1, y0 + h));
                scene.stroke(&side_stroke(w as f64), Affine::IDENTITY, color, None, &line);
            }
            if let Some((c, w)) = s_bottom {
                let c = tone_br(c);
                let h = (w as f64) * 0.5;
                let a = (c.a as f32 * alpha_mul) as u8;
                let color = Color::from_rgba8(c.r, c.g, c.b, a);
                let line = Line::new((x0, y1 - h), (x1, y1 - h));
                scene.stroke(&side_stroke(w as f64), Affine::IDENTITY, color, None, &line);
            }
            if let Some((c, w)) = s_left {
                let c = tone_tl(c);
                let h = (w as f64) * 0.5;
                let a = (c.a as f32 * alpha_mul) as u8;
                let color = Color::from_rgba8(c.r, c.g, c.b, a);
                let line = Line::new((x0 + h, y0), (x0 + h, y1));
                scene.stroke(&side_stroke(w as f64), Affine::IDENTITY, color, None, &line);
            }
            if let Some((c, w)) = s_right {
                let c = tone_br(c);
                let h = (w as f64) * 0.5;
                let a = (c.a as f32 * alpha_mul) as u8;
                let color = Color::from_rgba8(c.r, c.g, c.b, a);
                let line = Line::new((x1 - h, y0), (x1 - h, y1));
                scene.stroke(&side_stroke(w as f64), Affine::IDENTITY, color, None, &line);
            }
        }
        if let Some((oc, ow, off, ostyle)) = outline {
            let w = ow as f64;
            let half = w * 0.5;
            // outline se dibuja FUERA del border, separado por `offset`.
            let outset = (off as f64) + half;
            let mk_r = |extra: f64| {
                RoundedRect::new(
                    rect.x as f64 - off as f64 - extra,
                    rect.y as f64 - off as f64 - extra,
                    (rect.x + rect.w) as f64 + off as f64 + extra,
                    (rect.y + rect.h) as f64 + off as f64 + extra,
                    radius + off as f64 + extra,
                )
            };
            let a = (oc.a as f32 * alpha_mul) as u8;
            let color = Color::from_rgba8(oc.r, oc.g, oc.b, a);
            if let BorderLineStyle::Double = ostyle {
                let sw = (w / 3.0).max(1.0);
                scene.stroke(&Stroke::new(sw), Affine::IDENTITY, color, None, &mk_r(sw * 0.5));
                scene.stroke(&Stroke::new(sw), Affine::IDENTITY, color, None, &mk_r(w - sw * 0.5));
            } else {
                let mut stroke = Stroke::new(w);
                if let Some(p) = border_dash_pattern(ostyle, w) {
                    stroke = stroke.with_dashes(0.0, p);
                }
                scene.stroke(&stroke, Affine::IDENTITY, color, None, &mk_r(half));
            }
        }
        if let Some(d) = deco {
            let line_kind = d.line;
            let deco_style = d.style;
            // Posición vertical relativa al rect (sin baseline real). El
            // rect del leaf de texto tiene height = font_size * line_height
            // (≈1.4 default), así que el texto vive arriba-centro:
            //   overline    → top + line_height*0.10
            //   line-through → mid (≈ 0.55)
            //   underline   → ~ baseline (≈ 0.85)
            let y_frac = match line_kind {
                TextDecorationLine::Overline => 0.10,
                TextDecorationLine::LineThrough => 0.55,
                TextDecorationLine::Underline => 0.88,
                TextDecorationLine::None => return,
            };
            // `text-underline-offset` empuja la underline hacia abajo (lejos
            // del texto); sólo aplica a underline.
            let off_y = if matches!(line_kind, TextDecorationLine::Underline) {
                d.underline_offset.unwrap_or(0.0)
            } else {
                0.0
            };
            let y = rect.y as f64 + rect.h as f64 * y_frac + off_y;
            // `text-decoration-thickness` explícito gana; auto ≈ 7% del font.
            let thickness = d.thickness.unwrap_or(((d.font_size * 0.07) as f64).max(1.0)).max(0.5);
            let c = d.color;
            let dec_color = Color::from_rgba8(c.r, c.g, c.b, (c.a as f32 * alpha_mul) as u8);
            let x0 = rect.x as f64;
            let x1 = (rect.x + rect.w) as f64;
            // `text-decoration-style`: solid/double/dotted/dashed/wavy.
            // dotted/dashed → patrón de stroke; double → dos líneas; wavy
            // → zig-zag triangular aproximado (peniko no tiene línea ondulada).
            match deco_style {
                TextDecorationStyle::Wavy => {
                    let amp = thickness.max(1.0); // amplitud del zig-zag
                    let period = (amp * 4.0).max(3.0);
                    let mut path = KurboBezPath::new();
                    path.move_to((x0, y));
                    let mut x = x0;
                    let mut up = true;
                    while x < x1 {
                        let nx = (x + period).min(x1);
                        let ny = if up { y - amp } else { y + amp };
                        path.line_to((nx, ny));
                        x = nx;
                        up = !up;
                    }
                    scene.stroke(&Stroke::new(thickness), Affine::IDENTITY, dec_color, None, &path);
                }
                TextDecorationStyle::Double => {
                    let off = (thickness * 1.6).max(2.0);
                    for dy in [-off * 0.5, off * 0.5] {
                        let line = Line::new((x0, y + dy), (x1, y + dy));
                        scene.stroke(&Stroke::new(thickness), Affine::IDENTITY, dec_color, None, &line);
                    }
                }
                other => {
                    let mut stroke = Stroke::new(thickness);
                    if let TextDecorationStyle::Dotted = other {
                        stroke = stroke.with_dashes(0.0, [thickness, thickness * 1.5]);
                    } else if let TextDecorationStyle::Dashed = other {
                        stroke = stroke.with_dashes(0.0, [thickness * 3.0, thickness * 2.0]);
                    }
                    let line = Line::new((x0, y), (x1, y));
                    scene.stroke(&stroke, Affine::IDENTITY, dec_color, None, &line);
                }
            }
        }
    })
}
