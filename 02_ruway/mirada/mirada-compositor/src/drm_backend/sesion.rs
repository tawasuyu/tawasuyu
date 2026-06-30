use super::*;

impl DrmState {
    /// La sesión se cede a otra VT (`Ctrl+Alt+Fn`): suelta la GPU y deja
    /// de leer el ratón y el teclado, para no chocar con quien ahora
    /// manda en la pantalla.
    pub(super) fn pause_session(&mut self) {
        self.active = false;
        self.drm.pause();
        self.libinput.suspend();
        println!("mirada-compositor · sesión cedida a otra VT.");
    }

    /// La sesión vuelve a esta VT: recupera la GPU y la entrada, reinicia
    /// el estado de cada compositor y repinta.
    pub(super) fn resume_session(&mut self) {
        if self.libinput.resume().is_err() {
            dlog!("mirada-compositor · libinput.resume falló.");
        }
        if let Err(e) = self.drm.activate(false) {
            dlog!("mirada-compositor · drm.activate falló: {e}");
        }
        for ctx in &mut self.outputs {
            if let Err(e) = ctx.compositor.reset_state() {
                dlog!(
                    "mirada-compositor · compositor.reset_state[{}]: {e}",
                    ctx.name
                );
            }
            ctx.pending_flip = false;
        }
        self.active = true;
        self.render();
        println!("mirada-compositor · sesión recuperada.");
    }

    /// Apaga (`off=true`) o enciende físicamente las pantallas vía la propiedad
    /// `DPMS` de cada conector — el apagado por inactividad de la política de
    /// idle. Best-effort y tolerante a error: en algunos drivers con atomic el
    /// set legacy de `DPMS` puede no aplicar (la vía atómica correcta es togglear
    /// `ACTIVE` del CRTC en el commit del `DrmCompositor`) — **por verificar en
    /// metal**. Al encender, fuerza un repintado para recomponer lo que cambió.
    pub(super) fn set_dpms(&mut self, off: bool) {
        use smithay::reexports::drm::control::Device as _;
        // Recordá el estado: el wallpaper animado (video o marca) pausa su
        // decodificación/regeneración con la pantalla apagada (no se ve).
        self.dpms_off = off;
        // Valor estándar del kernel: 0 = On, 3 = Off.
        let value: u64 = if off { 3 } else { 0 };
        for ctx in &self.outputs {
            let Ok(props) = self.drm.get_properties(ctx.connector) else {
                continue;
            };
            let (handles, _values) = props.as_props_and_values();
            for &ph in handles {
                if let Ok(info) = self.drm.get_property(ph) {
                    if info.name().to_str() == Ok("DPMS") {
                        if let Err(e) = self.drm.set_property(ctx.connector, ph, value) {
                            dlog!("mirada-compositor · DPMS set falló ({}): {e}", ctx.name);
                        }
                        break;
                    }
                }
            }
        }
        if !off {
            self.render();
        }
        dlog!(
            "mirada-compositor · DPMS {} por inactividad.",
            if off { "off" } else { "on" }
        );
    }

    /// Tarea periódica: Cerebro enlazado, recarga del keymap, API de
    /// control, composición y vaciado hacia los clientes.
    pub(super) fn tick(&mut self) {
        self.app.brain_poll();

        // Política de inactividad: avanza el reloj de ocio y, si cruzó un umbral,
        // deja un pedido de DPMS / bloqueo. El apagado físico se aplica acá
        // (sólo el backend DRM tiene los conectores).
        self.app.idle_tick();
        if let Some(off) = self.app.pending_dpms.take() {
            self.set_dpms(off);
        }

        // Pedido de bloqueo (Super+Escape → `BrainCommand::Lock` → `request_lock`):
        // lanza el shell de credenciales en modo lock, compuesto encima de la
        // sesión. Se resuelve acá —no en `apply_commands`— porque hace falta el
        // emisor del canal del shell (`shell_tx`), que vive en `DrmState`.
        if let Some(user) = self.app.pending_lock.take() {
            let tx = self.shell_tx.clone();
            match crate::spawn_greeter(Some(&user), move |a| {
                let _ = tx.send(a);
            }) {
                Ok(stdin) => {
                    self.app.greeter_stdin = Some(stdin);
                    self.app.mode = crate::estado::BodyMode::Locked;
                    // Hero de lock: el próximo frame congela la pantalla y la
                    // encoge hasta el thumbnail antes de revelar el greeter.
                    self.app.pending_hero = true;
                    // Empuja el roster: el lock ofrece «cambiar usuario» a otra.
                    self.app.push_sessions_to_greeter();
                    dlog!("mirada-compositor · sesión bloqueada (lock de «{user}»).");
                }
                Err(e) => dlog!("mirada-compositor · no pude lanzar el lock: {e}"),
            }
        }
        // FUS «cambiar usuario»: relanza el greeter en modo LOGIN (sin `--lock`)
        // para hostear una sesión nueva encima de las residentes. El modo ya
        // quedó en `Greeter` por `request_new_session`.
        if self.app.pending_new_session && self.app.greeter_stdin.is_none() {
            let tx = self.shell_tx.clone();
            match crate::spawn_greeter(None::<&str>, move |a| {
                let _ = tx.send(a);
            }) {
                Ok(stdin) => {
                    self.app.greeter_stdin = Some(stdin);
                    dlog!("mirada-compositor · FUS: login para una sesión nueva.");
                }
                Err(e) => {
                    dlog!("mirada-compositor · no pude lanzar el login de FUS: {e}");
                    self.app.pending_new_session = false;
                    self.app.mode = crate::estado::BodyMode::Session;
                }
            }
        }

        let n = self.app.windows.len();
        if n != self.last_windows {
            dlog!("mirada-compositor · ventanas en pantalla: {n}");
            self.last_windows = n;
        }

        // Recarga en caliente de keymap/config/reglas si cambiaron en disco.
        // Si la config general cambió, refresca las cachés que el Cuerpo deriva
        // de ella (menú raíz, wallpaper, fuente de etiquetas) — el Cerebro ya
        // aplicó teselado/decoración/foco.
        if self.watches.poll(&mut self.app) {
            self.menu_entries = self.app.config_menu();
            // Re-siembra los umbrales de inactividad (pueden haber cambiado).
            self.app.sync_idle_config();
            // Reconstruye los presets de zonas y reacota el activo.
            let mut presets = vec![self.app.config_zones()];
            presets.extend(self.app.config_zone_presets());
            self.zone_presets = presets;
            if self.active_preset >= self.zone_presets.len() {
                self.active_preset = 0;
            }
            self.zones = self.zone_presets.get(self.active_preset).cloned().unwrap_or_default();
            // Tiledad del perfil nuevo: cambia el tamaño de la banda de snap.
            self.tiledad = self.app.config_tiledad();
            self.root_menu = None; // un menú abierto puede quedar obsoleto
            self.menu_output_idx = None;
            // Config nueva (wallpaper, fuente, menú): todo puede repintarse.
            crate::screencopy::danar_todo(&mut self.app);
            // Refresca el wallpaper por salida: cada `OutputCtx` resuelve su
            // ruta y su `fit` por nombre del conector (override o global).
            for ctx in &mut self.outputs {
                ctx.wallpaper_path = self.app.config_wallpaper_path_for(&ctx.name);
                ctx.wallpaper_fit = self.app.config_wallpaper_fit_for(&ctx.name);
                // Siempre rearmar: la fuente puede ser color/gradiente/procedural
                // (sin ruta), donde el cambio está en otros campos del config y
                // no en `wallpaper_path`. Rebuild una vez por guardado es barato.
                ctx.wallpaper = None;
            }
            self.text = crate::text::TextRenderer::system(self.app.config_font_path().as_deref());
        }

        if let Some(ctl) = &self.ctl {
            while let Some(mut conn) = ctl.poll() {
                let reply = match conn.read_request() {
                    // El ciclo de zonas es estado del Cuerpo (DRM): lo atendemos
                    // acá, no en el Cerebro. Avanza al siguiente preset.
                    Ok(Some(CtlRequest::CycleZones)) => {
                        if !self.zone_presets.is_empty() {
                            self.active_preset =
                                (self.active_preset + 1) % self.zone_presets.len();
                            self.zones = self.zone_presets[self.active_preset].clone();
                            self.preset_hud_label = format!(
                                "Zonas · {}/{}",
                                self.active_preset + 1,
                                self.zone_presets.len()
                            );
                            self.preset_hud_until = Some(Instant::now() + HUD_DURATION);
                        }
                        CtlReply::Ok
                    }
                    Ok(Some(req)) => self.app.serve_ctl(req),
                    Ok(None) => continue,
                    Err(e) => CtlReply::Error(format!("{e}")),
                };
                let _ = conn.reply(&reply);
            }
        }

        // Protocolo mirada-aware: apps que aportan botones a su titlebar.
        if let Some(aware) = &self.aware {
            while let Some(mut conn) = aware.poll() {
                if let Ok(Some(req)) = conn.read_request() {
                    let reply = self.app.serve_aware(req);
                    let _ = conn.reply(&reply);
                }
            }
        }

        // Slide de transición de escritorios (Win+Tab modo Hyprland/Prezi): al
        // detectar que el escritorio activo cambió, arranca un slide; el render
        // (cada tick) lo anima por tiempo y `emit_windows` aplica el offset.
        if let Some((active, _)) = self.app.workspace_overview() {
            let foc = self.app.focused_output_index();
            if active != self.last_active_ws {
                // Animar SÓLO si el escritorio cambió en el MISMO monitor (Win+Tab
                // / switch-workspace). Si cambió porque el foco saltó a otro
                // monitor (mover el mouse), `active_index` cambia sin que ningún
                // monitor cambie de contenido — animar ahí causaba el parpadeo
                // «los contenidos se intercambian entre monitores».
                // Sólo el modo Hyprland usa el slide horizontal. En Prezi la
                // transición es el vuelo de cámara de la vista espacial (no
                // queremos las dos animaciones peleando); en Direct, salto seco.
                let modo = self.app.config_workspace_switch_mode();
                if foc == self.last_focused_output {
                    let dir = if active > self.last_active_ws { 1.0 } else { -1.0 };
                    match modo {
                        mirada_brain::WorkspaceSwitchMode::Hyprland => {
                            self.ws_slide = Some((self.start.elapsed().as_millis() as u32, dir));
                        }
                        mirada_brain::WorkspaceSwitchMode::Cube => {
                            // Captura los dos escritorios a textura y arranca el giro.
                            self.start_cube(self.last_active_ws, active, dir, foc);
                        }
                        _ => {}
                    }
                }
                self.last_active_ws = active;
            }
            self.last_focused_output = foc;
        }
        if let Some((start_ms, _)) = self.ws_slide {
            if self.start.elapsed().as_millis() as u32 >= start_ms + self.app.config_slide_ms() {
                self.ws_slide = None;
            }
        }
        // El cubo termina como el slide: al cumplirse `slide_ms`, suelta las
        // texturas de las caras y vuelve a la escena normal.
        if let Some(c) = &self.cube {
            if self.start.elapsed().as_millis() as u32 >= c.start_ms + self.app.config_slide_ms() {
                self.cube = None;
            }
        }

        // Vista espacial (Prezi): animación de zoom de apertura/cierre. Al
        // detectar el flanco de apertura arranca el zoom-OUT; al pedir cierre
        // (`overview_closing`) arranca el zoom-IN y, al terminar, baja la vista.
        {
            // Cierre ROBUSTO de Win+Tab: en vez de depender de capturar el evento
            // de release de Super (que a veces no llega como esperamos), SONDEAMOS
            // su estado cada tick. Apenas Super deja de estar sostenido, se salta
            // al resaltado y se cierra — así la vista nunca queda «pegada» tapando
            // el escritorio (que dejaba sin funcionar el resto de la UI).
            if self.app.overview_open && self.app.overview_via_wintab && !self.app.overview_closing {
                let super_held = self
                    .app
                    .keyboard
                    .as_ref()
                    .is_some_and(|kb| kb.modifier_state().logo);
                if !super_held {
                    self.app.overview_commit();
                }
            }
            // Win+Tab de Prezi en modo ENLAZADO: la vista espacial la pinta la
            // app, pero sólo el Cuerpo ve el release de Super. Al soltarlo, le
            // reenviamos el keybind sentinela de commit para que salte al destino
            // resaltado (mismo «sondeo cada tick» que arriba, pero hacia la app).
            if self.app.prezi_wintab_linked {
                let super_held = self
                    .app
                    .keyboard
                    .as_ref()
                    .is_some_and(|kb| kb.modifier_state().logo);
                if !super_held {
                    self.app.prezi_wintab_linked = false;
                    // DEBE coincidir con OVERVIEW_WINTAB_COMMIT en mirada-app-llimphi.
                    let ev = self.app.body.keybind("PreziWintabCommit");
                    self.app.brain_feed(ev);
                }
            }
            let now = self.start.elapsed().as_millis() as u32;
            let anim_ms = self.app.config_overview_anim_ms().max(1);
            if self.app.overview_open && !self.prev_overview_open {
                self.overview_anim = Some((now, true)); // recién abierta → zoom-out
            }
            self.prev_overview_open = self.app.overview_open;
            if self.app.overview_closing && !matches!(self.overview_anim, Some((_, false))) {
                self.overview_anim = Some((now, false)); // cierre → zoom-in
            }
            if let Some((start, opening)) = self.overview_anim {
                if now >= start.saturating_add(anim_ms) {
                    if opening {
                        self.overview_anim = None; // queda abierta, desplegada
                    } else {
                        // Zoom-in de cierre terminado: baja la vista.
                        self.app.overview_open = false;
                        self.app.overview_closing = false;
                        self.app.overview_via_wintab = false;
                        self.overview_anim = None;
                        self.prev_overview_open = false;
                    }
                }
            }
            // Mientras la vista esté abierta (o cerrándose) repintamos cada tick:
            // así ambos zooms FLUYEN y el frame final sí se dibuja.
            if self.app.overview_open {
                crate::screencopy::danar_todo(&mut self.app);
            }
        }

        // Fondo automático (slideshow): rota por la carpeta cada N segundos.
        let (wp_dir, wp_interval) = self.app.config_wallpaper_slideshow();
        if !wp_dir.is_empty() && wp_interval > 0 {
            let now = self.start.elapsed().as_millis() as u32;
            if wp_dir != self.wp_dir {
                self.wp_images = crate::list_wallpaper_images(&wp_dir);
                self.wp_dir = wp_dir;
                self.wp_index = 0;
                self.wp_next_switch_ms = now; // aplicar la primera ya
            }
            if !self.wp_images.is_empty() && now >= self.wp_next_switch_ms {
                let img = self.wp_images[self.wp_index % self.wp_images.len()].clone();
                self.wp_index = self.wp_index.wrapping_add(1);
                self.wp_next_switch_ms = now.saturating_add(wp_interval.saturating_mul(1000));
                let s = img.to_string_lossy().to_string();
                for ctx in &mut self.outputs {
                    ctx.wallpaper_path = Some(s.clone());
                    ctx.wallpaper = None; // se rearma en el próximo render
                }
                crate::screencopy::danar_todo(&mut self.app);
            }
        } else if !self.wp_dir.is_empty() {
            self.wp_dir.clear();
            self.wp_images.clear();
        }

        // Fade-in de apertura o glow de foco en curso: forzá repintar cada tick
        // para que las rampas FLUYAN aunque el cliente ya no mande frames nuevos
        // (el damage de `DrmCompositor` no ve el cambio de alfa/color por sí solo).
        if self.open_anim_active() || self.focus_anim_active() {
            crate::screencopy::danar_todo(&mut self.app);
        }
        // El cubo gira por tiempo (las caras son texturas fijas): forzá repintar
        // cada tick mientras dure, si no el damage no ve el giro y se congela.
        if self.cube.is_some() {
            crate::screencopy::danar_todo(&mut self.app);
        }

        // Wallpaper en video: gestioná el worker y tomá su último frame ANTES de
        // render (corre aunque la sesión esté en otra VT, para poder pausar).
        self.manage_video_wallpaper();
        // Wallpaper de marca animado (fondo por defecto vivo): late a ~20 fps.
        self.tick_animated_default();
        // Esquinas calientes: dispara la zona bajo el puntero si cumplió el
        // reposo (acá, no en el motion, para medirlo aun con el cursor quieto).
        self.tick_hot_corners();

        self.render();
        let _ = self.display.flush_clients();
    }
}
