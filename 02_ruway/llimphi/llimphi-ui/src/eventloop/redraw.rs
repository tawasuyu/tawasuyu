// redraw.rs — Ciclo completo de repintado: layout → paint → GPU → present.
// Esta función es la ruta caliente: mount + compute + animaciones + vello +
// pasadas GPU + present + retención de frame.

use super::super::*;
use super::helpers::{
    build_selectable_layout, resolve_layout_builders, selectable_node_in,
};
use super::push_a11y_tree;

/// Ejecuta la pasada completa de redraw para la ventana primaria.
/// Se llama desde `handle_primary_window_event` cuando el evento es
/// `WindowEvent::RedrawRequested`. Recibe `state` y `handle` separados para
/// facilitar el borrow-checker (no necesita `&mut Runtime<A>` completo).
pub(super) fn handle_redraw<A: App>(
    state: &mut RuntimeState<A>,
    handle: &Handle<A::Msg>,
) {
    // **Retención de frame entero**. Si:
    //  (a) hay scene retenida del frame anterior (`retained`),
    //  (b) `last_render` SIGUE siendo `Some` — la invariante del
    //      runtime es que cualquier handler que muta visualmente
    //      pone `last_render = None`, así que `Some` ⇒ nadie tocó
    //      nada que afecte la pintura,
    //  (c) el frame retenido NO estaba animando ni ripplando
    //      (si lo estaba, el ticker NECESITA avanzarlo),
    //  (d) no hay overlay, drag, ni long-press en curso (camino
    //      conservador: esos casos suelen estar acoplados a
    //      cambios visuales que no atraviesan `last_render`),
    //  (e) el viewport sigue del mismo tamaño,
    // entonces `state.scene` ya tiene EXACTAMENTE lo que hay que
    // mostrar. Saltamos mount + layout + paint y solo hacemos un
    // render+present de la scene retenida. Cubre redraws espurios
    // (expose del compositor, refocus, el último frame de una anim
    // ya asentada). Si algo falla en el acquire, caemos al camino
    // completo (no es un error, sólo un viewport efímero).
    let cache_hit = state.last_render.is_some()
        && state.drag.is_none()
        && state.pending_long_press.is_none()
        && state.retained.as_ref().is_some_and(|r| {
            !r.animating
                && !r.rippling
                && !r.has_overlay
                && (r.w, r.h) == state.surface.size()
        });
    if cache_hit {
        match state.surface.acquire() {
            Ok(frame) => {
                if state
                    .renderer
                    .render(&state.hal, &state.scene, &frame, palette::css::BLACK)
                    .is_ok()
                {
                    state.surface.present(frame, &state.hal);
                    return;
                }
                // render falló → cae al camino completo
            }
            Err(_) => { /* surface efímera → camino completo */ }
        }
    }
    // Título dinámico (App::window_title): si cambió respecto del
    // último aplicado, se lo pasamos a winit. Barato: una
    // comparación de String por frame, set_title sólo en el cambio.
    if let Some(t) = A::window_title(state.model.as_ref().expect("model")) {
        if state.last_title.as_deref() != Some(t.as_str()) {
            state.window.set_title(&t);
            state.last_title = Some(t);
        }
    }
    // Posicioná la ventana de candidatos del IME junto al caret
    // (sólo con IME activo y si la app reporta el área).
    if A::ime_allowed() {
        if let Some((x, y, w, h)) =
            A::ime_cursor_area(state.model.as_ref().expect("model"))
        {
            state.window.set_ime_cursor_area(
                llimphi_hal::winit::dpi::PhysicalPosition::new(x as f64, y as f64),
                llimphi_hal::winit::dpi::PhysicalSize::new(
                    w.max(1.0) as u32,
                    h.max(1.0) as u32,
                ),
            );
        }
    }
    let frame = match state.surface.acquire() {
        Ok(f) => f,
        Err(_) => {
            let (w, h) = state.surface.size();
            state.surface.resize(w, h);
            state.window.request_redraw();
            return;
        }
    };
    let (w, h) = frame.size();
    // LayoutBuilder: resuelve los constructores diferidos en dos
    // pasadas (coste cero si no hay ninguno). Necesita el typesetter
    // para medir, así que va antes de tomar `model_ref` para el overlay.
    let mut view = resolve_layout_builders::<A>(
        state.model.as_ref().expect("model"),
        (w as f32, h as f32),
        &mut state.typesetter,
    );
    // Animaciones implícitas de **tamaño** (`View::animated_size`):
    // reconcila el `View` tree y parcha `style.size` ANTES del
    // mount/layout. Así siblings/hijos reflowean suave (la
    // animación se ve en el layout cascade, no sólo en el rect del
    // nodo aislado). Coste cero sin nodos `animated_size`.
    let frame_now = std::time::Instant::now();
    let size_animating = llimphi_compositor::reconcile_size_anim(
        &mut view,
        &mut state.size_anim_registry,
        frame_now,
    );
    let model_ref = state.model.as_ref().expect("model");
    let overlay_view = A::view_overlay(model_ref);
    // Reusamos los árboles de layout del runtime: `clear()` +
    // `mount` evita re-allocar el slotmap de taffy por frame.
    state.layout.clear();
    let mut mounted: Mounted<A::Msg> = mount(&mut state.layout, view);
    let computed = {
        let ts = &mut state.typesetter;
        let tmap = &mounted.text_measures;
        state
            .layout
            .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                match tmap.get(&nid) {
                    Some(tm) => measure_text_node(ts, tm, known, avail),
                    None => llimphi_layout::taffy::Size::ZERO,
                }
            })
            .expect("layout")
    };
    // Animaciones implícitas (`View::animated`): reconcilia el árbol
    // con el estado retenido DESPUÉS del layout y ANTES del paint —
    // interpola fill/radius de los nodos con `anim`. Si alguna sigue
    // viva pedimos otro frame al final (ticker autodetenido).
    let now = frame_now;
    let anim_active = state.anim_registry.reconcile(&mut mounted, now);
    // Heroes (`View::hero`): si la misma key cambió de rect entre
    // frames, escribe en `transform` la afín que "vuela" del rect
    // anterior al actual. Independiente del anim_registry — sólo
    // toca `transform`, que el paint ya respeta. Coste cero sin
    // nodos hero.
    let hero_active = state.hero_registry.reconcile(&mut mounted, &computed, now);
    // `size_animating` viene del reconcile previo al mount; lo
    // ORrijimos al `animating` global para que se pida el
    // próximo frame y el `retained.animating == true` invalide
    // la cache de retención (la siguiente pasada reconstruye con
    // el size interpolado).
    let animating = anim_active || hero_active || size_animating;
    // Mount + layout del overlay en un árbol aparte. Lo
    // computamos con el mismo tamaño de viewport para que
    // un scrim a percent(1.0) cubra toda la pantalla.
    let overlay_built = if let Some(v) = overlay_view {
        state.overlay_layout.clear();
        let omounted: Mounted<A::Msg> = mount(&mut state.overlay_layout, v);
        let ocomputed = {
            let ts = &mut state.typesetter;
            let tmap = &omounted.text_measures;
            state
                .overlay_layout
                .compute_with_measure(omounted.root, (w as f32, h as f32), |nid, known, avail| {
                    match tmap.get(&nid) {
                        Some(tm) => measure_text_node(ts, tm, known, avail),
                        None => llimphi_layout::taffy::Size::ZERO,
                    }
                })
                .expect("layout overlay")
        };
        let ohover = hit_test_hover(
            &omounted,
            &ocomputed,
            state.cursor.x as f32,
            state.cursor.y as f32,
        );
        Some(OverlayCache {
            mounted: omounted,
            computed: ocomputed,
            hover_idx: ohover,
        })
    } else {
        None
    };
    // Hover en el main solo si NO hay overlay — durante un
    // menú abierto, el fondo no debe reaccionar al ratón.
    let hover_idx = if overlay_built.is_some() {
        None
    } else {
        hit_test_hover(
            &mounted,
            &computed,
            state.cursor.x as f32,
            state.cursor.y as f32,
        )
    };
    // Drop hover sólo si hay drag activo con payload (un
    // drag bloquea el overlay; rara combinación pero la
    // resolvemos a favor del drag).
    let drop_hover_idx = state
        .drag
        .as_ref()
        .and_then(|d| d.payload.map(|_| ()))
        .and_then(|_| {
            hit_test_drop(
                &mounted,
                &computed,
                state.cursor.x as f32,
                state.cursor.y as f32,
            )
        });
    // Z-order del overlay sobre contenido `gpu_paint`: si el
    // árbol principal tiene painters gpu (p. ej. el video de
    // media) Y hay un overlay activo, el overlay NO va en la
    // escena principal (quedaría debajo del blit gpu). Se
    // rasteriza aparte sobre fondo transparente y se compone con
    // alpha DESPUÉS del pase gpu. Sin gpu o sin overlay, el camino
    // de siempre (overlay en la escena principal) — coste cero.
    let composite_overlay =
        overlay_built.is_some() && has_gpu_painter(&mounted);

    state.scene.reset();
    paint(
        &mut state.scene,
        &mounted,
        &computed,
        &mut state.typesetter,
        hover_idx,
        drop_hover_idx,
    );
    // Animación de salida (fade-out). 1) Capturá la subescena de
    // cada nodo `exit` presente (snapshot para cuando desaparezca).
    // 2) Reproducí los fantasmas de los que ya se fueron, con
    // opacidad decreciente — por encima del contenido, debajo del
    // overlay. Coste cero si ningún nodo usa `animated_exit`.
    for (idx, end, key) in state.anim_registry.live_exit_nodes(&mounted) {
        let (dur, easing) = {
            let a = mounted.nodes[idx].anim.expect("nodo exit lleva anim");
            (a.duration, a.easing)
        };
        let mut sub = vello::Scene::new();
        paint_range(
            &mut sub,
            &mounted,
            &computed,
            &mut state.typesetter,
            None,
            None,
            idx,
            end,
            vello::kurbo::Affine::IDENTITY,
        );
        state.anim_registry.store_live_exit(key, sub, dur, easing);
    }
    state
        .anim_registry
        .replay_ghosts(&mut state.scene, now, w as f32, h as f32);
    // Resaltado de la selección de texto activa (sobre el
    // contenido, bajo el overlay). Reconstruye el layout del nodo
    // seleccionado y pinta los rects de `parley::Selection` con un
    // tinte translúcido (deja leer el texto debajo).
    if let Some(tsel) = state.selection {
        if let Some((spec, (rx, ry, rw, _rh))) =
            selectable_node_in(&mounted, &computed, tsel.key)
        {
            let layout = build_selectable_layout(&mut state.typesetter, &spec, rw);
            use vello::kurbo::{Affine, Rect};
            use vello::peniko::{Color, Fill};
            let hl = Color::from_rgba8(86, 148, 246, 80);
            let scene = &mut state.scene;
            tsel.sel.geometry_with(&layout, |bb, _line| {
                let r = Rect::new(
                    rx as f64 + bb.x0,
                    ry as f64 + bb.y0,
                    rx as f64 + bb.x1,
                    ry as f64 + bb.y1,
                );
                scene.fill(Fill::NonZero, Affine::IDENTITY, hl, None, &r);
            });
        }
    }
    // Ripples/InkWell: las salpicaduras vivas se pintan sobre el
    // contenido (translúcidas, recortadas al nodo) y debajo del
    // overlay. Si alguna sigue viva, pide otro frame al final.
    let rippling =
        state
            .ripple_registry
            .paint(&mut state.scene, &mounted, &computed, now);
    if !composite_overlay {
        if let Some(ov) = overlay_built.as_ref() {
            paint(
                &mut state.scene,
                &ov.mounted,
                &ov.computed,
                &mut state.typesetter,
                ov.hover_idx,
                None,
            );
        }
    }
    if let Err(e) = state.renderer.render(
        &state.hal,
        &state.scene,
        &frame,
        palette::css::BLACK,
    ) {
        eprintln!("render error: {e}");
    }
    let (vw, vh) = frame.size();
    // Capa de overlay aparte (camino composite): vello la
    // rasteriza con fondo transparente en `frame.overlay_view()`.
    // Se renderiza ANTES del pase gpu para que el blit del
    // compositor (en `gpu_encoder`) la lea ya escrita.
    if composite_overlay {
        if let Some(ov) = overlay_built.as_ref() {
            state.scene.reset();
            paint(
                &mut state.scene,
                &ov.mounted,
                &ov.computed,
                &mut state.typesetter,
                ov.hover_idx,
                None,
            );
            if let Err(e) = state.renderer.render_to_view(
                &state.hal,
                &state.scene,
                frame.overlay_view(),
                vw,
                vh,
                palette::css::TRANSPARENT,
            ) {
                eprintln!("render overlay error: {e}");
            }
        }
    }
    // Pasada GPU directo (Fase 1 del SDD §"GPU directo wgpu"):
    // si algún View del main o del overlay registró un
    // `gpu_painter`, ejecutamos todos sus callbacks contra un
    // único `CommandEncoder`, encima de lo que vello acaba de
    // pintar sobre la intermediate. Submitimos antes del
    // present para que el blit al swapchain incluya las
    // primitivas GPU. Si nadie usó el hook, no se crea ni
    // submitea nada — coste cero.
    let mut gpu_encoder = state.hal.device.create_command_encoder(
        &llimphi_hal::wgpu::CommandEncoderDescriptor {
            label: Some("llimphi-ui-gpu-paint"),
        },
    );
    let viewport = frame.size();
    // Backdrop blur (Bloque 11): post-pasada Gauss separable sobre
    // la intermediate, restringida al rect de cada nodo
    // `.backdrop_blur(sigma)`. Sucede TRAS la rasterización vello
    // y ANTES de los `gpu_painter`/composite — los painters GPU
    // que se solapen con el blur ven el rect ya borroneado y se
    // dibujan encima nítidos. Coste cero sin nodos blur (loop
    // vacío + bandera `blurred` queda false).
    let backdrop_blurs =
        llimphi_compositor::collect_backdrop_blurs(&mounted, &computed);
    let blurred = !backdrop_blurs.is_empty();
    for b in &backdrop_blurs {
        state.blur_compositor.blur(
            &state.hal.device,
            &state.hal.queue,
            &mut gpu_encoder,
            frame.view(),
            viewport,
            b.rect,
            b.sigma,
        );
    }
    let mut any_gpu = blurred
        | paint_gpu(
            &mounted,
            &computed,
            &state.hal.device,
            &state.hal.queue,
            &mut gpu_encoder,
            frame.view(),
            viewport,
        );
    if let Some(ov) = overlay_built.as_ref() {
        // En el camino composite, los painters gpu del overlay van
        // sobre SU textura; si no, sobre la intermedia.
        let target = if composite_overlay {
            frame.overlay_view()
        } else {
            frame.view()
        };
        any_gpu |= paint_gpu(
            &ov.mounted,
            &ov.computed,
            &state.hal.device,
            &state.hal.queue,
            &mut gpu_encoder,
            target,
            viewport,
        );
    }
    // Composición alpha del overlay SOBRE la intermedia (que ya
    // tiene UI + video). Último pase del encoder → corre después
    // del blit del video. Garantiza menús por encima del video.
    if composite_overlay {
        state.overlay_compositor.composite(
            &state.hal.device,
            &mut gpu_encoder,
            frame.view(),
            frame.overlay_view(),
        );
        any_gpu = true;
    }
    if any_gpu {
        state
            .hal
            .queue
            .submit(std::iter::once(gpu_encoder.finish()));
    }
    state.surface.present(frame, &state.hal);
    // Ticker de animaciones implícitas: si quedó alguna en curso,
    // pedí el próximo frame. Cuando todas se asientan, `animating`
    // queda false y el loop de redraws se detiene solo (sin render
    // ocioso, sin spawn_periodic por animación).
    if animating || rippling {
        state.window.request_redraw();
    }
    state.retained = Some(RetainedScene {
        w,
        h,
        animating,
        rippling,
        has_overlay: overlay_built.is_some(),
    });
    state.last_render = Some(RenderCache {
        mounted,
        computed,
        hover_idx,
        drop_hover_idx,
        overlay: overlay_built,
    });
    // AccessKit: tras un paint exitoso, empujamos el árbol al
    // adapter. `update_if_active` se salta el closure si no hay
    // tecnología asistiva escuchando — coste cero en ese caso.
    push_a11y_tree::<A>(state);

    // `handle` se recibe pero el redraw no lo necesita directamente;
    // se pasa para mantener la firma consistente con el caller.
    let _ = handle;
}
