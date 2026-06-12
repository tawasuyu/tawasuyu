use super::*;

impl DrmState {
    /// Índice de la salida primaria en [`Self::outputs`]. Hoy hard-coded a 0
    /// (la primera descubierta); a futuro será configurable.
    pub(super) const PRIMARY: usize = 0;

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
                Ok(ctx) => {
                    println!("mirada-compositor · hotplug · monitor «{}» enchufado", ctx.name);
                    let (w, h) = mode.size();
                    let ev = self.app.body.add_output(
                        self.outputs.len() as u32,
                        w as i32,
                        h as i32,
                    );
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
            name,
            output: smithay_out,
            crtc: crtc_h,
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
        let env = mirada_brain::envolvente(&rects);
        let total_w = env.w.max(1);
        let total_h = env.h.max(1);
        self.app.output_size = (total_w, total_h);
        self.output_size = (total_w as f64, total_h as f64);
        // Resincronizar el registro Wayland.
        self.app.outputs = self.outputs.iter().map(|c| c.output.clone()).collect();
        self.app.output = self.outputs.first().map(|c| c.output.clone());
        // Reposicionar el puntero al centro de la primaria si quedó fuera.
        let (px, py) = self.app.pointer_loc;
        let (px, py) = self.clamp_to_outputs(px, py);
        self.app.pointer_loc = (px, py);
        // Reservas y borders pueden cambiar con la nueva geometría.
        self.app.recompute_reservations();
    }

    /// Work rect del monitor bajo el puntero — el "lienzo" de zonas para
    /// arrastres. Multi-monitor: los zonas se escalan al monitor donde
    /// está la acción, no al desktop global.
    pub(super) fn work_rect(&self) -> Rect {
        let (px, py) = self.app.pointer_loc;
        self.output_work_rect(self.output_at_point(px.round() as i32, py.round() as i32))
    }

    /// El rect en píxeles de la zona `i`, escalado al work-rect del
    /// monitor bajo el puntero. Devuelve coords globales.
    pub(super) fn zone_rect(&self, i: usize) -> Option<Rect> {
        let wr = self.work_rect();
        self.zones.get(i).map(|z| z.to_rect(wr))
    }

    /// El índice de la zona de arrastre bajo `(x, y)`, si la hay. Las zonas
    /// se hit-testean contra el work-rect del monitor que contiene `(x,y)`.
    pub(super) fn zone_at(&self, x: f64, y: f64) -> Option<usize> {
        if self.zones.is_empty() {
            return None;
        }
        let (xi, yi) = (x.round() as i32, y.round() as i32);
        let wr = self.output_work_rect(self.output_at_point(xi, yi));
        self.zones.iter().position(|z| {
            let r = z.to_rect(wr);
            xi >= r.x && yi >= r.y && xi < r.x + r.w && yi < r.y + r.h
        })
    }
}
