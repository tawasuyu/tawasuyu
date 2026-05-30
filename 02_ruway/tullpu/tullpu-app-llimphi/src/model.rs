//! Modelo y mensajes de la app `tullpu`: el `Model` (estado completo),
//! la enum `Msg` (eventos del bucle Elm), las herramientas y estructuras
//! auxiliares de selección/portapapeles, y las constantes de datos.
//!
//! Behavior-preserving split de `main.rs` — sin cambios funcionales.

use std::collections::HashMap;
use std::path::PathBuf;

use llimphi_module_file_picker::{PickerMsg, PickerState};
use llimphi_ui::llimphi_raster::peniko::Image;
use llimphi_ui::{KeyEvent, PaintRect};
use llimphi_widget_text_input::TextInputState;

use pixel_verbo_core::{OpPixel, Proveedor};
use tullpu_core::{Hash, Lienzo, OpLocal};
use tullpu_render::{AlmacenEnMemoria, FormatoExport};
use uuid::Uuid;

pub(crate) struct Model {
    pub(crate) lienzo: Lienzo,
    pub(crate) almacen: AlmacenEnMemoria,
    pub(crate) seleccionada: Option<Uuid>,
    pub(crate) imagen: Option<Image>,
    pub(crate) estado: String,
    pub(crate) proveedor: Box<dyn Proveedor>,
    pub(crate) proveedor_etiqueta: String,
    /// Cache de thumbnails por hash del buffer Rgba8. Una entrada se reusa
    /// mientras el hash siga vivo en alguna capa; tras `regenerar_stale`
    /// hacemos un GC simple sobre los hashes presentes en el lienzo.
    pub(crate) thumbs: HashMap<Hash, Image>,
    /// Raíz desde la que se walkearon los candidatos (CWD al arrancar).
    /// El picker pinta paths relativos a esta raíz.
    pub(crate) raiz: PathBuf,
    /// Lista de archivos imagen detectados bajo `raiz`. Se walkea una vez
    /// al arrancar; reordenar el lienzo no la recalcula.
    pub(crate) imagenes_disponibles: Vec<PathBuf>,
    /// Estado del fuzzy picker. `None` cuando está cerrado.
    pub(crate) picker: Option<PickerState>,
    /// Renombrado in-situ. `Some((uuid, input))` durante la edición —
    /// la fila correspondiente pinta el text-input en vez del botón de
    /// nombre. F2 entra, Enter confirma, Escape cancela.
    pub(crate) renombrando: Option<(Uuid, TextInputState)>,
    /// Pila de snapshots del [`Lienzo`] para undo/redo. Siempre no vacía:
    /// `historial[0]` es el lienzo al inicializar. `cursor_historial` apunta
    /// al estado vigente: `historial[cursor]` siempre cuadra con `lienzo` en
    /// régimen estable. Una mutación trunca cualquier rama de redo (todo lo
    /// que esté después de `cursor`) y pushea el nuevo estado al tope.
    /// Capado a [`HIST_CAP`] entradas para no inflar RAM en sesiones largas.
    pub(crate) historial: Vec<Lienzo>,
    pub(crate) cursor_historial: usize,
    /// Etiqueta del último snapshot pushado. Se usa para *coalescer* mutaciones
    /// continuas: si la próxima mutación viene con la misma etiqueta y
    /// estamos en el tope del historial, en lugar de agregar otra entrada se
    /// sustituye la del tope. Sirve para que un drag del slider de opacidad
    /// (decenas de eventos por segundo) cuente como una sola operación
    /// reversible. Sin coalesce, deshacer un drag costaría 100 Ctrl+Z.
    pub(crate) ultima_etiqueta_snapshot: Option<(Uuid, &'static str)>,
    /// Multiplicador de zoom sobre el fit-contain natural. 1.0 = fit (la
    /// imagen entra entera en el lienzo); 2.0 = el doble del tamaño fit;
    /// 0.5 = la mitad. Clamp en [`ZOOM_MIN`]..=[`ZOOM_MAX`].
    pub(crate) factor_zoom: f32,
    /// Offset de paneo en px de pantalla desde la posición centrada-fit. La
    /// imagen escala alrededor de `(centro_lienzo + pan)` (matemáticamente
    /// invariante bajo cambios de zoom — el píxel medio de la imagen
    /// permanece en el mismo punto al hacer wheel). Sin clamp: se puede
    /// "perder" la imagen, hotkey `0` resetea.
    pub(crate) pan_x: f32,
    pub(crate) pan_y: f32,
    /// Herramienta activa del lienzo. Cambia el cableado de eventos:
    /// `Mover` ⇒ click-drag panea; `Cuentagotas` ⇒ click lee el píxel
    /// bajo el cursor. El wheel zoom-ea en ambos modos.
    pub(crate) herramienta: Herramienta,
    /// Último color leído por el cuentagotas (RGBA del píxel del lienzo
    /// compuesto). `None` hasta que el usuario clickee con la
    /// herramienta `Cuentagotas` activa.
    pub(crate) color_picked: Option<[u8; 4]>,
    /// Histograma RGB del composite vigente — 256 bins por canal.
    /// Se recomputa en `aplicar_y_recomponer` cada vez que cambia el
    /// `model.imagen`. El painter de la sección "histograma" lee esto
    /// directamente (clone del array) en cada frame. `None` cuando
    /// todavía no hay composite.
    pub(crate) histograma: Option<[[u32; 256]; 3]>,
    /// Selección rectangular activa (Photoshop's marquee). En coords
    /// de imagen, no de pantalla — sobrevive a zoom/pan/rotación del
    /// viewport. `None` cuando no hay selección.
    pub(crate) seleccion: Option<RectImagen>,
    /// Estado del drag de selección mientras el usuario sostiene el
    /// click. `None` fuera de un drag. Se commitea a `seleccion` en
    /// el `End` y se limpia.
    pub(crate) seleccion_drag: Option<SeleccionDrag>,
    /// Estado del drag-to-move del contenido de una selección existente.
    /// `None` salvo mientras se arrastra desde adentro del rect. Excluye
    /// mutuamente a `seleccion_drag` (un press entra a uno u otro según
    /// caiga dentro o fuera de la selección vigente).
    pub(crate) mover_drag: Option<MoverDrag>,
    /// Portapapeles interno de píxeles (copy/cut). `None` hasta el
    /// primer Ctrl+C/Ctrl+X. Pegar (Ctrl+V) compone este clip sobre una
    /// capa nueva. Vive fuera del historial — un undo no lo limpia.
    pub(crate) portapapeles: Option<PortaPixeles>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Herramienta {
    /// Click-drag panea el lienzo. Es la herramienta por defecto.
    Mover,
    /// Click sobre el lienzo lee el RGBA del píxel compuesto. No
    /// dragea (el drag se reservaría para una pintura futura).
    Cuentagotas,
    /// Drag sobre el lienzo define un rectángulo de selección (marquee)
    /// en coords de imagen. La selección sirve de ROI para crop /
    /// fill / copy en fases posteriores; esta fase solo dibuja el rect.
    Marco,
    /// Click sobre el lienzo hace flood fill (balde) desde el píxel
    /// clickeado con el color activo, sobre la capa raster seleccionada.
    /// Si hay selección, el relleno queda acotado al rect.
    Balde,
}

impl Herramienta {
    pub(crate) fn etiqueta(self) -> &'static str {
        match self {
            Herramienta::Mover => "mover",
            Herramienta::Cuentagotas => "cuentagotas",
            Herramienta::Marco => "marco",
            Herramienta::Balde => "balde",
        }
    }
}

/// Rectángulo en coordenadas de imagen (half-open `[x0, x1) × [y0, y1)`).
/// Es la selección "marquee" del lienzo — los píxeles dentro caen en el
/// ROI; la convención de coords es la misma que `bbox_no_transparente`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct RectImagen {
    pub(crate) x0: u32,
    pub(crate) y0: u32,
    pub(crate) x1: u32,
    pub(crate) y1: u32,
}

/// Portapapeles interno de píxeles. Guarda un buffer Rgba8 **recortado
/// al rect** (tamaño `w × h`, NO el del lienzo) direccionado por
/// contenido en el almacén, más el origen `(ox, oy)` desde donde se
/// copió. Recortar (en vez de guardar un canvas entero como
/// `extraer_rect_a_buffer`) hace que el clip sobreviva a un crop o un
/// resize posterior del lienzo: al pegar se compone sobre el lienzo
/// vigente en `(ox, oy)` clampeado para que entre. No es parte del DAG
/// del documento — un undo no lo toca.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PortaPixeles {
    pub(crate) w: u32,
    pub(crate) h: u32,
    pub(crate) datos: Hash,
    pub(crate) ox: u32,
    pub(crate) oy: u32,
}

/// Estado intermedio mientras el usuario arrastra una selección. La
/// `ancla_*` es la coord-imagen del punto donde clickeó (puede caer
/// fuera del lienzo — se clampea al normalizar). `cur_l*` es la
/// posición local actual del cursor (en coords del rect del panel
/// lienzo), reconstruida acumulando los `dx, dy` de cada Move sobre
/// el `lx0, ly0` inicial. `rw, rh` se capturan al inicio del drag
/// para que el resize de ventana mid-drag no descoloque la conversión.
#[derive(Debug, Clone, Copy)]
pub(crate) struct SeleccionDrag {
    pub(crate) ancla_ix: i32,
    pub(crate) ancla_iy: i32,
    pub(crate) cur_lx: f32,
    pub(crate) cur_ly: f32,
    pub(crate) rw: f32,
    pub(crate) rh: f32,
}

/// Estado intermedio mientras el usuario arrastra el CONTENIDO de una
/// selección existente (drag-to-move, no construir un marquee nuevo). Se
/// entra cuando el press cae dentro de `model.seleccion` con la
/// herramienta Marco activa. `press_l*` es la posición local del press;
/// `cur_l*` la actual (acumulando los `dx, dy` de cada Move). `aplicado_*`
/// es el offset entero en coords-imagen YA aplicado al contenido — la
/// diferencia con el offset total deseado es el paso a mover en el frame
/// siguiente (el resto sub-píxel queda implícito en `cur - press`).
#[derive(Debug, Clone, Copy)]
pub(crate) struct MoverDrag {
    pub(crate) press_lx: f32,
    pub(crate) press_ly: f32,
    pub(crate) cur_lx: f32,
    pub(crate) cur_ly: f32,
    pub(crate) rw: f32,
    pub(crate) rh: f32,
    pub(crate) aplicado_ix: i32,
    pub(crate) aplicado_iy: i32,
}

/// Multiplicador por tick de wheel. 1.1 ≈ +10%, un escalón cómodo. El
/// factor entra como `factor_zoom *= base.powf(-delta.y)` (delta.y > 0 es
/// scroll hacia abajo en convención CSS → zoom out).
pub(crate) const ZOOM_BASE: f32 = 1.1;
pub(crate) const ZOOM_MIN: f32 = 0.05;
pub(crate) const ZOOM_MAX: f32 = 32.0;

/// Tope de la pila de undo. 64 estados × 32 capas × ~100 B postcard ≈ 200 KB
/// — despreciable. Si se excede, descartamos las entradas más viejas (FIFO).
pub(crate) const HIST_CAP: usize = 64;

#[derive(Clone)]
pub(crate) enum Msg {
    Seleccionar(Uuid),
    ToggleVisible(Uuid),
    BumpOpacidad(Uuid, f32),
    CiclarBlend(Uuid),
    CiclarBlendInverso(Uuid),
    MoverArriba(Uuid),
    MoverAbajo(Uuid),
    Duplicar(Uuid),
    Eliminar(Uuid),
    Agregar(OpLocal),
    AgregarIa(OpPixel),
    Recargar,
    Exportar(FormatoExport),
    Picker(PickerMsg),
    FileDrop(PathBuf),
    IniciarRenombrar(Uuid),
    TeclaRenombrar(KeyEvent),
    ConfirmarRenombrar,
    CancelarRenombrar,
    Undo,
    Redo,
    /// Wheel sobre el lienzo: multiplica `factor_zoom` por `mult` y, si
    /// hay un punto de anclaje conocido, ajusta `pan` para que el punto
    /// quede fijo bajo el cursor (zoom-a-cursor). `(rect, cursor)` son
    /// el último rect del lienzo y la posición global del cursor; ambos
    /// en px de pantalla. `None` ⇒ zoom-alrededor-del-centro.
    Zoom { mult: f32, ancla: Option<(PaintRect, f32, f32)> },
    /// Drag sobre el lienzo: acumula offset de paneo. `dx, dy` en px.
    Pan(f32, f32),
    /// Resetea zoom y pan al estado inicial (fit-contain centrado).
    ResetVista,
    /// Cambia la herramienta activa del lienzo (mover/cuentagotas).
    CambiarHerramienta(Herramienta),
    /// Click sobre el lienzo en modo cuentagotas: `(lx, ly)` relativo al
    /// rect del panel y `(rw, rh)` las dims actuales. El handler
    /// resuelve el píxel-imagen vía `transform_lienzo` y guarda el RGBA.
    RecogerColor {
        lx: f32,
        ly: f32,
        rw: f32,
        rh: f32,
    },
    /// Agrega una capa raster nueva del tamaño del lienzo llena con
    /// `color_picked` (o gris medio si no hay color leído). Cierra el
    /// loop pick→use: pickeás un color con el cuentagotas, después
    /// "+ relleno" aparece como capa nueva encima de la seleccionada.
    AgregarRelleno,
    /// Combina la capa identificada con la que está justo debajo (idx
    /// menor) en una sola capa raster que conserva el composite visual
    /// (respetando blend + opacidad + visibilidad). El par se reemplaza
    /// por una `Capa::raster` con defaults (Normal/1.0/visible). Si la
    /// capa ya está en el fondo (idx 0), no-op + estado descriptivo.
    Combinar(Uuid),
    /// Aplana todas las capas visibles a una sola raster con el
    /// composite del lienzo. Las hidden se preservan en su posición
    /// topológica; el resultado va donde estaba la más alta visible
    /// (Photoshop "Merge Visible"). Sin selección. No-op si hay 0 o 1
    /// visibles.
    AplanarVisibles,
    /// Rota el lienzo entero 90°. `cw=true` ⇒ sentido horario;
    /// `cw=false` ⇒ antihorario. Cada raster gana un buffer nuevo
    /// (rotado), las derivadas quedan stale y se regenan desde la
    /// madre rotada, las dims del lienzo se intercambian.
    RotarLienzo { cw: bool },
    /// Recorta el lienzo al bounding box no-transparente del composite
    /// vigente. Si no hay píxeles opacos, no-op + estado "lienzo
    /// vacío". Si el bbox cubre el lienzo entero, no-op + estado "ya
    /// está justo".
    AutotrimLienzo,
    /// Ajuste in-vivo de un parámetro de la capa derivada `id`. El
    /// slider del panel ops emite `dv` (en unidades del parámetro);
    /// `update` lo suma al valor actual, clamp-ea, marca la capa
    /// stale y propaga al cono. Coalesce por `(id, param.clave())`
    /// para que un drag entero ocupe 1 sola entrada de historial.
    AjustarParametro {
        id: Uuid,
        param: ParametroSlider,
        dv: f32,
    },
    /// Press sobre el lienzo en modo Marco: setea el ancla del drag
    /// en coords-imagen y captura el rect del panel para conversión
    /// estable. Emitido por `on_click_at`.
    IniciarSeleccion {
        lx: f32,
        ly: f32,
        rw: f32,
        rh: f32,
    },
    /// Move durante el drag de Marco: acumula deltas sobre la posición
    /// local actual, recomputa el rect-imagen y refresca `seleccion`
    /// (no espera al End — muestra preview en vivo).
    AjustarSeleccion {
        dx: f32,
        dy: f32,
    },
    /// End del drag de Marco: el `seleccion_drag` queda vacío; el
    /// `seleccion` ya tiene el rect final. Sin snapshot — la selección
    /// no es parte del DAG de imagen.
    FinalizarSeleccion,
    /// Esc o click "limpiar" → borra la selección + cualquier drag en
    /// curso. La barra de espacios libre.
    LimpiarSeleccion,
    /// Recorta el lienzo al rect de `model.seleccion`. No-op si no hay
    /// selección o si el rect cubre el lienzo entero. Limpia la
    /// selección post-crop (la unidad de coords-imagen cambió).
    RecortarASeleccion,
    /// Pone alfa=0 en los píxeles del rect de `model.seleccion` dentro
    /// de la capa raster seleccionada. No-op si no hay selección, no
    /// hay capa seleccionada, la capa es derivada (su buffer es cache,
    /// se sobrescribe en el siguiente recompose), o el rect ya era
    /// todo transparente. Mantiene la selección — un workflow típico
    /// es "marquee + Delete + re-pintar".
    LimpiarSeleccionEnCapa,
    /// Rellena los píxeles del rect de `model.seleccion` con el color
    /// activo (`color_picked`, o `RELLENO_DEFAULT` si no se leyó ninguno)
    /// dentro de la capa raster seleccionada. Mismas precondiciones que
    /// `LimpiarSeleccionEnCapa`; no-op extra si el rect ya tenía ese
    /// color exacto. Mantiene la selección.
    RellenarSeleccionEnCapa,
    /// Copia los píxeles del rect de `model.seleccion` de la capa
    /// seleccionada a una **capa raster nueva** del tamaño del lienzo,
    /// transparente fuera del rect. Inserta la capa encima de la madre
    /// y la selecciona (Photoshop: Ctrl+J "layer via copy"). No-op si
    /// no hay selección/capa, el rect tiene área cero, o el rect era
    /// todo transparente (nada que copiar). No es destructivo: la capa
    /// madre no se toca, así que copia desde cualquier capa (raster o
    /// derivada). Mantiene la selección.
    DuplicarSeleccionACapa,
    /// Copia los píxeles del rect de `model.seleccion` al portapapeles
    /// interno (`model.portapapeles`), recortados al rect. No destructivo
    /// — no toca la capa ni el historial. No-op si no hay selección/capa
    /// o el rect era todo transparente.
    CopiarSeleccion,
    /// Como `CopiarSeleccion` pero además limpia (alfa=0) el rect en la
    /// capa raster seleccionada. Si la capa es derivada, copia pero no
    /// borra. Snapshotea sólo si efectivamente borró.
    CortarSeleccion,
    /// Compone el clip de `model.portapapeles` sobre una capa raster
    /// nueva del tamaño del lienzo vigente, ubicada en su origen original
    /// clampeado para que entre. Inserta encima de la seleccionada y la
    /// selecciona. No-op si el portapapeles está vacío.
    PegarPortapapeles,
    /// Mueve los píxeles del rect de `model.seleccion` por el offset con
    /// signo `(dx, dy)` (en coords-imagen) dentro de la capa raster
    /// seleccionada: levanta el contenido del rect, lo borra de su lugar
    /// y lo recompone (alpha src-over) en el destino, recortando lo que
    /// salga del lienzo. La selección sigue al contenido. No-op si no hay
    /// selección/capa, la capa es derivada, o el movimiento no cambia
    /// nada (delta cero o todo fuera del lienzo). Snapshots coalescen por
    /// capa — una ráfaga de flechas = un solo Undo.
    MoverSeleccion { dx: i32, dy: i32 },
    /// Arma una selección que cubre el lienzo entero (`(0,0)..(w,h)`).
    /// No toca píxeles ni el historial. No-op si el lienzo es degenerado.
    SeleccionarTodo,
    /// Expande (`delta > 0`) o contrae (`delta < 0`) el rect de
    /// `model.seleccion` `delta` px por cada lado, clampeando al lienzo.
    /// Si la contracción colapsa el rect, limpia la selección. No toca
    /// píxeles ni el historial.
    ExpandirSeleccion(i32),
    /// Click con la herramienta Balde: flood fill desde el píxel local
    /// `(lx, ly)` con el color activo sobre la capa raster seleccionada.
    /// `rw, rh` son las dims del panel del lienzo (para la conversión
    /// local→imagen). Acotado a `model.seleccion` si la hay.
    RellenarFlood { lx: f32, ly: f32, rw: f32, rh: f32 },
}

/// Etiqueta del parámetro que se está editando con un slider in-situ
/// en el panel ops. Cada variante corresponde a un campo de una
/// `OpLocal` parametrizable. Los no-paramétricos (`Invertir`,
/// `Espejar*`) no aparecen acá — su edición es no-op.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ParametroSlider {
    BrilloDelta,
    ContrasteFactor,
    SaturacionFactor,
    TonalidadGrados,
    BlurRadio,
    OpacidadFactor,
    NivelesEntradaMin,
    NivelesEntradaMax,
    NivelesGamma,
}

impl ParametroSlider {
    /// Clave estable para `pushear_snapshot` — coalesce los eventos
    /// del mismo slider sobre la misma capa en una sola entrada de
    /// historial.
    pub(crate) fn clave_coalesce(self) -> &'static str {
        match self {
            ParametroSlider::BrilloDelta => "p:brillo",
            ParametroSlider::ContrasteFactor => "p:contraste",
            ParametroSlider::SaturacionFactor => "p:saturacion",
            ParametroSlider::TonalidadGrados => "p:tonalidad",
            ParametroSlider::BlurRadio => "p:blur",
            ParametroSlider::OpacidadFactor => "p:opacidad",
            ParametroSlider::NivelesEntradaMin => "p:niveles_min",
            ParametroSlider::NivelesEntradaMax => "p:niveles_max",
            ParametroSlider::NivelesGamma => "p:niveles_gamma",
        }
    }
}

// Constantes de datos compartidas (gradiente demo, thumbs, picker, histograma).
pub(crate) const W: u32 = 512;
pub(crate) const H: u32 = 320;

/// Lado del thumb en píxeles. Pequeño a propósito — la fila de capa es de
/// 28 px de alto y conviene dejar aire arriba/abajo.
pub(crate) const THUMB_LADO: u32 = 22;

/// Color de fallback cuando el cuentagotas todavía no leyó nada — un
/// gris medio opaco es el "neutro" que típicamente se usa como base.
pub(crate) const RELLENO_DEFAULT: [u8; 4] = [128, 128, 128, 255];

/// Tolerancia del balde (flood fill): suma de diferencias absolutas RGBA
/// permitida respecto al píxel semilla para considerar un vecino parte
/// de la misma región. `32` (≈8 por canal) tolera leve antialias sin
/// derramarse a colores distintos. Rango de la métrica: 0..=1020.
pub(crate) const TOL_BALDE: u32 = 32;

pub(crate) const PICKER_FILE_CAP: usize = 50_000;

pub(crate) const HIST_ALTO: f32 = 72.0;
