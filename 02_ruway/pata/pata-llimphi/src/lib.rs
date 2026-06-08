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

pub mod app_icons;
pub mod cava;
pub mod keys;
pub mod layer;
pub mod nouser;
pub mod open;
pub mod render;
pub mod sampler;
pub mod shuma;
pub mod toplevel;
pub mod tray;
pub mod weather;

use std::time::Duration;

use llimphi_motion::{animate, motion, Tween};
use llimphi_theme::Theme;
use llimphi_ui::{App, Handle, Key, KeyEvent, KeyState, NamedKey, View};

use llimphi_widget_navigator::{NavId, NavMode};

use pata_core::config::{FloatingCard, SurfaceKind};
use pata_core::widget::{build, Widget, WidgetCtx};
use pata_core::{Config, Frame, Rect};

use nouser::{MembersOutcome, NavState, PollOutcome};
use sampler::Sampler;
use shuma::ShumaState;
use tray::TrayHandle;

/// Los mensajes de la app.
#[derive(Clone, Debug)]
pub enum Msg {
    /// Refresh periódico (1 Hz): re-muestrea el sistema y `tick`ea los widgets.
    Tick,
    /// Refresh rápido del visualizador de audio (~20 Hz): drena el último cuadro
    /// de cava y re-pinta. Sólo se dispara si la config declara un `cava`.
    CavaTick,
    /// Desplegar/replegar el drawer de shuma.
    ShumaToggle,
    /// Un evento del **shell real** hospedado (`shuma-module-shell`): teclas,
    /// latido que drena la salida, clicks en cards/etapas, scroll, selección del
    /// cuerpo IDE-text… Todo el contenido del drawer llega por aquí (el `view`
    /// del módulo lo envuelve con su `lift`). pata sólo lo reenvía a
    /// `shuma_module_shell::update`.
    ShumaShell(shuma_module_shell::Msg),
    /// Tick de la animación de despliegue (sólo re-render). También sirve de
    /// no-op para absorber clicks sobre el borde del panel del drawer.
    ShumaAnim,
    /// Lanzar un programa (click sobre un widget con prop `exec`).
    Spawn(String),
    /// Saltar al escritorio virtual `n` (**1-based**), por click en una celda del
    /// `workspaces` switcher. Se lo pide al WM (`mirada-ctl workspace N`); el
    /// switcher refleja el cambio en el próximo tick.
    SwitchWorkspace(u8),
    /// Rueda del mouse sobre el medidor de volumen: ajusta el volumen del sink
    /// por defecto. El `f32` es el delta de la rueda (signo = dirección).
    VolumeWheel(f32),
    /// Click/click-derecho sobre el volumen: togglea el mute del sink.
    VolumeMute,
    /// Click en el `clipboard`: despliega/repliega el popup con el historial.
    ClipboardMenu,
    /// Elegir una entrada del historial: la vuelve a copiar (`wl-copy`) y cierra.
    ClipboardPick(String),
    /// Click en el reloj: despliega/repliega el panel para fijar fecha/hora.
    ClockPanel,
    /// Click izquierdo sobre el medidor de CPU (o el de cores): despliega/
    /// repliega su ventanita de interacción.
    CpuPanel,
    /// Click izquierdo sobre el medidor de RAM: despliega/repliega su ventanita.
    RamPanel,
    /// Click izquierdo sobre el medidor de volumen: despliega/repliega su
    /// ventanita (slider vertical + mute).
    VolumePanel,
    /// Click izquierdo sobre el medidor de brillo: despliega/repliega su
    /// ventanita (slider vertical).
    BrightnessPanel,
    /// Ajustar el volumen a una fracción exacta `0..1` desde la ventanita
    /// (click sobre la franja del slider). El sampler refleja en el próximo tick.
    VolumeSet(f32),
    /// Ajustar el brillo a una fracción exacta `0..1` desde la ventanita.
    BrightnessSet(f32),
    /// Ajusta un campo del borrador de fecha/hora `(campo 0..=4, delta)`:
    /// 0=año 1=mes 2=día 3=hora 4=minuto.
    ClockAdjust(u8, i32),
    /// Aplica el borrador al reloj del sistema (apaga NTP + `timedatectl`).
    ClockApply,
    /// Re-activa la sincronización NTP (vuelve a la hora automática).
    ClockSyncNtp,
    /// Rueda del mouse sobre el medidor de brillo: ajusta la luminosidad de la
    /// pantalla. El `f32` es el delta de la rueda (signo = dirección).
    BrightnessWheel(f32),
    /// Desplegar/replegar el menú del botón de inicio.
    StartToggle,
    /// Cicla al próximo estilo de menú (Classic → XP → GNOME → Classic).
    /// Right-click sobre el botón de inicio.
    StartStyleCycle,
    /// Carácter al buscador del menú de inicio.
    StartChar(char),
    /// Backspace en el buscador del menú de inicio.
    StartBackspace,
    /// Enter en el menú: lanza el primer resultado del filtro.
    StartLaunchFirst,
    /// Desplazar la lista del menú de inicio `delta` px (rueda).
    StartScroll(f32),
    /// Lanzar una app del menú de inicio por su `id` en el [`app_bus::AppRegistry`].
    LaunchApp(String),
    /// Activar una ventana del `window_list` (traerla al frente, o minimizarla si
    /// ya está activa — estilo KDE). El `u32` es el [`toplevel::Toplevel::id`];
    /// sólo el backend layer-shell sabe resolverlo.
    ActivateWindow(u32),
    /// Cerrar una ventana del task manager (clic derecho). El `u32` es el
    /// [`toplevel::Toplevel::id`]; sólo el backend layer-shell sabe resolverlo.
    CloseWindow(u32),
    /// Activar un item del `tray` (click). El `String` es la `key` del
    /// [`tray::TrayItem`]; sólo el backend layer-shell sabe resolverlo.
    TrayActivate(String),
    // --- Sidebar navegador (Fase 11c) ---
    /// Clic en un diente del rail `(surface_idx, tab_idx)`: despliega/repliega su
    /// panel navegador.
    NavTabActivate(usize, usize),
    /// Cerrar el panel navegador desplegado (Esc / clic fuera).
    NavClosePanel,
    /// Cambiar el modo del navegador (árbol/grafo).
    NavSetMode(NavMode),
    /// Seleccionar un nodo del navegador.
    NavSelect(NavId),
    /// Expandir/colapsar un nodo rama; al expandir una Mónada sin miembros
    /// resueltos dispara su `resolve_monad`.
    NavToggle(NavId),
    /// Right-click sobre un nodo: si es un archivo, abre el menú "Abrir con…"
    /// (precomputa sus apps); si no, no-op.
    NavContextMenu(NavId),
    /// Elegir cómo abrir el archivo del menú: `Some(app_id)` con esa app nativa,
    /// `None` con el handler del sistema (`xdg-open`).
    NavOpenWith(NavId, Option<String>),
    /// Cerrar el menú "Abrir con…" sin abrir nada.
    NavMenuCancel,
    /// Clic en un diente **hospedado** (de la app enfocada) en el rail de pata:
    /// `(app_id, tooth_id)`. Se reenvía a la app por el rail hospedado. Sólo el
    /// backend layer-shell (que conoce el foco y corre el `HostServer`) lo resuelve.
    HostToothActivate(String, u32),
    /// Desplazar el panel navegador `delta` px.
    NavScroll(f32),
    /// Disparo periódico del poll de Mónadas (`list_monads`).
    NavTick,
    /// Resultado del poll de Mónadas.
    NavPoll(PollOutcome),
    /// Resultado de resolver los miembros de una Mónada.
    NavMembers(MembersOutcome),
    /// Cerrar la app.
    Quit,
}

/// Un widget dentro de un slot: o un widget de `pata-core` (que emite un
/// view-model), o el `shuma_input` —interacción que pinta el frontend—.
pub enum SlotWidget {
    /// Un widget builtin de `pata-core`. `exec` es el comando que lanza al
    /// clickearlo (de la prop `exec` del spec), o `None` si no es clickeable.
    /// `kind` es el `WidgetSpec::kind` (cpu_meter/volume/brightness/clock…): el
    /// render lo usa para teñir el medidor con su gradiente propio y para
    /// cablear la interacción específica (rueda de volumen/brillo, click en el
    /// reloj). `cells` es el ancho cuantizado pedido (0 = automático).
    Core {
        kind: String,
        widget: Box<dyn Widget>,
        exec: Option<String>,
        cells: u32,
    },
    /// El botón de inicio: muestra su `label` y, al clickearlo, despliega el
    /// menú nativo de apps (o lanza `exec` si la config lo fija, override estilo
    /// waybar). Es interacción, no view-model de core.
    Start {
        /// Texto/ícono del botón (prop `label`, default `⊞`).
        label: String,
        /// Comando a lanzar en vez de abrir el menú, si la config lo fija.
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
    /// El clima: un dibujo colorido del cielo + la temperatura. Dato del host
    /// (servicio público por `curl`, ver [`weather`]). `exec` (opcional) abre el
    /// pronóstico al clickearlo.
    Weather {
        /// Comando a lanzar al click (un sitio del tiempo), o `None`.
        exec: Option<String>,
    },
    /// El visualizador de audio estilo CAVA: barras animadas con el espectro.
    /// Dato del host (el binario `cava` en modo raw, ver [`cava`]).
    Cava,
}

/// `true` si la config pide el reloj en **UTC** (`general.timezone = "UTC"`).
/// Cualquier otro valor (incluido `"auto"`) usa la hora local. Paridad con el
/// `TzMode` de mirada-launcher (que sólo distinguía auto/UTC). Compartido por
/// ambos backends para construir el sampler.
pub fn usa_utc(cfg: &Config) -> bool {
    cfg.general.timezone.trim().eq_ignore_ascii_case("utc")
}

/// Lanza `cmd` por `sh -c` como proceso hijo, sin esperarlo (no bloquea). Lo
/// usan ambos backends al recibir [`Msg::Spawn`].
pub fn spawn_cmd(cmd: &str) {
    let _ = std::process::Command::new("sh").arg("-c").arg(cmd).spawn();
}

/// Borrador editable de fecha/hora para el panel del reloj. Se inicializa con la
/// hora actual al abrir el panel; los botones ▲/▼ lo ajustan; "Aplicar" lo
/// escribe al reloj del sistema.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct ClockDraft {
    pub year: i32,
    pub month: i32,
    pub day: i32,
    pub hour: i32,
    pub minute: i32,
}

impl Default for ClockDraft {
    fn default() -> Self {
        Self {
            year: 2026,
            month: 1,
            day: 1,
            hour: 0,
            minute: 0,
        }
    }
}

impl ClockDraft {
    /// El borrador inicializado con la hora actual (UTC si `utc`, si no local).
    pub fn from_now(utc: bool) -> Self {
        use chrono::{Datelike, Timelike};
        let (y, mo, d, h, mi) = if utc {
            let n = chrono::Utc::now();
            (n.year(), n.month(), n.day(), n.hour(), n.minute())
        } else {
            let n = chrono::Local::now();
            (n.year(), n.month(), n.day(), n.hour(), n.minute())
        };
        Self {
            year: y,
            month: mo as i32,
            day: d as i32,
            hour: h as i32,
            minute: mi as i32,
        }
    }

    /// Ajusta el campo `f` (0=año…4=minuto) por `delta`. Mes/hora/minuto dan la
    /// vuelta; año y día se acotan a un rango sano.
    pub fn adjust(&mut self, f: u8, delta: i32) {
        let wrap = |v: i32, lo: i32, hi: i32| {
            let span = hi - lo + 1;
            (((v - lo) % span + span) % span) + lo
        };
        match f {
            0 => self.year = (self.year + delta).clamp(1970, 2100),
            1 => self.month = wrap(self.month + delta, 1, 12),
            2 => self.day = (self.day + delta).clamp(1, 31),
            3 => self.hour = wrap(self.hour + delta, 0, 23),
            4 => self.minute = wrap(self.minute + delta, 0, 59),
            _ => {}
        }
    }

    /// El campo `f` como texto a dos/cuatro dígitos.
    pub fn campo(&self, f: u8) -> String {
        match f {
            0 => format!("{:04}", self.year),
            1 => format!("{:02}", self.month),
            2 => format!("{:02}", self.day),
            3 => format!("{:02}", self.hour),
            4 => format!("{:02}", self.minute),
            _ => String::new(),
        }
    }

    /// El sello `"YYYY-MM-DD HH:MM:00"` que consume `timedatectl set-time`.
    pub fn stamp(&self) -> String {
        format!(
            "{:04}-{:02}-{:02} {:02}:{:02}:00",
            self.year, self.month, self.day, self.hour, self.minute
        )
    }
}

/// El grosor (px) de la primera barra que hospeda un widget de `kind`, para
/// posicionar su popup debajo. Default 32 si no se encuentra.
pub fn bar_thickness_for(cfg: &Config, kind: &str) -> f32 {
    cfg.surfaces
        .iter()
        .find(|s| {
            s.start
                .iter()
                .chain(&s.center)
                .chain(&s.end)
                .any(|w| w.kind == kind)
        })
        .map(|s| s.thickness)
        .unwrap_or(32.0)
}

/// Tope del historial de portapapeles.
pub const CLIP_HISTORY_MAX: usize = 16;

/// Agrega `nuevo` al frente del `historial` de portapapeles si no es vacío ni
/// igual al actual tope; deduplica (mueve al frente) y recorta a
/// [`CLIP_HISTORY_MAX`]. Compartido por ambos backends.
pub fn push_clip_history(historial: &mut Vec<String>, nuevo: &Option<String>) {
    let Some(s) = nuevo else { return };
    if s.is_empty() {
        return;
    }
    if historial.first().map(|f| f == s).unwrap_or(false) {
        return; // ya es el tope
    }
    historial.retain(|x| x != s);
    historial.insert(0, s.clone());
    historial.truncate(CLIP_HISTORY_MAX);
}

/// Envuelve `s` en comillas simples para `sh -c`, escapando comillas internas.
/// Para pasar rutas con espacios al stand-in de apertura (Fase 11d).
pub fn shell_quote(s: &str) -> String {
    format!("'{}'", s.replace('\'', "'\\''"))
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

/// La `place` (ciudad) del primer widget `weather` de la config, o `""` para que
/// el servicio detecte la ubicación por IP.
pub fn weather_place(cfg: &Config) -> String {
    primer_widget(cfg, "weather")
        .map(|w| w.str_prop("place", "").to_string())
        .unwrap_or_default()
}

/// El número de barras del primer widget `cava` (prop `bars`, default 12,
/// acotado a 4..=64).
pub fn cava_bars(cfg: &Config) -> u32 {
    primer_widget(cfg, "cava")
        .map(|w| (w.num_prop("bars", 12.0) as u32).clamp(4, 64))
        .unwrap_or(12)
}

/// El primer `WidgetSpec` de ese `kind` en cualquier slot de cualquier superficie.
fn primer_widget<'a>(cfg: &'a Config, kind: &str) -> Option<&'a pata_core::WidgetSpec> {
    cfg.surfaces.iter().find_map(|s| {
        s.start
            .iter()
            .chain(&s.center)
            .chain(&s.end)
            .find(|w| w.kind == kind)
    })
}

/// `true` si la config declara al menos un `SurfaceKind::Sidebar` con un diente
/// cuyo contenido es un navegador (`kind = "navigator"`). Sólo entonces arranca
/// el plano de datos de nouser (el poll periódico de Mónadas).
pub fn config_tiene_navigator(cfg: &Config) -> bool {
    cfg.surfaces
        .iter()
        .filter(|s| s.kind == SurfaceKind::Sidebar)
        .flat_map(|s| s.tabs.iter())
        .any(|t| t.content.kind == "navigator")
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
                SlotWidget::Start { .. }
                | SlotWidget::Shuma
                | SlotWidget::WindowList
                | SlotWidget::Clipboard { .. }
                | SlotWidget::Tray
                | SlotWidget::Weather { .. }
                | SlotWidget::Cava => None,
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
    /// Tarjetas flotantes (estilo conky) de las superficies `Panel`, cada una con
    /// sus widgets vivos. En layer-shell cada tarjeta es su propia surface; en el
    /// path winit se pintan en absoluto sobre la ventana única.
    pub cards: Vec<(FloatingCard, Vec<Box<dyn Widget>>)>,
    /// Estado del cabezal del shell y su drawer Quake.
    pub shuma: ShumaState,
    /// Registro de apps para el menú del botón de inicio.
    pub registry: app_bus::AppRegistry,
    /// `true` cuando el menú de inicio está desplegado.
    pub menu_open: bool,
    /// Texto del buscador del menú de inicio (filtra apps por label).
    pub menu_query: String,
    /// Desplazamiento de la lista del menú (px).
    pub menu_scroll: f32,
    /// Estilo visual del menú de inicio (alternable con right-click sobre
    /// el botón). Default `Classic`. Ver [`MenuStyle`].
    pub menu_style: MenuStyle,
    /// Muestreador del sistema (con estado para el delta de CPU).
    pub sampler: Sampler,
    /// Texto del portapapeles (una línea), para el widget `clipboard`. Se
    /// re-muestrea cada tick vía `wl-paste`.
    pub clipboard: Option<String>,
    /// Historial de copias (más reciente al frente, sin repetidos, tope 16). Lo
    /// alimenta cada tick desde `clipboard`; el popup lo lista.
    pub clip_history: Vec<String>,
    /// `true` cuando el popup del historial del portapapeles está desplegado.
    pub clip_open: bool,
    /// `true` cuando el panel del reloj (fijar fecha/hora) está desplegado.
    pub clock_open: bool,
    /// Borrador de fecha/hora que el panel del reloj edita.
    pub clock_draft: ClockDraft,
    /// `true` cuando la ventanita de CPU está desplegada.
    pub cpu_open: bool,
    /// `true` cuando la ventanita de RAM está desplegada.
    pub ram_open: bool,
    /// `true` cuando la ventanita de volumen está desplegada.
    pub volume_open: bool,
    /// `true` cuando la ventanita de brillo está desplegada.
    pub brightness_open: bool,
    /// Último snapshot del sistema — cacheado para alimentar las ventanitas
    /// (porcentajes en vivo, lista de cores) sin volver a llamar al sampler.
    pub last_ctx: pata_core::widget::WidgetCtx,
    /// La bandeja del sistema, corriendo en su propio hilo. `None` si la config no
    /// declara ningún widget `tray`.
    pub tray: Option<TrayHandle>,
    /// Feed de clima en su propio hilo. `None` si la config no declara `weather`.
    pub weather: Option<weather::WeatherHandle>,
    /// Última lectura del clima (se refresca con `latest()` cada tick).
    pub weather_now: Option<weather::Weather>,
    /// Visualizador de audio (cava) en su propio hilo. `None` si la config no
    /// declara `cava`.
    pub cava: Option<cava::CavaHandle>,
    /// Último cuadro del visualizador (una fracción `0..1` por banda).
    pub cava_frame: Vec<f32>,
    /// Estado del sidebar navegador (Mónadas de nouser). Vacío si la config no
    /// declara ningún `SurfaceKind::Sidebar` con un navegador.
    pub nav: NavState,
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
                    if spec.kind == "start_button" {
                        let exec = spec.str_prop("exec", "");
                        SlotWidget::Start {
                            label: spec.str_prop("label", "⊞").to_string(),
                            exec: (!exec.is_empty()).then(|| exec.to_string()),
                        }
                    } else if spec.kind == "shuma_input" {
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
                    } else if spec.kind == "weather" {
                        let exec = spec.str_prop("exec", "");
                        SlotWidget::Weather {
                            exec: (!exec.is_empty()).then(|| exec.to_string()),
                        }
                    } else if spec.kind == "cava" {
                        SlotWidget::Cava
                    } else {
                        let exec = spec.str_prop("exec", "");
                        SlotWidget::Core {
                            kind: spec.kind.clone(),
                            widget: build(spec),
                            exec: (!exec.is_empty()).then(|| exec.to_string()),
                            cells: spec.num_prop("cells", 0.0).max(0.0) as u32,
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

    /// Construye las tarjetas flotantes de todas las superficies `Panel` con sus
    /// widgets vivos. Compartido por el path winit ([`PataApp::init`]) y el
    /// layer-shell ([`crate::layer`]): el modelo se escribe una vez.
    pub fn construir_cards(cfg: &Config) -> Vec<(FloatingCard, Vec<Box<dyn Widget>>)> {
        cfg.surfaces
            .iter()
            .filter(|s| s.kind == SurfaceKind::Panel)
            .flat_map(|s| s.cards.iter())
            .map(|card| {
                let ws = card.widgets.iter().map(build).collect();
                (card.clone(), ws)
            })
            .collect()
    }

    /// `tick`ea todos los widgets de core (barras y tarjetas) con el contexto dado.
    fn tick_widgets(&mut self, ctx: &WidgetCtx) {
        for sw in &mut self.surfaces {
            for w in sw.core_mut() {
                w.tick(ctx);
            }
        }
        for (_, ws) in &mut self.cards {
            for w in ws {
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

/// Estilos del menú de inicio. El default `Classic` es el panel a la
/// izquierda con buscador + lista filtrable (el que la app trae desde
/// el inicio). `XP` evoca el menú de Windows XP — banda superior con
/// usuario, dos columnas (pinned + programs), footer "Apagar". `Gnome`
/// imita Activities — overlay full-screen con grid de tiles y buscador
/// centrado. El usuario alterna estilos con click-derecho sobre el
/// botón de inicio (`Msg::StartStyleCycle`).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MenuStyle {
    /// Panel sobrio a la izquierda — el estilo default de pata.
    Classic,
    /// Windows XP — banda azul superior con usuario, dos columnas
    /// (pinned a la izquierda, "todos los programas" a la derecha),
    /// franja inferior con "Cerrar sesión" / "Apagar".
    Xp,
    /// GNOME Activities — overlay full-screen con grid de tiles
    /// centrado y buscador grande arriba. Sin chrome, full-bleed.
    Gnome,
}

impl Default for MenuStyle {
    fn default() -> Self {
        MenuStyle::Classic
    }
}

impl MenuStyle {
    /// Próximo estilo en la rotación (right-click ciclo).
    pub fn next(self) -> Self {
        match self {
            MenuStyle::Classic => MenuStyle::Xp,
            MenuStyle::Xp => MenuStyle::Gnome,
            MenuStyle::Gnome => MenuStyle::Classic,
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
        Some("tawasuyu.pata")
    }

    fn initial_size() -> (u32, u32) {
        (PANTALLA.0 as u32, PANTALLA.1 as u32)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        let cfg = pata_config::load();
        let screen = PANTALLA;
        let frame = pata_core::resolve(&cfg, Rect::new(0, 0, screen.0, screen.1));
        let (surfaces, shuma) = Model::construir(&cfg);
        let cards = Model::construir_cards(&cfg);
        let mut sampler = Sampler::with_utc(usa_utc(&cfg));
        let ctx = sampler.sample();
        let clipboard = crate::sampler::leer_clipboard();
        let tray = config_tiene_widget(&cfg, "tray")
            .then(TrayHandle::spawn)
            .flatten();
        let weather = config_tiene_widget(&cfg, "weather")
            .then(|| weather::WeatherHandle::spawn(weather_place(&cfg)));
        let cava = config_tiene_widget(&cfg, "cava").then(|| cava::CavaHandle::spawn(cava_bars(&cfg)));

        let mut theme = Theme::dark();
        if let Some(c) = render::parse_hex(&cfg.general.accent) {
            theme.accent = c;
        }
        let mut model = Model {
            theme,
            cfg,
            frame,
            surfaces,
            cards,
            shuma,
            registry: app_bus::AppRegistry::discover_merged(),
            menu_open: false,
            menu_query: String::new(),
            menu_scroll: 0.0,
            menu_style: MenuStyle::default(),
            sampler,
            clipboard,
            clip_history: Vec::new(),
            clip_open: false,
            clock_open: false,
            clock_draft: ClockDraft::default(),
            cpu_open: false,
            ram_open: false,
            volume_open: false,
            brightness_open: false,
            last_ctx: pata_core::widget::WidgetCtx::default(),
            tray,
            weather,
            weather_now: None,
            cava,
            cava_frame: Vec::new(),
            nav: NavState::default(),
            screen,
        };
        // Primer tick para que los widgets arranquen con datos.
        model.tick_widgets(&ctx);

        handle.spawn_periodic(Duration::from_secs(1), || Msg::Tick);
        // Latido del shell hospedado: drena su salida (`Tick` del módulo) a
        // ~100 ms —igual que `shuma-shell-llimphi`—, sólo si la config declara un
        // `shuma_input`. El `update` puro avanza runs y PTY/TUI sin bloquear.
        if model.shuma.present {
            handle.spawn_periodic(Duration::from_millis(100), || {
                Msg::ShumaShell(shuma_module_shell::Msg::Tick)
            });
        }
        // Visualizador de audio: re-pinta a ~20 Hz (el cuadro de cava cambia
        // rápido), pero sólo si la config declara un `cava`.
        if model.cava.is_some() {
            handle.spawn_periodic(Duration::from_millis(50), || Msg::CavaTick);
        }
        // Plano de datos del sidebar: poll de Mónadas a nouser, sólo si la config
        // declara un navegador (no molestar al broker si no hace falta).
        if config_tiene_navigator(&model.cfg) {
            handle.dispatch(Msg::NavTick);
            handle.spawn_periodic(nouser::REFRESH_INTERVAL, || Msg::NavTick);
        }
        model
    }

    fn update(mut model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        match msg {
            Msg::Tick => {
                let ctx = model.sampler.sample();
                model.tick_widgets(&ctx);
                model.last_ctx = ctx;
                model.clipboard = crate::sampler::leer_clipboard();
                push_clip_history(&mut model.clip_history, &model.clipboard);
                if let Some(h) = &model.weather {
                    if let Some(w) = h.latest() {
                        model.weather_now = Some(w);
                    }
                }
            }
            Msg::CavaTick => {
                if let Some(h) = &model.cava {
                    if let Some(frame) = h.latest() {
                        model.cava_frame = frame;
                    }
                }
            }
            Msg::Quit => handle.quit(),
            Msg::ShumaToggle => {
                if model.shuma.present {
                    model.shuma.open = !model.shuma.open;
                    let destino = if model.shuma.open { 1.0 } else { 0.0 };
                    model.animar_shuma(destino, handle);
                }
            }
            Msg::ShumaShell(m) => {
                // Click sobre el input vivo de la barra dispara FocusInput; en
                // ese caso, además, despleguemos el drawer para que la salida
                // sea visible. Idempotente: si el drawer ya está abierto, no
                // hace nada extra.
                let focusing = matches!(m, shuma_module_shell::Msg::FocusInput);
                model.shuma.inner = shuma_module_shell::update(model.shuma.inner.clone(), m);
                if focusing && model.shuma.present && !model.shuma.open {
                    model.shuma.open = true;
                    model.animar_shuma(1.0, handle);
                }
            }
            Msg::ShumaAnim => {}
            Msg::Spawn(cmd) => spawn_cmd(&cmd),
            Msg::SwitchWorkspace(n) => sampler::switch_workspace(n),
            Msg::VolumeWheel(dy) => {
                if dy != 0.0 {
                    sampler::nudge_volume(dy > 0.0);
                }
            }
            Msg::VolumeMute => sampler::toggle_mute(),
            Msg::ClipboardMenu => {
                model.clip_open = !model.clip_open;
                if model.clip_open {
                    model.menu_open = false;
                }
            }
            Msg::ClipboardPick(text) => {
                sampler::copiar_clipboard(&text);
                model.clip_open = false;
            }
            Msg::ClockPanel => {
                model.clock_open = !model.clock_open;
                if model.clock_open {
                    model.clock_draft = ClockDraft::from_now(usa_utc(&model.cfg));
                    model.menu_open = false;
                    model.clip_open = false;
                }
            }
            Msg::ClockAdjust(f, delta) => model.clock_draft.adjust(f, delta),
            Msg::ClockApply => {
                sampler::set_system_time(&model.clock_draft.stamp());
                model.clock_open = false;
            }
            Msg::ClockSyncNtp => {
                sampler::sync_ntp();
                model.clock_open = false;
            }
            Msg::BrightnessWheel(dy) => {
                if dy != 0.0 {
                    sampler::nudge_brightness(dy > 0.0);
                }
            }
            Msg::CpuPanel => {
                model.cpu_open = !model.cpu_open;
                if model.cpu_open {
                    model.ram_open = false;
                    model.volume_open = false;
                    model.brightness_open = false;
                    model.clip_open = false;
                    model.clock_open = false;
                }
            }
            Msg::RamPanel => {
                model.ram_open = !model.ram_open;
                if model.ram_open {
                    model.cpu_open = false;
                    model.volume_open = false;
                    model.brightness_open = false;
                    model.clip_open = false;
                    model.clock_open = false;
                }
            }
            Msg::VolumePanel => {
                model.volume_open = !model.volume_open;
                if model.volume_open {
                    model.cpu_open = false;
                    model.ram_open = false;
                    model.brightness_open = false;
                    model.clip_open = false;
                    model.clock_open = false;
                }
            }
            Msg::BrightnessPanel => {
                model.brightness_open = !model.brightness_open;
                if model.brightness_open {
                    model.cpu_open = false;
                    model.ram_open = false;
                    model.volume_open = false;
                    model.clip_open = false;
                    model.clock_open = false;
                }
            }
            Msg::VolumeSet(frac) => sampler::set_volume(frac),
            Msg::BrightnessSet(frac) => sampler::set_brightness(frac),
            Msg::StartToggle => {
                model.menu_open = !model.menu_open;
                if !model.menu_open {
                    model.menu_query.clear();
                    model.menu_scroll = 0.0;
                }
            }
            Msg::StartStyleCycle => {
                model.menu_style = model.menu_style.next();
            }
            Msg::StartChar(c) => {
                if !c.is_control() {
                    model.menu_query.push(c);
                    model.menu_scroll = 0.0;
                }
            }
            Msg::StartBackspace => {
                model.menu_query.pop();
                model.menu_scroll = 0.0;
            }
            Msg::StartScroll(delta) => model.menu_scroll += delta,
            Msg::StartLaunchFirst => {
                let id = render::menu_filtered(model.registry.all(), &model.menu_query)
                    .first()
                    .map(|a| a.id.clone());
                if let Some(id) = id {
                    if let Some(app) = model.registry.get(&id) {
                        let _ = app.spawn();
                    }
                    model.menu_open = false;
                    model.menu_query.clear();
                    model.menu_scroll = 0.0;
                }
            }
            Msg::LaunchApp(id) => {
                if let Some(app) = model.registry.get(&id) {
                    let _ = app.spawn();
                }
                model.menu_open = false;
                model.menu_query.clear();
                model.menu_scroll = 0.0;
            }
            Msg::TrayActivate(key) => {
                if let Some(t) = &model.tray {
                    t.activate(key);
                }
            }
            // El window_list necesita el cliente foreign-toplevel del backend
            // layer-shell; bajo el compositor mirada llegará por su IPC. No-op acá.
            Msg::ActivateWindow(_) => {}
            Msg::CloseWindow(_) => {}
            // --- Sidebar navegador (Fase 11c) ---
            Msg::NavTabActivate(si, ti) => model.nav.toggle_tab(si, ti),
            Msg::NavClosePanel => model.nav.open = None,
            Msg::NavSetMode(m) => model.nav.mode = m,
            Msg::NavSelect(id) => model.nav.selected = Some(id),
            Msg::NavToggle(id) => {
                if model.nav.expanded.contains(&id) {
                    model.nav.expanded.remove(&id);
                } else {
                    model.nav.expanded.insert(id);
                    // Carga perezosa: al abrir una Mónada sin miembros, pídelos.
                    if let (Some(mid), Some(sock)) =
                        (model.nav.needs_resolve(id), model.nav.socket.clone())
                    {
                        handle.spawn(move || Msg::NavMembers(nouser::resolve(sock, mid)));
                    }
                }
            }
            Msg::NavContextMenu(id) => {
                // Fase 11d-extra: right-click sobre un archivo abre el menú "Abrir
                // con…". Precomputamos sus apps acá (con el registro) para que el
                // render no lo toque.
                if let Some(path) = model.nav.file_path(id).map(str::to_owned) {
                    let opts = open::handlers_for_path(&model.registry, &path);
                    model.nav.open_menu(id, opts);
                }
            }
            Msg::NavOpenWith(id, app_id) => {
                if let Some(path) = model.nav.file_path(id).map(str::to_owned) {
                    match app_id {
                        Some(aid) => {
                            let _ = open::open_with_id(&model.registry, &aid, &path);
                        }
                        None => {
                            let _ = open::open_system(&path);
                        }
                    }
                }
                model.nav.close_menu();
            }
            Msg::NavMenuCancel => model.nav.close_menu(),
            // El rail hospedado vive en el backend layer-shell (conoce el foco y
            // corre el HostServer). En winit no hay toplevels: no-op.
            Msg::HostToothActivate(_, _) => {}
            Msg::NavScroll(delta) => {
                model.nav.scroll = (model.nav.scroll + delta).max(0.0);
            }
            Msg::NavTick => {
                let sock = model.nav.socket.clone();
                handle.spawn(move || Msg::NavPoll(nouser::poll(sock)));
            }
            Msg::NavPoll(outcome) => match outcome {
                PollOutcome::Ok { socket, resp } => {
                    model.nav.socket = Some(socket);
                    model.nav.apply_monads(*resp);
                }
                PollOutcome::Failed(e) => {
                    // Invalida el socket cacheado para re-descubrir en el próximo poll.
                    model.nav.socket = None;
                    model.nav.error = Some(e);
                }
            },
            Msg::NavMembers(outcome) => match outcome {
                MembersOutcome::Ok { monad, members } => model.nav.apply_members(monad, members),
                MembersOutcome::Failed(e) => model.nav.error = Some(e),
            },
        }
        model
    }

    fn view(model: &Model) -> View<Msg> {
        render::root(model)
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        // El drawer Quake tiene prioridad; luego el menú de inicio; luego los
        // popups de widgets (historial de portapapeles, panel del reloj).
        if let Some(d) = shuma::drawer_overlay(&model.shuma, model.screen, &model.theme) {
            return Some(d);
        }
        if model.menu_open {
            let bar_h = bar_thickness_for(&model.cfg, "start_button");
            let screen_size = (model.screen.0 as f32, model.screen.1 as f32);
            return Some(match model.menu_style {
                MenuStyle::Classic => render::start_menu_overlay(
                    model.registry.all(),
                    &model.menu_query,
                    model.menu_scroll,
                    bar_h,
                    screen_size.1,
                    &model.theme,
                ),
                MenuStyle::Xp => render::start_menu_xp_overlay(
                    model.registry.all(),
                    &model.menu_query,
                    model.menu_scroll,
                    bar_h,
                    screen_size,
                    &model.theme,
                ),
                MenuStyle::Gnome => render::start_menu_gnome_overlay(
                    model.registry.all(),
                    &model.menu_query,
                    bar_h,
                    screen_size,
                    &model.theme,
                ),
            });
        }
        if model.clip_open {
            let bar_h = bar_thickness_for(&model.cfg, "clipboard");
            return Some(render::clipboard_overlay(
                &model.clip_history,
                bar_h,
                &model.theme,
            ));
        }
        if model.clock_open {
            let bar_h = bar_thickness_for(&model.cfg, "clock");
            return Some(render::clock_overlay(&model.clock_draft, bar_h, &model.theme));
        }
        if model.cpu_open {
            let bar_h = bar_thickness_for(&model.cfg, "cpu_meter");
            return Some(render::cpu_overlay(&model.last_ctx, bar_h, &model.theme));
        }
        if model.ram_open {
            let bar_h = bar_thickness_for(&model.cfg, "ram_meter");
            return Some(render::ram_overlay(&model.last_ctx, bar_h, &model.theme));
        }
        if model.volume_open {
            let bar_h = bar_thickness_for(&model.cfg, "volume");
            return Some(render::volume_overlay(&model.last_ctx, bar_h, &model.theme));
        }
        if model.brightness_open {
            let bar_h = bar_thickness_for(&model.cfg, "brightness");
            return Some(render::brightness_overlay(&model.last_ctx, bar_h, &model.theme));
        }
        None
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
        // 2) Con el drawer abierto, el teclado va al **shell real**. Ctrl+Shift+W
        // repliega (el shell sigue vivo); todo lo demás —Esc/Ctrl+C/flechas/Tab/
        // texto— va al módulo, que decide entre su input de línea y el PTY/TUI.
        if model.shuma.open {
            let m = &event.modifiers;
            if m.ctrl && m.shift {
                if let Key::Character(s) = &event.key {
                    if s.eq_ignore_ascii_case("w") {
                        return Some(Msg::ShumaToggle);
                    }
                }
            }
            return Some(Msg::ShumaShell(shuma_module_shell::Msg::Key(event.clone())));
        }
        // 2.5) Con el menú de inicio abierto, el teclado va al buscador.
        if model.menu_open {
            return match &event.key {
                Key::Named(NamedKey::Escape) => Some(Msg::StartToggle),
                Key::Named(NamedKey::Backspace) => Some(Msg::StartBackspace),
                Key::Named(NamedKey::Enter) => Some(Msg::StartLaunchFirst),
                Key::Character(s) => s.chars().next().map(Msg::StartChar),
                _ => None,
            };
        }
        // 2.6) Con el popup del portapapeles o el panel del reloj abierto, Esc
        // los cierra.
        if model.clip_open {
            if let Key::Named(NamedKey::Escape) = &event.key {
                return Some(Msg::ClipboardMenu);
            }
        }
        if model.clock_open {
            if let Key::Named(NamedKey::Escape) = &event.key {
                return Some(Msg::ClockPanel);
            }
        }
        if model.cpu_open {
            if let Key::Named(NamedKey::Escape) = &event.key {
                return Some(Msg::CpuPanel);
            }
        }
        if model.ram_open {
            if let Key::Named(NamedKey::Escape) = &event.key {
                return Some(Msg::RamPanel);
            }
        }
        if model.volume_open {
            if let Key::Named(NamedKey::Escape) = &event.key {
                return Some(Msg::VolumePanel);
            }
        }
        if model.brightness_open {
            if let Key::Named(NamedKey::Escape) = &event.key {
                return Some(Msg::BrightnessPanel);
            }
        }
        // 3) Con el menú "Abrir con…" abierto, Esc lo cierra primero.
        if model.nav.menu.is_some() {
            if let Key::Named(NamedKey::Escape) = &event.key {
                return Some(Msg::NavMenuCancel);
            }
        }
        // 4) Con el panel navegador desplegado, Esc lo cierra (no la app).
        if model.nav.open.is_some() {
            if let Key::Named(NamedKey::Escape) = &event.key {
                return Some(Msg::NavClosePanel);
            }
        }
        // 5) Sin nada abierto, Esc cierra la app.
        match &event.key {
            Key::Named(NamedKey::Escape) => Some(Msg::Quit),
            _ => None,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn historial_dedup_y_tope() {
        let mut h = Vec::new();
        push_clip_history(&mut h, &Some("a".into()));
        push_clip_history(&mut h, &Some("b".into()));
        push_clip_history(&mut h, &Some("a".into())); // re-copia: a vuelve al frente
        assert_eq!(h, vec!["a".to_string(), "b".to_string()]);
        // vacío y repetido del tope se ignoran
        push_clip_history(&mut h, &Some(String::new()));
        push_clip_history(&mut h, &Some("a".into()));
        assert_eq!(h.len(), 2);
        // tope
        for i in 0..30 {
            push_clip_history(&mut h, &Some(format!("x{i}")));
        }
        assert_eq!(h.len(), CLIP_HISTORY_MAX);
    }

    #[test]
    fn clock_draft_ajusta_con_wrap_y_clamp() {
        let mut d = ClockDraft {
            year: 2026,
            month: 12,
            day: 1,
            hour: 23,
            minute: 59,
        };
        d.adjust(1, 1); // mes 12 +1 → 1 (wrap)
        assert_eq!(d.month, 1);
        d.adjust(3, 1); // hora 23 +1 → 0 (wrap)
        assert_eq!(d.hour, 0);
        d.adjust(4, 1); // min 59 +1 → 0 (wrap)
        assert_eq!(d.minute, 0);
        d.adjust(0, -1000); // año clamp inferior
        assert_eq!(d.year, 1970);
        d.adjust(2, 100); // día clamp superior
        assert_eq!(d.day, 31);
    }

    #[test]
    fn clock_draft_stamp() {
        let d = ClockDraft {
            year: 2026,
            month: 6,
            day: 5,
            hour: 9,
            minute: 7,
        };
        assert_eq!(d.stamp(), "2026-06-05 09:07:00");
    }
}
