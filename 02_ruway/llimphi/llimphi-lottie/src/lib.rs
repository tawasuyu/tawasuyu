//! `llimphi-lottie` — puente fino entre `velato` (reproductor de Lottie de
//! Linebender) y Llimphi.
//!
//! `llimphi-svg` cubre el vector *estático* arbitrario (íconos `.desktop`,
//! logos). `llimphi-motion` cubre el movimiento *generado por código* (tweens
//! sobre el bucle Elm). Este crate cubre el hueco del medio: **animación
//! vectorial autorada por fuera** — los `.json` de Lottie que exportan los
//! diseñadores desde After Effects / lottiefiles (íconos animados,
//! ilustraciones, estados vacíos, onboarding).
//!
//! Es el gemelo de `llimphi-svg`, con un eje de tiempo: `velato` parsea el JSON
//! a una `Composition` y, para un `frame` dado, emite a la **misma**
//! `vello::Scene` que usa `llimphi-raster`.
//!
//! ## Uso
//!
//! ```ignore
//! use llimphi_lottie::LottieAsset;
//! use std::time::Duration;
//!
//! // Parseá UNA vez (al cargar la app):
//! let anim = LottieAsset::from_str(include_str!("spinner.json")).expect("lottie válido");
//!
//! // En el estado de la app guardás el tiempo transcurrido y lo avanzás con el
//! // bucle Elm — exactamente el patrón de los workers de simulación:
//! //   handle.spawn_periodic(Duration::from_millis(16), || Msg::Tick);
//! // y en update: self.t += 1.0 / 60.0;
//!
//! // Pintalo a su instante actual (escala + centra al rect, en loop):
//! View::new(style).children(vec![anim.view_at_time::<Msg>(self.t)])
//! ```
//!
//! El parse cuesta (corre el importador serde de Lottie); el `paint`/`view` no.
//! `LottieAsset` es `Clone` barato (`Arc` internamente) — el mismo asset en
//! varios nodos no replica memoria.
//!
//! ## Por qué un `Renderer` nuevo por paint
//!
//! `velato::Renderer` es `Default` y sólo guarda Vecs de scratch que se limpian
//! en cada `append`. No tiene recursos de GPU (eso es trabajo de `llimphi-hal`).
//! Crear uno por frame es despreciable frente a empujar la geometría — que se
//! hace igual — y nos deja un `LottieAsset` `Send + Sync` sin `Mutex`.
//!
//! ## Cobertura
//!
//! `velato` 0.9 implementa shapes, transforms animados, gradientes, trim paths,
//! máscaras y precomposiciones — el grueso de los Lottie reales de íconos e
//! ilustración. **No** implementa todavía capas de **texto**, **effects**
//! (blur/drop-shadow) ni **expresiones** (declarado en su propio
//! `schema/mod.rs`). Un asset que use esas features se pinta omitiéndolas, no
//! rompe. Cuando un `.json` concreto las necesite, el camino es vendorizar
//! `velato` como `shared/foreign-lottie` y completar el `todo` puntual.

#![forbid(unsafe_code)]

use std::sync::Arc;

use llimphi_ui::llimphi_layout::taffy::prelude::{percent, Size, Style};
use llimphi_ui::llimphi_layout::taffy::Position;
use llimphi_ui::llimphi_raster::kurbo::Affine;
use llimphi_ui::llimphi_raster::vello::Scene;
use llimphi_ui::{PaintRect, View};

// El motor es nuestro fork vendorizado de velato; lo aliasamos a `velato` para
// que el resto del crate lea igual que la documentación upstream.
use foreign_lottie as velato;
use velato::Composition;

/// Animación Lottie parseada y lista para stampear a cualquier `frame`.
/// Internamente guarda la `Composition` de `velato` (el modelo del `.json`) +
/// su geometría temporal. Cloneable barato (`Arc`) — un mismo asset en varios
/// nodos no replica memoria. `Send + Sync`.
#[derive(Clone)]
pub struct LottieAsset {
    inner: Arc<Composition>,
}

/// Error al parsear un Lottie. Hoy es un wrap del texto del error de `velato` —
/// las apps típicas lo tratan como "fallback" (no animan) y no inspeccionan la
/// variante.
#[derive(Debug)]
pub struct LottieError(pub String);

impl std::fmt::Display for LottieError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "lottie: {}", self.0)
    }
}

impl std::error::Error for LottieError {}

impl LottieAsset {
    /// Parsea un Lottie desde su JSON como string. Devuelve un asset inmutable +
    /// cloneable. **Hace el parseo completo** (serde + importación al modelo
    /// runtime de `velato`); pensado para llamarse UNA vez por asset, no por
    /// frame.
    pub fn from_str(json: &str) -> Result<Self, LottieError> {
        Self::from_bytes(json.as_bytes())
    }

    /// Parsea un Lottie desde sus bytes UTF-8 crudos (lo que devuelve
    /// `include_bytes!("…json")` o `std::fs::read`).
    ///
    /// **Blindaje contra panics.** Nuestro fork `foreign-lottie` ya eliminó los
    /// `todo!()`/`unimplemented!()` del importador de velato 0.9 (split
    /// rotation/position, blends `Add`/`HardMix`, assets desconocidos, transform
    /// sin rotación) — degradan con gracia en vez de paniquear. El `catch_unwind`
    /// queda como red de seguridad secundaria: por si un `.json` raro alcanza
    /// algún `panic!`/`unwrap` residual del deserializador, el asset cae al
    /// fallback en vez de tumbar el hilo de UI. El render (`append`) es seguro,
    /// así que no lo envolvemos por-frame.
    pub fn from_bytes(json: &[u8]) -> Result<Self, LottieError> {
        let parsed = std::panic::catch_unwind(|| Composition::from_slice(json));
        let comp = match parsed {
            Ok(Ok(comp)) => comp,
            Ok(Err(e)) => return Err(LottieError(e.to_string())),
            Err(_) => {
                return Err(LottieError(
                    "velato paniqueó al importar (feature no soportada: split \
                     rotation/position, asset o blend sin implementar)"
                        .to_string(),
                ))
            }
        };
        Ok(Self {
            inner: Arc::new(comp),
        })
    }

    /// Tamaño nominal de la animación en px (el `w`/`h` del Lottie). Útil para
    /// dimensionar el rect destino preservando aspect ratio.
    pub fn size(&self) -> (f64, f64) {
        (self.inner.width as f64, self.inner.height as f64)
    }

    /// Cuadros por segundo declarados en el `.json` (`fr`).
    pub fn frame_rate(&self) -> f64 {
        self.inner.frame_rate
    }

    /// Rango de frames activos `[ip, op)` del Lottie. `paint` espera un `frame`
    /// dentro de este rango; fuera de él, lo clampa a los extremos.
    pub fn frames(&self) -> std::ops::Range<f64> {
        self.inner.frames.clone()
    }

    /// Duración de una pasada completa, en segundos. `0.0` si el `.json` no
    /// declara `fr` o no tiene frames.
    pub fn duration_secs(&self) -> f64 {
        let span = self.inner.frames.end - self.inner.frames.start;
        if self.inner.frame_rate > 0.0 && span > 0.0 {
            span / self.inner.frame_rate
        } else {
            0.0
        }
    }

    /// Convierte un instante en segundos a un `frame` dentro del rango activo,
    /// **en loop** (módulo la duración). Es el mapeo que usa `view_at_time` /
    /// `paint_at_time`. Si el asset no tiene duración válida, devuelve el primer
    /// frame.
    pub fn frame_at_time(&self, t_secs: f64) -> f64 {
        let start = self.inner.frames.start;
        let span = self.inner.frames.end - start;
        if span <= 0.0 || self.inner.frame_rate <= 0.0 {
            return start;
        }
        let frames_elapsed = (t_secs.max(0.0) * self.inner.frame_rate) % span;
        start + frames_elapsed
    }

    /// Pinta la animación al `frame` indicado sobre `scene`, ajustada al `rect`.
    /// Escala uniforme al mínimo lado y **centra** dentro del rect (preserva
    /// aspect ratio). `frame` se clampa al rango activo del Lottie. Útil cuando
    /// el caller compone varios assets en un `paint_with` propio sin pasar por
    /// `view()`.
    pub fn paint(&self, scene: &mut Scene, rect: PaintRect, frame: f64) {
        self.paint_alpha(scene, rect, frame, 1.0);
    }

    /// Como `paint`, pero con una opacidad global `alpha` (0..1) aplicada a toda
    /// la composición. Es lo que habilita el **crossfade** entre clips de una
    /// máquina de estados (pintar el saliente a `1-mix` y el entrante a `mix`).
    /// `velato` lo soporta nativo: el `alpha` va al `append`.
    pub fn paint_alpha(&self, scene: &mut Scene, rect: PaintRect, frame: f64, alpha: f64) {
        let (vb_w, vb_h) = self.size();
        let side_w = rect.w as f64;
        let side_h = rect.h as f64;
        if side_w <= 0.0 || side_h <= 0.0 || vb_w <= 0.0 || vb_h <= 0.0 || alpha <= 0.0 {
            return;
        }
        let s = (side_w / vb_w).min(side_h / vb_h);
        let used_w = vb_w * s;
        let used_h = vb_h * s;
        let tx = rect.x as f64 + (side_w - used_w) * 0.5;
        let ty = rect.y as f64 + (side_h - used_h) * 0.5;
        let xform = Affine::translate((tx, ty)) * Affine::scale(s);

        let frame = frame.clamp(self.inner.frames.start, self.inner.frames.end);
        // Renderer nuevo por paint: es `Default`, sólo scratch Vecs, sin GPU.
        let mut renderer = velato::Renderer::new();
        renderer.append(&self.inner, frame, xform, alpha.clamp(0.0, 1.0), scene);
    }

    /// Como `paint`, pero recibe el instante en **segundos** y lo mapea a frame
    /// en loop vía `frame_at_time`. Es la forma esperada de animar desde el
    /// estado de la app (acumulás `t` en segundos en `update`).
    pub fn paint_at_time(&self, scene: &mut Scene, rect: PaintRect, t_secs: f64) {
        self.paint(scene, rect, self.frame_at_time(t_secs));
    }

    /// Construye un `View` posicionado en absoluto que ocupa todo el rect del
    /// padre y pinta la animación al `frame` indicado, centrada + escalada al
    /// mínimo lado. Gemelo de `SvgAsset::view`, con frame. Genérico sobre `Msg`
    /// igual que los widgets — el `View` no tiene handlers; la app los pone en
    /// el padre.
    pub fn view<Msg>(&self, frame: f64) -> View<Msg> {
        let asset = self.clone();
        View::new(absolute_fill())
            .paint_with(move |scene, _ts, rect| asset.paint(scene, rect, frame))
    }

    /// Como `view`, pero recibe el instante en segundos (loop vía
    /// `frame_at_time`). Es la variante que usás con un `t` acumulado en el
    /// estado de la app.
    pub fn view_at_time<Msg>(&self, t_secs: f64) -> View<Msg> {
        let asset = self.clone();
        View::new(absolute_fill())
            .paint_with(move |scene, _ts, rect| asset.paint_at_time(scene, rect, t_secs))
    }
}

/// Pinta un [`RenderFrame`] de una máquina de estados [`llimphi_anim`] usando
/// `clips` indexados por `ClipId` (= índice en el slice). Pinta el clip primario
/// y, si hay una transición en curso, hace **crossfade**: saliente a `1-mix`,
/// entrante a `mix`. Los `ClipId` fuera de rango se omiten (no rompe).
///
/// `ClipSample::time_secs` se mapea a frame con loop vía `frame_at_time`, así
/// que cada clip respeta su propio fps y duración.
///
/// [`RenderFrame`]: llimphi_anim::RenderFrame
pub fn paint_render_frame(
    scene: &mut Scene,
    rect: PaintRect,
    frame: &llimphi_anim::RenderFrame,
    clips: &[LottieAsset],
) {
    let sample = |s: llimphi_anim::ClipSample, alpha: f64, scene: &mut Scene| {
        if let Some(asset) = clips.get(s.clip as usize) {
            let f = asset.frame_at_time(s.time_secs);
            asset.paint_alpha(scene, rect, f, alpha);
        }
    };
    match frame.blend {
        None => sample(frame.primary, 1.0, scene),
        Some((incoming, mix)) => {
            let mix = mix as f64;
            // Dissolve: el saliente se desvanece mientras el entrante aparece.
            sample(frame.primary, 1.0 - mix, scene);
            sample(incoming, mix, scene);
        }
    }
}

/// Construye un `View` absoluto que ocupa el rect del padre y pinta el
/// [`RenderFrame`] actual de una máquina de estados (con crossfade). El caller
/// llama `instance.render_frame()` cada frame y le pasa el resultado + sus clips
/// (cloneados; baratos por `Arc`). Es el gemelo de [`LottieAsset::view`] para
/// animación dirigida por estado en vez de por un solo clip.
///
/// [`RenderFrame`]: llimphi_anim::RenderFrame
pub fn state_machine_view<Msg>(
    frame: llimphi_anim::RenderFrame,
    clips: Vec<LottieAsset>,
) -> View<Msg> {
    View::new(absolute_fill())
        .paint_with(move |scene, _ts, rect| paint_render_frame(scene, rect, &frame, &clips))
}

/// Estilo "ocupa todo el rect del padre, en absoluto" — compartido por las dos
/// variantes de `view`.
fn absolute_fill() -> Style {
    Style {
        position: Position::Absolute,
        size: Size {
            width: percent(1.0_f32),
            height: percent(1.0_f32),
        },
        ..Default::default()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Lottie mínimo válido: 60 frames a 30 fps, lienzo 100×100, sin capas.
    /// (Suficiente para ejercitar parse + geometría temporal; el render de un
    /// shape real lo cubren los assets de ejemplo, no un unit test.)
    const LOTTIE_OK: &str =
        r#"{"v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,"layers":[]}"#;

    #[test]
    fn from_str_parsea_ok() {
        let a = LottieAsset::from_str(LOTTIE_OK).expect("parsea");
        let (w, h) = a.size();
        assert_eq!((w, h), (100.0, 100.0));
        assert_eq!(a.frame_rate(), 30.0);
    }

    #[test]
    fn duracion_y_rango() {
        let a = LottieAsset::from_str(LOTTIE_OK).expect("parsea");
        assert_eq!(a.frames(), 0.0..60.0);
        // 60 frames / 30 fps = 2 s.
        assert!((a.duration_secs() - 2.0).abs() < 1e-9);
    }

    #[test]
    fn frame_at_time_hace_loop() {
        let a = LottieAsset::from_str(LOTTIE_OK).expect("parsea");
        // En t=0 → frame 0.
        assert!((a.frame_at_time(0.0) - 0.0).abs() < 1e-9);
        // En t=1 s (mitad) → frame 30.
        assert!((a.frame_at_time(1.0) - 30.0).abs() < 1e-9);
        // En t=2 s (una pasada exacta) → vuelve a 0 por el módulo.
        assert!((a.frame_at_time(2.0) - 0.0).abs() < 1e-9);
        // En t=2.5 s → frame 15 (loop).
        assert!((a.frame_at_time(2.5) - 15.0).abs() < 1e-9);
    }

    #[test]
    fn json_inválido_da_error() {
        assert!(LottieAsset::from_str("{no es lottie}").is_err());
    }

    #[test]
    fn asset_es_cloneable_barato() {
        let a = LottieAsset::from_str(LOTTIE_OK).expect("parsea");
        let b = a.clone();
        assert_eq!(a.size(), b.size());
    }

    #[test]
    fn paint_no_panica_con_rect_cero() {
        let a = LottieAsset::from_str(LOTTIE_OK).expect("parsea");
        let mut s = Scene::new();
        a.paint(&mut s, PaintRect { x: 0.0, y: 0.0, w: 0.0, h: 0.0 }, 0.0);
        a.paint_at_time(&mut s, PaintRect { x: 0.0, y: 0.0, w: 10.0, h: 0.0 }, 0.5);
    }

    /// `Send + Sync` es parte del contrato (igual que `SvgAsset`): el asset se
    /// mueve a closures de paint que pueden cruzar threads.
    #[test]
    fn asset_es_send_sync() {
        fn assert_send_sync<T: Send + Sync>() {}
        assert_send_sync::<LottieAsset>();
    }

    /// Lottie con un shape real: un rectángulo rojo de 80×80 centrado en un
    /// lienzo 100×100, estático 60 frames. Sirve para certificar — con
    /// evidencia textual, no PNG — que la geometría llega a la `vello::Scene`.
    const LOTTIE_RECT: &str = r#"{
      "v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,
      "layers":[{
        "ty":4,"ip":0,"op":60,"st":0,"sr":1,
        "ks":{"p":{"a":0,"k":[0,0]},"r":{"a":0,"k":0}},
        "shapes":[
          {"ty":"rc","p":{"a":0,"k":[50,50]},"s":{"a":0,"k":[80,80]},"r":{"a":0,"k":0}},
          {"ty":"fl","c":{"a":0,"k":[0.8,0,0,1]},"o":{"a":0,"k":100}}
        ]
      }]
    }"#;

    #[test]
    fn shape_real_emite_geometria_a_la_scene() {
        let a = LottieAsset::from_str(LOTTIE_RECT).expect("parsea");
        let mut scene = Scene::new();
        // Antes de pintar: la Scene está vacía.
        assert!(scene.encoding().is_empty(), "scene recién creada debe estar vacía");
        a.paint(&mut scene, PaintRect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 }, 0.0);
        // Después: velato empujó el rect → encoding con contenido.
        assert!(
            !scene.encoding().is_empty(),
            "tras paint de un shape real, la Scene debe tener geometría"
        );
    }

    /// Mismo layer pero con el transform SIN campo de rotación (`r`). En velato
    /// 0.9 upstream esto **paniquea** (`todo!("split rotation")`); nuestro fork
    /// `foreign-lottie` lo trata como rotación 0 y **parsea bien**. Aquí
    /// certificamos el resultado del fork: parse Ok, sin panic.
    const LOTTIE_SIN_ROTACION: &str = r#"{
      "v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,
      "layers":[{
        "ty":4,"ip":0,"op":60,"st":0,"sr":1,
        "ks":{"p":{"a":0,"k":[0,0]}},
        "shapes":[]
      }]
    }"#;

    #[test]
    fn lottie_sin_rotacion_parsea_con_el_fork() {
        // En velato upstream esto era un panic; el fork lo importa como rot=0.
        let a = LottieAsset::from_str(LOTTIE_SIN_ROTACION)
            .expect("el fork parsea un transform sin rotación");
        assert_eq!(a.size(), (100.0, 100.0));
    }

    /// Segundo clip distinto al rect: un círculo, para tener dos animaciones que
    /// mezclar en la máquina de estados.
    const LOTTIE_CIRC: &str = r#"{
      "v":"5.5.2","fr":30,"ip":0,"op":60,"w":100,"h":100,
      "layers":[{
        "ty":4,"ip":0,"op":60,"st":0,"sr":1,
        "ks":{"p":{"a":0,"k":[50,50]},"r":{"a":0,"k":0}},
        "shapes":[
          {"ty":"el","p":{"a":0,"k":[0,0]},"s":{"a":0,"k":[70,70]}},
          {"ty":"fl","c":{"a":0,"k":[0,0.4,0.9,1]},"o":{"a":0,"k":100}}
        ]
      }]
    }"#;

    /// E2E del Tier 1: máquina de estados (llimphi-anim) + dos clips Lottie +
    /// `paint_render_frame`. Verifica que tanto el estado simple como el
    /// crossfade emiten geometría a la `Scene` (evidencia textual, sin PNG).
    #[test]
    fn state_machine_pinta_clips_y_crossfade() {
        use llimphi_anim::{Condition, StateMachine};

        let rect = LottieAsset::from_str(LOTTIE_RECT).expect("rect");
        let circ = LottieAsset::from_str(LOTTIE_CIRC).expect("circ");
        let clips = vec![rect, circ]; // ClipId 0 = rect, 1 = circ

        let mut sm = StateMachine::new();
        let idle = sm.add_state("idle", 0, 1.0, true);
        let walk = sm.add_state("walk", 1, 1.0, true);
        sm.set_entry(idle);
        sm.transition(idle, walk, vec![Condition::bool("moving", true)], 0.4);
        let mut inst = sm.instance();

        let big = PaintRect { x: 0.0, y: 0.0, w: 200.0, h: 200.0 };

        // Estado simple: pinta sólo el clip primario (rect).
        let mut s0 = Scene::new();
        paint_render_frame(&mut s0, big, &inst.render_frame(), &clips);
        assert!(!s0.encoding().is_empty(), "estado simple debe pintar geometría");

        // Arranca la transición y caé a mitad del blend.
        inst.set_bool("moving", true);
        inst.advance(0.2); // 0.2/0.4 = mix 0.5
        let rf = inst.render_frame();
        assert!(rf.blend.is_some(), "debería estar en crossfade");
        let mut s1 = Scene::new();
        paint_render_frame(&mut s1, big, &rf, &clips);
        assert!(!s1.encoding().is_empty(), "el crossfade debe pintar geometría");

        // El view helper compila y produce un View sin panic.
        let _v = state_machine_view::<()>(rf, clips);
    }
}
