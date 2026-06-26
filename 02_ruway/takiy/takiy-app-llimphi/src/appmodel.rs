//! El `Model` de la app: el `EditorState` editable del lib más el estado
//! de runtime del binario (player, sf2, drag, view rect cacheado…).

use llimphi_theme::Theme;
use takiy_app::EditorState;
use takiy_playback::Player;
use takiy_synth::MultiProgramRenderer;

use crate::msg::{AutoHit, DockItem, DragState};

/// Pantalla activa de la app. El proyecto se abre en el **panorama** de
/// pistas (tipo Audacity: una lista vertical de carriles horizontales);
/// clickear un carril entra al **editor** de esa pista —el piano roll de
/// siempre— y `Esc` vuelve al panorama.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Screen {
    /// Lista de pistas como carriles (cada uno en midi u onda).
    Overview,
    /// Editor de una pista (piano roll). El índice es la pista abierta;
    /// coincide con `editor.active_track`.
    Track,
}

/// Estado del **modo grabación**: el teclado alfabético toca y graba MIDI
/// en una pista, opcionalmente con las demás pistas sonando de fondo.
pub(crate) struct RecState {
    /// Pista destino de la grabación.
    pub(crate) track: usize,
    /// Origen de tiempo (beat 0 de la toma). El beat actual se deriva del
    /// reloj real: `(now - started_at) * bpm / 60`.
    pub(crate) started_at: std::time::Instant,
    /// BPM congelado al arrancar la toma (define el mapeo tiempo→beat).
    pub(crate) bpm: f32,
    /// Si las demás pistas suenan de fondo mientras se graba.
    pub(crate) backing: bool,
    /// Octava base del mapeo de teclado (fila inferior = esta octava).
    pub(crate) base_octave: i32,
    /// Notas con tecla apretada ahora: `midi → beat de inicio`. Se cierran
    /// (se graban) al soltar la tecla.
    pub(crate) held: std::collections::HashMap<u8, f32>,
    /// Cantidad de notas grabadas en la toma (para la UI).
    pub(crate) count: usize,
    /// Último beat conocido (lo refresca `Tick` para el cabezal/HUD).
    pub(crate) last_beat: f32,
}

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
    /// Fila resaltada por teclado dentro del dropdown del menú principal
    /// (`usize::MAX` = ninguna). La mueven las flechas ↑/↓.
    pub(crate) menu_active: usize,
    /// Animación de aparición/swap del dropdown del menú principal (0→1).
    pub(crate) menu_anim: llimphi_motion::Tween<f32>,
    /// Menú contextual sobre la nota seleccionada: ancla `(x, y)` en
    /// coords de ventana. `None` cerrado. Sólo se abre con right-click
    /// cuando hay una nota seleccionada y el click no cayó sobre un
    /// objeto borrable (nota/dot) — esos siguen borrándose directo.
    pub(crate) context_menu: Option<(f32, f32)>,
    /// Diente activo del rail izquierdo (mixer/instrumento). `None` = el
    /// panel está colapsado y sólo se ve la tira de dientes.
    pub(crate) left_active: Option<DockItem>,
    /// Diente activo del rail derecho (efectos/tonalidad/automación).
    pub(crate) right_active: Option<DockItem>,
    /// Ancho del panel del sidebar izquierdo (px), arrastrable por el
    /// divisor. Clamp en `[160, 480]`.
    pub(crate) left_w: f32,
    /// Ancho del panel del sidebar derecho (px).
    pub(crate) right_w: f32,
    /// Pantalla activa: panorama de pistas o editor de una pista.
    pub(crate) screen: Screen,
    /// Selección de tiempo `[from, to)` en beats sobre la onda de la pista
    /// abierta (editor de onda). `None` = sin selección → las ops aplican
    /// a toda la pista. La fija el drag horizontal sobre la forma de onda.
    pub(crate) wave_sel: Option<(f32, f32)>,
    /// Modo grabación activo (`Some`) o no. Mientras está activo, el
    /// teclado alfabético toca y graba MIDI en `RecState.track`.
    pub(crate) recording: Option<RecState>,
    /// Caché de picos de onda por índice de pista, para el carril en
    /// modo `Onda`. Cada `Vec<f32>` es un perfil normalizado `[0, 1]`
    /// de resolución fija (`ONDA_PEAK_BUCKETS`) que el painter remapea
    /// al ancho real del carril. Se recalcula al entrar al panorama y
    /// al pasar una pista a onda — no en cada frame (el render de audio
    /// es caro). Ver `crate::overview::compute_onda_peaks`.
    pub(crate) onda_peaks: std::collections::HashMap<usize, Vec<f32>>,
}
