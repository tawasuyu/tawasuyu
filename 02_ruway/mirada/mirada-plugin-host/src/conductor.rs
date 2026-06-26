//! El `Conductor`: el cerebro real. Embebe un [`Desktop`] de `mirada-brain`
//! (toda la política probada: foco, atajos, reglas, multi-monitor) y deja que
//! los plugins lo **aumenten** — un plugin de layout refina la geometría, los
//! reactores emiten comandos extra. Es lo único que decide qué se manda al
//! Cuerpo; un plugin nunca puede suprimir el comando de otro.

use std::collections::HashMap;

use mirada_brain::{Config, Desktop, DesktopAction, Keymap, Permisos, Rules};
use mirada_protocol::{
    BodyEvent, BrainCommand, LayoutParams, OutputId, Rect, TileInput, WindowPlacement,
};

use crate::manifest::PluginKind;
use crate::wasm::LoadedPlugin;

/// Geometría rastreada de una salida, para bucketizar el teselado por monitor.
#[derive(Clone, Copy)]
struct OutputGeom {
    rect: Rect,
    /// Franjas reservadas: (top, bottom, left, right).
    reserved: (i32, i32, i32, i32),
}

impl OutputGeom {
    /// Área útil tras descontar las franjas reservadas.
    fn work(&self) -> Rect {
        let (t, b, l, r) = self.reserved;
        Rect::new(
            self.rect.x + l,
            self.rect.y + t,
            (self.rect.w - l - r).max(1),
            (self.rect.h - t - b).max(1),
        )
    }
}

/// Orquesta `Desktop` + plugins y arbitra el flujo de comandos.
pub struct Conductor {
    desktop: Desktop,
    /// El único plugin de layout activo (rol singleton).
    layout: Option<LoadedPlugin>,
    reactors: Vec<LoadedPlugin>,
    outputs: HashMap<OutputId, OutputGeom>,
    /// Atajos que pide el `Desktop` (se actualiza si emite `GrabKeys`).
    desktop_keys: Vec<String>,
    /// Atajos que pide cada reactor, por nombre.
    reactor_keys: HashMap<String, Vec<String>>,
    /// Última unión de atajos enviada, para no re-enviar sin cambios.
    last_grab: Option<Vec<String>>,
}

impl Conductor {
    /// Construye el conductor repartiendo los plugins en roles. Entre los de
    /// tipo `Layout`, gana el de mayor `priority`; el resto queda inactivo.
    pub fn new(desktop: Desktop, plugins: Vec<LoadedPlugin>) -> Self {
        let desktop_keys = grab_payload(&desktop.grab_keys());
        let mut layout: Option<LoadedPlugin> = None;
        let mut reactors = Vec::new();
        for p in plugins {
            match p.kind {
                PluginKind::Layout => match &layout {
                    Some(cur) if cur.priority >= p.priority => {
                        eprintln!(
                            "[conductor] layout {} ignorado (gana {} por prioridad)",
                            p.name, cur.name
                        );
                    }
                    _ => {
                        if let Some(prev) = layout.replace(p) {
                            eprintln!("[conductor] layout {} desplazado", prev.name);
                        }
                    }
                },
                PluginKind::Reactor => reactors.push(p),
            }
        }
        Self {
            desktop,
            layout,
            reactors,
            outputs: HashMap::new(),
            desktop_keys,
            reactor_keys: HashMap::new(),
            last_grab: None,
        }
    }

    /// Recarga en caliente: aplica un keymap nuevo y devuelve la `GrabKeys`
    /// actualizada (ruteada por la unión con los atajos de los reactores, para
    /// no pisarlos). Vacío si la unión no cambió.
    pub fn apply_keymap(&mut self, keymap: Keymap) -> Vec<BrainCommand> {
        let cmd = self.desktop.set_keymap(keymap);
        self.desktop_keys = grab_payload(&cmd);
        self.maybe_grab().into_iter().collect()
    }

    /// Recarga en caliente: aplica permisos nuevos y devuelve el
    /// `SetCapabilities` (el control de seguridad — denylists Wayland).
    pub fn apply_caps(&mut self, caps: Permisos) -> Vec<BrainCommand> {
        vec![self.desktop.set_caps(caps)]
    }

    /// Recarga en caliente: aplica la config general y devuelve el
    /// `SetDecorations` que de ella se deriva.
    pub fn apply_config(&mut self, config: Config) -> Vec<BrainCommand> {
        self.desktop.set_config(config);
        vec![self.desktop.decorations()]
    }

    /// Recarga en caliente: aplica reglas nuevas. No emite comando — las reglas
    /// sólo afectan a las ventanas que se abran a partir de ahora.
    pub fn apply_rules(&mut self, rules: Rules) {
        self.desktop.set_rules(rules);
    }

    /// Comandos a enviar al conectar: la unión de atajos, decoración y permisos.
    /// Espeja el handshake que hace `mirada-app-llimphi`.
    pub fn startup(&mut self) -> Vec<BrainCommand> {
        let mut out = vec![self.desktop.decorations(), self.desktop.capabilities()];
        if let Some(g) = self.maybe_grab() {
            out.insert(0, g);
        }
        out
    }

    /// Procesa un evento del Cuerpo y devuelve los comandos arbitrados.
    pub fn on_body_event(&mut self, ev: BodyEvent) -> Vec<BrainCommand> {
        self.track_output(&ev);

        // 1. El Desktop sigue siendo autoritativo.
        let mut cmds = match &ev {
            // Win+Tab en modo enlazado: el Desktop no ve el evento crudo, se
            // aplica como acción (igual que mirada-app-llimphi).
            BodyEvent::SwitchWorkspace(n) => {
                self.desktop.apply(DesktopAction::SwitchWorkspace(*n as usize))
            }
            _ => self.desktop.on_event(ev.clone()),
        };

        // 2. Los reactores aumentan; sus GrabKeys se desvían a la unión, y sus
        //    acciones de escritorio se recogen para aplicarlas al Desktop tras
        //    el bucle (no se puede tomar prestado `self.desktop` mientras se
        //    itera `self.reactors`).
        let mut pending_actions: Vec<String> = Vec::new();
        for r in &mut self.reactors {
            match r.call_on_event(&ev) {
                Ok(extra) => {
                    for c in extra {
                        if let BrainCommand::GrabKeys(keys) = c {
                            self.reactor_keys.insert(r.name.clone(), keys);
                        } else {
                            cmds.push(c);
                        }
                    }
                    pending_actions.extend(r.take_actions());
                }
                Err(e) => eprintln!("[conductor] reactor {} falló: {e}", r.name),
            }
        }

        // 2b. Las acciones pedidas por los reactores las aplica el Desktop
        //     autoritativo (igual que un atajo del usuario), manteniendo el
        //     estado consistente; los comandos resultantes (Place, …) siguen el
        //     mismo camino de arbitraje que el resto.
        for action in pending_actions {
            match action.parse::<DesktopAction>() {
                Ok(a) => cmds.extend(self.desktop.apply(a)),
                Err(e) => eprintln!("[conductor] acción de reactor inválida: {e}"),
            }
        }

        // 3. El layout refina la geometría del Place; sus GrabKeys (si hubiera)
        //    también van a la unión, igual que las del Desktop.
        self.intercept_desktop_keys(&mut cmds);
        self.apply_layout(&mut cmds);

        // 4. Arbitraje final: una sola GrabKeys con la unión, si cambió.
        if let Some(g) = self.maybe_grab() {
            cmds.insert(0, g);
        }
        cmds
    }

    /// Desvía cualquier `GrabKeys` del Desktop a `desktop_keys` (lo emite al
    /// recargar el keymap); no debe llegar crudo al Cuerpo.
    fn intercept_desktop_keys(&mut self, cmds: &mut Vec<BrainCommand>) {
        let mut i = 0;
        while i < cmds.len() {
            if let BrainCommand::GrabKeys(keys) = &cmds[i] {
                self.desktop_keys = keys.clone();
                cmds.remove(i);
            } else {
                i += 1;
            }
        }
    }

    /// La unión actual de atajos (Desktop ∪ todos los reactores), o `None` si no
    /// cambió desde la última vez que se envió.
    fn maybe_grab(&mut self) -> Option<BrainCommand> {
        let mut union: Vec<String> = Vec::new();
        for k in self
            .desktop_keys
            .iter()
            .chain(self.reactor_keys.values().flatten())
        {
            if !union.contains(k) {
                union.push(k.clone());
            }
        }
        if self.last_grab.as_ref() == Some(&union) {
            return None;
        }
        self.last_grab = Some(union.clone());
        Some(BrainCommand::GrabKeys(union))
    }

    /// Si hay layout activo, reescribe los `rect` de las ventanas teseladas de
    /// cada comando `Place`, bucketizadas por monitor. A cada monitor le pasa
    /// los `LayoutParams` que el `Desktop` usaría ahí — así los atajos del
    /// usuario (crecer maestra, etc.) siguen gobernando el teselado.
    fn apply_layout(&mut self, cmds: &mut [BrainCommand]) {
        if self.layout.is_none() {
            return;
        }
        // (área útil, params) por salida — recogido ANTES de tomar prestado
        // `self.layout`, para no chocar con el préstamo de `self.desktop`.
        let jobs: Vec<(Rect, LayoutParams)> = self
            .outputs
            .iter()
            .map(|(id, o)| {
                let params = self.desktop.params_for_output(*id).unwrap_or_default();
                (o.work(), params)
            })
            .collect();
        if jobs.is_empty() {
            return;
        }
        let layout = self.layout.as_mut().unwrap();
        for cmd in cmds.iter_mut() {
            if let BrainCommand::Place(ps) = cmd {
                Self::retile(layout, ps, &jobs);
            }
        }
    }

    /// Reparte, por cada área útil, las ventanas teseladas que caen en ella,
    /// honrando los `LayoutParams` de esa salida.
    fn retile(layout: &mut LoadedPlugin, ps: &mut [WindowPlacement], jobs: &[(Rect, LayoutParams)]) {
        for (work, params) in jobs {
            let ids: Vec<_> = ps
                .iter()
                .filter(|p| {
                    p.visible
                        && !p.floating
                        && !p.fullscreen
                        && !p.suspended
                        && center_in(p.rect, *work)
                })
                .map(|p| p.id)
                .collect();
            if ids.is_empty() {
                continue;
            }
            match layout.call_tile(&TileInput { ids, work: *work, params: *params }) {
                Ok(rects) => {
                    for (id, rect) in rects {
                        if let Some(p) = ps.iter_mut().find(|p| p.id == id) {
                            p.rect = rect;
                        }
                    }
                }
                Err(e) => eprintln!("[conductor] layout {} falló: {e}", layout.name),
            }
        }
    }

    /// Mantiene el mapa de salidas al día con los eventos del Cuerpo.
    fn track_output(&mut self, ev: &BodyEvent) {
        match *ev {
            BodyEvent::OutputAdded { id, width, height } => {
                self.outputs.insert(
                    id,
                    OutputGeom { rect: Rect::new(0, 0, width, height), reserved: (0, 0, 0, 0) },
                );
            }
            BodyEvent::OutputRemoved { id } => {
                self.outputs.remove(&id);
            }
            BodyEvent::OutputResized { id, width, height } => {
                if let Some(o) = self.outputs.get_mut(&id) {
                    o.rect.w = width;
                    o.rect.h = height;
                }
            }
            BodyEvent::OutputMoved { id, x, y } => {
                if let Some(o) = self.outputs.get_mut(&id) {
                    o.rect.x = x;
                    o.rect.y = y;
                }
            }
            BodyEvent::OutputReserved { id, top, bottom, left, right } => {
                if let Some(o) = self.outputs.get_mut(&id) {
                    o.reserved = (top, bottom, left, right);
                }
            }
            _ => {}
        }
    }
}

/// Extrae la lista de atajos de un `BrainCommand::GrabKeys` (vacía si no lo es).
fn grab_payload(cmd: &BrainCommand) -> Vec<String> {
    match cmd {
        BrainCommand::GrabKeys(keys) => keys.clone(),
        _ => Vec::new(),
    }
}

/// `true` si el centro de `r` cae dentro de `area`.
fn center_in(r: Rect, area: Rect) -> bool {
    let cx = r.x + r.w / 2;
    let cy = r.y + r.h / 2;
    cx >= area.x && cx < area.x + area.w && cy >= area.y && cy < area.y + area.h
}
