//! toolbar — barras de controles componibles por el usuario, estilo VLC /
//! launcher (eww): una o varias barras horizontales, cada una con una lista
//! de botones/controles que el usuario **elige y ordena**.
//!
//! Regla #2: el modelo de "qué botones hay y en qué barra/orden van" vive
//! acá, agnóstico de cómo se pintan. La UI mapea cada [`BarItem`] a un
//! botón concreto (un `MediaCommand`) o a un widget especial (timeline,
//! reloj, etiqueta de volumen). El editor de barras de la ventana de
//! configuración manipula esta estructura; la vista del reproductor la
//! recorre para pintar las barras reales.
//!
//! Serializable a RON dentro de `MediaConfig` (`#[serde(default)]`), así
//! agregar items nuevos no rompe una config vieja.

use serde::{Deserialize, Serialize};

/// Un control colocable en una barra. Catálogo **cerrado**: el reproductor
/// sabe pintar exactamente estos. Los hay de dos clases —
/// botones de acción (mapean a un `MediaCommand`) y widgets especiales
/// (timeline, reloj, etiquetas, separador elástico) que la UI trata aparte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub enum BarItem {
    // --- botones de acción ---
    PlayPause,
    Stop,
    Prev,
    Next,
    SeekBack,
    SeekForward,
    VolumeDown,
    VolumeUp,
    Mute,
    Repeat,
    Shuffle,
    SpeedDown,
    SpeedUp,
    SpeedReset,
    Snapshot,
    Record,
    Equalizer,
    Settings,
    // --- widgets especiales ---
    /// Barra de progreso scrubbeable (se estira para llenar el espacio).
    Timeline,
    /// Tiempo actual / duración.
    Clock,
    /// Etiqueta de volumen (porcentaje).
    VolumeLabel,
    /// Slider de volumen con relleno graduable arrastrando el mouse (estilo
    /// VLC): el `−`/`+` quedan a los lados como pasos discretos.
    VolumeSlider,
    /// Título del medio en reproducción.
    Title,
    /// Separador elástico que empuja a los vecinos (alinear a izq/der).
    Spacer,
}

impl BarItem {
    /// Catálogo completo, en orden de presentación para el selector del
    /// editor de barras.
    pub const ALL: &'static [BarItem] = &[
        BarItem::PlayPause,
        BarItem::Stop,
        BarItem::Prev,
        BarItem::Next,
        BarItem::SeekBack,
        BarItem::SeekForward,
        BarItem::VolumeDown,
        BarItem::VolumeUp,
        BarItem::Mute,
        BarItem::Repeat,
        BarItem::Shuffle,
        BarItem::SpeedDown,
        BarItem::SpeedUp,
        BarItem::SpeedReset,
        BarItem::Snapshot,
        BarItem::Record,
        BarItem::Equalizer,
        BarItem::Settings,
        BarItem::Timeline,
        BarItem::Clock,
        BarItem::VolumeLabel,
        BarItem::VolumeSlider,
        BarItem::Title,
        BarItem::Spacer,
    ];

    /// Slug estable (forma en disco / id). Agnóstico de idioma.
    pub fn slug(self) -> &'static str {
        match self {
            BarItem::PlayPause => "play_pause",
            BarItem::Stop => "stop",
            BarItem::Prev => "prev",
            BarItem::Next => "next",
            BarItem::SeekBack => "seek_back",
            BarItem::SeekForward => "seek_forward",
            BarItem::VolumeDown => "volume_down",
            BarItem::VolumeUp => "volume_up",
            BarItem::Mute => "mute",
            BarItem::Repeat => "repeat",
            BarItem::Shuffle => "shuffle",
            BarItem::SpeedDown => "speed_down",
            BarItem::SpeedUp => "speed_up",
            BarItem::SpeedReset => "speed_reset",
            BarItem::Snapshot => "snapshot",
            BarItem::Record => "record",
            BarItem::Equalizer => "equalizer",
            BarItem::Settings => "settings",
            BarItem::Timeline => "timeline",
            BarItem::Clock => "clock",
            BarItem::VolumeLabel => "volume_label",
            BarItem::VolumeSlider => "volume_slider",
            BarItem::Title => "title",
            BarItem::Spacer => "spacer",
        }
    }

    /// Etiqueta humana para el selector del editor (español).
    pub fn label(self) -> &'static str {
        match self {
            BarItem::PlayPause => "Play/Pausa",
            BarItem::Stop => "Detener",
            BarItem::Prev => "Anterior",
            BarItem::Next => "Siguiente",
            BarItem::SeekBack => "Retroceder",
            BarItem::SeekForward => "Avanzar",
            BarItem::VolumeDown => "Volumen −",
            BarItem::VolumeUp => "Volumen +",
            BarItem::Mute => "Silenciar",
            BarItem::Repeat => "Repetición",
            BarItem::Shuffle => "Aleatorio",
            BarItem::SpeedDown => "Velocidad −",
            BarItem::SpeedUp => "Velocidad +",
            BarItem::SpeedReset => "Velocidad 1×",
            BarItem::Snapshot => "Captura",
            BarItem::Record => "Grabar",
            BarItem::Equalizer => "Ecualizador",
            BarItem::Settings => "Configuración",
            BarItem::Timeline => "Línea de tiempo",
            BarItem::Clock => "Reloj",
            BarItem::VolumeLabel => "Etiqueta volumen",
            BarItem::VolumeSlider => "Barra de volumen",
            BarItem::Title => "Título",
            BarItem::Spacer => "Separador",
        }
    }

    /// Si el item se **estira** para llenar el espacio disponible (timeline
    /// y separador). La UI le da `flex_grow`; los demás son de ancho fijo.
    pub fn is_stretch(self) -> bool {
        matches!(self, BarItem::Timeline | BarItem::Spacer)
    }
}

/// Dónde se ancla una barra respecto del video.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum BarPosition {
    /// Encima del video.
    Above,
    /// Debajo del video (default — comportamiento histórico).
    #[default]
    Below,
}

impl BarPosition {
    pub fn toggled(self) -> BarPosition {
        match self {
            BarPosition::Above => BarPosition::Below,
            BarPosition::Below => BarPosition::Above,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            BarPosition::Above => "↑ arriba",
            BarPosition::Below => "↓ abajo",
        }
    }
}

/// Una barra horizontal: lista ordenada de items + dónde se ancla (arriba o
/// abajo del video). `position` es `#[serde(default)]` = `Below`, así una
/// config vieja (sin el campo) se lee como "abajo", el comportamiento previo.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Bar {
    pub items: Vec<BarItem>,
    #[serde(default)]
    pub position: BarPosition,
}

impl Bar {
    pub fn new(items: Vec<BarItem>) -> Self {
        Bar { items, position: BarPosition::Below }
    }

    /// Como [`Self::new`] pero anclando la barra arriba o abajo del video.
    pub fn at(items: Vec<BarItem>, position: BarPosition) -> Self {
        Bar { items, position }
    }
}

/// Configuración de las barras de control: una o varias barras apiladas.
/// La vista las pinta en orden (la barra 0 arriba). El [`Default`] arma un
/// layout tipo VLC: una barra de seek + una barra de transporte/volumen.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Toolbar {
    pub bars: Vec<Bar>,
}

impl Default for Toolbar {
    fn default() -> Self {
        use BarItem::*;
        Toolbar {
            bars: vec![
                // Barra de progreso, arriba (se estira).
                Bar::new(vec![Timeline]),
                // Transporte + reloj + modos + volumen + captura.
                Bar::new(vec![
                    PlayPause, Prev, Next, SeekBack, SeekForward, Spacer, Clock, Spacer,
                    Repeat, Shuffle, SpeedDown, SpeedReset, SpeedUp, Spacer, VolumeDown,
                    VolumeSlider, VolumeUp, VolumeLabel, Equalizer, Snapshot, Record, Settings,
                ]),
            ],
        }
    }
}

impl Toolbar {
    pub fn bar_count(&self) -> usize {
        self.bars.len()
    }

    /// Reconcilia una config cargada: si no quedó ninguna barra, vuelve al
    /// default (una toolbar sin barras no tendría controles). Idempotente.
    pub fn sanitized(mut self) -> Toolbar {
        if self.bars.is_empty() {
            self = Toolbar::default();
        }
        self
    }

    /// Agrega una barra vacía al final.
    pub fn add_bar(&mut self) {
        self.bars.push(Bar::default());
    }

    /// Quita la barra `idx` (no deja la toolbar sin barras: si era la
    /// última, la vacía en vez de borrarla). Devuelve `true` si cambió.
    pub fn remove_bar(&mut self, idx: usize) -> bool {
        if idx >= self.bars.len() {
            return false;
        }
        if self.bars.len() == 1 {
            self.bars[0].items.clear();
        } else {
            self.bars.remove(idx);
        }
        true
    }

    /// Empuja un item al final de la barra `bar`.
    pub fn add_item(&mut self, bar: usize, item: BarItem) {
        if let Some(b) = self.bars.get_mut(bar) {
            b.items.push(item);
        }
    }

    /// Quita el item en `(bar, pos)`. Devuelve el item quitado.
    pub fn remove_item(&mut self, bar: usize, pos: usize) -> Option<BarItem> {
        let b = self.bars.get_mut(bar)?;
        if pos < b.items.len() {
            Some(b.items.remove(pos))
        } else {
            None
        }
    }

    /// Mueve el item de `(bar, pos)` un lugar a la izquierda o derecha
    /// dentro de su barra (`dir` = -1 / +1). Para reordenar sin drag.
    pub fn nudge_item(&mut self, bar: usize, pos: usize, dir: i32) -> bool {
        let Some(b) = self.bars.get_mut(bar) else {
            return false;
        };
        let len = b.items.len();
        if pos >= len {
            return false;
        }
        let target = pos as i64 + dir as i64;
        if target < 0 || target as usize >= len {
            return false;
        }
        b.items.swap(pos, target as usize);
        true
    }

    /// Mueve el item de `(from_bar, from_pos)` al final de `to_bar`. Para
    /// pasar un botón de una barra a otra.
    pub fn move_to_bar(&mut self, from_bar: usize, from_pos: usize, to_bar: usize) -> bool {
        if from_bar >= self.bars.len() || to_bar >= self.bars.len() {
            return false;
        }
        let Some(item) = self.remove_item(from_bar, from_pos) else {
            return false;
        };
        self.bars[to_bar].items.push(item);
        true
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_es_tipo_vlc() {
        let t = Toolbar::default();
        assert_eq!(t.bar_count(), 2);
        assert_eq!(t.bars[0].items, vec![BarItem::Timeline]);
        assert!(t.bars[1].items.contains(&BarItem::PlayPause));
        assert!(t.bars[1].items.contains(&BarItem::VolumeUp));
    }

    #[test]
    fn catalogo_y_slugs_unicos() {
        // Todos los slugs distintos (no hay colisión de id en disco).
        let mut slugs: Vec<&str> = BarItem::ALL.iter().map(|i| i.slug()).collect();
        let total = slugs.len();
        slugs.sort();
        slugs.dedup();
        assert_eq!(slugs.len(), total, "hay slugs duplicados");
    }

    #[test]
    fn add_remove_item() {
        let mut t = Toolbar::default();
        let before = t.bars[1].items.len();
        t.add_item(1, BarItem::Mute);
        assert_eq!(t.bars[1].items.len(), before + 1);
        assert_eq!(t.bars[1].items.last(), Some(&BarItem::Mute));
        let removed = t.remove_item(1, t.bars[1].items.len() - 1);
        assert_eq!(removed, Some(BarItem::Mute));
        assert_eq!(t.bars[1].items.len(), before);
    }

    #[test]
    fn nudge_reordena_en_la_barra() {
        let mut t = Toolbar {
            bars: vec![Bar::new(vec![BarItem::Prev, BarItem::PlayPause, BarItem::Next])],
        };
        // Mover PlayPause (pos 1) a la izquierda.
        assert!(t.nudge_item(0, 1, -1));
        assert_eq!(
            t.bars[0].items,
            vec![BarItem::PlayPause, BarItem::Prev, BarItem::Next]
        );
        // Fuera de rango: no hace nada.
        assert!(!t.nudge_item(0, 0, -1));
        assert!(!t.nudge_item(0, 2, 1));
    }

    #[test]
    fn move_entre_barras() {
        let mut t = Toolbar {
            bars: vec![
                Bar::new(vec![BarItem::PlayPause, BarItem::Stop]),
                Bar::new(vec![BarItem::VolumeUp]),
            ],
        };
        assert!(t.move_to_bar(0, 1, 1)); // Stop → barra 1
        assert_eq!(t.bars[0].items, vec![BarItem::PlayPause]);
        assert_eq!(t.bars[1].items, vec![BarItem::VolumeUp, BarItem::Stop]);
    }

    #[test]
    fn add_remove_bar_nunca_deja_cero() {
        let mut t = Toolbar {
            bars: vec![Bar::new(vec![BarItem::PlayPause])],
        };
        t.add_bar();
        assert_eq!(t.bar_count(), 2);
        // Borrar una deja una.
        assert!(t.remove_bar(1));
        assert_eq!(t.bar_count(), 1);
        // Borrar la última no la elimina: la vacía.
        assert!(t.remove_bar(0));
        assert_eq!(t.bar_count(), 1);
        assert!(t.bars[0].items.is_empty());
    }

    #[test]
    fn sanitized_repuebla_si_vacia() {
        let t = Toolbar { bars: vec![] };
        let s = t.sanitized();
        assert_eq!(s, Toolbar::default());
    }

    #[test]
    fn round_trip_ron() {
        let t = Toolbar::default();
        let txt = ron::ser::to_string(&t).expect("serializa");
        let back: Toolbar = ron::from_str(&txt).expect("deserializa");
        assert_eq!(t, back);
    }

    #[test]
    fn stretch_solo_timeline_y_spacer() {
        assert!(BarItem::Timeline.is_stretch());
        assert!(BarItem::Spacer.is_stretch());
        assert!(!BarItem::PlayPause.is_stretch());
        assert!(!BarItem::Clock.is_stretch());
    }
}
