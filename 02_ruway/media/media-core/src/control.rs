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
    /// Salta a la posición **absoluta** `fraction` (0.0..=1.0) de la
    /// duración total. Es el comando del timeline clickeable: la UI reporta
    /// dónde se clickeó como fracción del ancho de la barra y el reproductor
    /// lo resuelve a tiempo (el core no sabe la duración). Donde VLC scrubea
    /// con el mouse, acá además se puede atar a teclas (un dígito → un %).
    SeekTo { fraction: f32 },
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
    /// Ejecuta el script Rhai nombrado de la biblioteca
    /// [`ControlSettings::scripts`]. Es el escape hatch "más flexible que
    /// VLC": una secuencia o condición sobre la API del reproductor en vez
    /// de una sola acción atómica. El core sólo **nombra** el script —
    /// sigue agnóstico de Rhai (no lo compila ni lo ejecuta); la app lo
    /// resuelve contra su runtime vivo (ver `run_script` en `media-app`).
    Script { name: String },
    /// Enciende/apaga el ecualizador (bypass real, sin costo de procesado).
    EqToggle,
    /// Ajusta la ganancia (dB) de la banda `idx` del EQ sumando `delta_db`
    /// (la app clampea al rango válido). Parametrizado: el mismo comando
    /// sirve para un realce o un corte de cualquier tamaño según el binding.
    EqBandBy { idx: usize, delta_db: f32 },
    /// Aplana el ecualizador (todas las bandas a 0 dB).
    EqReset,
    /// Ajusta el desfase A/V (lipsync) sumando `ms` milisegundos al offset
    /// actual (la app clampea a ±5 s). Positivo retrasa el video respecto
    /// del audio; negativo lo adelanta. Es el `--audio-delay` de mpv/VLC,
    /// reversible y sin tocar el stream de audio (corre la ventana de
    /// presentación de `crate::sync`).
    AvSyncBy { ms: i64 },
    /// Vuelve el desfase A/V a cero.
    AvSyncReset,
    /// Enciende/apaga los ajustes de color del video (bypass real).
    ColorToggle,
    /// Ajusta un parámetro de color del video (V4) sumando `delta` (la app
    /// clampea a su rango). Parametrizado igual que `EqBandBy`: el mismo
    /// comando sube o baja brillo/contraste/gamma/saturación según el binding.
    ColorBy { param: ColorParam, delta: f32 },
    /// Vuelve todos los ajustes de color a la identidad (imagen original).
    ColorReset,
    /// Rota el video 90° (`dir > 0` horario, `dir < 0` antihorario). V3.
    RotateBy { dir: i32 },
    /// Espeja el video horizontalmente (toggle).
    FlipH,
    /// Espeja el video verticalmente (toggle).
    FlipV,
    /// Vuelve a la orientación original (sin rotar ni espejar).
    OrientReset,
    /// Ajusta el delay de subtítulos sumando `ms` (la app clampea). Positivo
    /// **retrasa** el subtítulo (aparece más tarde), negativo lo adelanta.
    /// Es el `--sub-delay` de mpv / el H/G de VLC. S4 de PARIDAD.md.
    SubDelayBy { ms: i64 },
    /// Vuelve el delay de subtítulos a cero.
    SubDelayReset,
    /// Enciende/apaga la etapa de normalización + limitador (A5).
    NormToggle,
    /// Ajusta la ganancia de normalización sumando `db` (la app clampea).
    NormGainBy { db: f32 },
    /// Vuelve la ganancia de normalización a 0 dB (mantiene el limitador).
    NormReset,
    /// Normalización automática: mide la sonoridad integrada (EBU R128) de lo
    /// reproducido hasta ahora y fija la ganancia para llevarla al objetivo
    /// ReplayGain 2.0 (−18 LUFS). Necesita haber reproducido ≳ 1 s.
    NormAuto,
}

/// Qué parámetro de color ajusta [`MediaCommand::ColorBy`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ColorParam {
    Brightness,
    Contrast,
    Gamma,
    Saturation,
    /// Rotación de matiz en grados (`-180..180`).
    Hue,
}

impl ColorParam {
    /// Etiqueta humana para la ayuda / el palette.
    pub fn label(&self) -> &'static str {
        match self {
            ColorParam::Brightness => "brillo",
            ColorParam::Contrast => "contraste",
            ColorParam::Gamma => "gamma",
            ColorParam::Saturation => "saturación",
            ColorParam::Hue => "matiz",
        }
    }
}

impl MediaCommand {
    /// Etiqueta humana de la acción, para overlays de ayuda y docs.
    /// Agnóstica de idioma de UI — el repo trabaja en español.
    pub fn describe(&self) -> String {
        use MediaCommand::*;
        match self {
            TogglePause => "Play / pausa".to_string(),
            SeekBy { secs } if *secs < 0 => format!("Retroceder {}s", secs.abs()),
            SeekBy { secs } => format!("Avanzar {secs}s"),
            SeekTo { fraction } if *fraction <= 0.0 => "Volver al inicio".to_string(),
            SeekTo { fraction } if *fraction >= 1.0 => "Ir al final".to_string(),
            SeekTo { fraction } => format!("Ir al {:.0}%", fraction * 100.0),
            VolumeBy { delta } if *delta < 0.0 => "Bajar volumen".to_string(),
            VolumeBy { .. } => "Subir volumen".to_string(),
            SetVolume { level } => format!("Volumen al {:.0}%", level * 100.0),
            NextTrack => "Pista siguiente".to_string(),
            PrevTrack => "Pista anterior".to_string(),
            SpeedStep { dir } if *dir < 0 => "Velocidad más lenta".to_string(),
            SpeedStep { .. } => "Velocidad más rápida".to_string(),
            SetSpeed { mult } => format!("Velocidad {mult:.2}×"),
            CycleRepeat => "Ciclar repetición".to_string(),
            ToggleShuffle => "Alternar aleatorio".to_string(),
            Snapshot => "Captura de pantalla".to_string(),
            ToggleRecord => "Grabar / detener".to_string(),
            Script { name } => format!("Script «{name}»"),
            EqToggle => "Ecualizador on/off".to_string(),
            EqReset => "Ecualizador plano".to_string(),
            EqBandBy { idx, delta_db } => {
                // Etiqueta por frecuencia ISO de la banda (mismo banco que
                // `crate::eq::Equalizer::graphic_10band`) en vez del índice
                // crudo — más legible en la ayuda y el palette.
                let banda = match crate::eq::ISO_10_BANDS_HZ.get(*idx).copied() {
                    Some(hz) if hz >= 1000.0 => format!("{:.0} kHz", hz / 1000.0),
                    Some(hz) => format!("{hz:.0} Hz"),
                    None => format!("#{idx}"),
                };
                let signo = if *delta_db >= 0.0 { "+" } else { "" };
                format!("EQ {banda} {signo}{delta_db:.0} dB")
            }
            AvSyncBy { ms } if *ms < 0 => {
                format!("Sync A/V −{}ms (adelantar video)", ms.abs())
            }
            AvSyncBy { ms } => format!("Sync A/V +{ms}ms (retrasar video)"),
            AvSyncReset => "Sync A/V a cero".to_string(),
            ColorToggle => "Ajustes de color on/off".to_string(),
            ColorReset => "Color original".to_string(),
            ColorBy { param, delta } => {
                let signo = if *delta >= 0.0 { "+" } else { "" };
                format!("Color {} {signo}{delta:.2}", param.label())
            }
            RotateBy { dir } if *dir < 0 => "Rotar 90° antihorario".to_string(),
            RotateBy { .. } => "Rotar 90° horario".to_string(),
            FlipH => "Espejar horizontal".to_string(),
            FlipV => "Espejar vertical".to_string(),
            OrientReset => "Orientación original".to_string(),
            SubDelayBy { ms } if *ms < 0 => {
                format!("Subtítulo −{}ms (adelantar)", ms.abs())
            }
            SubDelayBy { ms } => format!("Subtítulo +{ms}ms (retrasar)"),
            SubDelayReset => "Subtítulo sin delay".to_string(),
            NormToggle => "Normalización on/off".to_string(),
            NormReset => "Normalización a 0 dB".to_string(),
            NormGainBy { db } if *db < 0.0 => format!("Normalización {db:.0} dB"),
            NormGainBy { db } => format!("Normalización +{db:.0} dB"),
            NormAuto => "Normalizar automático (ReplayGain)".to_string(),
        }
    }
}

/// Un script Rhai con nombre, guardado en la biblioteca de
/// [`ControlSettings`]. El `source` es un snippet sobre la API del
/// reproductor que la app bindea (`toggle_pause()`, `seek(s)`,
/// `set_volume(x)`, `set_speed(x)`, `next_track()`…). Se referencia desde
/// el keymap o el palette por [`MediaCommand::Script`] usando el `name`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct NamedScript {
    pub name: String,
    pub source: String,
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

    /// Forma legible para overlays de ayuda: `"Shift+S"`, `"Espacio"`,
    /// `"→"`. Las flechas y la barra espaciadora se prettyfican; el
    /// resto va en mayúscula. El orden de modificadores es estable
    /// (Ctrl, Alt, Shift) para que la ayuda no baile entre sesiones.
    pub fn display(&self) -> String {
        let key = match self.key.as_str() {
            "ArrowLeft" => "←".to_string(),
            "ArrowRight" => "→".to_string(),
            "ArrowUp" => "↑".to_string(),
            "ArrowDown" => "↓".to_string(),
            "Space" => "Espacio".to_string(),
            other => other.to_uppercase(),
        };
        let mut s = String::new();
        if self.ctrl {
            s.push_str("Ctrl+");
        }
        if self.alt {
            s.push_str("Alt+");
        }
        if self.shift {
            s.push_str("Shift+");
        }
        s.push_str(&key);
        s
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
    /// Biblioteca de scripts Rhai con nombre, referenciables desde el
    /// keymap o el palette por [`MediaCommand::Script`]. `#[serde(default)]`:
    /// un `controles.ron` viejo (sin este campo) sigue cargando con la lista
    /// vacía — agregar scripting nunca rompe una config existente (mismo
    /// criterio que el layout en `media-core::layout`).
    #[serde(default)]
    pub scripts: Vec<NamedScript>,
}

impl ControlSettings {
    /// Devuelve el `source` del script nombrado, si existe en la
    /// biblioteca. Lo usa la app para resolver un [`MediaCommand::Script`]
    /// antes de compilarlo y ejecutarlo.
    pub fn script(&self, name: &str) -> Option<&str> {
        self.scripts
            .iter()
            .find(|s| s.name == name)
            .map(|s| s.source.as_str())
    }
}

impl Default for ControlSettings {
    fn default() -> Self {
        let volume_step = 0.1;
        let seek_step_secs = 5;
        let speed_steps = vec![0.5, 0.75, 1.0, 1.25, 1.5, 2.0];
        let keymap = default_keymap(volume_step, seek_step_secs);
        // Un script de ejemplo: documenta la API disponible en el
        // `controles.ron` sembrado y deja la feature viva de fábrica
        // (atado a `b` por `default_keymap`).
        let scripts = vec![NamedScript {
            name: "potenciar".to_string(),
            source: "set_volume(1.0); set_speed(1.25);".to_string(),
        }];
        ControlSettings {
            volume_step,
            seek_step_secs,
            speed_steps,
            keymap,
            scripts,
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
            b(KeyChord::key("e"), EqToggle),
            // Lipsync: j adelanta el video (audio tarde), k lo retrasa,
            // Shift+J vuelve a cero. Pasos de 50 ms como mpv (Ctrl±).
            b(KeyChord::key("j"), AvSyncBy { ms: -50 }),
            b(KeyChord::key("k"), AvSyncBy { ms: 50 }),
            b(KeyChord::shift("j"), AvSyncReset),
            // Delay de subtítulo: g adelanta, h retrasa, Shift+G a cero
            // (como el G/H de VLC). Pasos de 100 ms.
            b(KeyChord::key("g"), SubDelayBy { ms: -100 }),
            b(KeyChord::key("h"), SubDelayBy { ms: 100 }),
            b(KeyChord::shift("g"), SubDelayReset),
            b(
                KeyChord::key("b"),
                Script {
                    name: "potenciar".to_string(),
                },
            ),
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
    fn display_prettyfica_y_ordena_modificadores() {
        assert_eq!(KeyChord::key("Space").display(), "Espacio");
        assert_eq!(KeyChord::key("ArrowRight").display(), "→");
        assert_eq!(KeyChord::shift("s").display(), "Shift+S");
        assert_eq!(
            KeyChord {
                key: "k".into(),
                ctrl: true,
                shift: true,
                alt: false,
            }
            .display(),
            "Ctrl+Shift+K"
        );
    }

    #[test]
    fn describe_refleja_signo_y_parametros() {
        assert_eq!(MediaCommand::SeekBy { secs: -5 }.describe(), "Retroceder 5s");
        assert_eq!(MediaCommand::SeekBy { secs: 30 }.describe(), "Avanzar 30s");
        assert_eq!(
            MediaCommand::VolumeBy { delta: -0.1 }.describe(),
            "Bajar volumen"
        );
        assert_eq!(
            MediaCommand::SetSpeed { mult: 1.0 }.describe(),
            "Velocidad 1.00×"
        );
    }

    #[test]
    fn describe_de_seek_to_usa_extremos_y_porcentaje() {
        assert_eq!(
            MediaCommand::SeekTo { fraction: 0.0 }.describe(),
            "Volver al inicio"
        );
        assert_eq!(
            MediaCommand::SeekTo { fraction: 1.0 }.describe(),
            "Ir al final"
        );
        assert_eq!(
            MediaCommand::SeekTo { fraction: 0.5 }.describe(),
            "Ir al 50%"
        );
    }

    #[test]
    fn describe_de_script_usa_el_nombre() {
        assert_eq!(
            MediaCommand::Script {
                name: "potenciar".into()
            }
            .describe(),
            "Script «potenciar»"
        );
    }

    #[test]
    fn default_trae_el_script_potenciar_y_lo_bindea_a_b() {
        let s = ControlSettings::default();
        assert!(s.script("potenciar").is_some());
        assert_eq!(s.script("no-existe"), None);
        assert_eq!(
            s.keymap.resolve(&KeyChord::key("b")),
            Some(&MediaCommand::Script {
                name: "potenciar".into()
            })
        );
    }

    #[test]
    fn controles_sin_scripts_carga_con_lista_vacia() {
        // Backward-compat: un RON viejo sin el campo `scripts` debe
        // deserializar con la biblioteca vacía, no fallar.
        let viejo = r#"ControlSettings(
            volume_step: 0.1,
            seek_step_secs: 5,
            speed_steps: [1.0],
            keymap: Keymap(bindings: []),
        )"#;
        let s: ControlSettings = ron::from_str(viejo).expect("deserializa sin scripts");
        assert!(s.scripts.is_empty());
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
