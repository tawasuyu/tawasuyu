//! control — vocabulario agnóstico de comandos de reproducción y
//! keymap configurable (estilo VLC, más flexible).
//!
//! La regla #2 del repo: la lógica de dominio no sabe quién la pinta
//! **ni qué teclas la disparan**. Por eso este módulo no depende de
//! `winit` ni de Llimphi: define un [`MediaCommand`] semántico, un
//! [`KeyChord`] de teclas normalizadas como `String`, y un [`Keymap`]
//! que resuelve chord → comando. La UI traduce su evento de teclado a
//! un `KeyChord`, lo resuelve, y despacha el comando.
//!
//! Más flexible que VLC porque los comandos están **parametrizados**:
//! el mismo [`MediaCommand::SeekBy`] sirve para un salto de 5 s o de
//! 30 s según el binding, y se puede atar una tecla directamente a
//! "velocidad 1.0×" ([`MediaCommand::SetSpeed`]) o "volumen 100 %"
//! ([`MediaCommand::SetVolume`]).
//!
//! Persistencia: todo deriva `Serialize`/`Deserialize` y se guarda en
//! RON (los enums de Rust se serializan legibles) — ver
//! `02_ruway/media/CONTROLES.md`.

use serde::{Deserialize, Serialize};

/// Acción semántica de reproducción. Lo que el reproductor sabe hacer,
/// independiente de qué tecla o botón lo dispare. Variantes
/// parametrizadas donde VLC usa constantes fijas.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum MediaCommand {
    /// Alterna play/pausa.
    TogglePause,
    /// Salta `secs` segundos relativos a la posición actual (negativo =
    /// hacia atrás). El paso ya no es una constante: vive en el binding.
    SeekBy { secs: i64 },
    /// Ajusta el volumen sumando `delta` (clampeado por el reproductor).
    VolumeBy { delta: f32 },
    /// Fija el volumen absoluto en `level` (0.0..=1.0).
    SetVolume { level: f32 },
    /// Pista siguiente de la playlist.
    NextTrack,
    /// Pista anterior de la playlist.
    PrevTrack,
    /// Cicla la velocidad de reproducción `dir` pasos por la lista de
    /// `speed_steps` (+1 sube, -1 baja).
    SpeedStep { dir: i32 },
    /// Fija la velocidad absoluta (p.ej. 1.0 para resetear).
    SetSpeed { mult: f32 },
    /// Cicla el modo de repetición (Off/One/All).
    CycleRepeat,
    /// Alterna reproducción aleatoria.
    ToggleShuffle,
    /// Guarda una captura del frame de video actual.
    Snapshot,
    /// Arma/cierra una grabación.
    ToggleRecord,
}

/// Una combinación de teclas normalizada, agnóstica de `winit`. `key`
/// es la forma canónica en `String`: para teclas con nombre el nombre
/// (`"Space"`, `"ArrowLeft"`, `"Enter"`), para caracteres el carácter
/// en minúscula (`"k"`, `"="`, `"]"`). La UI es responsable de producir
/// esta forma desde su evento nativo (ver `chord_from_event` en la app).
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct KeyChord {
    pub key: String,
    #[serde(default)]
    pub ctrl: bool,
    #[serde(default)]
    pub shift: bool,
    #[serde(default)]
    pub alt: bool,
}

impl KeyChord {
    /// Chord de una tecla sin modificadores.
    pub fn key(name: impl Into<String>) -> Self {
        KeyChord {
            key: name.into(),
            ctrl: false,
            shift: false,
            alt: false,
        }
    }

    /// Variante con Shift puesto (constructor de conveniencia para el
    /// mapa por defecto).
    pub fn shift(name: impl Into<String>) -> Self {
        KeyChord {
            key: name.into(),
            ctrl: false,
            shift: true,
            alt: false,
        }
    }
}

/// Asociación tecla → comando. Una entrada del keymap.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Binding {
    pub chord: KeyChord,
    pub command: MediaCommand,
}

/// Tabla de bindings. Resuelve linealmente (la lista es corta y el
/// orden importa: el primer match gana, así un binding de usuario
/// puesto antes puede sombrear uno por defecto).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Keymap {
    pub bindings: Vec<Binding>,
}

impl Keymap {
    /// Devuelve el comando atado a `chord`, si hay alguno.
    pub fn resolve(&self, chord: &KeyChord) -> Option<&MediaCommand> {
        self.bindings
            .iter()
            .find(|b| &b.chord == chord)
            .map(|b| &b.command)
    }
}

/// Configuración completa de controles, serializable a disco. Agrupa
/// los pasos (volumen/seek/velocidad) que antes eran constantes
/// hardcodeadas con el keymap. El [`Default`] arma un mapa inspirado en
/// VLC usando estos mismos pasos.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ControlSettings {
    /// Cuánto sube/baja el volumen cada paso (0.0..=1.0).
    pub volume_step: f32,
    /// Cuántos segundos salta el seek corto.
    pub seek_step_secs: i64,
    /// Multiplicadores de velocidad que cicla `SpeedStep`.
    pub speed_steps: Vec<f32>,
    /// Mapa de teclas → comandos.
    pub keymap: Keymap,
}

impl Default for ControlSettings {
    fn default() -> Self {
        let volume_step = 0.1;
        let seek_step_secs = 5;
        let speed_steps = vec![0.5, 0.75, 1.0, 1.25, 1.5, 2.0];
        let keymap = default_keymap(volume_step, seek_step_secs);
        ControlSettings {
            volume_step,
            seek_step_secs,
            speed_steps,
            keymap,
        }
    }
}

/// Construye el mapa por defecto (inspirado en VLC) con los pasos
/// dados ya horneados en los comandos parametrizados. Documentado en
/// `02_ruway/media/CONTROLES.md`.
pub fn default_keymap(volume_step: f32, seek_step_secs: i64) -> Keymap {
    use MediaCommand::*;
    let b = |chord: KeyChord, command: MediaCommand| Binding { chord, command };
    Keymap {
        bindings: vec![
            b(KeyChord::key("Space"), TogglePause),
            b(KeyChord::key("ArrowRight"), SeekBy { secs: seek_step_secs }),
            b(
                KeyChord::key("ArrowLeft"),
                SeekBy {
                    secs: -seek_step_secs,
                },
            ),
            b(KeyChord::key("ArrowUp"), VolumeBy { delta: volume_step }),
            b(
                KeyChord::key("ArrowDown"),
                VolumeBy {
                    delta: -volume_step,
                },
            ),
            b(KeyChord::key("n"), NextTrack),
            b(KeyChord::key("p"), PrevTrack),
            b(KeyChord::key("l"), CycleRepeat),
            b(KeyChord::key("r"), ToggleShuffle),
            b(KeyChord::key("]"), SpeedStep { dir: 1 }),
            b(KeyChord::key("["), SpeedStep { dir: -1 }),
            b(KeyChord::key("="), SetSpeed { mult: 1.0 }),
            b(KeyChord::key("c"), ToggleRecord),
            b(KeyChord::shift("s"), Snapshot),
        ],
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_resuelve_space_a_toggle_pause() {
        let s = ControlSettings::default();
        assert_eq!(
            s.keymap.resolve(&KeyChord::key("Space")),
            Some(&MediaCommand::TogglePause)
        );
    }

    #[test]
    fn seek_hornea_el_paso_y_el_signo() {
        let s = ControlSettings::default();
        assert_eq!(
            s.keymap.resolve(&KeyChord::key("ArrowRight")),
            Some(&MediaCommand::SeekBy { secs: 5 })
        );
        assert_eq!(
            s.keymap.resolve(&KeyChord::key("ArrowLeft")),
            Some(&MediaCommand::SeekBy { secs: -5 })
        );
    }

    #[test]
    fn snapshot_pide_shift() {
        let s = ControlSettings::default();
        // Sin shift no hay snapshot.
        assert_eq!(s.keymap.resolve(&KeyChord::key("s")), None);
        assert_eq!(
            s.keymap.resolve(&KeyChord::shift("s")),
            Some(&MediaCommand::Snapshot)
        );
    }

    #[test]
    fn tecla_sin_binding_no_resuelve() {
        let s = ControlSettings::default();
        assert_eq!(s.keymap.resolve(&KeyChord::key("z")), None);
    }

    #[test]
    fn primer_match_gana() {
        // Un binding de usuario puesto antes sombrea al default.
        let mut s = ControlSettings::default();
        s.keymap.bindings.insert(
            0,
            Binding {
                chord: KeyChord::key("Space"),
                command: MediaCommand::Snapshot,
            },
        );
        assert_eq!(
            s.keymap.resolve(&KeyChord::key("Space")),
            Some(&MediaCommand::Snapshot)
        );
    }

    #[test]
    fn round_trip_ron() {
        let s = ControlSettings::default();
        let txt = ron::ser::to_string(&s).expect("serializa");
        let back: ControlSettings = ron::from_str(&txt).expect("deserializa");
        assert_eq!(s, back);
    }

    #[test]
    fn los_pasos_propagan_al_keymap() {
        // Pasos custom → el default_keymap los hornea en los comandos.
        let km = default_keymap(0.05, 30);
        assert_eq!(
            km.resolve(&KeyChord::key("ArrowRight")),
            Some(&MediaCommand::SeekBy { secs: 30 })
        );
        assert_eq!(
            km.resolve(&KeyChord::key("ArrowUp")),
            Some(&MediaCommand::VolumeBy { delta: 0.05 })
        );
    }
}
