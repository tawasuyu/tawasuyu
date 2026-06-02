//! chapters — marcadores de capítulo y navegación (V7 de `PARIDAD.md`).
//!
//! Un capítulo es un punto con título dentro del medio (la división de un
//! DVD, los segmentos de un video de YouTube). Igual molde que
//! `crate::metadata` y los subtítulos: parser puro + consultas por tiempo,
//! testeable en CI sin tocar un archivo. La fuente típica es el formato
//! **ffmetadata** que escupe `ffprobe`/`ffmpeg` (`[CHAPTER]` con `TIMEBASE`,
//! `START`, `END`, `title`).
//!
//! La app extrae el texto ffmetadata del archivo (vía el puente
//! `foreign-av`) y lo pasa a [`Chapters::parse_ffmetadata`]; el core no
//! sabe de ffmpeg.

use std::time::Duration;

use serde::{Deserialize, Serialize};

/// Un capítulo: dónde empieza y su título.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chapter {
    pub start: Duration,
    pub title: String,
}

/// Lista de capítulos ordenada por tiempo. Consultas para "en qué
/// capítulo estoy" y navegación anterior/siguiente estilo VLC/mpv.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Chapters {
    chapters: Vec<Chapter>,
}

impl Chapters {
    pub fn new(mut chapters: Vec<Chapter>) -> Self {
        chapters.sort_by_key(|c| c.start);
        Chapters { chapters }
    }

    pub fn chapters(&self) -> &[Chapter] {
        &self.chapters
    }

    pub fn len(&self) -> usize {
        self.chapters.len()
    }

    pub fn is_empty(&self) -> bool {
        self.chapters.is_empty()
    }

    /// Índice y referencia del capítulo activo en `t` (el último cuyo
    /// `start <= t`). `None` antes del primer capítulo o si no hay ninguno.
    pub fn at(&self, t: Duration) -> Option<(usize, &Chapter)> {
        if self.chapters.is_empty() {
            return None;
        }
        // El candidato es el último con start <= t.
        let idx = match self.chapters.binary_search_by_key(&t, |c| c.start) {
            Ok(i) => i,
            Err(0) => return None,
            Err(i) => i - 1,
        };
        Some((idx, &self.chapters[idx]))
    }

    /// Primer capítulo que empieza **estrictamente después** de `t` — el
    /// destino de "capítulo siguiente".
    pub fn next(&self, t: Duration) -> Option<&Chapter> {
        self.chapters.iter().find(|c| c.start > t)
    }

    /// Destino de "capítulo anterior", estilo VLC: si ya pasaron más de
    /// `restart_within` desde el inicio del capítulo actual, vuelve a su
    /// inicio (reinicia el capítulo); si recién empezó, salta al anterior.
    /// `None` si no hay a dónde ir (antes del primero / sin capítulos).
    pub fn prev(&self, t: Duration, restart_within: Duration) -> Option<&Chapter> {
        let (idx, cur) = self.at(t)?;
        if t.saturating_sub(cur.start) > restart_within {
            Some(cur) // reinicia el capítulo actual
        } else if idx > 0 {
            Some(&self.chapters[idx - 1])
        } else {
            Some(cur) // ya en el primero: al menos reinícialo
        }
    }

    /// Parsea el formato **ffmetadata** (lo de `ffmpeg -f ffmetadata` /
    /// `ffprobe`). Sólo mira los bloques `[CHAPTER]`: lee `TIMEBASE=num/den`
    /// (default `1/1000`), `START` (en unidades del timebase) y `title`.
    /// Tolerante: ignora bloques sin `START` y todo lo demás (streams,
    /// tags globales). Devuelve los capítulos ordenados.
    pub fn parse_ffmetadata(text: &str) -> Chapters {
        let mut out: Vec<Chapter> = Vec::new();
        let mut in_chapter = false;
        let mut tb_num: u64 = 1;
        let mut tb_den: u64 = 1000;
        let mut start: Option<u64> = None;
        let mut title = String::new();

        // Cierra el bloque en curso volcándolo a `out` si tiene START.
        fn flush(
            out: &mut Vec<Chapter>,
            start: &mut Option<u64>,
            title: &mut String,
            tb_num: u64,
            tb_den: u64,
        ) {
            if let Some(s) = start.take() {
                let den = tb_den.max(1);
                let ms = (s as u128 * tb_num as u128 * 1000 / den as u128) as u64;
                out.push(Chapter {
                    start: Duration::from_millis(ms),
                    title: std::mem::take(title).trim().to_string(),
                });
            } else {
                title.clear();
            }
        }

        for raw in text.lines() {
            let line = raw.trim();
            if line.is_empty() || line.starts_with(';') || line.starts_with('#') {
                continue;
            }
            if line.eq_ignore_ascii_case("[CHAPTER]") {
                if in_chapter {
                    flush(&mut out, &mut start, &mut title, tb_num, tb_den);
                }
                in_chapter = true;
                tb_num = 1;
                tb_den = 1000;
                start = None;
                title.clear();
                continue;
            }
            if line.starts_with('[') {
                // Otra sección (p. ej. [STREAM]): cierra el capítulo en curso.
                if in_chapter {
                    flush(&mut out, &mut start, &mut title, tb_num, tb_den);
                }
                in_chapter = false;
                continue;
            }
            if !in_chapter {
                continue;
            }
            let Some((key, val)) = line.split_once('=') else {
                continue;
            };
            match key.trim().to_ascii_uppercase().as_str() {
                "TIMEBASE" => {
                    if let Some((n, d)) = val.trim().split_once('/') {
                        if let (Ok(n), Ok(d)) = (n.trim().parse(), d.trim().parse()) {
                            tb_num = n;
                            tb_den = d;
                        }
                    }
                }
                "START" => start = val.trim().parse().ok(),
                "TITLE" => title = val.trim().to_string(),
                _ => {}
            }
        }
        if in_chapter {
            flush(&mut out, &mut start, &mut title, tb_num, tb_den);
        }
        Chapters::new(out)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn d(ms: u64) -> Duration {
        Duration::from_millis(ms)
    }

    fn sample() -> Chapters {
        Chapters::new(vec![
            Chapter { start: d(0), title: "Intro".into() },
            Chapter { start: d(60_000), title: "Tema 1".into() },
            Chapter { start: d(120_000), title: "Tema 2".into() },
        ])
    }

    #[test]
    fn at_devuelve_el_capitulo_actual() {
        let c = sample();
        assert_eq!(c.at(d(0)).unwrap().1.title, "Intro");
        assert_eq!(c.at(d(30_000)).unwrap().1.title, "Intro");
        assert_eq!(c.at(d(60_000)).unwrap().1.title, "Tema 1");
        assert_eq!(c.at(d(125_000)).unwrap().1.title, "Tema 2");
        assert_eq!(c.at(d(125_000)).unwrap().0, 2);
    }

    #[test]
    fn next_salta_al_siguiente() {
        let c = sample();
        assert_eq!(c.next(d(0)).unwrap().title, "Tema 1");
        assert_eq!(c.next(d(70_000)).unwrap().title, "Tema 2");
        assert!(c.next(d(130_000)).is_none()); // ya en el último
    }

    #[test]
    fn prev_reinicia_o_retrocede() {
        let c = sample();
        let within = d(3_000);
        // Bien dentro de "Tema 1" → reinicia "Tema 1".
        assert_eq!(c.prev(d(90_000), within).unwrap().title, "Tema 1");
        // Recién entrado a "Tema 1" (a 1 s) → retrocede a "Intro".
        assert_eq!(c.prev(d(61_000), within).unwrap().title, "Intro");
        // En el primero, recién empezado → lo reinicia.
        assert_eq!(c.prev(d(500), within).unwrap().title, "Intro");
    }

    #[test]
    fn parse_ffmetadata_basico() {
        let text = ";FFMETADATA1\n\
            title=Mi Video\n\
            [CHAPTER]\n\
            TIMEBASE=1/1000\n\
            START=0\n\
            END=60000\n\
            title=Intro\n\
            [CHAPTER]\n\
            TIMEBASE=1/1000\n\
            START=60000\n\
            END=120000\n\
            title=Tema 1\n";
        let c = Chapters::parse_ffmetadata(text);
        assert_eq!(c.len(), 2);
        assert_eq!(c.chapters()[0].title, "Intro");
        assert_eq!(c.chapters()[0].start, d(0));
        assert_eq!(c.chapters()[1].title, "Tema 1");
        assert_eq!(c.chapters()[1].start, d(60_000));
    }

    #[test]
    fn parse_timebase_no_milisegundos() {
        // TIMEBASE en segundos: START en unidades de 1/1 s.
        let text = "[CHAPTER]\nTIMEBASE=1/1\nSTART=90\ntitle=Min uno y medio\n";
        let c = Chapters::parse_ffmetadata(text);
        assert_eq!(c.len(), 1);
        assert_eq!(c.chapters()[0].start, d(90_000));
    }

    #[test]
    fn parse_ignora_bloques_sin_start_y_otras_secciones() {
        let text = "[CHAPTER]\nTIMEBASE=1/1000\ntitle=sin start\n\
            [STREAM]\nSTART=999\n\
            [CHAPTER]\nSTART=5000\ntitle=válido\n";
        let c = Chapters::parse_ffmetadata(text);
        assert_eq!(c.len(), 1);
        assert_eq!(c.chapters()[0].title, "válido");
        assert_eq!(c.chapters()[0].start, d(5_000));
    }

    #[test]
    fn round_trip_ron() {
        let c = sample();
        let txt = ron::ser::to_string(&c).expect("serializa");
        let back: Chapters = ron::from_str(&txt).expect("deserializa");
        assert_eq!(c, back);
    }
}
