//! layout — vocabulario agnóstico de los paneles de control de un
//! reproductor y su orden persistible.
//!
//! Hermano de [`crate::control`]: igual que el mapeo de entrada vive en
//! `control` sin saber quién lo pinta, el **orden de los paneles** vive
//! acá sin saber cómo se pinta cada uno. La regla #2 del repo: la lógica
//! de dominio no sabe quién la pinta. La UI traduce cada [`PanelId`] a un
//! tile concreto; este módulo sólo dice *cuáles* paneles hay y *en qué
//! orden*.
//!
//! El layout es un **eje distinto** del mapeo de entrada (ver
//! `02_ruway/media/CONTROLES.md` §D3): por eso no cuelga de
//! [`crate::control::ControlSettings`] sino de su propio
//! [`LayoutSettings`], que la app persiste en un `layout.ron` aparte.
//! Así editar atajos no toca el layout y viceversa.

use serde::{Deserialize, Serialize};

/// Un panel de control del reproductor. Identificador semántico, agnóstico
/// de cómo lo dibuje el frontend. El slug ([`PanelId::slug`]) es estable
/// para que el RON en disco sea legible y diffable.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum PanelId {
    /// Transporte: prev/play-pausa/next + seek corto.
    Transport,
    /// Volumen + medidores peak/RMS.
    Volume,
    /// Ecualizador gráfico de 10 bandas (on/off + flat + gráfico de barras).
    Equalizer,
    /// Playlist: repeat/shuffle/velocidad.
    Playlist,
    /// Captura: grabación WAV + snapshot PNG.
    Recorder,
    /// Visor de forma de onda.
    Waveform,
    /// Visor waterfall (spectrogram histórico).
    Waterfall,
}

impl PanelId {
    /// Todos los paneles en el orden por defecto (agrupados por afinidad:
    /// transporte/volumen/playlist arriba, recorder/visores abajo).
    pub const ALL: &'static [PanelId] = &[
        PanelId::Transport,
        PanelId::Volume,
        PanelId::Equalizer,
        PanelId::Playlist,
        PanelId::Recorder,
        PanelId::Waveform,
        PanelId::Waterfall,
    ];

    /// Slug corto y estable del panel — etiqueta de la title bar del tile
    /// y forma canónica en disco. Agnóstico de idioma de UI.
    pub fn slug(self) -> &'static str {
        match self {
            PanelId::Transport => "transport",
            PanelId::Volume => "volume",
            PanelId::Equalizer => "equalizer",
            PanelId::Playlist => "playlist",
            PanelId::Recorder => "recorder",
            PanelId::Waveform => "waveform",
            PanelId::Waterfall => "waterfall",
        }
    }
}

/// Orden persistible de los paneles. La app lo carga al arrancar, lo
/// permuta con drag-to-swap y lo reescribe a disco en cada cambio.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct LayoutSettings {
    /// Paneles en el orden en que se muestran. El [`Default`] usa
    /// [`PanelId::ALL`].
    pub panels: Vec<PanelId>,
}

impl Default for LayoutSettings {
    fn default() -> Self {
        LayoutSettings {
            panels: PanelId::ALL.to_vec(),
        }
    }
}

impl LayoutSettings {
    /// Reconcilia un orden cargado de disco con el conjunto canónico de
    /// paneles: descarta entradas desconocidas o duplicadas y **anexa**
    /// los paneles que falten (en su orden por defecto). Así un
    /// `layout.ron` viejo —escrito antes de agregar un panel nuevo— no
    /// hace desaparecer el panel: la app lo muestra al final.
    ///
    /// Idempotente: aplicarlo dos veces da el mismo resultado.
    pub fn sanitized(&self) -> LayoutSettings {
        let mut seen: Vec<PanelId> = Vec::new();
        for &p in &self.panels {
            // Sólo paneles conocidos y sin repetir.
            if PanelId::ALL.contains(&p) && !seen.contains(&p) {
                seen.push(p);
            }
        }
        // Anexa los que falten, en orden canónico.
        for &p in PanelId::ALL {
            if !seen.contains(&p) {
                seen.push(p);
            }
        }
        LayoutSettings { panels: seen }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_es_el_orden_canonico() {
        assert_eq!(LayoutSettings::default().panels, PanelId::ALL.to_vec());
    }

    #[test]
    fn round_trip_ron() {
        let l = LayoutSettings {
            panels: vec![
                PanelId::Waterfall,
                PanelId::Transport,
                PanelId::Volume,
            ],
        };
        let txt = ron::ser::to_string(&l).expect("serializa");
        let back: LayoutSettings = ron::from_str(&txt).expect("deserializa");
        assert_eq!(l, back);
    }

    #[test]
    fn sanitized_descarta_duplicados_y_anexa_faltantes() {
        // Orden parcial con un duplicado: queda el primero, se anexan los
        // que faltan en orden canónico.
        let l = LayoutSettings {
            panels: vec![PanelId::Volume, PanelId::Volume, PanelId::Transport],
        };
        let s = l.sanitized();
        assert_eq!(s.panels[0], PanelId::Volume);
        assert_eq!(s.panels[1], PanelId::Transport);
        // El resto (Playlist, Recorder, Waveform, Waterfall) anexado.
        assert_eq!(s.panels.len(), PanelId::ALL.len());
        assert!(s.panels.contains(&PanelId::Waterfall));
    }

    #[test]
    fn sanitized_es_idempotente() {
        let l = LayoutSettings {
            panels: vec![PanelId::Recorder, PanelId::Transport],
        };
        let once = l.sanitized();
        let twice = once.sanitized();
        assert_eq!(once, twice);
    }

    #[test]
    fn sanitized_preserva_un_orden_completo_valido() {
        let l = LayoutSettings {
            panels: vec![
                PanelId::Waterfall,
                PanelId::Waveform,
                PanelId::Recorder,
                PanelId::Playlist,
                PanelId::Equalizer,
                PanelId::Volume,
                PanelId::Transport,
            ],
        };
        assert_eq!(l.sanitized(), l);
    }
}
