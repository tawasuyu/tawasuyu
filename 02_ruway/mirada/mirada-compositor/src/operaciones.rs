// Implementación de App — operaciones del compositor.
use crate::*;

impl App {
    /// La layer surface **interactiva** (capas Overlay/Top — p. ej. las barras de
    /// `pata`) bajo el punto físico `(x, y)`, con el origen de su geometría (para
    /// las coords locales del puntero). Las capas Bottom/Background NO reciben
    /// puntero (son fondo, como swaybg). `None` si no hay ninguna ahí. Lo usa el
    /// ruteo del puntero para que los clicks lleguen a las barras, no sólo a las
    /// ventanas.
    pub(crate) fn layer_under(&self, x: f64, y: f64) -> Option<(WlSurface, Point<f64, Logical>)> {
        let output = self.output.as_ref()?;
        let map = layer_map_for_output(output);
        for kind in [Layer::Overlay, Layer::Top] {
            if let Some(layer) = map.layer_under(kind, (x, y)) {
                let geo = map.layer_geometry(layer)?;
                return Some((
                    layer.wl_surface().clone(),
                    Point::from((geo.loc.x as f64, geo.loc.y as f64)),
                ));
            }
        }
        None
    }

    /// La layer surface bajo `(x, y)` que **acepta foco de teclado** (OnDemand o
    /// Exclusive), para enfocarla al clickearla — el cabezal de shuma de `pata`
    /// pide `OnDemand` y, al desplegar el drawer, `Exclusive`. `None` si la layer
    /// de abajo no quiere teclado (o no hay ninguna).
    pub(crate) fn keyboard_focusable_layer_under(&self, x: f64, y: f64) -> Option<WlSurface> {
        let output = self.output.as_ref()?;
        let map = layer_map_for_output(output);
        for kind in [Layer::Overlay, Layer::Top] {
            if let Some(layer) = map.layer_under(kind, (x, y)) {
                return layer
                    .can_receive_keyboard_focus()
                    .then(|| layer.wl_surface().clone());
            }
        }
        None
    }

    /// La layer surface (Overlay/Top, top-most) que reclama teclado **Exclusive**,
    /// si hay alguna. Mientras exista, el foco-sigue-ratón NO le roba el teclado
    /// (el drawer Quake de `pata` lo necesita para que escribas sin que mover el
    /// mouse sobre una ventana le quite el foco).
    pub(crate) fn exclusive_layer_surface(&self) -> Option<WlSurface> {
        let output = self.output.as_ref()?;
        let map = layer_map_for_output(output);
        for kind in [Layer::Overlay, Layer::Top] {
            if let Some(layer) = map.layers_on(kind).rev().find(|l| {
                l.cached_state().keyboard_interactivity == KeyboardInteractivity::Exclusive
            }) {
                return Some(layer.wl_surface().clone());
            }
        }
        None
    }

    /// La superficie de la ventana visible bajo el puntero (front-to-back: el
    /// shell gana a las flotantes, y éstas a las teseladas), si la hay. Espeja
    /// el hit-test de `DrmState::window_at` pero usando sólo campos de `App`
    /// (sin la lista de salidas del backend) — lo necesitan los handlers de
    /// restricción de puntero, que sólo ven `App`. La altura de salida se toma
    /// de `output_size` (correcta en monitor único; aproximada en multi-monitor,
    /// suficiente para decidir activación de un lock).
    pub(crate) fn surface_under_pointer(&self) -> Option<WlSurface> {
        let (x, y) = self.pointer_loc;
        let output_h = self.output_size.1;
        let tbh = self.decorations.titlebar_height;
        let mut idx: Vec<usize> = (0..self.windows.len())
            .filter(|&i| self.windows[i].visible)
            .collect();
        idx.sort_by_key(|&i| {
            let w = &self.windows[i];
            (!w.is_shell, !w.floating, !w.focused)
        });
        idx.into_iter().find_map(|i| {
            let w = &self.windows[i];
            let tb = crate::titlebar_for(w, tbh);
            let (lx, ly) = crate::render_loc(w, output_h, tbh);
            let (sw, sh) =
                crate::surface_px_size(w).unwrap_or((w.size.0, (w.size.1 - tb).max(1)));
            (x >= lx as f64 && y >= ly as f64 && x < (lx + sw) as f64 && y < (ly + sh) as f64)
                .then(|| w.surface.clone())
        })
    }

    /// Una layer (Overlay/Top, top-most) que acepta foco de teclado, **sin**
    /// mirar la posición del puntero — a diferencia de
    /// `keyboard_focusable_layer_under`. El shell-barra (`shuma`/`pata` en modo
    /// dock) se monta como wlr-layer-shell con `OnDemand`, así que NO es un
    /// toplevel y `keyboard_fallback_target` no lo encontraría entre `windows`.
    /// Esto lo cubre: es el destino de teclado del escritorio vacío cuando el
    /// shell es un layer-shell. `None` si ninguna layer quiere teclado (sólo
    /// hay wallpaper Background/None, p. ej.).
    pub(crate) fn keyboard_focusable_shell_layer(&self) -> Option<WlSurface> {
        let output = self.output.as_ref()?;
        let map = layer_map_for_output(output);
        for kind in [Layer::Overlay, Layer::Top] {
            if let Some(layer) = map
                .layers_on(kind)
                .rev()
                .find(|l| l.can_receive_keyboard_focus())
            {
                return Some(layer.wl_surface().clone());
            }
        }
        None
    }

    /// El destino de teclado cuando ninguna ventana reclama el foco: la
    /// ventana que el Cerebro marcó enfocada o —si no hay ninguna— el shell
    /// (`pata`), para poder tipear directo en su barra sin clickear primero.
    /// Si el shell no es un toplevel sino un **layer-shell** (`shuma`/`pata`
    /// en modo barra), cae a esa layer. `None` sólo si no hay ni ventana ni
    /// layer que acepte teclado (p. ej. greeter). Así, en un escritorio vacío,
    /// el teclado va al shell y winit le da hasta el auto-repeat de cada tecla.
    pub(crate) fn keyboard_fallback_target(&self) -> Option<WlSurface> {
        self.windows
            .iter()
            .find(|w| w.focused)
            .or_else(|| self.windows.iter().find(|w| w.is_shell && w.visible))
            .map(|w| w.surface.clone())
            .or_else(|| self.keyboard_focusable_shell_layer())
    }

    /// Reconcilia el foco del teclado con las layers Exclusive. Una layer que
    /// reclama `Exclusive` (el drawer Quake de `pata` abierto) debe **tener**
    /// el foco — antes lo conseguía sólo si la barra era `OnDemand` y la
    /// clickeabas; ahora se lo damos al volverse Exclusive, sin depender del
    /// click. Al soltar Exclusive (drawer cerrado o destruido) se lo
    /// devolvemos a la ventana que el Cerebro marcó enfocada, así una app
    /// recién lanzada recibe el teclado. Idempotente: sólo toca `set_focus`
    /// si el foco cambia, y nunca le roba el foco a una ventana (eso lo maneja
    /// el Cerebro vía `BodyOp::Focus`).
    pub(crate) fn reconcile_layer_keyboard(&mut self) {
        let Some(kb) = self.keyboard.clone() else {
            return;
        };
        let current = kb.current_focus();
        match self.exclusive_layer_surface() {
            Some(surf) => {
                if current.as_ref() != Some(&surf) {
                    kb.set_focus(self, Some(surf), SERIAL_COUNTER.next_serial());
                }
            }
            None => {
                // Si el foco ya está en una de nuestras ventanas, no lo tocamos
                // (manda el Cerebro). Sólo actuamos si quedó colgado en una
                // layer que ya no es Exclusive.
                let on_window = current
                    .as_ref()
                    .is_some_and(|s| self.windows.iter().any(|w| &w.surface == s));
                if !on_window {
                    let target = self.keyboard_fallback_target();
                    if current != target {
                        kb.set_focus(self, target, SERIAL_COUNTER.next_serial());
                    }
                }
            }
        }
    }

    /// Inyecta un evento del Cuerpo en el Cerebro y aplica su respuesta.
    pub(crate) fn brain_feed(&mut self, event: BodyEvent) {
        let cmds = match &mut self.brain {
            Brain::Embedded(desktop) => desktop.on_event(event),
            Brain::Linked(link) => {
                let _ = link.send(&event);
                Vec::new()
            }
        };
        self.apply_commands(cmds);
    }

    /// Drena los comandos de un Cerebro enlazado (no hace nada si es embebido).
    pub(crate) fn brain_poll(&mut self) {
        let cmds = match &self.brain {
            Brain::Linked(link) => link.drain(),
            Brain::Embedded(_) => Vec::new(),
        };
        if !cmds.is_empty() {
            self.apply_commands(cmds);
        }
    }

    /// Enfoca/activa la ventana `id` —pedido de la taskbar por
    /// `zwlr_foreign_toplevel_handle.activate`—. Va por el Cerebro embebido (con
    /// uno enlazado, el dueño externo decide). El `Focus` resultante reemite el
    /// estado a los handles wlr, así la barra resalta la activa.
    pub(crate) fn activar_ventana(&mut self, id: u64) {
        let cmds = match &mut self.brain {
            Brain::Embedded(d) => d.apply(mirada_brain::DesktopAction::FocusWindow(id)),
            Brain::Linked(_) => return,
        };
        self.apply_commands(cmds);
    }

    /// El escritorio activo sigue al monitor bajo el puntero (DM-sigue-mouse):
    /// al mover el mouse a otro monitor, ese pasa a ser la salida enfocada, así
    /// las ventanas nuevas y los cambios de escritorio van ahí.
    pub(crate) fn follow_pointer_output(&mut self) {
        let (x, y) = self.pointer_loc;
        if let Brain::Embedded(d) = &mut self.brain {
            d.focus_output_at(x as i32, y as i32);
        }
    }

    /// `true` si el Cerebro es **embebido** (no enlazado). El overview del
    /// compositor (Super+e local + `emit_overview`) sólo aplica embebido —
    /// `overview_data()` es `None` enlazado—; con Cerebro enlazado el dueño
    /// externo (mirada-app) tiene su propio overview y le reenviamos el atajo.
    pub(crate) fn brain_is_embedded(&self) -> bool {
        matches!(self.brain, Brain::Embedded(_))
    }

    /// El modo de transición de Win+Tab configurado (`direct`/`hyprland`/
    /// `prezi`). `Direct` con Cerebro enlazado (no decide animaciones acá).
    pub(crate) fn config_workspace_switch_mode(&self) -> mirada_brain::WorkspaceSwitchMode {
        match &self.brain {
            Brain::Embedded(d) => d.config().workspace_switch_mode,
            // Enlazado: el Cerebro nos manda `slide_ms` ya resuelto (`0` = sin
            // animación). Lo traducimos a un modo que dispare (o no) el slide.
            Brain::Linked(_) => match self.linked_ws.as_ref().map_or(0, |w| w.slide_ms) {
                0 => mirada_brain::WorkspaceSwitchMode::Direct,
                _ => mirada_brain::WorkspaceSwitchMode::Hyprland,
            },
        }
    }

    /// Duración (ms) del slide entre escritorios, de la config (default 220).
    /// `0` = salto seco. Con Cerebro enlazado: el default.
    pub(crate) fn config_slide_ms(&self) -> u32 {
        match &self.brain {
            Brain::Embedded(d) => d.config().slide_ms,
            // Enlazado: el `slide_ms` que empujó el Cerebro (0 hasta el 1er push).
            Brain::Linked(_) => self.linked_ws.as_ref().map_or(0, |w| w.slide_ms),
        }
    }

    /// Win+Tab estilo switcher sobre la vista espacial: abre la vista (si hacía
    /// falta) y mueve el resaltado al escritorio siguiente/anterior. La primera
    /// pulsación ya avanza uno (un Win+Tab suelto = saltar al siguiente al
    /// soltar Super). El salto real lo hace [`Self::overview_commit`] al soltar.
    pub(crate) fn overview_step(&mut self, forward: bool) {
        let Some((active, loads)) = self.workspace_overview() else {
            return;
        };
        // La RUEDA de Tab sólo recorre escritorios CON ventanas — los vacíos se
        // ven en el mapa pero no se navegan (saltar a uno vacío no aporta).
        let occupied: Vec<usize> = (0..loads.len()).filter(|&i| loads[i] > 0).collect();
        if occupied.is_empty() {
            return;
        }
        if !self.overview_open {
            self.overview_open = true;
            self.overview_closing = false;
            self.overview_via_wintab = true;
            self.overview_selected = active;
        }
        let n = occupied.len();
        // Posición del resaltado dentro de la rueda de ocupados (si el resaltado
        // cae en un vacío —p. ej. el activo está vacío— arrancamos del extremo).
        let next = match occupied.iter().position(|&w| w == self.overview_selected) {
            Some(p) if forward => (p + 1) % n,
            Some(p) => (p + n - 1) % n,
            None if forward => 0,
            None => n - 1,
        };
        self.overview_selected = occupied[next];
    }

    /// Confirma la navegación de la vista espacial: salta al escritorio
    /// resaltado y pide el cierre (zoom-in animado hacia él).
    pub(crate) fn overview_commit(&mut self) {
        if self.overview_open {
            self.cambiar_workspace(self.overview_selected);
            self.overview_closing = true;
        }
    }

    /// Duración (ms) del vuelo de cámara (zoom) de la vista espacial (Prezi).
    pub(crate) fn config_overview_anim_ms(&self) -> u32 {
        match &self.brain {
            Brain::Embedded(d) => d.config().overview_anim_ms,
            Brain::Linked(_) => 260,
        }
    }

    /// `(escritorio activo 0-based, ventanas por escritorio)` del Cerebro
    /// embebido — para el switcher visual de Win+Tab. `None` con Cerebro
    /// enlazado (el dueño externo maneja los escritorios).
    pub(crate) fn workspace_overview(&self) -> Option<(usize, Vec<usize>)> {
        match &self.brain {
            Brain::Embedded(d) => Some((d.active_index(), d.workspace_loads())),
            // Enlazado: el estado que empujó el Cerebro (`SetWorkspaces`).
            Brain::Linked(_) => self.linked_ws.as_ref().map(|w| (w.active, w.loads.clone())),
        }
    }

    /// Índice de la salida enfocada (Cerebro embebido; `0` si enlazado). Sirve
    /// para distinguir un **cambio de escritorio** (Win+Tab, anima) de un mero
    /// **cambio de monitor enfocado** (mover el mouse, NO anima).
    pub(crate) fn focused_output_index(&self) -> usize {
        match &self.brain {
            Brain::Embedded(d) => d.focused_output(),
            Brain::Linked(_) => 0,
        }
    }

    /// Datos para pintar la **vista espacial (Prezi)** en vivo: el escritorio
    /// activo, la geometría 2D (celda por escritorio), el rect de referencia y
    /// los rects de ventanas de cada escritorio (en ese rect, para normalizar a
    /// la miniatura). `None` con Cerebro enlazado.
    pub(crate) fn overview_data(&self) -> Option<OverviewData> {
        let Brain::Embedded(d) = &self.brain else {
            return None;
        };
        let work = d.overview_rect(mirada_brain::Rect::new(
            0,
            0,
            self.output_size.0.max(1),
            self.output_size.1.max(1),
        ));
        let loads = d.workspace_loads();
        let layouts = d
            .workspace_layouts(work)
            .into_iter()
            .map(|ws| ws.iter().filter(|p| p.visible).map(|p| (p.id, p.rect)).collect())
            .collect();
        Some(OverviewData {
            active: d.active_index(),
            places: d.config().overview_places_for(loads.len()),
            loads,
            work,
            layouts,
        })
    }

    /// Maximiza/restaura la ventana `id` (botón □ del titlebar): la enfoca y
    /// togglea su pantalla completa. Por el Cerebro embebido.
    pub(crate) fn maximizar_ventana(&mut self, id: u64) {
        let cmds = match &mut self.brain {
            Brain::Embedded(d) => {
                // Enfoca la ventana (la trae a la salida si está en otro
                // escritorio) y luego alterna MAXIMIZAR — flotar a toda el área
                // de trabajo conservando la barra de título, así el mismo botón
                // restaura. (Antes usaba ToggleFullscreen: quitaba la barra → sin
                // botón para volver, y "se apropiaba" del escritorio.)
                let mut c = d.apply(mirada_brain::DesktopAction::FocusWindow(id));
                c.extend(d.apply(mirada_brain::DesktopAction::ToggleMaximize));
                c
            }
            Brain::Linked(_) => return,
        };
        self.apply_commands(cmds);
    }

    /// Minimiza la ventana `id` mandándola al scratchpad (≈ ocultar). Botón ─
    /// del titlebar. Por el Cerebro embebido.
    pub(crate) fn minimizar_ventana(&mut self, id: u64) {
        let cmds = match &mut self.brain {
            Brain::Embedded(d) => {
                let mut c = d.apply(mirada_brain::DesktopAction::FocusWindow(id));
                c.extend(d.apply(mirada_brain::DesktopAction::SendToScratchpad));
                c
            }
            Brain::Linked(_) => return,
        };
        self.apply_commands(cmds);
    }

    /// Ejecuta una acción del **menú contextual de ventana** (click derecho en
    /// el titlebar) sobre la ventana `id`. `cmd` es el sufijo tras `@win:`
    /// (`close` / `max` / `min` / `float` / `ws:<n>`). Enfoca la ventana antes
    /// de aplicar para que las acciones del Cerebro caigan sobre ella.
    pub(crate) fn accion_ventana_menu(&mut self, id: u64, cmd: &str) {
        use mirada_brain::DesktopAction;
        if cmd == "close" {
            if let Some(w) = self.windows.iter().find(|w| w.id == id) {
                w.toplevel.send_close();
            }
            return;
        }
        if cmd == "max" {
            self.maximizar_ventana(id);
            return;
        }
        if cmd == "min" {
            self.minimizar_ventana(id);
            return;
        }
        let extra = if cmd == "float" {
            DesktopAction::ToggleFloat
        } else if let Some(n) = cmd.strip_prefix("ws:").and_then(|s| s.parse::<usize>().ok()) {
            DesktopAction::MoveToWorkspace(n)
        } else {
            return;
        };
        let cmds = match &mut self.brain {
            Brain::Embedded(d) => {
                let mut c = d.apply(DesktopAction::FocusWindow(id));
                c.extend(d.apply(extra));
                c
            }
            Brain::Linked(_) => return,
        };
        self.apply_commands(cmds);
    }

    /// Cambia al escritorio `idx` (0-based) — confirmación del switcher de
    /// Win+Tab. Por el Cerebro embebido.
    pub(crate) fn cambiar_workspace(&mut self, idx: usize) {
        let cmds = match &mut self.brain {
            Brain::Embedded(d) => d.apply(mirada_brain::DesktopAction::SwitchWorkspace(idx)),
            // Enlazado: el dueño externo cambia el escritorio; le mandamos el
            // salto y él reenvía el `SetWorkspaces` actualizado.
            Brain::Linked(link) => {
                let _ = link.send(&BodyEvent::SwitchWorkspace(idx as u32));
                return;
            }
        };
        self.apply_commands(cmds);
    }

    /// Atiende una petición del API de control (`mirada-ctl`).
    pub(crate) fn serve_ctl(&mut self, req: CtlRequest) -> CtlReply {
        match req {
            CtlRequest::Do(action) => {
                let cmds = match &mut self.brain {
                    Brain::Embedded(d) => Some(d.apply(action)),
                    Brain::Linked(_) => None,
                };
                match cmds {
                    Some(cmds) => {
                        self.apply_commands(cmds);
                        CtlReply::Ok
                    }
                    None => CtlReply::Error(
                        "el Cerebro es externo; usa mirada-ctl contra la app mirada".into(),
                    ),
                }
            }
            CtlRequest::ListWindows => match &self.brain {
                Brain::Embedded(d) => CtlReply::Windows(d.window_lines()),
                Brain::Linked(_) => CtlReply::Error("el Cerebro es externo".into()),
            },
            CtlRequest::Workspaces => match &self.brain {
                Brain::Embedded(d) => CtlReply::Workspaces(mirada_brain::WorkspacesState {
                    // `active_index` es 0-based; lo publicamos 1-based para casar
                    // con `workspace N` y los atajos `Super+1..9`.
                    active: d.active_index() + 1,
                    loads: d.workspace_loads(),
                    layout: mirada_brain::layout_slug(d.active_workspace().params().mode)
                        .to_string(),
                    on_other_outputs: d.workspaces_on_other_outputs(),
                }),
                Brain::Linked(_) => CtlReply::Error("el Cerebro es externo".into()),
            },
            // El ciclo de zonas lo intercepta el bucle de control del backend
            // DRM (las zonas son estado del Cuerpo). Si llega aquí (p. ej. en
            // winit, sin zonas), es un no-op.
            CtlRequest::CycleZones => CtlReply::Ok,
        }
    }

    /// Recarga el keymap del usuario en caliente. Conserva el anterior si
    /// el archivo nuevo es inválido. No-op con el Cerebro enlazado (el
    /// keymap es asunto suyo). Lo dispara [`ConfigWatches::poll`].
    pub(crate) fn reload_keymap_from(&mut self, path: &std::path::Path) {
        match Keymap::load(path) {
            Ok(mut km) => {
                // Mismo auto-curado que `load_or_init`: un keymap.ron recargado
                // en caliente (al cambiar de vista, p. ej.) puede venir
                // incompleto; `merge_defaults` rellena los binds que falten sin
                // pisar los propios. Sin esto, una vista que escribe un keymap
                // ralo dejaba los atajos sin grabs → «los shortcuts no andan».
                km.merge_defaults();
                let cmd = if let Brain::Embedded(d) = &mut self.brain {
                    Some(d.set_keymap(km))
                } else {
                    None
                };
                if let Some(cmd) = cmd {
                    self.apply_commands(vec![cmd]);
                    println!("mirada-compositor · keymap recargado.");
                }
            }
            Err(e) => {
                eprintln!("mirada-compositor · keymap inválido, conservo el anterior: {e}")
            }
        }
    }

    /// Recarga la config general (dropterm, teselado, foco, marco) en
    /// caliente y re-envía la decoración. Conserva la anterior si es
    /// inválida. No-op con el Cerebro enlazado.
    pub(crate) fn reload_config_from(&mut self, path: &std::path::Path) {
        match mirada_brain::Config::load(path) {
            Ok(cfg) => {
                let cmds = if let Brain::Embedded(d) = &mut self.brain {
                    d.reload_config(cfg)
                } else {
                    Vec::new()
                };
                if !cmds.is_empty() {
                    self.apply_commands(cmds);
                    println!("mirada-compositor · config recargada.");
                }
            }
            Err(e) => {
                eprintln!("mirada-compositor · config inválida, conservo la anterior: {e}")
            }
        }
    }

    /// Recarga las reglas de ventana en caliente. Aplican a las ventanas
    /// que se abran a partir de ahora; las ya abiertas no se tocan.
    /// Conserva las anteriores si son inválidas. No-op con Cerebro enlazado.
    pub(crate) fn reload_rules_from(&mut self, path: &std::path::Path) {
        match Rules::load(path) {
            Ok(rules) => {
                if let Brain::Embedded(d) = &mut self.brain {
                    d.set_rules(rules);
                    println!("mirada-compositor · reglas recargadas (aplican a ventanas nuevas).");
                }
            }
            Err(e) => {
                eprintln!("mirada-compositor · reglas inválidas, conservo las anteriores: {e}")
            }
        }
    }

    /// La ruta de fuente configurada (para las etiquetas del compositor), si
    /// el Cerebro es embebido y la config la fija. Vacía/None → se prueban
    /// las fuentes comunes del sistema.
    pub(crate) fn config_font_path(&self) -> Option<String> {
        match &self.brain {
            Brain::Embedded(d) => {
                let p = d.config().font_path.clone();
                (!p.is_empty()).then_some(p)
            }
            Brain::Linked(_) => None,
        }
    }

    /// La ruta del wallpaper configurado para la salida `name` (conector DRM:
    /// `HDMI-A-1`, `DP-1`, …) — el override de [`mirada_brain::OutputOverride`]
    /// si existe, o el global. `None` con Cerebro enlazado o si todo queda
    /// vacío (fondo de color sólido).
    /// `(carpeta, segundos)` del fondo automático (slideshow). Carpeta vacía o
    /// intervalo 0 = sin rotar.
    pub(crate) fn config_wallpaper_slideshow(&self) -> (String, u32) {
        match &self.brain {
            Brain::Embedded(d) => {
                let c = d.config();
                (c.wallpaper_dir.clone(), c.wallpaper_interval_secs)
            }
            Brain::Linked(_) => (String::new(), 0),
        }
    }

    pub(crate) fn config_wallpaper_path_for(&self, name: &str) -> Option<String> {
        match &self.brain {
            Brain::Embedded(d) => {
                let p = d.config().wallpaper_path_for(name).to_string();
                (!p.is_empty()).then_some(p)
            }
            Brain::Linked(_) => None,
        }
    }

    /// Resuelve la **fuente** del fondo de la salida `name` a un
    /// [`WallpaperSpec`] materializable. `ctx_path` es la ruta que la salida
    /// tiene cargada ahora (la global/override de la config, ya posiblemente
    /// pisada por el slideshow o el daemon remoto) — se usa para las fuentes de
    /// imagen. Las fuentes generadas (color/gradiente/procedural) ignoran la
    /// ruta. Con Cerebro enlazado cae a imagen-por-ruta o al default.
    pub(crate) fn config_wallpaper_spec_for(
        &self,
        name: &str,
        ctx_path: Option<&str>,
    ) -> crate::estado::WallpaperSpec {
        use crate::estado::WallpaperSpec;
        let img_or_default = |fit: mirada_brain::WallpaperFit| match ctx_path {
            Some(p) if !p.is_empty() => WallpaperSpec::Image(p.to_string(), fit),
            _ => WallpaperSpec::Default,
        };
        let Brain::Embedded(d) = &self.brain else {
            return img_or_default(mirada_brain::WallpaperFit::default());
        };
        let c = d.config();
        let fit = c.wallpaper_fit_for(name);
        match c.wallpaper_source.as_str() {
            "color" => WallpaperSpec::Solid(c.wallpaper_color),
            "gradient" => WallpaperSpec::Gradient(c.wallpaper_gradient.clone()),
            "procedural" => WallpaperSpec::Procedural(
                mirada_procedural::Pattern::from_slug(&c.wallpaper_pattern).unwrap_or_default(),
                c.wallpaper_palette.clone(),
            ),
            // auto / local / directory / remote → imagen por la ruta resuelta.
            _ => img_or_default(fit),
        }
    }

    /// Cómo se ajusta el wallpaper a la salida `name` (stretch/fit/fill/…) —
    /// el override de [`mirada_brain::OutputOverride`] si existe, o el global.
    /// Con Cerebro enlazado cae al default (stretch) — es sólo cosmético, el
    /// Cerebro no toma decisiones sobre el fondo.
    pub(crate) fn config_wallpaper_fit_for(&self, name: &str) -> mirada_brain::WallpaperFit {
        match &self.brain {
            Brain::Embedded(d) => d.config().wallpaper_fit_for(name),
            Brain::Linked(_) => mirada_brain::WallpaperFit::default(),
        }
    }

    /// El `order` configurado para la salida `name` — `0` si no hay override
    /// o si el Cerebro está enlazado (toma decisiones de layout sólo el
    /// Cuerpo embebido). Las salidas se disponen por `(order, name)`
    /// ascendente; la de menor `order` queda primaria.
    pub(crate) fn config_output_order_for(&self, name: &str) -> i32 {
        match &self.brain {
            Brain::Embedded(d) => d.config().output_order_for(name),
            Brain::Linked(_) => 0,
        }
    }

    /// La disposición global de los monitores: horizontal (default) o
    /// vertical. Con Cerebro enlazado cae al default — la geometría del
    /// escritorio compuesto la decide el Cuerpo embebido.
    pub(crate) fn config_output_disposition(&self) -> mirada_brain::Disposicion {
        match &self.brain {
            Brain::Embedded(d) => d.config().output_disposition(),
            Brain::Linked(_) => mirada_brain::Disposicion::Horizontal,
        }
    }

    /// Preferencias de puntero/touchpad (libinput): `(natural_scroll,
    /// tap_to_click, pointer_speed)`. Las aplica el backend a cada dispositivo
    /// nuevo (`DeviceAdded`). Con Cerebro enlazado cae a defaults.
    pub(crate) fn input_prefs(&self) -> (bool, bool, f64) {
        match &self.brain {
            Brain::Embedded(d) => {
                let c = d.config();
                (c.natural_scroll, c.tap_to_click, c.pointer_speed)
            }
            Brain::Linked(_) => (false, true, 0.0),
        }
    }

    /// Escala HiDPI en 120-avos para la salida `name`: override si existe,
    /// si no `120` (100 % nativo). Con Cerebro enlazado: 100 %.
    pub(crate) fn config_output_scale_120_for(&self, name: &str) -> u32 {
        match &self.brain {
            Brain::Embedded(d) => d.config().output_scale_120_for(name),
            Brain::Linked(_) => 120,
        }
    }

    /// Transformación de scanout para la salida `name`: override si existe,
    /// si no [`Transform::Normal`]. Parsea el slug en su sitio.
    pub(crate) fn config_output_transform_for(&self, name: &str) -> Transform {
        let slug = match &self.brain {
            Brain::Embedded(d) => d.config().output_transform_for(name).to_string(),
            Brain::Linked(_) => "normal".to_string(),
        };
        transform_from_slug(&slug)
    }

    /// El árbol del menú raíz configurado (con submenús anidados). Vacío con
    /// Cerebro enlazado. Si el config persistido del usuario trae `menu: []`
    /// (lo que dejaba a la pantalla sin nada al click-derecho), caemos al
    /// menú default — Terminal/Navegador/Archivos + submenús Mirada y Sesión
    /// con fallbacks `||` que andan sin saber qué tiene instalado.
    pub(crate) fn config_menu(&self) -> Vec<crate::menu::MenuNode> {
        match &self.brain {
            Brain::Embedded(d) => {
                let cfg_menu = &d.config().menu;
                if cfg_menu.is_empty() {
                    mirada_brain::default_root_menu()
                        .iter()
                        .map(menu_node_from_entry)
                        .collect()
                } else {
                    cfg_menu.iter().map(menu_node_from_entry).collect()
                }
            }
            Brain::Linked(_) => Vec::new(),
        }
    }

    /// Las zonas de arrastre configuradas (fracciones de la salida). Vacío con
    /// Cerebro enlazado o sin zonas en la config.
    pub(crate) fn config_zones(&self) -> Vec<mirada_brain::ZoneFrac> {
        match &self.brain {
            Brain::Embedded(d) => {
                let cfg = d.config();
                // Si la config (vieja, o de un perfil) no trae zonas, caemos a
                // las de fábrica (mitades izq./der.) — así drag-to-zone SIEMPRE
                // existe, sin depender de regenerar config.ron.
                let zonas = if cfg.zones.is_empty() {
                    mirada_brain::default_zones()
                } else {
                    cfg.zones.clone()
                };
                zonas
                    .iter()
                    .map(|z| mirada_brain::ZoneFrac { x: z.x, y: z.y, w: z.w, h: z.h })
                    .collect()
            }
            Brain::Linked(_) => Vec::new(),
        }
    }

    /// Los presets de zonas adicionales (cada uno una lista de zonas). Vacío
    /// con Cerebro enlazado o sin presets en la config.
    pub(crate) fn config_zone_presets(&self) -> Vec<Vec<mirada_brain::ZoneFrac>> {
        match &self.brain {
            Brain::Embedded(d) => d
                .config()
                .zone_presets
                .iter()
                .map(|set| {
                    set.iter()
                        .map(|z| mirada_brain::ZoneFrac { x: z.x, y: z.y, w: z.w, h: z.h })
                        .collect()
                })
                .collect(),
            Brain::Linked(_) => Vec::new(),
        }
    }

    /// Lanza `cmd` como el usuario de la sesión (igual que [`BodyOp::Spawn`]),
    /// salvo en modo greeter, donde no se lanza nada. Lo usa el menú raíz.
    pub(crate) fn spawn_user(&self, cmd: &str) {
        if self.mode == BodyMode::Greeter {
            eprintln!("mirada-compositor · «{cmd}» rechazado — modo greeter.");
            return;
        }
        spawn_command(cmd, self.session_user.as_ref(), &self.session_env);
    }

    /// Traduce los comandos del Cerebro a operaciones y las ejecuta.
    pub(crate) fn apply_commands(&mut self, cmds: Vec<BrainCommand>) {
        for cmd in cmds {
            match cmd {
                // El Cerebro enlazado empuja el estado de escritorios para el
                // switcher Win+Tab + slide; no produce BodyOps sobre superficies.
                BrainCommand::SetWorkspaces {
                    active,
                    loads,
                    slide_ms,
                } => {
                    self.linked_ws = Some(crate::estado::LinkedWorkspaces {
                        active: active as usize,
                        loads: loads.into_iter().map(|n| n as usize).collect(),
                        slide_ms,
                    });
                }
                other => {
                    for op in self.body.apply(other) {
                        self.exec_op(op);
                    }
                }
            }
        }
    }

    /// Ejecuta una operación concreta sobre las superficies reales.
    pub(crate) fn exec_op(&mut self, op: BodyOp) {
        match op {
            BodyOp::Configure { id, rect, visible, floating, fullscreen, suspended, frame_divisor } => {
                // La barra de título reserva una franja arriba: la superficie
                // del cliente se configura más baja por `tb` (no-shell, no
                // fullscreen). `w.size` guarda la celda entera; `render_loc`
                // baja la superficie por `tb`.
                let tbh = self.decorations.titlebar_height.max(0);
                let mut danio = None;
                if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
                    // La celda vieja y la nueva quedan dañadas (screencopy):
                    // mover/redimensionar/ocultar repinta ambas regiones.
                    let viejo: Rectangle<i32, Logical> =
                        Rectangle::new(w.loc.into(), w.size.into());
                    let nuevo = Rectangle::new((rect.x, rect.y).into(), (rect.w, rect.h).into());
                    if viejo != nuevo || w.visible != visible {
                        danio = Some(viejo.merge(nuevo));
                    }
                    w.loc = (rect.x, rect.y);
                    w.size = (rect.w, rect.h);
                    w.visible = visible;
                    w.floating = floating;
                    w.fullscreen = fullscreen;
                    w.suspended = suspended;
                    w.frame_divisor = frame_divisor.max(1);
                    let tb = if w.is_shell || fullscreen { 0 } else { tbh };
                    // Una ventana teselada (ni shell, ni flotante, ni fullscreen)
                    // recibe los estados `tiled`: así los clientes CSD (GTK/Qt)
                    // sueltan su margen de sombra flotante y las esquinas
                    // redondeadas — antes salían «forradas dentro de un margen
                    // grandísimo». Las flotantes conservan su decoración.
                    let teselada = !w.is_shell && !floating && !fullscreen;
                    w.toplevel.with_pending_state(|s| {
                        s.size = Some((rect.w.max(1), (rect.h - tb).max(1)).into());
                        if fullscreen {
                            s.states.set(xdg_toplevel::State::Fullscreen);
                        } else {
                            s.states.unset(xdg_toplevel::State::Fullscreen);
                        }
                        for st in [
                            xdg_toplevel::State::TiledLeft,
                            xdg_toplevel::State::TiledRight,
                            xdg_toplevel::State::TiledTop,
                            xdg_toplevel::State::TiledBottom,
                        ] {
                            if teselada {
                                s.states.set(st);
                            } else {
                                s.states.unset(st);
                            }
                        }
                    });
                    w.toplevel.send_pending_configure();
                }
                if let Some(d) = danio {
                    screencopy::danar(self, d);
                }
            }
            BodyOp::Focus(id) => {
                let mut target = None;
                let mut danios = Vec::new();
                for w in &mut self.windows {
                    let active = w.id == id;
                    if w.focused != active {
                        // El marco cambia de color: la celda queda dañada.
                        danios.push(Rectangle::new(w.loc.into(), w.size.into()));
                    }
                    w.focused = active;
                    if active {
                        target = Some(w.surface.clone());
                    }
                    w.toplevel.with_pending_state(|s| {
                        if active {
                            s.states.set(xdg_toplevel::State::Activated);
                        } else {
                            s.states.unset(xdg_toplevel::State::Activated);
                        }
                    });
                    w.toplevel.send_pending_configure();
                }
                for d in danios {
                    screencopy::danar(self, d);
                }
                if let Some(kb) = self.keyboard.clone() {
                    // Si la superficie destino aún no presentó buffer (ventana
                    // recién abierta, no mapeada), `set_focus` se perdería: el
                    // cliente puede no tener `wl_keyboard` bindeado todavía y el
                    // `enter` no llegaría —teclado mudo hasta abrir otra ventana.
                    // En ese caso diferimos el foco al primer commit con buffer
                    // (ver `handlers::commit`). Si ya está mapeada (re-foco por
                    // alt-tab/click), lo aplicamos al instante.
                    let mapeada = target
                        .as_ref()
                        .is_some_and(|s| surface_mapeada(s));
                    if target.is_none() || mapeada {
                        self.pending_kb_focus = None;
                        kb.set_focus(self, target, SERIAL_COUNTER.next_serial());
                    } else {
                        self.pending_kb_focus = target;
                    }
                }
                crate::foreign_toplevel::refrescar_estados(self);
            }
            BodyOp::Unfocus => {
                let mut danios = Vec::new();
                for w in &mut self.windows {
                    if w.focused {
                        danios.push(Rectangle::new(w.loc.into(), w.size.into()));
                    }
                    w.focused = false;
                }
                for d in danios {
                    screencopy::danar(self, d);
                }
                if let Some(kb) = self.keyboard.clone() {
                    // Sin ventana enfocada el teclado cae al shell (`pata`), no
                    // a la nada: el escritorio vacío sigue tipeable. Cualquier
                    // foco diferido pendiente queda obsoleto.
                    self.pending_kb_focus = None;
                    let target = self.keyboard_fallback_target();
                    kb.set_focus(self, target, SERIAL_COUNTER.next_serial());
                }
                crate::foreign_toplevel::refrescar_estados(self);
            }
            BodyOp::CloseClient(id) | BodyOp::KillClient(id) => {
                if let Some(w) = self.windows.iter().find(|w| w.id == id) {
                    w.toplevel.send_close();
                }
            }
            BodyOp::SetGrabs(keys) => {
                // Diagnóstico: cuántos atajos quedan registrados y si los
                // clásicos están entre ellos. Una línea en /tmp/mirada.log que
                // distingue «no se entregaron grabs» de «se entregaron pero el
                // combo no matchea» sin instrumentar nada más.
                println!(
                    "mirada-compositor · {} atajos registrados (Alt+Tab: {}, Super+q: {})",
                    keys.len(),
                    keys.iter().any(|k| k == "Alt+Tab"),
                    keys.iter().any(|k| k == "Super+q"),
                );
                self.grabs = keys;
            }
            BodyOp::SetCursor(_) => {}
            BodyOp::SetDecorations(d) => self.decorations = d,
            BodyOp::SetCapabilities(p) => *escribir_tolerante(&self.caps) = p,
            BodyOp::Spawn(cmd) => {
                // En modo greeter no se lanza nada: la pantalla de login
                // no es un sitio desde donde abrir programas.
                if self.mode == BodyMode::Greeter {
                    eprintln!("mirada-compositor · «{cmd}» rechazado — modo greeter.");
                } else {
                    spawn_command(&cmd, self.session_user.as_ref(), &self.session_env);
                }
            }
            BodyOp::Shutdown => self.running = false,
        }
    }

    /// Registra un toplevel recién creado y avisa al Cerebro.
    pub(crate) fn register_toplevel(&mut self, toplevel: ToplevelSurface) {
        let surface = toplevel.wl_surface().clone();
        let id = self.next_id;
        self.next_id += 1;

        let (app_id, title) = with_states(&surface, |states| {
            states
                .data_map
                .get::<XdgToplevelSurfaceData>()
                .and_then(|d| d.lock().ok())
                .map(|d| {
                    (
                        d.app_id.clone().unwrap_or_default(),
                        d.title.clone().unwrap_or_default(),
                    )
                })
                .unwrap_or_default()
        });
        // La ventana del shell (el marco pata) no se tesela: se acopla a un borde.
        let is_shell = is_shell_app_id(&app_id);

        // PID del cliente (lo guardó el accept-loop en `ClientState`) para el
        // linaje de las constelaciones — se lee ANTES de mover `surface` abajo.
        let client_pid = surface
            .client()
            .and_then(|c| c.get_data::<ClientState>().and_then(|s| s.pid));

        // Alta en el censo `ext_foreign_toplevel_list` — sólo ventanas del
        // usuario (el marco del shell no se anuncia a taskbars/switchers).
        let foreign_handle = (!is_shell).then(|| {
            self.foreign_toplevel_state
                .new_toplevel::<App>(title.clone(), app_id.clone())
        });

        self.windows.push(ManagedWindow {
            id,
            toplevel,
            surface,
            loc: (0, 0),
            size: (0, 0),
            visible: false,
            floating: false,
            focused: false,
            is_shell,
            fullscreen: false,
            suspended: false,
            frame_divisor: 1,
            frame_tick: 0,
            title: title.clone(),
            foreign_handle,
            wlr_handles: Vec::new(),
            borders: std::array::from_fn(|_| SolidColorBuffer::default()),
        });

        // Alta en el servidor wlr-foreign-toplevel (taskbar de pata): crea un
        // handle por manager bindeado. No-op para el shell o sin managers.
        crate::foreign_toplevel::anunciar_ventana(self, id);

        if is_shell {
            self.dock_shell();
        } else {
            let app_id = if app_id.is_empty() { "cliente".into() } else { app_id };
            let title = if title.is_empty() { format!("ventana {id}") } else { title };
            let ev = self.body.open_surface(id, app_id, title);
            self.brain_feed(ev);
            // Linaje de proceso para las constelaciones (best-effort): los
            // ancestros salen de /proc a partir del PID del cliente.
            if let Some(pid) = client_pid.filter(|&p| p > 0) {
                let ancestors = process_ancestors(pid);
                self.brain_feed(BodyEvent::WindowLineage {
                    id,
                    pid: pid as u32,
                    ancestors,
                });
            }
        }
    }

    /// Acopla la ventana del shell (el marco `pata`): reserva la zona exclusiva
    /// de su borde —el Cerebro tesela el resto, esquivándola— y dimensiona y
    /// coloca la franja ahí. Se llama al registrarla y al cambiar el tamaño de
    /// la salida. Funciona en cualquiera de los cuatro bordes: la reserva por
    /// insets desplaza y encoge el área útil sin tocar el tamaño físico.
    pub(crate) fn dock_shell(&mut self) {
        let (ow, oh) = self.output_size;
        if ow == 0 || oh == 0 {
            return; // la salida todavía no está lista
        }
        let dock = shell_dock();
        // El grosor no puede exceder el lado de la salida sobre el que recorta.
        let limite = if dock.anchor.es_horizontal() { oh } else { ow };
        let t = dock.thickness.clamp(1, limite.max(1));

        // Dimensiona la ventana del shell y la fija en la franja del borde.
        // Con autohide, su visibilidad la decide el puntero (estado actual).
        let visible = !(dock.autohide && self.shell_hidden);
        let mut danio = None;
        if let Some(w) = self.windows.iter_mut().find(|w| w.is_shell) {
            let (x, y, sw, sh) = shell_strip(dock.anchor, ow, oh, t);
            let viejo: Rectangle<i32, Logical> = Rectangle::new(w.loc.into(), w.size.into());
            let nuevo = Rectangle::new((x, y).into(), (sw, sh).into());
            if viejo != nuevo || w.visible != visible {
                danio = Some(viejo.merge(nuevo));
            }
            w.loc = (x, y);
            w.size = (sw, sh);
            w.visible = visible;
            w.toplevel.with_pending_state(|s| {
                s.size = Some((sw.max(1), sh.max(1)).into());
            });
            w.toplevel.send_pending_configure();
        }
        if let Some(d) = danio {
            screencopy::danar(self, d);
        }

        // La reserva del borde (franja pata + zonas exclusivas de
        // layer-shell) se computa en un solo lugar.
        self.recompute_reservations();
    }

    /// Recalcula y publica al Cerebro el área reservada del borde **por
    /// salida**: cada output reporta sus propias zonas exclusivas de layer
    /// surfaces (waybar, mako, swaybg…), y la primaria suma además la
    /// franja del shell (pata) si el dock está acoplado. Es la fuente única
    /// de los insets del teselado — el Brain ya soporta reservas distintas
    /// por `OutputId` (`Output.reserved`), así que un dock en monitor
    /// secundario no tapa las ventanas teseladas de ESE monitor.
    pub(crate) fn recompute_reservations(&mut self) {
        let dock = shell_dock();
        let has_shell = self.windows.iter().any(|w| w.is_shell);
        let outputs = self.outputs.clone();
        for (i, output) in outputs.iter().enumerate() {
            let Some(mode) = output.current_mode() else { continue };
            let (ow, oh) = (mode.size.w, mode.size.h);
            if ow == 0 || oh == 0 {
                continue;
            }
            let (mut top, mut bottom, mut left, mut right) = (0, 0, 0, 0);
            // Layer surfaces de ESTA salida (smithay las cuelga por output).
            let z = layer_map_for_output(output).non_exclusive_zone();
            top += z.loc.y.max(0);
            left += z.loc.x.max(0);
            right += (ow - (z.loc.x + z.size.w)).max(0);
            bottom += (oh - (z.loc.y + z.size.h)).max(0);
            // El dock pata vive sólo en la primaria (index 0). Autohide no
            // reserva: se superpone al revelarse, las ventanas usan todo.
            let is_primary = i == 0;
            if is_primary && has_shell && !dock.autohide {
                let limite = if dock.anchor.es_horizontal() { oh } else { ow };
                let t = dock.thickness.clamp(1, limite.max(1));
                let (st, sb, sl, sr) = shell_insets(dock.anchor, t);
                top += st;
                bottom += sb;
                left += sl;
                right += sr;
            }
            if is_primary {
                self.reserved = (top, bottom, left, right);
            }
            let ev = self.body.reserve_output(i as u32, top, bottom, left, right);
            self.brain_feed(ev);
        }
    }

    /// Con el dock autoescondido, ajusta su visibilidad según el puntero
    /// `(px, py)`: se revela al tocar la banda del borde anclado y se oculta al
    /// salir de su franja. Devuelve `true` si el estado cambió (el backend lo
    /// usa para recomponer). No-op sin autohide o sin dock acoplado.
    pub(crate) fn update_shell_autohide(&mut self, px: f64, py: f64) -> bool {
        let dock = shell_dock();
        if !dock.autohide {
            return false;
        }
        let (ow, oh) = self.output_size;
        if ow == 0 || oh == 0 || !self.windows.iter().any(|w| w.is_shell) {
            return false;
        }
        let limite = if dock.anchor.es_horizontal() { oh } else { ow };
        let t = dock.thickness.clamp(1, limite.max(1));
        let next = autohide_next_hidden(
            dock.anchor,
            ow,
            oh,
            t,
            px.round() as i32,
            py.round() as i32,
            self.shell_hidden,
            SHELL_REVEAL_BAND,
        );
        if next == self.shell_hidden {
            return false;
        }
        self.shell_hidden = next;
        let mut danio = None;
        if let Some(w) = self.windows.iter_mut().find(|w| w.is_shell) {
            w.visible = !next;
            danio = Some(Rectangle::new(w.loc.into(), w.size.into()));
        }
        if let Some(d) = danio {
            screencopy::danar(self, d);
        }
        true
    }

    /// El backend informa de un tamaño de salida nuevo (arranque o
    /// redimensión): fija el tamaño físico y, si hay shell acoplado, recalcula
    /// su franja (la reserva por insets se mantiene relativa al borde).
    pub(crate) fn output_changed(&mut self, width: i32, height: i32) {
        self.output_size = (width, height);
        // Cambió el modo: todo lo capturable queda dañado (screencopy).
        screencopy::danar_todo(self);
        // Mantené el Output (y su LayerMap) al día con el tamaño nuevo.
        if let Some(output) = self.output.clone() {
            output.change_current_state(
                Some(smithay::output::Mode {
                    size: (width, height).into(),
                    refresh: 60_000,
                }),
                None,
                None,
                None,
            );
            layer_map_for_output(&output).arrange();
        }
        let ev = self.body.resize_output(0, width, height);
        self.brain_feed(ev);
        if self.windows.iter().any(|w| w.is_shell) {
            self.dock_shell();
        } else {
            self.recompute_reservations();
        }
    }

    /// El traspaso del DM — la «mutación atómica». Llega el tiquet de un
    /// login válido y el compositor pasa de la pantalla de greeter a la
    /// sesión del usuario **sin reiniciar el servidor Wayland**: el mismo
    /// proceso, la misma GPU, las mismas ventanas. Idempotente — un
    /// segundo tiquet (no debería llegar) se ignora.
    pub(crate) fn complete_greeter_handoff(&mut self, ticket: SessionTicket) {
        if self.mode == BodyMode::Session {
            return; // ya en sesión — un tiquet de más, se ignora
        }
        println!(
            "mirada-compositor · traspaso a la sesión de «{}» (uid {}).",
            ticket.user.name, ticket.user.uid
        );
        if !nix::unistd::geteuid().is_root() {
            eprintln!(
                "mirada-compositor · aviso: no corro como root — la sesión \
                 heredará mis privilegios, sin setuid al usuario."
            );
        }
        self.mode = BodyMode::Session;
        self.session_user = Some(ticket.user.clone());

        // Ya en sesión: registra los atajos del escritorio y la decoración
        // (en modo greeter se omitieron a propósito — ver `build_app`).
        if let Brain::Embedded(desktop) = &self.brain {
            let cmds = vec![desktop.grab_keys(), desktop.decorations()];
            self.apply_commands(cmds);
        }

        // Arranca la sesión. Tres caminos:
        //  · vacío         → autostart del usuario (cliente de este compositor).
        //  · nativo (pata) → comando como cliente, sin reiniciar el servidor.
        //  · ajeno         → soltar el DRM y `exec` (otro compositor toma la
        //                    GPU). Se difiere al cierre del bucle: marcamos la
        //                    sesión pendiente y pedimos salir.
        let user = self.session_user.clone();
        // Prepara el entorno de sesión del usuario (runtime dir propio,
        // WAYLAND_DISPLAY absoluto, bus D-Bus) para que las apps nativas
        // —waybar, GTK/Qt— funcionen como en una sesión de verdad.
        if let Some(u) = &user {
            self.setup_user_session_env(u);
        }
        let env = self.session_env.clone();
        let cmd = ticket.session.trim();
        if cmd.is_empty() {
            spawn_autostart(user.as_ref(), &env);
        } else if ticket.foreign {
            println!(
                "mirada-compositor · sesión ajena «{cmd}» — cierro y cedo el DRM."
            );
            self.pending_session = Some((cmd.to_string(), user));
            self.running = false;
        } else {
            spawn_command(cmd, user.as_ref(), &env);
        }
    }

    /// Arma el entorno de sesión del usuario para las apps NATIVAS (clientes
    /// de este compositor): un `XDG_RUNTIME_DIR` propio y escribible
    /// (`/run/user/<uid>`), el `WAYLAND_DISPLAY` en ruta absoluta (el socket
    /// vive en el runtime dir del compositor, no en el del usuario) y un bus
    /// de sesión D-Bus. Sin esto, dconf no puede escribir y waybar/GTK/Qt
    /// fallan por «cannot autolaunch D-Bus».
    pub(crate) fn setup_user_session_env(&mut self, user: &UserInfo) {
        use std::os::unix::fs::PermissionsExt;
        let xrd = format!("/run/user/{}", user.uid);
        let _ = std::fs::create_dir_all(&xrd);
        let _ = std::fs::set_permissions(&xrd, std::fs::Permissions::from_mode(0o700));
        let _ = nix::unistd::chown(
            xrd.as_str(),
            Some(nix::unistd::Uid::from_raw(user.uid)),
            Some(nix::unistd::Gid::from_raw(user.gid)),
        );
        // El socket Wayland está en el runtime dir del COMPOSITOR (p. ej.
        // /run/mirada); WAYLAND_DISPLAY absoluto para que el cliente lo
        // encuentre aunque su XDG_RUNTIME_DIR sea otro.
        let wl = match (
            std::env::var("XDG_RUNTIME_DIR"),
            std::env::var("WAYLAND_DISPLAY"),
        ) {
            (Ok(rd), Ok(wd)) if !wd.starts_with('/') => format!("{rd}/{wd}"),
            (_, Ok(wd)) => wd,
            _ => String::new(),
        };
        let bus_path = format!("{xrd}/bus");
        let dbus_addr = format!("unix:path={bus_path}");
        // El socket de control vive en el runtime dir del COMPOSITOR (p. ej.
        // /run/mirada), no en el del usuario. Lo pasamos absoluto para que pata
        // y `mirada-ctl` de la sesión hablen con el Cerebro (sin esto, el
        // switcher de workspaces y el task-manager quedaban mudos).
        let ctl_sock = mirada_brain::ctl::default_socket_path().display().to_string();
        self.session_env = vec![
            ("XDG_RUNTIME_DIR".to_string(), xrd),
            ("WAYLAND_DISPLAY".to_string(), wl),
            ("DBUS_SESSION_BUS_ADDRESS".to_string(), dbus_addr.clone()),
            ("MIRADA_CTL_SOCK".to_string(), ctl_sock),
        ];
        // Levanta el bus de sesión D-Bus como el usuario, si no hay uno, y
        // espera (acotado) a que el socket exista: si lanzáramos waybar/GTK
        // antes, fallarían con «cannot autolaunch D-Bus». Es un bloqueo de
        // una sola vez al iniciar la sesión, no en el bucle de render.
        if !std::path::Path::new(&bus_path).exists() {
            let env = self.session_env.clone();
            spawn_command(
                &format!("dbus-daemon --session --address={dbus_addr} --nofork --nopidfile"),
                Some(user),
                &env,
            );
            for _ in 0..40 {
                if std::path::Path::new(&bus_path).exists() {
                    break;
                }
                std::thread::sleep(std::time::Duration::from_millis(25));
            }
            if std::path::Path::new(&bus_path).exists() {
                println!("mirada-compositor · bus D-Bus de sesión listo en {bus_path}.");
            } else {
                eprintln!(
                    "mirada-compositor · el bus D-Bus no apareció (¿dbus-daemon instalado?); las apps que lo exijan pueden fallar."
                );
            }
        }
    }
}
