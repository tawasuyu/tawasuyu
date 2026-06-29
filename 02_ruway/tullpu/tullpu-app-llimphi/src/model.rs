//! Modelo y mensajes de la app `tullpu`: el `Model` (estado completo),
//! la enum `Msg` (eventos del bucle Elm), las herramientas y estructuras
//! auxiliares de selección/portapapeles, y las constantes de datos.
//!
//! Behavior-preserving split de `main.rs` — sin cambios funcionales.

use std::collections::HashMap;
use std::path::PathBuf;

use llimphi_clipboard::SystemClipboard;
use llimphi_module_file_picker::{PickerMsg, PickerState};
use llimphi_motion::Tween;
use llimphi_ui::llimphi_raster::peniko::ImageBrush as Image;
use llimphi_ui::{KeyEvent, PaintRect};
use llimphi_widget_edit_menu::EditAction;
use llimphi_widget_text_input::TextInputState;
use llimphi_widget_toast::Toast;

use pixel_verbo_core::{OpPixel, Proveedor};
use tullpu_core::{Hash, Historial, Lienzo, OpLocal};
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
    /// Pila de undo/redo del [`Lienzo`] (snapshots + cursor + coalescing por
    /// etiqueta). El motor es `tullpu_core::Historial<Lienzo>` (regla #2);
    /// `historial.rs` deja sólo los wrappers que tocan también `lienzo` y la
    /// selección. Capado a [`HIST_CAP`] entradas.
    pub(crate) hist: Historial<Lienzo>,
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
    /// Máscara de selección no rectangular (varita mágica / por color):
    /// hash de un buffer de 1 canal `W·H` (255 = seleccionado). Cuando es
    /// `Some`, es la forma **autoritativa** de la selección y `seleccion`
    /// guarda su bounding box (para overlay y ops rápidas). Las herramientas
    /// rectangulares (marquee/select-all/expand) la limpian — degradan a rect.
    pub(crate) seleccion_mascara: Option<Hash>,
    /// Overlay cacheado de la máscara de selección: una imagen `W·H` teñida
    /// (cian translúcido donde está seleccionado, transparente fuera) que el
    /// painter del lienzo dibuja sobre el composite para mostrar la forma
    /// exacta de la selección no rectangular. Se reconstruye cuando cambia
    /// `seleccion_mascara` y se limpia cuando la selección vuelve a rect/None.
    pub(crate) seleccion_overlay: Option<Image>,
    /// Estado del drag de selección mientras el usuario sostiene el
    /// click. `None` fuera de un drag. Se commitea a `seleccion` en
    /// el `End` y se limpia.
    pub(crate) seleccion_drag: Option<SeleccionDrag>,
    /// Estado del drag-to-move del contenido de una selección existente.
    /// `None` salvo mientras se arrastra desde adentro del rect. Excluye
    /// mutuamente a `seleccion_drag` (un press entra a uno u otro según
    /// caiga dentro o fuera de la selección vigente).
    pub(crate) mover_drag: Option<MoverDrag>,
    /// Estado del trazo del pincel mientras se sostiene el click. `None`
    /// fuera de un trazo.
    pub(crate) pincel_drag: Option<PincelDrag>,
    /// Radio actual del pincel/borrador en px-imagen. Ajustable con
    /// `[`/`]` (cuando la herramienta es de trazo) o los botones del panel.
    pub(crate) radio_pincel: i32,
    /// Dureza del pincel/borrador en `[0.0, 1.0]` (1.0 = borde duro).
    /// Ajustable con `{`/`}` (Shift+`[`/`]`) o los botones del panel.
    pub(crate) dureza_pincel: f32,
    /// Estado vivo de la tecla Shift (lo sincroniza `on_key` desde los
    /// eventos de la tecla, porque el handler de click no recibe
    /// modifiers). Habilita el trazo en línea recta: Shift+click pinta
    /// desde [`Model::ultimo_pincel`] hasta el punto nuevo.
    pub(crate) shift_held: bool,
    /// Estado vivo de la tecla Alt (idéntico patrón que `shift_held`). El
    /// tampón de clonado lo usa: Alt+click fija el origen del clon.
    pub(crate) alt_held: bool,
    /// Origen del clon (coords-imagen) fijado con Alt+click. `None` hasta
    /// fijarlo. Persiste entre trazos hasta re-fijarlo o cambiar de herramienta.
    pub(crate) clon_ancla: Option<(i32, i32)>,
    /// Offset `origen − inicio_de_trazo` bloqueado al empezar un trazo de clon;
    /// cada estampa copia del píxel destino + este offset. `None` fuera de un
    /// trazo de clon.
    pub(crate) clon_offset: Option<(i32, i32)>,
    /// Último punto pintado por el pincel en coords-imagen, persistente
    /// **entre trazos** (a diferencia de `pincel_drag.last_i*`, que vive
    /// sólo durante un drag). Ancla del trazo recto con Shift. `None`
    /// hasta el primer trazo.
    pub(crate) ultimo_pincel: Option<(i32, i32)>,
    /// Simetría activa del trazo (espejo sobre ejes del lienzo).
    pub(crate) simetria: Simetria,
    /// Drag en curso del degradé. `None` fuera de un drag.
    pub(crate) gradiente_drag: Option<GradienteDrag>,
    /// Drag en curso del lazo (selección a mano alzada): posición local
    /// acumulada + dims del panel + los vértices recogidos en coords-imagen.
    /// `None` fuera de un drag. Al soltar se rasteriza a `seleccion_mascara`.
    pub(crate) lazo_drag: Option<LazoDrag>,
    /// Edición de capa de texto en curso: `(uuid, input)`. Mientras es `Some`,
    /// el panel de ops muestra el text-input y las teclas editan el contenido
    /// (re-rasterizando en vivo). Enter/Escape lo cierra.
    pub(crate) editando_texto: Option<(Uuid, TextInputState)>,
    /// Portapapeles interno de píxeles (copy/cut). `None` hasta el
    /// primer Ctrl+C/Ctrl+X. Pegar (Ctrl+V) compone este clip sobre una
    /// capa nueva. Vive fuera del historial — un undo no lo limpia.
    pub(crate) portapapeles: Option<PortaPixeles>,
    /// Modo "editar máscara": cuando es `true` y la capa seleccionada
    /// tiene máscara, las herramientas de trazo (pincel/borrador/balde/
    /// degradé) pintan sobre el buffer de máscara (1 canal) en vez del
    /// contenido Rgba8. Pincel/balde/degradé escriben `valor_mascara`
    /// (gris arbitrario), borrador oculta (0). Es un flag puro de UI: si
    /// la capa no tiene máscara, el trazo cae al contenido como siempre.
    /// Persiste al cambiar de capa.
    pub(crate) editando_mascara: bool,
    /// Valor de gris (0..255) que el pincel escribe en la máscara cuando
    /// `editando_mascara` está activo: 255 revela del todo, 0 oculta del
    /// todo, intermedios dan transparencia parcial. Balde y degradé usan
    /// el mismo valor como pico de revelado. El borrador ignora esto y
    /// siempre apunta a 0 (ocultar). Default 255 (calco de fase 53).
    pub(crate) valor_mascara: u8,
    /// Cache de thumbnails de máscara por hash del buffer de 1 canal. Se
    /// expande a Rgba8 gris (v,v,v,255) para mostrarse junto al thumb de
    /// contenido en la fila de capa. Espejo de [`Model::thumbs`].
    pub(crate) thumbs_mascara: HashMap<Hash, Image>,
    /// Drag en curso en el editor de curvas (sección "parámetros" cuando
    /// la capa es una derivada `Curvas`). `None` fuera de un drag. El press
    /// sobre el canvas de la curva lo fija (índice del punto activo + dims
    /// del canvas para convertir deltas-px a coords `[0,1]`); el `End` lo
    /// limpia.
    pub(crate) curva_arrastrando: Option<CurvaDrag>,
    /// Barra de menú principal: índice del menú raíz abierto (`None`
    /// cerrado). Lo enciende el click sobre la barra (vía `menubar_view`).
    pub(crate) menu_open: Option<usize>,
    /// Fila activa (teclado) del dropdown principal. `usize::MAX` = ninguna.
    pub(crate) menu_active: usize,
    /// Animación de aparición/swap del dropdown principal.
    pub(crate) menu_anim: Tween<f32>,
    /// Menú contextual sobre el lienzo/capa: ancla `(x, y)` en ventana
    /// (`None` cerrado). Lo abre el right-click sobre el panel del lienzo.
    pub(crate) context_menu: Option<(f32, f32)>,
    /// Menú de edición de TEXTO: ancla `(x, y)` en ventana (`None`
    /// cerrado). Sólo se usa mientras se renombra una capa (hay un
    /// `TextInputState` focuseado) — right-click sobre el input lo abre.
    pub(crate) edit_menu: Option<(f32, f32)>,
    /// Fila activa (teclado) del menú de edición. `usize::MAX` = ninguna.
    pub(crate) edit_active: usize,
    /// Animación de aparición del menú de edición.
    pub(crate) edit_anim: Tween<f32>,
    /// Portapapeles del sistema para el menú de edición de texto del
    /// renombrado de capas. Independiente del `portapapeles` de píxeles.
    pub(crate) clipboard: SystemClipboard,
    /// Toasts efímeros vivos (confirmaciones/errores de export e import).
    /// Cada uno se auto-descarta a los `TOAST_TTL` vía `Msg::ToastExpire`.
    pub(crate) toasts: Vec<Toast>,
    /// Id incremental para correlacionar un toast con su `Msg::ToastExpire`.
    pub(crate) next_toast: u64,
}

/// Punto activo de un drag en el editor de curvas tonales. `rw`/`rh` son
/// las dimensiones en px del canvas de la curva al momento del press —
/// permiten que el handler de drag (que sólo recibe deltas-px) los
/// normalice a `[0,1]`.
#[derive(Debug, Clone, Copy)]
pub(crate) struct CurvaDrag {
    pub(crate) idx: usize,
    pub(crate) rw: f32,
    pub(crate) rh: f32,
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
    /// Drag sobre el lienzo pinta un trazo a mano alzada con el color
    /// activo sobre la capa raster seleccionada (acotado a la selección).
    Pincel,
    /// Como `Pincel` pero borra (alfa=0) en vez de pintar.
    Borrador,
    /// Drag define un eje; al soltar rellena un degradé lineal del color
    /// activo (en el ancla) a transparente (en el extremo), compuesto
    /// src-over sobre la capa raster (acotado a la selección).
    Degradado,
    /// Click selecciona por color (varita mágica contigua): inunda desde el
    /// píxel clickeado sobre el composite y arma una máscara de selección no
    /// rectangular. Tolerancia fija ([`TOL_BALDE`]).
    Varita,
    /// Drag a mano alzada define un polígono; al soltar lo rasteriza a una
    /// máscara de selección (lazo). Reusa la misma maquinaria de máscara que
    /// la varita.
    Lazo,
    /// Click crea una capa de texto en esa posición y entra en edición; lo que
    /// se tipea se rasteriza en vivo a la capa.
    Texto,
    /// Tampón de clonado: Alt+click fija el origen; luego el drag copia píxeles
    /// del origen (con el offset del primer punto) sobre la capa raster.
    Clonar,
}

impl Herramienta {
    pub(crate) fn etiqueta(self) -> &'static str {
        match self {
            Herramienta::Mover => "mover",
            Herramienta::Cuentagotas => "cuentagotas",
            Herramienta::Marco => "marco",
            Herramienta::Balde => "balde",
            Herramienta::Pincel => "pincel",
            Herramienta::Borrador => "borrador",
            Herramienta::Degradado => "degradé",
            Herramienta::Varita => "varita",
            Herramienta::Lazo => "lazo",
            Herramienta::Texto => "texto",
            Herramienta::Clonar => "clonar",
        }
    }

    /// `true` para las herramientas de trazo (pincel y borrador), que
    /// comparten el cableado de drag y el control de radio.
    pub(crate) fn es_trazo(self) -> bool {
        matches!(self, Herramienta::Pincel | Herramienta::Borrador)
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

/// Estado intermedio mientras el usuario pinta un trazo a mano alzada
/// con el pincel. `cur_l*` es la posición local actual (acumulando los
/// `dx, dy` de cada Move); `last_i*` el último punto YA pintado en
/// coords-imagen, para interpolar el segmento hasta el punto nuevo y no
/// dejar huecos cuando el cursor se mueve rápido. `rw, rh` se capturan
/// al inicio para que un resize mid-trazo no descoloque la conversión.
#[derive(Debug, Clone, Copy)]
pub(crate) struct PincelDrag {
    pub(crate) cur_lx: f32,
    pub(crate) cur_ly: f32,
    pub(crate) rw: f32,
    pub(crate) rh: f32,
    pub(crate) last_ix: i32,
    pub(crate) last_iy: i32,
}

/// Simetría del trazo: refleja cada estampa sobre el/los eje(s) central(es)
/// del lienzo. `Ninguna` = comportamiento normal.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum Simetria {
    Ninguna,
    /// Espejo izquierda↔derecha (eje vertical en el centro).
    Vertical,
    /// Espejo arriba↔abajo (eje horizontal en el centro).
    Horizontal,
    /// Las dos: 4 estampas por punto.
    Ambas,
}

impl Simetria {
    /// Cicla Ninguna → Vertical → Horizontal → Ambas → Ninguna.
    pub(crate) fn siguiente(self) -> Simetria {
        match self {
            Simetria::Ninguna => Simetria::Vertical,
            Simetria::Vertical => Simetria::Horizontal,
            Simetria::Horizontal => Simetria::Ambas,
            Simetria::Ambas => Simetria::Ninguna,
        }
    }

    pub(crate) fn etiqueta(self) -> &'static str {
        match self {
            Simetria::Ninguna => "✕",
            Simetria::Vertical => "↔",
            Simetria::Horizontal => "↕",
            Simetria::Ambas => "✛",
        }
    }
}

/// Estado del drag del degradé: ancla en coords-imagen (donde empezó) +
/// posición local actual del cursor (acumulando los `dx, dy` de cada
/// Move). Al soltar, se convierte la posición local a coord-imagen para
/// el extremo del eje. `rw, rh` se capturan al inicio.
#[derive(Debug, Clone, Copy)]
pub(crate) struct GradienteDrag {
    pub(crate) ancla_ix: f32,
    pub(crate) ancla_iy: f32,
    pub(crate) cur_lx: f32,
    pub(crate) cur_ly: f32,
    pub(crate) rw: f32,
    pub(crate) rh: f32,
}

/// Estado del drag del lazo: posición local actual (acumulando `dx, dy` de
/// cada Move) + dims del panel capturadas al inicio + la polilínea de vértices
/// recogidos en coords-imagen. Al soltar, los vértices se rasterizan a una
/// máscara de selección por relleno par-impar.
#[derive(Debug, Clone)]
pub(crate) struct LazoDrag {
    pub(crate) cur_lx: f32,
    pub(crate) cur_ly: f32,
    pub(crate) rw: f32,
    pub(crate) rh: f32,
    pub(crate) puntos: Vec<(i32, i32)>,
}

/// Radio inicial del pincel en px-imagen (disco lleno; diámetro ≈ `2·r+1`).
pub(crate) const RADIO_PINCEL: i32 = 3;
/// Tope del radio ajustable (radio 0 = 1 px; 64 = disco de 129 px).
pub(crate) const RADIO_PINCEL_MAX: i32 = 64;
/// Dureza inicial del pincel: 1.0 = disco duro (borde neto); 0.0 = todo
/// el radio en degradé hacia el borde. El alfa del trazo cae linealmente
/// desde `dureza·radio` hasta el borde.
pub(crate) const DUREZA_PINCEL: f32 = 1.0;

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
    /// Agrega una **capa de ajuste** no destructiva encima de la seleccionada.
    /// A diferencia de `Agregar` (que deriva de UNA madre y cachea), el ajuste
    /// aplica `op` al compuesto de todo lo que tiene debajo dentro de su grupo,
    /// recalculado en vivo al componer. Ver `ClaseCapa::Ajuste`.
    AgregarAjuste(OpLocal),
    /// Mete la capa `id` en una carpeta-grupo nueva (Photoshop: "group layer").
    /// La selección pasa al grupo recién creado.
    Agrupar(Uuid),
    /// Voltea la capa raster activa: `true` = horizontal (↔), `false` = vertical
    /// (↕). Edición raster directa, dimensiones intactas.
    VoltearCapa { horizontal: bool },
    /// Alterna la clipping mask de la capa `id`: cuando está activa, la capa se
    /// recorta a la alfa de la capa inmediatamente inferior de su grupo.
    ToggleClipping(Uuid),
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
    /// Press sobre el canvas del editor de curvas de la capa derivada
    /// `id`. `(lx, ly)` es la posición local del click y `(rw, rh)` las
    /// dimensiones del canvas — `update` convierte a coords-curva `[0,1]`
    /// (y se invierte: arriba = salida 1.0), engancha el punto de control
    /// más cercano (o inserta uno nuevo si el click cae lejos de todos) y
    /// arranca el drag. Emitido por `on_click_at`.
    CurvaPress {
        id: Uuid,
        lx: f32,
        ly: f32,
        rw: f32,
        rh: f32,
    },
    /// Move durante el drag de un punto de la curva `id`. `(dx, dy)` son
    /// deltas-px incrementales; `update` los normaliza con las dims
    /// guardadas en `curva_arrastrando` y reubica el punto activo
    /// (clamp en `y∈[0,1]`, y en `x` acotado entre vecinos para no
    /// cruzarlos). Marca la capa stale y recompone en vivo.
    CurvaArrastrar {
        id: Uuid,
        dx: f32,
        dy: f32,
    },
    /// End del drag de la curva: limpia `curva_arrastrando` y snapshotea
    /// el resultado (1 sola entrada de historial por gesto, vía coalesce).
    CurvaSoltar {
        id: Uuid,
    },
    /// Resetea la curva de la capa `id` a la diagonal identidad
    /// `(0,0)→(1,1)`. Botón del editor.
    CurvaReset {
        id: Uuid,
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
    /// Invierte la selección vigente (máscara o rect) dentro del lienzo.
    InvertirSeleccion,
    /// Arma una selección que cubre el lienzo entero (`(0,0)..(w,h)`).
    /// No toca píxeles ni el historial. No-op si el lienzo es degenerado.
    SeleccionarTodo,
    /// Expande (`delta > 0`) o contrae (`delta < 0`) el rect de
    /// `model.seleccion` `delta` px por cada lado, clampeando al lienzo.
    /// Si la contracción colapsa el rect, limpia la selección. No toca
    /// píxeles ni el historial.
    ExpandirSeleccion(i32),
    /// Click con la herramienta Varita: selección por color contigua desde el
    /// píxel local `(lx, ly)` sobre el composite. `rw, rh` = dims del panel
    /// del lienzo (conversión local→imagen). Arma `seleccion_mascara` + bbox.
    SeleccionarVarita { lx: f32, ly: f32, rw: f32, rh: f32 },
    /// Click con la herramienta Texto: crea una capa de texto en `(lx, ly)`
    /// y entra en edición.
    AgregarTexto { lx: f32, ly: f32, rw: f32, rh: f32 },
    /// Tecla durante la edición de una capa de texto: actualiza el string y
    /// re-rasteriza en vivo.
    TextoTecla(KeyEvent),
    /// Ajusta el tamaño de la capa de texto en edición por `delta` px.
    TextoTamano(f32),
    /// Cierra la edición de texto (Enter/Escape/click afuera).
    TerminarTexto,
    /// Press con la herramienta Clonar: Alt+press fija el origen; si ya hay
    /// origen, arranca un trazo de clonado en `(lx, ly)`.
    IniciarClon { lx: f32, ly: f32, rw: f32, rh: f32 },
    /// Move del trazo de clonado: clona el segmento desde el último punto.
    ContinuarClon { dx: f32, dy: f32 },
    /// End del trazo de clonado.
    FinalizarClon,
    /// Sincroniza el estado vivo de la tecla Alt (emitido por `on_key`).
    SetAlt(bool),
    /// Press con la herramienta Lazo: arranca la polilínea en `(lx, ly)`.
    IniciarLazo { lx: f32, ly: f32, rw: f32, rh: f32 },
    /// Move del lazo: acumula `(dx, dy)` y agrega el vértice nuevo.
    ContinuarLazo { dx: f32, dy: f32 },
    /// End del lazo: cierra el polígono y lo rasteriza a `seleccion_mascara`.
    FinalizarLazo,
    /// Click con la herramienta Balde: flood fill desde el píxel local
    /// `(lx, ly)` con el color activo sobre la capa raster seleccionada.
    /// `rw, rh` son las dims del panel del lienzo (para la conversión
    /// local→imagen). Acotado a `model.seleccion` si la hay.
    RellenarFlood { lx: f32, ly: f32, rw: f32, rh: f32 },
    /// Press con la herramienta Pincel: arranca un trazo estampando un
    /// disco en el píxel local `(lx, ly)`.
    IniciarTrazo { lx: f32, ly: f32, rw: f32, rh: f32 },
    /// Move del trazo: acumula `(dx, dy)` y pinta el segmento desde el
    /// último punto hasta el nuevo.
    ContinuarTrazo { dx: f32, dy: f32 },
    /// End del trazo: cierra el `pincel_drag` y corta el coalesce para
    /// que el próximo trazo sea un Undo independiente.
    FinalizarTrazo,
    /// Ajusta el radio del pincel/borrador en `delta` px, clampeado a
    /// `[0, RADIO_PINCEL_MAX]`. No toca el lienzo ni el historial.
    BumpRadioPincel(i32),
    /// Ajusta la dureza del pincel/borrador en `delta`, clampeada a
    /// `[0.0, 1.0]`. No toca el lienzo ni el historial.
    BumpDurezaPincel(f32),
    /// Sincroniza el estado vivo de la tecla Shift (emitido por `on_key`
    /// al presionar/soltar Shift). Sólo actualiza `model.shift_held`.
    SetShift(bool),
    /// Cicla la simetría del trazo (Ninguna→V→H→Ambas). No toca el lienzo.
    CiclarSimetria,
    /// Press con la herramienta Degradado: fija el ancla del eje en el
    /// píxel local `(lx, ly)`.
    IniciarDegradado { lx: f32, ly: f32, rw: f32, rh: f32 },
    /// Move del degradé: acumula `(dx, dy)` para ubicar el extremo.
    AjustarDegradado { dx: f32, dy: f32 },
    /// End del degradé: rellena el degradado del ancla al extremo actual.
    FinalizarDegradado,
    /// Agrega una máscara blanca (todo visible) a la capa seleccionada.
    /// No-op si ya tiene máscara.
    AgregarMascara,
    /// Construye una máscara desde la selección: visible dentro del rect,
    /// oculto fuera. Reemplaza la máscara existente. No-op sin selección.
    AgregarMascaraDeSeleccion,
    /// Invierte la máscara de la capa seleccionada (visible ↔ oculto).
    InvertirMascara,
    /// Quita la máscara de la capa (no destructivo — la imagen vuelve
    /// entera).
    QuitarMascara,
    /// Hornea la máscara al alfa del raster y la quita (destructivo).
    AplicarMascara,
    /// Alterna el modo "editar máscara": las herramientas de trazo pintan
    /// el buffer de máscara en vez del contenido. No toca el lienzo.
    ToggleEditarMascara,
    /// Ajusta el valor de gris (0..255) que el pincel escribe en la
    /// máscara, por el delta dado (clamped). No toca el lienzo.
    BumpValorMascara(i32),
    /// Barra de menú principal: abre/cierra un menú raíz (`None` = cerrar).
    MenuOpen(Option<usize>),
    /// Comando elegido en el menú principal o contextual — se traduce al
    /// `Msg` real ya existente y se despacha.
    MenuCommand(String),
    /// Cierra cualquier menú abierto (click-fuera / Esc / tras elegir).
    CloseMenus,
    /// Right-click sobre el panel del lienzo: `(x, y)` en ventana. Abre el
    /// menú contextual de capa/selección si no estamos renombrando, o el
    /// menú de edición de texto si sí.
    RightPressAt { x: f32, y: f32 },
    /// Acción elegida en el menú de edición de texto.
    EditMenuAction(EditAction),
    /// Navegación por teclado en el dropdown del menú principal.
    MenuNav(i32),
    /// Ejecuta la fila activa del menú principal (Enter).
    MenuActivate,
    /// Tick de animación de los dropdowns (sólo re-render).
    MenuTick,
    /// Navegación por teclado en el menú de edición de texto.
    EditNav(i32),
    /// Ejecuta la fila activa del menú de edición (Enter).
    EditActivate,
    /// Un toast cumplió su `TOAST_TTL`: se descarta del stack.
    ToastExpire(u64),
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
