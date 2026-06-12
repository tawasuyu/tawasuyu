//! Parser y modelo de datos de subtítulos.
//!
//! Soporta tres formatos con autodetección:
//! - **SRT** (SubRip Text) — el más extendido.
//! - **WebVTT** — nativo de la web (cabecera `WEBVTT`).
//! - **ASS/SSA** (Advanced SubStation Alpha) — estilo visual completo.
//!
//! Tipos principales:
//! - [`SubtitleTrack`]: pista ordenada con query por timestamp.
//! - [`SubtitleCue`]: entrada individual con rango temporal y texto.
//! - [`SubtitleStyle`] / [`StyleSheet`]: estilos ASS nombrados.
//! - [`SubAlign`] / [`AssColor`]: datos de estilo y alineación.

use std::path::{Path, PathBuf};
use std::time::Duration;

// ============================================================
// SubAlign — alineación de subtítulo estilo ASS/numpad
// ============================================================

/// Alineación de subtítulo estilo **numpad** ASS v4+ (`\an1`..`\an9`): el
/// dígito mapea a las 9 anclas de un teclado numérico — `1` abajo-izquierda,
/// `5` centro, `9` arriba-derecha. Es lo que el renderer usa para posicionar
/// el texto en pantalla (S3). El default ASS es `2` (abajo-centro).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SubAlign {
    BottomLeft,
    BottomCenter,
    BottomRight,
    MiddleLeft,
    MiddleCenter,
    MiddleRight,
    TopLeft,
    TopCenter,
    TopRight,
}

impl Default for SubAlign {
    fn default() -> Self {
        SubAlign::BottomCenter
    }
}

impl SubAlign {
    /// Desde el código numpad ASS v4+ (`\an`, columna `Alignment` de
    /// `[V4+ Styles]`). `1`..`9`; fuera de rango → `None`.
    pub fn from_numpad(n: u8) -> Option<Self> {
        Some(match n {
            1 => SubAlign::BottomLeft,
            2 => SubAlign::BottomCenter,
            3 => SubAlign::BottomRight,
            4 => SubAlign::MiddleLeft,
            5 => SubAlign::MiddleCenter,
            6 => SubAlign::MiddleRight,
            7 => SubAlign::TopLeft,
            8 => SubAlign::TopCenter,
            9 => SubAlign::TopRight,
            _ => return None,
        })
    }

    /// Código numpad (`1`..`9`) — el inverso de [`Self::from_numpad`].
    pub fn numpad(self) -> u8 {
        match self {
            SubAlign::BottomLeft => 1,
            SubAlign::BottomCenter => 2,
            SubAlign::BottomRight => 3,
            SubAlign::MiddleLeft => 4,
            SubAlign::MiddleCenter => 5,
            SubAlign::MiddleRight => 6,
            SubAlign::TopLeft => 7,
            SubAlign::TopCenter => 8,
            SubAlign::TopRight => 9,
        }
    }

    /// Desde el código **legacy SSA v4** (sección `[V4 Styles]` y override
    /// `\a`): horizontal en los bits bajos (`1`=izq, `2`=centro, `3`=der),
    /// `+4` sube a *toptitle*, `+8` a *midtitle*. P. ej. `5`=arriba-izq,
    /// `10`=medio-centro, `11`=medio-der.
    pub fn from_ssa_legacy(n: u8) -> Option<Self> {
        let h = n & 0x3; // 1=izq, 2=centro, 3=der
        if h == 0 {
            return None;
        }
        let row = if n & 0x8 != 0 {
            // middle
            [SubAlign::MiddleLeft, SubAlign::MiddleCenter, SubAlign::MiddleRight]
        } else if n & 0x4 != 0 {
            // top
            [SubAlign::TopLeft, SubAlign::TopCenter, SubAlign::TopRight]
        } else {
            [SubAlign::BottomLeft, SubAlign::BottomCenter, SubAlign::BottomRight]
        };
        Some(row[(h - 1) as usize])
    }
}

// ============================================================
// AssColor — color con opacidad normalizada
// ============================================================

/// Color ASS con opacidad. El formato en archivo es `&HAABBGGRR&` (hex,
/// orden BGR invertido) o un entero decimal BGR (SSA v4 viejo). El byte de
/// alfa de ASS es **transparencia** (`00`=opaco, `FF`=transparente); acá se
/// normaliza a `a` = **opacidad** (`255`=opaco) para que el renderer no tenga
/// que invertirlo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AssColor {
    pub r: u8,
    pub g: u8,
    pub b: u8,
    pub a: u8,
}

impl AssColor {
    pub const WHITE: AssColor = AssColor { r: 255, g: 255, b: 255, a: 255 };
    pub const BLACK: AssColor = AssColor { r: 0, g: 0, b: 0, a: 255 };
}

// ============================================================
// SubtitleStyle / StyleSheet — estilos ASS nombrados
// ============================================================

/// Un estilo nombrado de la sección `[V4+ Styles]`/`[V4 Styles]`: lo que el
/// renderer necesita para pintar un cue que referencia ese estilo. Sólo
/// captura el subconjunto con impacto visual real (fuente, tamaño, colores,
/// negrita/itálica, alineación, márgenes); el resto de columnas ASS
/// (ScaleX/Y, Spacing, Angle, BorderStyle, Outline, Shadow, Encoding…) se
/// ignoran por ahora.
#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleStyle {
    pub name: String,
    pub font: String,
    pub size: f32,
    pub primary: AssColor,
    pub outline: AssColor,
    pub back: AssColor,
    pub bold: bool,
    pub italic: bool,
    pub align: SubAlign,
    pub margin_l: i32,
    pub margin_r: i32,
    pub margin_v: i32,
}

impl Default for SubtitleStyle {
    fn default() -> Self {
        SubtitleStyle {
            name: "Default".into(),
            font: "Arial".into(),
            size: 18.0,
            primary: AssColor::WHITE,
            outline: AssColor::BLACK,
            back: AssColor::BLACK,
            bold: false,
            italic: false,
            align: SubAlign::default(),
            margin_l: 0,
            margin_r: 0,
            margin_v: 0,
        }
    }
}

/// Colección de estilos nombrados de un ASS/SSA. Resolución case-insensitive;
/// `resolve(None)` o un nombre desconocido cae al estilo `Default` si existe.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct StyleSheet {
    pub(crate) styles: Vec<SubtitleStyle>,
}

impl StyleSheet {
    pub fn styles(&self) -> &[SubtitleStyle] {
        &self.styles
    }

    pub fn len(&self) -> usize {
        self.styles.len()
    }

    pub fn is_empty(&self) -> bool {
        self.styles.is_empty()
    }

    /// Estilo por nombre (case-insensitive), sin fallback.
    pub fn get(&self, name: &str) -> Option<&SubtitleStyle> {
        self.styles
            .iter()
            .find(|s| s.name.eq_ignore_ascii_case(name))
    }

    /// Resuelve el estilo aplicable: el nombrado si existe, si no el
    /// `Default`, si no `None`.
    pub fn resolve(&self, name: Option<&str>) -> Option<&SubtitleStyle> {
        if let Some(n) = name {
            if let Some(s) = self.get(n) {
                return Some(s);
            }
        }
        self.get("Default")
    }
}

// ============================================================
// SubtitleCue — entrada individual de subtítulo
// ============================================================

/// Una entrada de subtítulo con su rango temporal y el texto a
/// mostrar mientras dure. `text` puede contener saltos de línea
/// (las líneas múltiples del SRT se preservan con `\n`).
///
/// Los campos `style`/`align`/`pos` sólo los llena el parser ASS/SSA (S3):
/// `style` referencia un [`SubtitleStyle`] del [`StyleSheet`] de la pista,
/// y `align`/`pos` son overrides inline del propio `Dialogue` (`{\an8}`,
/// `{\pos(x,y)}`) que ganan sobre lo que diga el estilo. SRT/WebVTT los dejan
/// en `None`.
#[derive(Debug, Clone, PartialEq)]
pub struct SubtitleCue {
    pub start: Duration,
    pub end: Duration,
    pub text: String,
    /// Nombre del estilo ASS que aplica (columna `Style` del `Dialogue`).
    pub style: Option<String>,
    /// Override de alineación inline (`{\an8}`/`{\a..}`) — gana sobre el estilo.
    pub align: Option<SubAlign>,
    /// Override de posición absoluta inline (`{\pos(x,y)}`) en px de script.
    pub pos: Option<(f32, f32)>,
}

impl SubtitleCue {
    /// Cue de texto plano sin metadatos de estilo (lo que produce SRT/WebVTT).
    pub fn plain(start: Duration, end: Duration, text: String) -> Self {
        SubtitleCue { start, end, text, style: None, align: None, pos: None }
    }
}

// ============================================================
// SubtitleTrack — pista ordenada con query por timestamp
// ============================================================

/// Pista de subtítulos ordenada por tiempo. Querys binarias para
/// resolver "qué cue está activo en t". El consumidor (UI) le pasa
/// la posición actual del audio y recibe el texto a pintar. Para ASS/SSA
/// arrastra además su [`StyleSheet`] (S3): el renderer combina
/// `cue.align`/`cue.style` con [`StyleSheet::resolve`].
#[derive(Debug, Clone, Default)]
pub struct SubtitleTrack {
    cues: Vec<SubtitleCue>,
    styles: StyleSheet,
}

impl SubtitleTrack {
    pub fn new(mut cues: Vec<SubtitleCue>) -> Self {
        cues.sort_by_key(|c| c.start);
        Self { cues, styles: StyleSheet::default() }
    }

    /// Como [`Self::new`] pero adjuntando los estilos parseados (ASS/SSA).
    pub fn with_styles(mut cues: Vec<SubtitleCue>, styles: StyleSheet) -> Self {
        cues.sort_by_key(|c| c.start);
        Self { cues, styles }
    }

    /// Los estilos nombrados de la pista (vacío salvo ASS/SSA).
    pub fn styles(&self) -> &StyleSheet {
        &self.styles
    }

    /// El estilo efectivo de un cue: el que nombra su columna `Style`, con
    /// fallback al `Default` del sheet.
    pub fn style_for(&self, cue: &SubtitleCue) -> Option<&SubtitleStyle> {
        self.styles.resolve(cue.style.as_deref())
    }

    /// La alineación efectiva de un cue: override inline > estilo > default.
    pub fn align_for(&self, cue: &SubtitleCue) -> SubAlign {
        cue.align
            .or_else(|| self.style_for(cue).map(|s| s.align))
            .unwrap_or_default()
    }

    pub fn cues(&self) -> &[SubtitleCue] {
        &self.cues
    }

    pub fn len(&self) -> usize {
        self.cues.len()
    }

    pub fn is_empty(&self) -> bool {
        self.cues.is_empty()
    }

    /// Tope del delay de subtítulos aceptado por [`Self::at_with_delay`]
    /// (±60 s) — el mismo que aplicaba media-app.
    pub const MAX_DELAY_MS: i64 = 60_000;

    /// Como [`Self::at`], pero aplicando un **delay** en ms (positivo
    /// retrasa el subtítulo: al instante `t` se muestra el cue que caería
    /// en `t - delay`). El delay se clampea a ±[`Self::MAX_DELAY_MS`] y la
    /// consulta nunca cae por debajo de 0. Extraído de media-app (S4).
    pub fn at_with_delay(&self, position: Duration, delay_ms: i64) -> Option<&SubtitleCue> {
        let delay = delay_ms.clamp(-Self::MAX_DELAY_MS, Self::MAX_DELAY_MS);
        let q = position.as_millis() as i64 - delay;
        self.at(Duration::from_millis(q.max(0) as u64))
    }

    /// Candidatos de subtítulo "sidecar" de un video: mismo nombre base
    /// con extensión de subtítulo, en orden de preferencia. Puro.
    pub fn sidecar_candidates(video: &Path) -> Vec<PathBuf> {
        ["srt", "vtt", "ass", "ssa"]
            .iter()
            .map(|e| video.with_extension(e))
            .collect()
    }

    /// El primer sidecar que existe en disco junto al video, si hay.
    pub fn find_sidecar(video: &Path) -> Option<PathBuf> {
        Self::sidecar_candidates(video).into_iter().find(|c| c.is_file())
    }

    /// Lee y parsea un archivo de subtítulos (autodetect SRT/WebVTT/ASS
    /// por cabecera). Conveniencia host-side — la única función del crate
    /// que toca el filesystem.
    pub fn load(path: &Path) -> Result<SubtitleTrack, String> {
        let body = std::fs::read_to_string(path).map_err(|e| e.to_string())?;
        SubtitleTrack::parse_subtitles(&body).map_err(|e| e.to_string())
    }

    /// Devuelve el cue activo en `t`, si existe. Si dos cues se
    /// solapan, gana el de `start` más cercano por debajo de `t`
    /// (el último que arrancó).
    pub fn at(&self, t: Duration) -> Option<&SubtitleCue> {
        // Binary search por start; el cue candidato es el último con
        // start <= t. Si su end > t, es el activo.
        if self.cues.is_empty() {
            return None;
        }
        let idx = match self.cues.binary_search_by_key(&t, |c| c.start) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };
        let c = &self.cues[idx];
        if t < c.end {
            Some(c)
        } else {
            None
        }
    }

    /// Parsea un cuerpo SRT. Tolerante: salta entradas malformadas
    /// con un mensaje en el log de errores devuelto. Si el archivo
    /// entero no tiene cues válidos, devuelve `Err`.
    ///
    /// Formato SRT esperado por entrada:
    ///
    /// ```text
    /// 1
    /// 00:00:01,000 --> 00:00:03,500
    /// Línea uno
    /// Línea dos
    ///
    /// 2
    /// ...
    /// ```
    ///
    /// El número de índice se ignora. El separador `,` o `.` para
    /// los milisegundos se acepta indistinto (compat WebVTT mínimo).
    pub fn parse_srt(text: &str) -> Result<Self, String> {
        let mut cues: Vec<SubtitleCue> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        // Normalizamos line endings y partimos por bloques separados
        // por línea vacía.
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        for (i, block) in text.split("\n\n").enumerate() {
            let block = block.trim_matches('\n');
            if block.is_empty() {
                continue;
            }
            let mut lines = block.lines();
            // Primera línea puede ser índice (numérico) o ya la
            // línea de timing si el SRT lo omitió.
            let first = match lines.next() {
                Some(l) => l.trim(),
                None => continue,
            };
            let timing_line = if first.contains("-->") {
                first
            } else {
                match lines.next() {
                    Some(l) => l.trim(),
                    None => {
                        warnings.push(format!("bloque {i}: falta línea de timing"));
                        continue;
                    }
                }
            };
            let (start, end) = match parse_timing_line(timing_line) {
                Ok(t) => t,
                Err(e) => {
                    warnings.push(format!("bloque {i}: timing '{timing_line}' — {e}"));
                    continue;
                }
            };
            let rest: Vec<&str> = lines.collect();
            let text = rest.join("\n").trim().to_string();
            if text.is_empty() {
                continue;
            }
            cues.push(SubtitleCue::plain(start, end, text));
        }
        if cues.is_empty() {
            return Err(format!(
                "ningún cue válido en el SRT (avisos: {})",
                warnings.join(" · ")
            ));
        }
        Ok(Self::new(cues))
    }

    /// Parsea un cuerpo WebVTT — el formato de subtítulos nativo de la
    /// web (par del stack WebM + AV1 + Opus). Tolerante igual que
    /// [`Self::parse_srt`]: salta bloques malformados y devuelve `Err`
    /// sólo si no quedó ningún cue.
    ///
    /// Diferencias con SRT que cubre el parser:
    /// - Cabecera `WEBVTT` (con texto opcional en la misma línea) que
    ///   se descarta, más el BOM `\u{FEFF}` si está presente.
    /// - Bloques `NOTE`, `STYLE` y `REGION` que se ignoran enteros.
    /// - Identificador de cue opcional (línea previa al timing sin
    ///   `-->`) que se descarta.
    /// - Timestamps `MM:SS.mmm` (sin hora) además de `HH:MM:SS.mmm`.
    /// - Ajustes de posición tras el timestamp final
    ///   (`line:0 position:50%`…) que se ignoran.
    /// - Etiquetas en línea (`<b>`, `<i>`, `<c.foo>`, timestamps
    ///   `<00:00:01.000>`) que se eliminan, y entidades HTML comunes
    ///   (`&amp;` `&lt;` `&gt;` `&nbsp;` `&lrm;` `&rlm;`) que se
    ///   decodifican — queda texto plano listo para pintar.
    pub fn parse_webvtt(text: &str) -> Result<Self, String> {
        let mut cues: Vec<SubtitleCue> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();

        // Normalizamos line endings y quitamos el BOM si está.
        let text = text
            .trim_start_matches('\u{FEFF}')
            .replace("\r\n", "\n")
            .replace('\r', "\n");

        for (i, block) in text.split("\n\n").enumerate() {
            let block = block.trim_matches('\n');
            if block.is_empty() {
                continue;
            }
            // La cabecera WEBVTT vive en el primer bloque; el cue (si lo
            // hay pegado a ella) viene tras un \n, así que sólo
            // descartamos esa línea, no el bloque entero.
            let block = if i == 0 && block.starts_with("WEBVTT") {
                match block.split_once('\n') {
                    Some((_, rest)) => rest.trim_matches('\n'),
                    None => continue, // bloque era sólo la cabecera
                }
            } else {
                block
            };
            // Bloques de metadatos que no son cues.
            let head = block.lines().next().unwrap_or("").trim_start();
            if head == "NOTE"
                || head.starts_with("NOTE ")
                || head == "STYLE"
                || head == "REGION"
            {
                continue;
            }

            let mut lines = block.lines();
            let first = match lines.next() {
                Some(l) => l.trim(),
                None => continue,
            };
            // Identificador de cue opcional: si la primera línea no
            // tiene `-->`, es el id y la siguiente es el timing.
            let timing_line = if first.contains("-->") {
                first
            } else {
                match lines.next() {
                    Some(l) => l.trim(),
                    None => {
                        warnings.push(format!("bloque {i}: falta línea de timing"));
                        continue;
                    }
                }
            };
            let (start, end) = match parse_vtt_timing_line(timing_line) {
                Ok(t) => t,
                Err(e) => {
                    warnings.push(format!("bloque {i}: timing '{timing_line}' — {e}"));
                    continue;
                }
            };
            let rest: Vec<&str> = lines.collect();
            let raw = rest.join("\n");
            let text = strip_vtt_markup(&raw).trim().to_string();
            if text.is_empty() {
                continue;
            }
            cues.push(SubtitleCue::plain(start, end, text));
        }
        if cues.is_empty() {
            return Err(format!(
                "ningún cue válido en el WebVTT (avisos: {})",
                warnings.join(" · ")
            ));
        }
        Ok(Self::new(cues))
    }

    /// Parsea un cuerpo ASS/SSA (Advanced SubStation Alpha — el formato de
    /// subtítulos de anime/karaoke, el `libass` de mpv). Extrae **texto +
    /// timing** (igual que SRT/WebVTT) y además, para S3, el **estilo visual**:
    ///
    /// - La sección `[V4+ Styles]`/`[V4 Styles]` → un [`StyleSheet`] con cada
    ///   `Style:` nombrado (fuente, tamaño, colores `&HAABBGGRR`, negrita/
    ///   itálica, alineación numpad o legacy según la versión, márgenes).
    /// - Por cada `Dialogue:`: su columna `Style` queda en `cue.style`, y los
    ///   override tags inline `{\an8}`/`{\a..}` y `{\pos(x,y)}` quedan en
    ///   `cue.align`/`cue.pos` (ganan sobre el estilo). El resto de los
    ///   override tags se siguen descartando del texto (`strip_ass_markup`).
    ///
    /// El renderer combina ambos vía [`Self::style_for`]/[`Self::align_for`].
    /// Lo que aún no se interpreta: karaoke (`\k`), colores inline (`\c`),
    /// transformaciones (`\t`), dibujo vectorial (`\p`).
    ///
    /// Tolerante: saltea `Dialogue`/`Style` malformados (acumula avisos) y los
    /// `Comment:`. Si no hay ningún cue válido devuelve `Err`. Asume — como
    /// todo ASS real — que `Text` es la última columna, así las comas del
    /// diálogo no lo parten.
    pub fn parse_ass(text: &str) -> Result<Self, String> {
        let text = text.replace("\r\n", "\n").replace('\r', "\n");
        let mut cues: Vec<SubtitleCue> = Vec::new();
        let mut warnings: Vec<String> = Vec::new();
        let mut styles: Vec<SubtitleStyle> = Vec::new();

        // Sección actual + bandera de versión legacy (SSA v4: `[V4 Styles]`,
        // alineación con códigos legacy; v4+: `[V4+ Styles]`, numpad).
        #[derive(PartialEq)]
        enum Sec {
            Other,
            Styles,
            Events,
        }
        let mut sec = Sec::Other;
        let mut styles_legacy_align = false;

        // Orden de columnas por defecto de ASS v4+ de `[Events]` (Layer, Start,
        // End, Style, Name, MarginL, MarginR, MarginV, Effect, Text). La línea
        // `Format:` lo sobreescribe si difiere (p. ej. SSA v4 arranca con
        // `Marked` en vez de `Layer`).
        let mut idx_start = 1usize;
        let mut idx_end = 2usize;
        let mut idx_style = 3usize;
        let mut idx_text = 9usize;
        let mut num_cols = 10usize;
        // Orden de columnas de `[V4+ Styles]` (lo fija su propio `Format:`).
        let mut style_fmt: Vec<String> = Vec::new();

        for line in text.lines() {
            let trimmed = line.trim();
            if trimmed.is_empty() {
                continue;
            }
            if trimmed.starts_with('[') {
                let lc = trimmed.to_ascii_lowercase();
                sec = if lc == "[events]" {
                    Sec::Events
                } else if lc == "[v4+ styles]" || lc == "[v4 styles]" {
                    styles_legacy_align = lc == "[v4 styles]";
                    Sec::Styles
                } else {
                    Sec::Other
                };
                continue;
            }
            match sec {
                Sec::Other => continue,
                Sec::Styles => {
                    if let Some(rest) = trimmed.strip_prefix("Format:") {
                        style_fmt = rest
                            .split(',')
                            .map(|c| c.trim().to_ascii_lowercase())
                            .collect();
                        continue;
                    }
                    if let Some(rest) = trimmed.strip_prefix("Style:") {
                        match parse_ass_style(rest, &style_fmt, styles_legacy_align) {
                            Some(s) => styles.push(s),
                            None => warnings.push(format!("Style inválido: '{trimmed}'")),
                        }
                    }
                }
                Sec::Events => {
                    if let Some(rest) = trimmed.strip_prefix("Format:") {
                        let cols: Vec<String> = rest
                            .split(',')
                            .map(|c| c.trim().to_ascii_lowercase())
                            .collect();
                        if !cols.is_empty() {
                            num_cols = cols.len();
                            if let Some(i) = cols.iter().position(|c| c == "start") {
                                idx_start = i;
                            }
                            if let Some(i) = cols.iter().position(|c| c == "end") {
                                idx_end = i;
                            }
                            if let Some(i) = cols.iter().position(|c| c == "style") {
                                idx_style = i;
                            }
                            if let Some(i) = cols.iter().position(|c| c == "text") {
                                idx_text = i;
                            }
                        }
                        continue;
                    }
                    let Some(rest) = trimmed.strip_prefix("Dialogue:") else {
                        // Comment:, Picture:, etc. — se ignoran.
                        continue;
                    };
                    // `Text` es la última columna: partimos en `num_cols`
                    // campos para que el último capture las comas del diálogo.
                    let fields: Vec<&str> = rest.splitn(num_cols, ',').collect();
                    if fields.len() < num_cols || idx_text >= fields.len() {
                        warnings.push(format!("Dialogue con pocos campos: '{trimmed}'"));
                        continue;
                    }
                    let start = match parse_ass_timestamp(fields[idx_start]) {
                        Ok(d) => d,
                        Err(e) => {
                            warnings.push(e);
                            continue;
                        }
                    };
                    let end = match parse_ass_timestamp(fields[idx_end]) {
                        Ok(d) => d,
                        Err(e) => {
                            warnings.push(e);
                            continue;
                        }
                    };
                    let raw_text = fields[idx_text];
                    let (align, pos) = extract_ass_overrides(raw_text);
                    let body = strip_ass_markup(raw_text).trim().to_string();
                    if body.is_empty() {
                        continue;
                    }
                    let style = fields
                        .get(idx_style)
                        .map(|s| s.trim())
                        .filter(|s| !s.is_empty() && *s != "*Default")
                        .map(|s| s.to_string());
                    cues.push(SubtitleCue { start, end, text: body, style, align, pos });
                }
            }
        }

        if cues.is_empty() {
            return Err(format!(
                "ningún Dialogue válido en el ASS/SSA (avisos: {})",
                warnings.join(" · ")
            ));
        }
        Ok(Self::with_styles(cues, StyleSheet { styles }))
    }

    /// Autodetecta SRT vs WebVTT vs ASS/SSA y delega al parser
    /// correspondiente. Lo que usa el consumidor cuando no sabe el formato
    /// de antemano. WebVTT por la cabecera `WEBVTT`; ASS/SSA por su cabecera
    /// de secciones (`[Script Info]`/`[V4...]`/`[Events]`); el resto, SRT.
    pub fn parse_subtitles(text: &str) -> Result<Self, String> {
        let head = text.trim_start_matches('\u{FEFF}').trim_start();
        if head.starts_with("WEBVTT") {
            Self::parse_webvtt(text)
        } else if head.starts_with("[Script Info]")
            || head.starts_with("[V4")
            || head.starts_with("[Events]")
        {
            Self::parse_ass(text)
        } else {
            Self::parse_srt(text)
        }
    }
}

// ============================================================
// Helpers de parsing — funciones privadas
// ============================================================

/// Timing WebVTT: como el de SRT pero el lado derecho puede arrastrar
/// ajustes de posición tras el timestamp (`... --> 00:00:03.000 line:0
/// position:50%`). Tomamos sólo el primer token de cada lado.
fn parse_vtt_timing_line(s: &str) -> Result<(Duration, Duration), String> {
    let parts: Vec<&str> = s.split("-->").map(str::trim).collect();
    if parts.len() != 2 {
        return Err("esperaba 'MM:SS.mmm --> MM:SS.mmm'".into());
    }
    // El primer token whitespace-separado es el timestamp; el resto
    // (settings del cue) se ignora.
    let start_tok = parts[0].split_whitespace().next().unwrap_or(parts[0]);
    let end_tok = parts[1].split_whitespace().next().unwrap_or(parts[1]);
    let start = parse_timestamp(start_tok)?;
    let end = parse_timestamp(end_tok)?;
    Ok((start, end))
}

/// Elimina las etiquetas en línea de WebVTT (`<b>`, `<i>`, `<c.foo>`,
/// timestamps `<00:00:01.000>`, etc.) y decodifica las entidades HTML
/// comunes — deja texto plano para pintar. No es un parser HTML: sólo
/// borra todo lo que está entre `<` y `>`.
fn strip_vtt_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut depth = 0u32;
    for ch in s.chars() {
        match ch {
            '<' => depth += 1,
            '>' => depth = depth.saturating_sub(1),
            _ if depth == 0 => out.push(ch),
            _ => {}
        }
    }
    out.replace("&amp;", "&")
        .replace("&lt;", "<")
        .replace("&gt;", ">")
        .replace("&nbsp;", " ")
        .replace("&lrm;", "")
        .replace("&rlm;", "")
}

/// Borra los override tags de ASS (`{...}`) y convierte los escapes de
/// salto/espacio (`\N`, `\n`, `\h`) — deja texto plano para pintar. No
/// interpreta los tags (color/posición/karaoke); sólo los descarta. El
/// resto de los `\x` (escapes desconocidos) se preservan literales.
fn strip_ass_markup(s: &str) -> String {
    let mut out = String::with_capacity(s.len());
    let mut chars = s.chars().peekable();
    let mut in_brace = false;
    while let Some(c) = chars.next() {
        match c {
            '{' => in_brace = true,
            '}' => in_brace = false,
            _ if in_brace => {}
            '\\' => match chars.peek() {
                // \N salto duro, \n salto blando, \h espacio duro.
                Some('N') | Some('n') => {
                    out.push('\n');
                    chars.next();
                }
                Some('h') => {
                    out.push(' ');
                    chars.next();
                }
                _ => out.push('\\'),
            },
            _ => out.push(c),
        }
    }
    out
}

/// Parsea un color ASS: `&HAABBGGRR&` / `&HBBGGRR` (hex, BGR invertido) o un
/// entero decimal BGR (SSA v4 viejo). El byte de alfa de ASS es transparencia
/// (`00`=opaco); acá se normaliza a `a` = opacidad. Sin alfa → opaco.
fn parse_ass_color(s: &str) -> Option<AssColor> {
    let s = s.trim().trim_end_matches('&');
    let (radix, body) = if let Some(h) =
        s.strip_prefix("&H").or_else(|| s.strip_prefix("&h"))
    {
        (16, h)
    } else if let Some(h) = s.strip_prefix('&') {
        (16, h)
    } else {
        (10, s)
    };
    if body.is_empty() {
        return None;
    }
    let v = u32::from_str_radix(body, radix).ok()?;
    let has_alpha = body.len() > 6;
    let r = (v & 0xff) as u8;
    let g = ((v >> 8) & 0xff) as u8;
    let b = ((v >> 16) & 0xff) as u8;
    let a = if has_alpha {
        255u8.wrapping_sub(((v >> 24) & 0xff) as u8)
    } else {
        255
    };
    Some(AssColor { r, g, b, a })
}

/// Parsea una línea `Style:` de `[V4+ Styles]`/`[V4 Styles]` contra el orden
/// de columnas de su `Format:` (case-insensitive). Devuelve `None` si falta el
/// nombre o las columnas no alcanzan. Los campos ausentes caen al default.
/// `legacy_align` interpreta `Alignment` como código SSA v4 (vs numpad v4+).
fn parse_ass_style(rest: &str, fmt: &[String], legacy_align: bool) -> Option<SubtitleStyle> {
    if fmt.is_empty() {
        return None;
    }
    // `Style:` no tiene campo con comas libres → split simple, recortando.
    let vals: Vec<&str> = rest.splitn(fmt.len(), ',').map(str::trim).collect();
    let get = |key: &str| -> Option<&str> {
        fmt.iter().position(|c| c == key).and_then(|i| vals.get(i).copied())
    };
    let name = get("name").filter(|s| !s.is_empty())?.to_string();
    let mut st = SubtitleStyle { name, ..SubtitleStyle::default() };
    if let Some(f) = get("fontname").filter(|s| !s.is_empty()) {
        st.font = f.to_string();
    }
    if let Some(sz) = get("fontsize").and_then(|s| s.parse::<f32>().ok()) {
        if sz > 0.0 {
            st.size = sz;
        }
    }
    if let Some(c) = get("primarycolour").and_then(parse_ass_color) {
        st.primary = c;
    }
    if let Some(c) = get("outlinecolour").and_then(parse_ass_color) {
        st.outline = c;
    }
    if let Some(c) = get("backcolour").and_then(parse_ass_color) {
        st.back = c;
    }
    // Bold/Italic ASS: -1 (o cualquier ≠0) = activo, 0 = inactivo.
    if let Some(b) = get("bold").and_then(|s| s.parse::<i32>().ok()) {
        st.bold = b != 0;
    }
    if let Some(it) = get("italic").and_then(|s| s.parse::<i32>().ok()) {
        st.italic = it != 0;
    }
    if let Some(a) = get("alignment").and_then(|s| s.parse::<u8>().ok()) {
        let parsed = if legacy_align {
            SubAlign::from_ssa_legacy(a)
        } else {
            SubAlign::from_numpad(a)
        };
        if let Some(al) = parsed {
            st.align = al;
        }
    }
    if let Some(m) = get("marginl").and_then(|s| s.parse::<i32>().ok()) {
        st.margin_l = m;
    }
    if let Some(m) = get("marginr").and_then(|s| s.parse::<i32>().ok()) {
        st.margin_r = m;
    }
    if let Some(m) = get("marginv").and_then(|s| s.parse::<i32>().ok()) {
        st.margin_v = m;
    }
    Some(st)
}

/// Escanea los bloques `{...}` de un `Text` de `Dialogue` por los override
/// tags posicionales: `\an<d>` (numpad) o `\a<dd>` (legacy SSA) → alineación,
/// y `\pos(x,y)` → posición absoluta. El último de cada tipo gana (ASS aplica
/// de izquierda a derecha). El resto de los tags los descarta
/// [`strip_ass_markup`]. `\a` desnudo (sin `n`) es siempre código legacy SSA.
fn extract_ass_overrides(text: &str) -> (Option<SubAlign>, Option<(f32, f32)>) {
    let mut align = None;
    let mut pos = None;
    let mut rest = text;
    while let Some(open) = rest.find('{') {
        let after = &rest[open + 1..];
        let Some(close) = after.find('}') else { break };
        let block = &after[..close];
        // Tags dentro del bloque, separados por '\'.
        for tag in block.split('\\') {
            let tag = tag.trim();
            if let Some(n) = tag.strip_prefix("an") {
                if let Ok(v) = n.trim().parse::<u8>() {
                    if let Some(a) = SubAlign::from_numpad(v) {
                        align = Some(a);
                    }
                }
            } else if let Some(n) = tag.strip_prefix('a') {
                // `\a<dd>` legacy. (`\an` ya lo capturó la rama de arriba.)
                if let Ok(v) = n.trim().parse::<u8>() {
                    if let Some(a) = SubAlign::from_ssa_legacy(v) {
                        align = Some(a);
                    }
                }
            } else if let Some(args) = tag.strip_prefix("pos(") {
                let args = args.trim_end_matches(')');
                let mut it = args.split(',').map(|x| x.trim().parse::<f32>());
                if let (Some(Ok(x)), Some(Ok(y))) = (it.next(), it.next()) {
                    pos = Some((x, y));
                }
            }
        }
        rest = &after[close + 1..];
    }
    (align, pos)
}

/// Timestamp ASS/SSA: `H:MM:SS.cc`, donde la fracción son **centésimas**
/// (no milésimas como SRT) — por eso no reusa [`parse_timestamp`]. La
/// fracción se escala genéricamente por su cantidad de dígitos, así
/// `.5`/`.50`/`.500` valen todos 500 ms. La hora puede ser un solo dígito.
fn parse_ass_timestamp(s: &str) -> Result<Duration, String> {
    let s = s.trim();
    let (hms, frac) = s.rsplit_once('.').unwrap_or((s, ""));
    let parts: Vec<&str> = hms.split(':').collect();
    let (h, m, sec) = match parts.as_slice() {
        [hh, mm, ss] => (
            hh.parse::<u64>().map_err(|_| format!("hora inválida en '{s}'"))?,
            mm.parse::<u64>().map_err(|_| format!("minuto inválido en '{s}'"))?,
            ss.parse::<u64>().map_err(|_| format!("segundo inválido en '{s}'"))?,
        ),
        [mm, ss] => (
            0,
            mm.parse::<u64>().map_err(|_| format!("minuto inválido en '{s}'"))?,
            ss.parse::<u64>().map_err(|_| format!("segundo inválido en '{s}'"))?,
        ),
        _ => return Err(format!("timestamp ASS inválido '{s}'")),
    };
    // Fracción → ms escalando por la cantidad de dígitos (centésimas = 2).
    let frac_ms = if frac.is_empty() {
        0
    } else {
        let frac_int: u64 = frac
            .parse()
            .map_err(|_| format!("fracción inválida en '{s}'"))?;
        let denom = 10u64.pow(frac.len().min(9) as u32);
        frac_int * 1000 / denom
    };
    let total_ms = ((h * 3600) + (m * 60) + sec) * 1000 + frac_ms;
    Ok(Duration::from_millis(total_ms))
}

fn parse_timing_line(s: &str) -> Result<(Duration, Duration), String> {
    let parts: Vec<&str> = s.split("-->").map(str::trim).collect();
    if parts.len() != 2 {
        return Err("esperaba 'HH:MM:SS,mmm --> HH:MM:SS,mmm'".into());
    }
    let start = parse_timestamp(parts[0])?;
    let end = parse_timestamp(parts[1])?;
    Ok((start, end))
}

fn parse_timestamp(s: &str) -> Result<Duration, String> {
    // Acepta HH:MM:SS,mmm o HH:MM:SS.mmm (SRT) y MM:SS.mmm (WebVTT
    // omite la hora cuando es 0). Trim para tolerar espacios.
    let s = s.trim();
    let (hms, ms_part) = match s.rsplit_once(',').or_else(|| s.rsplit_once('.')) {
        Some(p) => p,
        None => (s, "0"),
    };
    let hms_parts: Vec<&str> = hms.split(':').collect();
    // 3 partes = HH:MM:SS ; 2 partes = MM:SS (la hora es implícita 0).
    let (h, m, sec) = match hms_parts.as_slice() {
        [hh, mm, ss] => (
            hh.parse::<u64>().map_err(|_| format!("hora inválida en '{s}'"))?,
            mm.parse::<u64>().map_err(|_| format!("minuto inválido en '{s}'"))?,
            ss.parse::<u64>().map_err(|_| format!("segundo inválido en '{s}'"))?,
        ),
        [mm, ss] => (
            0,
            mm.parse::<u64>().map_err(|_| format!("minuto inválido en '{s}'"))?,
            ss.parse::<u64>().map_err(|_| format!("segundo inválido en '{s}'"))?,
        ),
        _ => return Err(format!("timestamp inválido '{s}'")),
    };
    let ms: u64 = ms_part
        .parse()
        .map_err(|_| format!("ms inválidos en '{s}'"))?;
    let total_ms = ((h * 3600) + (m * 60) + sec) * 1000 + ms;
    Ok(Duration::from_millis(total_ms))
}

// ============================================================
// Tests
// ============================================================

#[cfg(test)]
mod tests_subtitles {
    use super::*;

    #[test]
    fn sidecar_candidates_orden_y_sin_extension() {
        let c = SubtitleTrack::sidecar_candidates(std::path::Path::new("/cine/peli.mp4"));
        assert_eq!(c[0], std::path::PathBuf::from("/cine/peli.srt"));
        assert_eq!(c[1], std::path::PathBuf::from("/cine/peli.vtt"));
        assert_eq!(c[2], std::path::PathBuf::from("/cine/peli.ass"));
        assert_eq!(c[3], std::path::PathBuf::from("/cine/peli.ssa"));
        assert_eq!(
            SubtitleTrack::sidecar_candidates(std::path::Path::new("clip"))[0],
            std::path::PathBuf::from("clip.srt")
        );
    }

    #[test]
    fn at_with_delay_corre_la_consulta() {
        let src = "1\n00:00:05,000 --> 00:00:07,000\nhola\n";
        let track = SubtitleTrack::parse_srt(src).unwrap();
        // Sin delay, a los 4s no hay cue.
        assert!(track.at_with_delay(Duration::from_secs(4), 0).is_none());
        // Delay -2000 (adelanta): a los 4s consultamos 6s → cue activo.
        assert!(track.at_with_delay(Duration::from_secs(4), -2000).is_some());
        // Delay +2000 (retrasa): a los 6s consultamos 4s → nada.
        assert!(track.at_with_delay(Duration::from_secs(6), 2000).is_none());
        // Clamp bajo cero: no panic.
        assert!(track.at_with_delay(Duration::ZERO, 5000).is_none());
        // Delay fuera de rango se clampea a ±60s (no revienta).
        assert!(track.at_with_delay(Duration::from_secs(66), 1_000_000).is_some());
    }

    #[test]
    fn parse_simple_srt() {
        let src = "1\n\
            00:00:01,000 --> 00:00:03,500\n\
            Hola mundo\n\
            \n\
            2\n\
            00:00:04,000 --> 00:00:06,000\n\
            Segunda línea\n";
        let track = SubtitleTrack::parse_srt(src).unwrap();
        assert_eq!(track.len(), 2);
        assert_eq!(track.cues()[0].text, "Hola mundo");
        assert_eq!(track.cues()[0].start, Duration::from_millis(1000));
        assert_eq!(track.cues()[0].end, Duration::from_millis(3500));
    }

    #[test]
    fn query_active_cue() {
        let src = "1\n\
            00:00:01,000 --> 00:00:03,000\n\
            uno\n\
            \n\
            2\n\
            00:00:05,000 --> 00:00:07,000\n\
            dos\n";
        let track = SubtitleTrack::parse_srt(src).unwrap();
        assert!(track.at(Duration::from_millis(500)).is_none());
        assert_eq!(track.at(Duration::from_millis(2000)).unwrap().text, "uno");
        // Entre cues: gap, sin activo.
        assert!(track.at(Duration::from_millis(4000)).is_none());
        assert_eq!(track.at(Duration::from_millis(6500)).unwrap().text, "dos");
    }

    #[test]
    fn multiline_text_preserved() {
        let src = "1\n\
            00:00:01,000 --> 00:00:02,000\n\
            primera\n\
            segunda\n";
        let track = SubtitleTrack::parse_srt(src).unwrap();
        assert_eq!(track.cues()[0].text, "primera\nsegunda");
    }

    #[test]
    fn dot_separator_accepted() {
        let src = "1\n00:00:01.500 --> 00:00:03.250\nhola\n";
        let track = SubtitleTrack::parse_srt(src).unwrap();
        assert_eq!(track.cues()[0].start, Duration::from_millis(1500));
        assert_eq!(track.cues()[0].end, Duration::from_millis(3250));
    }

    #[test]
    fn empty_srt_fails() {
        let err = SubtitleTrack::parse_srt("").unwrap_err();
        assert!(err.contains("cue"));
    }

    #[test]
    fn malformed_block_skipped() {
        let src = "1\n\
            no-es-timing\n\
            texto\n\
            \n\
            2\n\
            00:00:01,000 --> 00:00:02,000\n\
            válido\n";
        let track = SubtitleTrack::parse_srt(src).unwrap();
        // Sólo el segundo bloque entra.
        assert_eq!(track.len(), 1);
        assert_eq!(track.cues()[0].text, "válido");
    }

    // --- WebVTT ---

    #[test]
    fn parse_simple_webvtt() {
        let src = "WEBVTT\n\
            \n\
            00:00:01.000 --> 00:00:03.500\n\
            Hola mundo\n\
            \n\
            00:00:04.000 --> 00:00:06.000\n\
            Segunda línea\n";
        let track = SubtitleTrack::parse_webvtt(src).unwrap();
        assert_eq!(track.len(), 2);
        assert_eq!(track.cues()[0].text, "Hola mundo");
        assert_eq!(track.cues()[0].start, Duration::from_millis(1000));
        assert_eq!(track.cues()[0].end, Duration::from_millis(3500));
    }

    #[test]
    fn webvtt_mm_ss_timestamp() {
        // WebVTT permite omitir la hora cuando es 0.
        let src = "WEBVTT\n\n01:02.500 --> 01:05.000\nbreve\n";
        let track = SubtitleTrack::parse_webvtt(src).unwrap();
        assert_eq!(track.cues()[0].start, Duration::from_millis(62_500));
        assert_eq!(track.cues()[0].end, Duration::from_millis(65_000));
    }

    #[test]
    fn webvtt_cue_id_and_settings_ignored() {
        let src = "WEBVTT\n\
            \n\
            intro\n\
            00:00:01.000 --> 00:00:03.000 line:0 position:50% align:start\n\
            con ajustes\n";
        let track = SubtitleTrack::parse_webvtt(src).unwrap();
        assert_eq!(track.len(), 1);
        assert_eq!(track.cues()[0].text, "con ajustes");
        assert_eq!(track.cues()[0].end, Duration::from_millis(3000));
    }

    #[test]
    fn webvtt_note_style_region_skipped() {
        let src = "WEBVTT\n\
            \n\
            NOTE este bloque es un comentario\n\
            que ocupa varias líneas\n\
            \n\
            STYLE\n\
            ::cue { color: yellow }\n\
            \n\
            00:00:01.000 --> 00:00:02.000\n\
            sólo este cuenta\n";
        let track = SubtitleTrack::parse_webvtt(src).unwrap();
        assert_eq!(track.len(), 1);
        assert_eq!(track.cues()[0].text, "sólo este cuenta");
    }

    #[test]
    fn webvtt_strips_inline_tags_and_entities() {
        let src = "WEBVTT\n\
            \n\
            00:00:01.000 --> 00:00:02.000\n\
            <c.loud>Hola</c> <b>mundo</b> <00:00:01.500>cruel & feo\n";
        let track = SubtitleTrack::parse_webvtt(src).unwrap();
        assert_eq!(track.cues()[0].text, "Hola mundo cruel & feo");
    }

    #[test]
    fn webvtt_header_with_trailing_text() {
        // La cabecera puede llevar texto y el primer cue venir pegado.
        let src = "WEBVTT - Mi película\n\
            \n\
            00:00:01.000 --> 00:00:02.000\n\
            primero\n";
        let track = SubtitleTrack::parse_webvtt(src).unwrap();
        assert_eq!(track.len(), 1);
        assert_eq!(track.cues()[0].text, "primero");
    }

    #[test]
    fn parse_subtitles_autodetects() {
        let vtt = "WEBVTT\n\n00:00:01.000 --> 00:00:02.000\nvtt\n";
        let srt = "1\n00:00:01,000 --> 00:00:02,000\nsrt\n";
        let ass = "[Script Info]\n[Events]\n\
            Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
            Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,ass\n";
        assert_eq!(SubtitleTrack::parse_subtitles(vtt).unwrap().cues()[0].text, "vtt");
        assert_eq!(SubtitleTrack::parse_subtitles(srt).unwrap().cues()[0].text, "srt");
        assert_eq!(SubtitleTrack::parse_subtitles(ass).unwrap().cues()[0].text, "ass");
    }

    #[test]
    fn empty_webvtt_fails() {
        let err = SubtitleTrack::parse_webvtt("WEBVTT\n").unwrap_err();
        assert!(err.contains("cue"));
    }

    #[test]
    fn parse_simple_ass() {
        // Centésimas, no milésimas: .50 = 500 ms.
        let src = "[Script Info]\n\
            Title: prueba\n\
            \n\
            [V4+ Styles]\n\
            Format: Name, Fontname\n\
            Style: Default,Arial\n\
            \n\
            [Events]\n\
            Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
            Dialogue: 0,0:00:01.00,0:00:03.50,Default,,0,0,0,,Hola mundo\n";
        let track = SubtitleTrack::parse_ass(src).unwrap();
        assert_eq!(track.len(), 1);
        assert_eq!(track.cues()[0].text, "Hola mundo");
        assert_eq!(track.cues()[0].start, Duration::from_millis(1000));
        assert_eq!(track.cues()[0].end, Duration::from_millis(3500));
    }

    #[test]
    fn ass_strips_override_tags_and_breaks() {
        let src = "[Events]\n\
            Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
            Dialogue: 0,0:00:00.00,0:00:02.00,Default,,0,0,0,,{\\an8}{\\i1}Hola{\\i0}\\NMundo cruel\n";
        let track = SubtitleTrack::parse_ass(src).unwrap();
        // Override tags fuera, \\N → salto de línea.
        assert_eq!(track.cues()[0].text, "Hola\nMundo cruel");
    }

    #[test]
    fn ass_text_conserva_las_comas() {
        // El texto es la última columna: sus comas no deben partir el campo.
        let src = "[Events]\n\
            Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
            Dialogue: 0,0:00:01.00,0:00:02.00,Default,,0,0,0,,Uno, dos, tres\n";
        let track = SubtitleTrack::parse_ass(src).unwrap();
        assert_eq!(track.cues()[0].text, "Uno, dos, tres");
    }

    #[test]
    fn ass_ignora_comments_y_lineas_fuera_de_events() {
        let src = "[Script Info]\n\
            ; un comentario de cabecera\n\
            Dialogue: esto NO está en [Events] y se ignora\n\
            \n\
            [Events]\n\
            Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
            Comment: 0,0:00:00.00,0:00:09.00,Default,,0,0,0,,no soy diálogo\n\
            Dialogue: 0,0:00:02.00,0:00:04.00,Default,,0,0,0,,el único de verdad\n";
        let track = SubtitleTrack::parse_ass(src).unwrap();
        assert_eq!(track.len(), 1);
        assert_eq!(track.cues()[0].text, "el único de verdad");
        assert_eq!(track.cues()[0].start, Duration::from_millis(2000));
    }

    #[test]
    fn ass_respeta_orden_de_columnas_no_estandar() {
        // Format con Text antes del final igual se ubica por nombre. Acá Text
        // es la última columna pero el orden de Start/End viene invertido en
        // la lista — se resuelven por nombre, no por posición fija.
        let src = "[Events]\n\
            Format: Start, End, Text\n\
            Dialogue: 0:00:05.00,0:00:06.00,corto\n";
        let track = SubtitleTrack::parse_ass(src).unwrap();
        assert_eq!(track.cues()[0].text, "corto");
        assert_eq!(track.cues()[0].start, Duration::from_millis(5000));
        assert_eq!(track.cues()[0].end, Duration::from_millis(6000));
    }

    #[test]
    fn empty_ass_fails() {
        let err = SubtitleTrack::parse_ass("[Events]\n").unwrap_err();
        assert!(err.contains("Dialogue"));
    }

    // ---- S3: estilo ASS/SSA ----

    #[test]
    fn ass_color_hex_con_y_sin_alfa() {
        // &HAABBGGRR: alfa=transparencia (00=opaco → opacidad 255).
        let c = parse_ass_color("&H00FF8040").unwrap();
        assert_eq!((c.r, c.g, c.b, c.a), (0x40, 0x80, 0xFF, 255));
        // Sin alfa (6 dígitos) → opaco.
        let c = parse_ass_color("&H00FF00").unwrap();
        assert_eq!((c.r, c.g, c.b, c.a), (0x00, 0xFF, 0x00, 255));
        // Alfa FF (transparente) → opacidad 0.
        let c = parse_ass_color("&HFF0000FF").unwrap();
        assert_eq!((c.r, c.g, c.b, c.a), (0xFF, 0x00, 0x00, 0));
        // Decimal BGR (SSA viejo): 255 = rojo (0x0000FF en BGR).
        let c = parse_ass_color("255").unwrap();
        assert_eq!((c.r, c.g, c.b), (255, 0, 0));
        assert!(parse_ass_color("&H").is_none());
    }

    #[test]
    fn subalign_numpad_y_legacy() {
        assert_eq!(SubAlign::from_numpad(8), Some(SubAlign::TopCenter));
        assert_eq!(SubAlign::from_numpad(2), Some(SubAlign::BottomCenter));
        assert_eq!(SubAlign::from_numpad(0), None);
        assert_eq!(SubAlign::TopRight.numpad(), 9);
        // Legacy SSA: 5 = top|left, 10 = mid|center, 11 = mid|right.
        assert_eq!(SubAlign::from_ssa_legacy(5), Some(SubAlign::TopLeft));
        assert_eq!(SubAlign::from_ssa_legacy(10), Some(SubAlign::MiddleCenter));
        assert_eq!(SubAlign::from_ssa_legacy(11), Some(SubAlign::MiddleRight));
        assert_eq!(SubAlign::from_ssa_legacy(2), Some(SubAlign::BottomCenter));
        assert_eq!(SubAlign::from_ssa_legacy(0), None);
    }

    #[test]
    fn ass_parsea_styles_v4plus() {
        let src = "[V4+ Styles]\n\
            Format: Name, Fontname, Fontsize, PrimaryColour, OutlineColour, BackColour, Bold, Italic, Alignment, MarginL, MarginR, MarginV\n\
            Style: Default,Arial,28,&H00FFFFFF,&H00000000,&H00000000,0,0,2,10,10,20\n\
            Style: Titulo,Verdana,48,&H0000FFFF,&H00000000,&H00000000,-1,0,8,0,0,0\n\
            \n\
            [Events]\n\
            Format: Layer, Start, End, Style, Name, MarginL, MarginR, MarginV, Effect, Text\n\
            Dialogue: 0,0:00:01.00,0:00:03.00,Titulo,,0,0,0,,hola\n";
        let track = SubtitleTrack::parse_ass(src).unwrap();
        let sheet = track.styles();
        assert_eq!(sheet.len(), 2);
        let def = sheet.get("default").unwrap();
        assert_eq!(def.font, "Arial");
        assert_eq!(def.size, 28.0);
        assert_eq!(def.align, SubAlign::BottomCenter);
        assert_eq!(def.margin_v, 20);
        let tit = sheet.get("Titulo").unwrap();
        assert!(tit.bold);
        assert_eq!(tit.align, SubAlign::TopCenter);
        assert_eq!(tit.primary, AssColor { r: 255, g: 255, b: 0, a: 255 });
        // El cue referencia el estilo Titulo y lo resuelve.
        let cue = &track.cues()[0];
        assert_eq!(cue.style.as_deref(), Some("Titulo"));
        assert_eq!(track.style_for(cue).unwrap().font, "Verdana");
        assert_eq!(track.align_for(cue), SubAlign::TopCenter);
    }

    #[test]
    fn ass_override_inline_gana_sobre_estilo() {
        let src = "[V4+ Styles]\n\
            Format: Name, Alignment\n\
            Style: Default,2\n\
            [Events]\n\
            Format: Layer, Start, End, Style, Text\n\
            Dialogue: 0,0:00:01.00,0:00:03.00,Default,{\\an8\\b1}arriba\n";
        let track = SubtitleTrack::parse_ass(src).unwrap();
        let cue = &track.cues()[0];
        assert_eq!(cue.text, "arriba"); // tags fuera del texto
        assert_eq!(cue.align, Some(SubAlign::TopCenter));
        // align_for prioriza el override inline sobre el estilo (que es 2).
        assert_eq!(track.align_for(cue), SubAlign::TopCenter);
    }

    #[test]
    fn ass_pos_inline() {
        let src = "[Events]\n\
            Format: Start, End, Text\n\
            Dialogue: 0:00:01.00,0:00:02.00,{\\pos(320.5,100)}centro\n";
        let track = SubtitleTrack::parse_ass(src).unwrap();
        let cue = &track.cues()[0];
        assert_eq!(cue.text, "centro");
        assert_eq!(cue.pos, Some((320.5, 100.0)));
    }

    #[test]
    fn ass_a_legacy_inline() {
        // `\a` desnudo (sin n) usa códigos legacy: 6 = top-center.
        let src = "[Events]\n\
            Format: Start, End, Text\n\
            Dialogue: 0:00:01.00,0:00:02.00,{\\a6}x\n";
        let track = SubtitleTrack::parse_ass(src).unwrap();
        assert_eq!(track.cues()[0].align, Some(SubAlign::TopCenter));
    }

    #[test]
    fn ass_v4_legacy_styles_alignment() {
        // Sección [V4 Styles] (SSA viejo) → alineación legacy.
        let src = "[V4 Styles]\n\
            Format: Name, Alignment\n\
            Style: Default,6\n\
            [Events]\n\
            Format: Start, End, Text\n\
            Dialogue: 0:00:01.00,0:00:02.00,hola\n";
        let track = SubtitleTrack::parse_ass(src).unwrap();
        // 6 legacy = top-center (vs numpad 6 = middle-right).
        assert_eq!(track.styles().get("Default").unwrap().align, SubAlign::TopCenter);
    }

    #[test]
    fn ass_sin_styles_resuelve_a_none_y_default_align() {
        let src = "[Events]\n\
            Format: Start, End, Text\n\
            Dialogue: 0:00:01.00,0:00:02.00,hola\n";
        let track = SubtitleTrack::parse_ass(src).unwrap();
        assert!(track.styles().is_empty());
        let cue = &track.cues()[0];
        assert!(track.style_for(cue).is_none());
        assert_eq!(track.align_for(cue), SubAlign::BottomCenter);
    }

    #[test]
    fn srt_y_vtt_dejan_estilo_vacio() {
        let srt = SubtitleTrack::parse_srt("1\n00:00:01,000 --> 00:00:02,000\nx\n").unwrap();
        assert!(srt.styles().is_empty());
        assert_eq!(srt.cues()[0].style, None);
        assert_eq!(srt.align_for(&srt.cues()[0]), SubAlign::BottomCenter);
    }

    #[test]
    fn stylesheet_resuelve_desconocido_a_default() {
        let src = "[V4+ Styles]\n\
            Format: Name, Fontname\n\
            Style: Default,Arial\n\
            [Events]\n\
            Format: Start, End, Style, Text\n\
            Dialogue: 0:00:01.00,0:00:02.00,NoExiste,hola\n";
        let track = SubtitleTrack::parse_ass(src).unwrap();
        let cue = &track.cues()[0];
        // El estilo nombrado no existe → cae al Default.
        assert_eq!(track.style_for(cue).unwrap().name, "Default");
    }
}
