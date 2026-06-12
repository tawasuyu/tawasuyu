//! Definición del struct [`Desktop`], constructores y accesores de sólo lectura.

use std::collections::HashMap;

use mirada_layout::{LayoutParams, Rect, WindowId, Workspace};
use mirada_protocol::{BrainCommand, WindowPlacement};

use crate::action::WORKSPACE_COUNT;
use crate::activity::ActivityGraph;
use crate::config::Config;
use crate::keymap::Keymap;
use crate::permisos::Permisos;
use crate::rules::Rules;
use crate::session::SpaceShape;

use super::tipos::{Output, WindowInfo};

/// El estado completo del escritorio.
///
/// Mantiene las salidas físicas, [`WORKSPACE_COUNT`] escritorios
/// virtuales, el registro de ventanas, el keymap y las reglas. El único
/// punto de entrada es [`Desktop::on_event`]: traga un [`BodyEvent`],
/// muta el estado y devuelve los [`BrainCommand`]s a enviar al Cuerpo.
///
/// **Multi-monitor**: cada salida muestra un escritorio distinto; el
/// teselado se calcula para todas y el `Place` resultante las cubre. Un
/// escritorio se ve en una salida como mucho — pedir uno que ya muestra
/// otra salida las intercambia.
pub struct Desktop {
    /// Salidas físicas, en fila horizontal y en orden de aparición.
    pub(super) outputs: Vec<Output>,
    /// Escritorios virtuales — `WORKSPACE_COUNT` fijos.
    pub(super) workspaces: Vec<Workspace>,
    /// Índice (en `outputs`) de la salida con el foco.
    pub(super) focused_output: usize,
    /// Identidad de cada ventana conocida.
    pub(super) windows: HashMap<WindowId, WindowInfo>,
    /// Atajos globales → acción. Configurable, recargable en caliente.
    pub(super) keymap: Keymap,
    /// Reglas de ventana — escritorio/flotante por `app_id`/título.
    pub(super) rules: Rules,
    /// Permisos de capacidad por ejecutable — hoy, la denylist del snoop de
    /// portapapeles. El Cerebro los empuja al Cuerpo (que es quien otorga el
    /// protocolo) con [`Desktop::capabilities`].
    pub(super) caps: Permisos,
    /// Config general del WM — dropterm, parámetros del teselado, foco.
    pub(super) config: Config,
    /// Ventanas del scratchpad: se invocan flotando y se ocultan a
    /// voluntad; mientras están guardadas no viven en ningún escritorio.
    pub(super) scratchpad: Vec<WindowId>,
    /// Mapa salida→escritorio pendiente de aplicar, restaurado de una sesión
    /// guardada: al restaurar en el arranque aún no hay salidas conectadas, así
    /// que se aplica a medida que aparecen (por orden), en `OutputAdded`.
    pub(super) pending_output_workspaces: Vec<usize>,
    /// `app_id` → escritorio donde vivía, restaurado de una sesión guardada.
    /// Cuando una ventana de esa app **reaparece**, vuelve a ese escritorio; la
    /// entrada se consume (se quita) en el primer acierto, así que sólo
    /// restaura la primera ventana de cada app y no fija las posteriores.
    pub(super) restored_homes: HashMap<String, usize>,
    /// Agrupaciones (árbol fractal del zoom-Z) pendientes de rematerializar,
    /// por índice de escritorio, restauradas de una sesión. Cada una se
    /// reconstruye cuando todas sus apps miembro vuelven a estar abiertas en su
    /// escritorio (mapeando los `WindowId` nuevos por `app_id`), y entonces se
    /// quita de aquí. Si alguna app no reabre, queda pendiente sin efecto.
    pub(super) restored_groupings: HashMap<usize, SpaceShape>,
    /// Grafo de actividad: el linaje de proceso de cada ventana, para agrupar y
    /// navegar por *constelación* (familias de ventanas emparentadas).
    pub(super) activity: ActivityGraph,
}

impl Default for Desktop {
    fn default() -> Self {
        Self::new()
    }
}

impl Desktop {
    /// Escritorio recién arrancado: sin salidas ni ventanas, con los
    /// escritorios virtuales vacíos y el mapa de teclas por defecto.
    pub fn new() -> Self {
        Self::with_keymap(Keymap::default())
    }

    /// Como [`Desktop::new`], pero con un keymap dado — el que la app
    /// cargó del archivo de configuración del usuario.
    pub fn with_keymap(keymap: Keymap) -> Self {
        let workspaces = (0..WORKSPACE_COUNT)
            .map(|_| Workspace::new(LayoutParams::default()))
            .collect();
        Self {
            outputs: Vec::new(),
            workspaces,
            focused_output: 0,
            windows: HashMap::new(),
            keymap,
            rules: Rules::default(),
            caps: Permisos::default(),
            config: Config::default(),
            scratchpad: Vec::new(),
            pending_output_workspaces: Vec::new(),
            restored_homes: HashMap::new(),
            restored_groupings: HashMap::new(),
            activity: ActivityGraph::default(),
        }
    }

    /// Reemplaza las reglas de ventana. Se aplican a las ventanas que se
    /// abran a partir de ahora; las ya abiertas no se tocan.
    pub fn set_rules(&mut self, rules: Rules) {
        self.rules = rules;
    }

    /// Reemplaza los permisos de capacidad y devuelve el [`BrainCommand`] que
    /// el dueño debe enviar al Cuerpo para que tomen efecto. La app lo llama
    /// tras recargar `caps.ron` en caliente.
    pub fn set_caps(&mut self, caps: Permisos) -> BrainCommand {
        self.caps = caps;
        self.capabilities()
    }

    /// El comando que fija los permisos de capacidad en el Cuerpo (hoy, la
    /// denylist del snoop de portapapeles). La app lo envía al arrancar —junto
    /// a [`grab_keys`](Desktop::grab_keys) y [`decorations`](Desktop::decorations)—
    /// y tras recargar los permisos.
    pub fn capabilities(&self) -> BrainCommand {
        BrainCommand::SetCapabilities(self.caps.clone())
    }

    /// Aplica la config general del WM. Los parámetros de teselado
    /// (modo/gap/ratio/nmaster) se siembran en **todos** los escritorios;
    /// el resto (dropterm, foco-sigue-ratón) se consulta cuando hace falta.
    /// Pensado para llamarse una vez al arrancar, antes de conectar salidas.
    pub fn set_config(&mut self, config: Config) {
        let params = config.layout_params();
        for ws in &mut self.workspaces {
            ws.set_params(params);
        }
        self.config = config;
    }

    /// La config general vigente — para un HUD o un editor de ajustes.
    pub fn config(&self) -> &Config {
        &self.config
    }

    /// El comando que registra los atajos globales en el Cuerpo. La app
    /// lo envía al conectar, y de nuevo tras cada recarga del keymap.
    pub fn grab_keys(&self) -> BrainCommand {
        BrainCommand::GrabKeys(self.keymap.grab_list())
    }

    /// El comando que fija la decoración de ventana (marco, …) en el
    /// Cuerpo, según la config. La app lo envía al arrancar (junto a
    /// [`grab_keys`](Desktop::grab_keys)) y tras recargar la config.
    pub fn decorations(&self) -> BrainCommand {
        BrainCommand::SetDecorations(self.config.decorations())
    }

    /// Reemplaza el keymap en caliente. Devuelve el [`BrainCommand`] que
    /// el dueño debe enviar al Cuerpo para reajustar qué teclas intercepta.
    pub fn set_keymap(&mut self, keymap: Keymap) -> BrainCommand {
        self.keymap = keymap;
        self.grab_keys()
    }

    /// El keymap vigente — para un HUD o un editor visual de atajos.
    pub fn keymap(&self) -> &Keymap {
        &self.keymap
    }

    /// Recarga la config en caliente: re-siembra los parámetros de teselado
    /// (el archivo manda — un cambio de gap/modo/ratio se ve al guardar,
    /// aunque pise un layout cambiado a mano) y devuelve el comando que
    /// re-envía la decoración al Cuerpo. dropterm/foco se leen en vivo.
    pub fn reload_config(&mut self, config: Config) -> Vec<BrainCommand> {
        self.set_config(config);
        vec![self.decorations()]
    }

    /// Geometría de la salida enfocada, si hay alguna conectada.
    pub fn screen(&self) -> Option<Rect> {
        self.outputs.get(self.focused_output).map(|o| o.rect)
    }

    // --- Accesores de sólo lectura, para el HUD ---------

    /// El escritorio activo — el de la salida enfocada.
    pub fn active_workspace(&self) -> &Workspace {
        &self.workspaces[self.active_index()]
    }

    /// Las salidas conectadas, en orden, con el escritorio que muestran.
    pub fn outputs(&self) -> &[Output] {
        &self.outputs
    }

    /// Índice (en [`outputs`](Desktop::outputs)) de la salida enfocada.
    pub fn focused_output(&self) -> usize {
        self.focused_output
    }

    /// Identidad de una ventana conocida.
    pub fn window_info(&self, id: WindowId) -> Option<&WindowInfo> {
        self.windows.get(&id)
    }

    /// La ventana con el foco del teclado: la enfocada del escritorio
    /// activo — o su ventana en pantalla completa, si la hay.
    pub fn focused_window(&self) -> Option<WindowId> {
        let ws = &self.workspaces[self.active_index()];
        ws.fullscreen().or_else(|| ws.focused())
    }

    /// Cuántas ventanas hay en cada escritorio virtual.
    pub fn workspace_loads(&self) -> Vec<usize> {
        self.workspaces.iter().map(Workspace::len).collect()
    }

    /// La geometría teselada de **cada** escritorio, calculada contra `rect`
    /// (normalmente el [`work_rect`](Output::work_rect) de la salida primaria),
    /// para pintar miniaturas sin cambiar de escritorio. Es lo que consume la
    /// **vista espacial** (el "Prezi" de mirada): un mosaico por escritorio con
    /// sus ventanas a escala. Cada `Vec` respeta el modo de teselado propio de
    /// su escritorio y marca el foco de ese escritorio. `out[i]` = escritorio
    /// `i` (0-based, casa con [`workspace_loads`](Desktop::workspace_loads)).
    pub fn workspace_layouts(&self, rect: Rect) -> Vec<Vec<WindowPlacement>> {
        use mirada_protocol::placements;
        self.workspaces
            .iter()
            .map(|ws| placements(ws, rect))
            .collect()
    }

    /// El rectángulo de referencia para la vista espacial: el área teselable de
    /// la salida enfocada, o el rect dado por defecto si no hay salidas (modo
    /// simulación). Da la relación de aspecto correcta a las miniaturas.
    pub fn overview_rect(&self, fallback: Rect) -> Rect {
        self.outputs
            .get(self.focused_output)
            .map(Output::work_rect)
            .unwrap_or(fallback)
    }

    /// Una vista de todas las ventanas conocidas, en todos los
    /// escritorios — la base de `mirada-ctl windows` y de una taskbar.
    pub fn window_lines(&self) -> Vec<crate::ctl::WindowLine> {
        let active = self.active_index();
        let mut lines = Vec::new();
        for (n, ws) in self.workspaces.iter().enumerate() {
            let ws_focus = ws.focused();
            for &id in ws.windows() {
                let info = self.windows.get(&id);
                lines.push(crate::ctl::WindowLine {
                    id,
                    app_id: info.map(|i| i.app_id.clone()).unwrap_or_default(),
                    title: info.map(|i| i.title.clone()).unwrap_or_default(),
                    workspace: n + 1,
                    focused: n == active && ws_focus == Some(id),
                    minimized: false,
                });
            }
        }
        // Ventanas guardadas en el scratchpad — en ningún escritorio.
        for &id in &self.scratchpad {
            let stashed = !self.workspaces.iter().any(|ws| ws.windows().contains(&id));
            if stashed {
                let info = self.windows.get(&id);
                lines.push(crate::ctl::WindowLine {
                    id,
                    app_id: info.map(|i| i.app_id.clone()).unwrap_or_default(),
                    title: info.map(|i| i.title.clone()).unwrap_or_default(),
                    workspace: 0, // 0 = guardada en el scratchpad
                    focused: false,
                    minimized: true, // guardada = minimizada/oculta
                });
            }
        }
        lines
    }
}

// Importado aquí para que `active_index` sea visible en este impl.
// Los métodos mutantes viven en los módulos de eventos/acciones.
impl Desktop {
    /// El índice del escritorio activo — el que muestra la salida
    /// enfocada. `0` si todavía no hay ninguna salida.
    pub fn active_index(&self) -> usize {
        self.outputs
            .get(self.focused_output)
            .map(|o| o.workspace)
            .unwrap_or(0)
    }
}

