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
    diag, CardState, LayerApp, LayerDrag, MenuKind, PanelGpu, Panel, RenderCache, DRAWER_H, MENU_H,
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

    /// La lista de ventanas para el render del `window_list`.
    pub(super) fn window_entries(&self) -> Vec<WindowEntry> {
        self.toplevels
            .iter()
            .map(|t| WindowEntry {
                id: t.id,
                label: t.etiqueta(),
                app_id: t.app_id.clone(),
                active: t.activated,
                minimized: t.minimized,
            })
            .collect()
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
        let h = if open { 10_000 } else { self.shuma_bar_px };
        let layer = &self.panels[pi].layer;
        layer.set_size(0, h);
        layer.set_keyboard_interactivity(if open {
            smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::Exclusive
        } else {
            smithay_client_toolkit::shell::wlr_layer::KeyboardInteractivity::None
        });
        layer.commit();
        self.panels[pi].cache = None;
        self.panels[pi].dirty = true;
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

    /// Despliega/repliega el menú de inicio.
    pub(super) fn set_menu_open(&mut self, open: bool) {
        let Some(pi) = self.menu_panel else { return };
        if self.menu_open == open {
            return;
        }
        self.menu_open = open;
        if !open {
            self.menu_query.clear();
            self.menu_scroll = 0.0;
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
            let _ = app.spawn();
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
        if cambios {
            self.marcar_sidebars_dirty();
        }
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

    /// Índice (en `panels`) de la layer surface del sidebar `si`.
    pub(super) fn sidebar_panel_de(&self, si: usize) -> Option<usize> {
        self.panels.iter().position(|p| p.idx == si && p.card.is_none())
    }

    /// Activa/repliega el diente `(si, ti)`.
    pub(super) fn set_sidebar_open(&mut self, si: usize, ti: usize) {
        self.nav.toggle_tab(si, ti);
        let Some(pi) = self.sidebar_panel_de(si) else {
            return;
        };
        let s = &self.cfg.surfaces[si];
        let thickness = s.thickness.max(1.0) as u32;
        let abierto = matches!(self.nav.open, Some((s2, _)) if s2 == si);
        let w = if abierto {
            thickness + s.panel_width.max(1.0) as u32
        } else {
            thickness
        };
        {
            let layer = &self.panels[pi].layer;
            layer.set_size(w, 0);
            layer.commit();
        }
        self.panels[pi].cache = None;
        self.panels[pi].dirty = true;
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
        if cfg.surfaces.len() != self.cfg.surfaces.len() {
            eprintln!(
                "pata · la config cambió la cantidad de barras; reiniciá pata para reanclar las \
                 layer surfaces (el reorden de dientes dentro de una barra sí recarga solo)"
            );
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

    pub(super) fn maybe_sample(&mut self) {
        let Some((ctx, clipboard)) = self.sampler.latest() else {
            return;
        };
        self.maybe_recargar_config();
        self.ctx = ctx;
        crate::push_clip_history(&mut self.clip_history, &clipboard);
        self.clipboard = clipboard;
        if let Some(h) = &self.weather {
            if let Some(w) = h.latest() {
                self.weather_now = Some(w);
            }
        }
        for sw in &mut self.surfaces {
            for w in sw.core_mut() {
                w.tick(&ctx);
            }
        }
        for p in &mut self.panels {
            if let Some(c) = p.card.as_mut() {
                for w in &mut c.widgets {
                    w.tick(&ctx);
                }
            }
            p.dirty = true;
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
            match RawSurface::from_surface(hal, wgpu_surface, w, h) {
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
        self.maybe_sample();
        self.maybe_cava();
        self.poll_nav();
        self.poll_host();
        self.ensure_gpu(pi);

        // Drawer abierto: el shell hospedado avanza solo.
        if self.shuma_panel == Some(pi) && self.shuma.open {
            self.shuma.inner =
                shuma_module_shell::update(self.shuma.inner.clone(), shuma_module_shell::Msg::Tick);
            self.panels[pi].dirty = true;
        }

        if !self.panels[pi].dirty {
            self.latido(pi, qh);
            return;
        }

        let idx = self.panels[pi].idx;
        let (w, h) = (self.panels[pi].width, self.panels[pi].height);
        let windows = self.window_entries();
        let tray_items = self.tray.as_ref().map(|t| t.items()).unwrap_or_default();
        let data = render::BarData {
            windows: &windows,
            clipboard: self.clipboard.as_deref(),
            tray: &tray_items,
            weather: self.weather_now.as_ref(),
            cava: &self.cava_frame,
        };

        let view = if self.tooltip_pi == Some(pi) {
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
                ),
                MenuKind::Clipboard => render::clipboard_menu_view(
                    &self.cfg.surfaces[idx],
                    &self.surfaces[idx],
                    &self.shuma,
                    &data,
                    &self.theme,
                    self.menu_bar_px as f32,
                    &self.clip_history,
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
            }
        } else if self.shuma_panel == Some(pi) && self.shuma.open {
            render::shuma_open_view(
                &self.cfg.surfaces[idx],
                &self.surfaces[idx],
                &self.shuma,
                &data,
                &self.theme,
                self.shuma_bar_px as f32,
                DRAWER_H as f32,
            )
        } else if self.cfg.surfaces[idx].kind == SurfaceKind::Sidebar {
            let hosted = {
                let app = self.focused_app_id().map(|s| s.to_string());
                match (app, self.host.as_ref()) {
                    (Some(id), Some(h)) => h.snapshot(&id).map(|(_, teeth)| (id, teeth)),
                    _ => None,
                }
            };
            let (hosted_app, hosted_teeth): (&str, &[pata_host::HostedTooth]) = match &hosted {
                Some((id, teeth)) => (id.as_str(), teeth.as_slice()),
                None => ("", &[]),
            };
            render::sidebar_surface_view(
                &self.cfg.surfaces[idx],
                idx,
                w as f32,
                h as f32,
                &self.nav,
                hosted_teeth,
                hosted_app,
                &self.shuma,
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
            Msg::ShumaShell(m) => {
                let focusing = matches!(m, shuma_module_shell::Msg::FocusInput);
                self.shuma.inner = shuma_module_shell::update(self.shuma.inner.clone(), m);
                if focusing && !self.shuma.open {
                    self.set_shuma_open(true);
                }
                self.marcar_shuma_dirty();
            }
            Msg::Spawn(cmd) => crate::spawn_cmd(&cmd),
            Msg::VolumeWheel(dy) => {
                if dy != 0.0 {
                    crate::sampler::nudge_volume(dy > 0.0);
                }
            }
            Msg::VolumeMute => crate::sampler::toggle_mute(),
            Msg::VolumeSet(f) => crate::sampler::set_volume(f),
            Msg::VolumePanel => crate::spawn_cmd("pavucontrol || pavucontrol-qt"),
            Msg::BrightnessWheel(dy) => {
                if dy != 0.0 {
                    crate::sampler::nudge_brightness(dy > 0.0);
                }
            }
            Msg::BrightnessSet(f) => crate::sampler::set_brightness(f),
            Msg::BrightnessPanel => {}
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
            Msg::ActivateWindow(id) => self.activar_ventana(id),
            Msg::CloseWindow(id) => self.cerrar_ventana(id),
            Msg::TrayActivate(key) => {
                if let Some(t) = &self.tray {
                    t.activate(key);
                }
            }
            Msg::NavTabActivate(si, ti) => self.set_sidebar_open(si, ti),
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
            Msg::Quit => self.exit = true,
            _ => {}
        }
    }

    /// Click en una ventana del task manager: activa o minimiza.
    pub(super) fn activar_ventana(&mut self, id: u32) {
        let Some(seat) = self.seat.clone() else { return };
        if let Some(t) = self.toplevel_por_id(id) {
            if t.activated {
                t.handle.set_minimized();
            } else {
                t.handle.unset_minimized();
                t.handle.activate(&seat);
            }
        }
    }

    /// Cierra la ventana `id`.
    pub(super) fn cerrar_ventana(&mut self, id: u32) {
        if let Some(t) = self.toplevel_por_id(id) {
            t.handle.close();
        }
    }
}
