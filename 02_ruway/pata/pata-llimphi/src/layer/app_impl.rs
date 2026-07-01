//! Implementación de los métodos de `LayerApp`: lógica de la aplicación,
//! gestión de panels, muestreo, render y manejo de mensajes.

use std::ffi::c_void;
use std::ptr::NonNull;

use raw_window_handle::{
    RawDisplayHandle, RawWindowHandle, WaylandDisplayHandle, WaylandWindowHandle,
};
use smithay_client_toolkit::shell::WaylandSurface;
use wayland_client::{protocol::wl_surface, Proxy, QueueHandle};

use llimphi_ui::llimphi_compositor::{
    hit_test_click, hit_test_hover, hit_test_scroll, measure_text_node, mount, paint, DragPhase,
};
use llimphi_ui::llimphi_hal::{wgpu, Hal, RawSurface, Surface as _};
use llimphi_ui::llimphi_layout::taffy;
use llimphi_ui::llimphi_raster::{peniko::color::palette, vello, Renderer};
use llimphi_ui::llimphi_text::Typesetter;

use pata_core::SurfaceKind;

use crate::nouser::{MembersOutcome, PollOutcome};
use pata_host::HostServer;
use crate::toplevel::{Toplevel, WindowEntry};
use crate::{render, Msg};

use super::{
    diag, CardState, LayerApp, LayerDrag, MenuKind, PanelGpu, Panel, RenderCache, TaskDrag,
    DRAWER_H, MENU_H,
};

impl LayerApp {
    /// Índice del panel cuya layer surface es `surface`.
    pub(super) fn panel_de(&self, surface: &wl_surface::WlSurface) -> Option<usize> {
        self.panels
            .iter()
            .position(|p| p.layer.wl_surface() == surface)
    }

    /// Marca la barra de shuma para re-pintar.
    pub(super) fn marcar_shuma_dirty(&mut self) {
        if let Some(pi) = self.shuma_panel {
            self.panels[pi].dirty = true;
        }
    }

    /// Marca todas las barras para re-pintar.
    pub(super) fn marcar_todo_dirty(&mut self) {
        for p in &mut self.panels {
            p.dirty = true;
        }
    }

    /// Tras rodar la rueda sobre el volumen: refleja el valor nuevo YA (sin
    /// esperar el ciclo del sampler de fondo) re-muestreando a `self.ctx` y
    /// marcando todo para repintar — así el tooltip se actualiza en tiempo real.
    pub(super) fn refresh_volume_now(&mut self) {
        if let Some((v, _muted)) = crate::sampler::sample_volume() {
            self.ctx.volume = v;
        }
        self.marcar_todo_dirty();
    }

    /// Igual que [`Self::refresh_volume_now`] para el brillo.
    pub(super) fn refresh_brightness_now(&mut self) {
        if let Some(b) = crate::sampler::sample_backlight() {
            self.ctx.brightness = b;
        }
        self.marcar_todo_dirty();
    }

    /// Dispara el cartel OSD (volumen/brillo) y marca su surface para repintar.
    pub(super) fn flash_osd(&mut self, kind: crate::render::OsdKind, level: f32, muted: bool) {
        self.osd = Some(crate::render::Osd::flash(kind, level, muted));
        if let Some(pi) = self.osd_pi {
            self.panels[pi].dirty = true;
        }
        // El diente vivo también reacciona al volumen al instante (sin esperar al
        // muestreo de 1 Hz): dispara su transitorio con la misma señal del OSD.
        if kind == crate::render::OsdKind::Volume {
            let now = self.diente_t0.elapsed().as_secs_f64();
            self.atencion.flash(
                pata_core::atencion::Manifestacion::Volumen { frac: level, muted },
                pata_core::atencion::VOLUMEN_TTL,
                now,
            );
            let s = self.senales_diente();
            self.diente_manifest = self.atencion.resolver(s, now);
        }
    }

    /// La lista de ventanas para el render del `window_list`, en el orden propio
    /// definido por el drag-to-reorder (`task_order`). Las ventanas que no
    /// figuran en ese orden (recién abiertas) quedan al final en orden natural.
    pub(super) fn window_entries(&self) -> Vec<WindowEntry> {
        let mut entries: Vec<WindowEntry> = self
            .toplevels
            .iter()
            .map(|t| WindowEntry {
                id: t.id,
                label: t.etiqueta(),
                app_id: t.app_id.clone(),
                active: t.activated,
                minimized: t.minimized,
            })
            .collect();
        if !self.task_order.is_empty() {
            // `sort_by_key` es estable: las desconocidas (clave `usize::MAX`)
            // conservan su orden natural relativo al final de la lista.
            entries.sort_by_key(|e| {
                self.task_order
                    .iter()
                    .position(|&id| id == e.id)
                    .unwrap_or(usize::MAX)
            });
        }
        entries
    }

    /// El toplevel con ese `id`, si sigue abierto.
    pub(super) fn toplevel_por_id(&self, id: u32) -> Option<&Toplevel> {
        self.toplevels.iter().find(|t| t.id == id)
    }

    /// Despliega o repliega el drawer Quake.
    pub(super) fn set_shuma_open(&mut self, open: bool) {
        let Some(pi) = self.shuma_panel else { return };
        if self.shuma.open == open {
            return;
        }
        self.shuma.open = open;
        self.shuma_opened_at = open.then(std::time::Instant::now);
        let h = if open { 10_000 } else { self.shuma_bar_px };
        let layer = &self.panels[pi].layer;
        layer.set_size(0, h);
        // Abierto = Exclusive (el drawer agarra todo el teclado). Plegado =
        // OnDemand (no `None`): la barra sigue pudiendo reclamar el teclado, así
        // mirada se lo da en escritorio vacío (keyboard_fallback_target) sin
        // robárselo a una ventana enfocada.
        layer.set_keyboard_interactivity(if open {
            smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::Exclusive
        } else {
            smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::OnDemand
        });
        layer.commit();
        self.panels[pi].cache = None;
        self.panels[pi].dirty = true;
    }

    /// Drena la cola de la shuma completa (live-wire) y aplica cada `Msg`.
    /// Repinta el panel si el drawer está abierto (plegado igual avanza el
    /// modelo, sólo no fuerza repaint).
    pub(super) fn drain_shuma_full(&mut self, pi: usize) {
        let msgs: Vec<crate::shuma_app::Msg> = match self.shuma_full_rx.as_ref() {
            Some(rx) => rx.try_iter().collect(),
            None => return,
        };
        if msgs.is_empty() {
            return;
        }
        self.apply_shuma_full(msgs);
        if self.shuma.open {
            self.panels[pi].dirty = true;
        }
    }

    /// Aplica una tanda de `Msg` a la shuma completa con el handle
    /// channel-backed (sus follow-ups vuelven a la cola).
    pub(super) fn apply_shuma_full(&mut self, msgs: Vec<crate::shuma_app::Msg>) {
        let Some(handle) = self.shuma_full_handle.clone() else {
            return;
        };
        if let Some(mut full) = self.shuma_full.take() {
            for m in msgs {
                // Click sobre el input de la barra → FocusInput de la sesión
                // activa: además de focalizar, despleguemos el drawer.
                let abrir = !self.shuma.open && crate::shuma_app::msg_is_focus_input_raw(&m);
                full = crate::shuma_app::update(full, m, &handle, |x| x);
                if abrir {
                    self.shuma_full = Some(full);
                    self.set_shuma_open(true);
                    full = self.shuma_full.take().unwrap();
                }
            }
            self.shuma_full = Some(full);
        }
    }

    /// Traduce un evento de teclado de SCTK al `llimphi_ui::KeyEvent`.
    pub(super) fn keysym_to_keyevent(
        &self,
        event: &smithay_client_toolkit::seat::keyboard::KeyEvent,
    ) -> Option<llimphi_ui::KeyEvent> {
        use llimphi_ui::{Key, NamedKey};
        use smithay_client_toolkit::seat::keyboard::Keysym as K;
        let named = match event.keysym {
            K::Return | K::KP_Enter => Some(NamedKey::Enter),
            K::BackSpace => Some(NamedKey::Backspace),
            K::Tab | K::ISO_Left_Tab => Some(NamedKey::Tab),
            K::Escape => Some(NamedKey::Escape),
            K::Up => Some(NamedKey::ArrowUp),
            K::Down => Some(NamedKey::ArrowDown),
            K::Right => Some(NamedKey::ArrowRight),
            K::Left => Some(NamedKey::ArrowLeft),
            K::Home => Some(NamedKey::Home),
            K::End => Some(NamedKey::End),
            K::Page_Up => Some(NamedKey::PageUp),
            K::Page_Down => Some(NamedKey::PageDown),
            K::Delete => Some(NamedKey::Delete),
            K::Insert => Some(NamedKey::Insert),
            K::F1 => Some(NamedKey::F1),
            K::F2 => Some(NamedKey::F2),
            K::F3 => Some(NamedKey::F3),
            K::F4 => Some(NamedKey::F4),
            K::F5 => Some(NamedKey::F5),
            K::F6 => Some(NamedKey::F6),
            K::F7 => Some(NamedKey::F7),
            K::F8 => Some(NamedKey::F8),
            K::F9 => Some(NamedKey::F9),
            K::F10 => Some(NamedKey::F10),
            K::F11 => Some(NamedKey::F11),
            K::F12 => Some(NamedKey::F12),
            _ => None,
        };
        let modifiers = llimphi_ui::Modifiers {
            shift: self.mods.shift,
            ctrl: self.mods.ctrl,
            alt: self.mods.alt,
            meta: self.mods.logo,
        };
        let (key, text) = if let Some(n) = named {
            (Key::Named(n), None)
        } else {
            let txt = match event.utf8.as_deref() {
                Some(s) if !s.is_empty() && !s.chars().all(char::is_control) => s.to_string(),
                _ => event.keysym.key_char()?.to_string(),
            };
            (Key::Character(txt.as_str().into()), Some(txt))
        };
        Some(llimphi_ui::KeyEvent {
            key,
            state: llimphi_ui::KeyState::Pressed,
            text,
            modifiers,
            repeat: false,
        })
    }

    /// Reencuentra el panel que hospeda el menú de inicio (el del `start_button`
    /// o, en CDE, el `front_panel`). Se computa una vez al arrancar, pero un
    /// hot-reload o un orden de creación inesperado lo pueden dejar en `None`;
    /// esto lo resana sobre los paneles vivos. Devuelve `None` si de verdad no
    /// hay barra con botón de inicio.
    pub(super) fn resolve_menu_panel(&mut self) -> Option<usize> {
        if self.menu_panel.is_none() {
            self.menu_panel = self.panels.iter().position(|p| {
                let s = &self.cfg.surfaces[p.idx];
                s.start
                    .iter()
                    .chain(&s.center)
                    .chain(&s.end)
                    .any(|w| w.kind == "start_button" || w.kind == "front_panel")
            });
            if self.menu_panel.is_none() && std::env::var_os("PATA_DIAG").is_some() {
                eprintln!(
                    "pata diag · menú inicio: ningún panel tiene start_button/front_panel \
                     (paneles={}); el botón no abrirá nada",
                    self.panels.len()
                );
            }
        }
        self.menu_panel
    }

    /// Despliega/repliega el menú de inicio.
    pub(super) fn set_menu_open(&mut self, open: bool) {
        let Some(pi) = self.resolve_menu_panel() else { return };
        if self.menu_open == open {
            return;
        }
        self.menu_open = open;
        self.menu_opened_at = open.then(std::time::Instant::now);
        if open {
            // Cada apertura arranca en la primera categoría.
            self.menu_cat = None;
        } else {
            self.menu_query.clear();
            self.menu_scroll = 0.0;
            self.menu_cat = None;
        }
        let h = if open { MENU_H } else { self.menu_bar_px };
        let layer = &self.panels[pi].layer;
        layer.set_size(0, h);
        let toma_teclado = open && self.menu_kind == MenuKind::Apps;
        layer.set_keyboard_interactivity(if toma_teclado {
            smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::Exclusive
        } else {
            smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::None
        });
        layer.commit();
        self.panels[pi].cache = None;
        self.panels[pi].dirty = true;
    }

    /// Drena las solicitudes del agente polkit. La primera abre el diálogo (crece
    /// el panel del menú como `Polkit` y captura el teclado); si ya hay una en
    /// curso, la nueva se rechaza.
    pub(super) fn poll_polkit(&mut self) {
        let Some(h) = &self.polkit else { return };
        let mut nuevas = Vec::new();
        while let Some(req) = h.try_recv() {
            nuevas.push(req);
        }
        for req in nuevas {
            if self.polkit_prompt.is_none() {
                self.polkit_input.clear();
                self.polkit_prompt = Some(req);
                self.menu_kind = MenuKind::Polkit;
                self.set_menu_open(true);
                self.set_menu_keyboard(true);
            } else {
                let _ = req.reply.send(None);
            }
        }
    }

    /// Cierra el diálogo de polkit: revoca el teclado y repliega el menú.
    pub(super) fn cerrar_polkit(&mut self) {
        self.polkit_input.clear();
        self.set_menu_keyboard(false);
        self.set_menu_open(false);
    }

    /// Concede o revoca el foco de teclado al panel del menú abierto (lo usa la
    /// entrada de contraseña Wi-Fi, que necesita teclear dentro del popup como el
    /// buscador del menú de inicio).
    pub(super) fn set_menu_keyboard(&mut self, exclusive: bool) {
        let Some(pi) = self.menu_panel else { return };
        use smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity;
        let layer = &self.panels[pi].layer;
        layer.set_keyboard_interactivity(if exclusive {
            KeyboardInteractivity::Exclusive
        } else {
            KeyboardInteractivity::None
        });
        layer.commit();
    }

    /// Abre/cierra el drawer de la barra del menú mostrando el cuerpo `kind`.
    pub(super) fn toggle_menu(&mut self, kind: MenuKind) {
        if self.menu_open && self.menu_kind == kind {
            self.set_menu_open(false);
        } else if self.menu_open {
            self.menu_kind = kind;
            if let Some(pi) = self.menu_panel {
                let toma = kind == MenuKind::Apps;
                let layer = &self.panels[pi].layer;
                layer.set_keyboard_interactivity(if toma {
                    smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::Exclusive
                } else {
                    smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::None
                });
                layer.commit();
                self.panels[pi].cache = None;
                self.panels[pi].dirty = true;
            }
        } else {
            self.menu_kind = kind;
            self.set_menu_open(true);
        }
    }

    /// Actualiza el tooltip flotante para el nodo `node_idx` bajo el cursor.
    pub(super) fn update_tooltip(&mut self, pi: usize, node_idx: Option<usize>, qh: &QueueHandle<Self>) {
        let Some(tpi) = self.tooltip_pi else { return };
        if pi == tpi {
            return;
        }
        let info = node_idx.and_then(|i| {
            let c = self.panels[pi].cache.as_ref()?;
            let node = c.mounted.nodes.get(i)?;
            let text = node.tooltip.clone()?;
            let rect = c.computed.get(node.id)?;
            Some((text, rect))
        });
        match info {
            Some((text, rect)) => {
                let x = rect.x.max(0.0) as i32;
                let y = self.panels[pi].height as i32 + 4;
                let w = (text.chars().count() as u32 * 8 + 16).clamp(24, 600);
                let h = 24u32;
                self.tooltip_text = Some(text);
                {
                    let layer = &self.panels[tpi].layer;
                    layer.set_margin(y, 0, 0, x);
                    layer.commit();
                    layer.set_size(w, h);
                    layer.commit();
                }
                self.panels[tpi].width = w;
                self.panels[tpi].height = h;
                self.panels[tpi].dirty = true;
                self.draw(tpi, qh);
            }
            None => self.hide_tooltip(qh),
        }
    }

    /// Oculta el tooltip encogiendo la surface a 1×1.
    pub(super) fn hide_tooltip(&mut self, qh: &QueueHandle<Self>) {
        let Some(tpi) = self.tooltip_pi else { return };
        if self.tooltip_text.is_none() {
            return;
        }
        self.tooltip_text = None;
        {
            let layer = &self.panels[tpi].layer;
            layer.set_size(1, 1);
            layer.commit();
        }
        self.panels[tpi].width = 1;
        self.panels[tpi].height = 1;
        self.panels[tpi].dirty = true;
        self.draw(tpi, qh);
    }

    /// Lanza una app del menú por su `id` y cierra el menú.
    pub(super) fn lanzar_app(&mut self, id: String) {
        if let Some(app) = self.registry.get(&id) {
            // Vía arje si está levantado (Ente OneShot); si no, crudo.
            arje_applaunch::launch_entry(app);
        }
        self.set_menu_open(false);
    }

    /// Marca para re-pintar la barra que hospeda el menú de inicio.
    pub(super) fn marcar_menu_dirty(&mut self) {
        if let Some(pi) = self.menu_panel {
            self.panels[pi].cache = None;
            self.panels[pi].dirty = true;
        }
    }

    /// Enter en el menú de inicio: lanza el primer resultado del filtro.
    pub(super) fn lanzar_primero_menu(&mut self) {
        let id = render::menu_filtered(self.registry.all(), &self.menu_query)
            .first()
            .map(|a| a.id.clone());
        if let Some(id) = id {
            self.lanzar_app(id);
        }
    }

    /// Sondea el plano de datos del sidebar.
    pub(super) fn poll_nav(&mut self) {
        let mut cambios = false;
        if let Some(rx) = self.nav_rx.as_ref() {
            let mut ultimo = None;
            while let Ok(o) = rx.try_recv() {
                ultimo = Some(o);
            }
            if let Some(outcome) = ultimo {
                match outcome {
                    PollOutcome::Ok { socket, resp } => {
                        self.nav.socket = Some(socket);
                        self.nav.apply_monads(*resp);
                    }
                    PollOutcome::Failed(e) => {
                        self.nav.socket = None;
                        self.nav.error = Some(e);
                    }
                }
                cambios = true;
            }
        }
        while let Ok(outcome) = self.members_rx.try_recv() {
            match outcome {
                MembersOutcome::Ok { monad, members } => self.nav.apply_members(monad, members),
                MembersOutcome::Failed(e) => self.nav.error = Some(e),
            }
            cambios = true;
        }
        // Resultados del motor RAG (respuesta/error/listo): los procesa el mismo
        // `handle_msg`, que marca los sidebars sucios al mutar el estado.
        while let Ok(m) = self.rag_rx.try_recv() {
            self.handle_msg(m);
        }
        if cambios {
            self.marcar_sidebars_dirty();
        }
    }

    /// `true` si el diente abierto del sidebar es el panel RAG (su contenido es
    /// `rag`/`search`). El teclado se rutea a su buscador sólo entonces.
    pub(super) fn rag_panel_open(&self) -> bool {
        let Some((si, ti)) = self.nav.open else {
            return false;
        };
        self.cfg
            .surfaces
            .get(si)
            .and_then(|s| s.tabs.get(ti))
            .map(|t| crate::rag::is_rag_kind(&t.content.kind))
            .unwrap_or(false)
    }

    /// El `app_id` del toplevel que tiene foco ahora.
    pub(super) fn focused_app_id(&self) -> Option<&str> {
        self.toplevels
            .iter()
            .find(|t| t.activated)
            .map(|t| t.app_id.as_str())
    }

    /// Sondea el rail hospedado.
    pub(super) fn poll_host(&mut self) {
        let Some(h) = &self.host else { return };
        let rev = h.revision();
        if rev != self.last_host_rev {
            self.last_host_rev = rev;
            self.marcar_sidebars_dirty();
        }
    }

    /// Marca todas las superficies sidebar para re-pintar.
    pub(super) fn marcar_sidebars_dirty(&mut self) {
        for p in &mut self.panels {
            if p.card.is_none() && self.cfg.surfaces[p.idx].kind == SurfaceKind::Sidebar {
                p.dirty = true;
            }
        }
    }

    /// Índice (en `panels`) de la layer surface del **rail** del sidebar `si` (no
    /// su drawer: el drawer comparte `idx` pero lleva `drawer == true`).
    pub(super) fn sidebar_panel_de(&self, si: usize) -> Option<usize> {
        self.panels
            .iter()
            .position(|p| p.idx == si && p.card.is_none() && !p.drawer)
    }

    /// Activa/repliega el diente `(si, ti)`. Sólo toca el ESTADO (`nav.open`) y
    /// marca el rail sucio; el drawer (una surface aparte) lo crea/destruye
    /// [`Self::reconcile_drawer`] en el próximo `draw` (que tiene el `QueueHandle`).
    /// Ya NO redimensiona el rail — ese resize por-diente era lo que fallaba en
    /// Iris Xe (reconfigurar el swapchain de una layer surface), dejaba el panel
    /// sin recibir clicks (bbox del buffer = 44px) y traspasaba el puntero a la
    /// ventana de atrás.
    pub(super) fn set_sidebar_open(&mut self, si: usize, ti: usize) {
        self.nav.toggle_tab(si, ti);
        // El rail repinta la pastilla activa del diente; el drawer se reconcilia solo.
        if let Some(pi) = self.sidebar_panel_de(si) {
            self.panels[pi].cache = None;
            self.panels[pi].dirty = true;
        }
    }

    /// Crea/destruye el **drawer** del sidebar para que refleje `nav.open`. Se llama
    /// desde `draw` (donde hay `QueueHandle`, necesario para crear surfaces). Barato
    /// y idempotente: si el sidebar abierto no cambió, retorna sin tocar nada.
    ///
    /// El drawer es una layer surface APARTE del rail, de tamaño fijo (`panel_width`
    /// × alto), pegada al borde interno del rail. Al no redimensionarse nunca, evita
    /// el bug de Iris Xe; al ser su propia surface, el compositor la rutea al puntero
    /// por su propio bbox (los clicks caen dentro, no traspasan).
    pub(super) fn reconcile_drawer(&mut self, qh: &QueueHandle<Self>) {
        let want_si = self.nav.open.map(|(si, _)| si);
        if want_si == self.drawer_si {
            return; // estable (mismo sidebar, o ninguno): el contenido lo refresca `dirty`.
        }
        diag!(
            "pata diag · reconcile_drawer want_si={want_si:?} drawer_si={:?} → recrear",
            self.drawer_si
        );
        // Cambió el sidebar abierto (o se cerró): destruir el drawer viejo…
        self.destroy_drawer();
        // …y, si hay uno abierto ahora, crear el suyo.
        if let Some(si) = want_si {
            self.create_drawer(si, qh);
        }
    }

    /// Crea la surface del drawer para el sidebar `si`, pegada a su rail. La primera
    /// `configure` (respuesta del compositor al `commit`) fija su tamaño real y
    /// dispara su primer `draw` → `ensure_gpu` → present, igual que las surfaces del
    /// arranque. No forzamos un draw acá (aún no hay tamaño configurado).
    fn create_drawer(&mut self, si: usize, qh: &QueueHandle<Self>) {
        use smithay_client_toolkit::shell::wlr_layer::{Anchor as LayerAnchor, KeyboardInteractivity, Layer};
        let Some(rail_pi) = self.sidebar_panel_de(si) else {
            return; // sin rail no hay dónde pegar el drawer.
        };
        let s = &self.cfg.surfaces[si];
        let thickness = s.thickness.max(1.0) as u32;
        let pw = s.panel_width.max(1.0) as u32;
        // Margen lateral hacia el rail. Si el rail RESERVA franja (`exclusive_zone`
        // = thickness, docked y no autohide), esa zona ya corre el área usable → el
        // drawer (con `exclusive_zone = 0`) arranca pegado al rail sin margen extra.
        // Si el rail flota (zona 0), hay que despejar su ancho con un margen.
        let docked = s.reserve.unwrap_or(self.sidebar_docked);
        let rail_reserva = docked && !s.autohide;
        let side_margin = if rail_reserva { 0 } else { thickness as i32 };
        let anchor = s.anchor;
        let output = self.panels[rail_pi].output.clone();
        // (anchor sctk, márgenes top/right/bottom/left) según el borde del sidebar.
        let (sctk_anchor, margins) = match anchor {
            pata_core::Anchor::Right => (
                LayerAnchor::RIGHT | LayerAnchor::TOP | LayerAnchor::BOTTOM,
                (0, side_margin, 0, 0),
            ),
            // Izquierda (default para un sidebar): pegado al borde izquierdo.
            _ => (
                LayerAnchor::LEFT | LayerAnchor::TOP | LayerAnchor::BOTTOM,
                (0, 0, 0, side_margin),
            ),
        };
        let layer = {
            let comp = self.compositor.as_ref().expect("compositor retenido");
            let ls = self.layer_shell.as_ref().expect("layer_shell retenido");
            let wl_surface = comp.create_surface(qh);
            let layer = ls.create_layer_surface(
                qh,
                wl_surface,
                Layer::Top,
                Some("pata-sidebar-panel".to_string()),
                output.as_ref(),
            );
            layer.set_anchor(sctk_anchor);
            layer.set_size(pw, 0); // alto 0 → el compositor lo estira a la salida.
            layer.set_margin(margins.0, margins.1, margins.2, margins.3);
            // Sin zona exclusiva propia: el drawer FLOTA sobre el contenido (como el
            // drawer del backend winit). Respeta las zonas de las barras (top/bottom)
            // y del rail, así queda alineado y a la altura correcta.
            layer.set_exclusive_zone(0);
            layer.set_keyboard_interactivity(KeyboardInteractivity::None);
            layer.commit();
            layer
        };
        self.panels.push(Panel {
            idx: si,
            card: None,
            drawer: true,
            output,
            layer,
            cache: None,
            width: pw,
            height: 1, // provisional hasta la primera `configure`.
            dirty: true,
            hover_idx: None,
            cursor_x: None,
            gpu: None,
        });
        self.drawer_pi = Some(self.panels.len() - 1);
        self.drawer_si = Some(si);
        diag!(
            "pata diag · create_drawer si={si} pi={} pw={pw} margin=(t{},r{},b{},l{}) anchor={anchor:?}",
            self.panels.len() - 1,
            margins.0,
            margins.1,
            margins.2,
            margins.3
        );
    }

    /// Destruye la surface del drawer viva (si hay). El drawer es SIEMPRE el último
    /// panel (único creado en runtime, ≤ 1 a la vez), así que `pop` no corre los
    /// índices de los paneles fijos (`osd_pi`/`tooltip_pi`/`menu_panel`/…).
    pub(super) fn destroy_drawer(&mut self) {
        let Some(pi) = self.drawer_pi.take() else { return };
        self.drawer_si = None;
        if pi >= self.panels.len() {
            return; // ya no está (defensivo).
        }
        // Soltamos la surface wgpu ANTES que la wl_surface (evita que el handle raw
        // quede colgando al dropear el `LayerSurface`).
        self.panels[pi].gpu = None;
        self.panels[pi].cache = None;
        if pi == self.panels.len() - 1 {
            self.panels.pop(); // drop del `LayerSurface` → destruye la surface.
        } else {
            // No debería pasar (el drawer es el tail), pero si otro panel se apiló
            // después, `remove` es correcto: no hay paneles fijos tras el drawer.
            self.panels.remove(pi);
        }
    }

    /// Aplica EN VIVO el eje docked de la surface `si`: cambia el `exclusive_zone`
    /// de sus layer surfaces (reserva franja `thickness` si docked y no autohide;
    /// `0` si flota) y re-renderiza. Sin re-exec — el compositor re-tesela las
    /// ventanas al recibir el commit.
    pub(super) fn aplicar_docked_sidebar(&mut self, si: usize, docked: bool) {
        let (thickness, autohide) = match self.cfg.surfaces.get(si) {
            Some(s) => (s.thickness.max(1.0) as i32, s.autohide),
            None => return,
        };
        if let Some(s) = self.cfg.surfaces.get_mut(si) {
            s.reserve = Some(docked);
        }
        let rail_reserva = docked && !autohide;
        let excl = if rail_reserva { thickness } else { 0 };
        // El drawer no reserva zona propia, pero su margen lateral depende de si el
        // rail reserva (0) o flota (thickness): al cambiar el docked en vivo hay que
        // re-anclarlo para que siga pegado al rail sin hueco ni solape.
        let side_margin = if rail_reserva { 0 } else { thickness };
        let anchor = self.cfg.surfaces.get(si).map(|s| s.anchor);
        for p in &self.panels {
            if p.idx != si {
                continue;
            }
            if p.drawer {
                match anchor {
                    Some(pata_core::Anchor::Right) => p.layer.set_margin(0, side_margin, 0, 0),
                    _ => p.layer.set_margin(0, 0, 0, side_margin),
                }
            } else {
                p.layer.set_exclusive_zone(excl);
            }
            p.layer.commit();
        }
        self.marcar_sidebars_dirty();
    }

    /// Cierra el panel del sidebar (si alguno está abierto).
    pub(super) fn cerrar_sidebar(&mut self) {
        if let Some((si, ti)) = self.nav.open {
            self.set_sidebar_open(si, ti);
        }
    }

    /// Expande/colapsa un nodo del navegador.
    pub(super) fn nav_toggle(&mut self, id: u64) {
        if self.nav.expanded.contains(&id) {
            self.nav.expanded.remove(&id);
        } else {
            self.nav.expanded.insert(id);
            if let (Some(mid), Some(sock)) =
                (self.nav.needs_resolve(id), self.nav.socket.clone())
            {
                let tx = self.members_tx.clone();
                std::thread::spawn(move || {
                    let _ = tx.send(crate::nouser::resolve(sock, mid));
                });
            }
        }
        self.marcar_sidebars_dirty();
    }

    /// Recoge el último snapshot del hilo de muestreo. Incluye hot-reload de config.
    pub(super) fn maybe_recargar_config(&mut self) -> bool {
        if !self.cfg_watch.changed() {
            return false;
        }
        let cfg = pata_config::load();
        // Comparamos el conteo de superficies ENCENDIDAS: agregar/quitar una
        // barra O prenderla/apagarla cambia cuántas layer surfaces hay que
        // anclar → re-exec. (Editar dientes dentro de una barra: hot-reload.)
        let enc = |c: &pata_core::Config| c.surfaces.iter().filter(|s| s.enabled).count();
        if enc(&cfg) != enc(&self.cfg) {
            // Cambió la CANTIDAD de superficies (p. ej. vista mac/mirada con
            // 2 superficies vs. una vista de 1): no se pueden reanclar layer
            // surfaces en caliente sin recrearlas. La vía limpia: re-ejecutar
            // pata en el mismo proceso (`exec`), que arranca leyendo el nuevo
            // launcher.toml y ancla las superficies correctas. Sin esto, cambiar
            // a mac/mirada "no hacía nada" (el reload se descartaba).
            self.re_exec_pata("cambió la cantidad de superficies");
            return false;
        }
        self.surfaces = crate::Model::construir_surfaces(&cfg);
        let mut theme = llimphi_theme::Theme::dark();
        if let Some(c) = crate::render::parse_hex(&cfg.general.accent) {
            theme.accent = c;
        }
        self.theme = theme;
        self.cfg = cfg;
        true
    }

    /// Re-ejecuta pata en el mismo proceso (`exec`) para reanclar las layer
    /// surfaces cuando un cambio no se puede aplicar en caliente. Sólo retorna si
    /// el exec falló.
    pub(super) fn re_exec_pata(&self, motivo: &str) {
        eprintln!("pata · {motivo}; re-ejecutando para reanclar las layer surfaces.");
        if let Ok(exe) = std::env::current_exe() {
            use std::os::unix::process::CommandExt;
            let args: Vec<String> = std::env::args().skip(1).collect();
            let err = std::process::Command::new(exe).args(args).exec();
            eprintln!("pata · re-exec falló: {err}; reiniciá pata a mano.");
        }
    }

    pub(super) fn maybe_sample(&mut self) {
        let Some((mut ctx, clipboard)) = self.sampler.latest() else {
            return;
        };
        // Sostiene el realce optimista del switcher hasta que el muestreo
        // confirme el salto (un sample viejo reportaría el escritorio anterior y
        // parpadearía). Misma lógica pura que el backend winit.
        let (pending, active) =
            crate::sampler::reconcile_optimistic(self.pending_ws, ctx.active_workspace);
        self.pending_ws = pending;
        ctx.active_workspace = active;
        self.maybe_recargar_config();
        // Los toggles GLOBALes de sidebar (WawaConfig) cambian el anclaje de los
        // rails → hay que reanclar. Re-exec al detectar cambio en cualquiera de los
        // dos ejes: `dientes_outside` (posición del rail) o `sidebar_docked`
        // (reserva de franja / exclusive_zone).
        let wcfg = wawa_config::WawaConfig::load();
        if wcfg.dientes_outside != self.dientes_outside {
            self.dientes_outside = wcfg.dientes_outside;
            self.re_exec_pata("cambió la posición del rail (dientes adentro/afuera)");
        }
        if wcfg.sidebar_docked != self.sidebar_docked {
            self.sidebar_docked = wcfg.sidebar_docked;
            self.re_exec_pata("cambió el docked del sidebar (reserva de franja)");
        }
        self.ctx = ctx;
        if crate::push_clip_history(&mut self.clip_history, &clipboard) {
            if let Some(t) = &clipboard {
                willay_emit::emitir_silencioso(&crate::evento_clip(t, willay_emit::ahora_usec()));
            }
        }
        self.clipboard = clipboard;
        if let Some(h) = &self.weather {
            if let Some(w) = h.latest() {
                self.weather_now = Some(w);
            }
        }
        if let Some(h) = &self.network {
            if let Some(n) = h.latest() {
                self.network_now = Some(n);
            }
        }
        if let Some(h) = &self.mpris {
            if let Some(m) = h.latest() {
                self.media_now = Some(m);
            }
        }
        if let Some(h) = &self.bluetooth {
            if let Some(b) = h.latest() {
                self.bluetooth_now = Some(b);
            }
        }
        if let Some(h) = &self.unidades {
            if let Some(s) = h.latest() {
                self.unidades_now = Some(s);
            }
        }
        if let Some(h) = &self.flota_discover {
            if let Some(v) = h.latest() {
                self.flota_remoto = Some(v);
            }
        }
        // Mezclador: refresca mientras su popup está abierto (sliders en vivo).
        if self.menu_open && self.menu_kind == MenuKind::Volume {
            self.sink_inputs = crate::sampler::sample_sink_inputs();
            self.sinks = crate::sampler::sample_sinks();
        }
        // Aviso de batería baja (una vez por escalón al descargar).
        if let Some((pct, charging)) = crate::bateria::read() {
            let (nuevo, aviso) = crate::bateria::decidir(pct, charging, self.bat_avisado);
            self.bat_avisado = nuevo;
            self.bat_now = Some((pct as f32 / 100.0, charging));
            if let Some(a) = aviso {
                crate::bateria::avisar(a, pct);
            }
        }
        self.cpu_temp = crate::sampler::cpu_temp_celsius();
        // El control center persistente necesita perfil de energía + luz nocturna
        // frescos (el flyout los leía sólo al abrirse).
        if crate::config_tiene_diente_vivo(&self.cfg) {
            let (pp, night) = crate::render::read_power_night();
            self.control_extras.power_profile = pp;
            self.control_extras.night = night;
        }
        // Diente vivo: refresca su manifestación con las señales nuevas.
        self.actualizar_diente();
        // `WidgetCtx` ya no es `Copy` (lleva el título de la ventana enfocada),
        // así que los widgets tickean contra `&self.ctx` (recién asignado).
        for sw in &mut self.surfaces {
            for w in sw.core_mut() {
                w.tick(&self.ctx);
            }
        }
        for p in &mut self.panels {
            if let Some(c) = p.card.as_mut() {
                for w in &mut c.widgets {
                    w.tick(&self.ctx);
                }
            }
            p.dirty = true;
        }
    }

    /// Arma la notificación de inactividad si el compositor la expone y el idle
    /// de energía está habilitado. Idempotente: no re-crea si ya hay una viva.
    pub(super) fn ensure_idle_arm(&mut self, qh: &QueueHandle<Self>) {
        if self.idle_notif.is_some() || !self.energia_cfg.habilitado {
            return;
        }
        let secs = self.energia_cfg.suspender_secs;
        if secs > 0 {
            self.armar_idle(secs, qh);
        }
    }

    /// (Re)crea la notificación de inactividad con `secs` de timeout. Necesita
    /// notifier + seat; cae al primer seat conocido si `self.seat` aún es `None`
    /// (mismo fallback que `activar_ventana`).
    fn armar_idle(&mut self, secs: u32, qh: &QueueHandle<Self>) {
        let Some(notifier) = self.idle_notifier.clone() else {
            return;
        };
        let seat = self
            .seat
            .clone()
            .or_else(|| self.seat_state.seats().next());
        let Some(seat) = seat else {
            return;
        };
        if let Some(old) = self.idle_notif.take() {
            old.destroy();
        }
        let notif = notifier.get_idle_notification(secs.saturating_mul(1000), &seat, qh, ());
        self.idle_notif = Some(notif);
    }

    /// El sistema cumplió el umbral de inactividad: consulta el veto (unidades
    /// del plano de control + carga del sistema) y suspende, **pospone** (con
    /// aviso del motivo) o no hace nada según la política.
    pub(super) fn energia_al_ociar(&mut self, qh: &QueueHandle<Self>) {
        if self.energia_disparado {
            return;
        }
        // Hay batería y NO está cargando = corriendo con batería.
        let en_bateria = matches!(self.bat_now, Some((_, false)));
        let bloqueos =
            crate::energia::reunir_bloqueos(self.unidades_now.as_ref(), &self.energia_cfg);
        let accion = crate::energia::decidir(
            &self.energia_cfg,
            crate::energia::Nivel::Suspender,
            en_bateria,
            &bloqueos,
        );
        match accion {
            crate::energia::Accion::Suspender | crate::energia::Accion::Apagar => {
                crate::energia::ejecutar(&accion, false);
                self.energia_disparado = true;
            }
            crate::energia::Accion::Posponer { .. } => {
                // Avisar el motivo una sola vez; reintentar más tarde si la
                // inactividad sigue (el trabajo puede terminar y entonces sí
                // conviene suspender).
                crate::energia::ejecutar(&accion, !self.energia_pospuesto);
                self.energia_pospuesto = true;
                self.armar_idle(super::REINTENTO_ENERGIA_SECS, qh);
            }
            crate::energia::Accion::Nada => {}
        }
    }

    /// Reconcilia el inhibidor de inactividad del compositor con el estado del
    /// café: lo crea cuando se enciende (pausa apagado-de-pantalla y bloqueo en
    /// mirada) y lo destruye al apagarlo. Idempotente.
    pub(super) fn ensure_cafe_inhibitor(&mut self, qh: &QueueHandle<Self>) {
        use smithay_client_toolkit::shell::WaylandSurface;
        let quiere = self.energia_cfg.cafe;
        if quiere == self.idle_inhibitor.is_some() {
            return;
        }
        if quiere {
            let Some(mgr) = self.idle_inhibit_mgr.clone() else {
                return;
            };
            let Some(panel) = self.panels.first() else {
                return;
            };
            let inh = mgr.create_inhibitor(panel.layer.wl_surface(), qh, ());
            self.idle_inhibitor = Some(inh);
        } else if let Some(inh) = self.idle_inhibitor.take() {
            inh.destroy();
        }
    }

    /// El usuario volvió (hubo actividad): reinicia el ciclo del idle de energía.
    pub(super) fn energia_al_volver(&mut self, qh: &QueueHandle<Self>) {
        self.energia_disparado = false;
        self.energia_pospuesto = false;
        let secs = self.energia_cfg.suspender_secs;
        if self.energia_cfg.habilitado && secs > 0 {
            self.armar_idle(secs, qh);
        }
    }

    /// Drena el último cuadro del visualizador (cava).
    pub(super) fn maybe_cava(&mut self) {
        let Some(h) = &self.cava else {
            return;
        };
        let Some(frame) = h.latest() else {
            return;
        };
        self.cava_frame = frame;
        for p in &mut self.panels {
            if p.card.is_none() {
                p.dirty = true;
            }
        }
    }

    /// Arma las [`pata_core::atencion::Senales`] del diente vivo desde el estado
    /// actual: volumen/mute/CPU del `WidgetCtx`, batería de `bat_now`, música de
    /// `media_now`.
    fn senales_diente(&self) -> pata_core::atencion::Senales {
        pata_core::atencion::Senales {
            volume: self.ctx.volume,
            muted: self.ctx.muted,
            cpu: self.ctx.cpu,
            cpu_temp: self.cpu_temp,
            bateria: self.bat_now.map(|(f, _)| f),
            cargando: self.bat_now.map(|(_, c)| c).unwrap_or(false),
            musica: self.media_now.as_ref().map(|m| m.playing).unwrap_or(false),
        }
    }

    /// Refresca la manifestación del diente vivo (señales frescas → árbitro).
    pub(super) fn actualizar_diente(&mut self) {
        let s = self.senales_diente();
        let now = self.diente_t0.elapsed().as_secs_f64();
        self.diente_manifest = self.atencion.update(s, now);
    }

    /// Detecta el cambio de escritorio activo y arranca/expira la animación del
    /// switcher (el resaltado que viaja de la celda vieja a la nueva).
    pub(super) fn update_ws_anim(&mut self) {
        let cur = self.ctx.active_workspace;
        if cur != 0 && self.ws_last_active != 0 && cur != self.ws_last_active {
            // Arranca desde donde estábamos (o desde el destino de una cometa aún
            // en vuelo, si el usuario encadena cambios rápidos).
            let from = self
                .ws_anim
                .map(|a| a.to)
                .unwrap_or(self.ws_last_active);
            self.ws_anim = Some(crate::layer::WsAnimState {
                from,
                to: cur,
                start: std::time::Instant::now(),
            });
        }
        if cur != 0 {
            self.ws_last_active = cur;
        }
        if let Some(a) = self.ws_anim {
            if a.start.elapsed() >= crate::layer::WS_ANIM {
                self.ws_anim = None;
            }
        }
    }

    /// Fase de apertura del menú de inicio `0..1` (0 = recién abierto, 1 =
    /// asentado). `1.0` si no hay menú abierto. Mueve el fade + slide de entrada.
    pub(super) fn menu_open_t(&self) -> f32 {
        match self.menu_opened_at {
            Some(t) => (t.elapsed().as_secs_f32() / crate::layer::MENU_OPEN.as_secs_f32())
                .clamp(0.0, 1.0),
            None => 1.0,
        }
    }

    /// La cometa del switcher para este frame (posición interpolada de la cabeza),
    /// o `None` si no hay animación en curso.
    pub(super) fn ws_comet(&self) -> Option<render::WsComet> {
        let a = self.ws_anim?;
        let dur = crate::layer::WS_ANIM.as_secs_f32();
        let t = (a.start.elapsed().as_secs_f32() / dur).clamp(0.0, 1.0);
        let e = 1.0 - (1.0 - t).powi(3); // ease-out cúbico
        let from = a.from as f32 - 1.0;
        let to = a.to as f32 - 1.0;
        Some(render::WsComet {
            head: from + (to - from) * e,
            dir: if to >= from { 1.0 } else { -1.0 },
        })
    }

    /// Crea el estado wgpu de un panel.
    pub(super) fn ensure_gpu(&mut self, pi: usize) {
        if self.panels[pi].gpu.is_some() {
            return;
        }
        let display_ptr = self.conn.backend().display_ptr() as *mut c_void;
        let surface_ptr = self.panels[pi].layer.wl_surface().id().as_ptr() as *mut c_void;
        let (w, h) = (self.panels[pi].width, self.panels[pi].height);
        let display_handle = RawDisplayHandle::Wayland(WaylandDisplayHandle::new(
            NonNull::new(display_ptr).expect("wl_display ptr"),
        ));
        let window_handle = RawWindowHandle::Wayland(WaylandWindowHandle::new(
            NonNull::new(surface_ptr).expect("wl_surface ptr"),
        ));
        // SAFETY: los handles apuntan a objetos Wayland que `self` mantiene vivos.
        let make_target = || wgpu::SurfaceTargetUnsafe::RawHandle {
            raw_display_handle: display_handle,
            raw_window_handle: window_handle,
        };

        let surface = if self.hal.is_none() {
            match pollster::block_on(unsafe { Hal::new_for_raw_surface(make_target, w, h) }) {
                Ok((hal, surface)) => {
                    self.hal = Some(hal);
                    surface
                }
                Err(e) => {
                    eprintln!("pata layer · panel {pi} sin gpu: {e}");
                    return;
                }
            }
        } else {
            let hal = self.hal.as_ref().expect("hal");
            let wgpu_surface = match unsafe { hal.instance.create_surface_unsafe(make_target()) } {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("pata layer · panel {pi} sin gpu: {e}");
                    return;
                }
            };
            match RawSurface::from_surface(hal, wgpu_surface, display_handle, window_handle, w, h) {
                Ok(s) => s,
                Err(e) => {
                    eprintln!("pata layer · panel {pi} sin gpu: {e}");
                    return;
                }
            }
        };
        let hal = self.hal.as_ref().expect("hal");
        diag!(
            "pata diag · panel {pi} surface creada {w}x{h} · backend={:?} format={:?}",
            hal.adapter.get_info().backend,
            surface.format(),
        );
        let renderer = Renderer::new(hal).expect("renderer");
        self.panels[pi].gpu = Some(PanelGpu {
            surface,
            renderer,
            typesetter: llimphi_ui::llimphi_text::Typesetter::new(),
            scene: vello::Scene::new(),
            layout: llimphi_ui::llimphi_layout::LayoutTree::new(),
        });
    }

    /// Mantiene vivo el latido de un panel: pide su siguiente frame-callback.
    pub(super) fn latido(&self, pi: usize, qh: &QueueHandle<Self>) {
        let surface = self.panels[pi].layer.wl_surface();
        surface.frame(qh, surface.clone());
        surface.commit();
    }

    /// Avanza el frame de un panel.
    pub(super) fn draw(&mut self, pi: usize, qh: &QueueHandle<Self>) {
        // Reconciliar el drawer del sidebar (crear/destruir su surface) según
        // `nav.open`. Lo hacemos SÓLO desde los paneles fijos (rails/barras, que
        // laten en continuo), nunca desde el propio drawer: así reconcile jamás
        // toca el panel que estamos por dibujar. El `pi >= len` es una red por si
        // otro camino lo destruyó.
        if !self.panels[pi].drawer {
            self.reconcile_drawer(qh);
        }
        if pi >= self.panels.len() {
            return;
        }
        // Empuje del OSD: su surface arranca 1×1 y podría no recibir frames
        // propios; las barras (que laten en continuo) sirven su draw cuando hay
        // un cartel que mostrar o que encoger. (`pi != osd_pi` evita recursión.)
        if self.osd_pi.is_some() && self.osd_pi != Some(pi) {
            let opi = self.osd_pi.unwrap();
            let needs = self.panels[opi].dirty
                || self.osd.is_some()
                || self.panels[opi].width > 1;
            if needs {
                self.draw(opi, qh);
            }
        }
        self.maybe_sample();
        self.ensure_idle_arm(qh);
        self.ensure_cafe_inhibitor(qh);
        self.maybe_cava();
        self.poll_nav();
        self.poll_host();
        self.poll_polkit();
        // Animación del switcher: si cambió el escritorio, el resaltado viaja.
        // Mientras dura, mantené el panel pintándose para animar suave.
        self.update_ws_anim();
        let ws_anim = self.ws_comet();
        if ws_anim.is_some() {
            self.panels[pi].dirty = true;
        }
        // Diente vivo: re-resolvé la manifestación cada frame (para que los
        // transitorios caduquen suave) y mantené latiendo el panel del sidebar con
        // un diente vivo —incluso en reposo, para la respiración ambiental del halo—.
        {
            let s = self.senales_diente();
            let now = self.diente_t0.elapsed().as_secs_f64();
            self.diente_manifest = self.atencion.resolver(s, now);
        }
        {
            let idx = self.panels[pi].idx;
            let es_sidebar_animado = self
                .cfg
                .surfaces
                .get(idx)
                .map(|s| {
                    s.kind == SurfaceKind::Sidebar
                        && s.tabs.iter().any(|t| {
                            crate::es_diente_vivo(&t.content.kind)
                                || crate::es_monitor(&t.content.kind)
                                || crate::es_unidades(&t.content.kind)
                        })
                })
                .unwrap_or(false);
            if es_sidebar_animado {
                self.panels[pi].dirty = true;
            }
        }
        // Mientras el menú de inicio entra (fade + slide), repintá su panel.
        if self.menu_open && self.menu_panel == Some(pi) && self.menu_open_t() < 1.0 {
            self.panels[pi].dirty = true;
        }
        self.ensure_gpu(pi);

        // Shell hospedado: avanza solo.
        if self.shuma_panel == Some(pi) {
            if self.shuma_full.is_some() {
                // Live-wire: drenar los Msg que la shuma completa empujó al canal
                // (ticks/async/follow-ups) y aplicarlos. Repinta si está abierto.
                self.drain_shuma_full(pi);
            } else if self.shuma.open {
                self.shuma.inner = shuma_module_shell::update(
                    self.shuma.inner.clone(),
                    shuma_module_shell::Msg::Tick,
                );
                self.panels[pi].dirty = true;
            }
        }

        // El panel del OSD crece al dispararse (volumen/brillo) y se encoge al
        // expirar; mantiene su latido para reaparecer sin recrear la surface.
        if self.osd_pi == Some(pi) {
            let visible = self.osd.map(|o| !o.expired()).unwrap_or(false);
            let target = if visible {
                (render::OSD_W, render::OSD_H)
            } else {
                (1u32, 1u32)
            };
            if (self.panels[pi].width, self.panels[pi].height) != target {
                {
                    let layer = &self.panels[pi].layer;
                    layer.set_size(target.0, target.1);
                    layer.commit();
                }
                self.panels[pi].width = target.0;
                self.panels[pi].height = target.1;
                self.panels[pi].cache = None;
                self.panels[pi].dirty = true;
            }
            if !visible {
                // Expiró (o nunca se mostró): suelta el cartel. NO retornamos sin
                // pintar —eso dejaría el último buffer 240×60 pegado a la surface
                // (bug)—: caemos al render con la vista vacía de abajo para
                // presentar un frame 1×1 limpio (como `hide_tooltip`). Si ya estaba
                // en 1×1 y no quedó sucio, el chequeo de `dirty` corta sin re-pintar.
                self.osd = None;
            }
        }

        if !self.panels[pi].dirty {
            self.latido(pi, qh);
            return;
        }

        let idx = self.panels[pi].idx;
        let (w, h) = (self.panels[pi].width, self.panels[pi].height);
        let windows = self.window_entries();
        let tray_items = self.tray.as_ref().map(|t| t.items()).unwrap_or_default();
        let notif = self.notifications.as_ref().map(|n| n.snapshot());
        let data = render::BarData {
            windows: &windows,
            clipboard: self.clipboard.as_deref(),
            tray: &tray_items,
            weather: self.weather_now.as_ref(),
            network: self.network_now.as_ref(),
            media: self.media_now.as_ref(),
            bluetooth: self.bluetooth_now.as_ref(),
            notifications: notif.as_ref(),
            cava: &self.cava_frame,
            apps: self.registry.all(),
            shuma_full: self.shuma_full.as_ref(),
            workspace: (
                self.ctx.active_workspace,
                self.ctx.workspace_count,
                self.ctx.workspace_occupied,
            ),
            clock: (self.ctx.clock.hour, self.ctx.clock.minute),
            // En la barra real los botones de ventana se reordenan arrastrándolos.
            reorderable_tasks: true,
            ws_anim,
        };

        let view = if self.osd_pi == Some(pi) {
            // Con cartel vigente, lo pintamos; al expirar, una vista vacía limpia
            // el frame 1×1 (NO un `bar_view`, que metería la barra en la surface
            // del OSD).
            match self.osd {
                Some(osd) => render::osd_surface_view(&osd, &self.theme),
                None => llimphi_ui::View::new(Default::default()),
            }
        } else if self.tooltip_pi == Some(pi) {
            render::tooltip_view(self.tooltip_text.as_deref().unwrap_or(""), &self.theme)
        } else if let Some(c) = self.panels[pi].card.as_ref() {
            render::card_view(&c.spec, &c.widgets, &self.theme)
        } else if self.menu_panel == Some(pi) && self.menu_open {
            match self.menu_kind {
                MenuKind::Apps => render::start_menu_view(
                    &self.cfg.surfaces[idx],
                    &self.surfaces[idx],
                    &self.shuma,
                    &data,
                    &self.theme,
                    self.menu_bar_px as f32,
                    self.registry.all(),
                    &self.menu_query,
                    self.menu_scroll,
                    h as f32,
                    // El estilo del menú lo fija la vista vía la config de pata.
                    crate::MenuStyle::from_cfg(&self.cfg.general.menu_style),
                    self.cfg.general.menu_columns,
                    self.menu_cat,
                    self.menu_open_t(),
                ),
                MenuKind::Clipboard => render::clipboard_menu_view(
                    &self.cfg.surfaces[idx],
                    &self.surfaces[idx],
                    &self.shuma,
                    &data,
                    &self.theme,
                    self.menu_bar_px as f32,
                    &self.clip_history,
                    // Ancla bajo el widget que lo abrió (último x del puntero en
                    // esa barra), acotado al ancho de la barra.
                    self.panels[idx].cursor_x.unwrap_or(self.panels[idx].width as f32 * 0.5),
                    self.panels[idx].width as f32,
                ),
                MenuKind::Clock => render::clock_menu_view(
                    &self.cfg.surfaces[idx],
                    &self.surfaces[idx],
                    &self.shuma,
                    &data,
                    &self.theme,
                    self.menu_bar_px as f32,
                    &self.clock_draft,
                ),
                MenuKind::Control => render::control_menu_view(
                    &self.cfg.surfaces[idx],
                    &self.surfaces[idx],
                    &self.shuma,
                    &data,
                    &self.theme,
                    self.menu_bar_px as f32,
                    self.ctx.volume,
                    self.ctx.muted,
                    self.ctx.brightness,
                    &self.control_extras,
                    self.panels[idx].cursor_x.unwrap_or(self.panels[idx].width as f32 * 0.5),
                    self.panels[idx].width as f32,
                ),
                MenuKind::Network => render::network_menu_view(
                    &self.cfg.surfaces[idx],
                    &self.surfaces[idx],
                    &self.shuma,
                    &data,
                    &self.theme,
                    self.menu_bar_px as f32,
                    self.network_now.as_ref(),
                    self.net_password.as_ref().map(|(s, p)| (s.as_str(), p.as_str())),
                    self.panels[idx].cursor_x.unwrap_or(self.panels[idx].width as f32 * 0.5),
                    self.panels[idx].width as f32,
                ),
                MenuKind::Volume => render::volume_menu_view(
                    &self.cfg.surfaces[idx],
                    &self.surfaces[idx],
                    &self.shuma,
                    &data,
                    &self.theme,
                    self.menu_bar_px as f32,
                    &self.ctx,
                    &self.sinks,
                    &self.sink_inputs,
                    self.panels[idx].cursor_x.unwrap_or(self.panels[idx].width as f32 * 0.5),
                    self.panels[idx].width as f32,
                ),
                MenuKind::Session => render::session_menu_view(
                    &self.cfg.surfaces[idx],
                    &self.surfaces[idx],
                    &self.shuma,
                    &data,
                    &self.theme,
                    self.menu_bar_px as f32,
                    self.session_confirm,
                    self.panels[idx].cursor_x.unwrap_or(self.panels[idx].width as f32 * 0.5),
                    self.panels[idx].width as f32,
                ),
                MenuKind::Bluetooth => render::bluetooth_menu_view(
                    &self.cfg.surfaces[idx],
                    &self.surfaces[idx],
                    &self.shuma,
                    &data,
                    &self.theme,
                    self.menu_bar_px as f32,
                    self.bluetooth_now.as_ref(),
                    self.panels[idx].cursor_x.unwrap_or(self.panels[idx].width as f32 * 0.5),
                    self.panels[idx].width as f32,
                ),
                MenuKind::Notifications => render::notifications_menu_view(
                    &self.cfg.surfaces[idx],
                    &self.surfaces[idx],
                    &self.shuma,
                    &data,
                    &self.theme,
                    self.menu_bar_px as f32,
                    notif.as_ref(),
                    self.panels[idx].cursor_x.unwrap_or(self.panels[idx].width as f32 * 0.5),
                    self.panels[idx].width as f32,
                ),
                MenuKind::Polkit => render::polkit_menu_view(
                    &self.cfg.surfaces[idx],
                    &self.surfaces[idx],
                    &self.shuma,
                    &data,
                    &self.theme,
                    self.menu_bar_px as f32,
                    self.polkit_prompt.as_ref().map(|r| r.message.as_str()).unwrap_or(""),
                    &self.polkit_input,
                    self.panels[idx].width as f32,
                ),
            }
        } else if self.shuma_panel == Some(pi) && self.shuma.open {
            render::shuma_open_view(
                &self.cfg.surfaces[idx],
                &self.surfaces[idx],
                &self.shuma,
                &data,
                &self.theme,
                self.shuma_bar_px as f32,
                // Alto del drawer = fracción configurable de la pantalla
                // (general.shuma_height). `h` es el alto de la superficie (=
                // pantalla, ya que al abrir crece a 10_000 y el compositor la
                // capa). Cae a DRAWER_H si la superficie aún no se configuró.
                {
                    // Maximizado (botón ▢ de la barra de título) → casi pantalla
                    // completa; si no, la fracción configurable.
                    let frac = if self.shuma.maximized {
                        0.95
                    } else {
                        self.cfg.general.shuma_height.clamp(0.1, 0.95)
                    };
                    if h > self.shuma_bar_px + 10 {
                        h as f32 * frac
                    } else {
                        DRAWER_H as f32
                    }
                },
            )
        } else if self.cfg.surfaces[idx].kind == SurfaceKind::Sidebar {
            let hosted = {
                let app = self.focused_app_id().map(|s| s.to_string());
                match (app, self.host.as_ref()) {
                    (Some(id), Some(h)) => {
                        h.snapshot(&id).map(|(_, teeth, active)| (id, teeth, active))
                    }
                    _ => None,
                }
            };
            let (hosted_app, hosted_teeth, hosted_active): (&str, &[pata_host::HostedTooth], Option<u32>) =
                match &hosted {
                    Some((id, teeth, active)) => (id.as_str(), teeth.as_slice(), *active),
                    None => ("", &[], None),
                };
            let vivo = render::DienteVivo {
                manifest: self.diente_manifest,
                cava_frame: &self.cava_frame,
                ctx: &self.ctx,
                unidades: self.unidades_now.as_ref(),
                flota_remoto: self.flota_remoto.as_deref(),
                t: self.diente_t0.elapsed().as_secs_f64(),
            };
            let extras = render::extras_vivos(
                self.bat_now,
                self.network_now
                    .as_ref()
                    .map(|n| n.wifi_enabled)
                    .unwrap_or(self.control_extras.wifi),
                self.bluetooth_now
                    .as_ref()
                    .map(|b| b.powered)
                    .unwrap_or(self.control_extras.bt),
                &self.control_extras,
            );
            let centro = render::CentroDatos {
                ctx: &self.ctx,
                extras: &extras,
                media: self.media_now.as_ref(),
                net: self.network_now.as_ref(),
                net_password: self
                    .net_password
                    .as_ref()
                    .map(|(s, p)| (s.as_str(), p.as_str())),
                bt: self.bluetooth_now.as_ref(),
                flota: self.flota.as_ref(),
                flota_remoto: self.flota_remoto.as_deref(),
                unidades: self.unidades_now.as_ref(),
            };
            // Estado EFECTIVO de los dos ejes de esta surface, para que la barrita
            // muestre los switches en su posición actual: el override por-sidebar
            // (`reserve`/`rail_outside`) gana; si es `None`, el global.
            let s = &self.cfg.surfaces[idx];
            let docked_ef = s.reserve.unwrap_or(self.sidebar_docked);
            let rail_outside_ef = s.rail_outside.unwrap_or(self.dientes_outside);
            if self.panels[pi].drawer {
                // El **drawer**: sólo la barrita + el contenido del diente, a ancho
                // fijo `panel_width`. El rail vive en su propia surface aparte.
                let ti = self.nav.open.map(|(_, ti)| ti).unwrap_or(0);
                render::sidebar_drawer_view(
                    &self.cfg.surfaces[idx],
                    idx,
                    ti,
                    w as f32,
                    h as f32,
                    &self.nav,
                    &self.shuma,
                    &self.rag,
                    &centro,
                    docked_ef,
                    rail_outside_ef,
                    &self.theme,
                )
            } else {
                // El **rail**: sólo la franja de dientes (ya no crece para alojar el
                // panel; de eso se encarga el drawer).
                render::sidebar_surface_view(
                    &self.cfg.surfaces[idx],
                    idx,
                    w as f32,
                    h as f32,
                    &self.nav,
                    hosted_teeth,
                    hosted_app,
                    hosted_active,
                    &self.shuma,
                    &vivo,
                    &self.theme,
                )
            }
        } else if self.cfg.surfaces[idx].kind == SurfaceKind::Dock {
            // Dock estilo macOS: apps fijadas (lanzadores) + ventanas abiertas,
            // magnificados por el puntero. Los pins se resuelven en el registro;
            // los que no existan se omiten.
            let pins: Vec<app_bus::AppEntry> = self.cfg.surfaces[idx]
                .dock_pins
                .iter()
                .filter_map(|id| self.registry.get(id).cloned())
                .collect();
            render::dock_view(
                &self.cfg.surfaces[idx],
                &pins,
                &windows,
                &self.theme,
                w as f32,
                self.panels[pi].cursor_x,
            )
        } else if self.cfg.surfaces[idx].kind == SurfaceKind::Background {
            // Fondo de escritorio (capa Background): llena la pantalla.
            render::background_view(
                &self.cfg.surfaces[idx],
                &self.surfaces[idx],
                &self.shuma,
                &data,
                &self.theme,
            )
        } else {
            render::bar_view(
                &self.cfg.surfaces[idx],
                &self.surfaces[idx],
                &self.shuma,
                &data,
                &self.theme,
            )
        };

        let hover_idx = self.panels[pi].hover_idx;
        let hal = self.hal.as_ref().expect("hal");
        let gpu = match self.panels[pi].gpu.as_mut() {
            Some(g) => g,
            None => {
                self.latido(pi, qh);
                return;
            }
        };
        gpu.surface.resize(w, h);
        let frame = match gpu.surface.acquire() {
            Ok(f) => f,
            Err(_) => {
                let _ = gpu;
                self.latido(pi, qh);
                return;
            }
        };
        gpu.layout.clear();
        let mounted = mount(&mut gpu.layout, view);
        let computed = {
            let ts = &mut gpu.typesetter;
            let tmap = &mounted.text_measures;
            gpu.layout
                .compute_with_measure(mounted.root, (w as f32, h as f32), |nid, known, avail| {
                    match tmap.get(&nid) {
                        Some(tm) => measure_text_node(ts, tm, known, avail),
                        None => taffy::Size::ZERO,
                    }
                })
                .expect("layout")
        };
        gpu.scene.reset();
        paint(&mut gpu.scene, &mounted, &computed, &mut gpu.typesetter, hover_idx, None);
        if let Err(e) = gpu.renderer.render(hal, &gpu.scene, &frame, palette::css::TRANSPARENT) {
            eprintln!("pata layer · render: {e}");
        }
        gpu.surface.present(frame, hal);
        diag!("pata diag · present panel {pi} {w}x{h}");

        self.panels[pi].dirty = false;
        self.panels[pi].cache = Some(RenderCache { mounted, computed });
        self.latido(pi, qh);
    }

    /// Aplica el `Msg` que produjo un click.
    pub(super) fn handle_msg(&mut self, msg: Msg) {
        match msg {
            Msg::ShumaToggle => self.set_shuma_open(!self.shuma.open),
            Msg::ShumaAutoClose => {
                // Deshover: replegá salvo el evento espurio de recién abierto.
                let churn = self
                    .shuma_opened_at
                    .is_some_and(|t| t.elapsed() < super::MENU_LEAVE_GRACE);
                if self.shuma.open && !churn {
                    self.set_shuma_open(false);
                }
            }
            Msg::ShumaMaximize => {
                self.shuma.maximized = !self.shuma.maximized;
                self.marcar_shuma_dirty();
            }
            Msg::ShumaUndock => {
                // Desacople real ("mover de verdad"): la sesión embebida se va a
                // un shuma standalone con su scrollback (handoff), cwd e
                // historial, y el drawer queda en limpio — ya no duplica.
                crate::undock_shuma_session(&mut self.shuma.inner);
                self.set_shuma_open(false);
            }
            Msg::ShumaShell(m) => {
                let focusing = matches!(m, shuma_module_shell::Msg::FocusInput);
                self.shuma.inner = shuma_module_shell::update(self.shuma.inner.clone(), m);
                if focusing && !self.shuma.open {
                    self.set_shuma_open(true);
                }
                self.marcar_shuma_dirty();
            }
            // Live-wire: click sobre la shuma completa (cuerpo o input de la
            // barra). `apply_shuma_full` ya abre el drawer ante un FocusInput.
            Msg::ShumaFull(m) => {
                self.apply_shuma_full(vec![m.0]);
                self.marcar_shuma_dirty();
            }
            Msg::Spawn(cmd) => crate::spawn_cmd(&cmd),
            Msg::VolumeWheel(dy) => {
                // Rueda arriba = subir. El stack entrega dy>0 al rodar hacia
                // abajo, así que invertimos: scroll-up (dy<0) sube el volumen.
                if dy != 0.0 {
                    crate::sampler::nudge_volume(dy < 0.0);
                    self.refresh_volume_now();
                    self.flash_osd(crate::render::OsdKind::Volume, self.ctx.volume, self.ctx.muted);
                }
            }
            Msg::VolumeMute => {
                crate::sampler::toggle_mute();
                self.flash_osd(crate::render::OsdKind::Volume, self.ctx.volume, !self.ctx.muted);
            }
            Msg::VolumeSet(f) => {
                crate::sampler::set_volume(f);
                self.flash_osd(crate::render::OsdKind::Volume, f, false);
            }
            Msg::VolumePanel => {
                // Antes lanzaba pavucontrol externo; ahora el mezclador nativo.
                if !(self.menu_open && self.menu_kind == MenuKind::Volume) {
                    self.sink_inputs = crate::sampler::sample_sink_inputs();
                    self.sinks = crate::sampler::sample_sinks();
                }
                self.toggle_menu(MenuKind::Volume);
            }
            Msg::SinkInputVolume(index, frac) => {
                crate::sampler::set_sink_input_volume(index, frac);
            }
            Msg::SinkInputMute(index) => crate::sampler::toggle_sink_input_mute(index),
            Msg::SinkSelect(name) => {
                crate::sampler::set_default_sink(&name);
                // Refleja al toque el nuevo default (la marca ●) sin esperar al
                // próximo refresco del panel.
                for s in &mut self.sinks {
                    s.is_default = s.name == name;
                }
            }
            Msg::SessionToggle => {
                self.session_confirm = None;
                self.toggle_menu(MenuKind::Session);
            }
            Msg::SessionConfirm(a) => {
                self.session_confirm = Some(a);
                self.marcar_menu_dirty();
            }
            Msg::SessionCancel => {
                self.session_confirm = None;
                self.marcar_menu_dirty();
            }
            Msg::SessionRun(a) => {
                crate::run_session_action(a);
                self.session_confirm = None;
                self.set_menu_open(false);
            }
            Msg::MediaPlayPause => crate::mpris::play_pause(),
            Msg::MediaNext => crate::mpris::next(),
            Msg::MediaPrev => crate::mpris::previous(),
            Msg::BluetoothToggle => self.toggle_menu(MenuKind::Bluetooth),
            Msg::BluetoothPower(on) => {
                crate::bluetooth::set_power(on);
                if let Some(b) = &mut self.bluetooth_now {
                    b.powered = on;
                }
                self.marcar_menu_dirty();
            }
            Msg::BluetoothConnect(mac) => crate::bluetooth::connect(&mac),
            Msg::BluetoothDisconnect(mac) => crate::bluetooth::disconnect(&mac),
            Msg::NotificationsToggle => self.toggle_menu(MenuKind::Notifications),
            Msg::NotificationsDnd(on) => {
                if let Some(h) = &self.notifications {
                    h.set_dnd(on);
                }
                self.marcar_menu_dirty();
            }
            Msg::NotificationsClear => {
                if let Some(h) = &self.notifications {
                    h.clear();
                }
                self.marcar_menu_dirty();
            }
            Msg::PolkitChar(c) => {
                self.polkit_input.push(c);
                self.marcar_menu_dirty();
            }
            Msg::PolkitBackspace => {
                self.polkit_input.pop();
                self.marcar_menu_dirty();
            }
            Msg::PolkitSubmit => {
                if let Some(req) = self.polkit_prompt.take() {
                    let _ = req.reply.send(Some(std::mem::take(&mut self.polkit_input)));
                }
                self.cerrar_polkit();
            }
            Msg::PolkitCancel => {
                if let Some(req) = self.polkit_prompt.take() {
                    let _ = req.reply.send(None);
                }
                self.cerrar_polkit();
            }
            Msg::BrightnessWheel(dy) => {
                if dy != 0.0 {
                    crate::sampler::nudge_brightness(dy < 0.0);
                    self.refresh_brightness_now();
                    self.flash_osd(crate::render::OsdKind::Brightness, self.ctx.brightness, false);
                }
            }
            Msg::BrightnessSet(f) => {
                crate::sampler::set_brightness(f);
                self.flash_osd(crate::render::OsdKind::Brightness, f, false);
            }
            Msg::BrightnessPanel => {}
            Msg::ControlToggle => {
                // Antes el engranaje ⚙ no hacía nada en el DM. Ahora abre el
                // control panel (ajustes rápidos) como menú; al abrir, refresca
                // batería/wifi/bt.
                if !(self.menu_open && self.menu_kind == MenuKind::Control) {
                    self.control_extras = crate::render::ControlExtras::read();
                }
                self.toggle_menu(MenuKind::Control);
            }
            // Antes el path layer-shell no atendía estos toggles del Control panel
            // (caían al `_ => {}`): los switches de Wi-Fi/BT no hacían nada en el DM.
            Msg::ControlWifi(on) => {
                crate::render::set_radio("wlan", on);
                self.control_extras.wifi = on;
                self.marcar_menu_dirty();
            }
            Msg::ControlBt(on) => {
                crate::render::set_radio("bluetooth", on);
                self.control_extras.bt = on;
                self.marcar_menu_dirty();
            }
            Msg::ControlPowerProfile(id) => {
                crate::render::set_power_profile(&id);
                self.control_extras.power_profile = Some(id);
                self.marcar_menu_dirty();
            }
            Msg::ControlNight(on) => {
                crate::render::set_night(on);
                self.control_extras.night = on;
                self.marcar_menu_dirty();
            }
            Msg::ControlCafe(on) => {
                // «Mantener despierto»: gatea el idle de energía (vía
                // `energia_cfg.cafe`) y, además, el inhibidor del compositor se
                // crea/destruye en `ensure_cafe_inhibitor` (necesita `qh`).
                self.energia_cfg.cafe = on;
                self.control_extras.cafe = on;
                self.marcar_menu_dirty();
            }
            Msg::Magnify(pct) => {
                // Lupa de pantalla: el compositor la aplica (sigue el puntero).
                // Guardamos el nivel para resaltar el segmento activo (best-effort:
                // los atajos de teclado lo mueven sin que pata se entere).
                crate::spawn_cmd(&format!("mirada-ctl magnify {pct}"));
                self.control_extras.magnify_pct = pct;
                self.marcar_menu_dirty();
            }
            Msg::Record(on) => {
                // Grabar pantalla: el compositor toma sus cuadros y los encodea.
                crate::spawn_cmd(if on {
                    "mirada-ctl record start"
                } else {
                    "mirada-ctl record stop"
                });
                self.control_extras.recording = on;
                self.marcar_menu_dirty();
            }
            Msg::NetworkToggle => {
                self.net_password = None;
                self.set_menu_keyboard(false);
                self.toggle_menu(MenuKind::Network);
            }
            Msg::NetworkPasswordPrompt(ssid) => {
                self.net_password = Some((ssid, String::new()));
                // El campo necesita foco de teclado (como el menú de inicio).
                self.set_menu_keyboard(true);
                self.marcar_menu_dirty();
            }
            Msg::NetworkPasswordChar(c) => {
                if let Some((_, pw)) = &mut self.net_password {
                    pw.push(c);
                    self.marcar_menu_dirty();
                }
            }
            Msg::NetworkPasswordBackspace => {
                if let Some((_, pw)) = &mut self.net_password {
                    pw.pop();
                    self.marcar_menu_dirty();
                }
            }
            Msg::NetworkPasswordSubmit => {
                if let Some((ssid, pw)) = self.net_password.take() {
                    crate::network::connect_with(&ssid, &pw);
                    self.set_menu_keyboard(false);
                    self.set_menu_open(false);
                }
            }
            Msg::NetworkPasswordCancel => {
                self.net_password = None;
                self.set_menu_keyboard(false);
                self.marcar_menu_dirty();
            }
            Msg::NetworkConnect(ssid) => {
                crate::network::connect(&ssid);
                self.set_menu_open(false);
            }
            Msg::NetworkDisconnect(ssid) => {
                crate::network::disconnect(&ssid);
                self.set_menu_open(false);
            }
            Msg::NetworkRadio(on) => {
                crate::network::set_wifi_radio(on);
                // Reflejo optimista: el próximo muestreo confirma. Repinta el popup.
                if let Some(n) = &mut self.network_now {
                    n.wifi_enabled = on;
                }
                self.marcar_menu_dirty();
            }
            Msg::ClipboardMenu => self.toggle_menu(MenuKind::Clipboard),
            Msg::ClipboardPick(text) => {
                crate::sampler::copiar_clipboard(&text);
                self.set_menu_open(false);
            }
            Msg::ClockPanel => {
                if !(self.menu_open && self.menu_kind == MenuKind::Clock) {
                    self.clock_draft = crate::ClockDraft::from_now(crate::usa_utc(&self.cfg));
                }
                self.toggle_menu(MenuKind::Clock);
            }
            Msg::ClockAdjust(f, delta) => {
                self.clock_draft.adjust(f, delta);
                self.marcar_menu_dirty();
            }
            Msg::ClockApply => {
                crate::sampler::set_system_time(&self.clock_draft.stamp());
                self.set_menu_open(false);
            }
            Msg::ClockSyncNtp => {
                crate::sampler::sync_ntp();
                self.set_menu_open(false);
            }
            Msg::StartToggle => self.toggle_menu(MenuKind::Apps),
            Msg::MenuHoverCategory(i) => {
                if self.menu_cat != Some(i) {
                    self.menu_cat = Some(i);
                    self.menu_scroll = 0.0;
                    self.marcar_menu_dirty();
                }
            }
            Msg::StartScroll(delta) => {
                let count =
                    render::menu_filtered(self.registry.all(), &self.menu_query).len();
                let content = count as f32 * 30.0;
                let viewport =
                    (MENU_H as f32 - self.menu_bar_px as f32 - 62.0).max(28.0);
                self.menu_scroll = llimphi_widget_scroll::clamp_offset(
                    self.menu_scroll + delta,
                    content,
                    viewport,
                );
                self.marcar_menu_dirty();
            }
            Msg::LaunchApp(id) => self.lanzar_app(id),
            Msg::SwitchPacha(id) => {
                diag!("pata diag · SwitchPacha({id}) → pacha switch {id}");
                crate::spawn_cmd(&format!("pacha switch {id}"));
            }
            // Conmutar de escritorio: lo pide el switcher de la barra (dwm/
            // hyprland/solaris). Faltaba el arm en el path layer-shell → los
            // botones de workspace no hacían nada en el DM (sólo en winit).
            Msg::SwitchWorkspace(n) => {
                diag!("pata diag · SwitchWorkspace({n}) → mirada-ctl workspace {n}");
                crate::sampler::switch_workspace(n);
                // Feedback INSTANTÁNEO: el sampler de fondo refresca cada ~1s (y
                // cada tick corre varios subprocesos), así que el resalte tardaba
                // segundos y parecía que el click no entraba. Movemos el activo ya
                // y lo sostenemos unos samples (`pending_ws`) para que un muestreo
                // viejo no lo revierta antes de que el WM aplique el salto.
                self.ctx.active_workspace = n;
                self.pending_ws = Some((n, crate::sampler::OPTIMISTIC_TICKS));
                self.marcar_todo_dirty();
            }
            Msg::ActivateWindow(id) => {
                diag!(
                    "pata diag · ActivateWindow({id}) seat={} toplevel={}",
                    self.seat.is_some(),
                    self.toplevel_por_id(id).is_some()
                );
                self.activar_ventana(id);
                // Feedback inmediato: marcá esta ventana como activa en la lista
                // (el foco real lo confirma el compositor en el próximo censo).
                for t in &mut self.toplevels {
                    t.activated = t.id == id;
                }
                self.marcar_todo_dirty();
            }
            Msg::CloseWindow(id) => self.cerrar_ventana(id),
            Msg::TaskDragMove(id, dx) => self.task_drag_move(id, dx),
            Msg::TaskDragEnd(id) => self.task_drag_end(id),
            Msg::TrayActivate(key) => {
                if let Some(t) = &self.tray {
                    t.activate(key);
                }
            }
            Msg::NavTabActivate(si, ti) => self.set_sidebar_open(si, ti),
            // Barrita del sidebar: se aplica EN VIVO (sin re-exec ni parpadeo).
            // Docked = sólo cambia el `exclusive_zone` de la surface; posición del
            // rail = puramente render. Ambos persisten en el TOML.
            Msg::SidebarSetDocked(si, docked) => {
                crate::persistir_eje_sidebar(si, Some(docked), None);
                self.aplicar_docked_sidebar(si, docked);
            }
            Msg::SidebarSetRailOutside(si, outside) => {
                crate::persistir_eje_sidebar(si, None, Some(outside));
                if let Some(s) = self.cfg.surfaces.get_mut(si) {
                    s.rail_outside = Some(outside);
                }
                self.marcar_sidebars_dirty();
            }
            Msg::NavClosePanel => self.cerrar_sidebar(),
            Msg::NavSetMode(m) => {
                self.nav.mode = m;
                self.marcar_sidebars_dirty();
            }
            Msg::NavSelect(id) => {
                self.nav.selected = Some(id);
                self.marcar_sidebars_dirty();
            }
            Msg::NavToggle(id) => self.nav_toggle(id),
            Msg::NavContextMenu(id) => {
                if let Some(path) = self.nav.file_path(id).map(str::to_owned) {
                    let opts = crate::open::handlers_for_path(&self.registry, &path);
                    self.nav.open_menu(id, opts);
                    self.marcar_sidebars_dirty();
                }
            }
            Msg::NavOpenWith(id, app_id) => {
                if let Some(path) = self.nav.file_path(id).map(str::to_owned) {
                    match app_id {
                        Some(aid) => {
                            let _ = crate::open::open_with_id(&self.registry, &aid, &path);
                        }
                        None => {
                            let _ = crate::open::open_system(&path);
                        }
                    }
                }
                self.nav.close_menu();
                self.marcar_sidebars_dirty();
            }
            Msg::NavMenuCancel => {
                self.nav.close_menu();
                self.marcar_sidebars_dirty();
            }
            Msg::HostToothActivate(app_id, tooth) => {
                if let Some(h) = &self.host {
                    h.activate(&app_id, tooth);
                }
            }
            Msg::NavScroll(delta) => {
                self.nav.scroll = (self.nav.scroll + delta).max(0.0);
                self.marcar_sidebars_dirty();
            }
            // --- Sidebar RAG ---
            Msg::RagEngineReady { ok, corpus } => {
                self.rag.corpus_len = corpus;
                self.rag.status = if ok {
                    crate::rag::RagStatus::Idle
                } else {
                    crate::rag::RagStatus::Unavailable
                };
                self.marcar_sidebars_dirty();
            }
            Msg::RagChar(c) => {
                if !c.is_control()
                    && matches!(
                        self.rag.status,
                        crate::rag::RagStatus::Idle | crate::rag::RagStatus::Ready
                    )
                {
                    self.rag.query.push(c);
                    self.marcar_sidebars_dirty();
                }
            }
            Msg::RagBackspace => {
                self.rag.query.pop();
                self.marcar_sidebars_dirty();
            }
            Msg::RagClear => {
                self.rag.query.clear();
                self.rag.answer.clear();
                self.rag.sources.clear();
                self.rag.error = None;
                if matches!(self.rag.status, crate::rag::RagStatus::Ready) {
                    self.rag.status = crate::rag::RagStatus::Idle;
                }
                self.marcar_sidebars_dirty();
            }
            Msg::RagSubmit => {
                let q = self.rag.query.trim().to_string();
                if !q.is_empty()
                    && matches!(
                        self.rag.status,
                        crate::rag::RagStatus::Idle | crate::rag::RagStatus::Ready
                    )
                {
                    self.rag.status = crate::rag::RagStatus::Asking;
                    self.rag.answer.clear();
                    self.rag.sources.clear();
                    self.rag.error = None;
                    if let Ok(guard) = self.rag.engine.lock() {
                        if let Some(engine) = guard.as_ref() {
                            let tx = self.rag_tx.clone();
                            engine.ask(q, Box::new(move |res| {
                                let m = match res {
                                    Ok(a) => Msg::RagResult {
                                        answer: a.answer,
                                        sources: a.sources,
                                    },
                                    Err(e) => Msg::RagError(e.to_string()),
                                };
                                let _ = tx.send(m);
                            }));
                        } else {
                            self.rag.status = crate::rag::RagStatus::Unavailable;
                        }
                    }
                    self.marcar_sidebars_dirty();
                }
            }
            Msg::RagResult { answer, sources } => {
                self.rag.answer = answer;
                self.rag.sources = sources;
                self.rag.error = None;
                self.rag.status = crate::rag::RagStatus::Ready;
                self.marcar_sidebars_dirty();
            }
            Msg::RagError(e) => {
                self.rag.error = Some(e);
                self.rag.status = crate::rag::RagStatus::Ready;
                self.marcar_sidebars_dirty();
            }
            Msg::Quit => self.exit = true,
            _ => {}
        }
    }

    /// Click en una ventana del task manager: activa o minimiza.
    pub(super) fn activar_ventana(&mut self, id: u32) {
        // El `activate` del foreign-toplevel necesita un `wl_seat`. Normalmente
        // lo captura `new_seat`, pero si ese callback aún no corrió (o la barra
        // no bindeó capacidades) `self.seat` quedaba `None` y el click "no hacía
        // nada" SILENCIOSAMENTE. Caemos al primer seat conocido por `SeatState`.
        let seat = self.seat.clone().or_else(|| {
            let s = self.seat_state.seats().next();
            if s.is_some() {
                self.seat = s.clone();
            }
            s
        });
        let Some(seat) = seat else {
            diag!("pata diag · activar_ventana({id}) SIN seat — activate NO enviado");
            return;
        };
        if let Some(t) = self.toplevel_por_id(id) {
            // SIEMPRE activar (enfocar/levantar). Antes alternaba a minimizar la
            // ventana ya activa, pero mirada ignora `set_minimized` (no-op) → el
            // click sobre el taskicon de la ventana enfocada "no hacía nada".
            t.handle.unset_minimized();
            t.handle.activate(&seat);
            diag!("pata diag · activar_ventana({id}) → activate enviado");
        } else {
            diag!("pata diag · activar_ventana({id}) sin toplevel para ese id");
        }
    }

    /// Cierra la ventana `id`.
    pub(super) fn cerrar_ventana(&mut self, id: u32) {
        if let Some(t) = self.toplevel_por_id(id) {
            t.handle.close();
        }
    }

    /// Paso de un arrastre de reordenamiento del task manager: acumula el delta
    /// y reescribe `task_order` recolocando la ventana arrastrada según cuántos
    /// slots se movió el puntero. Se recalcula desde `orden_base` en cada paso
    /// para no acumular deriva.
    fn task_drag_move(&mut self, id: u32, dx: f32) {
        // Al primer `Move` (o si cambió la ventana arrastrada) capturamos el
        // orden visible actual como base del arrastre.
        if self.task_drag.as_ref().map(|d| d.id) != Some(id) {
            let orden: Vec<u32> = self.window_entries().iter().map(|e| e.id).collect();
            let idx_base = orden.iter().position(|&x| x == id).unwrap_or(0);
            self.task_drag = Some(TaskDrag {
                id,
                dx_acc: 0.0,
                movido: 0.0,
                orden_base: orden,
                idx_base,
            });
        }
        let Some(d) = self.task_drag.as_mut() else { return };
        d.dx_acc += dx;
        d.movido += dx.abs();
        // Cuántos slots (botón + gap) se desplazó respecto del inicio.
        let salto = (d.dx_acc / TASK_SLOT_W).round() as isize;
        let len = d.orden_base.len() as isize;
        let destino = (d.idx_base as isize + salto).clamp(0, (len - 1).max(0)) as usize;
        // Reconstruimos el orden desde la base, moviendo `id` a `destino`.
        let mut nuevo = d.orden_base.clone();
        if let Some(pos) = nuevo.iter().position(|&x| x == id) {
            let v = nuevo.remove(pos);
            nuevo.insert(destino.min(nuevo.len()), v);
        }
        self.task_order = nuevo;
        self.marcar_todo_dirty();
    }

    /// Fin de un arrastre del task manager. Si la ventana apenas se movió fue un
    /// click (el `draggable` reemplaza al `on_click`): activamos la ventana. Si
    /// hubo arrastre real, el nuevo `task_order` ya quedó aplicado en vivo.
    fn task_drag_end(&mut self, id: u32) {
        let arrastrado = self
            .task_drag
            .take()
            .map(|d| d.movido >= TASK_DRAG_UMBRAL)
            .unwrap_or(false);
        if !arrastrado {
            self.activar_ventana(id);
            // Feedback inmediato del foco (igual que `Msg::ActivateWindow`).
            for t in &mut self.toplevels {
                t.activated = t.id == id;
            }
        }
        self.marcar_todo_dirty();
    }
}

/// Ancho aproximado de un slot del task manager (botón fijo + gap), en px, para
/// traducir el delta del arrastre a saltos de posición. Debe seguir a `TASK_W`
/// de `render::task_manager` (170 px) + el gap chico (≤ 4 px).
const TASK_SLOT_W: f32 = 174.0;

/// Movimiento mínimo (px) para considerar un arrastre "real" y no un click.
const TASK_DRAG_UMBRAL: f32 = 6.0;
