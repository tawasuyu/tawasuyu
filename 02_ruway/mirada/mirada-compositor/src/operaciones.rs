// Implementación de App — operaciones del compositor.
use crate::*;
use smithay::utils::IsAlive;

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

    /// El popup (menú de app) topmost bajo `(x, y)` y el origen GLOBAL de su
    /// superficie — para entregarle el puntero igual que a una ventana. `None`
    /// si el cursor no está sobre ningún popup. Recorre las ventanas en orden de
    /// pintado inverso (topmost primero) y, por cada una, su árbol de popups
    /// (submenús incluidos). Sin esto los clicks sobre un menú GTK irían a la
    /// ventana de atrás y el menú quedaría inerte.
    pub(crate) fn popup_under(&self, x: f64, y: f64) -> Option<(WlSurface, Point<f64, Logical>)> {
        let output_h = self.output_size.1;
        let tbh = self.decorations.titlebar_height;
        let mut order: Vec<usize> =
            (0..self.windows.len()).filter(|&i| self.windows[i].visible).collect();
        order.sort_by_key(|&i| {
            let w = &self.windows[i];
            (!w.is_shell, !w.floating, !w.focused)
        });
        for &i in order.iter().rev() {
            let w = &self.windows[i];
            let (gx, gy) = crate::render_loc(w, output_h, tbh);
            let (ox, oy) = crate::content_offset(w);
            if let Some(found) = popup_under_tree(&w.surface, (gx + ox, gy + oy), x, y) {
                return Some(found);
            }
        }
        None
    }

    /// `true` si hay algún popup (menú) abierto. Lo usa el handler de click para
    /// decidir si un click afuera debe cerrarlos.
    pub(crate) fn has_popups(&self) -> bool {
        self.windows.iter().any(|w| {
            smithay::desktop::PopupManager::popups_for_surface(&w.surface)
                .next()
                .is_some()
        })
    }

    /// La superficie del popup más PROFUNDO abierto (la hoja del árbol de menús
    /// = el submenú activo), o `None` si no hay ninguno. Sólo cuenta superficies
    /// vivas. Es a quien debe ir el foco de teclado para navegar el menú.
    pub(crate) fn topmost_popup_surface(&self) -> Option<WlSurface> {
        let mut best: Option<(usize, WlSurface)> = None;
        for w in &self.windows {
            deepest_popup(&w.surface, 0, &mut best);
        }
        best.map(|(_, s)| s)
    }

    /// Mantiene el foco de teclado sobre el menú abierto para que el cliente
    /// (GTK/Qt) lo navegue con flechas/Enter/Escape. Al abrirse el primer popup
    /// recuerda a quién devolvérselo; sigue el foco al submenú más profundo; y al
    /// cerrarse todos lo restaura. Se llama desde `grab`, `commit` (con menú
    /// activo) y `popup_destroyed`.
    pub(crate) fn reconcile_popup_keyboard(&mut self) {
        let Some(kb) = self.keyboard.clone() else {
            return;
        };
        match self.topmost_popup_surface() {
            Some(surface) => {
                if self.popup_saved_focus.is_none() {
                    // Primer popup del menú: recordamos el foco actual.
                    self.popup_saved_focus = Some(kb.current_focus());
                }
                if kb.current_focus().as_ref() != Some(&surface) {
                    kb.set_focus(self, Some(surface), smithay::utils::SERIAL_COUNTER.next_serial());
                }
            }
            None => {
                // El menú se cerró del todo: devolvemos el teclado a su dueño.
                if let Some(prev) = self.popup_saved_focus.take() {
                    kb.set_focus(self, prev, smithay::utils::SERIAL_COUNTER.next_serial());
                }
            }
        }
    }

    /// Cierra todos los popups abiertos mandándoles `popup_done` (el cliente
    /// destruye el menú). Se llama al click fuera de cualquier popup.
    pub(crate) fn dismiss_popups(&mut self) {
        let mut kinds = Vec::new();
        for w in &self.windows {
            collect_popup_kinds(&w.surface, &mut kinds);
        }
        for k in kinds {
            if let smithay::desktop::PopupKind::Xdg(p) = k {
                p.send_popup_done();
            }
        }
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
        // El shell de credenciales (login/lock) atrapa TODO el puntero: va
        // primero en el hit-test, igual que en el pintado (`is_greeter` al
        // frente). Sin esto el clic caía a la ventana de la sesión por debajo
        // —el lock «no bloqueaba nada»— porque sólo se ordenaba por shell/foco.
        idx.sort_by_key(|&i| {
            let w = &self.windows[i];
            (!w.is_greeter, !w.is_shell, !w.floating, !w.focused)
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
        // Miga para el contexto de un eventual crash: el último puñado de
        // eventos del Cuerpo suele explicar el panic mejor que el backtrace.
        crate::diag::miga(format!("evt {event:?}"));
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
        // Antes de drenar: si el Cerebro murió, intentá reconectar uno nuevo.
        self.reconcile_brain();
        let cmds = match &self.brain {
            Brain::Linked(link) => link.drain(),
            Brain::Embedded(_) => Vec::new(),
        };
        if !cmds.is_empty() {
            self.apply_commands(cmds);
        }
    }

    /// Si el Cerebro **enlazado** murió (reinicio a propósito o crash), re-acepta
    /// uno nuevo en el listener persistente y le re-sincroniza el mundo —sin
    /// tocar las conexiones Wayland de los clientes, que siguen vivas en el
    /// Cuerpo—. Mientras nadie reconecte, el Cuerpo **sigue componiendo el último
    /// frame**: las apps no se inmutan. No-op en modo embebido.
    ///
    /// Esto es lo que convierte un panic del Cerebro (layout/UX/plugins) de
    /// «perdés la sesión» a «un parpadeo»: un supervisor relanza el Cerebro y
    /// acá lo re-enganchamos.
    pub(crate) fn reconcile_brain(&mut self) {
        let muerto = matches!(&self.brain, Brain::Linked(link) if !link.is_alive());
        if !muerto {
            return;
        }
        let Some(server) = self.brain_server.as_ref() else {
            return;
        };
        let nuevo = match server.try_accept() {
            Ok(Some(link)) => link,
            Ok(None) => return, // nadie reconectó todavía; seguí componiendo
            Err(e) => {
                eprintln!("mirada-compositor · reconexión del Cerebro falló: {e}");
                return;
            }
        };
        println!("mirada-compositor · Cerebro reconectado — re-sincronizando estado.");
        self.brain = Brain::Linked(nuevo);
        // El primer `Place` del Cerebro fresco puede llegar con el modelo aún
        // incompleto (antes de digerir el censo): que no oculte nada por error.
        self.body.arm_suppress_next_hide();
        // Re-anunciá el mundo: salidas (del Cuerpo) + ventanas (de nuestro
        // registro, que tiene app_id/title). El Cerebro reconstruye su modelo y
        // vuelve a emitir `Place`; el shell re-asienta sus reservas al re-anclar.
        let mut censo = self.body.census_outputs();
        for w in &self.windows {
            if w.is_shell || w.is_greeter {
                continue;
            }
            censo.push(BodyEvent::WindowOpened {
                id: w.id,
                app_id: w.app_id.clone(),
                title: w.title.clone(),
            });
        }
        if let Brain::Linked(link) = &mut self.brain {
            for ev in censo {
                let _ = link.send(&ev);
            }
        }
        // Re-emití las reservas (zonas exclusivas del shell): re-derivadas del
        // layer_map, no dependen de que pata re-commitee. Tras el censo, así el
        // Cerebro fresco no tesela sobre la barra.
        self.recompute_reservations();
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
        let cmds = match &mut self.brain {
            Brain::Embedded(d) => {
                // Si el puntero cruzó a otro monitor, la salida activa lo siguió:
                // re-emitimos la colocación para que el FOCO DE TECLADO la siga
                // también. Sin esto, la salida activa cambiaba pero el teclado se
                // quedaba en el monitor anterior — escribías en la pantalla
                // equivocada o en ninguna (el síntoma «el 2º monitor pierde el
                // foco»). `refresh` es idempotente: si nada cambió, no hay BodyOps.
                if d.focus_output_at(x as i32, y as i32) {
                    d.refresh()
                } else {
                    Vec::new()
                }
            }
            Brain::Linked(_) => Vec::new(),
        };
        if !cmds.is_empty() {
            self.apply_commands(cmds);
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
    /// `prezi`/`cube`).
    pub(crate) fn config_workspace_switch_mode(&self) -> mirada_brain::WorkspaceSwitchMode {
        match &self.brain {
            Brain::Embedded(d) => d.config().workspace_switch_mode,
            // Enlazado: el modo REAL que empujó el Cerebro (`SetWorkspaces`).
            // Antes se inferría de `slide_ms` y colapsaba todo a Direct/Hyprland,
            // dejando Cube y Prezi inalcanzables en modo DE. `Direct` hasta el
            // primer push (no hay estado todavía).
            Brain::Linked(_) => self
                .linked_ws
                .as_ref()
                .map_or(mirada_brain::WorkspaceSwitchMode::Direct, |w| w.switch_mode),
        }
    }

    /// `true` si «reducir movimiento» (a11y) está activo: el compositor pone en
    /// cero todas las duraciones de animación. Sólo el Cerebro embebido lo
    /// conoce; enlazado cae a `false` (el dueño externo ya empuja 0 si quiere).
    pub(crate) fn config_reduce_motion(&self) -> bool {
        match &self.brain {
            Brain::Embedded(d) => d.config().reduce_motion,
            Brain::Linked(_) => false,
        }
    }

    /// Duración (ms) del slide entre escritorios, de la config (default 220).
    /// `0` = salto seco. Con Cerebro enlazado: el default. Cero si «reducir
    /// movimiento».
    pub(crate) fn config_slide_ms(&self) -> u32 {
        if self.config_reduce_motion() {
            return 0;
        }
        match &self.brain {
            Brain::Embedded(d) => d.config().slide_ms,
            // Enlazado: el `slide_ms` que empujó el Cerebro (0 hasta el 1er push).
            Brain::Linked(_) => self.linked_ws.as_ref().map_or(0, |w| w.slide_ms),
        }
    }

    /// Duración (ms) del fade-in de apertura de ventana (default 160). `0` =
    /// aparición seca. Con Cerebro enlazado cae al default; cero si «reducir
    /// movimiento». Lo lee el render para sellar y correr la rampa de alfa.
    pub(crate) fn config_window_open_ms(&self) -> u32 {
        if self.config_reduce_motion() {
            return 0;
        }
        match &self.brain {
            Brain::Embedded(d) => d.config().window_open_ms,
            Brain::Linked(_) => 160,
        }
    }

    /// Curva del fade-in de apertura. Default `EaseOutCubic` (= slide y Prezi).
    pub(crate) fn config_window_open_easing(&self) -> mirada_brain::Easing {
        match &self.brain {
            Brain::Embedded(d) => d.config().window_open_easing,
            Brain::Linked(_) => mirada_brain::Easing::default(),
        }
    }

    /// Escala inicial del «pop» de apertura como fracción (0.5–1.0). `1.0` = sin
    /// pop. Con Cerebro enlazado cae al default (0.92). El render la aplica
    /// envolviendo la ventana en un `RescaleRenderElement` durante el fade.
    pub(crate) fn config_window_open_scale(&self) -> f32 {
        let pct = match &self.brain {
            Brain::Embedded(d) => d.config().window_open_scale_pct,
            Brain::Linked(_) => 92,
        };
        (pct as f32 / 100.0).clamp(0.5, 1.0)
    }

    /// Duración (ms) del *glow* de foco (crossfade del marco al ganar/perder
    /// foco). `0` = cambio seco. Con Cerebro enlazado cae al default (140); cero
    /// si «reducir movimiento».
    pub(crate) fn config_focus_glow_ms(&self) -> u32 {
        if self.config_reduce_motion() {
            return 0;
        }
        match &self.brain {
            Brain::Embedded(d) => d.config().focus_glow_ms,
            Brain::Linked(_) => 140,
        }
    }

    /// Duración (ms) del fade al cerrar una ventana. `0` (default) = cierre seco
    /// y sin costo (no se captura nada). Con Cerebro enlazado cae a `0` (el
    /// dueño externo no participa de este motor). Cero si «reducir movimiento».
    pub(crate) fn config_window_close_ms(&self) -> u32 {
        if self.config_reduce_motion() {
            return 0;
        }
        match &self.brain {
            Brain::Embedded(d) => d.config().window_close_ms,
            Brain::Linked(_) => 0,
        }
    }

    /// Intensidad (fracción 0.0–0.8) del velo que atenúa las ventanas sin foco.
    /// `0.0` (default) = sin atenuar. No depende de «reducir movimiento» (es un
    /// aspecto, no un movimiento); su crossfade sí, vía `focus_glow_ms`.
    pub(crate) fn config_unfocused_dim(&self) -> f32 {
        let pct = match &self.brain {
            Brain::Embedded(d) => d.config().unfocused_dim_pct,
            Brain::Linked(_) => 0,
        };
        (pct as f32 / 100.0).clamp(0.0, 0.8)
    }

    // El radio de esquinas redondeadas ya no se lee de la config (que sólo
    // existe con Cerebro Embedded): viaja en `Decorations::corner_radius`, así
    // funciona también con el Cerebro enlazado. Ver `render`.

    /// Radio (px) del **blur del fondo glass** (frosted) detrás del chrome. `0`
    /// (default) = sin glass. Con Cerebro enlazado: `0`.
    pub(crate) fn config_glass_blur(&self) -> u8 {
        match &self.brain {
            Brain::Embedded(d) => d.config().glass_blur,
            Brain::Linked(_) => 0,
        }
    }

    /// **Calidad del backdrop glass**: `0` sólo wallpaper desenfocado · `1`
    /// backdrop REAL bajo el menú raíz · `2` además por barra flotante (calidad
    /// N). Sólo importa con `glass_blur > 0`. Con Cerebro enlazado el glass está
    /// apagado (`config_glass_blur` → 0), así que el valor es indiferente: `2`.
    pub(crate) fn config_glass_quality(&self) -> u8 {
        match &self.brain {
            Brain::Embedded(d) => d.config().glass_quality,
            Brain::Linked(_) => 2,
        }
    }

    /// `true` si el fondo por defecto debe ser el **wallpaper de marca animado**
    /// (chakana + plano cartesiano vivo). Aplica cuando la fuente cae al fondo
    /// por defecto (familia `auto`/`local`/`directory`/`remote` **sin** imagen) y
    /// «reducir movimiento» está apagado. Con `reduce_motion` el default vuelve a
    /// la marca estática (byte-idéntico al de antes). Global, no por-salida.
    pub(crate) fn config_animated_default(&self) -> bool {
        let Brain::Embedded(d) = &self.brain else {
            return false;
        };
        let c = d.config();
        if c.reduce_motion || !c.wallpaper_path.is_empty() {
            return false;
        }
        matches!(
            c.wallpaper_source.as_str(),
            "auto" | "local" | "directory" | "remote"
        )
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
    /// resaltado y pide el cierre ANIMADO (zoom-in hacia él — que ahora ya pasó a
    /// ser el activo, así que la cámara aterriza sobre él).
    pub(crate) fn overview_commit(&mut self) {
        if self.overview_open {
            self.cambiar_workspace(self.overview_selected);
            self.overview_closing = true;
        }
    }

    /// Duración (ms) del vuelo de cámara (zoom) de la vista espacial (Prezi).
    /// Cero si «reducir movimiento».
    pub(crate) fn config_overview_anim_ms(&self) -> u32 {
        if self.config_reduce_motion() {
            return 0;
        }
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

    /// El `id` de la ventana que respalda este toplevel, si la conocemos.
    pub(crate) fn id_para_superficie(
        &self,
        surface: &smithay::wayland::shell::xdg::ToplevelSurface,
    ) -> Option<u64> {
        self.windows
            .iter()
            .find(|w| w.surface == *surface.wl_surface())
            .map(|w| w.id)
    }

    /// Arranca un arrastre interactivo de **mover** sobre la ventana `id`,
    /// como si el usuario hubiera agarrado su barra. Lo pide un cliente CSD
    /// (Zen, GTK…) vía `xdg_toplevel.move` al arrastrar su propia barra de
    /// título — sin esto esas apps no se podían mover. Reusa la misma infra de
    /// `DragGrab` que `Super`+arrastre: el puntero ya está apretado, así que el
    /// release lo termina.
    pub(crate) fn start_interactive_move(&mut self, id: u64) {
        let Some(rect) = self
            .windows
            .iter()
            .find(|w| w.id == id)
            .map(|w| (w.loc.0, w.loc.1, w.size.0, w.size.1))
        else {
            return;
        };
        self.drag = Some(DragGrab {
            id,
            mode: DragMode::Move,
            start_pointer: self.pointer_loc,
            start_rect: rect,
        });
    }

    /// Como [`start_interactive_move`](Self::start_interactive_move) pero
    /// **redimensiona** (lo pide un cliente vía `xdg_toplevel.resize`). El
    /// borde concreto se ignora: redimensiona desde la esquina inferior-derecha
    /// (igual que `Super`+derecho) — suficiente para que el gesto del cliente
    /// surta efecto.
    pub(crate) fn start_interactive_resize(&mut self, id: u64) {
        let Some(rect) = self
            .windows
            .iter()
            .find(|w| w.id == id)
            .map(|w| (w.loc.0, w.loc.1, w.size.0, w.size.1))
        else {
            return;
        };
        self.drag = Some(DragGrab {
            id,
            mode: DragMode::Resize,
            start_pointer: self.pointer_loc,
            start_rect: rect,
        });
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
        } else if cmd == "fullscreen" {
            DesktopAction::ToggleFullscreen
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

    /// Ejecuta la acción de un **botón del titlebar** sobre la ventana `id`.
    /// Devuelve `false` para [`TitlebarAction::Menu`] —que el llamador resuelve
    /// abriendo el menú contextual, porque necesita la posición del puntero— y
    /// `true` para todo lo demás (ya manejado acá).
    pub(crate) fn accion_titlebar(&mut self, id: u64, action: &mirada_brain::TitlebarAction) -> bool {
        use mirada_brain::{DesktopAction, TitlebarAction as A};
        match action {
            A::Close => {
                if let Some(w) = self.windows.iter().find(|w| w.id == id) {
                    w.toplevel.send_close();
                }
            }
            A::Minimize => self.minimizar_ventana(id),
            A::Maximize => self.maximizar_ventana(id),
            A::Spawn(cmd) => self.spawn_user(cmd),
            A::Menu => return false, // lo abre el llamador (necesita el puntero)
            A::Float | A::Fullscreen => {
                let extra = if matches!(action, A::Float) {
                    DesktopAction::ToggleFloat
                } else {
                    DesktopAction::ToggleFullscreen
                };
                let cmds = match &mut self.brain {
                    Brain::Embedded(d) => {
                        let mut c = d.apply(DesktopAction::FocusWindow(id));
                        c.extend(d.apply(extra));
                        c
                    }
                    Brain::Linked(_) => return true,
                };
                self.apply_commands(cmds);
            }
        }
        true
    }

    /// Cambia al escritorio `idx` (0-based) — confirmación del switcher de
    /// Win+Tab. Por el Cerebro embebido.
    /// El escritorio ("zona") activo, para el clipboard por zona. `0` si no se
    /// conoce el estado (un solo escritorio efectivo: sin particionar).
    pub(crate) fn active_zone(&self) -> usize {
        self.workspace_overview().map(|(a, _)| a).unwrap_or(0)
    }

    /// Al entrar a una `zone`, re-ofrece su portapapeles de texto guardado como
    /// una selección **server-side** (o limpia la selección si esa zona no copió
    /// nada todavía), de modo que cada escritorio tenga su propio clipboard.
    /// No-op si el clipboard por zona está apagado. Ver [`crate::zone_clipboard`].
    pub(crate) fn restore_zone_clipboard(&mut self, zone: usize) {
        if !self.clipboard_por_zona {
            return;
        }
        use smithay::wayland::selection::data_device::{
            clear_data_device_selection, set_data_device_selection,
        };
        let mimes = self
            .zone_clipboard
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .for_zone(zone)
            .map(|c| c.mime_types.clone());
        let dh = self.dh.clone();
        match mimes {
            Some(mimes) if !mimes.is_empty() => {
                set_data_device_selection(&dh, &self.seat, mimes, zone)
            }
            _ => clear_data_device_selection(&dh, &self.seat),
        }
    }

    pub(crate) fn cambiar_workspace(&mut self, idx: usize) {
        let cmds = match &mut self.brain {
            Brain::Embedded(d) => d.apply(mirada_brain::DesktopAction::SwitchWorkspace(idx)),
            // Enlazado: el dueño externo cambia el escritorio; le mandamos el
            // salto y él reenvía el `SetWorkspaces` actualizado. El clipboard por
            // zona del path enlazado se restauraría al recibir ese `SetWorkspaces`
            // (no cableado aún: el escenario de metal corre el Cerebro embebido).
            Brain::Linked(link) => {
                let _ = link.send(&BodyEvent::SwitchWorkspace(idx as u32));
                return;
            }
        };
        self.apply_commands(cmds);
        // Recién ahora la zona activa es `idx`: re-ofrece su portapapeles.
        self.restore_zone_clipboard(idx);
    }

    /// Recompila el keymap del teclado vivo con la distribución/variante/opciones
    /// dadas (recarga en caliente). `layout`/`variant` aceptan listas con coma
    /// para multi-distribución; `options` lleva el `grp:*toggle` de cambio. Si la
    /// compilación falla (XKB inválido) conserva el keymap anterior. Tras
    /// aplicarlo, refresca el indicador. No-op sin teclado.
    pub(crate) fn apply_xkb_config(&mut self, layout: &str, variant: &str, options: &str) {
        let Some(kbd) = self.keyboard.clone() else {
            return;
        };
        let xkb = smithay::input::keyboard::XkbConfig {
            layout,
            variant,
            options: (!options.is_empty()).then(|| options.to_string()),
            ..Default::default()
        };
        match kbd.set_xkb_config(self, xkb) {
            Ok(()) => self.refresh_kbd_layout(),
            Err(e) => dlog!("mirada-compositor · XKB inválido, conservo el anterior: {e}"),
        }
    }

    /// Relee la distribución de teclado activa del estado XKB y la cachea en
    /// `kbd_layout` (para `mirada-ctl workspaces` → indicador de `pata`). Se
    /// llama tras cada evento de teclado: así un `grp:*toggle` que cambió el
    /// grupo se refleja en la barra. Barato (un lock + lectura); sólo escribe si
    /// cambió. No-op sin teclado o con Cerebro externo (no dueño del teclado).
    pub(crate) fn refresh_kbd_layout(&mut self) {
        let Some(kbd) = self.keyboard.clone() else {
            return;
        };
        let csv = match &self.brain {
            Brain::Embedded(d) => d.config().xkb_layout.clone(),
            Brain::Linked(_) => return,
        };
        let code = kbd.with_xkb_state(self, |ctx| crate::short_layout(ctx.xkb(), &csv));
        if self.kbd_layout != code {
            self.kbd_layout = code;
        }
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
                    keyboard_layout: self.kbd_layout.clone(),
                }),
                Brain::Linked(_) => CtlReply::Error("el Cerebro es externo".into()),
            },
            // El ciclo de zonas lo intercepta el bucle de control del backend
            // DRM (las zonas son estado del Cuerpo). Si llega aquí (p. ej. en
            // winit, sin zonas), es un no-op.
            CtlRequest::CycleZones => CtlReply::Ok,
        }
    }

    /// Atiende una petición del protocolo **mirada-aware** (botones que las apps
    /// aportan a su barra). Stateless: guarda/retira contribuciones por `app_id`
    /// y drena los clicks pendientes.
    pub(crate) fn serve_aware(
        &mut self,
        req: mirada_aware::AwareRequest,
    ) -> mirada_aware::AwareReply {
        use mirada_aware::{AwareReply, AwareRequest};
        match req {
            AwareRequest::Register { app_id, items } => {
                if items.is_empty() {
                    self.aware_items.remove(&app_id);
                } else {
                    self.aware_items.insert(app_id, items);
                }
                crate::screencopy::danar_todo(self); // repintar las barras
                AwareReply::Ok
            }
            AwareRequest::Unregister { app_id } => {
                self.aware_items.remove(&app_id);
                self.aware_clicks.remove(&app_id);
                crate::screencopy::danar_todo(self);
                AwareReply::Ok
            }
            AwareRequest::PollClicks { app_id } => {
                let clicks = self.aware_clicks.remove(&app_id).unwrap_or_default();
                AwareReply::Clicks(clicks)
            }
        }
    }

    /// Lanza un comando de **autoexec** como el usuario de la sesión y devuelve su
    /// PID. No-op (con un shell de credenciales arriba) → `None`.
    fn spawn_autoexec(&self, cmd: &str) -> Option<u32> {
        if self.shell_activo() {
            return None;
        }
        spawn_command(cmd, self.active_user().as_ref(), &self.active_env())
    }

    /// Reconcilia las **apps de arranque de la vista** (`autoexec`) con lo que ya
    /// está corriendo: termina (`SIGTERM`) los efímeros que dejaron de estar
    /// (cambio de vista) y lanza los nuevos. **No relanza** lo ya lanzado (respeta
    /// que el usuario lo cierre a mano). La decisión la toma
    /// [`mirada_brain::autoexec_plan`]; acá sólo se ejecuta.
    pub(crate) fn reconcile_autoexec(&mut self, autoexec: &[mirada_brain::AutoExec]) {
        let running: std::collections::HashMap<String, bool> =
            self.autoexec_procs.iter().map(|(c, (_, eph))| (c.clone(), *eph)).collect();
        let (kill, launch) = mirada_brain::autoexec_plan(&running, autoexec);
        for cmd in kill {
            if let Some((pid, _)) = self.autoexec_procs.remove(&cmd) {
                let _ = nix::sys::signal::kill(
                    nix::unistd::Pid::from_raw(pid as i32),
                    nix::sys::signal::Signal::SIGTERM,
                );
                println!("mirada-compositor · autoexec efímero terminado (pid {pid}): {cmd}");
            }
        }
        for a in launch {
            if let Some(pid) = self.spawn_autoexec(&a.command) {
                self.autoexec_procs.insert(a.command.clone(), (pid, a.ephemeral));
            }
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
                dlog!("mirada-compositor · keymap inválido, conservo el anterior: {e}")
            }
        }
    }

    /// Recarga la config general (dropterm, teselado, foco, marco) en
    /// caliente y re-envía la decoración. Conserva la anterior si es
    /// inválida. No-op con el Cerebro enlazado.
    pub(crate) fn reload_config_from(&mut self, path: &std::path::Path) {
        match mirada_brain::Config::load(path) {
            Ok(cfg) => {
                // El autoexec viaja en la config: lo reconciliamos esté el Cerebro
                // embebido o enlazado (lanzar/matar procesos es del Cuerpo).
                let autoexec = cfg.autoexec.clone();
                // La distribución de teclado (XKB) es asunto del Cuerpo: la
                // aplicamos EN CALIENTE recompilando el keymap del teclado vivo,
                // así cambiar `xkb_layout`/`xkb_variant`/`xkb_options` (p. ej.
                // desde wawa-panel) surte efecto sin reiniciar la sesión.
                self.apply_xkb_config(&cfg.xkb_layout, &cfg.xkb_variant, &cfg.xkb_options);
                let cmds = if let Brain::Embedded(d) = &mut self.brain {
                    d.reload_config(cfg)
                } else {
                    Vec::new()
                };
                if !cmds.is_empty() {
                    self.apply_commands(cmds);
                    println!("mirada-compositor · config recargada.");
                }
                self.reconcile_autoexec(&autoexec);
            }
            Err(e) => {
                dlog!("mirada-compositor · config inválida, conservo la anterior: {e}")
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
                dlog!("mirada-compositor · reglas inválidas, conservo las anteriores: {e}")
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
    /// `(tema, tamaño)` del cursor configurado — el set XCursor que pinta el
    /// puntero. Tema vacío con Cerebro enlazado o sin config → el cuadrado de
    /// software por defecto.
    pub(crate) fn config_cursor_theme(&self) -> (String, u32) {
        match &self.brain {
            Brain::Embedded(d) => {
                let c = d.config();
                (c.cursor_theme.clone(), c.cursor_size)
            }
            Brain::Linked(_) => (String::new(), 24),
        }
    }

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
            // Video: la ruta resuelta es el archivo; si está vacía cae al default.
            "video" => match ctx_path {
                Some(p) if !p.is_empty() => WallpaperSpec::Video(p.to_string()),
                _ => WallpaperSpec::Default,
            },
            // Lottie/rive: el asset es un único archivo global (no daemon por
            // salida) → se toma de `wallpaper_path`. El render reproduce su cache
            // bakeada (fondo-bake). Sin ruta cae al default.
            "lottie" if !c.wallpaper_path.is_empty() => {
                WallpaperSpec::Fondo(mirada_fondo::FondoSpec::Lottie {
                    path: c.wallpaper_path.clone(),
                })
            }
            "rive" if !c.wallpaper_path.is_empty() => {
                WallpaperSpec::Fondo(mirada_fondo::FondoSpec::Rive {
                    path: c.wallpaper_path.clone(),
                })
            }
            "lottie" | "rive" => WallpaperSpec::Default,
            // auto / local / directory / remote → imagen por la ruta resuelta.
            _ => img_or_default(fit),
        }
    }

    /// `true` si el wallpaper es **animado** (regenera frames): la chakana viva
    /// por defecto o un Lottie/rive bakeado. Lo usa el late de `tick` para marcar
    /// daño a ~20 fps sólo cuando hace falta.
    pub(crate) fn config_wallpaper_live(&self) -> bool {
        if self.config_animated_default() {
            return true;
        }
        let Brain::Embedded(d) = &self.brain else {
            return false;
        };
        let c = d.config();
        matches!(c.wallpaper_source.as_str(), "lottie" | "rive") && !c.wallpaper_path.is_empty()
    }

    /// Si el fondo de la salida `name` es **video**, devuelve `(ruta, fps)` para
    /// su worker (`fps = 0` ⇒ nativo). **Por salida:** la fuente es global pero la
    /// ruta se resuelve por salida ([`Self::config_wallpaper_path_for`] — el
    /// override de [`mirada_brain::OutputOverride`] gana, si no el global), así
    /// cada monitor puede correr su propio archivo. `None` con otra fuente, sin
    /// ruta, o con Cerebro enlazado.
    pub(crate) fn config_video_wallpaper_for(&self, name: &str) -> Option<(String, u32)> {
        let Brain::Embedded(d) = &self.brain else {
            return None;
        };
        if d.config().wallpaper_source != "video" {
            return None;
        }
        let fps = d.config().wallpaper_video_fps;
        let path = self.config_wallpaper_path_for(name).filter(|p| !p.is_empty())?;
        Some((path, fps))
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

    /// La **tiledad** difusa de la config (`0.0..=1.0`): cuán grande es el área
    /// de drag-to-zone (la banda de borde que pre-pinta y captura la ventana al
    /// soltar). Cerebro enlazado → el equilibrado `0.5`. Ver
    /// [`mirada_brain::Config::tiledad`].
    pub(crate) fn config_tiledad(&self) -> f32 {
        match &self.brain {
            Brain::Embedded(d) => d.config().tiledad.clamp(0.0, 1.0),
            Brain::Linked(_) => 0.5,
        }
    }

    /// Lanza `cmd` como el usuario de la sesión (igual que [`BodyOp::Spawn`]),
    /// salvo con un shell de credenciales arriba (greeter o lock), donde no se
    /// lanza nada. Lo usa el menú raíz.
    pub(crate) fn spawn_user(&self, cmd: &str) {
        if self.shell_activo() {
            dlog!("mirada-compositor · «{cmd}» rechazado — shell de credenciales activo.");
            return;
        }
        spawn_command(cmd, self.active_user().as_ref(), &self.active_env());
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
                    switch_mode,
                } => {
                    self.linked_ws = Some(crate::estado::LinkedWorkspaces {
                        active: active as usize,
                        loads: loads.into_iter().map(|n| n as usize).collect(),
                        slide_ms,
                        switch_mode: mirada_brain::WorkspaceSwitchMode::from_slug(&switch_mode)
                            .unwrap_or_default(),
                    });
                }
                // Lupa: zoom de pantalla completa. No produce BodyOps sobre
                // superficies (no cambia geometría) — sólo fija el factor y fuerza
                // un repintado completo; el render escala la escena alrededor del
                // puntero. `100` (1.0×) la apaga.
                BrainCommand::SetMagnify { factor_pct } => {
                    self.magnify = (factor_pct as f32 / 100.0).max(1.0);
                    crate::screencopy::danar_todo(self);
                }
                other => {
                    for op in self.body.apply(other) {
                        self.exec_op(op);
                    }
                }
            }
        }
        // Tras cualquier acción del Cerebro (cambiar de escritorio, cerrar la
        // última ventana, reenfocar…) reconciliamos el foco de teclado: en un
        // escritorio que quedó **vacío** cae al shell-barra (shuma/pata), así se
        // puede tipear sin clickear. Idempotente y nunca le roba el foco a una
        // ventana enfocada.
        self.reconcile_layer_keyboard();
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
                // Política «barra sólo en flotantes» vigente: se estampa en la
                // ventana antes de `titlebar_for`, para que el gate la respete.
                let tb_float_only = self.decorations.titlebar_floating_only;
                // En modo DM la ventana del greeter cubre TODO el espacio: la
                // unión de las salidas, anclada en (0,0). Así el fondo animado se
                // pinta en cada monitor y la tarjeta de login —que posiciona el
                // propio cliente— viaja al monitor con el ratón. Sin barra de
                // título (`tb = 0`). El backend reafirma esta geometría cada
                // frame (`sync_greeter_layout`) por si una carrera de arranque la
                // dejó en un solo monitor.
                let span = self.output_size;
                let mut danio = None;
                if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
                    let greeter_win = w.is_greeter;
                    let (rx, ry, rw, rh) = if greeter_win {
                        (0, 0, span.0, span.1)
                    } else {
                        (rect.x, rect.y, rect.w, rect.h)
                    };
                    // La celda vieja y la nueva quedan dañadas (screencopy):
                    // mover/redimensionar/ocultar repinta ambas regiones.
                    let viejo: Rectangle<i32, Logical> =
                        Rectangle::new(w.loc.into(), w.size.into());
                    let nuevo = Rectangle::new((rx, ry).into(), (rw, rh).into());
                    if viejo != nuevo || w.visible != visible {
                        danio = Some(viejo.merge(nuevo));
                    }
                    w.loc = (rx, ry);
                    w.size = (rw, rh);
                    w.visible = visible;
                    w.floating = floating;
                    w.titlebar_floating_only = tb_float_only;
                    w.fullscreen = fullscreen;
                    w.suspended = suspended;
                    w.frame_divisor = frame_divisor.max(1);
                    // `titlebar_for` ya descuenta shell/fullscreen/greeter y,
                    // ahora, las ventanas CSD (`!w.ssd`): a éstas se les configura
                    // la celda entera, sin reservar barra (la dibuja el cliente).
                    let tb = crate::titlebar_for(w, tbh);
                    // Una ventana teselada (ni shell, ni flotante, ni fullscreen)
                    // recibe los estados `tiled`: así los clientes CSD (GTK/Qt)
                    // sueltan su margen de sombra flotante y las esquinas
                    // redondeadas — antes salían «forradas dentro de un margen
                    // grandísimo». Las flotantes conservan su decoración.
                    let teselada = !w.is_shell && !floating && !fullscreen && !greeter_win;
                    w.toplevel.with_pending_state(|s| {
                        s.size = Some((rw.max(1), (rh - tb).max(1)).into());
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
            BodyOp::SetTitlebarLayout(l) => self.titlebar_layout = l,
            BodyOp::SetCapabilities(p) => *escribir_tolerante(&self.caps) = p,
            BodyOp::Spawn(cmd) => {
                // Con un shell de credenciales arriba (greeter o lock) no se
                // lanza nada: ni la pantalla de login ni el lock son un sitio
                // desde donde abrir programas.
                if self.shell_activo() {
                    dlog!("mirada-compositor · «{cmd}» rechazado — shell de credenciales activo.");
                } else {
                    spawn_command(&cmd, self.active_user().as_ref(), &self.active_env());
                }
            }
            BodyOp::Lock => self.request_lock(),
            BodyOp::Logout => self.logout(),
            BodyOp::Shutdown => self.running = false,
            BodyOp::SetEffects(v) => {
                for (id, effects) in v {
                    if let Some(w) = self.windows.iter_mut().find(|w| w.id == id) {
                        w.effects = effects;
                    }
                }
            }
        }
    }

    /// Registra un toplevel recién creado y avisa al Cerebro.
    pub(crate) fn register_toplevel(&mut self, toplevel: ToplevelSurface) {
        let surface = toplevel.wl_surface().clone();
        let id = self.next_id;
        self.next_id += 1;

        // ¿El cliente aceptó decoración del servidor? La negociación
        // `xdg-decoration` suele completarse antes del mapeo, así que el set
        // ya refleja su preferencia. Ausente = se decora solo (CSD).
        let ssd = self.ssd_surfaces.contains(&surface);

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
        // La ventana del shell de credenciales (greeter de login o lock): sin
        // barra de título, y el backend la expande a la unión de salidas. Se
        // detecta por MODO, no por `app_id`: al registrar el toplevel el cliente
        // todavía no presentó su `app_id` (lo setea en el primer commit), así
        // que comparar la cadena daba siempre falso. Con un shell arriba el
        // próximo cliente no-shell ES el shell. Caveat N=1: si una app de la
        // sesión abriera una ventana justo mientras está bloqueada, se la
        // marcaría is_greeter por error (glitch transitorio); es raro y se
        // resuelve al desbloquear. El `app_id` la cubre una vez presentado.
        let is_greeter =
            (self.shell_activo() && !is_shell) || app_id == "mirada.greeter";

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
            // Se re-estampa en el primer `Configure` desde la decoración vigente.
            titlebar_floating_only: self.decorations.titlebar_floating_only,
            focused: false,
            is_shell,
            is_greeter,
            // Dueña: la sesión activa al abrirse (FUS). Sin sesión (greeter de
            // arranque) cae al id 0 — da igual: esas ventanas son el propio
            // shell de credenciales, exento del gate por `is_greeter`.
            session: self
                .roster
                .active_id()
                .unwrap_or(mirada_brain::SessionId(0)),
            // Mismo `app_id` normalizado que se le pasa al Cerebro abajo, para
            // re-inyectar idéntico al saltar de sesión (FUS).
            app_id: if app_id.is_empty() { "cliente".into() } else { app_id.clone() },
            fullscreen: false,
            suspended: false,
            frame_divisor: 1,
            frame_tick: 0,
            title: title.clone(),
            foreign_handle,
            wlr_handles: Vec::new(),
            borders: std::array::from_fn(|_| SolidColorBuffer::default()),
            ssd,
            effects: mirada_brain::WindowEffects::default(),
            // Aún sin pintar: el render lo sella en el primer frame sano y ahí
            // arranca el fade-in de apertura.
            mapped_ms: None,
            // Nace sin foco y sin transición de glow estampada.
            focus_ms: None,
            was_focused: false,
            // Sin instantánea de cierre hasta que el render la capture.
            close_snapshot: None,
            last_snapshot_ms: 0,
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

    /// Apunta/desapunta una superficie como decorada por el servidor (SSD) y,
    /// si su ventana ya está mapeada, refleja el cambio: ajusta `w.ssd`,
    /// reconfigura el tamaño (reservar o liberar la franja de barra) y marca
    /// daño para que el render aparezca/quite la barra. La negociación suele
    /// llegar antes del mapeo: en ese caso sólo toca el set y `register_toplevel`
    /// lee el flag al crear la ventana.
    pub(crate) fn set_ssd_for(
        &mut self,
        toplevel: &smithay::wayland::shell::xdg::ToplevelSurface,
        ssd: bool,
    ) {
        let surface = toplevel.wl_surface().clone();
        if ssd {
            self.ssd_surfaces.insert(surface.clone());
        } else {
            self.ssd_surfaces.remove(&surface);
        }
        let tbh = self.decorations.titlebar_height;
        let mut danio = None;
        if let Some(w) = self.windows.iter_mut().find(|w| w.surface == surface) {
            if w.ssd != ssd {
                w.ssd = ssd;
                danio = Some(Rectangle::new(w.loc.into(), w.size.into()));
                let tb = crate::titlebar_for(w, tbh);
                let (rw, rh) = w.size;
                w.toplevel.with_pending_state(|s| {
                    s.size = Some((rw.max(1), (rh - tb).max(1)).into());
                });
                w.toplevel.send_pending_configure();
            }
        }
        if let Some(d) = danio {
            screencopy::danar(self, d);
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
            // Direccionar la reserva por el id ESTABLE del monitor (no el índice
            // `i`, que cambia al reordenar): así un dock en el monitor secundario
            // reserva en ESE monitor aunque la lista se haya reordenado. Sin un
            // id mapeado (no debería pasar) cae al índice como antes.
            let oid = self.output_ids.get(i).copied().unwrap_or(i as u32);
            let ev = self.body.reserve_output(oid, top, bottom, left, right);
            self.brain_feed(ev);
        }
    }

    /// La config de [esquinas calientes](mirada_brain::HotCorners) vigente.
    /// Con Cerebro enlazado no hay config local: se devuelve el default
    /// (deshabilitado), igual que el resto de getters `config_*`.
    pub(crate) fn config_hot_corners(&self) -> mirada_brain::HotCorners {
        match &self.brain {
            Brain::Embedded(d) => d.config().hot_corners.clone(),
            Brain::Linked(_) => mirada_brain::HotCorners::default(),
        }
    }

    /// Despliega la barra/dock autoescondido (la acción `reveal-shell` de una
    /// esquina caliente): la hace visible aunque el puntero no esté en la banda
    /// del borde. El autohide normal la vuelve a ocultar al salir de su franja
    /// (de ahí que convenga mapearla a la zona del MISMO borde que el dock).
    /// No-op sin autohide, sin dock acoplado, o si ya está visible.
    pub(crate) fn reveal_shell(&mut self) -> bool {
        if !self.shell_hidden || !shell_dock().autohide {
            return false;
        }
        if !self.windows.iter().any(|w| w.is_shell) {
            return false;
        }
        self.shell_hidden = false;
        let mut danio = None;
        if let Some(w) = self.windows.iter_mut().find(|w| w.is_shell) {
            w.visible = true;
            danio = Some(Rectangle::new(w.loc.into(), w.size.into()));
        }
        if let Some(d) = danio {
            screencopy::danar(self, d);
        }
        true
    }

    /// Abre (o pide cerrar) la vista espacial «Prezi» — la acción `overview` de
    /// una esquina caliente. Espeja el toggle de `Super+e`: con Cerebro embebido
    /// la pinta el Cuerpo; enlazado, le reenvía el atajo a la app dueña.
    pub(crate) fn open_overview(&mut self) {
        if !self.brain_is_embedded() {
            let ev = self.body.keybind("Super+e".to_string());
            self.brain_feed(ev);
            return;
        }
        if self.overview_open {
            self.overview_closing = true;
        } else {
            self.overview_open = true;
            self.overview_closing = false;
            self.overview_via_wintab = false;
            self.overview_selected = self.workspace_overview().map_or(0, |(a, _)| a);
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

    /// Usuario de la sesión activa — a quien rebajar privilegios al lanzar sus
    /// procesos. `None` sin sesión (modo greeter) o si la sesión corre con los
    /// privilegios del compositor (modo dev / sin root).
    pub(crate) fn active_user(&self) -> Option<UserInfo> {
        self.roster.active().and_then(|s| s.user.clone())
    }

    /// Entorno de la sesión activa (runtime dir, WAYLAND_DISPLAY, D-Bus, ctl).
    /// Vacío sin sesión.
    pub(crate) fn active_env(&self) -> Vec<(String, String)> {
        self.roster
            .active()
            .map(|s| s.env.clone())
            .unwrap_or_default()
    }

    /// ¿Se compone esta ventana? Sólo se pintan y animan las ventanas de la
    /// sesión **activa** (FUS); las de las sesiones residentes —o las huérfanas
    /// de una sesión recién cerrada por logout— quedan ocultas y sin frame
    /// callbacks (suspendidas de hecho). El shell/greeter (overlay de
    /// credenciales) no pertenece a ninguna sesión y siempre pasa. Sin sesión
    /// activa (greeter de arranque) no hay ventanas de usuario que ocultar.
    /// Con una sola sesión todas sus ventanas la tienen por activa, así que es
    /// byte-idéntico al camino single-session de siempre.
    pub(crate) fn session_visible(&self, w: &ManagedWindow) -> bool {
        if w.is_shell || w.is_greeter {
            return true;
        }
        match self.roster.active_id() {
            Some(active) => w.session == active,
            None => true,
        }
    }

    /// ¿Hay un shell de credenciales (greeter de login o lock) compuesto encima?
    /// En ese estado el input va al shell y se suspenden las interacciones
    /// normales de la sesión (menús, drag, spawn). Ver [`BodyMode::Locked`].
    pub(crate) fn shell_activo(&self) -> bool {
        matches!(self.mode, BodyMode::Greeter | BodyMode::Locked)
    }

    /// Despacha la acción que el shell de credenciales emitió por su stdout
    /// (canal [`auth_core::ShellAction`]).
    pub(crate) fn handle_shell_action(&mut self, action: auth_core::ShellAction) {
        use auth_core::ShellAction;
        match action {
            ShellAction::StartSession(ticket) => self.start_session(ticket),
            ShellAction::Unlock => self.unlock(),
            ShellAction::NewSession => self.request_new_session(),
            ShellAction::SwitchTo(id) => self.switch_session(mirada_brain::SessionId(id)),
            // El lock no ofrece cancelar sin desbloquear; reservado para FUS.
            ShellAction::Cancel => {}
        }
    }

    /// FUS «cambiar usuario»: pide volver al **login** para hostear una sesión
    /// nueva junto a la actual (que queda residente debajo). No relanza el
    /// greeter acá — necesita el emisor del canal, que vive en el bucle del
    /// backend; deja [`pending_new_session`](Self::pending_new_session) y vuelve
    /// a [`BodyMode::Greeter`]. El bucle lanza el greeter en modo login; el
    /// próximo `start_session` da de alta la sesión extra. No-op si ya hay un
    /// pedido en curso o estamos en el greeter de arranque (sin sesión que
    /// suspender). Sirve desde el lock (`Locked`) o desde la sesión viva.
    pub(crate) fn request_new_session(&mut self) {
        if self.pending_new_session || self.mode == BodyMode::Greeter {
            return;
        }
        self.pending_new_session = true;
        self.mode = BodyMode::Greeter;
        // Dejamos de empujarle la disposición de monitores al lock que se va (si
        // veníamos de un lock); el login nuevo recibe la suya al relanzarse.
        self.greeter_stdin = None;
        dlog!("mirada-compositor · FUS: abro el login para una sesión nueva.");
    }

    /// FUS: salta el foco a la sesión `id` (la elegida en el selector del lock).
    /// Cambia la sesión activa del roster — con ello el gate de
    /// [`session_visible`](Self::session_visible) pasa a mostrar sus ventanas y
    /// ocultar las de las demás — y sale de cualquier shell de credenciales
    /// (desbloqueo dirigido). No-op si no existe tal sesión.
    pub(crate) fn switch_session(&mut self, id: mirada_brain::SessionId) {
        let outgoing = self.roster.active_id();
        if !self.roster.switch_to(id) {
            return;
        }
        if self.mode == BodyMode::Locked {
            self.mode = BodyMode::Session;
            self.greeter_stdin = None;
        }
        // Reconstruye el escritorio embebido para la sesión entrante: guarda la
        // forma de la saliente, retira sus ventanas y restaura+re-inyecta las de
        // la entrante, para que cada usuario tesele en su propio escritorio.
        self.rebuild_desktop_for_active(outgoing);
        // El foco de teclado quedó en una ventana de la sesión anterior (ahora
        // oculta): hay que reencaminarlo a la sesión recién activada.
        self.refocus_active_session();
        dlog!("mirada-compositor · FUS: sesión activa → id {}.", id.0);
    }

    /// Reconstruye el `Desktop` embebido tras un cambio de sesión activa (FUS),
    /// para que cada usuario tesele en su **propio** escritorio en vez de
    /// compartir slots con las ventanas (ocultas) de las demás. El `Desktop` es
    /// uno solo y sigue a la sesión activa; aquí se hace el relevo:
    ///   1. **Guarda** la forma de la sesión `outgoing` (`snapshot`) en su roster
    ///      y **retira** sus ventanas vivas del escritorio (`WindowClosed` al
    ///      Cerebro — la `ManagedWindow` sigue viva, sólo sale del teselado).
    ///   2. **Restaura** la forma de la entrante y aplica su mapa salida→escritorio
    ///      en vivo (las salidas no se reconectan en un salto de sesión).
    ///   3. **Re-inyecta** las ventanas vivas de la entrante (`WindowOpened`), que
    ///      vuelven a su escritorio por `app_id` (homes) y se teselan solas.
    /// No-op con Cerebro enlazado o con ≤1 sesión (camino single-session intacto).
    /// **Por verificar en sesión gráfica** — el relevo de ventanas vivas no se
    /// puede certificar headless.
    fn rebuild_desktop_for_active(&mut self, outgoing: Option<mirada_brain::SessionId>) {
        // Sólo con Cerebro embebido (el enlazado es DE single-session). No se
        // condiciona al número de sesiones: el logout a la última sesión deja
        // `len == 1` y *necesita* re-inyectar sus ventanas. En un mundo
        // single-session puro este método no se llama nunca (no hay saltos).
        if !matches!(self.brain, Brain::Embedded(_)) {
            return;
        }
        let active = self.roster.active_id();
        if outgoing == active {
            return; // saltar a la misma sesión: nada que reconstruir
        }
        // 1 · Guardar la forma de la saliente y retirar sus ventanas.
        if let Some(out) = outgoing {
            if let Brain::Embedded(d) = &self.brain {
                let snap = d.snapshot();
                if let Some(s) = self.roster.get_mut(out) {
                    s.shape = Some(snap);
                }
            }
            let salientes: Vec<u64> = self
                .windows
                .iter()
                .filter(|w| !w.is_shell && !w.is_greeter && w.session == out)
                .map(|w| w.id)
                .collect();
            for id in salientes {
                self.brain_feed(BodyEvent::WindowClosed { id });
            }
        }
        // 2 · Restaurar la forma de la entrante (+ mapa de salidas en vivo).
        let Some(act) = active else { return };
        if let Some(shape) = self.roster.get(act).and_then(|s| s.shape.clone()) {
            let cmds = if let Brain::Embedded(d) = &mut self.brain {
                d.restore(&shape);
                d.apply_restored_output_workspaces()
            } else {
                Vec::new()
            };
            self.apply_commands(cmds);
        }
        // 3 · Re-inyectar las ventanas vivas de la entrante.
        let entrantes: Vec<(u64, String, String)> = self
            .windows
            .iter()
            .filter(|w| !w.is_shell && !w.is_greeter && w.session == act)
            .map(|w| (w.id, w.app_id.clone(), w.title.clone()))
            .collect();
        for (id, app_id, title) in entrantes {
            self.brain_feed(BodyEvent::WindowOpened { id, app_id, title });
        }
    }

    /// Tras un salto de sesión, pone el foco en una ventana visible de la sesión
    /// activa (la última en orden de aparición), o lo limpia si no hay ninguna.
    /// Mueve tanto el flag visual `focused` como el foco de teclado real de
    /// smithay — si no, el teclado seguiría yendo a una ventana de la sesión que
    /// acaba de ocultarse.
    fn refocus_active_session(&mut self) {
        let target = self
            .windows
            .iter()
            .rev()
            .find(|w| !w.is_shell && !w.is_greeter && self.session_visible(w))
            .map(|w| (w.id, w.surface.clone()));
        let nuevo_id = target.as_ref().map(|(id, _)| *id);
        for w in &mut self.windows {
            w.focused = Some(w.id) == nuevo_id;
        }
        // Foco de teclado real: a la superficie de la ventana elegida (o a None
        // si la sesión recién activada no tiene ventanas todavía).
        if let Some(kb) = self.keyboard.clone() {
            let surf = target.map(|(_, s)| s);
            kb.set_focus(self, surf, SERIAL_COUNTER.next_serial());
        }
    }

    /// Siembra la política de inactividad con la config del usuario (umbrales de
    /// apagado/bloqueo + respeto a inhibidores). Sólo con Cerebro embebido (el
    /// enlazado es DE: la inactividad la gestiona su propio entorno). Se llama al
    /// arrancar y tras cada recarga de config.
    pub(crate) fn sync_idle_config(&mut self) {
        if let Brain::Embedded(d) = &self.brain {
            let ic = d.config().idle_config();
            self.idle.set_config(ic);
        }
    }

    /// Avanza la política de inactividad un paso (lo llama el tick de cada
    /// backend). Mide el `dt` desde el tick anterior, consulta si hay multimedia
    /// inhibiendo (alguna superficie con idle-inhibitor) y ejecuta las acciones
    /// resultantes (apagar/encender pantalla, bloquear).
    pub(crate) fn idle_tick(&mut self) {
        let now = std::time::Instant::now();
        let dt_ms = match self.last_idle_tick.replace(now) {
            Some(prev) => now.saturating_duration_since(prev).as_millis() as u64,
            None => return, // primer tick: sólo fija la base temporal
        };
        let inhibited = !self.idle_inhibitors.is_empty();
        let actions = self.idle.tick(dt_ms, inhibited);
        self.apply_idle_actions(actions);
        // Clientes externos (ext-idle-notify) con su propio reloj: mismo dt, misma
        // consciencia de inhibición.
        self.drive_idle_notifs(dt_ms, inhibited);
    }

    /// Hubo **input** del usuario: reinicia la inactividad y, si la pantalla
    /// estaba apagada por ocio, pide encenderla. Lo llaman los handlers de
    /// entrada de cada backend.
    pub(crate) fn idle_activity(&mut self) {
        let actions = self.idle.activity();
        self.apply_idle_actions(actions);
        self.idle_notify_activity();
    }

    /// Ejecuta las acciones de la política de inactividad. El apagado/encendido
    /// se enruta por [`pending_dpms`](Self::pending_dpms) (lo consume el backend
    /// DRM); el bloqueo reusa [`request_lock`](Self::request_lock) (no-op si ya
    /// hay un shell de credenciales arriba).
    fn apply_idle_actions(&mut self, actions: Vec<mirada_brain::IdleAction>) {
        use mirada_brain::IdleAction;
        for a in actions {
            match a {
                IdleAction::ScreenOff => self.pending_dpms = Some(true),
                IdleAction::ScreenOn => self.pending_dpms = Some(false),
                IdleAction::Lock => self.request_lock(),
            }
        }
    }

    /// Pide bloquear la sesión activa. No cambia el modo todavía: deja un
    /// [`pending_lock`](Self::pending_lock) que el bucle del backend consume
    /// para lanzar el shell de credenciales en modo lock (necesita el emisor
    /// del canal, que no vive en `App`). No-op si ya hay un shell en pantalla
    /// (greeter o lock) — no se apila otro.
    pub(crate) fn request_lock(&mut self) {
        if self.shell_activo() || self.pending_lock.is_some() {
            return;
        }
        // A quién pedirle la contraseña: el dueño de la sesión activa, o —en
        // modo dev, corriendo ya como el usuario— el `$USER` del entorno. El
        // shell-lock valida contra PAM ese nombre.
        let user = self
            .active_user()
            .map(|u| u.name)
            .or_else(|| std::env::var("USER").ok())
            .unwrap_or_default();
        self.pending_lock = Some(user);
    }

    /// Desbloquea: el shell-lock validó la contraseña del dueño. Vuelve a
    /// [`BodyMode::Session`]; el proceso del shell-lock se cierra solo (emitió
    /// `Unlock` y llamó a `quit`), y al destruirse su superficie la sesión de
    /// abajo recupera foco y composición normales. No-op si no estaba bloqueada.
    pub(crate) fn unlock(&mut self) {
        if self.mode != BodyMode::Locked {
            return;
        }
        self.mode = BodyMode::Session;
        // Dejamos de empujarle la disposición de monitores al shell que se va.
        self.greeter_stdin = None;
        dlog!("mirada-compositor · sesión desbloqueada.");
    }

    /// Cierra la sesión activa (FUS logout): manda cerrar (ordenadamente) sus
    /// ventanas, la da de baja del roster y pasa el control a otra sesión
    /// hosteada — o, si no queda ninguna, vuelve al login para hostear una nueva.
    /// Disparado por `BodyOp::Logout` (atajo `Super+Shift+Escape`). No-op sin
    /// sesión activa (greeter de arranque). No mata procesos por uid (la sesión
    /// dev comparte el uid del compositor): el cierre `xdg_toplevel.close` es
    /// suficiente y uniforme — al desconectarse el cliente, su superficie muere.
    pub(crate) fn logout(&mut self) {
        let Some(id) = self.roster.active_id() else {
            return;
        };
        // 1 · Cerrar las ventanas de la sesión (pedido ordenado al cliente).
        for w in &self.windows {
            if !w.is_shell && !w.is_greeter && w.session == id {
                w.toplevel.send_close();
            }
        }
        // 2 · Baja del roster — el foco cae en la última sesión restante (o None).
        self.roster.remove(id);
        // 3 · Reconfigurar según lo que quede.
        if let Some(act) = self.roster.active_id() {
            // Otra sesión toma el control: restaurá su escritorio y re-inyectá
            // sus ventanas. La saliente ya no está en el roster; sus ventanas se
            // destruyen solas y el gate las oculta mientras tanto (`outgoing` =
            // `None`: no hay forma que guardar de la que se fue).
            self.rebuild_desktop_for_active(None);
            self.refocus_active_session();
            println!("mirada-compositor · FUS: logout — control a la sesión id {}.", act.0);
        } else {
            // No queda ninguna: volvé al login (como un arranque de DM). Reusa el
            // camino de `pending_new_session`: el bucle del backend relanza el
            // greeter en modo login y el próximo `start_session` da de alta otra.
            self.mode = BodyMode::Greeter;
            self.pending_new_session = true;
            self.greeter_stdin = None;
            println!("mirada-compositor · FUS: logout — sin sesiones, vuelvo al login.");
        }
    }

    /// Línea `SESSIONS` para el lock: el roster de sesiones hosteadas como
    /// `id:nombre`, con el id de la activa al frente, para que el shell de
    /// credenciales pinte el selector «cambiar usuario». `None` si no hay
    /// ninguna (nada que listar). El nombre se sanea (sin espacios ni `:`, que
    /// romperían el parseo del otro lado).
    pub(crate) fn sessions_line(&self) -> Option<String> {
        if self.roster.is_empty() {
            return None;
        }
        let active = self.roster.active_id().map(|i| i.0).unwrap_or(0);
        let mut line = format!("SESSIONS {active}");
        for (id, s) in self.roster.iter() {
            let name = s
                .user
                .as_ref()
                .map(|u| u.name.clone())
                .or_else(|| std::env::var("USER").ok())
                .unwrap_or_else(|| "sesión".into());
            let name = name.replace([' ', ':'], "_");
            line.push_str(&format!(" {}:{name}", id.0));
        }
        Some(line)
    }

    /// Empuja el roster al shell de credenciales por su stdin (si hay tubería),
    /// para que el lock liste las sesiones hosteadas. Se llama al lanzar el lock.
    /// Además marca [`pending_thumbs`](Self::pending_thumbs): el backend captura
    /// las miniaturas de las sesiones en el próximo cuadro (necesita el renderer).
    pub(crate) fn push_sessions_to_greeter(&mut self) {
        use std::io::Write;
        let Some(line) = self.sessions_line() else {
            return;
        };
        if let Some(stdin) = self.greeter_stdin.as_mut() {
            let _ = writeln!(stdin, "{line}").and_then(|_| stdin.flush());
        }
        // El próximo frame del backend captura las previews (ver `thumbs`).
        self.pending_thumbs = true;
    }

    /// Empuja al lock las rutas de las miniaturas capturadas (`THUMBS id=ruta …`).
    /// Backward-compatible: un greeter viejo ignora la línea. Sin miniaturas
    /// (preview apagada o nada que rendir) no manda nada — el lock cae a tarjetas
    /// genéricas. La llama el backend tras [`crate::thumbs::capturar`].
    pub(crate) fn send_thumbs(&mut self, thumbs: &[(mirada_brain::SessionId, std::path::PathBuf)]) {
        use std::io::Write;
        if thumbs.is_empty() {
            return;
        }
        let mut line = String::from("THUMBS");
        for (id, path) in thumbs {
            // Las rutas del runtime dir no llevan espacios; aun así, saltamos
            // cualquiera que los tenga para no romper el parseo del otro lado.
            let p = path.to_string_lossy();
            if p.contains(char::is_whitespace) {
                continue;
            }
            line.push_str(&format!(" {}={p}", id.0));
        }
        if let Some(stdin) = self.greeter_stdin.as_mut() {
            let _ = writeln!(stdin, "{line}").and_then(|_| stdin.flush());
        }
    }

    /// Arranca una sesión nueva tras un login válido — la «mutación atómica»
    /// del DM: el compositor pasa de la pantalla de greeter a la sesión del
    /// usuario **sin reiniciar el servidor Wayland** (el mismo proceso, la
    /// misma GPU). Sólo desde el login de arranque ([`BodyMode::Greeter`]); un
    /// tiquet de más se ignora. El compositor **no** hace `setuid` de sí mismo:
    /// queda con sus privilegios y lanza los clientes de la sesión rebajados al
    /// usuario — la forma que deja crecer a multisesión.
    fn start_session(&mut self, ticket: SessionTicket) {
        if self.mode != BodyMode::Greeter {
            return; // sólo desde el login (de arranque o de «cambiar usuario»)
        }
        // ¿Login de arranque (primera sesión) o «cambiar usuario» de FUS (una
        // sesión más junto a las residentes)? Lo segundo lo marcó
        // `request_new_session`; en cualquier caso el alta es la misma.
        let nueva = self.pending_new_session;
        self.pending_new_session = false;
        println!(
            "mirada-compositor · {} la sesión de «{}» (uid {}).",
            if nueva { "sumo" } else { "traspaso a" },
            ticket.user.name,
            ticket.user.uid
        );
        if !nix::unistd::geteuid().is_root() {
            dlog!(
                "mirada-compositor · aviso: no corro como root — la sesión \
                 heredará mis privilegios, sin setuid al usuario."
            );
        }
        self.mode = BodyMode::Session;
        // Quién era la activa antes del alta — para el relevo de escritorio (FUS):
        // sus ventanas se retiran del `Desktop` al sumar la nueva. `None` en el
        // login de arranque (roster vacío).
        let outgoing = self.roster.active_id();
        // Alta de la sesión y activación (el recién llegado pasa al frente). El
        // entorno se completa abajo con `setup_user_session_env`. El id estable
        // que devuelve el roster es el que llevarán sus ventanas.
        self.roster.add(crate::estado::Session {
            user: Some(ticket.user.clone()),
            env: Vec::new(),
            shape: None,
        });
        // Relevo de escritorio: la sesión saliente guarda su forma y suelta sus
        // ventanas del teselado; la nueva arranca con un escritorio propio
        // (vacío). No-op en el arranque (una sola sesión).
        self.rebuild_desktop_for_active(outgoing);

        // Ya en sesión: registra los atajos del escritorio y la decoración
        // (en modo greeter se omitieron a propósito — ver `build_app`).
        if let Brain::Embedded(desktop) = &self.brain {
            let cmds = vec![desktop.grab_keys(), desktop.decorations(), desktop.titlebar_layout()];
            let autoexec = desktop.config().autoexec.clone();
            self.apply_commands(cmds);
            // Apps de arranque de la vista inicial (ya con el entorno de sesión).
            self.reconcile_autoexec(&autoexec);
        }

        // Arranca la sesión. Tres caminos:
        //  · vacío         → autostart del usuario (cliente de este compositor).
        //  · nativo (pata) → comando como cliente, sin reiniciar el servidor.
        //  · ajeno         → soltar el DRM y `exec` (otro compositor toma la
        //                    GPU). Se difiere al cierre del bucle: marcamos la
        //                    sesión pendiente y pedimos salir.
        let user = Some(ticket.user.clone());
        // Prepara el entorno de sesión del usuario (runtime dir propio,
        // WAYLAND_DISPLAY absoluto, bus D-Bus) para que las apps nativas
        // —waybar, GTK/Qt— funcionen como en una sesión de verdad.
        if let Some(u) = &user {
            self.setup_user_session_env(u);
        }
        let env = self.active_env();
        let cmd = ticket.session.trim();
        if cmd.is_empty() {
            spawn_autostart(user.as_ref(), &env);
            spawn_config_startup(user.as_ref(), &env);
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
        // El socket Wayland lo creó el compositor como ROOT en su runtime dir
        // (p. ej. /run/arje, que arje deja en 0700). La sesión corre como el
        // USUARIO → sin esto no puede ni entrar al dir ni conectar al socket:
        // los clientes (pata, etc.) mueren con `NoCompositor`. Abrimos el dir
        // (traverse) y le damos el socket al usuario. No-op cuando el compositor
        // ya corre como el usuario (su runtime dir es propio).
        if !wl.is_empty() {
            if let Some(dir) = std::path::Path::new(&wl).parent() {
                let _ = std::fs::set_permissions(dir, std::fs::Permissions::from_mode(0o755));
            }
            let _ = nix::unistd::chown(
                wl.as_str(),
                Some(nix::unistd::Uid::from_raw(user.uid)),
                Some(nix::unistd::Gid::from_raw(user.gid)),
            );
        }
        let bus_path = format!("{xrd}/bus");
        let dbus_addr = format!("unix:path={bus_path}");
        // El socket de control vive en el runtime dir del COMPOSITOR (p. ej.
        // /run/mirada), no en el del usuario. Lo pasamos absoluto para que pata
        // y `mirada-ctl` de la sesión hablen con el Cerebro (sin esto, el
        // switcher de workspaces y el task-manager quedaban mudos).
        let ctl_sock = mirada_brain::ctl::default_socket_path().display().to_string();
        let env = vec![
            ("XDG_RUNTIME_DIR".to_string(), xrd),
            ("WAYLAND_DISPLAY".to_string(), wl),
            ("DBUS_SESSION_BUS_ADDRESS".to_string(), dbus_addr.clone()),
            ("MIRADA_CTL_SOCK".to_string(), ctl_sock),
        ];
        // Guarda el entorno en la sesión activa (la que se acaba de crear).
        if let Some(s) = self.roster.active_mut() {
            s.env = env.clone();
        }
        // Levanta el bus de sesión D-Bus como el usuario, si no hay uno, y
        // espera (acotado) a que el socket exista: si lanzáramos waybar/GTK
        // antes, fallarían con «cannot autolaunch D-Bus». Es un bloqueo de
        // una sola vez al iniciar la sesión, no en el bucle de render.
        if !std::path::Path::new(&bus_path).exists() {
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
                dlog!(
                    "mirada-compositor · el bus D-Bus no apareció (¿dbus-daemon instalado?); las apps que lo exijan pueden fallar."
                );
            }
        }
    }
}

/// Recorre recursivamente los popups colgados de `parent` y devuelve el topmost
/// bajo `(x, y)` con el origen GLOBAL de su superficie. Los hijos (submenús) se
/// prueban primero porque se pintan por encima. `base` es el origen de geometría
/// del parent en coords globales. Función libre para reusar sin `&self`.
fn popup_under_tree(
    parent: &WlSurface,
    base: (i32, i32),
    x: f64,
    y: f64,
) -> Option<(WlSurface, Point<f64, Logical>)> {
    let mut hit = None;
    for (popup, ploc) in smithay::desktop::PopupManager::popups_for_surface(parent) {
        let psurf = popup.wl_surface().clone();
        let pgeo = popup.geometry();
        let geo_origin = (base.0 + ploc.x, base.1 + ploc.y);
        if let Some(found) = popup_under_tree(&psurf, geo_origin, x, y) {
            return Some(found);
        }
        let (rx, ry) = geo_origin;
        if x >= rx as f64
            && y >= ry as f64
            && x < (rx + pgeo.size.w) as f64
            && y < (ry + pgeo.size.h) as f64
        {
            // Origen de la SUPERFICIE (0,0) = origen de geometría − su offset.
            let sx = (rx - pgeo.loc.x) as f64;
            let sy = (ry - pgeo.loc.y) as f64;
            hit = Some((psurf, Point::from((sx, sy))));
        }
    }
    hit
}

/// Busca el popup VIVO más profundo colgado de `parent` (la hoja del árbol de
/// menús). Guarda en `best` el par `(profundidad, superficie)` de mayor
/// profundidad. Ignora superficies muertas (un popup recién destruido puede
/// seguir listado en el árbol hasta el `cleanup`).
fn deepest_popup(parent: &WlSurface, depth: usize, best: &mut Option<(usize, WlSurface)>) {
    for (popup, _) in smithay::desktop::PopupManager::popups_for_surface(parent) {
        let s = popup.wl_surface().clone();
        if !s.alive() {
            continue;
        }
        let d = depth + 1;
        if best.as_ref().map_or(true, |(bd, _)| d > *bd) {
            *best = Some((d, s.clone()));
        }
        deepest_popup(&s, d, best);
    }
}

/// Junta recursivamente todos los popups (menús) colgados de `parent`.
fn collect_popup_kinds(parent: &WlSurface, out: &mut Vec<smithay::desktop::PopupKind>) {
    for (popup, _) in smithay::desktop::PopupManager::popups_for_surface(parent) {
        let s = popup.wl_surface().clone();
        out.push(popup);
        collect_popup_kinds(&s, out);
    }
}
