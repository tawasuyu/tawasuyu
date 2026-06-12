use super::*;

// Geometría fija del panel de output. Debe coincidir EXACTAMENTE con los
// `Style` de `output_pane`/`command_card`: el scroll calcula `content_h`
// con estas constantes (no medimos el árbol; con alturas fijas alcanza).
pub(crate) const PANE_PAD_V: f32 = 12.0; // padding top 6 + bottom 6 del column interno
pub(crate) const PANE_GAP: f32 = 6.0; // gap entre cards / líneas sueltas
pub(crate) const CARD_PAD_V: f32 = 9.0; // card padding top 4 + bottom 5
pub(crate) const CARD_GAP: f32 = 2.0; // gap entre hijos de la card
pub(crate) const HEADER_H: f32 = 20.0; // header de la card
pub(crate) const STAGES_H: f32 = 20.0; // fila de etapas de pipe
pub(crate) const ROW_H: f32 = 16.0; // una línea de output

/// Duración del fade de colapso/despliegue de los bloques del output.
pub(crate) const COLLAPSE_ANIM: std::time::Duration = std::time::Duration::from_millis(160);

/// Sobre cuántos comandos hacia atrás se difumina el negro de recencia: el
/// más reciente es negro profundo, y al cabo de `RECENCY_FADE` comandos el
/// fondo llega al tono normal de card.
pub(crate) const RECENCY_FADE: f32 = 6.0;

/// Mezcla lineal de dos colores sRGB (`t=0` → `a`, `t=1` → `b`).
pub(crate) fn mix_color(
    a: llimphi_ui::llimphi_raster::peniko::Color,
    b: llimphi_ui::llimphi_raster::peniko::Color,
    t: f32,
) -> llimphi_ui::llimphi_raster::peniko::Color {
    use llimphi_ui::llimphi_raster::peniko::Color;
    let t = t.clamp(0.0, 1.0);
    let ca = a.components;
    let cb = b.components;
    Color::from_rgba8(
        ((ca[0] + (cb[0] - ca[0]) * t) * 255.0).round() as u8,
        ((ca[1] + (cb[1] - ca[1]) * t) * 255.0).round() as u8,
        ((ca[2] + (cb[2] - ca[2]) * t) * 255.0).round() as u8,
        255,
    )
}

/// Fondo de una card según su `depth` de recencia (0 = más reciente, negro
/// profundo; 1 = viejo, tono normal `bg_panel_alt`).
pub(crate) fn recency_base(theme: &Theme, depth: f32) -> llimphi_ui::llimphi_raster::peniko::Color {
    // Negro profundo derivado del tema (canal × 0.28) — para temas oscuros
    // queda casi negro; para claros, un gris hundido.
    let alt = theme.bg_panel_alt.components;
    use llimphi_ui::llimphi_raster::peniko::Color;
    let deep = Color::from_rgba8(
        (alt[0] * 0.28 * 255.0).round() as u8,
        (alt[1] * 0.28 * 255.0).round() as u8,
        (alt[2] * 0.28 * 255.0).round() as u8,
        255,
    );
    mix_color(deep, theme.bg_panel_alt, depth)
}

pub(crate) fn output_pane<HostMsg: Clone + 'static>(
    state: &State,
    theme: &Theme,
    lift: &(impl Fn(Msg) -> HostMsg + Clone + Send + Sync + 'static),
) -> View<HostMsg> {
    const MAX_VISIBLE: usize = 400;
    let start = state.output.len().saturating_sub(MAX_VISIBLE);
    let visible = &state.output[start..];

    // Agrupamos por `block` COLECTANDO todas las líneas del bloque aunque
    // se intercalen en el buffer (un job de fondo que escupe entre líneas
    // del foreground ya no fragmenta ni contamina ninguna card). El orden
    // de las cards es el de primera aparición del bloque.
    let mut order: Vec<u64> = Vec::new();
    let mut groups: std::collections::HashMap<u64, Vec<&OutputLine>> =
        std::collections::HashMap::new();
    for line in visible {
        if !groups.contains_key(&line.block) {
            order.push(line.block);
        }
        groups.entry(line.block).or_default().push(line);
    }

    // Bloque-comando más reciente visible → ancla del gradiente de recencia:
    // el último es negro profundo, los de más arriba menos negros.
    let newest_cmd = order
        .iter()
        .copied()
        .filter(|id| {
            groups
                .get(id)
                .and_then(|g| g.first())
                .map(|l| l.kind == OutputKind::Prompt)
                .unwrap_or(false)
        })
        .max()
        .unwrap_or(0);

    // Cada item lleva su alto exacto → `content_h` para el scroll.
    let mut items: Vec<(View<HostMsg>, f32)> = Vec::new();
    for id in &order {
        let g = &groups[id];
        // Un bloque REAL (id != 0) va siempre a `command_card` (cuerpo IDE con
        // select/copy/numeración), aunque su línea Prompt se haya recortado del
        // buffer por el tope (output gigante tipo `ls -alR`): antes caía a
        // `render_output_line` (líneas planas, sin IDE). Sólo `id == 0` (líneas
        // huérfanas sin comando dueño) sigue como líneas sueltas. (El render
        // plano que el usuario NO quiere ver — la app existe para desplanar.)
        if *id != 0 {
            // depth 0 = el más reciente (negro profundo); crece hacia atrás.
            let depth = if newest_cmd > 0 {
                (newest_cmd.saturating_sub(*id) as f32 / RECENCY_FADE).clamp(0.0, 1.0)
            } else {
                0.0
            };
            items.push(command_card::<HostMsg>(
                g.as_slice(),
                *id,
                depth,
                state,
                theme,
                lift,
            ));
        } else {
            // Líneas sueltas (notices iniciales sin bloque dueño).
            for &line in g.iter() {
                items.push((
                    render_output_line::<HostMsg>(line, &state.cwd, theme, lift),
                    ROW_H,
                ));
            }
        }
    }

    let content_h = if items.is_empty() {
        PANE_PAD_V
    } else {
        PANE_PAD_V
            + items.iter().map(|(_, h)| *h).sum::<f32>()
            + PANE_GAP * (items.len() as f32 - 1.0)
    };
    let children: Vec<View<HostMsg>> = items.into_iter().map(|(v, _)| v).collect();

    // Scroll: el viewport lo midió el painter el frame anterior. Por
    // defecto pegado al fondo (lo último visible, como una terminal);
    // `scroll_px` (rueda) desplaza hacia el historial. Publicamos el
    // overflow para que `Msg::Scroll` clampe sin recomputar geometría.
    let viewport_h = state.out_viewport_h.lock().map(|g| *g).unwrap_or(0.0);
    let overflow = (content_h - viewport_h).max(0.0);
    if let Ok(mut g) = state.out_overflow.lock() {
        *g = overflow;
    }
    let ty: f64 = if viewport_h < 1.0 {
        0.0 // primer frame, todavía sin medir → tope
    } else {
        (state.scroll_px.clamp(0.0, overflow) - overflow) as f64
    };

    let inner = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        padding: Rect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(6.0_f32),
            bottom: length(6.0_f32),
        },
        gap: Size {
            width: length(0.0_f32),
            height: length(PANE_GAP),
        },
        align_items: Some(AlignItems::Stretch),
        ..Default::default()
    })
    .transform(vello::kurbo::Affine::translate((0.0, ty)))
    .children(children);

    // El painter publica el alto del viewport; coexiste con los hijos
    // (el compositor pinta painter y luego children).
    let slot = Arc::clone(&state.out_viewport_h);
    let painter = move |_scene: &mut vello::Scene,
                        _ts: &mut llimphi_ui::llimphi_text::Typesetter,
                        rect: llimphi_ui::PaintRect| {
        if let Ok(mut g) = slot.lock() {
            *g = rect.h;
        }
    };

    // Barra de scroll arrastrable, sobre la geometría canónica de
    // `llimphi-widget-scroll` (su `thumb_geometry` es público justo para
    // callers que pintan su propia barra dentro de su layout). Sólo cuando
    // hay overflow y ya medimos el viewport. Da el eje "arrastre" del scroll
    // (la rueda ya entra por `on_wheel` del chasis) + indicador visible.
    let mut pane_children = vec![inner];
    if overflow > 0.5 && viewport_h > 1.0 {
        // `scroll_px` mide px desde el fondo; `thumb_geometry` quiere offset
        // desde el tope. offset_top=0 (thumb arriba) ⇔ scroll_px=overflow.
        let offset_top = overflow - state.scroll_px.clamp(0.0, overflow);
        let (thumb_h, thumb_y, offset_per_px) =
            llimphi_widget_scroll::thumb_geometry(offset_top, content_h, viewport_h);
        let pal = llimphi_widget_scroll::ScrollPalette::from_theme(theme);
        let bar_w = pal.bar_width;
        // Track tenue de fondo, a lo alto del viewport.
        pane_children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: auto(),
                    right: length(1.0_f32),
                    top: length(0.0_f32),
                    bottom: auto(),
                },
                size: Size {
                    width: length(bar_w),
                    height: length(viewport_h),
                },
                ..Default::default()
            })
            .fill(pal.track)
            .radius((bar_w / 2.0) as f64),
        );
        // Thumb arrastrable. Arrastrar hacia abajo (dy>0) lleva al fondo:
        // el offset-desde-el-tope sube, así que `scroll_px` (desde el fondo)
        // baja → `Scroll(-dy * offset_per_px)`.
        let lift_drag = (*lift).clone();
        pane_children.push(
            View::new(Style {
                position: Position::Absolute,
                inset: Rect {
                    left: auto(),
                    right: length(1.0_f32),
                    top: length(thumb_y),
                    bottom: auto(),
                },
                size: Size {
                    width: length(bar_w),
                    height: length(thumb_h),
                },
                ..Default::default()
            })
            .fill(pal.thumb)
            .hover_fill(pal.thumb_hover)
            .radius((bar_w / 2.0) as f64)
            .draggable(move |_phase, _dx, dy| {
                if dy == 0.0 {
                    None
                } else {
                    Some(lift_drag(Msg::Scroll(-dy * offset_per_px)))
                }
            }),
        );
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size {
            width: percent(1.0_f32),
            height: Dimension::auto(),
        },
        // Región scrolleable en una flex column: `flex_basis: 0` +
        // `min_height: 0` para que tome SÓLO el espacio sobrante (tras el
        // header y el input) y NO el tamaño de su contenido. Sin esto el
        // alto del contenido (un `ls` largo) se filtra al flex-basis y el
        // panel aplasta/expulsa el input. El contenido se clipa adentro.
        flex_basis: length(0.0_f32),
        flex_grow: 1.0,
        min_size: Size {
            width: Dimension::auto(),
            height: length(0.0_f32),
        },
        ..Default::default()
    })
    // Superficie hundida (un escalón más profunda que el chrome): el output
    // se lee recesado y con más contraste, como un panel de terminal. Las
    // cards (`bg_panel_alt`) flotan por encima.
    .fill(theme.sunken())
    .radius(3.0)
    .clip(true)
    .paint_with(painter)
    .children(pane_children)
}
