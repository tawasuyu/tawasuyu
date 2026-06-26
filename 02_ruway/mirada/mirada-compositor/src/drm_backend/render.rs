use super::*;

/// One-shot: marca el instante (epoch ms) en que mirada presenta su PRIMER
/// frame sobre el DRM. Sirve para medir el gap del handoff contra el `RELEASED`
/// del splash (ver `SDD-ARRANQUE-SIN-PARPADEO.md` §Verificación). Es un `Once`
/// de proceso para no engordar `DrmState` ni su constructor.
static PRIMER_FRAME: std::sync::Once = std::sync::Once::new();

/// Mezcla del *glow* de foco para una ventana: `0.0` = color sin foco, `1.0` =
/// color con foco. Al ganar foco sube `0→1`; al perderlo baja `1→0`; ambos por
/// `glow_ms` con desaceleración cúbica. Sin transición estampada (`since=None`)
/// o con `glow_ms=0` devuelve el extremo seco según `focused`. Pura y testeada.
fn focus_mix(focused: bool, glow_ms: u32, since: Option<u32>, now: u32) -> f32 {
    if glow_ms == 0 {
        return if focused { 1.0 } else { 0.0 };
    }
    match since {
        Some(s) => {
            let t = (now.saturating_sub(s) as f32 / glow_ms as f32).clamp(0.0, 1.0);
            let e = mirada_brain::Easing::EaseOutCubic.apply(t);
            if focused {
                e
            } else {
                1.0 - e
            }
        }
        None => {
            if focused {
                1.0
            } else {
                0.0
            }
        }
    }
}

/// Interpola dos colores RGBA `[u8;4]` por `m∈[0,1]` (`0`=a, `1`=b), por canal.
fn lerp_rgba(a: [u8; 4], b: [u8; 4], m: f32) -> [u8; 4] {
    let m = m.clamp(0.0, 1.0);
    let mix = |x: u8, y: u8| (x as f32 + (y as f32 - x as f32) * m).round().clamp(0.0, 255.0) as u8;
    [mix(a[0], b[0]), mix(a[1], b[1]), mix(a[2], b[2]), mix(a[3], b[3])]
}

/// Transformación **cover** para escalar por GPU un buffer `sw×sh` a una salida
/// `ow×oh`: devuelve `(offset_x, offset_y, escala)` — escala uniforme = máximo de
/// las dos razones (llena la salida, recorta el sobrante), con el sobrante
/// centrado (offsets ≤ 0). El render pinta el buffer en `(ox,oy)` y lo envuelve en
/// un `RescaleRenderElement` (escala alrededor de ese origen). Pura y testeada —
/// el escalado real lo hace la GPU (no hay resize en CPU por frame).
fn cover_transform(sw: i32, sh: i32, ow: i32, oh: i32) -> (f64, f64, f64) {
    let sw = sw.max(1) as f64;
    let sh = sh.max(1) as f64;
    let ow = ow.max(1) as f64;
    let oh = oh.max(1) as f64;
    let k = (ow / sw).max(oh / sh);
    ((ow - sw * k) / 2.0, (oh - sh * k) / 2.0, k)
}

impl DrmState {
    /// Compone un cuadro por cada salida y avisa a los clientes una sola vez.
    /// Si una salida tiene su `pending_flip` puesto, se saltea hasta el
    /// próximo VBlank. Refresca los búferes de marco una vez al principio.
    pub(super) fn render(&mut self) {
        if !self.active || self.dpms_off {
            // Sesión en otra VT, o pantalla apagada por inactividad (DPMS off):
            // no tocamos la GPU. Componer y encolar un page-flip contra un
            // conector que el kernel tiene en DPMS-off rebota con EINVAL (os
            // error 22) por cada cuadro — un busy-loop de commits fallidos. Al
            // despertar, `set_dpms(false)` baja `dpms_off` *antes* de llamar a
            // `render()`, así el scanout se reanuda en el mismo paso.
            return;
        }
        self.stamp_open_animations();
        self.stamp_focus_animations();
        self.refresh_window_borders();
        // Motor de transición del fade al cerrar: refrescá las instantáneas y
        // hacé avanzar (o retirar) los fantasmas. Ambos son no-op con el efecto
        // apagado (default), así que el camino normal no paga nada.
        self.capture_close_snapshots();
        self.advance_ghosts();
        // Modo DM: si el ratón cruzó a otro monitor, avisale al greeter para
        // que la tarjeta de login viaje allí (no-op fuera de greeter o si el
        // monitor activo no cambió).
        self.sync_greeter_layout(false);
        for i in 0..self.outputs.len() {
            self.render_output(i);
        }
        self.send_frames_to_clients();
        // La marca animada ya se compuso este cuadro (el video resetea su
        // `video_dirty` por-salida en `emit_wallpaper`).
        self.anim_default_dirty = false;
    }

    /// Si el puntero global cae sobre `rect`, emite el cursor en coordenadas
    /// **locales** a `rect`. Si el cliente publicó una superficie de cursor,
    /// usa esa; si no, el cuadrado por defecto. `Hidden` o puntero fuera del
    /// rect no emiten nada.
    fn emit_cursor(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        let (cx, cy) = self.app.pointer_loc;
        let (cxi, cyi) = (cx.round() as i32, cy.round() as i32);
        if cxi < rect.x || cyi < rect.y || cxi >= rect.x + rect.w || cyi >= rect.y + rect.h {
            return;
        }
        // Si el cliente publicó una superficie de cursor sana, esa manda
        // (el cursor de la app). Si no, el puntero del escritorio es "con
        // nombre" (`Named`): lo resolvemos contra el tema de cursor configurado
        // (`Soberania`, etc.); y si el tema no aplica, cae al cuadrado.
        let named = match &self.app.cursor_status {
            CursorImageStatus::Hidden => return,
            CursorImageStatus::Surface(surface)
                if surface.alive() && crate::buffer_render_sano(surface) =>
            {
                let (hx, hy) = crate::cursor_hotspot(surface);
                let loc = (cxi - rect.x - hx, cyi - rect.y - hy);
                for el in render_elements_from_surface_tree(
                    &mut self.renderer,
                    surface,
                    loc,
                    1.0,
                    1.0,
                    Kind::Cursor,
                ) {
                    into.push(Frame::Window(el));
                }
                return;
            }
            CursorImageStatus::Named(icon) => Some(*icon),
            // Superficie no sana u otro estado: tratamos como el puntero por
            // defecto del tema.
            _ => None,
        };

        // Cursor temático: lo subimos como textura (igual que las etiquetas),
        // anclado al hotspot del cursor.
        if self.cursor_theme.is_active() {
            let names = match named {
                Some(icon) => crate::cursor_theme::icon_names(icon),
                None => vec!["default"],
            };
            if let Some(loaded) = self.cursor_theme.get(&names) {
                let (hx, hy) = loaded.hotspot;
                let loc = ((cxi - rect.x - hx) as f64, (cyi - rect.y - hy) as f64);
                if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                    &mut self.renderer,
                    loc,
                    &loaded.buffer,
                    None,
                    None,
                    None,
                    Kind::Cursor,
                ) {
                    into.push(Frame::Text(el));
                    return;
                }
            }
        }

        // Sin tema (o no resolvió): el cuadrado de software de siempre.
        let cursor_rect = Rectangle::new(
            Point::<i32, Physical>::from((cxi - rect.x, cyi - rect.y)),
            Size::<i32, Physical>::from((CURSOR_SIZE, CURSOR_SIZE)),
        );
        into.push(Frame::Solid(SolidColorRenderElement::new(
            self.cursor_id.clone(),
            cursor_rect,
            CommitCounter::default(),
            CURSOR_COLOR,
            Kind::Cursor,
        )));
    }

    /// Sella el instante de apertura de cada ventana recién mapeada — el origen
    /// del fade-in («animaciones de Wayland»). Se sella la primera vez que la
    /// ventana es visible **con buffer sano** (no antes: si no, la rampa correría
    /// sobre fotogramas vacíos y la apertura se vería ya empezada). Una sola vez:
    /// re-mostrar una ventana (cambio de escritorio) no re-anima — de eso se
    /// encarga el slide. Barato: sólo paga mientras quede alguna sin sellar.
    fn stamp_open_animations(&mut self) {
        if self.app.windows.iter().all(|w| w.mapped_ms.is_some()) {
            return;
        }
        let now = self.start.elapsed().as_millis() as u32;
        for w in &mut self.app.windows {
            if w.mapped_ms.is_none() && w.visible && crate::buffer_render_sano(&w.surface) {
                w.mapped_ms = Some(now);
            }
        }
    }

    /// **Motor de transición — captura de instantáneas para el fade al cerrar.**
    /// Con el efecto activo (`window_close_ms>0`), cada ~150 ms saca una foto CPU
    /// del contenido de cada ventana de usuario y la guarda en su
    /// `close_snapshot`; al cerrarse, [`App::toplevel_destroyed`] la convierte en
    /// un fantasma que se desvanece. Estrangulado porque la captura lee de la GPU
    /// (caro). Con el efecto apagado (default) sale de una → costo cero.
    fn capture_close_snapshots(&mut self) {
        let close_ms = self.app.config_window_close_ms();
        if close_ms == 0 || self.outputs.get(Self::PRIMARY).is_none() {
            return;
        }
        const SNAP_INTERVAL_MS: u32 = 150;
        let now = self.start.elapsed().as_millis() as u32;
        let tbh = self.app.decorations.titlebar_height;
        let primary_h = self.outputs[Self::PRIMARY].rect.h;
        for i in 0..self.app.windows.len() {
            // 1) Decidir y medir SIN tomar el renderer (préstamo inmutable corto).
            let plan = {
                let w = &self.app.windows[i];
                let fresca = w.last_snapshot_ms != 0
                    && now.saturating_sub(w.last_snapshot_ms) < SNAP_INTERVAL_MS;
                if w.is_shell || w.is_greeter || !w.visible || fresca {
                    None
                } else if !crate::buffer_render_sano(&w.surface) {
                    None
                } else {
                    let (gx, gy) = crate::render_loc(w, primary_h, tbh);
                    let (sw, sh) = crate::surface_px_size(w).unwrap_or((w.size.0, w.size.1));
                    (sw > 0 && sh > 0).then(|| (w.surface.clone(), gx, gy, sw, sh))
                }
            };
            let Some((surface, gx, gy, sw, sh)) = plan else {
                continue;
            };
            // 2) Renderizar el árbol de superficie a un offscreen → bytes CPU.
            let elems: Vec<WaylandSurfaceRenderElement<GlesRenderer>> =
                render_elements_from_surface_tree(
                    &mut self.renderer,
                    &surface,
                    (0, 0),
                    1.0,
                    1.0,
                    Kind::Unspecified,
                );
            if elems.is_empty() {
                continue;
            }
            if let Some(rgba) =
                crate::screencopy::render_elements_offscreen(&mut self.renderer, (sw, sh), &elems)
            {
                let w = &mut self.app.windows[i];
                w.close_snapshot = Some(crate::CloseSnapshot { rgba, w: sw, h: sh, x: gx, y: gy });
                w.last_snapshot_ms = now;
            }
        }
    }

    /// Sella el `t0` de los fantasmas nuevos, descarta los expirados y, mientras
    /// queden, marca daño para que el fade-out FLUYA (igual que el fade-in).
    fn advance_ghosts(&mut self) {
        if self.app.closing_ghosts.is_empty() {
            return;
        }
        let close_ms = self.app.config_window_close_ms().max(1);
        let now = self.start.elapsed().as_millis() as u32;
        for g in &mut self.app.closing_ghosts {
            if g.t0.is_none() {
                g.t0 = Some(now);
            }
        }
        self.app
            .closing_ghosts
            .retain(|g| g.t0.map_or(true, |t0| now.saturating_sub(t0) < close_ms));
        if !self.app.closing_ghosts.is_empty() {
            crate::screencopy::danar_todo(&mut self.app);
        }
    }

    /// Pinta los fantasmas de cierre que intersectan `rect`: la instantánea
    /// desvaneciéndose (alfa `1→0`, ease-out) y encogiéndose apenas. Coords
    /// globales → locales a la salida, igual que las ventanas.
    fn emit_ghosts(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        if self.app.closing_ghosts.is_empty() {
            return;
        }
        let close_ms = self.app.config_window_close_ms().max(1) as f32;
        let now = self.start.elapsed().as_millis() as u32;
        for idx in 0..self.app.closing_ghosts.len() {
            let (lx, ly, gw, gh, alpha, scale) = {
                let g = &self.app.closing_ghosts[idx];
                let t0 = g.t0.unwrap_or(now);
                let t = (now.saturating_sub(t0) as f32 / close_ms).clamp(0.0, 1.0);
                let eased = mirada_brain::Easing::EaseOutCubic.apply(t);
                let (gw, gh) = (g.snap.w, g.snap.h);
                // Cull por intersección con la salida.
                if g.snap.x + gw <= rect.x
                    || g.snap.y + gh <= rect.y
                    || g.snap.x >= rect.x + rect.w
                    || g.snap.y >= rect.y + rect.h
                {
                    continue;
                }
                (
                    g.snap.x - rect.x,
                    g.snap.y - rect.y,
                    gw,
                    gh,
                    (1.0 - eased).clamp(0.0, 1.0),
                    1.0 - 0.06 * eased,
                )
            };
            let buf = MemoryRenderBuffer::from_slice(
                &self.app.closing_ghosts[idx].snap.rgba,
                Fourcc::Argb8888,
                (gw, gh),
                1,
                Transform::Normal,
                None,
            );
            if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                &mut self.renderer,
                (lx as f64, ly as f64),
                &buf,
                Some(alpha),
                None,
                None,
                Kind::Unspecified,
            ) {
                if (scale - 1.0).abs() < 1e-3 {
                    into.push(Frame::Text(el));
                } else {
                    let origin = Point::<i32, Physical>::from((lx + gw / 2, ly + gh / 2));
                    into.push(Frame::ScaledText(RescaleRenderElement::from_element(
                        el,
                        origin,
                        scale as f64,
                    )));
                }
            }
        }
    }

    /// Estampa el instante de cada cambio de foco — origen del *glow* del marco.
    /// Detecta el flanco (`focused != was_focused`) y sella `focus_ms`. Con el
    /// glow apagado sólo sincroniza `was_focused` (para no disparar un crossfade
    /// retroactivo al prenderlo). Se llama antes de `refresh_window_borders`, que
    /// ya consume el color interpolado.
    fn stamp_focus_animations(&mut self) {
        let glow_ms = self.app.config_focus_glow_ms();
        if glow_ms == 0 {
            for w in &mut self.app.windows {
                w.was_focused = w.focused;
            }
            return;
        }
        let now = self.start.elapsed().as_millis() as u32;
        for w in &mut self.app.windows {
            if w.focused != w.was_focused {
                w.focus_ms = Some(now);
                w.was_focused = w.focused;
            }
        }
    }

    /// Color base del marco/barra de `w` con el *glow* aplicado: interpola entre
    /// `border_normal` y `border_focus` según la mezcla de foco en `now`. En
    /// reposo (sin transición viva) devuelve exactamente el color seco de antes.
    fn focus_base(&self, w: &crate::ManagedWindow, now: u32, glow_ms: u32) -> [u8; 4] {
        let dec = self.app.decorations;
        let m = focus_mix(w.focused, glow_ms, w.focus_ms, now);
        lerp_rgba(dec.border_normal, dec.border_focus, m)
    }

    /// `true` si alguna ventana está dentro de su crossfade de glow de foco. El
    /// `tick` lo usa para forzar repintado mientras dura (igual que el fade-in).
    pub(super) fn focus_anim_active(&self) -> bool {
        let glow_ms = self.app.config_focus_glow_ms();
        if glow_ms == 0 {
            return false;
        }
        let now = self.start.elapsed().as_millis() as u32;
        self.app.windows.iter().any(|w| {
            !w.is_shell && matches!(w.focus_ms, Some(s) if now.saturating_sub(s) < glow_ms)
        })
    }

    /// `true` si alguna ventana de usuario está dentro de su ventana de fade-in
    /// de apertura. El `tick` lo usa para forzar repintado: el damage tracker de
    /// `DrmCompositor` no ve por sí solo el cambio de alfa (sólo geometría), así
    /// que sin esto la rampa se congelaría apenas el cliente deja de mandar
    /// frames. Mismo recurso que el zoom del Prezi.
    pub(super) fn open_anim_active(&self) -> bool {
        let open_ms = self.app.config_window_open_ms();
        if open_ms == 0 {
            return false;
        }
        let now = self.start.elapsed().as_millis() as u32;
        self.app.windows.iter().any(|w| {
            !w.is_shell
                && !w.is_greeter
                && matches!(w.mapped_ms, Some(m) if now.saturating_sub(m) < open_ms)
        })
    }

    /// Factor de alfa del fade-in de apertura para `w` en `now` (ms desde
    /// `start`): `1.0` salvo durante los primeros `open_ms` tras sellarse. El
    /// chrome (shell `pata`, greeter) nunca se desvanece — su aparición la
    /// gobiernan sus propias transiciones, no este fade de ventana de usuario.
    fn open_anim_alpha(&self, w: &crate::ManagedWindow, now: u32, open_ms: u32, easing: mirada_brain::Easing) -> f32 {
        if open_ms == 0 || w.is_shell || w.is_greeter {
            return 1.0;
        }
        match w.mapped_ms {
            Some(m) => {
                let t = (now.saturating_sub(m) as f32 / open_ms as f32).clamp(0.0, 1.0);
                easing.apply(t).clamp(0.0, 1.0)
            }
            // Aún sin sellar (buffer no sano todavía): invisible para no
            // estampar un flash a alfa pleno antes de que arranque la rampa.
            None => 0.0,
        }
    }

    /// Factor de escala del «pop» de apertura para `w`: crece de `start` (p. ej.
    /// 0.92) a `1.0` durante `open_ms`, siguiendo la misma curva que el alfa. Con
    /// `EaseOutBack` el eased rebasa 1.0 → la escala sobre-impulsa y asienta (el
    /// rebote). `1.0` (sin pop) cuando la animación está apagada, `start>=1`, o el
    /// chrome (shell/greeter). El render lo aplica con un `RescaleRenderElement`
    /// centrado en la ventana — el mismo recurso que las miniaturas del Prezi.
    fn open_anim_scale(
        &self,
        w: &crate::ManagedWindow,
        now: u32,
        open_ms: u32,
        easing: mirada_brain::Easing,
        start: f32,
    ) -> f32 {
        if open_ms == 0 || start >= 0.999 || w.is_shell || w.is_greeter {
            return 1.0;
        }
        match w.mapped_ms {
            Some(m) => {
                let t = (now.saturating_sub(m) as f32 / open_ms as f32).clamp(0.0, 1.0);
                // El eased puede pasar de 1.0 (EaseOutBack) → overshoot del pop.
                start + (1.0 - start) * easing.apply(t)
            }
            // Aún sin sellar: arranca chica (cuadra con alfa 0 → no se ve igual).
            None => start,
        }
    }

    /// Emite todas las ventanas visibles cuya posición global intersecta `rect`,
    /// traducidas a coordenadas locales a `rect`. Incluye marcos, barras de
    /// título y el árbol de superficie del cliente, en orden front-to-back
    /// (`shell` arriba > flotantes > teseladas). Se saltea ventanas que no
    /// caen sobre `rect` para no malgastar trabajo del compositor.
    fn emit_windows(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        // FUS: con ≥2 sesiones, sólo se pintan las ventanas de la activa
        // (`session_visible`); con ≤1 el filtro es transparente.
        let mut shown: Vec<_> = self
            .app
            .windows
            .iter()
            .filter(|w| w.visible && self.app.session_visible(w))
            .collect();
        // `is_greeter` al frente del todo: el shell de credenciales (login o
        // lock) tapa la sesión —incluido el shell/pata— mientras está arriba.
        shown.sort_by_key(|w| (!w.is_greeter, !w.is_shell, !w.floating, !w.focused));
        let tbh = self.app.decorations.titlebar_height;
        // `render_loc` necesita el alto de la "salida lógica" sólo para anclar
        // el shell al borde inferior. En multi-monitor esa salida es la
        // primaria (output 0); el shell vive ahí.
        let primary_h = self.outputs[Self::PRIMARY].rect.h;
        // Offset del slide de transición entre escritorios: las ventanas del
        // escritorio entrante se deslizan desde un costado hasta su lugar.
        // Ease-out cúbico; 0 cuando no hay slide.
        let slide_ms = self.app.config_slide_ms().max(1);
        let slide_dx: i32 = match self.ws_slide {
            Some((start_ms, dir)) => {
                let now = self.start.elapsed().as_millis() as u32;
                let t = (now.saturating_sub(start_ms) as f32 / slide_ms as f32).clamp(0.0, 1.0);
                let eased = 1.0 - (1.0 - t).powi(3);
                (dir * rect.w as f32 * (1.0 - eased)) as i32
            }
            None => 0,
        };
        // El slide sólo debe mover las ventanas del monitor ENFOCADO (el que
        // cambió de escritorio). Los demás monitores quedan quietos — si no, un
        // Win+Tab en un monitor sacudía las ventanas del otro.
        let focused_rect = self
            .outputs
            .get(self.app.focused_output_index())
            .map(|o| o.rect);
        let shadows_on = self.shadows_on;
        // Fade-in de apertura: duración + curva (config), y el reloj común. La
        // rampa por-ventana se evalúa abajo (`open_anim_alpha`); con `open_ms=0`
        // o «reducir movimiento» es `1.0` y la composición queda byte-idéntica.
        let open_ms = self.app.config_window_open_ms();
        let open_easing = self.app.config_window_open_easing();
        let open_scale_start = self.app.config_window_open_scale();
        let glow_ms = self.app.config_focus_glow_ms();
        let dim_frac = self.app.config_unfocused_dim();
        let anim_now = self.start.elapsed().as_millis() as u32;

        // Popups (menú de aplicación y contextuales de apps GTK/Qt) PRIMERO: el
        // backend compone front-to-back (el primer elemento queda ARRIBA), así
        // que apilarlos antes de las ventanas los deja por ENCIMA. Antes iban al
        // final = detrás de la ventana → «el menú existe pero no pinta». Se
        // dibujan recursivamente (submenús) relativos al origen de geometría del
        // parent.
        for w in &shown {
            if !crate::buffer_render_sano(&w.surface) {
                continue;
            }
            let (gx, gy) = crate::render_loc(w, primary_h, tbh);
            let on_focused = focused_rect.map_or(true, |fr| gx >= fr.x && gx < fr.x + fr.w);
            let gx = if w.is_shell || !on_focused { gx } else { gx + slide_dx };
            let (off_x, off_y) = crate::content_offset(w);
            emit_popups(&mut self.renderer, &w.surface, (gx + off_x, gy + off_y), rect, into);
        }

        for w in &shown {
            if !crate::buffer_render_sano(&w.surface) {
                continue; // buffer degenerado/desmesurado: ni decoración ni superficie
            }
            let tb = crate::titlebar_for(w, tbh);
            let (gx, gy) = crate::render_loc(w, primary_h, tbh);
            // El marco (pata) no se desliza; tampoco las ventanas de un monitor
            // que no es el enfocado. Sólo el escritorio entrante del monitor que
            // cambió se desliza.
            let on_focused = focused_rect.map_or(true, |fr| gx >= fr.x && gx < fr.x + fr.w);
            let gx = if w.is_shell || !on_focused { gx } else { gx + slide_dx };
            let (sw, sh) = crate::surface_px_size(w).unwrap_or((w.size.0, (w.size.1 - tb).max(1)));
            // Tamaño del CONTENIDO (sin la sombra CSD) y desplazamiento de la
            // sombra dentro del buffer: las decoraciones (barra/marco/sombra)
            // abrazan el contenido, no el buffer-con-sombra. Sin geometría
            // declarada coinciden con el buffer (offset 0).
            let (cw, ch) = crate::content_px_size(w).unwrap_or((sw, sh));
            let (off_x, off_y) = crate::content_offset(w);
            // Rect decorado en coords globales (incluye barra + superficie).
            let gxd = gx;
            let gyd = gy - tb;
            let gwd = sw;
            let ghd = sh + tb;
            // Filtrar por intersección con `rect`.
            if gxd + gwd <= rect.x
                || gyd + ghd <= rect.y
                || gxd >= rect.x + rect.w
                || gyd >= rect.y + rect.h
            {
                continue;
            }
            // Alfa del fade-in de apertura (1.0 fuera de la rampa). Multiplica a
            // TODO lo que pinta esta ventana —decoración, marco, sombra y la
            // superficie— para que entre como un solo bloque que se materializa.
            let anim_alpha = self.open_anim_alpha(w, anim_now, open_ms, open_easing);
            // Escala del pop de apertura (1.0 fuera de la rampa). Si ≠1.0,
            // envolvemos TODO lo que esta ventana empuja en un rescale centrado
            // (ver el flush al final del bloque). `win_start` marca dónde empieza.
            let anim_scale = self.open_anim_scale(w, anim_now, open_ms, open_easing, open_scale_start);
            let win_start = into.len();
            // `x,y` = origen del BUFFER (donde se pinta el árbol de superficie).
            // `cx,cy` = origen del CONTENIDO (buffer + offset de sombra): ahí
            // arrancan barra y marco para que abracen lo visible.
            let x = gx - rect.x;
            let y = gy - rect.y;
            let cx = x + off_x;
            let cy = y + off_y;
            let dec_y = cy - tb;
            let dec_h = ch + tb;

            // Velo de atenuación de las ventanas SIN foco — el primer elemento
            // del grupo de esta ventana = queda ARRIBA (tapa barra+contenido). Su
            // alfa anima con el foco (misma curva que el glow): `focus_mix=1`
            // (enfocada) → sin velo; `0` (sin foco) → velo pleno. Va dentro del
            // grupo `win_start..` para que escale con el pop. Sin el efecto
            // (`dim_frac=0`) o enfocada no se empuja nada → byte-idéntico.
            if dim_frac > 0.0 && !w.is_shell && !w.is_greeter {
                let m = focus_mix(w.focused, glow_ms, w.focus_ms, anim_now);
                let veil = dim_frac * (1.0 - m);
                if veil > 0.001 {
                    let mut scrim = SolidColorBuffer::default();
                    scrim.update((cw, dec_h), [0.0, 0.0, 0.0, veil]);
                    into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                        &scrim,
                        (cx, dec_y),
                        1.0,
                        anim_alpha,
                        Kind::Unspecified,
                    )));
                }
            }

            if tb > 0 {
                if let Some(tr) = &self.text {
                    if !w.title.is_empty() {
                        if self.text_cache.len() > 256 {
                            self.text_cache.clear();
                        }
                        let buf = self
                            .text_cache
                            .entry((w.title.clone(), TITLE_COLOR))
                            .or_insert_with(|| title_buffer(tr, &w.title));
                        let ty = dec_y + (tb - TITLE_PX as i32) / 2;
                        if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                            &mut self.renderer,
                            ((cx + 8) as f64, ty as f64),
                            buf,
                            Some(anim_alpha),
                            None,
                            None,
                            Kind::Unspecified,
                        ) {
                            into.push(Frame::Text(el));
                        }
                    }
                    // Botones del titlebar a la derecha: maximizar (cuadrado) y
                    // cerrar (X). El más a la derecha es cerrar. Se dibujan a
                    // mano (no por fuente): los glyphs ✕/□ salían como tofu en
                    // fuentes sin esos puntos. El hit-test del click usa las
                    // mismas posiciones (TB_BTN_W).
                    let _ = tr; // los íconos no dependen de la fuente
                    for (slot, icon) in [
                        (0i32, crate::text::icon_close(TITLE_PX, TITLE_COLOR)),
                        (1i32, crate::text::icon_square(TITLE_PX, TITLE_COLOR)),
                        (2i32, crate::text::icon_minimize(TITLE_PX, TITLE_COLOR)),
                    ] {
                        if cw < (slot + 1) * crate::TB_BTN_W + 8 {
                            continue; // ventana muy angosta: sin botón
                        }
                        {
                            let r = icon;
                            let cell_x = cx + cw - (slot + 1) * crate::TB_BTN_W;
                            let bx = cell_x + (crate::TB_BTN_W - r.width) / 2;
                            let by = dec_y + (tb - r.height) / 2;
                            let bbuf = MemoryRenderBuffer::from_slice(
                                &r.rgba,
                                Fourcc::Argb8888,
                                (r.width, r.height),
                                1,
                                Transform::Normal,
                                None,
                            );
                            if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                                &mut self.renderer,
                                (bx as f64, by as f64),
                                &bbuf,
                                Some(anim_alpha),
                                None,
                                None,
                                Kind::Unspecified,
                            ) {
                                into.push(Frame::Text(el));
                            }
                        }
                    }
                }
                // Glow de foco: el color de la barra crossfadea como el marco.
                let base = self.focus_base(w, anim_now, glow_ms);
                if self.app.decorations.titlebar_gradient && tb >= 4 {
                    // Degradé vertical (claro arriba → base abajo) por franjas
                    // sólidas: el compositor no tiene primitivo de gradiente, así
                    // que apilamos ~8 bandas de luminancia decreciente.
                    const BANDS: i32 = 8;
                    let band_h = (tb + BANDS - 1) / BANDS; // techo, cubre todo tb
                    for b in 0..BANDS {
                        let by = dec_y + b * band_h;
                        let h = band_h.min(dec_y + tb - by);
                        if h <= 0 {
                            break;
                        }
                        // t: 0 arriba (más claro) → 1 abajo (base). Aclarado del
                        // ~28 % en la banda superior, desvaneciendo a 0.
                        let t = b as f32 / (BANDS - 1) as f32;
                        let lift = 0.28 * (1.0 - t);
                        let shade = |c: u8| {
                            let f = c as f32 / 255.0;
                            (((f + (1.0 - f) * lift).clamp(0.0, 1.0)) * 255.0) as u8
                        };
                        let col = rgba_f32([shade(base[0]), shade(base[1]), shade(base[2]), base[3]]);
                        let mut band = SolidColorBuffer::default();
                        band.update((cw, h), col);
                        into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                            &band,
                            (cx, by),
                            1.0,
                            anim_alpha,
                            Kind::Unspecified,
                        )));
                    }
                } else {
                    let color = rgba_f32(base);
                    let mut bar = SolidColorBuffer::default();
                    bar.update((cw, tb), color);
                    into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                        &bar,
                        (cx, dec_y),
                        1.0,
                        anim_alpha,
                        Kind::Unspecified,
                    )));
                }
            } else if w.focused && w.ssd && !w.is_shell && !w.is_greeter && !w.title.is_empty() {
                // El greeter no lleva ni barra ni este título flotante (aplastaba
                // su barra de menú). Las ventanas CSD (`!w.ssd`) tampoco: el
                // cliente ya pinta su propio título.
                if let Some(tr) = &self.text {
                    if self.text_cache.len() > 256 {
                        self.text_cache.clear();
                    }
                    let buf = self
                        .text_cache
                        .entry((w.title.clone(), TITLE_COLOR))
                        .or_insert_with(|| title_buffer(tr, &w.title));
                    if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                        &mut self.renderer,
                        ((cx + 6) as f64, (cy + 4) as f64),
                        buf,
                        Some(anim_alpha),
                        None,
                        None,
                        Kind::Unspecified,
                    ) {
                        into.push(Frame::Text(el));
                    }
                }
            }
            // Marco del servidor: para SSD siempre; para CSD sólo si está
            // teselada (los estados `tiled` ya le quitaron la sombra, así que el
            // borde abraza el contenido). En CSD flotante lo omitimos: el borde
            // envolvería la sombra invisible del buffer → el «margen grande».
            if !w.is_shell && self.app.decorations.border_width > 0 && (w.ssd || !w.floating) {
                let rects = border_rects(cx, dec_y, cw, dec_h, self.app.decorations.border_width);
                for (buf, (bx, by, _, _)) in w.borders.iter().zip(rects) {
                    into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                        buf,
                        (bx, by),
                        1.0,
                        anim_alpha,
                        Kind::Unspecified,
                    )));
                }
            }
            for el in render_elements_from_surface_tree(
                &mut self.renderer,
                &w.surface,
                (x, y),
                1.0,
                w.effects.opacity as f32 / 255.0 * anim_alpha,
                Kind::Unspecified,
            ) {
                into.push(Frame::Window(el));
            }
            // Sombra: capas negras translúcidas DETRÁS de la ventana (se empujan
            // después del contenido = quedan al fondo). Sin shader — rects que se
            // expanden y se desplazan hacia abajo fingen un degradé suave. Gateada
            // por MIRADA_SHADOW (global) o por el efecto de ventana que fija el
            // Cerebro vía SetEffects (p. ej. sombrear sólo la enfocada).
            if (shadows_on || w.effects.shadow) && !w.is_shell {
                // (expansión, desplazamiento-y, alfa): de afuera-tenue a cerca-fuerte.
                for &(exp, dy, a) in &[(12i32, 10i32, 0.06f32), (6, 5, 0.10), (2, 2, 0.16)] {
                    let mut sh = SolidColorBuffer::default();
                    sh.update((cw + exp * 2, dec_h + exp * 2), [0.0, 0.0, 0.0, a]);
                    into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                        &sh,
                        (cx - exp, dec_y - exp + dy),
                        1.0,
                        anim_alpha,
                        Kind::Unspecified,
                    )));
                }
            }
            // Pop de apertura: si la escala no es neutra, re-empujamos todo lo
            // que esta ventana metió (de `win_start` en adelante) envuelto en un
            // `RescaleRenderElement` centrado en la ventana. En reposo
            // (`anim_scale≈1`) no se toca nada → composición byte-idéntica.
            if (anim_scale - 1.0).abs() > 1e-3 {
                let origin = Point::<i32, Physical>::from((cx + cw / 2, dec_y + dec_h / 2));
                let tail: Vec<Frame<GlesRenderer>> = into.split_off(win_start);
                for f in tail {
                    let s = anim_scale as f64;
                    into.push(match f {
                        Frame::Window(e) => {
                            Frame::ScaledWindow(RescaleRenderElement::from_element(e, origin, s))
                        }
                        Frame::Text(e) => {
                            Frame::ScaledText(RescaleRenderElement::from_element(e, origin, s))
                        }
                        Frame::Solid(e) => {
                            Frame::ScaledSolid(RescaleRenderElement::from_element(e, origin, s))
                        }
                        // Las variantes ya escaladas no aparecen acá (esta ventana
                        // sólo empuja Window/Text/Solid); pasan tal cual por prudencia.
                        other => other,
                    });
                }
            }
        }
    }

    /// Emite el HUD del preset activo en la salida `rect` — un panel discreto
    /// arriba al centro de la salida (no del escritorio global) mientras dure
    /// la ventana de feedback. Si el deadline pasó, limpia el estado. Llamar
    /// sólo en la salida dueña del HUD (hoy la primaria).
    fn emit_hud(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        let Some(deadline) = self.preset_hud_until else {
            return;
        };
        if Instant::now() >= deadline {
            self.preset_hud_until = None;
            return;
        }
        let Some(tr) = &self.text else { return };
        if self.preset_hud_label.is_empty() {
            return;
        }
        let Some(r) = tr.rasterize(&self.preset_hud_label, HUD_TEXT_PX, HUD_TEXT_COLOR) else {
            return;
        };
        let tw = r.width;
        let th = r.height;
        let panel_w = tw + 2 * HUD_PAD;
        let panel_h = th.max(HUD_TEXT_PX as i32) + 2 * HUD_PAD;
        // Centra el panel en el ancho de la salida (no del escritorio total),
        // en coords locales — el frame de esta salida arranca en (0,0).
        let panel_x = ((rect.w - panel_w) / 2).max(0);
        let panel_y = HUD_TOP;
        let tx = panel_x + (panel_w - tw) / 2;
        let ty = panel_y + (panel_h - th) / 2;
        let buf = MemoryRenderBuffer::from_slice(
            &r.rgba,
            Fourcc::Argb8888,
            (tw, th),
            1,
            Transform::Normal,
            None,
        );
        if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
            &mut self.renderer,
            (tx as f64, ty as f64),
            &buf,
            None,
            None,
            None,
            Kind::Unspecified,
        ) {
            into.push(Frame::Text(el));
        }
        let mut bg = SolidColorBuffer::default();
        bg.update((panel_w, panel_h), HUD_BG);
        into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
            &bg,
            (panel_x, panel_y),
            1.0,
            1.0,
            Kind::Unspecified,
        )));
    }

    /// Emite el overlay del **switcher de ventanas** (Alt-Tab): un panel
    /// centrado con la lista de ventanas, la seleccionada resaltada. Sólo
    /// mientras hay una sesión de switcher viva. Mismo text rendering que el HUD.
    fn emit_switcher(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        const SW_PX: f32 = 18.0;
        const SW_ROW_H: i32 = 36;
        const SW_PAD: i32 = 18;
        const SW_TEXT: [u8; 4] = [225, 228, 235, 255];
        const SW_TEXT_SEL: [u8; 4] = [255, 255, 255, 255];
        const SW_BG: [f32; 4] = [0.08, 0.08, 0.11, 0.94];
        const SW_SEL_BG: [f32; 4] = [0.20, 0.40, 0.85, 0.95];

        // Etiquetas (owned) ANTES de tomar el text renderer, para no chocar
        // préstamos con `&mut self.renderer` más abajo (igual que el HUD).
        let rows: Vec<(String, bool)> = match &self.app.switcher {
            Some(sw) if !sw.labels.is_empty() => sw
                .labels
                .iter()
                .enumerate()
                .map(|(i, label)| (label.clone(), i == sw.sel))
                .collect(),
            _ => return,
        };
        let Some(tr) = &self.text else { return };
        let mut rasters: Vec<(crate::text::Rasterized, bool)> = Vec::new();
        for (label, s) in &rows {
            let color = if *s { SW_TEXT_SEL } else { SW_TEXT };
            if let Some(r) = tr.rasterize(label, SW_PX, color) {
                rasters.push((r, *s));
            }
        }
        if rasters.is_empty() {
            return;
        }

        let max_tw = rasters.iter().map(|(r, _)| r.width).max().unwrap_or(0);
        let inner_w = max_tw.max(180);
        let panel_w = inner_w + 2 * SW_PAD;
        let n = rasters.len() as i32;
        let panel_h = n * SW_ROW_H + 2 * SW_PAD;
        let panel_x = ((rect.w - panel_w) / 2).max(0);
        let panel_y = ((rect.h - panel_h) / 2).max(0);

        // Textos (al frente). El orden front-to-back: primero lo de arriba.
        for (i, (r, _)) in rasters.iter().enumerate() {
            let row_y = panel_y + SW_PAD + i as i32 * SW_ROW_H;
            let tx = panel_x + SW_PAD;
            let ty = row_y + (SW_ROW_H - r.height) / 2;
            let buf = MemoryRenderBuffer::from_slice(
                &r.rgba,
                Fourcc::Argb8888,
                (r.width, r.height),
                1,
                Transform::Normal,
                None,
            );
            if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                &mut self.renderer,
                (tx as f64, ty as f64),
                &buf,
                None,
                None,
                None,
                Kind::Unspecified,
            ) {
                into.push(Frame::Text(el));
            }
        }
        // Resalte de la fila seleccionada (detrás del texto, delante del panel).
        let hl_idx = rasters.iter().position(|(_, s)| *s).unwrap_or(0) as i32;
        let hl_y = panel_y + SW_PAD + hl_idx * SW_ROW_H;
        let mut hl = SolidColorBuffer::default();
        hl.update((panel_w - 16, SW_ROW_H - 4), SW_SEL_BG);
        into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
            &hl,
            (panel_x + 8, hl_y + 2),
            1.0,
            1.0,
            Kind::Unspecified,
        )));
        // Fondo del panel (al fondo).
        let mut bg = SolidColorBuffer::default();
        bg.update((panel_w, panel_h), SW_BG);
        into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
            &bg,
            (panel_x, panel_y),
            1.0,
            1.0,
            Kind::Unspecified,
        )));
    }

    /// Renderiza un tile **rotado** de la vista espacial con su contenido VIVO:
    /// compone (fondo + ventanas reales a escala + borde + número) en una textura
    /// offscreen axis-aligned y la rota en CPU ([`crate::text::rotate_buffer`]) —
    /// la única forma de rotar superficies vivas a un ángulo libre, ya que los
    /// elementos GL sólo giran en pasos de 90°. Devuelve el búfer del AABB rotado
    /// (`Argb8888`, esquinas transparentes) listo para colocar centrado, o `None`
    /// si algún paso de GPU falla (el llamante cae al esquema de rects).
    #[allow(clippy::too_many_arguments)]
    fn render_tile_live_rotated(
        &mut self,
        tile: (i32, i32, i32, i32), // (x, y, w, h) en pantalla
        scale: f32,
        rot: f32,
        wins: &[(u64, i32, i32, i32, i32, bool)],
        border: Option<[u8; 4]>,
        badge: &crate::text::Rasterized,
        colors: ([f32; 4], [f32; 4], [f32; 4]), // (tile_bg, win_bg, win_focus)
    ) -> Option<(crate::text::Rasterized, f32)> {
        let (tx, ty, tw, th) = tile;
        if tw <= 0 || th <= 0 {
            return None;
        }
        // Resolución de composición ACOTADA. El render-vivo-rotado hace un
        // offscreen + readback + rotación CPU, O(área). Con el tile agrandado
        // durante el zoom (cerca de pantalla completa) componer a tamaño pleno
        // tironea. En vez de caer al esquema gris (lo de antes), componemos a una
        // resolución TOPE y el llamante ESCALA el bitmap por GPU hasta el tamaño
        // real: el giro vivo se ve durante todo el zoom y el costo CPU queda
        // acotado. Devolvemos `rs` (escala de composición) para que el llamante
        // escale por `1/rs`. Asentado (tile ≤ tope) → rs=1 → sin escalar, nítido.
        const LIVE_ROT_MAX: i32 = 560;
        let rs = (LIVE_ROT_MAX as f32 / tw.max(th) as f32).min(1.0);
        let (cw, ch) = (
            ((tw as f32 * rs).round() as i32).max(1),
            ((th as f32 * rs).round() as i32).max(1),
        );
        let _ = scale; // el tamaño de la miniatura ya viene en (ww2,wh2)
        let (tile_bg, win_bg, win_focus) = colors;
        use smithay::backend::renderer::gles::GlesTexture;
        use smithay::backend::renderer::utils::{import_surface_tree, with_renderer_surface_state};
        use smithay::backend::renderer::{Color32F, ImportMem, Renderer, Texture};
        use smithay::utils::{Buffer as BufCoord, Physical as Phys, Rectangle as SRect, Size as SSize};

        // 1) Extraer la GlesTexture de cada ventana (importándola antes) + su rect
        //    destino local al tile. Pasarla EXPLÍCITA a `render_texture_from_to`
        //    salta la búsqueda por context_id del render-element, que en este
        //    contexto devolvía None → ventana vacía. Esta es la apuesta.
        let ctx = self.renderer.context_id();
        let mut draws: Vec<(Option<GlesTexture>, (i32, i32, i32, i32), bool)> = Vec::new();
        for (id, wx, wy, ww2, wh2, focus) in wins {
            // Local al tile y escalado a la resolución de composición `rs`.
            let lx = (((wx - tx) as f32) * rs).round() as i32;
            let ly = (((wy - ty) as f32) * rs).round() as i32;
            let w2 = ((*ww2 as f32 * rs).round() as i32).max(1);
            let h2 = ((*wh2 as f32 * rs).round() as i32).max(1);
            let surface = self
                .app
                .windows
                .iter()
                .find(|w| w.id == *id)
                .filter(|w| crate::buffer_render_sano(&w.surface))
                .map(|w| w.surface.clone());
            let tex = surface.and_then(|s| {
                let _ = import_surface_tree(&mut self.renderer, &s);
                with_renderer_surface_state(&s, |st| st.texture(ctx.clone()).cloned()).flatten()
            });
            draws.push((tex, (lx, ly, w2, h2), *focus));
        }
        // Diag one-shot: el offscreen dibuja texturas (probado headless en metal,
        // examples/offscreen_*_diag), así que si el tile vivo rotado cae al
        // esquema, es porque la EXTRACCIÓN devolvió None. Esto lo confirma sin
        // mirar píxeles: cuántas ventanas dieron textura vs None, una sola vez.
        if !wins.is_empty() {
            use std::sync::atomic::{AtomicBool, Ordering as O};
            static DIAG: AtomicBool = AtomicBool::new(false);
            if !DIAG.swap(true, O::Relaxed) {
                let con = draws.iter().filter(|(t, ..)| t.is_some()).count();
                eprintln!(
                    "mirada-compositor · prezi vivo-rotado · extracción: {con}/{} ventanas con textura",
                    draws.len()
                );
            }
        }
        // 2) Número como textura (RGBA → Abgr8888).
        let (bw, bh) = (badge.width.max(1), badge.height.max(1));
        let badge_size: SSize<i32, BufCoord> = (bw, bh).into();
        let badge_tex = self
            .renderer
            .import_memory(&badge.rgba, Fourcc::Abgr8888, badge_size, false)
            .ok();
        let border_f = border
            .map(|bc| [bc[0] as f32 / 255.0, bc[1] as f32 / 255.0, bc[2] as f32 / 255.0, bc[3] as f32 / 255.0]);

        // 3) Componer el offscreen DIBUJANDO las texturas a mano (clear = fondo).
        //    Todo a la resolución de composición acotada `(cw, ch)`; el badge y el
        //    grosor del borde se escalan por `rs` para mantener proporción.
        let (bdx, bdy) = ((8.0 * rs).round() as i32, (6.0 * rs).round() as i32);
        let (bdw, bdh) = (
            ((bw as f32 * rs).round() as i32).max(1),
            ((bh as f32 * rs).round() as i32).max(1),
        );
        let bt_px = ((3.0 * rs).round() as i32).max(1);
        let px = crate::screencopy::render_offscreen_drawing(
            &mut self.renderer,
            (cw, ch),
            Color32F::from(tile_bg),
            |frame| {
                let dmg = [SRect::<i32, Phys>::from_size((cw, ch).into())];
                for (tex, (lx, ly, w2, h2), focus) in &draws {
                    let dst = SRect::<i32, Phys>::new((*lx, *ly).into(), (*w2, *h2).into());
                    match tex {
                        Some(t) => {
                            let src = SRect::from_size(t.size().to_f64());
                            frame.render_texture_from_to(
                                t, src, dst, &dmg, &[], Transform::Normal, 1.0, None, &[],
                            )?;
                        }
                        None => {
                            frame.draw_solid(
                                dst,
                                &dmg,
                                Color32F::from(if *focus { win_focus } else { win_bg }),
                            )?;
                        }
                    }
                }
                if let Some(bt) = &badge_tex {
                    let bdst = SRect::<i32, Phys>::new((bdx, bdy).into(), (bdw, bdh).into());
                    let bsrc = SRect::from_size(bt.size().to_f64());
                    frame.render_texture_from_to(
                        bt, bsrc, bdst, &dmg, &[], Transform::Normal, 1.0, None, &[],
                    )?;
                }
                if let Some(bf) = border_f {
                    for (x, y, w, h) in [
                        (0, 0, cw, bt_px),
                        (0, ch - bt_px, cw, bt_px),
                        (0, 0, bt_px, ch),
                        (cw - bt_px, 0, bt_px, ch),
                    ] {
                        frame.draw_solid(
                            SRect::<i32, Phys>::new((x, y).into(), (w, h).into()),
                            &dmg,
                            Color32F::from(bf),
                        )?;
                    }
                }
                Ok(())
            },
        )?;

        // 4) Rotar el bitmap (a resolución acotada) y devolverlo junto con `rs`.
        //    No hay heurístico de "variedad de color": probamos en metal (headless,
        //    examples/offscreen_*_diag) que el offscreen dibuja texturas, así que
        //    `render_offscreen_drawing` sólo devuelve None ante un fallo REAL de
        //    GPU — y ahí el llamante ya cae al esquema. Las ventanas sin buffer
        //    sano ya caen a un rect sólido dentro de la composición.
        Some((crate::text::rotate_buffer(&px, cw, ch, rot), rs))
    }

    /// Emite la **vista espacial (Prezi)** en vivo: un zoom-out con un mosaico
    /// por escritorio ocupado, arreglado según la geometría 2D, con las ventanas
    /// a escala y el activo resaltado. Pobla `overview_tiles` para el hit-test
    /// del click. Esquemática (rects + número), no miniaturas en vivo.
    fn emit_overview(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        self.overview_tiles.clear();
        if !self.app.overview_open {
            return;
        }
        let Some(data) = self.app.overview_data() else {
            self.app.overview_open = false;
            return;
        };
        // Progreso del zoom: 1 = mosaico desplegado; 0 = el escritorio activo
        // llena la pantalla (el punto de partida/llegada del vuelo de cámara).
        let t_open = match self.overview_anim {
            Some((start, opening)) => {
                let now = self.start.elapsed().as_millis() as u32;
                let anim_ms = self.app.config_overview_anim_ms().max(1) as f32;
                let raw = (now.saturating_sub(start) as f32 / anim_ms).clamp(0.0, 1.0);
                let eased = 1.0 - (1.0 - raw).powi(3); // ease-out
                if opening { eased } else { 1.0 - eased }
            }
            None => 1.0,
        };
        // Crossfade del contenido al/desde el escritorio real en el TRAMO FINAL
        // del zoom. El fondo opaco del tile (TILE_BG) tapaba el escritorio real y
        // cortaba con un «plop» gris al cerrar (y un flash al abrir). Con un fade
        // corto pegado a `t_open=0` —donde el tile activo ya está alineado con el
        // layout real— el overview DISUELVE en vez de cortar. El ramp es corto a
        // propósito: a mitad de vuelo el contenido está desalineado del real, así
        // que recién lo dejamos asomar cuando están casi superpuestos.
        let fade = (t_open / 0.12).min(1.0);
        // Scrim OPACO cuando está desplegada (esconde el escritorio real detrás);
        // se desvanece con el zoom para que al «salir» del activo no haya corte.
        const SCRIM: [f32; 4] = [0.04, 0.05, 0.07, 1.0];
        const TILE_BG: [f32; 4] = [0.12, 0.13, 0.17, 1.0];
        const WIN_BG: [f32; 4] = [0.26, 0.30, 0.40, 1.0];
        const WIN_FOCUS: [f32; 4] = [0.22, 0.45, 0.85, 1.0];
        const ACTIVE_BORDER: [f32; 4] = [0.20, 0.50, 0.95, 1.0];
        const BADGE_TX: [u8; 4] = [235, 238, 245, 255];

        // Mostramos TODOS los escritorios (mapa completo), también los vacíos —
        // así se ve el mapa entero reducido. Los vacíos salen como tiles sin
        // ventanas (sólo fondo + número) y no se resaltan (la rueda de Tab los
        // saltea, ver `overview_step`).
        let occ: Vec<usize> = (0..data.loads.len()).collect();
        // Scrim siempre (al fondo, lo empujamos último).
        let cw = rect.w as f32;
        let ch = rect.h as f32;
        if occ.is_empty() {
            let mut scrim = SolidColorBuffer::default();
            scrim.update((rect.w, rect.h), SCRIM);
            into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                &scrim,
                (0, 0),
                1.0,
                t_open,
                Kind::Unspecified,
            )));
            return;
        }

        // Extensión del plano (en unidades de celda) desde la colocación rica de
        // los ocupados — posición + tamaño libres.
        let span_cols = occ
            .iter()
            .map(|&i| data.places[i].x + data.places[i].w)
            .fold(1.0_f32, f32::max)
            .max(1.0);
        let span_rows = occ
            .iter()
            .map(|&i| data.places[i].y + data.places[i].h)
            .fold(1.0_f32, f32::max)
            .max(1.0);
        let cols = span_cols.ceil().max(1.0);
        let rows = span_rows.ceil().max(1.0);
        const MARGIN: f32 = 64.0;
        const GAP: f32 = 28.0;
        // Fracción del área disponible que ocupa el mosaico — `<1` deja aire y da
        // un zoom-out marcado (las miniaturas quedan claramente chicas, no casi a
        // pantalla completa como cuando hay pocos escritorios).
        const MOSAIC_FILL: f32 = 0.62;
        let aspect = data.work.w as f32 / data.work.h.max(1) as f32;
        let avail_w = (cw - 2.0 * MARGIN - GAP * (cols - 1.0)).max(1.0);
        let avail_h = (ch - 2.0 * MARGIN - GAP * (rows - 1.0)).max(1.0);
        let cell_w = (avail_w / cols).min(avail_h / rows * aspect) * MOSAIC_FILL;
        let cell_h = cell_w / aspect;
        let grid_w = cell_w * cols + GAP * (cols - 1.0);
        let grid_h = cell_h * rows + GAP * (rows - 1.0);
        let gx = (cw - grid_w) / 2.0;
        let gy = (ch - grid_h) / 2.0;

        // Recolectar geometría (sin pintar todavía, para ordenar capas).
        let ww = data.work.w.max(1) as f32;
        let wh = data.work.h.max(1) as f32;
        struct Tile {
            x: i32,
            y: i32,
            w: i32,
            h: i32,
            /// Giro propio del tile (rad). `0` = camino rápido de quads sólidos;
            /// `≠0` = se compone en CPU y se rota (ver más abajo).
            rot: f32,
            /// El escritorio del que SALE/ENTRA el zoom (el activo del Cerebro) —
            /// pivote de la cámara.
            active: bool,
            /// El escritorio RESALTADO (cursor de navegación Win+Tab) — su borde
            /// se marca. Coincide con `active` salvo mientras se navega con Tab.
            selected: bool,
            /// `(id, x, y, w, h, focus)` por ventana: el `id` mapea a la superficie
            /// viva; el rect es la miniatura ya posicionada.
            wins: Vec<(u64, i32, i32, i32, i32, bool)>,
            /// Factor de miniaturización (miniatura/real) para escalar la
            /// superficie viva de cada ventana al pintarla en el tile.
            scale: f32,
            num: String,
        }
        let selected_ws = self.app.overview_selected;
        let mut tiles = Vec::new();
        for &i in &occ {
            let p = data.places[i];
            // Posición: pitch (celda+gap) — la grilla por defecto queda igual.
            // Tamaño: unidades de celda. El giro `p.rot` se respeta componiendo el
            // tile en CPU y rotándolo (los `SolidColorRenderElement` no rotan).
            let tx = gx + p.x * (cell_w + GAP);
            let ty = gy + p.y * (cell_h + GAP);
            let tw = p.w * cell_w;
            let th = p.h * cell_h;
            self.overview_tiles.push((i, Rect::new(tx as i32, ty as i32, tw as i32, th as i32)));
            // Ventanas a escala dentro del tile (con un pequeño margen interno).
            const PAD: f32 = 6.0;
            let iw = tw - 2.0 * PAD;
            let ih = th - 2.0 * PAD;
            // Escala miniatura: el área de trabajo (`ww` px) entra en `iw` px.
            let scale = (iw / ww).max(0.0);
            let wins = data.layouts[i]
                .iter()
                .map(|(id, wr)| {
                    let nx = (wr.x - data.work.x) as f32 / ww;
                    let ny = (wr.y - data.work.y) as f32 / wh;
                    let nw = (wr.w as f32 / ww).clamp(0.0, 1.0);
                    let nh = (wr.h as f32 / wh).clamp(0.0, 1.0);
                    (
                        *id,
                        (tx + PAD + nx * iw) as i32,
                        (ty + PAD + ny * ih) as i32,
                        (nw * iw).max(2.0) as i32,
                        (nh * ih).max(2.0) as i32,
                        false,
                    )
                })
                .collect();
            tiles.push(Tile {
                x: tx as i32,
                y: ty as i32,
                w: tw as i32,
                h: th as i32,
                rot: p.rot,
                active: i == data.active,
                selected: i == selected_ws,
                wins,
                scale,
                num: format!("{}", i + 1),
            });
        }

        // Rasterizar los números ANTES de tomar el renderer (como el HUD).
        let badges: Vec<crate::text::Rasterized> = {
            let Some(tr) = &self.text else {
                return;
            };
            tiles
                .iter()
                .filter_map(|t| tr.rasterize(&t.num, 20.0, BADGE_TX))
                .collect()
        };

        // ── Vuelo de cámara (zoom Prezi) ──────────────────────────────────────
        // A `t_open=1` se ve el mosaico tal cual; a `t_open=0` el escritorio
        // ACTIVO llena la pantalla. Interpolamos una escala + traslación global
        // (pivote = centro del tile activo) y la aplicamos a cada tile y a las
        // posiciones/escala de sus ventanas. Así el Win+Tab «sale» del activo y
        // hace zoom-out al mosaico, y al cerrar hace zoom-in de vuelta.
        if t_open < 0.999 {
            let (acx, acy, s0) = match tiles.iter().find(|t| t.active) {
                Some(a) => {
                    let aw = a.w.max(1) as f32;
                    let ah = a.h.max(1) as f32;
                    (a.x as f32 + aw / 2.0, a.y as f32 + ah / 2.0, (cw / aw).max(ch / ah))
                }
                // Activo vacío (sin tile): zoom desde el centro, sin ampliar.
                None => (cw / 2.0, ch / 2.0, 1.0),
            };
            let s = 1.0 + (s0 - 1.0) * (1.0 - t_open);
            let ox = cw / 2.0 + (acx - cw / 2.0) * t_open;
            let oy = ch / 2.0 + (acy - ch / 2.0) * t_open;
            let cam = |x: f32, y: f32| ((x - acx) * s + ox, (y - acy) * s + oy);
            for tl in &mut tiles {
                let (nx, ny) = cam(tl.x as f32, tl.y as f32);
                tl.x = nx.round() as i32;
                tl.y = ny.round() as i32;
                tl.w = (tl.w as f32 * s).round() as i32;
                tl.h = (tl.h as f32 * s).round() as i32;
                tl.scale *= s;
                // El GIRO también sigue la curva del zoom: a `t_open=0` (escritorio
                // activo a pantalla completa) el tile está DERECHO; a `t_open=1`
                // (mosaico) toma su ángulo pleno. Sin esto aparecía rotado de golpe
                // al abrir y se quedaba en diagonal al cerrar. Lineal en la `t_open`
                // ya eased → se voltea «según la curva», y des-rota al cerrar.
                tl.rot *= t_open;
                for win in &mut tl.wins {
                    let (wx, wy) = cam(win.1 as f32, win.2 as f32);
                    win.1 = wx.round() as i32;
                    win.2 = wy.round() as i32;
                    win.3 = (win.3 as f32 * s).round() as i32;
                    win.4 = (win.4 as f32 * s).round() as i32;
                }
            }
        }

        // Umbral para tratar un tile como "girado" (evita el costo CPU por ruido).
        let girado = |rot: f32| rot.abs() > 1e-4;
        // [f32;4] (R,G,B,A 0..1) → [u8;4] R,G,B,A para el rasterizador CPU.
        let to_u8 =
            |c: [f32; 4]| [(c[0] * 255.0) as u8, (c[1] * 255.0) as u8, (c[2] * 255.0) as u8, (c[3] * 255.0) as u8];

        // CAPAS front→back: tiles girados (compuestos+rotados en CPU), números,
        // ventanas, borde activo, fondo de tile, scrim. Los tiles girados son
        // auto-contenidos (fondo+ventanas+borde+número ya horneados) y van al
        // frente; los rectos siguen el camino rápido de quads sólidos.
        for (t, badge) in tiles.iter().zip(badges.iter()) {
            if !girado(t.rot) {
                continue;
            }
            let border = t.selected.then(|| to_u8(ACTIVE_BORDER));
            // CAMINO VIVO: renderizamos el tile (fondo + ventanas REALES a escala +
            // borde + número) a una textura offscreen, lo leemos y lo ROTAMOS en
            // CPU. Así el contenido vivo se ve girado de verdad. Si algún paso de
            // GPU falla, caemos al esquema (rects) —rotado igual— para no quedar
            // sin nada.
            // `comp` viene compuesto a resolución acotada `rs` (≤1): durante el
            // zoom `rs<1` y el bitmap se escala por GPU (`1/rs`); asentado `rs=1`.
            let (comp, rs) = self
                .render_tile_live_rotated(
                    (t.x, t.y, t.w, t.h),
                    t.scale,
                    t.rot,
                    &t.wins,
                    border,
                    badge,
                    (TILE_BG, WIN_BG, WIN_FOCUS),
                )
                .unwrap_or_else(|| {
                    let wins_local: Vec<(i32, i32, i32, i32, bool)> = t
                        .wins
                        .iter()
                        .map(|(_id, wx, wy, ww2, wh2, f)| (wx - t.x, wy - t.y, *ww2, *wh2, *f))
                        .collect();
                    (
                        crate::text::rasterize_tile_rotated(
                            t.w,
                            t.h,
                            t.rot,
                            to_u8(TILE_BG),
                            border,
                            &wins_local,
                            to_u8(WIN_BG),
                            to_u8(WIN_FOCUS),
                            Some(badge),
                        ),
                        1.0,
                    )
                });
            let buf = MemoryRenderBuffer::from_slice(
                &comp.rgba,
                Fourcc::Argb8888,
                (comp.width, comp.height),
                1,
                Transform::Normal,
                None,
            );
            // Coloca el AABB (escalado por `1/rs`) centrado en el centro del tile.
            let k = (1.0 / rs as f64).max(1.0);
            let cx = t.x as f64 + t.w as f64 / 2.0;
            let cy = t.y as f64 + t.h as f64 / 2.0;
            let ax = cx - k * comp.width as f64 / 2.0;
            let ay = cy - k * comp.height as f64 / 2.0;
            if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                &mut self.renderer,
                (ax, ay),
                &buf,
                Some(fade),
                None,
                None,
                Kind::Unspecified,
            ) {
                if (k - 1.0).abs() < 1e-3 {
                    into.push(Frame::Text(el));
                } else {
                    // Escala el bitmap (GPU) alrededor de su esquina = posición del
                    // elemento, así el centro del AABB cae en el centro del tile.
                    let origin = Point::<i32, Physical>::from((ax.round() as i32, ay.round() as i32));
                    into.push(Frame::ScaledText(RescaleRenderElement::from_element(el, origin, k)));
                }
            }
        }
        for (t, badge) in tiles.iter().zip(badges.iter()) {
            if girado(t.rot) {
                continue; // el número va horneado en el tile rotado
            }
            let buf = MemoryRenderBuffer::from_slice(
                &badge.rgba,
                Fourcc::Argb8888,
                (badge.width, badge.height),
                1,
                Transform::Normal,
                None,
            );
            if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                &mut self.renderer,
                ((t.x + 8) as f64, (t.y + 6) as f64),
                &buf,
                Some(fade),
                None,
                None,
                Kind::Unspecified,
            ) {
                into.push(Frame::Text(el));
            }
        }
        for t in tiles.iter().filter(|t| !girado(t.rot)) {
            for (id, wx, wy, ww2, wh2, focus) in &t.wins {
                // Miniatura VIVA: pintamos la superficie real de la ventana a
                // escala `t.scale` en su lugar del tile. Si la ventana no tiene
                // buffer sano (recién abierta, sin presentar), caemos al rect
                // sólido — así el overview nunca queda "vacío" ni revienta.
                let surface = self
                    .app
                    .windows
                    .iter()
                    .find(|w| w.id == *id)
                    .filter(|w| crate::buffer_render_sano(&w.surface))
                    .map(|w| w.surface.clone());
                if let Some(surface) = surface {
                    // La pintamos a TAMAÑO COMPLETO anclada en `(wx,wy)` y luego la
                    // reescalamos por `t.scale` alrededor de ese mismo punto: el
                    // `scale` de `render_elements_from_surface_tree` es la escala de
                    // SALIDA (no achica la ventana), así que el encogido real lo hace
                    // el `RescaleRenderElement`. Sin esto las ventanas salían a
                    // pantalla completa apiladas y «no se veía reducido».
                    let elems = render_elements_from_surface_tree(
                        &mut self.renderer,
                        &surface,
                        (*wx, *wy),
                        1.0,
                        fade,
                        Kind::Unspecified,
                    );
                    if !elems.is_empty() {
                        let origin = Point::<i32, Physical>::from((*wx, *wy));
                        for el in elems {
                            into.push(Frame::ScaledWindow(RescaleRenderElement::from_element(
                                el,
                                origin,
                                t.scale as f64,
                            )));
                        }
                        continue;
                    }
                }
                let mut wb = SolidColorBuffer::default();
                wb.update((*ww2, *wh2), if *focus { WIN_FOCUS } else { WIN_BG });
                into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                    &wb,
                    (*wx, *wy),
                    1.0,
                    fade,
                    Kind::Unspecified,
                )));
            }
        }
        for t in tiles.iter().filter(|t| t.selected && !girado(t.rot)) {
            // Borde activo: un rect un poco más grande detrás del tile.
            let mut br = SolidColorBuffer::default();
            br.update((t.w + 6, t.h + 6), ACTIVE_BORDER);
            into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                &br,
                (t.x - 3, t.y - 3),
                1.0,
                fade,
                Kind::Unspecified,
            )));
        }
        for t in tiles.iter().filter(|t| !girado(t.rot)) {
            let mut tb = SolidColorBuffer::default();
            tb.update((t.w, t.h), TILE_BG);
            into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                &tb,
                (t.x, t.y),
                1.0,
                fade,
                Kind::Unspecified,
            )));
        }
        let mut scrim = SolidColorBuffer::default();
        scrim.update((rect.w, rect.h), SCRIM);
        into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
            &scrim,
            (0, 0),
            1.0,
            t_open,
            Kind::Unspecified,
        )));
    }

    /// Emite el overlay de zonas de arrastre (drag-to-zone) — visible sólo
    /// durante un arrastre Move/Tile. Las zonas se escalan al monitor bajo
    /// el puntero y se emiten traducidas a coords locales de `rect`. Si las
    /// zonas no caen sobre `rect` (drag en otro monitor), no emite nada.
    fn emit_zone_overlay(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        let drag_mode = self.app.drag.as_ref().map(|d| d.mode);
        if !matches!(drag_mode, Some(DragMode::Move) | Some(DragMode::Tile)) {
            return;
        }
        // Rect destino del snap (global). Sin snap (puntero en el centro) no se
        // pinta nada — la ventana cae libre.
        let Some(r) = self.drag_zone else { return };
        // Sólo en la salida que contiene la zona (drag en otro monitor → nada acá).
        if r.x + r.w <= rect.x
            || r.y + r.h <= rect.y
            || r.x >= rect.x + rect.w
            || r.y >= rect.y + rect.h
        {
            return;
        }
        let acc = self.app.decorations.border_focus;
        let fill = |a: f32| {
            [
                acc[0] as f32 / 255.0,
                acc[1] as f32 / 255.0,
                acc[2] as f32 / 255.0,
                a,
            ]
        };
        // Estilo KDE: pintamos SÓLO la zona objetivo como previsualización
        // prominente — relleno translúcido + un borde sólido. Sin objetivo no se
        // pinta nada (ya se filtró arriba).
        let (lx, ly) = (r.x - rect.x, r.y - rect.y);
        // Orden front-to-back: lo que se empuja PRIMERO queda encima. Empujamos
        // el borde antes que el relleno para que el contorno quede prominente.
        let bw = 4;
        let bcol = fill(0.85);
        let mut push_band = |x: i32, y: i32, w: i32, h: i32, into: &mut Vec<Frame<GlesRenderer>>| {
            if w <= 0 || h <= 0 {
                return;
            }
            let mut b = SolidColorBuffer::default();
            b.update((w, h), bcol);
            into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                &b, (x, y), 1.0, 1.0, Kind::Unspecified,
            )));
        };
        // Borde sólido grueso (4 lados) — el contorno claro de la previsualización.
        push_band(lx, ly, r.w, bw, into); // arriba
        push_band(lx, ly + r.h - bw, r.w, bw, into); // abajo
        push_band(lx, ly, bw, r.h, into); // izquierda
        push_band(lx + r.w - bw, ly, bw, r.h, into); // derecha
        // Relleno translúcido del acento (debajo del borde) — bien transparente,
        // sólo un tinte; el borde es lo que marca la zona.
        let mut buf = SolidColorBuffer::default();
        buf.update((r.w, r.h), fill(0.09));
        into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
            &buf, (lx, ly), 1.0, 1.0, Kind::Unspecified,
        )));
    }

    /// Emite el menú raíz en `rect` si esta salida es la dueña del menú.
    /// El menú vive en **coords locales** de su salida (se abrió ahí), así
    /// que las posiciones de las columnas no necesitan traducción.
    fn emit_menu(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        let Some(m) = self.root_menu.as_ref() else { return };
        // El menú se rasteriza con el puntero **local** a su salida — así
        // resaltado y hover apuntan a la fila correcta.
        let (px, py) = self.app.pointer_loc;
        let cols = m.render(px.round() as i32 - rect.x, py.round() as i32 - rect.y);
        let menu_hl_color = rgba_f32(self.app.decorations.border_focus);
        if self.text_cache.len() > 256 {
            self.text_cache.clear();
        }
        for col in cols.iter().rev() {
            // Texto (caché).
            if let Some(tr) = &self.text {
                for row in &col.rows {
                    let label = if row.submenu {
                        format!("{}   ›", row.label)
                    } else {
                        row.label.clone()
                    };
                    let buf = self
                        .text_cache
                        .entry((label.clone(), MENU_TEXT_COLOR))
                        .or_insert_with(|| {
                            match tr.rasterize(&label, MENU_TEXT_PX, MENU_TEXT_COLOR) {
                                Some(r) => MemoryRenderBuffer::from_slice(
                                    &r.rgba,
                                    Fourcc::Argb8888,
                                    (r.width, r.height),
                                    1,
                                    Transform::Normal,
                                    None,
                                ),
                                None => MemoryRenderBuffer::from_slice(
                                    &[0u8; 4],
                                    Fourcc::Argb8888,
                                    (1, 1),
                                    1,
                                    Transform::Normal,
                                    None,
                                ),
                            }
                        });
                    let ty = row.y + (crate::menu::ITEM_H - MENU_TEXT_PX as i32) / 2;
                    if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                        &mut self.renderer,
                        ((row.x + 10) as f64, ty as f64),
                        buf,
                        None,
                        None,
                        None,
                        Kind::Unspecified,
                    ) {
                        into.push(Frame::Text(el));
                    }
                }
            }
            // Resaltado.
            for row in &col.rows {
                if row.highlighted {
                    let mut hl = SolidColorBuffer::default();
                    hl.update((col.w, crate::menu::ITEM_H), menu_hl_color);
                    into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                        &hl,
                        (row.x, row.y),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    )));
                }
            }
            // Fondo.
            let mut bg = SolidColorBuffer::default();
            bg.update((col.w, col.h), MENU_BG);
            into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                &bg,
                (col.x, col.y),
                1.0,
                1.0,
                Kind::Unspecified,
            )));
        }
    }

    /// Emite la pista de revelado del dock autoescondido — una franja fina en
    /// el borde anclado mientras está oculto. Sólo en la salida donde vive
    /// el shell (primaria).
    fn emit_reveal_band(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        if !(crate::shell_dock().autohide && self.app.shell_hidden) {
            return;
        }
        let (ow, oh) = (rect.w, rect.h);
        if ow <= 0 || oh <= 0 {
            return;
        }
        let dock = crate::shell_dock();
        let limite = if dock.anchor.es_horizontal() { oh } else { ow };
        let t = dock.thickness.clamp(1, limite.max(1));
        let (bx, by, bw, bh) =
            crate::shell_reveal_band(dock.anchor, ow, oh, t, crate::SHELL_REVEAL_BAND);
        let menu_hl_color = rgba_f32(self.app.decorations.border_focus);
        let mut band = SolidColorBuffer::default();
        band.update((bw, bh), menu_hl_color);
        into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
            &band,
            (bx, by),
            1.0,
            1.0,
            Kind::Unspecified,
        )));
    }

    /// Gestiona los workers del wallpaper en **video** — uno **por salida**. Por
    /// cada salida: arranca/reemplaza/suelta su worker según su config (la fuente
    /// es global pero la **ruta se resuelve por salida**, así cada monitor puede
    /// correr su propio archivo), lo pausa cuando no se ve (otra VT, DPMS, o una
    /// ventana a pantalla completa **sobre esa salida**), y consume su último
    /// frame a `ctx.video_frame`. Corre en `tick` (antes de `render`) para poder
    /// pausar aun con la sesión en otra VT. Con fuente no-video suelta el worker.
    pub(super) fn manage_video_wallpaper(&mut self) {
        let inactive = !self.active || self.dpms_off;
        for i in 0..self.outputs.len() {
            let name = self.outputs[i].name.clone();
            match self.app.config_video_wallpaper_for(&name) {
                Some((path, fps)) => {
                    let need_restart = self.outputs[i]
                        .video_wp
                        .as_ref()
                        .map_or(true, |vw| !vw.matches(&path, fps));
                    if need_restart {
                        self.outputs[i].video_wp =
                            Some(super::video_wallpaper::VideoWallpaper::start(&path, fps));
                        self.outputs[i].video_frame = None;
                    }
                    // ¿Tapado por una ventana a pantalla completa SOBRE esta salida?
                    let rect = self.outputs[i].rect;
                    let covered = self.app.windows.iter().any(|w| {
                        w.fullscreen && w.visible && w.loc.0 >= rect.x && w.loc.0 < rect.x + rect.w
                    });
                    let paused = inactive || covered;
                    if let Some(vw) = self.outputs[i].video_wp.as_ref() {
                        vw.set_paused(paused);
                    }
                    if !paused {
                        let frame =
                            self.outputs[i].video_wp.as_mut().and_then(|vw| vw.take_new_frame());
                        if let Some(frame) = frame {
                            self.outputs[i].video_frame = Some(frame);
                            self.outputs[i].video_dirty = true;
                            crate::screencopy::danar_todo(&mut self.app);
                        }
                    }
                }
                None => {
                    // Fuente no-video en esta salida: soltá su worker (su `Drop`
                    // para el hilo) e invalidá su buffer para que la nueva fuente
                    // se pinte.
                    if self.outputs[i].video_wp.take().is_some() {
                        self.outputs[i].video_frame = None;
                        self.outputs[i].wallpaper = None;
                    }
                }
            }
        }
    }

    /// Late del **wallpaper de marca animado**: cuando el fondo por defecto es el
    /// vivo y se ve (sesión activa, pantalla encendida, no tapado), marca daño a
    /// ~20 fps para que `emit_wallpaper` regenere el frame. Pausado en cualquier
    /// otro caso → costo cero. Corre en `tick`, antes de `render`.
    pub(super) fn tick_animated_default(&mut self) {
        if !self.active || self.dpms_off || !self.app.config_animated_default() {
            return;
        }
        if self.app.windows.iter().any(|w| w.fullscreen && w.visible) {
            return; // tapado por una ventana a pantalla completa.
        }
        let now = self.start.elapsed().as_millis() as u32;
        if self.anim_default_ms != 0 && now.saturating_sub(self.anim_default_ms) < 50 {
            return; // ~20 fps
        }
        self.anim_default_ms = now;
        self.anim_default_dirty = true;
        crate::screencopy::danar_todo(&mut self.app);
    }

    fn emit_wallpaper(&mut self, idx: usize, into: &mut Vec<Frame<GlesRenderer>>) {
        let name = self.outputs[idx].name.clone();
        let cur_path = self.outputs[idx].wallpaper_path.clone();
        let size = (self.outputs[idx].rect.w, self.outputs[idx].rect.h);
        if size.0 <= 0 || size.1 <= 0 {
            return;
        }
        let stale = self.outputs[idx]
            .wallpaper
            .as_ref()
            .map(|(_, s)| *s != size)
            .unwrap_or(true);
        use crate::estado::WallpaperSpec;
        let spec = self.app.config_wallpaper_spec_for(&name, cur_path.as_deref());
        if let WallpaperSpec::Video(_) = spec {
            // Fondo en VIDEO: subimos el último frame a **tamaño nativo** (sólo
            // swizzle, sin resize) cuando llegó uno nuevo (`video_dirty`); la GPU
            // lo escala (cover) al pintar. Si aún no hay frame (worker calentando)
            // dejamos un fondo de marca a tamaño de salida para no parpadear.
            let no_buf = self.outputs[idx].wallpaper.is_none();
            if self.outputs[idx].video_dirty || no_buf {
                let native = self.outputs[idx].video_frame.as_ref().and_then(|(rgba, fw, fh)| {
                    rgba_native_membuffer(rgba, *fw, *fh).map(|b| (b, (*fw as i32, *fh as i32)))
                });
                if let Some((buf, src)) = native {
                    self.outputs[idx].wallpaper = Some((buf, src));
                } else if no_buf {
                    let buf = make_marca_wallpaper(mirada_brain::WallpaperFit::Fill, size.0, size.1)
                        .or_else(|| Some(make_default_wallpaper(size.0, size.1)));
                    self.outputs[idx].wallpaper = buf.map(|b| (b, size));
                }
                self.outputs[idx].video_dirty = false;
            }
        } else if matches!(spec, WallpaperSpec::Default) && self.app.config_animated_default() {
            // Fondo por defecto VIVO: la chakana + plano cartesiano de marca,
            // generado a tamaño **acotado** (~720p) y escalado por GPU al pintar —
            // así no cuesta resolución de salida (importa en 4K). Regenerado
            // estrangulado a ~20 fps (`tick_animated_default` → `anim_default_dirty`).
            if self.anim_default_dirty || self.outputs[idx].wallpaper.is_none() {
                let (iw, ih) = capped_anim_size(size.0, size.1);
                let t = self.start.elapsed().as_secs_f32();
                let bytes = marca::animated_frame(t, iw, ih);
                if let Some(buf) = bgra_membuffer(&bytes, iw as i32, ih as i32) {
                    self.outputs[idx].wallpaper = Some((buf, (iw as i32, ih as i32)));
                }
            }
        } else if stale {
            // Despacho por la FUENTE elegida (color/gradiente/procedural/imagen).
            let buf = match spec {
                WallpaperSpec::Image(p, fit) => load_wallpaper(&p, fit, size.0, size.1),
                WallpaperSpec::Solid(c) => Some(make_solid_wallpaper(c, size.0, size.1)),
                WallpaperSpec::Gradient(stops) => {
                    Some(make_gradient_wallpaper(&stops, size.0, size.1))
                }
                WallpaperSpec::Procedural(pat, pal) => {
                    Some(make_procedural_wallpaper(pat, &pal, size.0, size.1))
                }
                // Inalcanzable: la rama de arriba ya cubrió el video.
                WallpaperSpec::Video(_) => None,
                // Sin imagen configurada: el wallpaper de marca (chakana + 4
                // cuadrantes) es el fondo por defecto; si no decodifica, gradiente.
                WallpaperSpec::Default => {
                    make_marca_wallpaper(mirada_brain::WallpaperFit::Fill, size.0, size.1)
                        .or_else(|| Some(make_default_wallpaper(size.0, size.1)))
                }
            };
            self.outputs[idx].wallpaper = buf.map(|b| (b, size));
        }
        let ctx = &self.outputs[idx];
        let Some((buf, (sw, sh))) = &ctx.wallpaper else {
            return;
        };
        let (sw, sh) = (*sw, *sh);
        if sw == size.0 && sh == size.1 {
            // Buffer ya a tamaño de salida (fuentes estáticas) → 1:1, sin escalar.
            if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                &mut self.renderer,
                (0.0, 0.0),
                buf,
                None,
                None,
                None,
                Kind::Unspecified,
            ) {
                into.push(Frame::Text(el));
            }
        } else {
            // Buffer a otra resolución (video nativo / marca acotada) → la GPU lo
            // escala **cover** alrededor del origen calculado.
            let (ox, oy, k) = cover_transform(sw, sh, size.0, size.1);
            if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                &mut self.renderer,
                (ox, oy),
                buf,
                None,
                None,
                None,
                Kind::Unspecified,
            ) {
                let origin = Point::<i32, Physical>::from((ox.round() as i32, oy.round() as i32));
                into.push(Frame::ScaledText(RescaleRenderElement::from_element(el, origin, k)));
            }
        }
    }

    /// Refresca los búferes de marco (color por foco) de todas las ventanas
    /// visibles. Es estado global que no depende de cuál salida se está
    /// rindiendo — se hace una vez al inicio de [`Self::render`].
    fn refresh_window_borders(&mut self) {
        let dec = self.app.decorations;
        if self.outputs.get(Self::PRIMARY).is_none() {
            return; // sin monitores: no hay bordes que recalcular
        }
        // Glow de foco: el color del marco crossfadea sin-foco↔con-foco. En
        // reposo (`glow_ms=0` o sin transición viva) da el color seco de antes.
        let glow_ms = self.app.config_focus_glow_ms();
        let now = self.start.elapsed().as_millis() as u32;
        for w in &mut self.app.windows {
            if !w.visible || w.is_shell {
                continue;
            }
            let tb = crate::titlebar_for(w, dec.titlebar_height);
            // Sólo importan los TAMAÑOS de los 4 lados (la posición se calcula al
            // pintar). Se dimensionan sobre el CONTENIDO (sin sombra CSD), igual
            // que el marco que se dibuja en el loop principal.
            let (cw, ch) = crate::content_px_size(w).unwrap_or((w.size.0, w.size.1 - tb));
            let color = rgba_f32(lerp_rgba(
                dec.border_normal,
                dec.border_focus,
                focus_mix(w.focused, glow_ms, w.focus_ms, now),
            ));
            let rects = border_rects(0, 0, cw, ch + tb, dec.border_width);
            for (buf, (_, _, bw, bh)) in w.borders.iter_mut().zip(rects) {
                buf.update((bw, bh), color);
            }
        }
    }

    /// Avisa a cada cliente (ventanas, layers de la primaria, cursor) de
    /// que puede dibujar el siguiente cuadro. Se llama una sola vez por
    /// `render`, no por salida.
    fn send_frames_to_clients(&mut self) {
        let time = self.start.elapsed().as_millis() as u32;
        // FUS: las ventanas de una sesión no activa tampoco reciben frames —
        // quedan inertes detrás, como las capas dormidas.
        let activa = self.app.roster.active_id();
        for w in &mut self.app.windows {
            w.frame_tick = w.frame_tick.wrapping_add(1);
            // Las capas dormidas (zoom-Z) no reciben frame callbacks.
            if w.suspended {
                continue;
            }
            // Sesión no activa bajo FUS: sin frames (igual que `suspended`).
            if let Some(a) = activa {
                if !w.is_shell && !w.is_greeter && w.session != a {
                    continue;
                }
            }
            // Throttle de fondo: 1 de cada `frame_divisor` vblanks.
            let div = w.frame_divisor.max(1);
            if div > 1 && w.frame_tick % div != 0 {
                continue;
            }
            send_frames_surface_tree(&w.surface, time);
            // Sus popups (menús) también, si no el resaltado del menú se congela.
            crate::send_frames_popups(&w.surface, time);
        }
        // Layers de TODAS las salidas — un cliente puede tener barras en
        // distintos monitores, cada una con su frame-callback propio.
        for output in self.app.outputs.clone() {
            for layer in smithay::desktop::layer_map_for_output(&output).layers() {
                send_frames_surface_tree(layer.wl_surface(), time);
            }
        }
        if let CursorImageStatus::Surface(surface) = &self.app.cursor_status {
            if surface.alive() {
                send_frames_surface_tree(surface, time);
            }
        }
    }

    /// Render unificado de una salida. Cada feature decide si pertenece o
    /// no a esta salida (gates por dueño) — el cursor en la del puntero,
    /// HUD/layer-shell/reveal-band en primaria, menú y zonas en la salida
    /// donde se inició la acción, ventanas y wallpaper en todas.
    fn render_output(&mut self, idx: usize) {
        if self.outputs[idx].pending_flip {
            return;
        }
        let rect = self.outputs[idx].rect;
        let is_primary = idx == Self::PRIMARY;
        let owns_menu = self.menu_output_idx == Some(idx);

        let elements: Vec<Frame<GlesRenderer>> = {
            let mut out: Vec<Frame<GlesRenderer>> = Vec::new();

            // 1. Cursor (si el puntero cae sobre esta salida).
            self.emit_cursor(rect, &mut out);

            // 2. HUD del preset + switcher de ventanas + vista espacial (Prezi),
            //    todos en la primaria, centrados.
            if is_primary {
                self.emit_hud(rect, &mut out);
                self.emit_switcher(rect, &mut out);
                self.emit_overview(rect, &mut out);
            }

            // 3. Zonas de arrastre — en la salida bajo el puntero durante
            //    un drag (helper filtra por intersección de work-rect).
            self.emit_zone_overlay(rect, &mut out);

            // 4. Menú raíz — sólo en la salida donde se abrió.
            if owns_menu {
                self.emit_menu(rect, &mut out);
            }

            // 5. Pista de revelado del dock autoescondido — primaria, el
            //    shell vive ahí.
            if is_primary {
                self.emit_reveal_band(rect, &mut out);
            }

            // 6. Layer surfaces (waybar, swaybg…) de **esta** salida —
            //    smithay las cuelga por output, así que un layer mapeado a
            //    una secundaria con `output_hint` aparece donde toca.
            let output_for_layers = self.outputs[idx].output.clone();
            let (over_layers, under_layers) =
                crate::layer_render_elements(Some(&output_for_layers), &mut self.renderer);
            for el in over_layers {
                out.push(Frame::Window(el));
            }
            // 7. Ventanas entre layers Overlay/Top y Bottom/Background.
            self.emit_windows(rect, &mut out);
            // 7.bis Fantasmas de cierre (fade-out), a la altura de las ventanas.
            self.emit_ghosts(rect, &mut out);
            for el in under_layers {
                out.push(Frame::Window(el));
            }

            // 8. Wallpaper al fondo (por salida).
            self.emit_wallpaper(idx, &mut out);

            out
        };

        let ctx = &mut self.outputs[idx];
        match ctx.compositor.render_frame::<_, _>(
            &mut self.renderer,
            &elements,
            CLEAR_COLOR,
            FrameFlags::DEFAULT,
        ) {
            Ok(result) => {
                if !result.is_empty {
                    match ctx.compositor.queue_frame(()) {
                        Ok(()) => {
                            ctx.pending_flip = true;
                            let name = ctx.name.clone();
                            PRIMER_FRAME.call_once(|| {
                                let ms = std::time::SystemTime::now()
                                    .duration_since(std::time::UNIX_EPOCH)
                                    .map(|d| d.as_millis())
                                    .unwrap_or(0);
                                eprintln!(
                                    "[handoff] primer queue_frame presentado · epoch_ms={ms} salida={name}"
                                );
                            });
                        }
                        Err(e) => eprintln!(
                            "mirada-compositor · queue_frame[{}]: {e}",
                            ctx.name
                        ),
                    }
                }
            }
            Err(e) => eprintln!(
                "mirada-compositor · render_frame[{}]: {e}",
                ctx.name
            ),
        }

        // Capturas screencopy pendientes de esta salida: el framebuffer real
        // vive dentro del DrmCompositor, así que se re-componen los mismos
        // elementos en un offscreen y se copia de ahí.
        if !self.app.pending_screencopy.is_empty() {
            let output = self.outputs[idx].output.clone();
            let capturas =
                crate::screencopy::tomar_capturas(&mut self.app, &output, (rect.x, rect.y));
            if !capturas.is_empty() {
                crate::screencopy::servir_offscreen(
                    &mut self.renderer,
                    (rect.w, rect.h),
                    &elements,
                    CLEAR_COLOR.into(),
                    capturas,
                );
            }
        }
    }
}

/// Dibuja recursivamente los `xdg_popup` colgados de `parent` (los menús de las
/// apps). `base` es el origen de geometría (contenido) del parent en coords
/// GLOBALES; `rect` es la salida que se está rindiendo (para pasar a locales).
/// Cada popup va en `base + posición_relativa - su_offset_de_geometría`; sus
/// hijos (submenús) cuelgan de su propio origen de geometría. Es función libre
/// (no método) para tomar `&mut renderer` sin chocar con el préstamo de `windows`.
fn emit_popups(
    renderer: &mut GlesRenderer,
    parent: &smithay::reexports::wayland_server::protocol::wl_surface::WlSurface,
    base: (i32, i32),
    rect: Rect,
    into: &mut Vec<Frame<GlesRenderer>>,
) {
    for (popup, ploc) in smithay::desktop::PopupManager::popups_for_surface(parent) {
        let psurf = popup.wl_surface().clone();
        let pgeo = popup.geometry();
        let geo_origin = (base.0 + ploc.x, base.1 + ploc.y);
        let draw_x = geo_origin.0 - pgeo.loc.x - rect.x;
        let draw_y = geo_origin.1 - pgeo.loc.y - rect.y;
        for el in render_elements_from_surface_tree(
            renderer,
            &psurf,
            (draw_x, draw_y),
            1.0,
            1.0,
            Kind::Unspecified,
        ) {
            into.push(Frame::Window(el));
        }
        emit_popups(renderer, &psurf, geo_origin, rect, into);
    }
}

#[cfg(test)]
mod tests {
    use super::{cover_transform, focus_mix, lerp_rgba};

    #[test]
    fn cover_transform_llena_y_centra() {
        // Mismo aspecto: escala exacta, sin offset.
        let (ox, oy, k) = cover_transform(1280, 720, 2560, 1440);
        assert!((k - 2.0).abs() < 1e-9);
        assert!(ox.abs() < 1e-9 && oy.abs() < 1e-9);
        // Fuente más "cuadrada" que la salida (16:9): cover por el ancho, recorta
        // arriba/abajo → offset_y negativo, offset_x 0.
        let (ox, oy, k) = cover_transform(1000, 1000, 1920, 1080);
        assert!((k - 1.92).abs() < 1e-6, "k={k}");
        assert!(ox.abs() < 1e-6, "ox={ox}");
        assert!(oy < 0.0, "oy={oy} debe recortar verticalmente");
        // El recorte es simétrico: el centro del buffer cae en el centro de salida.
        assert!((oy + 1000.0 * k / 2.0 - 540.0).abs() < 1e-6);
    }

    #[test]
    fn focus_mix_extremos_secos_sin_transicion() {
        // Sin glow (glow_ms=0): salto seco según el foco.
        assert_eq!(focus_mix(true, 0, Some(100), 200), 1.0);
        assert_eq!(focus_mix(false, 0, Some(100), 200), 0.0);
        // Sin transición estampada: también seco.
        assert_eq!(focus_mix(true, 140, None, 200), 1.0);
        assert_eq!(focus_mix(false, 140, None, 200), 0.0);
    }

    #[test]
    fn focus_mix_rampa_en_ambos_sentidos() {
        let glow = 100;
        // Ganando foco: arranca en 0, termina en 1, monótona creciente.
        assert!(focus_mix(true, glow, Some(0), 0).abs() < 1e-6);
        assert!((focus_mix(true, glow, Some(0), 100) - 1.0).abs() < 1e-6);
        assert!(focus_mix(true, glow, Some(0), 50) > 0.0);
        assert!(focus_mix(true, glow, Some(0), 50) < 1.0);
        // Pasado el final se mantiene en 1 (clamp).
        assert!((focus_mix(true, glow, Some(0), 9999) - 1.0).abs() < 1e-6);
        // Perdiendo foco: arranca en 1 y baja a 0.
        assert!((focus_mix(false, glow, Some(0), 0) - 1.0).abs() < 1e-6);
        assert!(focus_mix(false, glow, Some(0), 100).abs() < 1e-6);
    }

    #[test]
    fn lerp_rgba_interpola_por_canal() {
        let a = [0, 0, 0, 0];
        let b = [255, 100, 50, 200];
        assert_eq!(lerp_rgba(a, b, 0.0), a);
        assert_eq!(lerp_rgba(a, b, 1.0), b);
        // Punto medio (redondeo al más cercano).
        assert_eq!(lerp_rgba(a, b, 0.5), [128, 50, 25, 100]);
        // Fuera de rango se recorta.
        assert_eq!(lerp_rgba(a, b, 2.0), b);
        assert_eq!(lerp_rgba(a, b, -1.0), a);
    }
}
