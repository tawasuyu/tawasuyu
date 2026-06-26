//! `mirada-body` — el estado del Cuerpo del compositor.
//!
//! El "Cuerpo" de mirada (`mirada-compositor`, sobre `smithay`) tiene
//! dos mitades: el *backend*, que habla Wayland y posee el hardware, y
//! esta *contabilidad* — qué salidas y superficies existen y con qué
//! geometría. Aislarla deja el backend reducido a "ejecuta estas
//! [`BodyOp`]" y la hace testeable sin un servidor gráfico.
//!
//! El flujo es simétrico al del Cerebro:
//!
//! - El backend avisa de cambios de hardware/clientes con los mutadores
//!   ([`BodyState::open_surface`], [`BodyState::add_output`], …), que
//!   devuelven el [`BodyEvent`] a mandar al Cerebro.
//! - El Cerebro responde con [`BrainCommand`]s; [`BodyState::apply`] los
//!   traduce a [`BodyOp`]s concretas que el backend ejecuta.

#![forbid(unsafe_code)]

use std::collections::BTreeMap;

use mirada_protocol::{
    BodyEvent, BrainCommand, Decorations, OutputId, Permisos, Rect, WindowEffects, WindowId,
};

/// Una superficie Wayland desde la óptica del Cuerpo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Surface {
    pub app_id: String,
    pub title: String,
    /// Geometría aplicada — `None` hasta la primera [`BodyOp::Configure`].
    pub geometry: Option<Rect>,
    pub visible: bool,
    pub focused: bool,
    /// `true` si flota: el backend la pinta por encima de las teseladas.
    pub floating: bool,
    /// `true` si está en pantalla completa.
    pub fullscreen: bool,
    /// `true` si duerme tras una capa de zoom: oculta y con los frame
    /// callbacks suspendidos (el cliente queda inerte).
    pub suspended: bool,
    /// Divisor de frames: 1 de cada N frame callbacks (1 = pleno ritmo). El
    /// backend lo consulta para espaciar el pintado de las ventanas de fondo.
    pub frame_divisor: u32,
}

impl Surface {
    fn new(app_id: String, title: String) -> Self {
        Self {
            app_id,
            title,
            geometry: None,
            visible: false,
            focused: false,
            floating: false,
            fullscreen: false,
            suspended: false,
            frame_divisor: 1,
        }
    }
}

/// Una orden concreta para el backend (smithay, headless, …).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BodyOp {
    /// Recoloca una superficie, la muestra u oculta y dice si flota o
    /// está en pantalla completa (el backend ajusta el orden de pintado
    /// y el estado `xdg_toplevel` en consecuencia).
    Configure {
        id: WindowId,
        rect: Rect,
        visible: bool,
        floating: bool,
        fullscreen: bool,
        /// `true` si la superficie duerme: el backend no le envía frame
        /// callbacks (queda inerte) además de ocultarla.
        suspended: bool,
        /// Divisor de frames: el backend le envía 1 de cada N frame callbacks
        /// (1 = pleno ritmo). Throttle de las ventanas de fondo.
        frame_divisor: u32,
    },
    /// Da el foco del teclado a una superficie.
    Focus(WindowId),
    /// Quita el foco a todas las superficies.
    Unfocus,
    /// Pide el cierre ordenado de un cliente.
    CloseClient(WindowId),
    /// Mata a un cliente que no responde.
    KillClient(WindowId),
    /// Registra los atajos globales a interceptar.
    SetGrabs(Vec<String>),
    /// Cambia el cursor del puntero.
    SetCursor(String),
    /// Fija los parámetros de decoración de las ventanas (marco, …).
    SetDecorations(Decorations),
    /// Fija los permisos de capacidad por ejecutable: el backend los consulta
    /// al decidir qué clientes ven los globals sensibles (el snoop de
    /// portapapeles `zwlr_data_control`, la inyección de teclas
    /// `zwp_virtual_keyboard`, el censo de ventanas `ext_foreign_toplevel_list`,
    /// la captura de pantalla `zwlr_screencopy`).
    SetCapabilities(Permisos),
    /// Lanza un programa como proceso hijo del compositor.
    Spawn(String),
    /// Apaga el compositor y libera el hardware.
    Shutdown,
    /// Bloquea la sesión activa: el compositor compone el shell de credenciales
    /// (greeter en modo lock) encima y le rutea el input hasta el desbloqueo.
    Lock,
    /// Cierra la sesión activa (FUS logout): pide cerrar sus ventanas, la da de
    /// baja del roster y pasa a otra sesión hosteada — o al login si no queda
    /// ninguna.
    Logout,
    /// Fija los efectos visuales (opacidad, sombra…) de ciertas ventanas; el
    /// backend los aplica al componer cada superficie.
    SetEffects(Vec<(WindowId, WindowEffects)>),
}

/// La contabilidad del Cuerpo: salidas y superficies.
#[derive(Debug, Default)]
pub struct BodyState {
    outputs: Vec<(OutputId, Rect)>,
    /// `BTreeMap` para que el orden de las `BodyOp` sea determinista.
    surfaces: BTreeMap<WindowId, Surface>,
    focused: Option<WindowId>,
}

impl BodyState {
    /// Cuerpo recién arrancado: sin salidas ni superficies.
    pub fn new() -> Self {
        Self::default()
    }

    // --- Traducción de comandos del Cerebro --------------------------

    /// Traduce un comando del Cerebro a las operaciones de backend que lo
    /// materializan. Sólo emite lo que de verdad cambia: un `Place`
    /// idéntico al estado actual no produce ninguna `BodyOp`.
    pub fn apply(&mut self, cmd: BrainCommand) -> Vec<BodyOp> {
        match cmd {
            // El estado de escritorios (switcher Win+Tab en modo enlazado) lo
            // consume el compositor en `apply_commands` antes de delegar acá; el
            // `Body` no materializa superficies para él.
            BrainCommand::SetWorkspaces { .. } => Vec::new(),
            BrainCommand::Place(placements) => {
                let mut ops = Vec::new();
                let listed: Vec<WindowId> = placements.iter().map(|p| p.id).collect();
                let mut new_focus = None;

                // Reconfigura las superficies que aparecen en la lista.
                for p in &placements {
                    if p.focused {
                        new_focus = Some(p.id);
                    }
                    if let Some(s) = self.surfaces.get_mut(&p.id) {
                        if s.geometry != Some(p.rect)
                            || s.visible != p.visible
                            || s.floating != p.floating
                            || s.fullscreen != p.fullscreen
                            || s.suspended != p.suspended
                            || s.frame_divisor != p.frame_divisor
                        {
                            s.geometry = Some(p.rect);
                            s.visible = p.visible;
                            s.floating = p.floating;
                            s.fullscreen = p.fullscreen;
                            s.suspended = p.suspended;
                            s.frame_divisor = p.frame_divisor;
                            ops.push(BodyOp::Configure {
                                id: p.id,
                                rect: p.rect,
                                visible: p.visible,
                                floating: p.floating,
                                fullscreen: p.fullscreen,
                                suspended: p.suspended,
                                frame_divisor: p.frame_divisor,
                            });
                        }
                    }
                }

                // Oculta lo que el Cerebro ya no coloca.
                for (id, s) in &mut self.surfaces {
                    if !listed.contains(id) && s.visible {
                        s.visible = false;
                        // Oculta por omisión (otro escritorio, scratchpad…): no
                        // es el sueño dirigido del zoom.
                        s.suspended = false;
                        // Oculta: vuelve a pleno ritmo (el throttle es de fondo
                        // *visible*; lo oculto no pinta).
                        s.frame_divisor = 1;
                        let rect = s.geometry.unwrap_or(Rect::new(0, 0, 0, 0));
                        ops.push(BodyOp::Configure {
                            id: *id,
                            rect,
                            visible: false,
                            floating: s.floating,
                            fullscreen: s.fullscreen,
                            suspended: false,
                            frame_divisor: 1,
                        });
                    }
                }

                // Reasigna el foco sólo si cambió.
                if new_focus != self.focused {
                    self.focused = new_focus;
                    for (id, s) in &mut self.surfaces {
                        s.focused = Some(*id) == new_focus;
                    }
                    ops.push(match new_focus {
                        Some(id) => BodyOp::Focus(id),
                        None => BodyOp::Unfocus,
                    });
                }
                ops
            }
            BrainCommand::Close(id) => vec![BodyOp::CloseClient(id)],
            BrainCommand::Kill(id) => vec![BodyOp::KillClient(id)],
            BrainCommand::GrabKeys(keys) => vec![BodyOp::SetGrabs(keys)],
            BrainCommand::SetCursor(name) => vec![BodyOp::SetCursor(name)],
            BrainCommand::SetDecorations(d) => vec![BodyOp::SetDecorations(d)],
            BrainCommand::SetCapabilities(p) => vec![BodyOp::SetCapabilities(p)],
            BrainCommand::Spawn(cmd) => vec![BodyOp::Spawn(cmd)],
            BrainCommand::Shutdown => vec![BodyOp::Shutdown],
            BrainCommand::Lock => vec![BodyOp::Lock],
            BrainCommand::Logout => vec![BodyOp::Logout],
            // Los efectos son estado de superficie puro; el backend los aplica
            // directo (no afectan la contabilidad de geometría/foco del Cuerpo).
            BrainCommand::SetEffects(v) => vec![BodyOp::SetEffects(v)],
        }
    }

    // --- Mutadores del backend → eventos para el Cerebro -------------

    /// Registra una salida recién conectada.
    pub fn add_output(&mut self, id: OutputId, width: i32, height: i32) -> BodyEvent {
        self.outputs.push((id, Rect::new(0, 0, width, height)));
        BodyEvent::OutputAdded { id, width, height }
    }

    /// Da de baja una salida desconectada.
    pub fn remove_output(&mut self, id: OutputId) -> BodyEvent {
        self.outputs.retain(|(o, _)| *o != id);
        BodyEvent::OutputRemoved { id }
    }

    /// Cambia el área útil de una salida sin desconectarla — al
    /// redimensionar la ventana anfitriona o al reservar/liberar la
    /// franja del shell. Conserva el escritorio que muestra.
    pub fn resize_output(&mut self, id: OutputId, width: i32, height: i32) -> BodyEvent {
        if let Some((_, rect)) = self.outputs.iter_mut().find(|(o, _)| *o == id) {
            rect.w = width;
            rect.h = height;
        }
        BodyEvent::OutputResized { id, width, height }
    }

    /// Fija el **origen global** de una salida (su esquina superior-izquierda en
    /// el espacio compuesto). El backend lo emite tras recalcular la disposición
    /// de monitores; el Cuerpo es la fuente única de esa geometría, así que el
    /// Cerebro la adopta tal cual en vez de reconstruirla. No toca el tamaño.
    pub fn move_output(&mut self, id: OutputId, x: i32, y: i32) -> BodyEvent {
        if let Some((_, rect)) = self.outputs.iter_mut().find(|(o, _)| *o == id) {
            rect.x = x;
            rect.y = y;
        }
        BodyEvent::OutputMoved { id, x, y }
    }

    /// Reserva —o libera— franjas en los bordes de una salida: las zonas
    /// exclusivas (px desde cada borde) que el teselado debe esquivar. Las usa
    /// el marco (`pata`) para acoplar sus barras sin que las ventanas las tapen;
    /// cero en los cuatro libera la reserva. No toca el tamaño físico, así que
    /// admite barras en varios bordes a la vez.
    pub fn reserve_output(
        &self,
        id: OutputId,
        top: i32,
        bottom: i32,
        left: i32,
        right: i32,
    ) -> BodyEvent {
        BodyEvent::OutputReserved {
            id,
            top,
            bottom,
            left,
            right,
        }
    }

    /// Registra una superficie recién creada por un cliente.
    pub fn open_surface(
        &mut self,
        id: WindowId,
        app_id: impl Into<String>,
        title: impl Into<String>,
    ) -> BodyEvent {
        let app_id = app_id.into();
        let title = title.into();
        self.surfaces
            .insert(id, Surface::new(app_id.clone(), title.clone()));
        BodyEvent::WindowOpened { id, app_id, title }
    }

    /// Da de baja una superficie destruida. `None` si no se conocía.
    pub fn close_surface(&mut self, id: WindowId) -> Option<BodyEvent> {
        self.surfaces.remove(&id)?;
        if self.focused == Some(id) {
            self.focused = None;
        }
        Some(BodyEvent::WindowClosed { id })
    }

    /// Actualiza el título de una superficie. `None` si no se conocía.
    pub fn retitle_surface(&mut self, id: WindowId, title: impl Into<String>) -> Option<BodyEvent> {
        let title = title.into();
        let s = self.surfaces.get_mut(&id)?;
        s.title = title.clone();
        Some(BodyEvent::WindowRetitled { id, title })
    }

    /// Construye un evento de puntero entrando en una superficie.
    pub fn pointer_enter(&self, id: WindowId) -> BodyEvent {
        BodyEvent::PointerEntered { id }
    }

    /// Construye un evento de click (foco-al-click) sobre una superficie.
    pub fn clicked(&self, id: WindowId) -> BodyEvent {
        BodyEvent::Clicked { id }
    }

    /// Construye un evento de arrastre teselado al punto `(x, y)`.
    pub fn window_dragged(&self, id: WindowId, x: i32, y: i32) -> BodyEvent {
        BodyEvent::WindowDragged { id, x, y }
    }

    /// Construye un evento de atajo pulsado.
    pub fn keybind(&self, combo: impl Into<String>) -> BodyEvent {
        BodyEvent::Keybind(combo.into())
    }

    // --- Accesores de sólo lectura -----------------------------------

    /// Las salidas conectadas.
    pub fn outputs(&self) -> &[(OutputId, Rect)] {
        &self.outputs
    }

    /// Una superficie conocida.
    pub fn surface(&self, id: WindowId) -> Option<&Surface> {
        self.surfaces.get(&id)
    }

    /// Número de superficies registradas.
    pub fn surface_count(&self) -> usize {
        self.surfaces.len()
    }

    /// Las superficies visibles, en orden de id.
    pub fn visible(&self) -> impl Iterator<Item = (WindowId, &Surface)> {
        self.surfaces.iter().filter(|(_, s)| s.visible).map(|(id, s)| (*id, s))
    }

    /// La superficie enfocada.
    pub fn focused(&self) -> Option<WindowId> {
        self.focused
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use mirada_protocol::WindowPlacement;

    fn placement(id: WindowId, visible: bool, focused: bool) -> WindowPlacement {
        WindowPlacement {
            id,
            rect: Rect::new(0, 0, 800, 600),
            visible,
            focused,
            floating: false,
            fullscreen: false,
            suspended: false,
            frame_divisor: 1,
        }
    }

    /// Cuerpo con dos superficies abiertas.
    fn body_with_two() -> BodyState {
        let mut b = BodyState::new();
        b.add_output(0, 1920, 1080);
        b.open_surface(1, "app1", "uno");
        b.open_surface(2, "app2", "dos");
        b
    }

    #[test]
    fn opening_a_surface_yields_a_window_opened_event() {
        let mut b = BodyState::new();
        let ev = b.open_surface(7, "org.brahman.shuma", "shell");
        assert_eq!(
            ev,
            BodyEvent::WindowOpened {
                id: 7,
                app_id: "org.brahman.shuma".into(),
                title: "shell".into()
            }
        );
        assert_eq!(b.surface_count(), 1);
    }

    #[test]
    fn placing_surfaces_configures_and_focuses_them() {
        let mut b = body_with_two();
        let ops = b.apply(BrainCommand::Place(vec![
            placement(1, true, false),
            placement(2, true, true),
        ]));
        // Dos Configure + un Focus.
        let configures = ops.iter().filter(|o| matches!(o, BodyOp::Configure { .. })).count();
        assert_eq!(configures, 2);
        assert!(ops.contains(&BodyOp::Focus(2)));
        assert_eq!(b.focused(), Some(2));
        assert!(b.surface(2).unwrap().focused);
    }

    #[test]
    fn an_identical_place_produces_no_ops() {
        let mut b = body_with_two();
        let cmd = BrainCommand::Place(vec![placement(1, true, true), placement(2, true, false)]);
        assert!(!b.apply(cmd.clone()).is_empty());
        // Repetir el mismo Place no cambia nada.
        assert!(b.apply(cmd).is_empty());
    }

    #[test]
    fn dropping_a_surface_from_the_list_hides_it() {
        let mut b = body_with_two();
        b.apply(BrainCommand::Place(vec![placement(1, true, true), placement(2, true, false)]));
        // El Cerebro deja de colocar la 2 (p. ej. cambió de escritorio).
        let ops = b.apply(BrainCommand::Place(vec![placement(1, true, true)]));
        assert!(ops.contains(&BodyOp::Configure {
            id: 2,
            rect: Rect::new(0, 0, 800, 600),
            visible: false,
            floating: false,
            fullscreen: false,
            suspended: false,
            frame_divisor: 1,
        }));
        assert!(!b.surface(2).unwrap().visible);
    }

    #[test]
    fn placement_for_an_unknown_surface_is_ignored() {
        let mut b = body_with_two();
        // La 99 no existe — no debe producir Configure.
        let ops = b.apply(BrainCommand::Place(vec![placement(99, true, true)]));
        assert!(!ops.iter().any(|o| matches!(o, BodyOp::Configure { id: 99, .. })));
    }

    #[test]
    fn close_and_kill_map_to_client_ops() {
        let mut b = body_with_two();
        assert_eq!(b.apply(BrainCommand::Close(1)), vec![BodyOp::CloseClient(1)]);
        assert_eq!(b.apply(BrainCommand::Kill(2)), vec![BodyOp::KillClient(2)]);
    }

    #[test]
    fn grab_keys_cursor_and_shutdown_pass_through() {
        let mut b = BodyState::new();
        assert_eq!(
            b.apply(BrainCommand::GrabKeys(vec!["Super+q".into()])),
            vec![BodyOp::SetGrabs(vec!["Super+q".into()])]
        );
        assert_eq!(
            b.apply(BrainCommand::SetCursor("crosshair".into())),
            vec![BodyOp::SetCursor("crosshair".into())]
        );
        assert_eq!(b.apply(BrainCommand::Shutdown), vec![BodyOp::Shutdown]);
    }

    #[test]
    fn set_capabilities_passes_through() {
        let mut b = BodyState::new();
        let p = Permisos {
            clipboard_denylist: vec!["wl-paste".into()],
            virtual_input_denylist: vec!["wtype".into()],
            window_list_denylist: vec!["lswt".into()],
            screencopy_denylist: vec!["grim".into()],
            dmabuf_denylist: vec!["leak".into()],
        };
        assert_eq!(
            b.apply(BrainCommand::SetCapabilities(p.clone())),
            vec![BodyOp::SetCapabilities(p)]
        );
    }

    #[test]
    fn closing_a_surface_clears_its_focus() {
        let mut b = body_with_two();
        b.apply(BrainCommand::Place(vec![placement(1, true, true)]));
        assert_eq!(b.focused(), Some(1));
        let ev = b.close_surface(1);
        assert_eq!(ev, Some(BodyEvent::WindowClosed { id: 1 }));
        assert_eq!(b.focused(), None);
        assert_eq!(b.surface_count(), 1);
    }

    #[test]
    fn closing_an_unknown_surface_yields_nothing() {
        let mut b = body_with_two();
        assert!(b.close_surface(404).is_none());
    }

    #[test]
    fn retitling_updates_the_surface() {
        let mut b = body_with_two();
        let ev = b.retitle_surface(1, "uno bis");
        assert_eq!(ev, Some(BodyEvent::WindowRetitled { id: 1, title: "uno bis".into() }));
        assert_eq!(b.surface(1).unwrap().title, "uno bis");
    }

    #[test]
    fn move_output_repositions_and_emits_the_event() {
        let mut b = BodyState::new();
        b.add_output(0, 1920, 1080);
        b.add_output(1, 1280, 1024);
        let ev = b.move_output(1, 5000, 0);
        assert_eq!(ev, BodyEvent::OutputMoved { id: 1, x: 5000, y: 0 });
        assert_eq!(b.outputs()[1].1, Rect::new(5000, 0, 1280, 1024));
        // No toca el otro monitor.
        assert_eq!(b.outputs()[0].1, Rect::new(0, 0, 1920, 1080));
    }

    #[test]
    fn outputs_are_tracked() {
        let mut b = BodyState::new();
        b.add_output(0, 2560, 1440);
        b.add_output(1, 1920, 1080);
        assert_eq!(b.outputs().len(), 2);
        b.remove_output(0);
        assert_eq!(b.outputs().len(), 1);
        assert_eq!(b.outputs()[0].0, 1);
    }

    #[test]
    fn moving_focus_emits_a_single_focus_op() {
        let mut b = body_with_two();
        b.apply(BrainCommand::Place(vec![placement(1, true, true), placement(2, true, false)]));
        // Cambia el foco a la 2; geometría igual → sólo un Focus.
        let ops = b.apply(BrainCommand::Place(vec![
            placement(1, true, false),
            placement(2, true, true),
        ]));
        assert_eq!(ops, vec![BodyOp::Focus(2)]);
    }

    #[test]
    fn a_suspend_change_alone_triggers_a_configure_and_sticks() {
        let mut b = body_with_two();
        let p1 = placement(1, true, true);
        b.apply(BrainCommand::Place(vec![p1, placement(2, true, false)]));
        // La 2 se duerme: oculta + suspendida (sin foco).
        let mut p2 = placement(2, false, false);
        p2.suspended = true;
        let ops = b.apply(BrainCommand::Place(vec![p1, p2]));
        assert!(ops
            .iter()
            .any(|o| matches!(o, BodyOp::Configure { id: 2, suspended: true, .. })));
        assert!(b.surface(2).unwrap().suspended);
        assert!(!b.surface(2).unwrap().visible);
    }

    #[test]
    fn a_floating_change_alone_triggers_a_configure() {
        let mut b = body_with_two();
        let mut p1 = placement(1, true, true);
        b.apply(BrainCommand::Place(vec![p1, placement(2, true, false)]));
        // Sólo cambia `floating` — misma geometría y visibilidad.
        p1.floating = true;
        let ops = b.apply(BrainCommand::Place(vec![p1, placement(2, true, false)]));
        assert!(ops
            .iter()
            .any(|o| matches!(o, BodyOp::Configure { id: 1, floating: true, .. })));
        assert!(b.surface(1).unwrap().floating);
    }

    #[test]
    fn visible_iterates_only_shown_surfaces() {
        let mut b = body_with_two();
        b.apply(BrainCommand::Place(vec![placement(1, true, true), placement(2, false, false)]));
        let shown: Vec<_> = b.visible().map(|(id, _)| id).collect();
        assert_eq!(shown, vec![1]);
    }
}
