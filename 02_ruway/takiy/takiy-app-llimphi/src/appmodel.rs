//! El `Model` de la app: el `EditorState` editable del lib más el estado
//! de runtime del binario (player, sf2, drag, view rect cacheado…).

use llimphi_theme::Theme;
use takiy_app::EditorState;
use takiy_playback::Player;
use takiy_synth::MultiProgramRenderer;

use crate::msg::{AutoHit, DragState};

pub(crate) struct Model {
    pub(crate) editor: EditorState,
    pub(crate) source: String,
    pub(crate) theme: Theme,
    pub(crate) player: Option<Player>,
    pub(crate) sf2: Option<MultiProgramRenderer>,
    pub(crate) engine: String,
    pub(crate) playing: bool,
    pub(crate) status: String,
    /// BPM con el que se lanzó el render actual. Se congela en `TogglePlay`:
    /// si cambia el tempo durante la reproducción, el cursor avanza a la
    /// velocidad del render real (no al BPM editado).
    pub(crate) playback_bpm: f32,
    /// Última dimensión conocida del view raíz. La cacheamos del último
    /// `PressAt` para que `DragNote` pueda convertir píxeles a beats sin
    /// que llimphi-ui le pase el rect del nodo en cada fase del drag.
    pub(crate) last_rect: Option<(f32, f32)>,
    /// Drag-to-move en curso. `None` cuando no hay drag activo.
    pub(crate) drag: Option<DragState>,
    /// Hit-test pendiente de un dot de automación: el press lo detecta
    /// y el primer evento `Move` del drag lo consume para arrancar un
    /// `DragMode::Automation`. Se limpia al final del drag o si llega
    /// otro press sin hit.
    pub(crate) auto_pending: Option<AutoHit>,
    /// Offset global del rango MIDI visible (en semitonos). Lo mueve la
    /// rueda del mouse — `pitch_range_with_offset` lo aplica.
    pub(crate) midi_offset: i32,
    /// Último Instant en el que se disparó un blip de audition. Sirve
    /// para throttlear repeticiones rápidas (autorepeat de flechas,
    /// re-selecciones múltiples) y evitar saturar el device.
    pub(crate) last_audition_at: Option<std::time::Instant>,
    /// Barra de menú principal: índice del menú raíz abierto (`None`
    /// cerrado). Lo abre/cierra `menubar_view` vía `Msg::MenuOpen`.
    pub(crate) menu_open: Option<usize>,
    /// Menú contextual sobre la nota seleccionada: ancla `(x, y)` en
    /// coords de ventana. `None` cerrado. Sólo se abre con right-click
    /// cuando hay una nota seleccionada y el click no cayó sobre un
    /// objeto borrable (nota/dot) — esos siguen borrándose directo.
    pub(crate) context_menu: Option<(f32, f32)>,
}
