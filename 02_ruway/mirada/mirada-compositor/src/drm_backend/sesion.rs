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
            eprintln!("mirada-compositor · libinput.resume falló.");
        }
        if let Err(e) = self.drm.activate(false) {
            eprintln!("mirada-compositor · drm.activate falló: {e}");
        }
        for ctx in &mut self.outputs {
            if let Err(e) = ctx.compositor.reset_state() {
                eprintln!(
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

    /// Tarea periódica: Cerebro enlazado, recarga del keymap, API de
    /// control, composición y vaciado hacia los clientes.
    pub(super) fn tick(&mut self) {
        self.app.brain_poll();

        let n = self.app.windows.len();
        if n != self.last_windows {
            eprintln!("mirada-compositor · ventanas en pantalla: {n}");
            self.last_windows = n;
        }

        // Recarga en caliente de keymap/config/reglas si cambiaron en disco.
        // Si la config general cambió, refresca las cachés que el Cuerpo deriva
        // de ella (menú raíz, wallpaper, fuente de etiquetas) — el Cerebro ya
        // aplicó teselado/decoración/foco.
        if self.watches.poll(&mut self.app) {
            self.menu_entries = self.app.config_menu();
            // Reconstruye los presets de zonas y reacota el activo.
            let mut presets = vec![self.app.config_zones()];
            presets.extend(self.app.config_zone_presets());
            self.zone_presets = presets;
            if self.active_preset >= self.zone_presets.len() {
                self.active_preset = 0;
            }
            self.zones = self.zone_presets.get(self.active_preset).cloned().unwrap_or_default();
            self.root_menu = None; // un menú abierto puede quedar obsoleto
            self.menu_output_idx = None;
            // Config nueva (wallpaper, fuente, menú): todo puede repintarse.
            crate::screencopy::danar_todo(&mut self.app);
            // Refresca el wallpaper por salida: cada `OutputCtx` resuelve su
            // ruta y su `fit` por nombre del conector (override o global).
            for ctx in &mut self.outputs {
                let new_wp = self.app.config_wallpaper_path_for(&ctx.name);
                let new_fit = self.app.config_wallpaper_fit_for(&ctx.name);
                if new_wp != ctx.wallpaper_path || new_fit != ctx.wallpaper_fit {
                    ctx.wallpaper_path = new_wp;
                    ctx.wallpaper_fit = new_fit;
                    ctx.wallpaper = None; // se rearma en el próximo render
                }
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
                if foc == self.last_focused_output
                    && self.app.config_workspace_switch_mode()
                        != mirada_brain::WorkspaceSwitchMode::Direct
                {
                    let dir = if active > self.last_active_ws { 1.0 } else { -1.0 };
                    self.ws_slide = Some((self.start.elapsed().as_millis() as u32, dir));
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

        self.render();
        let _ = self.display.flush_clients();
    }
}
