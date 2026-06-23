use super::*;

impl DrmState {
    /// Compone un cuadro por cada salida y avisa a los clientes una sola vez.
    /// Si una salida tiene su `pending_flip` puesto, se saltea hasta el
    /// próximo VBlank. Refresca los búferes de marco una vez al principio.
    pub(super) fn render(&mut self) {
        if !self.active {
            return; // la sesión está en otra VT — no tocamos la GPU
        }
        self.refresh_window_borders();
        for i in 0..self.outputs.len() {
            self.render_output(i);
        }
        self.send_frames_to_clients();
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
        match &self.app.cursor_status {
            CursorImageStatus::Hidden => {}
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
            }
            _ => {
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
        }
    }

    /// Emite todas las ventanas visibles cuya posición global intersecta `rect`,
    /// traducidas a coordenadas locales a `rect`. Incluye marcos, barras de
    /// título y el árbol de superficie del cliente, en orden front-to-back
    /// (`shell` arriba > flotantes > teseladas). Se saltea ventanas que no
    /// caen sobre `rect` para no malgastar trabajo del compositor.
    fn emit_windows(&mut self, rect: Rect, into: &mut Vec<Frame<GlesRenderer>>) {
        let mut shown: Vec<_> = self.app.windows.iter().filter(|w| w.visible).collect();
        shown.sort_by_key(|w| (!w.is_shell, !w.floating, !w.focused));
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
            // Posición local de la superficie y de la decoración.
            let x = gx - rect.x;
            let y = gy - rect.y;
            let dec_y = y - tb;
            let dec_h = sh + tb;

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
                            ((x + 8) as f64, ty as f64),
                            buf,
                            None,
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
                        if sw < (slot + 1) * crate::TB_BTN_W + 8 {
                            continue; // ventana muy angosta: sin botón
                        }
                        {
                            let r = icon;
                            let cell_x = x + sw - (slot + 1) * crate::TB_BTN_W;
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
                                None,
                                None,
                                None,
                                Kind::Unspecified,
                            ) {
                                into.push(Frame::Text(el));
                            }
                        }
                    }
                }
                let base = if w.focused {
                    self.app.decorations.border_focus
                } else {
                    self.app.decorations.border_normal
                };
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
                        band.update((sw, h), col);
                        into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                            &band,
                            (x, by),
                            1.0,
                            1.0,
                            Kind::Unspecified,
                        )));
                    }
                } else {
                    let color = rgba_f32(base);
                    let mut bar = SolidColorBuffer::default();
                    bar.update((sw, tb), color);
                    into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                        &bar,
                        (x, dec_y),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    )));
                }
            } else if w.focused && !w.is_shell && !w.title.is_empty() {
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
                        ((x + 6) as f64, (y + 4) as f64),
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
            if !w.is_shell && self.app.decorations.border_width > 0 {
                let rects = border_rects(x, dec_y, sw, dec_h, self.app.decorations.border_width);
                for (buf, (bx, by, _, _)) in w.borders.iter().zip(rects) {
                    into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                        buf,
                        (bx, by),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    )));
                }
            }
            for el in render_elements_from_surface_tree(
                &mut self.renderer,
                &w.surface,
                (x, y),
                1.0,
                1.0,
                Kind::Unspecified,
            ) {
                into.push(Frame::Window(el));
            }
            // Sombra: capas negras translúcidas DETRÁS de la ventana (se empujan
            // después del contenido = quedan al fondo). Sin shader — rects que se
            // expanden y se desplazan hacia abajo fingen un degradé suave. Gateada
            // por MIRADA_SHADOW mientras se verifica en pantalla.
            if shadows_on && !w.is_shell {
                // (expansión, desplazamiento-y, alfa): de afuera-tenue a cerca-fuerte.
                for &(exp, dy, a) in &[(12i32, 10i32, 0.06f32), (6, 5, 0.10), (2, 2, 0.16)] {
                    let mut sh = SolidColorBuffer::default();
                    sh.update((sw + exp * 2, dec_h + exp * 2), [0.0, 0.0, 0.0, a]);
                    into.push(Frame::Solid(SolidColorRenderElement::from_buffer(
                        &sh,
                        (x - exp, dec_y - exp + dy),
                        1.0,
                        1.0,
                        Kind::Unspecified,
                    )));
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
        // Scrim OPACO cuando está desplegada (esconde el escritorio real detrás);
        // se desvanece con el zoom para que al «salir» del activo no haya corte.
        const SCRIM: [f32; 4] = [0.04, 0.05, 0.07, 1.0];
        const TILE_BG: [f32; 4] = [0.12, 0.13, 0.17, 1.0];
        const WIN_BG: [f32; 4] = [0.26, 0.30, 0.40, 1.0];
        const WIN_FOCUS: [f32; 4] = [0.22, 0.45, 0.85, 1.0];
        const ACTIVE_BORDER: [f32; 4] = [0.20, 0.50, 0.95, 1.0];
        const BADGE_TX: [u8; 4] = [235, 238, 245, 255];

        let occ: Vec<usize> = (0..data.loads.len()).filter(|&i| data.loads[i] > 0).collect();
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
            // Ventanas en coords LOCALES del tile (el rasterizador pinta local).
            // Los tiles girados se componen en CPU como esquema (rects), no con
            // la superficie viva — el camino vivo es sólo para tiles rectos.
            let wins_local: Vec<(i32, i32, i32, i32, bool)> =
                t.wins.iter().map(|(_id, wx, wy, ww2, wh2, f)| (wx - t.x, wy - t.y, *ww2, *wh2, *f)).collect();
            let border = t.selected.then(|| to_u8(ACTIVE_BORDER));
            let comp = crate::text::rasterize_tile_rotated(
                t.w,
                t.h,
                t.rot,
                to_u8(TILE_BG),
                border,
                &wins_local,
                to_u8(WIN_BG),
                to_u8(WIN_FOCUS),
                Some(badge),
            );
            let buf = MemoryRenderBuffer::from_slice(
                &comp.rgba,
                Fourcc::Argb8888,
                (comp.width, comp.height),
                1,
                Transform::Normal,
                None,
            );
            // Coloca el AABB centrado en el centro del tile.
            let ax = t.x as f64 + t.w as f64 / 2.0 - comp.width as f64 / 2.0;
            let ay = t.y as f64 + t.h as f64 / 2.0 - comp.height as f64 / 2.0;
            if let Ok(el) = MemoryRenderBufferRenderElement::from_buffer(
                &mut self.renderer,
                (ax, ay),
                &buf,
                None,
                None,
                None,
                Kind::Unspecified,
            ) {
                into.push(Frame::Text(el));
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
                None,
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
                    let elems = render_elements_from_surface_tree(
                        &mut self.renderer,
                        &surface,
                        (*wx, *wy),
                        t.scale as f64,
                        1.0,
                        Kind::Unspecified,
                    );
                    if !elems.is_empty() {
                        for el in elems {
                            into.push(Frame::Window(el));
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
                    1.0,
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
                1.0,
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
                1.0,
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

    /// Emite el wallpaper de la salida `idx` al fondo (rearmándolo si quedó
    /// stale). Cada salida tiene su propio búfer escalado, su propia ruta y
    /// su propio modo de ajuste — un override por nombre puede pintarle un
    /// fondo distinto a cada monitor.
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
        if stale {
            // Despacho por la FUENTE elegida (color/gradiente/procedural/imagen).
            use crate::estado::WallpaperSpec;
            let spec = self.app.config_wallpaper_spec_for(&name, cur_path.as_deref());
            let buf = match spec {
                WallpaperSpec::Image(p, fit) => load_wallpaper(&p, fit, size.0, size.1),
                WallpaperSpec::Solid(c) => Some(make_solid_wallpaper(c, size.0, size.1)),
                WallpaperSpec::Gradient(stops) => {
                    Some(make_gradient_wallpaper(&stops, size.0, size.1))
                }
                WallpaperSpec::Procedural(pat, pal) => {
                    Some(make_procedural_wallpaper(pat, &pal, size.0, size.1))
                }
                WallpaperSpec::Default => Some(make_default_wallpaper(size.0, size.1)),
            };
            self.outputs[idx].wallpaper = buf.map(|b| (b, size));
        }
        let ctx = &self.outputs[idx];
        let Some((buf, _)) = &ctx.wallpaper else {
            return;
        };
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
    }

    /// Refresca los búferes de marco (color por foco) de todas las ventanas
    /// visibles. Es estado global que no depende de cuál salida se está
    /// rindiendo — se hace una vez al inicio de [`Self::render`].
    fn refresh_window_borders(&mut self) {
        let dec = self.app.decorations;
        let Some(primary) = self.outputs.get(Self::PRIMARY) else {
            return; // sin monitores: no hay bordes que recalcular
        };
        let output_h = primary.rect.h;
        for w in &mut self.app.windows {
            if !w.visible || w.is_shell {
                continue;
            }
            let tb = crate::titlebar_for(w, dec.titlebar_height);
            let (x, y) = crate::render_loc(w, output_h, dec.titlebar_height);
            let (sw, sh) = crate::surface_px_size(w).unwrap_or((w.size.0, w.size.1 - tb));
            let (x, y, sh) = (x, y - tb, sh + tb);
            let color = rgba_f32(if w.focused {
                dec.border_focus
            } else {
                dec.border_normal
            });
            let rects = border_rects(x, y, sw, sh, dec.border_width);
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
        for w in &mut self.app.windows {
            w.frame_tick = w.frame_tick.wrapping_add(1);
            // Las capas dormidas (zoom-Z) no reciben frame callbacks.
            if w.suspended {
                continue;
            }
            // Throttle de fondo: 1 de cada `frame_divisor` vblanks.
            let div = w.frame_divisor.max(1);
            if div > 1 && w.frame_tick % div != 0 {
                continue;
            }
            send_frames_surface_tree(&w.surface, time);
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
                        Ok(()) => ctx.pending_flip = true,
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
