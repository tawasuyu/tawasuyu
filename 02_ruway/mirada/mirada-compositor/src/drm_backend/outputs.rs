use super::*;

impl DrmState {
    /// Índice de la salida primaria en [`Self::outputs`]. Hoy hard-coded a 0
    /// (la primera descubierta); a futuro será configurable.
    pub(super) const PRIMARY: usize = 0;

    /// Aplica las rampas de gamma que dejó pendientes el protocolo
    /// `wlr-gamma-control` (luz nocturna): por cada `(salida, rampa)` busca su
    /// CRTC y llama `set_gamma`; con `None` (control soltado) restaura la
    /// identidad. Se llama una vez por iteración del bucle, tras despachar a los
    /// clientes. Errores del driver se loguean sin abortar el frame.
    pub(super) fn aplicar_gamma_pendiente(&mut self) {
        if self.app.pending_gamma.is_empty() {
            return;
        }
        for (output, rampa) in std::mem::take(&mut self.app.pending_gamma) {
            let Some(ctx) = self.outputs.iter().find(|c| c.output == output) else {
                continue; // la salida se fue
            };
            let crtc = ctx.crtc;
            let rampa = match rampa {
                Some(r) => r,
                None => {
                    // Reset a identidad: tamaño = gamma_length del CRTC.
                    let len = self.drm.get_crtc(crtc).map(|i| i.gamma_length()).unwrap_or(0);
                    if len == 0 {
                        continue;
                    }
                    crate::gamma_control::identidad(len)
                }
            };
            if let Err(e) = self
                .drm
                .set_gamma(crtc, &rampa.red, &rampa.green, &rampa.blue)
            {
                eprintln!("mirada-compositor · set_gamma falló en CRTC {crtc:?}: {e}");
            }
        }
    }

    /// Encuentra el índice de la salida cuyo CRTC es `crtc`, si existe.
    pub(super) fn output_index_by_crtc(
        &self,
        crtc: smithay::reexports::drm::control::crtc::Handle,
    ) -> Option<usize> {
        self.outputs.iter().position(|o| o.crtc == crtc)
    }

    /// Índice de la salida que contiene el punto global `(gx, gy)`. Si el
    /// punto cae en zona muerta entre rects (puede pasar con salidas de
    /// distinto tamaño dispuestas side-by-side), devuelve la primaria.
    pub(super) fn output_at_point(&self, gx: i32, gy: i32) -> usize {
        self.outputs
            .iter()
            .position(|o| {
                gx >= o.rect.x
                    && gy >= o.rect.y
                    && gx < o.rect.x + o.rect.w
                    && gy < o.rect.y + o.rect.h
            })
            .unwrap_or(Self::PRIMARY)
    }

    /// Acota un punto al **interior de algún output**, respetando la geometría
    /// real. Si `(x, y)` ya cae sobre un output, se devuelve tal cual; si no
    /// (zona muerta entre rects de distinto tamaño), se proyecta al borde
    /// del output euclídeamente más cercano. Sin esto, el cursor podía
    /// quedar atrapado en zonas que ninguna salida pinta — el usuario lo ve
    /// como un cursor fantasma sobre fondo negro.
    pub(super) fn clamp_to_outputs(&self, x: f64, y: f64) -> (f64, f64) {
        let xi = x.round() as i32;
        let yi = y.round() as i32;
        if self.outputs.iter().any(|o| {
            xi >= o.rect.x
                && yi >= o.rect.y
                && xi < o.rect.x + o.rect.w
                && yi < o.rect.y + o.rect.h
        }) {
            return (x, y);
        }
        // El menor cuadrado-distancia al rect proyecta `(x, y)` al borde.
        let Some(first) = self.outputs.first() else {
            return (x, y); // sin monitores conectados: nada a lo que recortar
        };
        let mut best = (first.rect, f64::INFINITY);
        for o in &self.outputs {
            let r = o.rect;
            if r.w <= 0 || r.h <= 0 {
                continue;
            }
            let cx = x.clamp(r.x as f64, (r.x + r.w - 1) as f64);
            let cy = y.clamp(r.y as f64, (r.y + r.h - 1) as f64);
            let d = (x - cx).powi(2) + (y - cy).powi(2);
            if d < best.1 {
                best = (r, d);
            }
        }
        let r = best.0;
        (
            x.clamp(r.x as f64, (r.x + r.w - 1) as f64),
            y.clamp(r.y as f64, (r.y + r.h - 1) as f64),
        )
    }

    /// El área útil (rect menos reservas) **de una salida concreta**: a las
    /// layers exclusivas de su `layer_map` se le suma, sólo en la primaria,
    /// la franja del shell (pata). Devuelve rect en coords globales — el
    /// teselado y las zonas de arrastre lo usan como dominio efectivo.
    pub(super) fn output_work_rect(&self, idx: usize) -> Rect {
        // `output_at_point` cae a `PRIMARY` cuando el punto no toca ninguna
        // salida — incluido el caso de 0 monitores, donde `outputs` está
        // vacío. Y un `idx` de antes de un desenchufe puede quedar fuera de
        // rango. En ambos casos el dominio de zonas degenera al tamaño lógico,
        // sin reservas: no hay panic ni salida a la que recortar.
        let Some(o) = self.outputs.get(idx) else {
            return Rect::new(0, 0, self.output_size.0 as i32, self.output_size.1 as i32);
        };
        // Layers exclusivas de ESTA salida: la zona "no exclusiva" da los
        // insets directos.
        let z = smithay::desktop::layer_map_for_output(&o.output).non_exclusive_zone();
        let mut top = z.loc.y.max(0);
        let mut left = z.loc.x.max(0);
        let mut right = (o.rect.w - (z.loc.x + z.size.w)).max(0);
        let mut bottom = (o.rect.h - (z.loc.y + z.size.h)).max(0);
        // El dock del shell (pata) sólo se descuenta en la primaria.
        if idx == Self::PRIMARY {
            let (rt, rb, rl, rr) = self.app.reserved;
            // Las reservas del shell ya están sumadas en `self.app.reserved`
            // (recompute_reservations las publica), pero `app.reserved`
            // incluye también las de layer-shell (no podemos restarlas
            // limpiamente). Tomamos `max` para no doble-contar: la mayor
            // gana, y el shell (que es la suma) cubre los dos casos.
            top = top.max(rt);
            bottom = bottom.max(rb);
            left = left.max(rl);
            right = right.max(rr);
        }
        Rect::new(
            o.rect.x + left,
            o.rect.y + top,
            (o.rect.w - left - right).max(1),
            (o.rect.h - top - bottom).max(1),
        )
    }

    /// Reenumera los conectores DRM y aplica las diferencias con
    /// [`Self::outputs`]: monitores recién enchufados se agregan, los que
    /// dejan de estar Connected se quitan. En cualquier cambio, se re-dispone
    /// la geometría global, se notifica al Brain y se rearman las reservas.
    pub(super) fn detect_connector_changes(&mut self) {
        use smithay::reexports::drm::control::{connector, crtc};
        let resources = match self.drm.resource_handles() {
            Ok(r) => r,
            Err(e) => {
                eprintln!("mirada-compositor · hotplug · no pude releer DRM: {e}");
                return;
            }
        };
        // Conectores Connected ahora, con su handle + nombre.
        let mut live: Vec<(connector::Handle, String)> = Vec::new();
        for &h in resources.connectors() {
            let Ok(c) = self.drm.get_connector(h, false) else {
                continue;
            };
            if c.state() == ConnectorState::Connected {
                live.push((h, format!("{:?}-{}", c.interface(), c.interface_id())));
            }
        }
        let live_names: std::collections::HashSet<&str> =
            live.iter().map(|(_, n)| n.as_str()).collect();
        let known_names: std::collections::HashSet<String> =
            self.outputs.iter().map(|o| o.name.clone()).collect();

        let mut changed = false;

        // 1 · Desenchufes — drop OutputCtx + remove_output al Brain.
        let to_remove: Vec<usize> = self
            .outputs
            .iter()
            .enumerate()
            .filter(|(_, o)| !live_names.contains(o.name.as_str()))
            .map(|(i, _)| i)
            .collect();
        for &i in to_remove.iter().rev() {
            let name = self.outputs[i].name.clone();
            println!("mirada-compositor · hotplug · monitor «{name}» desenchufado");
            let ev = self.app.body.remove_output(i as u32);
            self.app.brain_feed(ev);
            // Drop del compositor + smithay::Output: la GPU libera recursos.
            let _ = self.outputs.remove(i);
            changed = true;
        }

        // El menú raíz se ancla a una salida por índice; tras un desenchufe ese
        // índice puede quedar viejo (fuera de rango o apuntando a otro monitor).
        // Lo cerramos para que no pinte con coords muertas ni indexe de más.
        if self
            .menu_output_idx
            .is_some_and(|i| i >= self.outputs.len())
        {
            self.root_menu = None;
            self.menu_output_idx = None;
        }

        // 2 · Enchufes — armar OutputCtx para cada conector nuevo.
        let used_crtcs: Vec<crtc::Handle> = self.outputs.iter().map(|o| o.crtc).collect();
        let mut taken: Vec<crtc::Handle> = used_crtcs.clone();
        for (conn_handle, name) in &live {
            if known_names.contains(name) {
                continue;
            }
            let Ok(conn) = self.drm.get_connector(*conn_handle, false) else {
                continue;
            };
            // Modo: el de mayor área (a igualdad, mayor refresco).
            let Some(mode) = conn
                .modes()
                .iter()
                .max_by_key(|m| {
                    let (w, h) = m.size();
                    (w as u32 * h as u32, m.vrefresh())
                })
                .copied()
            else {
                continue;
            };
            // CRTC libre compatible.
            let crtc_choice = conn
                .encoders()
                .iter()
                .filter_map(|enc| self.drm.get_encoder(*enc).ok())
                .find_map(|enc| {
                    resources
                        .filter_crtcs(enc.possible_crtcs())
                        .into_iter()
                        .find(|c| !taken.contains(c))
                });
            let Some(crtc_h) = crtc_choice else {
                eprintln!("mirada-compositor · hotplug · «{name}» sin CRTC libre — se ignora");
                continue;
            };
            taken.push(crtc_h);
            match self.armar_output_ctx(*conn_handle, crtc_h, mode, name.clone()) {
                Ok(mut ctx) => {
                    println!("mirada-compositor · hotplug · monitor «{}» enchufado", ctx.name);
                    let (w, h) = mode.size();
                    // Id estable y nunca reusado (ver `next_output_id`): así un
                    // monitor que reaparece tras un desenchufe no colisiona con
                    // otro que heredó su antiguo índice.
                    let id = self.next_output_id;
                    self.next_output_id += 1;
                    ctx.id = id;
                    let ev = self.app.body.add_output(id, w as i32, h as i32);
                    self.app.brain_feed(ev);
                    self.outputs.push(ctx);
                    changed = true;
                }
                Err(e) => eprintln!("mirada-compositor · hotplug · falló «{name}»: {e}"),
            }
        }

        if changed {
            self.redisponer_outputs();
        }
    }

    /// Crea un `OutputCtx` nuevo desde un conector recién enchufado: arma
    /// `DrmSurface` + `DrmCompositor` + `smithay::Output` con la escala y
    /// transformación que mande la config. Idéntica a la rama del discovery
    /// inicial — el día que haya que tocar uno hay que tocar el otro.
    pub(super) fn armar_output_ctx(
        &mut self,
        conn_handle: smithay::reexports::drm::control::connector::Handle,
        crtc_h: smithay::reexports::drm::control::crtc::Handle,
        mode: smithay::reexports::drm::control::Mode,
        name: String,
    ) -> Result<OutputCtx, String> {
        let (w, h) = mode.size();
        let surface = self
            .drm
            .create_surface(crtc_h, mode, &[conn_handle])
            .map_err(|e| format!("create_surface: {e}"))?;
        let scale_120 = self.app.config_output_scale_120_for(&name);
        let transform = self.app.config_output_transform_for(&name);
        let scale_f64 = (if scale_120 > 0 { scale_120 } else { 120 }) as f64 / 120.0;
        let mode_source = OutputModeSource::Static {
            size: Size::from((w as i32, h as i32)),
            scale: Scale::from(scale_f64),
            transform,
        };
        let compositor: Compositor = DrmCompositor::new(
            mode_source,
            surface,
            None,
            self.allocator.clone(),
            self.exporter.clone(),
            [Fourcc::Argb8888, Fourcc::Xrgb8888],
            self.renderer_formats.clone(),
            self.drm.cursor_size(),
            Some(self.gbm.clone()),
        )
        .map_err(|e| format!("DrmCompositor::new: {e}"))?;
        let refresh_mhz = mode.vrefresh() as i32 * 1000;
        let smithay_out = crate::announce_output(
            &self.dh,
            &name,
            w as i32,
            h as i32,
            refresh_mhz,
            scale_120,
            transform,
        );
        let wp_path = self.app.config_wallpaper_path_for(&name);
        let wp_fit = self.app.config_wallpaper_fit_for(&name);
        Ok(OutputCtx {
            // id real lo asigna `detect_connector_changes` antes de registrar.
            id: 0,
            name,
            output: smithay_out,
            crtc: crtc_h,
            connector: conn_handle,
            compositor,
            // rect lo fija `redisponer_outputs` después de añadir.
            rect: Rect::new(0, 0, w as i32, h as i32),
            refresh_mhz,
            wallpaper: None,
            wallpaper_path: wp_path,
            wallpaper_fit: wp_fit,
            pending_flip: false,
        })
    }

    /// Re-ordena las salidas por `(order, name)` de la config, recalcula sus
    /// rects globales con `mirada-layout::disponer`, actualiza el espacio
    /// total y resincroniza `app.outputs`/`app.output`/`app.output_size`,
    /// invalida wallpapers (rearmados al próximo render) y re-emite las
    /// reservas. Lo que NO toca: ventanas — el Brain decide a dónde van.
    pub(super) fn redisponer_outputs(&mut self) {
        if self.outputs.is_empty() {
            self.app.outputs.clear();
            self.app.output_ids.clear();
            self.app.output = None;
            self.app.output_size = (1, 1);
            return;
        }
        // Sort por (order, name) — primaria queda en outputs[0].
        let app_ref = &self.app;
        self.outputs.sort_by(|a, b| {
            let oa = app_ref.config_output_order_for(&a.name);
            let ob = app_ref.config_output_order_for(&b.name);
            oa.cmp(&ob).then_with(|| a.name.cmp(&b.name))
        });
        let tamanos: Vec<(i32, i32)> = self.outputs.iter().map(|o| (o.rect.w, o.rect.h)).collect();
        let disp = self.app.config_output_disposition();
        let rects = mirada_brain::disponer(&tamanos, disp);
        for (ctx, r) in self.outputs.iter_mut().zip(rects.iter()) {
            ctx.rect = *r;
            ctx.wallpaper = None; // el tamaño global no cambia, pero la posición sí
        }
        // Empujar al Cerebro la posición global de cada monitor por su id
        // estable: es la fuente única de la disposición. Sin esto el Cerebro
        // la reconstruía por orden de aparición y, al diferir del orden real
        // `(order, name)`, maximizaba/teselaba en el monitor equivocado.
        let geo: Vec<(u32, i32, i32)> =
            self.outputs.iter().map(|c| (c.id, c.rect.x, c.rect.y)).collect();
        for (id, x, y) in geo {
            let ev = self.app.body.move_output(id, x, y);
            self.app.brain_feed(ev);
        }
        let env = mirada_brain::envolvente(&rects);
        let total_w = env.w.max(1);
        let total_h = env.h.max(1);
        self.app.output_size = (total_w, total_h);
        self.output_size = (total_w as f64, total_h as f64);
        // Resincronizar el registro Wayland.
        self.app.outputs = self.outputs.iter().map(|c| c.output.clone()).collect();
        self.app.output_ids = self.outputs.iter().map(|c| c.id).collect();
        self.app.output = self.outputs.first().map(|c| c.output.clone());
        // Reposicionar el puntero al centro de la primaria si quedó fuera.
        let (px, py) = self.app.pointer_loc;
        let (px, py) = self.clamp_to_outputs(px, py);
        self.app.pointer_loc = (px, py);
        // Reservas y borders pueden cambiar con la nueva geometría.
        self.app.recompute_reservations();
        // Modo DM: la unión de salidas cambió — reenviá la disposición al
        // greeter (rects nuevos aunque el índice activo no haya cambiado).
        self.sync_greeter_layout(true);
    }

    /// Empuja al greeter (modo DM) la disposición de monitores y cuál tiene el
    /// ratón, por su `stdin`. El greeter usa esto para que la tarjeta de login
    /// viaje al monitor activo mientras el fondo animado sigue en todos. Los
    /// rects van en coordenadas **locales a la superficie** (el greeter cubre la
    /// unión de las salidas anclada en su origen). No-op sin un shell de
    /// credenciales arriba (login o lock) o sin tubería. Con `force`, reenvía
    /// aunque el monitor activo no haya cambiado (arranque / hotplug, donde
    /// cambian los rects pero no el índice).
    pub(super) fn sync_greeter_layout(&mut self, force: bool) {
        use std::io::Write;
        if !self.app.shell_activo() || self.outputs.is_empty() {
            return;
        }
        // Origen de la superficie = esquina superior-izquierda de la unión.
        let ox = self.outputs.iter().map(|o| o.rect.x).min().unwrap_or(0);
        let oy = self.outputs.iter().map(|o| o.rect.y).min().unwrap_or(0);
        let (span_w, span_h) = self.app.output_size;

        // Auto-reparación de la geometría: reafirma que el greeter cubre la
        // unión entera. Sin esto, una carrera de arranque (el Cerebro lo teseló
        // en un solo monitor antes de descubrir el segundo, o el cliente no
        // había redimensionado aún) dejaba un monitor en negro. Sólo re-emite el
        // configure cuando el tamaño difiere — no spamea cada frame.
        if let Some(w) = self.app.windows.iter_mut().find(|w| w.is_greeter) {
            if w.loc != (ox, oy) || w.size != (span_w, span_h) {
                w.loc = (ox, oy);
                w.size = (span_w, span_h);
                w.visible = true;
                w.toplevel.with_pending_state(|s| {
                    s.size = Some((span_w.max(1), span_h.max(1)).into());
                });
                w.toplevel.send_pending_configure();
                crate::screencopy::danar_todo(&mut self.app);
            }
        }

        let (px, py) = self.app.pointer_loc;
        let active = self.output_at_point(px.round() as i32, py.round() as i32);
        if !force && active == self.app.greeter_active_output {
            return;
        }
        let mut line = format!("LAYOUT {active}");
        for o in &self.outputs {
            line.push_str(&format!(
                " {},{},{},{}",
                o.rect.x - ox,
                o.rect.y - oy,
                o.rect.w,
                o.rect.h
            ));
        }
        line.push('\n');
        let mut drop_pipe = false;
        if let Some(stdin) = self.app.greeter_stdin.as_mut() {
            if stdin.write_all(line.as_bytes()).and_then(|_| stdin.flush()).is_err() {
                drop_pipe = true; // el greeter cerró su stdin: dejamos de empujar
            }
        }
        if drop_pipe {
            self.app.greeter_stdin = None;
        }
        self.app.greeter_active_output = active;
    }

    /// Work rect del monitor bajo el puntero — el "lienzo" de zonas para
    /// arrastres. Multi-monitor: los zonas se escalan al monitor donde
    /// está la acción, no al desktop global.
    pub(super) fn work_rect(&self) -> Rect {
        let (px, py) = self.app.pointer_loc;
        self.output_work_rect(self.output_at_point(px.round() as i32, py.round() as i32))
    }

    /// El **rect destino** del drag-to-zone bajo `(x, y)`, en coords globales, o
    /// `None` si el puntero no está pegado a ningún borde (la ventana cae libre).
    /// Estilo KDE6: esquinas→cuartos, arriba→maximizar / mitad superior,
    /// abajo→mitad inferior, izq/der→mitades. La decisión pura vive en
    /// [`snap_target`].
    pub(super) fn zone_at(&self, x: f64, y: f64) -> Option<Rect> {
        let (xi, yi) = (x.round() as i32, y.round() as i32);
        let wr = self.output_work_rect(self.output_at_point(xi, yi));
        let (mx, my) = zone_margins(wr, self.tiledad);
        snap_target(wr, xi, yi, mx, my)
    }
}

/// Banda de snap mínima (px) con tiledad 0: sólo el borde mismo captura — el
/// resto del área deja caer la ventana libre (z-order, estilo Windows).
pub(super) const ZONE_BAND_MIN: i32 = 16;

/// Las **medias-bandas** de snap `(horizontal, vertical)` en px, derivadas de
/// la *tiledad* `t∈[0,1]` y el work-rect del monitor. La banda crece como `t²`
/// (poco snap a tiledad baja, mucho arriba) desde [`ZONE_BAND_MIN`] hasta la
/// media-extensión del monitor: con `t=1` cualquier punto cae dentro de alguna
/// banda → soltar tesela siempre (Hyprland); con `t≈0` sólo el borde fino hace
/// snap y el centro queda flotante (Windows). Es la traducción del valor difuso
/// de [`mirada_brain::Config::tiledad`] al **tamaño del área que pre-pinta** el
/// drag-to-zone.
pub(super) fn zone_margins(wr: Rect, t: f32) -> (i32, i32) {
    let curve = t.clamp(0.0, 1.0).powi(2);
    let band = |half: i32| -> i32 {
        let half = half.max(1);
        let m = ZONE_BAND_MIN as f32 + (half as f32 - ZONE_BAND_MIN as f32) * curve;
        (m.round() as i32).clamp(ZONE_BAND_MIN.min(half), half)
    };
    (band(wr.w / 2), band(wr.h / 2))
}

/// Decisión PURA del drag-to-zone, estilo KDE6. Dado el work-rect del monitor
/// (coords globales), el punto del puntero y las medias-bandas de borde
/// `(mx, my)` (horizontal/vertical — ver [`zone_margins`]), devuelve el **rect
/// destino** donde aterrizará la ventana, o `None` si el puntero está en el
/// centro fuera de toda banda (sin snap → cae libre). Prioridad: esquinas
/// (cuartos) antes que bordes. En el borde superior: muy arriba = maximizar; un
/// poco más abajo = mitad superior.
pub(super) fn snap_target(wr: Rect, xi: i32, yi: i32, mx: i32, my: i32) -> Option<Rect> {
    let dl = xi - wr.x;
    let dr = wr.x + wr.w - xi;
    let dt = yi - wr.y;
    let db = wr.y + wr.h - yi;
    let (near_l, near_r) = (dl < mx, dr < mx);
    let (near_t, near_b) = (dt < my, db < my);

    let hw = wr.w / 2;
    let hh = wr.h / 2;
    let left = Rect::new(wr.x, wr.y, hw, wr.h);
    let right = Rect::new(wr.x + hw, wr.y, wr.w - hw, wr.h);
    let top = Rect::new(wr.x, wr.y, wr.w, hh);
    let bottom = Rect::new(wr.x, wr.y + hh, wr.w, wr.h - hh);
    let tl = Rect::new(wr.x, wr.y, hw, hh);
    let tr = Rect::new(wr.x + hw, wr.y, wr.w - hw, hh);
    let bl = Rect::new(wr.x, wr.y + hh, hw, wr.h - hh);
    let br = Rect::new(wr.x + hw, wr.y + hh, wr.w - hw, wr.h - hh);

    // Esquinas primero (cuartos).
    if near_t && near_l {
        return Some(tl);
    }
    if near_t && near_r {
        return Some(tr);
    }
    if near_b && near_l {
        return Some(bl);
    }
    if near_b && near_r {
        return Some(br);
    }
    // Bordes.
    if near_t {
        // Tira fina al tope = maximizar; el resto de la banda = mitad superior.
        // El umbral sigue a la banda vertical pero se acota para que maximizar
        // sea un gesto deliberado del borde, no media pantalla a tiledad alta.
        let max_strip = (my / 3).clamp(6, 48);
        return Some(if dt < max_strip { wr } else { top });
    }
    if near_b {
        return Some(bottom);
    }
    if near_l {
        return Some(left);
    }
    if near_r {
        return Some(right);
    }
    None
}

#[cfg(test)]
mod zone_tests {
    use super::{snap_target, zone_margins, Rect, ZONE_BAND_MIN};

    const WR: Rect = Rect { x: 0, y: 0, w: 1920, h: 1080 };
    const M: i32 = 64;

    #[test]
    fn esquina_superior_izquierda_es_un_cuarto() {
        assert_eq!(snap_target(WR, 10, 10, M, M), Some(Rect::new(0, 0, 960, 540)));
    }

    #[test]
    fn borde_superior_muy_arriba_maximiza() {
        // y=5 (< margin/3=21) en el medio horizontal → pantalla completa.
        assert_eq!(snap_target(WR, 960, 5, M, M), Some(WR));
    }

    #[test]
    fn borde_superior_un_poco_mas_abajo_es_mitad_superior() {
        // y=40 (> 21, < 64) → mitad superior.
        assert_eq!(snap_target(WR, 960, 40, M, M), Some(Rect::new(0, 0, 1920, 540)));
    }

    #[test]
    fn borde_inferior_es_mitad_inferior() {
        assert_eq!(snap_target(WR, 960, 1075, M, M), Some(Rect::new(0, 540, 1920, 540)));
    }

    #[test]
    fn bordes_laterales_son_mitades() {
        assert_eq!(snap_target(WR, 10, 540, M, M), Some(Rect::new(0, 0, 960, 1080)));
        assert_eq!(snap_target(WR, 1915, 540, M, M), Some(Rect::new(960, 0, 960, 1080)));
    }

    #[test]
    fn el_centro_no_hace_snap() {
        assert_eq!(snap_target(WR, 960, 540, M, M), None);
    }

    #[test]
    fn respeta_el_origen_del_monitor() {
        // Monitor secundario con origen (1920,0): borde izq = x=1920.
        let wr = Rect::new(1920, 0, 1920, 1080);
        assert_eq!(snap_target(wr, 1930, 540, M, M), Some(Rect::new(1920, 0, 960, 1080)));
        assert_eq!(snap_target(wr, 2880, 540, M, M), None); // centro
    }

    // --- Tiledad → tamaño de la banda de drag-to-zone --------------------

    #[test]
    fn tiledad_cero_es_banda_minima() {
        // Flotante puro: sólo el borde fino captura.
        let (mx, my) = zone_margins(WR, 0.0);
        assert_eq!(mx, ZONE_BAND_MIN);
        assert_eq!(my, ZONE_BAND_MIN);
    }

    #[test]
    fn tiledad_uno_cubre_media_extension() {
        // Teselado puro: la banda llega a la media-extensión → casi todo punto
        // cae en alguna banda y tesela al soltar.
        let (mx, my) = zone_margins(WR, 1.0);
        assert_eq!(mx, WR.w / 2);
        assert_eq!(my, WR.h / 2);
    }

    #[test]
    fn tiledad_crece_monotona_y_la_banda_la_sigue() {
        // Más tiledad ⇒ banda no menor (y estrictamente mayor entre extremos).
        let bandas: Vec<i32> = [0.0_f32, 0.2, 0.5, 0.8, 1.0]
            .iter()
            .map(|&t| zone_margins(WR, t).0)
            .collect();
        for w in bandas.windows(2) {
            assert!(w[1] >= w[0]);
        }
        assert!(bandas[0] < bandas[4]);
    }

    #[test]
    fn tiledad_baja_deja_libre_el_centro_pero_alta_lo_tesela() {
        // Mismo punto (centro-ish, lejos de bordes): a tiledad baja cae libre,
        // a tiledad alta hace snap. Esa es la diferencia Windows↔Hyprland.
        let p = (700, 400); // dentro pero lejos del borde
        let (lo_x, lo_y) = zone_margins(WR, 0.15);
        assert_eq!(snap_target(WR, p.0, p.1, lo_x, lo_y), None);
        let (hi_x, hi_y) = zone_margins(WR, 0.95);
        assert!(snap_target(WR, p.0, p.1, hi_x, hi_y).is_some());
    }

    #[test]
    fn tiledad_se_clampa_fuera_de_rango() {
        assert_eq!(zone_margins(WR, -3.0), zone_margins(WR, 0.0));
        assert_eq!(zone_margins(WR, 9.0), zone_margins(WR, 1.0));
    }
}
