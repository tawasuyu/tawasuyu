//! config — la configuración **completa y única** del reproductor, el
//! modelo que edita la ventana de configuración (regla #2: la lógica de
//! "qué se puede personalizar" vive acá; la ventana sólo la pinta).
//!
//! [`MediaConfig`] agrega todo lo personalizable en una estructura
//! seccionada (cada sección = una pestaña de la ventana): los **controles**
//! (keymap + pasos + scripts, [`crate::control::ControlSettings`]), el
//! **layout** de paneles ([`crate::layout::LayoutSettings`]) y prefs nuevas
//! de **playlist**, **audio**, **video**, **subtítulos** y **comportamiento**.
//!
//! Diseño forward-compatible: cada campo lleva `#[serde(default)]`, así un
//! `config.ron` viejo —escrito antes de agregar una sección— sigue cargando
//! (la sección faltante toma su default). Igual criterio que `layout`/
//! `control`. La app persiste el RON; este módulo no hace I/O — sólo
//! defaults, `sanitized()` (clampea rangos) y round-trip.

use serde::{Deserialize, Serialize};

use crate::control::ControlSettings;
use crate::layout::LayoutSettings;
use crate::playlist::Repeat;
use crate::toolbar::Toolbar;

/// Configuración completa del reproductor. Una sección por pestaña de la
/// ventana de configuración.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct MediaConfig {
    /// Controles: keymap, pasos de seek/volumen/velocidad, scripts Rhai.
    #[serde(default)]
    pub controls: ControlSettings,
    /// Orden de los paneles de control.
    #[serde(default)]
    pub layout: LayoutSettings,
    /// Barras de controles componibles (botones simples estilo VLC/eww).
    #[serde(default)]
    pub toolbar: Toolbar,
    /// Comportamiento de la cola: repeat/shuffle/resume por defecto.
    #[serde(default)]
    pub playlist: PlaylistPrefs,
    /// Audio: volumen inicial, EQ, normalización, downmix.
    #[serde(default)]
    pub audio: AudioPrefs,
    /// Video: ajustes de color y orientación por defecto.
    #[serde(default)]
    pub video: VideoPrefs,
    /// Subtítulos: auto-carga, desfase, tamaño.
    #[serde(default)]
    pub subtitles: SubtitlePrefs,
    /// Comportamiento general (crossfade, dónde guardar capturas…).
    #[serde(default)]
    pub behavior: BehaviorPrefs,
}

impl Default for MediaConfig {
    fn default() -> Self {
        MediaConfig {
            controls: ControlSettings::default(),
            layout: LayoutSettings::default(),
            toolbar: Toolbar::default(),
            playlist: PlaylistPrefs::default(),
            audio: AudioPrefs::default(),
            video: VideoPrefs::default(),
            subtitles: SubtitlePrefs::default(),
            behavior: BehaviorPrefs::default(),
        }
    }
}

impl MediaConfig {
    /// Reconcilia una config cargada de disco: sanea cada sección
    /// (clampea rangos, anexa paneles faltantes, etc.). Idempotente.
    pub fn sanitized(self) -> MediaConfig {
        MediaConfig {
            controls: self.controls,
            layout: self.layout.sanitized(),
            toolbar: self.toolbar.sanitized(),
            playlist: self.playlist.sanitized(),
            audio: self.audio.sanitized(),
            video: self.video.sanitized(),
            subtitles: self.subtitles.sanitized(),
            behavior: self.behavior.sanitized(),
        }
    }
}

/// Comportamiento por defecto de la cola de reproducción.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct PlaylistPrefs {
    /// Modo de repetición con el que arranca.
    #[serde(default)]
    pub repeat: Repeat,
    /// Si arranca en aleatorio.
    #[serde(default)]
    pub shuffle: bool,
    /// Si al abrir un medio conocido ofrece reanudar donde quedó (U2).
    #[serde(default = "yes")]
    pub resume_on_open: bool,
    /// Tope de entradas del historial (evicción LRU).
    #[serde(default = "default_history_cap")]
    pub history_capacity: usize,
}

impl Default for PlaylistPrefs {
    fn default() -> Self {
        PlaylistPrefs {
            repeat: Repeat::Off,
            shuffle: false,
            resume_on_open: true,
            history_capacity: 200,
        }
    }
}

impl PlaylistPrefs {
    pub fn sanitized(mut self) -> Self {
        self.history_capacity = self.history_capacity.clamp(1, 100_000);
        self
    }
}

/// Audio: estado inicial de la cadena (volumen, EQ, normalización, mezcla).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct AudioPrefs {
    /// Volumen inicial (0.0..=4.0; 1.0 = 100 %).
    #[serde(default = "one")]
    pub volume: f32,
    /// EQ encendido al arrancar.
    #[serde(default)]
    pub eq_enabled: bool,
    /// Ganancias por banda en dB (típicamente 10 bandas ISO).
    #[serde(default = "default_eq_bands")]
    pub eq_bands_db: Vec<f32>,
    /// Normalización (makeup + limitador) encendida al arrancar.
    #[serde(default)]
    pub normalization_enabled: bool,
    /// Objetivo de sonoridad para la normalización automática (LUFS).
    #[serde(default = "default_lufs")]
    pub normalization_target_lufs: f32,
    /// Plegar multicanal a estéreo (downmix) en vez de pedir el layout nativo.
    #[serde(default = "yes")]
    pub downmix_to_stereo: bool,
}

impl Default for AudioPrefs {
    fn default() -> Self {
        AudioPrefs {
            volume: 1.0,
            eq_enabled: false,
            eq_bands_db: vec![0.0; 10],
            normalization_enabled: false,
            normalization_target_lufs: -18.0,
            downmix_to_stereo: true,
        }
    }
}

impl AudioPrefs {
    pub fn sanitized(mut self) -> Self {
        self.volume = self.volume.clamp(0.0, 4.0);
        for b in &mut self.eq_bands_db {
            *b = b.clamp(-24.0, 24.0);
        }
        self.normalization_target_lufs = self.normalization_target_lufs.clamp(-40.0, 0.0);
        self
    }
}

/// Video: ajustes de color (V4) y orientación (V3) por defecto.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct VideoPrefs {
    /// Ajustes de color encendidos al arrancar.
    #[serde(default)]
    pub color_enabled: bool,
    #[serde(default)]
    pub brightness: f32,
    #[serde(default = "one")]
    pub contrast: f32,
    #[serde(default = "one")]
    pub gamma: f32,
    #[serde(default = "one")]
    pub saturation: f32,
    #[serde(default)]
    pub hue: f32,
    /// Rotación horaria por defecto (0/90/180/270).
    #[serde(default)]
    pub rotation: u16,
    #[serde(default)]
    pub flip_h: bool,
    #[serde(default)]
    pub flip_v: bool,
}

impl Default for VideoPrefs {
    fn default() -> Self {
        VideoPrefs {
            color_enabled: false,
            brightness: 0.0,
            contrast: 1.0,
            gamma: 1.0,
            saturation: 1.0,
            hue: 0.0,
            rotation: 0,
            flip_h: false,
            flip_v: false,
        }
    }
}

impl VideoPrefs {
    pub fn sanitized(mut self) -> Self {
        self.brightness = self.brightness.clamp(-1.0, 1.0);
        self.contrast = self.contrast.clamp(0.0, 4.0);
        self.gamma = self.gamma.clamp(0.1, 5.0);
        self.saturation = self.saturation.clamp(0.0, 4.0);
        // Hue en (-180, 180].
        self.hue = wrap_hue(self.hue);
        // Rotación al múltiplo de 90 más cercano dentro de {0,90,180,270}.
        self.rotation = match ((self.rotation as f32 / 90.0).round() as i64).rem_euclid(4) {
            1 => 90,
            2 => 180,
            3 => 270,
            _ => 0,
        };
        self
    }
}

/// Subtítulos: auto-carga, desfase y tamaño por defecto.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct SubtitlePrefs {
    /// Auto-cargar el sidecar `.srt`/`.vtt`/`.ass` junto al video (S5).
    #[serde(default = "yes")]
    pub autoload_sidecar: bool,
    /// Desfase inicial del subtítulo en ms (S4). Positivo lo retrasa.
    #[serde(default)]
    pub delay_ms: i64,
    /// Factor de escala del tamaño de letra (1.0 = tamaño base).
    #[serde(default = "one")]
    pub font_scale: f32,
}

impl Default for SubtitlePrefs {
    fn default() -> Self {
        SubtitlePrefs {
            autoload_sidecar: true,
            delay_ms: 0,
            font_scale: 1.0,
        }
    }
}

impl SubtitlePrefs {
    pub fn sanitized(mut self) -> Self {
        self.delay_ms = self.delay_ms.clamp(-60_000, 60_000);
        self.font_scale = self.font_scale.clamp(0.3, 4.0);
        self
    }
}

/// Comportamiento general del reproductor.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct BehaviorPrefs {
    /// Crossfade entre pistas en segundos (0 = gapless, sin fundido) (A6).
    #[serde(default)]
    pub crossfade_secs: f32,
    /// Recordar/persistir el layout de paneles al cerrar.
    #[serde(default = "yes")]
    pub remember_layout: bool,
    /// Carpeta donde guardar las capturas (None = directorio actual).
    #[serde(default)]
    pub snapshot_dir: Option<String>,
}

impl Default for BehaviorPrefs {
    fn default() -> Self {
        BehaviorPrefs {
            crossfade_secs: 0.0,
            remember_layout: true,
            snapshot_dir: None,
        }
    }
}

impl BehaviorPrefs {
    pub fn sanitized(mut self) -> Self {
        self.crossfade_secs = self.crossfade_secs.clamp(0.0, 12.0);
        self
    }
}

// ---------- helpers de #[serde(default = "...")] ----------
// serde necesita una fn con nombre para defaults != Default::default().

fn yes() -> bool {
    true
}
fn one() -> f32 {
    1.0
}
fn default_history_cap() -> usize {
    200
}
fn default_lufs() -> f32 {
    -18.0
}
fn default_eq_bands() -> Vec<f32> {
    vec![0.0; 10]
}

/// Envuelve un ángulo a `(-180, 180]`.
fn wrap_hue(deg: f32) -> f32 {
    let mut h = deg % 360.0;
    if h > 180.0 {
        h -= 360.0;
    } else if h <= -180.0 {
        h += 360.0;
    }
    h
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_es_coherente() {
        let c = MediaConfig::default();
        assert_eq!(c.audio.volume, 1.0);
        assert_eq!(c.audio.eq_bands_db.len(), 10);
        assert!(c.playlist.resume_on_open);
        assert_eq!(c.playlist.repeat, Repeat::Off);
        assert!(c.subtitles.autoload_sidecar);
        assert_eq!(c.video.rotation, 0);
    }

    #[test]
    fn sanitized_clampa_todo() {
        let mut c = MediaConfig::default();
        c.audio.volume = 99.0;
        c.audio.eq_bands_db = vec![100.0, -100.0];
        c.audio.normalization_target_lufs = 50.0;
        c.video.contrast = -5.0;
        c.video.gamma = 0.0;
        c.video.hue = 540.0; // → 180
        c.video.rotation = 100; // → 90
        c.subtitles.delay_ms = 999_999;
        c.subtitles.font_scale = 50.0;
        c.behavior.crossfade_secs = 99.0;
        c.playlist.history_capacity = 0;

        let s = c.sanitized();
        assert_eq!(s.audio.volume, 4.0);
        assert!(s.audio.eq_bands_db.iter().all(|&b| b.abs() <= 24.0));
        assert_eq!(s.audio.normalization_target_lufs, 0.0);
        assert_eq!(s.video.contrast, 0.0);
        assert!((s.video.gamma - 0.1).abs() < 1e-6);
        assert!((s.video.hue - 180.0).abs() < 1e-4);
        assert_eq!(s.video.rotation, 90);
        assert_eq!(s.subtitles.delay_ms, 60_000);
        assert!((s.subtitles.font_scale - 4.0).abs() < 1e-6);
        assert!((s.behavior.crossfade_secs - 12.0).abs() < 1e-6);
        assert_eq!(s.playlist.history_capacity, 1);
    }

    #[test]
    fn rotation_redondea_al_multiplo_de_90() {
        let mut v = VideoPrefs::default();
        v.rotation = 269;
        assert_eq!(v.clone().sanitized().rotation, 270);
        v.rotation = 44;
        assert_eq!(v.clone().sanitized().rotation, 0);
        v.rotation = 46;
        assert_eq!(v.sanitized().rotation, 90);
    }

    #[test]
    fn round_trip_ron_completo() {
        let c = MediaConfig::default();
        let txt = ron::ser::to_string(&c).expect("serializa");
        let back: MediaConfig = ron::from_str(&txt).expect("deserializa");
        assert_eq!(c, back);
    }

    #[test]
    fn ron_parcial_toma_defaults() {
        // Un config.ron mínimo (sólo una sección) debe cargar: el resto
        // toma sus defaults por #[serde(default)]. Esto es lo que protege
        // a una config vieja cuando se agrega una sección nueva.
        let parcial = "(audio: (volume: 0.5))";
        let c: MediaConfig = ron::from_str(parcial).expect("carga parcial");
        assert_eq!(c.audio.volume, 0.5);
        // El resto, defaults.
        assert!(c.subtitles.autoload_sidecar);
        assert_eq!(c.playlist.history_capacity, 200);
        assert_eq!(c.layout.panels.len(), crate::layout::PanelId::ALL.len());
    }
}
