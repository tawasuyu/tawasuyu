//! `pata-llimphi` — el frontend Linux del marco.
//!
//! Monta el modelo agnóstico de [`pata_core`] sobre Llimphi. El reparto de
//! responsabilidades es la regla dura del repo (UIs intercambiables sobre un
//! `*-core` agnóstico):
//!
//! - **`pata-core`** decide *qué* mostrar: resuelve la geometría
//!   ([`pata_core::layout::resolve`]) y, por cada [`WidgetSpec`], materializa un
//!   [`Widget`] que emite un view-model ([`WidgetView`]) en cada `tick`.
//! - **este crate** decide *cómo*: muestrea el sistema en un
//!   [`WidgetCtx`](pata_core::widget::WidgetCtx) (ver [`sampler`]) y traduce el
//!   view-model a `View<Msg>` de Llimphi (ver [`render`]).
//!
//! El `shuma_input` es la excepción: es **interacción**, no modelo de dominio,
//! así que lo intercepta el frontend (ver [`shuma`]) en lugar de pasar por el
//! `build` agnóstico —igual que `mirada-launcher` trata su shuma_bar—.
//!
//! Hoy todas las superficies se pintan en una sola ventana, en los rects que el
//! layout resolvió. Cuando el compositor `mirada` reconozca superficies `pata`
//! (Fase 8), cada una será su propia ventana acoplada.

pub mod keys;
pub mod layer;
pub mod render;
pub mod sampler;
pub mod shuma;
pub mod toplevel;
pub mod tray;

use std::time::Duration;

use llimphi_motion::{animate, motion, Tween};
use llimphi_theme::Theme;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};

use pata_core::widget::{build, Widget, WidgetCtx};
use pata_core::{Config, Frame, Rect};

use sampler::Sampler;
use shuma::ShumaState;
use tray::TrayHandle;

/// Los mensajes de la app.
#[derive(Clone, Debug)]
pub enum Msg {
    /// Refresh periódico (1 Hz): re-muestrea el sistema y `tick`ea los widgets.
    Tick,
    /// Desplegar/replegar el drawer de shuma.
    ShumaToggle,
    /// Carácter al input de shuma.
    ShumaChar(char),
    /// Backspace en el input de shuma.
    ShumaBackspace,
    /// Enter en el input de shuma — ejecuta el comando.
    ShumaSubmit,
    /// Resultado estructurado del comando (líneas + código) para la card.
    ShumaResult(shuma::RunResult),
    /// Re-ejecutar una línea (clic en el comando de una card sin pipe).
    ShumaRunLine(String),
    /// Revelar/ocultar la salida capturada (tee) de una etapa intermedia de la
    /// card `idx`: `(idx_card, idx_etapa)`.
    ShumaStageToggle(usize, usize),
    /// Plegar/desplegar la card `idx` del historial.
    ShumaCollapse(usize),
    /// Desplazar el historial del drawer `delta` px (rueda / arrastre de barra).
    ShumaScroll(f32),
    /// Tick de la animación de despliegue (sólo re-render).
    ShumaAnim,
    /// Lanzar un programa (click sobre un widget con prop `exec`).
    Spawn(String),
    /// Activar una ventana del `window_list` (traerla al frente). El `u32` es el
    /// [`toplevel::Toplevel::id`]; sólo el backend layer-shell sabe resolverlo.
    ActivateWindow(u32),
    /// Activar un item del `tray` (click). El `String` es la `key` del
    /// [`tray::TrayItem`]; sólo el backend layer-shell sabe resolverlo.
    TrayActivate(String),
    /// Cerrar la app.
    Quit,
}

/// Un widget dentro de un slot: o un widget de `pata-core` (que emite un
/// view-model), o el `shuma_input` —interacción que pinta el frontend—.
pub enum SlotWidget {
    /// Un widget builtin de `pata-core`. `exec` es el comando que lanza al
    /// clickearlo (de la prop `exec` del spec), o `None` si no es clickeable.
    Core {
        widget: Box<dyn Widget>,
        exec: Option<String>,
    },
    /// El cabezal del shell; su estado vive en [`Model::shuma`].
    Shuma,
    /// La lista de ventanas abiertas. Es interacción + IPC (igual que `Shuma`):
    /// los datos los provee el backend (vía wlr-foreign-toplevel en layer-shell)
    /// y se pasan al render aparte, no por el view-model de core.
    WindowList,
    /// El portapapeles: muestra el texto copiado actual. Dato del host (vía
    /// `wl-paste`), no del view-model de core. `exec` (opcional) es el comando a
    /// lanzar al clickearlo — típicamente un selector de historial (cliphist).
    Clipboard {
        /// Comando del selector de historial, o `None` si no es clickeable.
        exec: Option<String>,
    },
    /// La bandeja del sistema (StatusNotifierItem). Dato del host (vía D-Bus, ver
    /// [`tray`]), no del view-model de core. Cada item se activa al clickearlo.
    Tray,
}

/// Lanza `cmd` por `sh -c` como proceso hijo, sin esperarlo (no bloquea). Lo
/// usan ambos backends al recibir [`Msg::Spawn`].
pub fn spawn_cmd(cmd: &str) {
    let _ = std::process::Command::new("sh").arg("-c").arg(cmd).spawn();
}

/// `true` si la config declara al menos un widget de ese `kind` en cualquier slot
/// de cualquier superficie. Lo usan ambos backends para arrancar servicios caros
/// (el tray, que toma el nombre del watcher) sólo si hacen falta.
pub fn config_tiene_widget(cfg: &Config, kind: &str) -> bool {
    cfg.surfaces.iter().any(|s| {
        s.start
            .iter()
            .chain(&s.center)
            .chain(&s.end)
            .any(|w| w.kind == kind)
    })
}

/// Los widgets vivos de una superficie, repartidos por slot.
pub struct SurfaceWidgets {
    /// Slot inicial (izquierda / arriba).
    pub start: Vec<SlotWidget>,
    /// Slot central.
    pub center: Vec<SlotWidget>,
    /// Slot final (derecha / abajo).
    pub end: Vec<SlotWidget>,
}

impl SurfaceWidgets {
    /// Itera los widgets de core de la superficie (los que se `tick`ean).
    fn core_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn Widget>> {
        self.start
            .iter_mut()
            .chain(self.center.iter_mut())
            .chain(self.end.iter_mut())
            .filter_map(|sw| match sw {
                SlotWidget::Core { widget, .. } => Some(widget),
                SlotWidget::Shuma
                | SlotWidget::WindowList
                | SlotWidget::Clipboard { .. }
                | SlotWidget::Tray => None,
            })
    }
}

/// El estado de la app: config + geometría resuelta + widgets vivos + sampler.
pub struct Model {
    /// Paleta de Llimphi.
    pub theme: Theme,
    /// El marco declarado.
    pub cfg: Config,
    /// La geometría resuelta sobre la pantalla.
    pub frame: Frame,
    /// Widgets vivos, en el mismo orden que `cfg.surfaces`.
    pub surfaces: Vec<SurfaceWidgets>,
    /// Estado del cabezal del shell y su drawer Quake.
    pub shuma: ShumaState,
    /// Muestreador del sistema (con estado para el delta de CPU).
    pub sampler: Sampler,
    /// Texto del portapapeles (una línea), para el widget `clipboard`. Se
    /// re-muestrea cada tick vía `wl-paste`.
    pub clipboard: Option<String>,
    /// La bandeja del sistema, corriendo en su propio hilo. `None` si la config no
    /// declara ningún widget `tray`.
    pub tray: Option<TrayHandle>,
    /// Tamaño de la pantalla en píxeles.
    pub screen: (i32, i32),
}

impl Model {
    /// Construye los widgets de cada superficie y el estado de shuma desde la
    /// config. El primer `shuma_input` que aparece define el cabezal.
    fn construir(cfg: &Config) -> (Vec<SurfaceWidgets>, ShumaState) {
        let mut shuma = ShumaState::default();
        let mut build_slot = |specs: &[pata_core::WidgetSpec]| -> Vec<SlotWidget> {
            specs
                .iter()
                .map(|spec| {
                    if spec.kind == "shuma_input" {
                        if !shuma.present {
                            shuma = ShumaState::from_spec(spec);
                        }
                        SlotWidget::Shuma
                    } else if spec.kind == "window_list" {
                        SlotWidget::WindowList
                    } else if spec.kind == "clipboard" {
                        let exec = spec.str_prop("exec", "");
                        SlotWidget::Clipboard {
                            exec: (!exec.is_empty()).then(|| exec.to_string()),
                        }
                    } else if spec.kind == "tray" {
                        SlotWidget::Tray
                    } else {
                        let exec = spec.str_prop("exec", "");
                        SlotWidget::Core {
                            widget: build(spec),
                            exec: (!exec.is_empty()).then(|| exec.to_string()),
                        }
                    }
                })
                .collect()
        };
        let surfaces = cfg
            .surfaces
            .iter()
            .map(|s| SurfaceWidgets {
                start: build_slot(&s.start),
                center: build_slot(&s.center),
                end: build_slot(&s.end),
            })
            .collect();
        (surfaces, shuma)
    }

    /// `tick`ea todos los widgets de core con el contexto dado.
    fn tick_widgets(&mut self, ctx: &WidgetCtx) {
        for sw in &mut self.surfaces {
            for w in sw.core_mut() {
                w.tick(ctx);
            }
        }
    }

    /// Arranca la animación del drawer hacia `destino` (0 = replegado, 1 =
    /// desplegado) y dispara el bucle de `ShumaAnim`.
    fn animar_shuma(&mut self, destino: f32, handle: &Handle<Msg>) {
        let desde = self.shuma.anim.value();
        self.shuma.anim = Tween::new(desde, destino, motion::FAST, motion::ease_out_cubic);
        animate(handle, motion::FAST, || Msg::ShumaAnim);
    }
}

/// Tamaño inicial de la ventana. Cuando mirada acople las superficies (Fase 8)
/// esto lo fijará el compositor; por ahora cubrimos un 1080p.
const PANTALLA: (i32, i32) = (1920, 1080);

/// La app Llimphi del marco.
pub struct PataApp;

impl App for PataApp {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "pata"
    }

    fn app_id() -> Option<&'static str> {
        Some("gioser.pata")
    }

    fn initial_size() -> (u32, u32) {
        (PANTALLA.0 as u32, PANTALLA.1 as u32)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let cfg = pata_config::load();
        let screen = PANTALLA;
        let frame = pata_core::resolve(&cfg, Rect::new(0, 0, screen.0, screen.1));
        let (surfaces, shuma) = Model::construir(&cfg);
        let mut sampler = Sampler::new();
        let ctx = sampler.sample();
        let clipboard = crate::sampler::leer_clipboard();
        let tray = config_tiene_widget(&cfg, "tray")
            .then(TrayHandle::spawn)
            .flatten();

        let mut model = Model {
            theme: Theme::dark(),
            cfg,
            frame,
            surfaces,
            shuma,
            sampler,
            clipboard,
            tray,
            screen,
        };
        // Primer tick para que los widgets arranquen con datos.
        model.tick_widgets(&ctx);

        handle.spawn_periodic(Duration::from_secs(1), || Msg::Tick);
        model
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Tick => {
                let ctx = model.sampler.sample();
                model.tick_widgets(&ctx);
                model.clipboard = crate::sampler::leer_clipboard();
            }
            Msg::Quit => handle.quit(),
            Msg::ShumaToggle => {
                if model.shuma.present {
                    model.shuma.open = !model.shuma.open;
                    let destino = if model.shuma.open { 1.0 } else { 0.0 };
                    model.animar_shuma(destino, handle);
                }
            }
            Msg::ShumaChar(c) => {
                if model.shuma.open {
                    model.shuma.buffer.push(c);
                }
            }
            Msg::ShumaBackspace => {
                if model.shuma.open {
                    model.shuma.buffer.pop();
                }
            }
            Msg::ShumaSubmit => {
                if model.shuma.open && !model.shuma.buffer.trim().is_empty() {
                    let cmd = std::mem::take(&mut model.shuma.buffer);
                    model.shuma.push_pending(cmd.clone());
                    handle.spawn(move || Msg::ShumaResult(shuma::ejecutar(&cmd)));
                }
            }
            Msg::ShumaResult(res) => model.shuma.finish_last(res),
            Msg::ShumaRunLine(line) => {
                if model.shuma.open && !line.trim().is_empty() {
                    model.shuma.push_pending(line.clone());
                    handle.spawn(move || Msg::ShumaResult(shuma::ejecutar(&line)));
                }
            }
            Msg::ShumaStageToggle(idx, stage) => {
                if let Some(b) = model.shuma.blocks.get_mut(idx) {
                    b.expanded_stage = if b.expanded_stage == Some(stage) {
                        None
                    } else {
                        Some(stage)
                    };
                }
            }
            Msg::ShumaCollapse(idx) => {
                if let Some(b) = model.shuma.blocks.get_mut(idx) {
                    b.collapsed = !b.collapsed;
                }
            }
            Msg::ShumaScroll(delta) => model.shuma.scroll_by(delta),
            Msg::ShumaAnim => {}
            Msg::Spawn(cmd) => spawn_cmd(&cmd),
            Msg::TrayActivate(key) => {
                if let Some(t) = &model.tray {
                    t.activate(key);
                }
            }
            // El window_list necesita el cliente foreign-toplevel del backend
            // layer-shell; bajo el compositor mirada llegará por su IPC. No-op acá.
            Msg::ActivateWindow(_) => {}
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        render::root(model)
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        shuma::drawer_overlay(&model.shuma, model.screen, &model.theme)
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        // 1) El hotkey del shuma_input abre/cierra el drawer (prioridad).
        if model.shuma.present {
            if let Some(hk) = &model.shuma.hotkey {
                if keys::matches(hk, &event.key) {
                    return Some(Msg::ShumaToggle);
                }
            }
        }
        // 2) Con el drawer abierto, el teclado va al input.
        if model.shuma.open {
            return match &event.key {
                Key::Named(NamedKey::Escape) => Some(Msg::ShumaToggle),
                Key::Named(NamedKey::Backspace) => Some(Msg::ShumaBackspace),
                Key::Named(NamedKey::Enter) => Some(Msg::ShumaSubmit),
                Key::Character(s) => s.chars().next().map(Msg::ShumaChar),
                _ => None,
            };
        }
        // 3) Sin drawer, Esc cierra la app.
        match &event.key {
            Key::Named(NamedKey::Escape) => Some(Msg::Quit),
            _ => None,
        }
    }
}
