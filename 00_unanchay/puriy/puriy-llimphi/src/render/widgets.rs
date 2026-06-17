use super::*;

/// `<input type=text>` / `<input type=search>` / `<input type=password>` /
/// `<textarea>`: arma un widget `text_input_view` ligado al
/// `TextInputState` del slot DFS `idx`. Click→focus dispara
/// `Msg::FocusInput(idx)`. El estilo del input se mantiene básico
/// (border gris claro + padding); el font-size hereda del nodo. Sin
/// soporte de submit/Enter por ahora — Enter en un input single-line
/// no hace nada (en un textarea inserta newline via apply_key).
pub(crate) fn render_input(
    b: &BoxNode,
    kind: puriy_engine::InputKind,
    idx: usize,
    ctx: &mut RenderCtx<'_>,
) -> View<Msg> {
    let zoom = ctx.zoom;
    // Checkbox / radio / submit: widgets a parte (no text_input_view).
    match kind {
        puriy_engine::InputKind::Checkbox => {
            return render_checkbox_radio(b, idx, ctx, /* radio */ false);
        }
        puriy_engine::InputKind::Radio => {
            return render_checkbox_radio(b, idx, ctx, /* radio */ true);
        }
        puriy_engine::InputKind::Submit => {
            return render_submit_button(b, idx, ctx);
        }
        _ => {}
    }
    let focused = ctx.focused_input == Some(idx);
    // Estado por slot — usamos un blank si todavía no hay (no debería
    // pasar tras Loaded, pero defensivo).
    let blank = TextInputState::new();
    let state = ctx.inputs.get(idx).unwrap_or(&blank);

    let placeholder = b
        .input_placeholder
        .as_deref()
        .unwrap_or(match kind {
            puriy_engine::InputKind::Search => "buscar…",
            puriy_engine::InputKind::Password => "contraseña",
            puriy_engine::InputKind::TextArea => "",
            _ => "",
        });

    let palette = TextInputPalette::default();
    let input = text_input_view(state, placeholder, focused, &palette, Msg::FocusInput(idx));

    // Tamaño: ancho 100% del contenedor por default (los autores suelen
    // poner `width: 200px` o similar; el CSS engine ya lo materializa
    // como `b.width`). El alto: una línea para text/search/password, un
    // textarea recibe ~5 líneas.
    let line_h = (b.font_size * zoom).max(14.0_f32 * zoom) + 12.0;
    let height = match kind {
        puriy_engine::InputKind::TextArea => line_h * 5.0,
        _ => line_h,
    };
    let css_width = length_to_taffy(b.width, zoom);

    // Background base: CSS background-color del nodo si lo seteó; sino
    // blanco. Cuando está focado y el autor escribió `:focus { background:
    // X }`, aplicamos X.
    let base_bg = b
        .background
        .map(|c| Color::from_rgba8(c.r, c.g, c.b, c.a))
        .unwrap_or(Color::WHITE);
    let bg = if focused {
        b.focus_background
            .map(|c| Color::from_rgba8(c.r, c.g, c.b, c.a))
            .unwrap_or(base_bg)
    } else {
        base_bg
    };
    // ¿El autor proveyó un `outline` activo en `:focus`? Si sí, lo dibuja
    // `apply_decorations`; si no, le damos el ring de cortesía azul.
    let author_outline = focused
        && b.outline.style_active
        && b.outline.width > 0.0
        && b.outline.color.is_some();

    // Caja interna: fill base + radius default + el `text_input_view`. El
    // `border` / `border-radius` / `box-shadow` / `outline` del autor los pinta
    // `apply_decorations` (Fase 7.1243) — antes el text-input los ignoraba. El
    // `.radius(3.0)` baseline lo pisa apply_decorations si el autor fijó
    // `border-radius`. El margin del flow va en la shell externa.
    let inner = View::new(Style {
        size: Size {
            width: css_width.unwrap_or_else(|| length(220.0_f32 * zoom)),
            height: length(height),
        },
        padding: Rect {
            left: length(6.0_f32 * zoom),
            right: length(6.0_f32 * zoom),
            top: length(4.0_f32 * zoom),
            bottom: length(4.0_f32 * zoom),
        },
        ..Default::default()
    })
    .fill(bg)
    .radius(3.0)
    .on_click(Msg::FocusInput(idx))
    .children(vec![input]);
    let inner = apply_decorations(inner, b, zoom);

    // Shell externa: lleva el margin del flow y, cuando el input está focado y
    // el autor NO puso outline, el ring de cortesía azul (feedback de focus
    // gratis). Va aparte de `apply_decorations` porque `paint_with` guarda un
    // solo painter — la shell evita pisar el painter de las decoraciones.
    let mut outer = View::new(Style {
        margin: Rect {
            left: margin_left_lpa(b, zoom),
            right: margin_right_lpa(b, zoom, 0.0),
            top: length(b.margin.top * zoom),
            bottom: length(b.margin.bottom * zoom),
        },
        ..Default::default()
    });
    if focused && !author_outline {
        outer = outer.paint_with(|scene, _ts, rect| {
            let stroke = Stroke::new(2.0);
            let half = stroke.width * 0.5;
            let r = RoundedRect::new(
                rect.x as f64 - half,
                rect.y as f64 - half,
                (rect.x + rect.w) as f64 + half,
                (rect.y + rect.h) as f64 + half,
                3.0 + half,
            );
            scene.stroke(
                &stroke,
                Affine::IDENTITY,
                Color::from_rgba8(40, 110, 220, 255),
                None,
                &r,
            );
        });
    }
    outer.children(vec![inner])
}

/// Color del glifo de un checkbox/radio según `accent-color` (Fase 7.1238).
/// Sólo el estado MARCADO toma el accent: los navegadores colorean el "fill"
/// del control (☑ / ●) pero dejan el contorno vacío (☐ / ○) en el gris neutro.
/// `accent == None` (CSS `auto`) o el control desmarcado ⇒ gris neutro.
pub(crate) fn checkbox_glyph_color(
    accent: Option<puriy_engine::Color>,
    checked: bool,
) -> Color {
    let neutral = Color::from_rgb8(40, 40, 50);
    match accent {
        Some(c) if checked => Color::from_rgba8(c.r, c.g, c.b, c.a),
        _ => neutral,
    }
}

/// `<input type=checkbox|radio>`: caja chica con `☐`/`☑` (o circle
/// vacío/lleno para radio) clickeable. Sin label asociada — el `<label
/// for="...">` no se cablea todavía, pero el click sobre el widget
/// alcanza para toggle.
pub(crate) fn render_checkbox_radio(
    b: &BoxNode,
    idx: usize,
    ctx: &mut RenderCtx<'_>,
    radio: bool,
) -> View<Msg> {
    let zoom = ctx.zoom;
    let checked = ctx.input_checks.get(idx).copied().unwrap_or(false);
    let glyph = if radio {
        if checked { "●" } else { "○" }
    } else if checked {
        "☑"
    } else {
        "☐"
    };
    let msg = if radio { Msg::SelectRadio(idx) } else { Msg::ToggleCheckbox(idx) };
    let size_px = (b.font_size * zoom).max(14.0 * zoom);
    // `appearance: none` (Fase 7.1240): apaga el chrome nativo del control —sin
    // el glifo ☑/●/☐/○— y lo pinta como una caja normal con el `background` y
    // las decoraciones del autor (border/radius/shadow), clickeable. Patrón
    // canónico de toggles custom: `appearance:none` + `:checked { background }`
    // + tamaño/borde del autor. El estado marcado lo refleja `:checked` del
    // autor (match estático del atributo `checked`); el toggle dinámico sigue
    // disparando el `Msg`. Tamaño: ancho/alto del autor si los fijó, si no la
    // caja chica default (igual que el chrome nativo).
    if matches!(b.appearance, puriy_engine::Appearance::None) {
        let default_dim = length(size_px + 4.0);
        let w = length_to_taffy(b.width, zoom).unwrap_or(default_dim);
        let h = length_to_taffy(b.height, zoom).unwrap_or(default_dim);
        let mut view = View::new(Style {
            size: Size { width: w, height: h },
            margin: Rect {
                left: margin_left_lpa(b, zoom),
                right: margin_right_lpa(b, zoom, 4.0),
                top: length(b.margin.top * zoom),
                bottom: length(b.margin.bottom * zoom),
            },
            ..Default::default()
        })
        .on_click(msg);
        if let Some(bg) = b.background {
            view = view.fill(Color::from_rgba8(bg.r, bg.g, bg.b, bg.a));
        }
        return apply_decorations(view, b, zoom);
    }
    // `accent-color` (Fase 7.1238): tinta el estado MARCADO del control (el
    // "fill" del ☑ / ●). El dato llega heredado al box (Fase 7.239).
    let glyph_color = checkbox_glyph_color(b.accent_color, checked);
    View::new(Style {
        size: Size {
            width: length(size_px + 4.0),
            height: length(size_px + 4.0),
        },
        margin: Rect {
            left: margin_left_lpa(b, zoom),
            right: margin_right_lpa(b, zoom, 4.0),
            top: length(b.margin.top * zoom),
            bottom: length(b.margin.bottom * zoom),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .on_click(msg)
    .text_aligned(
        glyph.to_string(),
        size_px,
        glyph_color,
        Alignment::Center,
    )
}

/// `<input type=submit|button>` — botón con label desde `value` o
/// default `Submit`. Click submitea el form.
pub(crate) fn render_submit_button(b: &BoxNode, idx: usize, ctx: &mut RenderCtx<'_>) -> View<Msg> {
    let zoom = ctx.zoom;
    let label = b
        .input_initial
        .clone()
        .unwrap_or_else(|| "Submit".to_string());
    let css_width = length_to_taffy(b.width, zoom);
    let h = (b.font_size * zoom).max(14.0 * zoom) + 12.0;
    // `appearance: none` (Fase 7.1242): apaga el chrome nativo del botón —el
    // fondo gris y el radius default— y deja sólo el estilo del autor:
    // `background` + color del texto + decoraciones (border/radius/shadow vía
    // `apply_decorations`). Con `appearance: auto` (default) el botón conserva su
    // look nativo gris clickeable.
    let bare = matches!(b.appearance, puriy_engine::Appearance::None);
    let mut view = View::new(Style {
        size: Size {
            width: css_width.unwrap_or_else(|| length(120.0 * zoom)),
            height: length(h),
        },
        padding: Rect {
            left: length(10.0 * zoom),
            right: length(10.0 * zoom),
            top: length(6.0 * zoom),
            bottom: length(6.0 * zoom),
        },
        margin: Rect {
            left: margin_left_lpa(b, zoom),
            right: margin_right_lpa(b, zoom, 0.0),
            top: length(b.margin.top * zoom),
            bottom: length(b.margin.bottom * zoom),
        },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .on_click(Msg::SubmitForm(idx));
    let text_color = if bare {
        Color::from_rgba8(b.color.r, b.color.g, b.color.b, b.color.a)
    } else {
        Color::from_rgb8(30, 30, 40)
    };
    if bare {
        if let Some(bg) = b.background {
            view = view.fill(Color::from_rgba8(bg.r, bg.g, bg.b, bg.a));
        }
        if let Some(hb) = b.hover_background {
            view = view.hover_fill(Color::from_rgba8(hb.r, hb.g, hb.b, hb.a));
        }
        return apply_decorations(
            view.text_aligned(label, b.font_size * zoom, text_color, Alignment::Center),
            b,
            zoom,
        );
    }
    view.fill(Color::from_rgb8(230, 230, 240))
        .hover_fill(Color::from_rgb8(215, 220, 235))
        .radius(3.0)
        .text_aligned(label, b.font_size * zoom, text_color, Alignment::Center)
}

/// `<select>` con `<option>`s: renderea un header click-toggle con la
/// opción elegida + flecha; cuando está abierto, expande la lista
/// debajo. Click en una opción la selecciona y cierra el dropdown.
pub(crate) fn render_select(
    b: &BoxNode,
    info: &puriy_engine::SelectInfo,
    idx: usize,
    ctx: &mut RenderCtx<'_>,
) -> View<Msg> {
    let zoom = ctx.zoom;
    let state = ctx.selects.get(idx);
    let selected = state.map(|s| s.selected).unwrap_or(info.initial);
    let open = state.map(|s| s.open).unwrap_or(false);
    let current_label = info
        .options
        .get(selected)
        .map(|o| o.label.clone())
        .unwrap_or_default();

    // `appearance: none` (Fase 7.1241): apaga el chrome nativo del `<select>` —la
    // flecha ▼/▲ y el doble fondo gris/blanco— y deja sólo el estilo del autor:
    // `background` + decoraciones (border/radius/shadow vía `apply_decorations`).
    // Patrón canónico del dropdown custom: `appearance:none` + `background` +
    // borde del autor (+ su propia flecha como background-image si la quiere). El
    // header sigue siendo click-toggle y la lista expandida vive en el overlay.
    let bare = matches!(b.appearance, puriy_engine::Appearance::None);

    let css_width = length_to_taffy(b.width, zoom);
    let header_h = (b.font_size * zoom).max(14.0_f32 * zoom) + 10.0;
    let mut header_kids: Vec<View<Msg>> = vec![View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(header_h - 8.0) },
        ..Default::default()
    })
    .text_aligned(
        truncate(&current_label, 80),
        b.font_size * zoom,
        Color::from_rgb8(30, 30, 40),
        Alignment::Start,
    )];
    if !bare {
        // Flecha nativa: sólo con chrome (`appearance: auto`).
        header_kids.push(
            View::new(Style {
                size: Size {
                    width: length(14.0_f32 * zoom),
                    height: length(header_h - 8.0),
                },
                ..Default::default()
            })
            .text_aligned(
                if open { "▲".to_string() } else { "▼".to_string() },
                b.font_size * zoom * 0.8,
                Color::from_rgb8(80, 80, 95),
                Alignment::End,
            ),
        );
    }
    let mut header = View::new(Style {
        size: Size {
            width: css_width.clone().unwrap_or_else(|| length(220.0_f32 * zoom)),
            height: length(header_h),
        },
        padding: Rect {
            left: length(8.0_f32 * zoom),
            right: length(8.0_f32 * zoom),
            top: length(4.0_f32 * zoom),
            bottom: length(4.0_f32 * zoom),
        },
        flex_direction: FlexDirection::Row,
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .on_click(Msg::SelectToggle(idx));
    // Con chrome: header blanco redondeado. `appearance:none`: el fondo lo pone
    // el autor (lo aplica el wrapper de abajo), header transparente.
    if !bare {
        header = header.fill(Color::WHITE).radius(3.0);
    }
    let header = header.children(header_kids);

    // El header se rendera siempre; la lista expandida ahora vive en
    // `view_overlay` (popup flotante) cuando `open=true`. Esto evita
    // empujar el flow del documento al abrir un select.
    let _ = (selected, info, open); // ya consumidos en el overlay
    let all: Vec<View<Msg>> = vec![header];

    let mut wrapper = View::new(Style {
        size: Size {
            width: css_width.unwrap_or_else(|| length(220.0_f32 * zoom)),
            height: auto(),
        },
        margin: Rect {
            left: margin_left_lpa(b, zoom),
            right: margin_right_lpa(b, zoom, 0.0),
            top: length(b.margin.top * zoom),
            bottom: length(b.margin.bottom * zoom),
        },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    });
    if bare {
        // Sólo el estilo del autor: `background` + border/radius/shadow.
        if let Some(bg) = b.background {
            wrapper = wrapper.fill(Color::from_rgba8(bg.r, bg.g, bg.b, bg.a));
        }
        return apply_decorations(wrapper.children(all), b, zoom);
    }
    wrapper
        .fill(Color::from_rgb8(220, 220, 230))
        .radius(3.0)
        .children(all)
}

/// Overlay flotante con la lista de opciones del `<select>` abierto.
/// Centrado en la ventana; backdrop semitransparente que cierra el
/// dropdown al clickear fuera de la lista.
pub(crate) fn select_overlay_view(idx: usize, selected: usize, info: puriy_engine::SelectInfo) -> View<Msg> {
    let row_h = 28.0_f32;
    let total_h = (info.options.len() as f32 * row_h).min(360.0);
    let rows: Vec<View<Msg>> = info
        .options
        .iter()
        .enumerate()
        .map(|(i, opt)| {
            let is_sel = i == selected;
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(row_h) },
                padding: Rect {
                    left: length(12.0_f32),
                    right: length(12.0_f32),
                    top: length(4.0_f32),
                    bottom: length(4.0_f32),
                },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .fill(if is_sel {
                Color::from_rgb8(220, 230, 250)
            } else {
                Color::WHITE
            })
            .hover_fill(Color::from_rgb8(238, 240, 248))
            .on_click(Msg::SelectPick(idx, i))
            .text_aligned(
                truncate(&opt.label, 80),
                13.0,
                Color::from_rgb8(30, 30, 40),
                Alignment::Start,
            )
        })
        .collect();

    let list = View::new(Style {
        size: Size { width: length(320.0_f32), height: length(total_h) },
        flex_direction: FlexDirection::Column,
        ..Default::default()
    })
    .fill(Color::from_rgb8(245, 245, 250))
    .radius(4.0)
    .clip(true)
    .children(rows);

    // Backdrop fullscreen con flex centering del list. Click en el
    // backdrop cierra el dropdown via SelectToggle.
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_direction: FlexDirection::Column,
        justify_content: Some(JustifyContent::Center),
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .fill(Color::from_rgba8(0, 0, 0, 60))
    .on_click(Msg::SelectToggle(idx))
    .children(vec![list])
}
