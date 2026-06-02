//! Pistas embebidas (S2/A2): modelo agnóstico de las pistas de **audio** y
//! **subtítulo** de un medio multi-stream, más la lógica de **selección y
//! ciclado** entre ellas. La extracción —saber qué streams trae un archivo—
//! la hace el puente `shared/foreign-av` leyendo `ffprobe` (regla #4); acá
//! vive sólo el modelo y la selección (regla #2), 100% testeable sin ffmpeg.
//!
//! El consumidor (la UI) ofrece menús "Pista de audio" / "Subtítulos" y, al
//! cambiar, le pasa al decoder el `index` del stream elegido (lo que ffmpeg
//! mapea con `-map 0:<index>`).

use serde::{Deserialize, Serialize};

/// Tipo de pista seleccionable. El video no se cicla (se asume uno solo), así
/// que sólo modelamos audio y subtítulo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TrackKind {
    Audio,
    Subtitle,
}

/// Una pista embebida del contenedor. `index` es el índice de stream dentro
/// del archivo (el `0:<index>` de ffmpeg), estable para mapear el decoder.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct MediaTrack {
    pub index: u32,
    pub kind: TrackKind,
    pub codec: String,
    /// Código de idioma ISO-639 si el contenedor lo declara (`spa`, `eng`…).
    pub lang: Option<String>,
    /// Título legible si el contenedor lo trae (`"Comentario del director"`).
    pub title: Option<String>,
    /// La pista marcada como predeterminada por el contenedor.
    pub default: bool,
    /// Subtítulo "forzado" (sólo carteles/idiomas extranjeros).
    pub forced: bool,
    /// Canales de audio (`2` estéreo, `6` 5.1…); `None` en subtítulos.
    pub channels: Option<u16>,
}

impl MediaTrack {
    /// Etiqueta legible estilo VLC: `"#3 Español — AC3 5.1 (forzado)"`. Usa lo
    /// que haya: título > idioma > "Pista N", más el códec y, si es audio, la
    /// disposición de canales.
    pub fn label(&self) -> String {
        let mut s = format!("#{}", self.index);
        if let Some(t) = self.title.as_deref().filter(|t| !t.is_empty()) {
            s.push_str(&format!(" {t}"));
        } else if let Some(l) = self.lang.as_deref().filter(|l| !l.is_empty()) {
            s.push_str(&format!(" {}", language_name(l)));
        }
        if !self.codec.is_empty() {
            s.push_str(&format!(" — {}", self.codec.to_ascii_uppercase()));
        }
        if let Some(ch) = self.channels {
            s.push(' ');
            s.push_str(channel_layout(ch));
        }
        if self.forced {
            s.push_str(" (forzado)");
        }
        s
    }
}

/// Nombre amigable de unos pocos códigos ISO-639 comunes; el resto se devuelve
/// tal cual (en mayúscula inicial no — crudo) para no cargar una tabla entera.
fn language_name(code: &str) -> String {
    match code.to_ascii_lowercase().as_str() {
        "spa" | "es" => "Español",
        "eng" | "en" => "Inglés",
        "fra" | "fre" | "fr" => "Francés",
        "deu" | "ger" | "de" => "Alemán",
        "ita" | "it" => "Italiano",
        "por" | "pt" => "Portugués",
        "jpn" | "ja" => "Japonés",
        "rus" | "ru" => "Ruso",
        "zho" | "chi" | "zh" => "Chino",
        "ara" | "ar" => "Árabe",
        "und" => "Desconocido",
        _ => return code.to_string(),
    }
    .to_string()
}

/// Disposición de canales legible (`"2.0"`, `"5.1"`…).
fn channel_layout(ch: u16) -> &'static str {
    match ch {
        1 => "Mono",
        2 => "2.0",
        3 => "2.1",
        6 => "5.1",
        8 => "7.1",
        _ => "multicanal",
    }
}

/// Conjunto de pistas de un medio + el estado de selección actual. Separa
/// audio de subtítulo; el subtítulo admite "apagado" (`None`).
#[derive(Debug, Clone, Default, PartialEq)]
pub struct TrackSet {
    audio: Vec<MediaTrack>,
    subs: Vec<MediaTrack>,
    /// Posición seleccionada dentro de `audio` (si hay alguna).
    audio_sel: Option<usize>,
    /// Posición seleccionada dentro de `subs`; `None` = subtítulos apagados.
    sub_sel: Option<usize>,
}

impl TrackSet {
    /// Construye desde una lista cruda de pistas (en cualquier orden). Reparte
    /// por tipo conservando el orden de aparición y fija la selección inicial:
    /// audio → la pista `default` (o la primera); subtítulo → la `forced`, si
    /// no la `default`, si no **apagado** (lo VLC-like: no imponer subtítulo).
    pub fn from_tracks(tracks: impl IntoIterator<Item = MediaTrack>) -> Self {
        let mut audio = Vec::new();
        let mut subs = Vec::new();
        for t in tracks {
            match t.kind {
                TrackKind::Audio => audio.push(t),
                TrackKind::Subtitle => subs.push(t),
            }
        }
        let audio_sel = if audio.is_empty() {
            None
        } else {
            Some(audio.iter().position(|t| t.default).unwrap_or(0))
        };
        let sub_sel = subs
            .iter()
            .position(|t| t.forced)
            .or_else(|| subs.iter().position(|t| t.default));
        TrackSet { audio, subs, audio_sel, sub_sel }
    }

    pub fn audio_tracks(&self) -> &[MediaTrack] {
        &self.audio
    }

    pub fn subtitle_tracks(&self) -> &[MediaTrack] {
        &self.subs
    }

    /// ¿Hay más de una pista de audio para elegir?
    pub fn has_audio_choice(&self) -> bool {
        self.audio.len() > 1
    }

    /// ¿Hay subtítulos embebidos?
    pub fn has_subtitles(&self) -> bool {
        !self.subs.is_empty()
    }

    pub fn current_audio(&self) -> Option<&MediaTrack> {
        self.audio_sel.and_then(|i| self.audio.get(i))
    }

    pub fn current_subtitle(&self) -> Option<&MediaTrack> {
        self.sub_sel.and_then(|i| self.subs.get(i))
    }

    /// Selecciona la pista de audio por su `index` de stream. `true` si existía.
    pub fn select_audio(&mut self, stream_index: u32) -> bool {
        if let Some(pos) = self.audio.iter().position(|t| t.index == stream_index) {
            self.audio_sel = Some(pos);
            true
        } else {
            false
        }
    }

    /// Selecciona el subtítulo por `index` de stream, o `None` para apagarlo.
    /// `true` si la operación fue válida (apagar siempre lo es).
    pub fn select_subtitle(&mut self, stream_index: Option<u32>) -> bool {
        match stream_index {
            None => {
                self.sub_sel = None;
                true
            }
            Some(idx) => {
                if let Some(pos) = self.subs.iter().position(|t| t.index == idx) {
                    self.sub_sel = Some(pos);
                    true
                } else {
                    false
                }
            }
        }
    }

    /// Avanza a la siguiente pista de audio (envuelve). Devuelve la nueva
    /// seleccionada. No-op si hay 0/1 pistas.
    pub fn cycle_audio(&mut self) -> Option<&MediaTrack> {
        if !self.audio.is_empty() {
            let cur = self.audio_sel.unwrap_or(0);
            self.audio_sel = Some((cur + 1) % self.audio.len());
        }
        self.current_audio()
    }

    /// Cicla el subtítulo estilo VLC (tecla `v`): apagado → sub 0 → sub 1 →
    /// … → último → apagado. No-op si no hay subtítulos.
    pub fn cycle_subtitle(&mut self) {
        if self.subs.is_empty() {
            return;
        }
        self.sub_sel = match self.sub_sel {
            None => Some(0),
            Some(i) if i + 1 < self.subs.len() => Some(i + 1),
            Some(_) => None, // tras el último, apagar
        };
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn audio(index: u32, lang: &str, default: bool, ch: u16) -> MediaTrack {
        MediaTrack {
            index,
            kind: TrackKind::Audio,
            codec: "aac".into(),
            lang: Some(lang.into()),
            title: None,
            default,
            forced: false,
            channels: Some(ch),
        }
    }

    fn sub(index: u32, lang: &str, default: bool, forced: bool) -> MediaTrack {
        MediaTrack {
            index,
            kind: TrackKind::Subtitle,
            codec: "subrip".into(),
            lang: Some(lang.into()),
            title: None,
            default,
            forced,
            channels: None,
        }
    }

    #[test]
    fn reparte_por_tipo_y_default_inicial() {
        let set = TrackSet::from_tracks([
            audio(1, "eng", false, 6),
            audio(2, "spa", true, 2),
            sub(3, "spa", false, false),
        ]);
        assert_eq!(set.audio_tracks().len(), 2);
        assert_eq!(set.subtitle_tracks().len(), 1);
        // audio: arranca en el default (spa, index 2).
        assert_eq!(set.current_audio().unwrap().index, 2);
        // subtítulo: ni forced ni default → apagado.
        assert!(set.current_subtitle().is_none());
        assert!(set.has_audio_choice());
        assert!(set.has_subtitles());
    }

    #[test]
    fn sin_default_arranca_en_la_primera() {
        let set = TrackSet::from_tracks([audio(5, "eng", false, 2), audio(6, "spa", false, 2)]);
        assert_eq!(set.current_audio().unwrap().index, 5);
    }

    #[test]
    fn subtitulo_forced_se_autoselecciona() {
        let set = TrackSet::from_tracks([sub(2, "eng", false, false), sub(3, "spa", false, true)]);
        assert_eq!(set.current_subtitle().unwrap().index, 3);
    }

    #[test]
    fn select_por_index() {
        let mut set = TrackSet::from_tracks([audio(1, "eng", true, 2), audio(2, "spa", false, 6)]);
        assert!(set.select_audio(2));
        assert_eq!(set.current_audio().unwrap().index, 2);
        assert!(!set.select_audio(99)); // inexistente, no cambia
        assert_eq!(set.current_audio().unwrap().index, 2);
    }

    #[test]
    fn cycle_audio_envuelve() {
        let mut set =
            TrackSet::from_tracks([audio(1, "a", true, 2), audio(2, "b", false, 2), audio(3, "c", false, 2)]);
        assert_eq!(set.current_audio().unwrap().index, 1);
        assert_eq!(set.cycle_audio().unwrap().index, 2);
        assert_eq!(set.cycle_audio().unwrap().index, 3);
        assert_eq!(set.cycle_audio().unwrap().index, 1); // envuelve
    }

    #[test]
    fn cycle_subtitle_apaga_al_final() {
        let mut set = TrackSet::from_tracks([sub(1, "eng", false, false), sub(2, "spa", false, false)]);
        assert!(set.current_subtitle().is_none()); // arranca apagado
        set.cycle_subtitle();
        assert_eq!(set.current_subtitle().unwrap().index, 1);
        set.cycle_subtitle();
        assert_eq!(set.current_subtitle().unwrap().index, 2);
        set.cycle_subtitle();
        assert!(set.current_subtitle().is_none()); // tras el último, apaga
    }

    #[test]
    fn select_subtitle_off() {
        let mut set = TrackSet::from_tracks([sub(7, "spa", false, true)]);
        assert!(set.current_subtitle().is_some());
        assert!(set.select_subtitle(None));
        assert!(set.current_subtitle().is_none());
        assert!(set.select_subtitle(Some(7)));
        assert_eq!(set.current_subtitle().unwrap().index, 7);
        assert!(!set.select_subtitle(Some(99))); // inexistente
    }

    #[test]
    fn label_legible() {
        let t = audio(3, "spa", false, 6);
        assert_eq!(t.label(), "#3 Español — AAC 5.1");
        let mut f = sub(4, "eng", false, true);
        f.codec = "subrip".into();
        assert_eq!(f.label(), "#4 Inglés — SUBRIP (forzado)");
        let titled = MediaTrack {
            title: Some("Comentario".into()),
            ..audio(2, "eng", false, 2)
        };
        assert_eq!(titled.label(), "#2 Comentario — AAC 2.0");
    }

    #[test]
    fn vacio_no_rompe() {
        let mut set = TrackSet::default();
        assert!(set.current_audio().is_none());
        assert!(set.cycle_audio().is_none());
        set.cycle_subtitle();
        assert!(set.current_subtitle().is_none());
        assert!(!set.has_audio_choice());
    }
}
