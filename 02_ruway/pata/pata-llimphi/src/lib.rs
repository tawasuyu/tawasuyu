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
//! Hoy todas las superficies se pintan en una sola ventana, en los rects que el
//! layout resolvió. Cuando el compositor `mirada` reconozca superficies `pata`
//! (Fase 8), cada una será su propia ventana acoplada.

pub mod render;
pub mod sampler;

use std::time::Duration;

use llimphi_theme::Theme;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};

use pata_core::widget::{build_all, Widget, WidgetCtx};
use pata_core::{Config, Frame, Rect};

use sampler::Sampler;

/// Los mensajes de la app. Por ahora mínimos: el refresh periódico y la salida.
/// El despliegue Quake del `shuma_input` llega en la Fase 7; los widgets con
/// IPC (`window_list`, `astro`, `tray`) en la Fase 6.
#[derive(Clone, Debug)]
pub enum Msg {
    /// Refresh periódico (1 Hz): re-muestrea el sistema y `tick`ea los widgets.
    Tick,
    /// Cerrar la app.
    Quit,
}

/// Los widgets vivos de una superficie, repartidos por slot. Paralelo a
/// [`pata_core::Surface`]; los paneles (tarjetas flotantes) llegan después.
pub struct SurfaceWidgets {
    /// Slot inicial (izquierda / arriba).
    pub start: Vec<Box<dyn Widget>>,
    /// Slot central.
    pub center: Vec<Box<dyn Widget>>,
    /// Slot final (derecha / abajo).
    pub end: Vec<Box<dyn Widget>>,
}

impl SurfaceWidgets {
    /// Itera todos los widgets de la superficie (los tres slots).
    fn iter_mut(&mut self) -> impl Iterator<Item = &mut Box<dyn Widget>> {
        self.start
            .iter_mut()
            .chain(self.center.iter_mut())
            .chain(self.end.iter_mut())
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
    /// Muestreador del sistema (con estado para el delta de CPU).
    pub sampler: Sampler,
    /// Tamaño de la pantalla en píxeles.
    pub screen: (i32, i32),
}

impl Model {
    /// Construye los widgets de cada superficie desde la config.
    fn construir_widgets(cfg: &Config) -> Vec<SurfaceWidgets> {
        cfg.surfaces
            .iter()
            .map(|s| SurfaceWidgets {
                start: build_all(&s.start),
                center: build_all(&s.center),
                end: build_all(&s.end),
            })
            .collect()
    }

    /// `tick`ea todos los widgets de todas las superficies con el contexto dado.
    fn tick_widgets(&mut self, ctx: &WidgetCtx) {
        for sw in &mut self.surfaces {
            for w in sw.iter_mut() {
                w.tick(ctx);
            }
        }
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
        let surfaces = Model::construir_widgets(&cfg);
        let mut sampler = Sampler::new();
        let ctx = sampler.sample();

        let mut model = Model {
            theme: Theme::dark(),
            cfg,
            frame,
            surfaces,
            sampler,
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
            }
            Msg::Quit => handle.quit(),
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        render::root(model)
    }

    fn on_key(_model: &Model, event: &KeyEvent) -> Option<Msg> {
        if event.state != KeyState::Pressed {
            return None;
        }
        match &event.key {
            Key::Named(NamedKey::Escape) => Some(Msg::Quit),
            _ => None,
        }
    }
}
