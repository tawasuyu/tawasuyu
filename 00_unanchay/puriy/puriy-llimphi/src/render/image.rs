use super::*;

/// View dimensionada para una imagen — ancho hasta `width_px` pero
/// nunca más que el contenedor (`max_width: 100%`), altura proporcional
/// vía aspect ratio inverso (`width / height`).
pub(crate) fn image_view(width: u32, height: u32, zoom: f32) -> View<Msg> {
    let w = (width.max(1)) as f32 * zoom;
    let h = (height.max(1)) as f32 * zoom;
    let ratio = if height > 0 { Some(width as f32 / height as f32) } else { None };
    View::new(Style {
        size: Size { width: length(w), height: length(h) },
        // `max-width: 100%` clampa el ancho al contenedor (responsive
        // por default — sin esto, imágenes grandes rompen layouts narrow);
        // `aspect_ratio` deja que taffy preserve la proporción cuando el
        // ancho real (post-clamp) sea menor que `length(w)`.
        max_size: Size {
            width: percent(1.0_f32),
            height: auto(),
        },
        aspect_ratio: ratio,
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(4.0_f32 * zoom),
            bottom: length(4.0_f32 * zoom),
        },
        ..Default::default()
    })
}

/// Escala por eje `(sx, sy)` que aplica `object-fit` a una imagen de tamaño
/// natural `iw×ih` dentro de una caja `rw×rh` (en px de pantalla). `z` = zoom
/// (factor que vuelve el tamaño natural a px de pantalla, para `None`/
/// `ScaleDown`). `Fill` estira por eje; `Contain`/`Cover` preservan aspecto
/// (cabe / cubre); `None` deja el natural; `ScaleDown` = menor entre contain
/// y natural. Fase 7.230.
pub(crate) fn object_fit_scale(
    fit: puriy_engine::ObjectFit,
    rw: f64,
    rh: f64,
    iw: f64,
    ih: f64,
    z: f64,
) -> (f64, f64) {
    use puriy_engine::ObjectFit;
    if iw <= 0.0 || ih <= 0.0 {
        return (1.0, 1.0);
    }
    match fit {
        ObjectFit::Fill => (rw / iw, rh / ih),
        ObjectFit::Contain => {
            let s = (rw / iw).min(rh / ih);
            (s, s)
        }
        ObjectFit::Cover => {
            let s = (rw / iw).max(rh / ih);
            (s, s)
        }
        ObjectFit::None => (z, z),
        ObjectFit::ScaleDown => {
            let s = (rw / iw).min(rh / ih).min(z);
            (s, s)
        }
    }
}

/// Mapea `image-rendering` CSS a la calidad de muestreo de peniko (Fase 7.1239).
/// `auto` → `None` (conserva el default `Medium`). `pixelated` y `crisp-edges`
/// → `Low` (nearest-neighbour, sin AA al escalar — pixel art nítido). `smooth`
/// → `High` (bilineal/trilineal, máximo suavizado).
pub(crate) fn image_quality_for(
    r: puriy_engine::ImageRendering,
) -> Option<llimphi_raster::peniko::ImageQuality> {
    use llimphi_raster::peniko::ImageQuality;
    use puriy_engine::ImageRendering as IR;
    match r {
        IR::Auto => None,
        IR::Smooth => Some(ImageQuality::High),
        IR::CrispEdges | IR::Pixelated => Some(ImageQuality::Low),
    }
}

/// Aplica el `image-rendering` del box a la `ImageBrush` (Fase 7.1239). Si el
/// modo no es `auto`, fija la calidad de muestreo; vello la respeta al escalar.
pub(crate) fn with_image_rendering(
    peniko: PenikoImage,
    r: puriy_engine::ImageRendering,
) -> PenikoImage {
    match image_quality_for(r) {
        Some(q) => peniko.with_quality(q),
        None => peniko,
    }
}

/// View de un `<img>` con `object-fit` explícito (Fase 7.230). A diferencia de
/// [`image_view`] (que delega el encaje al compositor — siempre contain), aquí
/// dibujamos la imagen a mano (`paint_with`) con el escalado pedido y la
/// recortamos a la caja. El tamaño de la caja sale de las dimensiones CSS
/// (`width`/`height`) si están, si no del intrínseco. `object-position` queda
/// fijo en el centro (50% 50%).
pub(crate) fn image_fit_view(
    b: &BoxNode,
    peniko: PenikoImage,
    fit: puriy_engine::ObjectFit,
    zoom: f32,
) -> View<Msg> {
    let iw = peniko.image.width.max(1) as f64;
    let ih = peniko.image.height.max(1) as f64;
    let w_dim = length_to_taffy(b.width, zoom).unwrap_or_else(|| length(iw as f32 * zoom));
    let h_dim = length_to_taffy(b.height, zoom).unwrap_or_else(|| length(ih as f32 * zoom));
    let z = zoom as f64;
    // `object-position` (default centro). `Px` = offset desde el borde;
    // `Pct` = alinea ese % de la imagen con ese % de la caja.
    let obj_pos = b.object_position.unwrap_or(puriy_engine::style::BackgroundPosition {
        x: puriy_engine::style::LengthVal::Pct(50.0),
        y: puriy_engine::style::LengthVal::Pct(50.0),
    });
    View::new(Style {
        size: Size { width: w_dim, height: h_dim },
        // Igual que image_view: clamp responsivo al contenedor.
        max_size: Size { width: percent(1.0_f32), height: auto() },
        margin: Rect {
            left: length(0.0_f32),
            right: length(0.0_f32),
            top: length(4.0_f32 * zoom),
            bottom: length(4.0_f32 * zoom),
        },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        let rw = rect.w as f64;
        let rh = rect.h as f64;
        if rw <= 0.0 || rh <= 0.0 {
            return;
        }
        let (sx, sy) = object_fit_scale(fit, rw, rh, iw, ih, z);
        let dw = iw * sx;
        let dh = ih * sy;
        // Coloca la imagen escalada dentro de la caja según object-position.
        let off = |lv: puriy_engine::style::LengthVal, free: f64| -> f64 {
            match lv {
                puriy_engine::style::LengthVal::Px(n) => n as f64 * z,
                puriy_engine::style::LengthVal::Pct(p) => free * p as f64 / 100.0,
                // Auto y keywords intrínsecos (inválidos en object-position) → centro.
                _ => free * 0.5,
            }
        };
        let tx = rect.x as f64 + off(obj_pos.x, rw - dw);
        let ty = rect.y as f64 + off(obj_pos.y, rh - dh);
        let clip = RoundedRect::new(
            rect.x as f64,
            rect.y as f64,
            (rect.x + rect.w) as f64,
            (rect.y + rect.h) as f64,
            0.0,
        );
        scene.push_layer(llimphi_raster::peniko::Fill::NonZero, llimphi_raster::peniko::BlendMode::default(), 1.0, Affine::IDENTITY, &clip);
        scene.draw_image(
            &peniko,
            Affine::translate((tx, ty)) * Affine::scale_non_uniform(sx, sy),
        );
        scene.pop_layer();
    })
}

pub(crate) fn render_link_subtree(
    b: &BoxNode,
    target: &str,
    color: Color,
    new_tab: bool,
    ctx: &mut RenderCtx<'_>,
) -> View<Msg> {
    let zoom = ctx.zoom;
    // <details> dentro de un <a> es HTML inválido en la práctica; pero
    // si aparece, contamos el slot igualmente para no desalinear el
    // counter global. No reescribimos el comportamiento interactivo:
    // dentro de un link el subtree colapsado se ignora.
    if b.tag.as_deref() == Some("details") {
        skip_count_details(b, &mut ctx.details_counter);
    }
    let nav_msg = |t: &str| {
        if new_tab {
            Msg::NavigateNewTab(t.to_string())
        } else {
            Msg::Navigate(t.to_string())
        }
    };
    let mut view = View::new(box_style(b, zoom))
        .on_click(nav_msg(target))
        .on_middle_click(Msg::NavigateNewTab(target.to_string()));
    let find_hit = b
        .text
        .as_ref()
        .map(|s| ctx.matcher.matches(s))
        .unwrap_or(false);
    let find_hit_color: Option<Color> = if find_hit {
        ctx.find_counter += 1;
        let is_current = ctx.find_current != 0 && ctx.find_counter == ctx.find_current;
        Some(if is_current {
            Color::from_rgba8(255, 140, 0, 240)
        } else {
            Color::from_rgba8(255, 230, 0, 200)
        })
    } else {
        None
    };
    if let Some(c) = find_hit_color {
        view = view.fill(c);
    } else if let Some(bg) = b.background {
        view = view.fill(Color::from_rgb8(bg.r, bg.g, bg.b));
    }
    if let Some(img) = &b.image {
        let blob = Blob::from(img.rgba.clone());
        let peniko = with_image_rendering(
            PenikoImage::new(ImageData { data: blob, format: ImageFormat::Rgba8, alpha_type: ImageAlphaType::Alpha, width: img.width, height: img.height }),
            b.image_rendering,
        );
        return match b.object_fit {
            Some(fit) => image_fit_view(b, peniko, fit, zoom).on_click(nav_msg(target)),
            None => image_view(img.width, img.height, zoom)
                .image(peniko)
                .on_click(nav_msg(target)),
        };
    }
    if let Some(text) = &b.text {
        let base = if b.font_weight >= 600 { b.font_size * 1.1 } else { b.font_size };
        let size = base * zoom;
        let italic = matches!(b.font_style, puriy_engine::FontStyle::Italic);
        return view
            .text_aligned_full(
                text.clone(),
                size,
                color,
                Alignment::Start,
                italic,
                b.font_family.clone(),
            )
            .line_height(b.line_height.unwrap_or(1.2));
    }
    if !b.children.is_empty() {
        let target_owned = target.to_string();
        view = view.children(
            b.children
                .iter()
                .map(|c| render_link_subtree(c, &target_owned, color, new_tab, ctx))
                .collect(),
        );
    }
    view
}

/// Pinta las primitivas de un `<svg>` dentro de un rect del tamaño
/// `scene.width × scene.height` (escalado por zoom). Si `view_box` está
/// definido, las primitivas se mapean a [0..1] vía viewBox y luego se
/// escalan al rect del nodo (preservando aspect ratio, "meet").
pub(crate) fn render_svg(scene: &puriy_engine::SvgScene, zoom: f32) -> View<Msg> {
    use llimphi_raster::kurbo::{Circle as KurboCircle, Line as KurboLine};
    let w = scene.width * zoom;
    let h = scene.height * zoom;
    let prims = scene.prims.clone();
    let view_box = scene.view_box;
    let svg_w = scene.width;
    let svg_h = scene.height;
    View::new(Style {
        size: Size { width: length(w), height: length(h) },
        ..Default::default()
    })
    .paint_with(move |scene, _ts, rect| {
        // Mapping local → pantalla. Si hay viewBox, normalizamos por él
        // y escalamos al rect; sino usamos directamente width/height del
        // svg como dominio.
        let (src_x, src_y, src_w, src_h) = view_box.unwrap_or((0.0, 0.0, svg_w, svg_h));
        let sx = if src_w > 0.0 { rect.w as f64 / src_w as f64 } else { 1.0 };
        let sy = if src_h > 0.0 { rect.h as f64 / src_h as f64 } else { 1.0 };
        let s = sx.min(sy).max(0.001);
        let to_x = |x: f32| rect.x as f64 + ((x - src_x) as f64) * s;
        let to_y = |y: f32| rect.y as f64 + ((y - src_y) as f64) * s;
        let to_color = |c: puriy_engine::Color| {
            Color::from_rgba8(c.r, c.g, c.b, c.a)
        };
        for p in &prims {
            match *p {
                puriy_engine::SvgPrim::Rect {
                    x, y, w, h, rx, fill, stroke, stroke_w,
                } => {
                    let r = RoundedRect::new(
                        to_x(x),
                        to_y(y),
                        to_x(x + w),
                        to_y(y + h),
                        (rx as f64) * s,
                    );
                    if let Some(f) = fill {
                        scene.fill(Fill::NonZero, Affine::IDENTITY, to_color(f), None, &r);
                    }
                    if let Some(st) = stroke {
                        let stroke = Stroke::new(stroke_w as f64 * s);
                        scene.stroke(&stroke, Affine::IDENTITY, to_color(st), None, &r);
                    }
                }
                puriy_engine::SvgPrim::Circle { cx, cy, r, fill, stroke, stroke_w } => {
                    let c = KurboCircle::new((to_x(cx), to_y(cy)), r as f64 * s);
                    if let Some(f) = fill {
                        scene.fill(Fill::NonZero, Affine::IDENTITY, to_color(f), None, &c);
                    }
                    if let Some(st) = stroke {
                        let stroke = Stroke::new(stroke_w as f64 * s);
                        scene.stroke(&stroke, Affine::IDENTITY, to_color(st), None, &c);
                    }
                }
                puriy_engine::SvgPrim::Line { x1, y1, x2, y2, stroke, stroke_w } => {
                    let l = KurboLine::new((to_x(x1), to_y(y1)), (to_x(x2), to_y(y2)));
                    let stroke_obj = Stroke::new(stroke_w as f64 * s);
                    scene.stroke(&stroke_obj, Affine::IDENTITY, to_color(stroke), None, &l);
                }
                puriy_engine::SvgPrim::Polyline {
                    ref points, closed, fill, stroke, stroke_w,
                } => {
                    use llimphi_raster::kurbo::{BezPath, PathEl, Point as KurboPoint};
                    let mut path = BezPath::new();
                    let mut iter = points.iter();
                    if let Some(&(x, y)) = iter.next() {
                        path.push(PathEl::MoveTo(KurboPoint::new(to_x(x), to_y(y))));
                        for &(x, y) in iter {
                            path.push(PathEl::LineTo(KurboPoint::new(to_x(x), to_y(y))));
                        }
                        if closed {
                            path.push(PathEl::ClosePath);
                        }
                    }
                    if let Some(f) = fill {
                        scene.fill(Fill::NonZero, Affine::IDENTITY, to_color(f), None, &path);
                    }
                    if let Some(st) = stroke {
                        let stroke_obj = Stroke::new(stroke_w as f64 * s);
                        scene.stroke(&stroke_obj, Affine::IDENTITY, to_color(st), None, &path);
                    }
                }
                puriy_engine::SvgPrim::Path { ref d, fill, stroke, stroke_w } => {
                    use llimphi_raster::kurbo::{BezPath, PathEl, Point as KurboPoint};
                    let mut path = BezPath::new();
                    for cmd in d {
                        match *cmd {
                            puriy_engine::PathCmd::MoveTo(x, y) => {
                                path.push(PathEl::MoveTo(KurboPoint::new(to_x(x), to_y(y))));
                            }
                            puriy_engine::PathCmd::LineTo(x, y) => {
                                path.push(PathEl::LineTo(KurboPoint::new(to_x(x), to_y(y))));
                            }
                            puriy_engine::PathCmd::CubicTo(x1, y1, x2, y2, x, y) => {
                                path.push(PathEl::CurveTo(
                                    KurboPoint::new(to_x(x1), to_y(y1)),
                                    KurboPoint::new(to_x(x2), to_y(y2)),
                                    KurboPoint::new(to_x(x), to_y(y)),
                                ));
                            }
                            puriy_engine::PathCmd::QuadTo(x1, y1, x, y) => {
                                path.push(PathEl::QuadTo(
                                    KurboPoint::new(to_x(x1), to_y(y1)),
                                    KurboPoint::new(to_x(x), to_y(y)),
                                ));
                            }
                            puriy_engine::PathCmd::ClosePath => {
                                path.push(PathEl::ClosePath);
                            }
                        }
                    }
                    if let Some(f) = fill {
                        scene.fill(Fill::NonZero, Affine::IDENTITY, to_color(f), None, &path);
                    }
                    if let Some(st) = stroke {
                        let stroke_obj = Stroke::new(stroke_w as f64 * s);
                        scene.stroke(&stroke_obj, Affine::IDENTITY, to_color(st), None, &path);
                    }
                }
            }
        }
    })
}
