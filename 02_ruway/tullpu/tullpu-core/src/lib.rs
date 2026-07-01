//! `tullpu-core` — el hueso del editor de imágenes por capas.
//!
//! Tullpu (quechua: *teñir, dar color, pigmento*) es un editor donde la pila
//! de capas **es** un DAG content-addressed: cada capa es un nodo, cada
//! ajuste/filtro/op IA es una capa **derivada** que apunta a su madre por
//! `Uuid`. Cambiar la madre marca *stale* el cono descendiente — la UI ofrece
//! "regenerar". Es el mismo patrón que el haz de cuerpos de pluma
//! (`pluma-cuerpo`) traducido a píxeles.
//!
//! Este crate define **solo el modelo**. Sin gráficos, sin Llimphi, sin
//! conocimiento de qué modelo IA realiza un inpaint. El compositor vive en
//! `tullpu-render`; el catálogo de ops en `tullpu-ops`; la UI en
//! `tullpu-app-llimphi`.
//!
//! ## Serialización
//!
//! Un [`Lienzo`] se vuelca a un [`format::Objeto`] del grafo: `datos` lleva
//! el postcard de la cabecera (dimensiones + lista de capas con sus
//! parámetros), `hijos` lleva los hashes BLAKE3 de los buffers Rgba8 y las
//! máscaras —deduplicados—. El almacén direccionado por contenido garantiza
//! que dos capas con el mismo buffer se guardan una sola vez.

#![forbid(unsafe_code)]

pub mod historial;
pub use historial::{Etiqueta, Historial};

pub mod pixel;

use serde::{Deserialize, Serialize};
pub use uuid::Uuid;

/// Re-exporta el hash del grafo (BLAKE3, 32 bytes) tal como lo define
/// `shared/format`. Una capa apunta a su contenido por este hash; el
/// almacén content-addressed resuelve hash → bytes.
pub type Hash = format::Hash;

/// Hashea un buffer arbitrario con BLAKE3 — la primitiva del grafo. Es el
/// puente entre "los píxeles que tengo en RAM" y "el hash con el que esa
/// capa apunta a ellos".
pub fn hash_bytes(bytes: &[u8]) -> Hash {
    *blake3::hash(bytes).as_bytes()
}

// =============================================================================
//  Modos de fusión
// =============================================================================

/// Cómo se compone una capa sobre la composición de las capas inferiores.
/// El catálogo arranca con los modos canónicos de Porter-Duff + los aritméticos
/// más usados y crece para cubrir el set por-canal de Photoshop. Ampliable a
/// medida que el compositor (`tullpu-render`) los soporte; cada variante nueva
/// debe quedar cubierta por un test de regresión allá.
///
/// El orden de las variantes es **estable**: postcard serializa enums por
/// índice de variante, así que nuevas variantes se agregan **al final** —
/// nunca insertar en medio ni reordenar (rompe lienzos persistidos).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ModoFusion {
    /// `src.over(dst)` clásico — la capa pinta encima respetando su alfa.
    Normal,
    /// `out = src * dst`. Oscurece.
    Multiplicar,
    /// `out = 1 - (1-src)*(1-dst)`. Aclara.
    Pantalla,
    /// `out = if dst < 0.5 { 2*src*dst } else { 1 - 2*(1-src)*(1-dst) }`.
    Superponer,
    /// `out = max(src, dst)` por canal.
    Aclarar,
    /// `out = min(src, dst)` por canal.
    Oscurecer,
    /// `out = |src - dst|`.
    Diferencia,
    /// `out = src + dst` saturado.
    Aditivo,
    // ---- Familia "burn" (oscurecen aún más que multiplicar) -----------------
    /// Color Burn de Photoshop: `out = 1 - (1-dst)/src`, con `src=0 ⇒ 0`.
    SubExpQuemado,
    /// Linear Burn: `out = src + dst - 1` (clamped a 0).
    SubLinealQuemado,
    // ---- Familia "dodge" (aclaran aún más que pantalla) ---------------------
    /// Color Dodge: `out = dst / (1-src)`, con `src=1 ⇒ 1`.
    SobreExpAclarado,
    // ---- Familia "light" (Superponer ↔ HardLight con src/dst intercambiados,
    //      más las variantes que combinan burn + dodge) ----------------------
    /// Hard Light: igual que [`Superponer`] pero con `src`/`dst` intercambiados.
    LuzFuerte,
    /// Soft Light (fórmula Photoshop):
    /// `g(d) = (d ≤ 0.25) ? ((16*d - 12)*d + 4)*d : sqrt(d)`,
    /// `out = (s ≤ 0.5) ? d - (1-2s)*d*(1-d) : d + (2s-1)*(g(d) - d)`.
    LuzSuave,
    /// Vivid Light: Color Burn si `src < 0.5`, Color Dodge si no
    /// (con `src` reescalado a `[0,1]` en cada rama).
    LuzViva,
    /// Linear Light: `out = dst + 2*src - 1`.
    LuzLineal,
    /// Pin Light: `out = (src < 0.5) ? min(dst, 2*src) : max(dst, 2*src - 1)`.
    LuzPunto,
    /// Hard Mix: `out = (src + dst ≥ 1) ? 1 : 0` por canal. Posteriza fuerte.
    MezclaDura,
    // ---- Familia "comparativos" + aritméticos faltantes ---------------------
    /// Exclusion: `out = src + dst - 2*src*dst`. Como Diferencia pero más suave.
    Exclusion,
    /// Subtract: `out = dst - src` (clamped a 0).
    Resta,
    /// Divide: `out = dst / src` (clamped a 1; `src=0 ⇒ 1`).
    Division,
    // ---- Familia HSL (operan sobre el triple, no por canal) ----------------
    // Sigue el W3C Compositing & Blending spec: luminosidad ponderada
    // `Lum = 0.3R + 0.59G + 0.11B`, saturación `max - min`, y SetLum/SetSat
    // con ClipColor. Cuatro variantes simétricas que extraen una componente
    // de src y dos del dst.
    /// Hue: matiz de src, saturación y luminosidad de dst.
    HslTono,
    /// Saturation: saturación de src, matiz y luminosidad de dst.
    HslSaturacion,
    /// Color: matiz y saturación de src, luminosidad de dst.
    HslColor,
    /// Luminosity: luminosidad de src, matiz y saturación de dst.
    HslLuminosidad,
    // ---- Comparativos por luminosidad (eligen el triple completo) ----------
    // No per-canal: el blend mira `Lum(src)` vs `Lum(dst)` y elige uno de los
    // dos triples completo. Cortocircuitan antes del despacho per-channel.
    /// Darker Color de Photoshop: el triple (src o dst) con menor luminosidad.
    ColorMasOscuro,
    /// Lighter Color: el triple con mayor luminosidad.
    ColorMasClaro,
    // ---- Estocástico por píxel (umbralizador con PRNG espacial) ------------
    /// Dissolve de Photoshop: por cada píxel se calcula un umbral PRNG
    /// estable (sembrado por el `Uuid` de la capa) y se compara con el alfa
    /// efectivo del src. Si `src_alpha > umbral`, el píxel sale 100% src
    /// con alfa 1.0; si no, queda el dst. El resultado es un patrón de ruido
    /// granulado en lugar del fade de Normal. Como no es ni separable por
    /// canal ni una mezcla `(s,d)` determinista en el sentido habitual,
    /// vive como rama propia en `fundir_capa` — `mezclar_canal` no lo ve.
    Disolver,
}

impl Default for ModoFusion {
    fn default() -> Self {
        ModoFusion::Normal
    }
}

// =============================================================================
//  Frescura — el patrón stale/fresh del cono derivado
// =============================================================================

/// Estado de una capa derivada respecto a su madre. `Fresca` significa que el
/// buffer cacheado corresponde al output actual de la operación sobre el
/// contenido vigente de la madre. `Stale` significa que la madre cambió
/// (parámetro, contenido o transitivamente) y este buffer ya no es válido —
/// la UI pinta la conexión punteada y ofrece "regenerar".
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Frescura {
    Fresca,
    Stale,
}

impl Default for Frescura {
    fn default() -> Self {
        Frescura::Fresca
    }
}

// =============================================================================
//  Operaciones — el catálogo declarativo
// =============================================================================

/// Operaciones deterministas en proceso, sin IA. Las implementa
/// `tullpu-ops`; este crate solo declara su forma (para serializar el DAG).
/// La frontera entre "qué op existe" y "cómo se ejecuta" es exactamente la
/// misma que `pluma-notebook-core` ↔ `pluma-notebook-kernel-*`.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OpLocal {
    /// Invierte cada canal RGB; alfa intacto.
    Invertir,
    /// Suma `delta` a cada canal RGB. Rango sugerido `[-1.0, 1.0]`.
    Brillo { delta: f32 },
    /// Reescala alrededor de 0.5: `c' = (c - 0.5) * factor + 0.5`.
    Contraste { factor: f32 },
    /// Mapeo de niveles tipo curva: `(c - min) / (max - min)` elevado a `1/gamma`.
    Niveles {
        entrada_min: f32,
        entrada_max: f32,
        gamma: f32,
    },
    /// Desenfoque gaussiano isotrópico, `radio` en píxeles.
    Blur { radio: f32 },
    /// Multiplica el alfa por `factor`.
    Opacidad { factor: f32 },
    /// Saturación HSL: `factor=0` → escala de grises, `factor=1` → identidad,
    /// `factor>1` → satura.
    Saturacion { factor: f32 },
    /// Rota el matiz HSL por `grados` (mod 360).
    Tonalidad { grados: f32 },
    /// Espeja el buffer horizontalmente (swap de columnas alrededor del
    /// eje vertical central). No cambia dimensiones — encaja como
    /// derivada pixel-a-pixel.
    EspejarHorizontal,
    /// Espeja el buffer verticalmente (swap de filas alrededor del eje
    /// horizontal central). No cambia dimensiones.
    EspejarVertical,
    /// Curva tonal maestra: una función de transferencia `entrada→salida`
    /// definida por puntos de control en `[0,1]²` (`(x_entrada, y_salida)`),
    /// aplicada por igual a los tres canales RGB (alfa intacto). `tullpu-ops`
    /// interpola los puntos a una LUT de 256 entradas (Hermite monótona, sin
    /// overshoot) y la mapea por canal. Es la generalización de `Niveles`:
    /// donde Niveles ofrece negro/blanco/gamma, una curva permite cualquier
    /// forma (S de contraste, inversión parcial, solarizado…). Convención: los
    /// puntos viajan sin ordenar — `tullpu-ops` los ordena por `x` y clampa a
    /// `[0,1]`; con < 2 puntos válidos la LUT cae a identidad.
    Curvas { puntos: Vec<(f32, f32)> },
}

impl OpLocal {
    /// Curva tonal identidad: la diagonal `(0,0)→(1,1)`. Punto de partida
    /// para una capa de ajuste de curvas recién creada (la UI luego arrastra
    /// los puntos).
    pub fn curvas_identidad() -> OpLocal {
        OpLocal::Curvas {
            puntos: vec![(0.0, 0.0), (1.0, 1.0)],
        }
    }
}

/// La operación que produce una capa derivada a partir de su madre. Local =
/// determinista en proceso, Ia = a través de `pixel-verbo-daemon` (modelo
/// ONNX por socket). El daemon vive fuera de este crate; el modelo de datos
/// solo lleva el nombre + prompt + payload opaco.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum TransformacionPixel {
    Local(OpLocal),
    /// Operación servida por `pixel-verbo-daemon`. `modelo` es el nombre del
    /// modelo cargado en el daemon, `prompt` el texto cuando aplica (inpaint
    /// con prompt, restyle…), `params` un blob opaco (postcard) que el
    /// daemon interpreta — tullpu no conoce su forma.
    Ia {
        modelo: String,
        prompt: Option<String>,
        params: Vec<u8>,
    },
}

impl TransformacionPixel {
    /// Etiqueta legible para el grafo y la UI.
    pub fn etiqueta(&self) -> String {
        match self {
            TransformacionPixel::Local(op) => match op {
                OpLocal::Invertir => "invertir".into(),
                OpLocal::Brillo { .. } => "brillo".into(),
                OpLocal::Contraste { .. } => "contraste".into(),
                OpLocal::Niveles { .. } => "niveles".into(),
                OpLocal::Blur { .. } => "blur".into(),
                OpLocal::Opacidad { .. } => "opacidad".into(),
                OpLocal::Saturacion { .. } => "saturación".into(),
                OpLocal::Tonalidad { .. } => "tonalidad".into(),
                OpLocal::EspejarHorizontal => "espejar ↔".into(),
                OpLocal::EspejarVertical => "espejar ↕".into(),
                OpLocal::Curvas { .. } => "curvas".into(),
            },
            TransformacionPixel::Ia { modelo, .. } => format!("ia:{modelo}"),
        }
    }
}

// =============================================================================
//  Origen y capa
// =============================================================================

/// De dónde vino una capa. `Raster` es pintada/importada a mano — su
/// `contenido` es el único origen autoritativo. `Derivada` apunta a una
/// capa madre por `Uuid` y a la `TransformacionPixel` que la produce; su
/// `contenido` es un caché regenerable.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum OrigenCapa {
    Raster,
    Derivada {
        madre: Uuid,
        op: TransformacionPixel,
        estado: Frescura,
    },
}

/// Qué **clase** de capa es, ortogonal a su [`OrigenCapa`]. La mayoría de las
/// capas son `Pixeles` (un buffer Rgba8, pintado/importado/derivado). Un
/// `Grupo` es un contenedor: no tiene buffer propio (`contenido` se ignora),
/// sus hijos son las capas cuyo `grupo` apunta a su `id`, y el compositor las
/// funde en aislamiento antes de aplicar el blend/opacidad/máscara del grupo
/// —exactamente como una carpeta de Photoshop—. Un `Ajuste` es una capa de
/// ajuste no destructiva: no tiene buffer; al componer aplica su [`OpLocal`]
/// (per-píxel) al **compuesto de todo lo que tiene debajo dentro de su scope**,
/// modulado por su opacidad y máscara. A diferencia de una capa `Derivada`
/// (que transforma **una** madre y cachea el resultado), el ajuste se recalcula
/// en vivo y afecta a la pila entera inferior.
///
/// El orden de las variantes es estable (postcard serializa por índice):
/// variantes nuevas se agregan **al final**.
/// Parámetros editables de una capa de texto. El texto se rasteriza a un buffer
/// Rgba8 del tamaño del lienzo (que vive en `Capa::contenido`, como cualquier
/// raster) — por eso el compositor no necesita saber tipografía: trata la capa
/// de texto igual que una de píxeles. Guardar los params permite **re-editar**
/// el texto y re-rasterizar (no destructivo respecto al string original). La
/// rasterización vive en el frontend (la app, con fontdue) — `tullpu-core` no
/// depende de ninguna fuente.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamsTexto {
    pub texto: String,
    pub tamano: f32,
    pub color: [u8; 4],
    /// Esquina superior-izquierda del bloque de texto en coords-imagen.
    pub x: u32,
    pub y: u32,
}

/// Una orden de trazado de un path vectorial, en coordenadas-imagen (px,
/// origen arriba-izquierda). Cúbicas de Bézier + líneas; `Cerrar` une el
/// sub-path con su último `MoverA`. El orden de variantes es estable (postcard
/// serializa por índice): nuevas variantes se agregan **al final**.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ComandoPath {
    /// Arranca un sub-path nuevo en `(x, y)`.
    MoverA { x: f32, y: f32 },
    /// Línea recta hasta `(x, y)`.
    LineaA { x: f32, y: f32 },
    /// Bézier cúbica con dos puntos de control hasta `(x, y)`.
    CurvaA {
        c1x: f32,
        c1y: f32,
        c2x: f32,
        c2y: f32,
        x: f32,
        y: f32,
    },
    /// Cierra el sub-path actual (línea de vuelta al `MoverA`).
    Cerrar,
}

/// Operación booleana entre dos paths, resuelta de forma **no destructiva** por
/// *compound path* (concatenar sub-paths + regla de relleno) — exacta y sin
/// clipping de curvas. Sólo se ofrecen las dos que son exactas en el caso
/// general: `Unir` (relleno no-cero: el área es la unión si ambos paths giran
/// igual) y `Excluir` (par-impar: diferencia simétrica, agujero en la
/// intersección). La **resta** (`a − b`) y la **intersección** verdaderas NO son
/// expresables por regla de relleno para paths que se solapan parcialmente
/// (la resta por winding invertido sólo vale si `b ⊆ a`) — necesitan un clipper
/// de curvas y quedan fuera por ahora.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum BooleanoPath {
    /// Unión: área cubierta por cualquiera de los dos (relleno no-cero).
    Unir,
    /// Diferencia simétrica: área de exactamente uno (relleno par-impar).
    Excluir,
}

/// Regla de relleno para sub-paths que se cruzan o se anidan.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum ReglaRelleno {
    /// Par-impar (even-odd): un punto está dentro si lo cruza un nº impar de
    /// bordes. Hace agujeros con sub-paths anidados sin importar su sentido.
    ParImpar,
    /// No-cero (nonzero winding): cuenta el sentido de cruce.
    NoCero,
}

/// Un relleno de gradiente. Las paradas son `(offset 0..1, color RGBA8)`,
/// ordenadas por offset. Coordenadas en px-imagen.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum Gradiente {
    /// Gradiente lineal del punto `(x1, y1)` al `(x2, y2)`.
    Lineal {
        x1: f32,
        y1: f32,
        x2: f32,
        y2: f32,
        paradas: Vec<(f32, [u8; 4])>,
    },
    /// Gradiente radial centrado en `(cx, cy)` con radio `r`.
    Radial {
        cx: f32,
        cy: f32,
        r: f32,
        paradas: Vec<(f32, [u8; 4])>,
    },
}

impl Gradiente {
    /// Gradiente lineal de dos colores a lo largo de `(x1,y1)→(x2,y2)`.
    pub fn lineal(x1: f32, y1: f32, x2: f32, y2: f32, a: [u8; 4], b: [u8; 4]) -> Self {
        Gradiente::Lineal { x1, y1, x2, y2, paradas: vec![(0.0, a), (1.0, b)] }
    }

    /// Gradiente radial de dos colores: `a` en el centro, `b` en el borde.
    pub fn radial(cx: f32, cy: f32, r: f32, a: [u8; 4], b: [u8; 4]) -> Self {
        Gradiente::Radial { cx, cy, r, paradas: vec![(0.0, a), (1.0, b)] }
    }
}

/// Parámetros editables de una capa **vectorial**. Como en texto, el path se
/// rasteriza a un buffer Rgba8 del tamaño del lienzo (que vive en
/// `Capa::contenido`, como cualquier raster) — por eso el compositor trata la
/// capa vectorial igual que una de píxeles. Guardar los comandos permite
/// **re-editar** (mover puntos de control, cambiar relleno/trazo) y
/// re-rasterizar sin pérdida. La rasterización vive **fuera** de `tullpu-core`
/// (en `tullpu-ops`, con tiny-skia) — el core sólo lleva el modelo.
/// Remate (cap) de los extremos de un trazo abierto.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum CapTrazo {
    /// Corte a ras del extremo (butt).
    #[default]
    Plano,
    /// Semicírculo (round).
    Redondo,
    /// Cuadrado que sobresale media-anchura (square).
    Cuadrado,
}

/// Unión (join) de los vértices de un trazo.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum JoinTrazo {
    /// Punta (miter).
    #[default]
    Punta,
    /// Redondeada (round).
    Redondo,
    /// Biselada (bevel).
    Bisel,
}

/// Estilo extendido de trazo (cap/join/dash). Cuando `ParamsVector.estilo_trazo`
/// es `None` se usan los defaults (Plano/Punta, sólido).
#[derive(Debug, Clone, PartialEq, Default, Serialize, Deserialize)]
pub struct EstiloTrazo {
    pub cap: CapTrazo,
    pub join: JoinTrazo,
    /// Patrón de guiones `[trazo, hueco, trazo, hueco, …]` en px; vacío = sólido.
    pub dash: Vec<f32>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct ParamsVector {
    pub comandos: Vec<ComandoPath>,
    /// Color de relleno RGBA8; `None` = sin relleno sólido.
    pub relleno: Option<[u8; 4]>,
    /// Relleno de **gradiente**; tiene prioridad sobre `relleno` (sólido) cuando
    /// es `Some`. `None` = sin gradiente.
    pub gradiente: Option<Gradiente>,
    pub regla: ReglaRelleno,
    /// Color de trazo (contorno) RGBA8; `None` = sin trazo.
    pub trazo: Option<[u8; 4]>,
    /// Ancho de trazo en px (ignorado si `trazo` es `None`).
    pub ancho_trazo: f32,
    /// Estilo extendido del trazo (cap/join/dash); `None` = defaults.
    pub estilo_trazo: Option<EstiloTrazo>,
}

impl ParamsVector {
    /// Rectángulo con esquina superior-izquierda `(x, y)` y tamaño `w×h`,
    /// relleno sólido, sin trazo.
    pub fn rectangulo(x: f32, y: f32, w: f32, h: f32, relleno: [u8; 4]) -> Self {
        Self {
            comandos: vec![
                ComandoPath::MoverA { x, y },
                ComandoPath::LineaA { x: x + w, y },
                ComandoPath::LineaA { x: x + w, y: y + h },
                ComandoPath::LineaA { x, y: y + h },
                ComandoPath::Cerrar,
            ],
            relleno: Some(relleno),
            gradiente: None,
            regla: ReglaRelleno::NoCero,
            trazo: None,
            ancho_trazo: 0.0,
            estilo_trazo: None,
        }
    }

    /// Elipse centrada en `(cx, cy)` con radios `(rx, ry)`, aproximada con
    /// cuatro cúbicas de Bézier (kappa de círculo), relleno sólido, sin trazo.
    pub fn elipse(cx: f32, cy: f32, rx: f32, ry: f32, relleno: [u8; 4]) -> Self {
        // Constante para aproximar un cuarto de círculo con una cúbica.
        const K: f32 = 0.552_284_75;
        let (ox, oy) = (rx * K, ry * K);
        Self {
            comandos: vec![
                ComandoPath::MoverA { x: cx + rx, y: cy },
                ComandoPath::CurvaA { c1x: cx + rx, c1y: cy + oy, c2x: cx + ox, c2y: cy + ry, x: cx, y: cy + ry },
                ComandoPath::CurvaA { c1x: cx - ox, c1y: cy + ry, c2x: cx - rx, c2y: cy + oy, x: cx - rx, y: cy },
                ComandoPath::CurvaA { c1x: cx - rx, c1y: cy - oy, c2x: cx - ox, c2y: cy - ry, x: cx, y: cy - ry },
                ComandoPath::CurvaA { c1x: cx + ox, c1y: cy - ry, c2x: cx + rx, c2y: cy - oy, x: cx + rx, y: cy },
                ComandoPath::Cerrar,
            ],
            relleno: Some(relleno),
            gradiente: None,
            regla: ReglaRelleno::NoCero,
            trazo: None,
            ancho_trazo: 0.0,
            estilo_trazo: None,
        }
    }

    // ---- Edición de path (primitivas puras para el pen tool) ----------------

    /// Puntos de ancla (on-curve) del path, cada uno con el índice del comando
    /// que lo define. Es lo que un editor dibuja como handles y usa para
    /// hit-test. `Cerrar` no aporta ancla.
    pub fn puntos_ancla(&self) -> Vec<(usize, [f32; 2])> {
        let mut v = Vec::new();
        for (i, c) in self.comandos.iter().enumerate() {
            match *c {
                ComandoPath::MoverA { x, y }
                | ComandoPath::LineaA { x, y }
                | ComandoPath::CurvaA { x, y, .. } => v.push((i, [x, y])),
                ComandoPath::Cerrar => {}
            }
        }
        v
    }

    /// Mueve el ancla (on-curve) del comando `idx` a `(x, y)`, arrastrando de
    /// forma **rígida** los controles de Bézier pegados a ella — el control
    /// entrante (`c2` de este comando si es `CurvaA`) y el saliente (`c1` del
    /// comando siguiente si es `CurvaA`) — para que mover el ancla no deforme
    /// las curvas adyacentes. No-op si `idx` no apunta a un ancla.
    pub fn mover_ancla(&mut self, idx: usize, x: f32, y: f32) {
        let (ax, ay) = match self.comandos.get(idx) {
            Some(ComandoPath::MoverA { x, y })
            | Some(ComandoPath::LineaA { x, y })
            | Some(ComandoPath::CurvaA { x, y, .. }) => (*x, *y),
            _ => return,
        };
        let (dx, dy) = (x - ax, y - ay);
        match self.comandos.get_mut(idx) {
            Some(ComandoPath::MoverA { x: cx, y: cy })
            | Some(ComandoPath::LineaA { x: cx, y: cy }) => {
                *cx = x;
                *cy = y;
            }
            Some(ComandoPath::CurvaA { c2x, c2y, x: cx, y: cy, .. }) => {
                *c2x += dx;
                *c2y += dy;
                *cx = x;
                *cy = y;
            }
            _ => return,
        }
        // Control saliente: el `c1` del comando siguiente, si es una curva.
        if let Some(ComandoPath::CurvaA { c1x, c1y, .. }) = self.comandos.get_mut(idx + 1) {
            *c1x += dx;
            *c1y += dy;
        }
    }

    /// Agrega un vértice recto `(x, y)` al final del path (lo que hace el pen
    /// tool al clickear). El primer punto se vuelve un `MoverA`; el resto,
    /// `LineaA`. Si el path termina en `Cerrar`, inserta antes para preservar
    /// el cierre.
    pub fn agregar_vertice(&mut self, x: f32, y: f32) {
        let nuevo = if self
            .comandos
            .iter()
            .any(|c| !matches!(c, ComandoPath::Cerrar))
        {
            ComandoPath::LineaA { x, y }
        } else {
            ComandoPath::MoverA { x, y }
        };
        if matches!(self.comandos.last(), Some(ComandoPath::Cerrar)) {
            let i = self.comandos.len() - 1;
            self.comandos.insert(i, nuevo);
        } else {
            self.comandos.push(nuevo);
        }
    }

    /// Elimina el comando de ancla en `idx`. No-op si está fuera de rango o si
    /// apunta a un `Cerrar`.
    pub fn eliminar_vertice(&mut self, idx: usize) {
        if idx < self.comandos.len() && !matches!(self.comandos[idx], ComandoPath::Cerrar) {
            self.comandos.remove(idx);
        }
    }

    /// Cierra el path (agrega `Cerrar` si tiene comandos y no termina ya en uno).
    pub fn cerrar_path(&mut self) {
        if !self.comandos.is_empty()
            && !matches!(self.comandos.last(), Some(ComandoPath::Cerrar))
        {
            self.comandos.push(ComandoPath::Cerrar);
        }
    }

    /// Puntos de control de las cúbicas, cada uno con `(índice de comando,
    /// es_c1, [x, y])`. `es_c1 = true` es el control saliente del ancla previa;
    /// `false` es el entrante al endpoint de esa curva. Para el editor de
    /// handles de Bézier.
    pub fn puntos_control(&self) -> Vec<(usize, bool, [f32; 2])> {
        let mut v = Vec::new();
        for (i, c) in self.comandos.iter().enumerate() {
            if let ComandoPath::CurvaA { c1x, c1y, c2x, c2y, .. } = *c {
                v.push((i, true, [c1x, c1y]));
                v.push((i, false, [c2x, c2y]));
            }
        }
        v
    }

    /// Mueve un punto de control de la cúbica en `idx` a `(x, y)`. `es_c1`
    /// elige el control saliente (`c1`) o el entrante (`c2`). No-op si `idx` no
    /// es una `CurvaA`.
    pub fn mover_control(&mut self, idx: usize, es_c1: bool, x: f32, y: f32) {
        if let Some(ComandoPath::CurvaA { c1x, c1y, c2x, c2y, .. }) = self.comandos.get_mut(idx) {
            if es_c1 {
                *c1x = x;
                *c1y = y;
            } else {
                *c2x = x;
                *c2y = y;
            }
        }
    }

    /// Convierte el segmento recto (`LineaA`) en `idx` a una cúbica, sembrando
    /// los controles a 1/3 y 2/3 del segmento (curva idéntica a la recta hasta
    /// que el usuario arrastre los handles). No-op si `idx` no es `LineaA` o no
    /// hay punto previo.
    pub fn convertir_a_curva(&mut self, idx: usize) {
        let (ex, ey) = match self.comandos.get(idx) {
            Some(ComandoPath::LineaA { x, y }) => (*x, *y),
            _ => return,
        };
        let prev = match idx.checked_sub(1).and_then(|i| self.comandos.get(i)) {
            Some(ComandoPath::MoverA { x, y })
            | Some(ComandoPath::LineaA { x, y })
            | Some(ComandoPath::CurvaA { x, y, .. }) => (*x, *y),
            _ => return,
        };
        let c1 = (prev.0 + (ex - prev.0) / 3.0, prev.1 + (ey - prev.1) / 3.0);
        let c2 = (prev.0 + 2.0 * (ex - prev.0) / 3.0, prev.1 + 2.0 * (ey - prev.1) / 3.0);
        self.comandos[idx] = ComandoPath::CurvaA {
            c1x: c1.0, c1y: c1.1, c2x: c2.0, c2y: c2.1, x: ex, y: ey,
        };
    }

    /// Convierte la cúbica (`CurvaA`) en `idx` de vuelta a un segmento recto
    /// (`LineaA`) al mismo endpoint — "esquinar" el nodo, inverso de
    /// [`Self::convertir_a_curva`]. Descarta los controles. No-op si `idx` no es
    /// `CurvaA`.
    pub fn convertir_a_linea(&mut self, idx: usize) {
        if let Some(ComandoPath::CurvaA { x, y, .. }) = self.comandos.get(idx).copied() {
            self.comandos[idx] = ComandoPath::LineaA { x, y };
        }
    }

    /// Inserta un ancla nuevo **en medio** del segmento cuyo endpoint es el
    /// comando `idx`, en el parámetro `t ∈ (0,1)`. Para un `LineaA` interpola
    /// linealmente; para un `CurvaA` parte la cúbica con de Casteljau en dos
    /// cúbicas que reproducen la curva original exactamente. No-op si `idx` no
    /// apunta a un `LineaA`/`CurvaA`, si no hay ancla previa, o si `t` no está en
    /// `(0,1)`. Es el "agregar punto a un trazado" del pen tool.
    pub fn insertar_vertice_en_segmento(&mut self, idx: usize, t: f32) {
        if !(t > 0.0 && t < 1.0) {
            return;
        }
        // Ancla previa (punto de arranque del segmento).
        let prev = match idx.checked_sub(1).and_then(|i| self.comandos.get(i)) {
            Some(ComandoPath::MoverA { x, y })
            | Some(ComandoPath::LineaA { x, y })
            | Some(ComandoPath::CurvaA { x, y, .. }) => (*x, *y),
            _ => return,
        };
        let lerp = |a: (f32, f32), b: (f32, f32)| {
            (a.0 + (b.0 - a.0) * t, a.1 + (b.1 - a.1) * t)
        };
        match self.comandos.get(idx).copied() {
            Some(ComandoPath::LineaA { x, y }) => {
                let m = lerp(prev, (x, y));
                // El endpoint original queda; insertamos el punto medio antes.
                self.comandos.insert(idx, ComandoPath::LineaA { x: m.0, y: m.1 });
            }
            Some(ComandoPath::CurvaA { c1x, c1y, c2x, c2y, x, y }) => {
                // de Casteljau: parte la cúbica P0-c1-c2-P1 en dos en `t`.
                let p0 = prev;
                let p1 = (c1x, c1y);
                let p2 = (c2x, c2y);
                let p3 = (x, y);
                let a = lerp(p0, p1);
                let b = lerp(p1, p2);
                let c = lerp(p2, p3);
                let d = lerp(a, b);
                let e = lerp(b, c);
                let m = lerp(d, e); // punto de división (on-curve)
                // Primera mitad: P0 –a– d– M ; segunda: M –e– c– P1.
                self.comandos[idx] = ComandoPath::CurvaA {
                    c1x: a.0, c1y: a.1, c2x: d.0, c2y: d.1, x: m.0, y: m.1,
                };
                self.comandos.insert(idx + 1, ComandoPath::CurvaA {
                    c1x: e.0, c1y: e.1, c2x: c.0, c2y: c.1, x: p3.0, y: p3.1,
                });
            }
            _ => {}
        }
    }

    /// Segmento del path más cercano a `(px, py)`: `(idx, t, dist)` con `idx` =
    /// comando endpoint del segmento, `t ∈ [0,1]` el parámetro del punto más
    /// próximo sobre él, y `dist` la distancia euclídea. `None` si no hay
    /// segmentos. Las cúbicas se muestrean en 32 pasos (suficiente para
    /// hit-testing interactivo). Alimenta el "agregar punto sobre el trazado":
    /// combinado con [`Self::insertar_vertice_en_segmento`] parte donde se clickea.
    pub fn segmento_mas_cercano(&self, px: f32, py: f32) -> Option<(usize, f32, f32)> {
        let mut prev: Option<(f32, f32)> = None;
        let mut mejor: Option<(usize, f32, f32)> = None;
        let mut considerar = |i: usize, t: f32, d: f32| {
            if mejor.map(|(_, _, md)| d < md).unwrap_or(true) {
                mejor = Some((i, t, d));
            }
        };
        for (i, c) in self.comandos.iter().enumerate() {
            match *c {
                ComandoPath::MoverA { x, y } => prev = Some((x, y)),
                ComandoPath::LineaA { x, y } => {
                    if let Some(p0) = prev {
                        let (t, d) = dist_punto_segmento(p0, (x, y), (px, py));
                        considerar(i, t, d);
                    }
                    prev = Some((x, y));
                }
                ComandoPath::CurvaA { c1x, c1y, c2x, c2y, x, y } => {
                    if let Some(p0) = prev {
                        let (t, d) = dist_punto_cubica(p0, (c1x, c1y), (c2x, c2y), (x, y), (px, py));
                        considerar(i, t, d);
                    }
                    prev = Some((x, y));
                }
                ComandoPath::Cerrar => {}
            }
        }
        mejor
    }

    /// Invierte la orientación de cada sub-path (lo recorre al revés,
    /// intercambiando los controles de cada cúbica) preservando la forma.
    /// Invertir dos veces es la identidad. Cambia el sentido del winding y el
    /// punto de arranque del trazo/dash.
    pub fn invertir_orientacion(&mut self) {
        self.comandos = invertir_comandos(&self.comandos);
    }

    /// Combina este path con `otro` por *compound path* según `modo`
    /// ([`BooleanoPath`]): concatena los sub-paths y fija la regla de relleno.
    /// No destructivo y exacto (sin clipping). Conserva el relleno/trazo de
    /// `self`. El resultado sigue siendo editable como un solo path vectorial.
    pub fn combinar_con(&self, otro: &ParamsVector, modo: BooleanoPath) -> ParamsVector {
        let mut out = self.clone();
        out.regla = match modo {
            BooleanoPath::Unir => ReglaRelleno::NoCero,
            BooleanoPath::Excluir => ReglaRelleno::ParImpar,
        };
        out.comandos.extend_from_slice(&otro.comandos);
        out
    }

    /// Traslada **todo** el path por `(dx, dy)` (mover la capa vectorial entera).
    pub fn trasladar(&mut self, dx: f32, dy: f32) {
        self.transformar([1.0, 0.0, 0.0, 1.0, dx, dy]);
    }

    /// Aplica una transformación afín `[a, b, c, d, e, f]` a **todos** los
    /// puntos del path (anclas y controles): `x' = a·x + c·y + e`,
    /// `y' = b·x + d·y + f`. Cubre mover/escalar/rotar/sesgar la capa vectorial
    /// sin pérdida (sigue siendo vectorial). Misma convención que un
    /// `[scaleX, skewY, skewX, scaleY, transX, transY]` de SVG/Canvas.
    pub fn transformar(&mut self, m: [f32; 6]) {
        let [a, b, c, d, e, f] = m;
        let map = |x: f32, y: f32| (a * x + c * y + e, b * x + d * y + f);
        for cmd in &mut self.comandos {
            match cmd {
                ComandoPath::MoverA { x, y } | ComandoPath::LineaA { x, y } => {
                    let (nx, ny) = map(*x, *y);
                    *x = nx;
                    *y = ny;
                }
                ComandoPath::CurvaA { c1x, c1y, c2x, c2y, x, y } => {
                    let (a1, b1) = map(*c1x, *c1y);
                    let (a2, b2) = map(*c2x, *c2y);
                    let (ex, ey) = map(*x, *y);
                    *c1x = a1; *c1y = b1;
                    *c2x = a2; *c2y = b2;
                    *x = ex; *y = ey;
                }
                ComandoPath::Cerrar => {}
            }
        }
    }

    /// Segmento recto de `(x1, y1)` a `(x2, y2)` — sólo trazo (sin relleno).
    pub fn linea(x1: f32, y1: f32, x2: f32, y2: f32, color: [u8; 4], ancho: f32) -> Self {
        Self {
            comandos: vec![
                ComandoPath::MoverA { x: x1, y: y1 },
                ComandoPath::LineaA { x: x2, y: y2 },
            ],
            relleno: None,
            gradiente: None,
            regla: ReglaRelleno::NoCero,
            trazo: Some(color),
            ancho_trazo: ancho.max(0.5),
            estilo_trazo: None,
        }
    }

    /// Rectángulo de esquinas redondeadas (radio `r`, clampeado a la mitad del
    /// lado menor). Esquinas como cúbicas de Bézier (kappa de círculo).
    pub fn rect_redondeado(x: f32, y: f32, w: f32, h: f32, r: f32, relleno: [u8; 4]) -> Self {
        const K: f32 = 0.552_284_75;
        let rr = r.min(w * 0.5).min(h * 0.5).max(0.0);
        let o = rr * (1.0 - K); // offset de control desde la esquina
        let (x0, y0, x1, y1) = (x, y, x + w, y + h);
        let comandos = vec![
            ComandoPath::MoverA { x: x0 + rr, y: y0 },
            ComandoPath::LineaA { x: x1 - rr, y: y0 },
            ComandoPath::CurvaA { c1x: x1 - o, c1y: y0, c2x: x1, c2y: y0 + o, x: x1, y: y0 + rr },
            ComandoPath::LineaA { x: x1, y: y1 - rr },
            ComandoPath::CurvaA { c1x: x1, c1y: y1 - o, c2x: x1 - o, c2y: y1, x: x1 - rr, y: y1 },
            ComandoPath::LineaA { x: x0 + rr, y: y1 },
            ComandoPath::CurvaA { c1x: x0 + o, c1y: y1, c2x: x0, c2y: y1 - o, x: x0, y: y1 - rr },
            ComandoPath::LineaA { x: x0, y: y0 + rr },
            ComandoPath::CurvaA { c1x: x0, c1y: y0 + o, c2x: x0 + o, c2y: y0, x: x0 + rr, y: y0 },
            ComandoPath::Cerrar,
        ];
        Self {
            comandos,
            relleno: Some(relleno),
            gradiente: None,
            regla: ReglaRelleno::NoCero,
            trazo: None,
            ancho_trazo: 0.0,
            estilo_trazo: None,
        }
    }

    /// Estrella de `puntas` vértices alternando radio externo/interno alrededor
    /// de `(cx, cy)`. Arranca con una punta hacia arriba.
    pub fn estrella(cx: f32, cy: f32, r_ext: f32, r_int: f32, puntas: u32, relleno: [u8; 4]) -> Self {
        let n = puntas.max(2);
        let paso = core::f32::consts::PI / n as f32;
        let mut pts = Vec::with_capacity((2 * n) as usize);
        for i in 0..(2 * n) {
            let r = if i % 2 == 0 { r_ext } else { r_int };
            let ang = -core::f32::consts::FRAC_PI_2 + i as f32 * paso;
            pts.push((cx + r * ang.cos(), cy + r * ang.sin()));
        }
        Self::poligono(&pts, relleno)
    }

    /// Polígono regular de `lados` (≥ 3) inscrito en el círculo `(cx, cy, r)`,
    /// con un vértice hacia arriba.
    pub fn poligono_regular(cx: f32, cy: f32, r: f32, lados: u32, relleno: [u8; 4]) -> Self {
        let n = lados.max(3);
        let paso = core::f32::consts::TAU / n as f32;
        let mut pts = Vec::with_capacity(n as usize);
        for i in 0..n {
            let ang = -core::f32::consts::FRAC_PI_2 + i as f32 * paso;
            pts.push((cx + r * ang.cos(), cy + r * ang.sin()));
        }
        Self::poligono(&pts, relleno)
    }

    /// Polígono cerrado por los vértices dados (≥ 2), relleno sólido, sin trazo.
    pub fn poligono(puntos: &[(f32, f32)], relleno: [u8; 4]) -> Self {
        let mut comandos = Vec::with_capacity(puntos.len() + 1);
        if let Some(&(x, y)) = puntos.first() {
            comandos.push(ComandoPath::MoverA { x, y });
            for &(x, y) in &puntos[1..] {
                comandos.push(ComandoPath::LineaA { x, y });
            }
            comandos.push(ComandoPath::Cerrar);
        }
        Self {
            comandos,
            relleno: Some(relleno),
            gradiente: None,
            regla: ReglaRelleno::NoCero,
            trazo: None,
            ancho_trazo: 0.0,
            estilo_trazo: None,
        }
    }
}

/// Invierte la orientación de una lista de comandos de path, sub-path por
/// sub-path (cada uno delimitado por un `MoverA`, con un `Cerrar` opcional al
/// final). Cada sub-path se recorre al revés: el último ancla pasa a ser el
/// `MoverA`, cada segmento se invierte y los controles de las cúbicas se
/// intercambian (`c1 ↔ c2`). Pura; aplicarla dos veces reproduce la entrada.
fn invertir_comandos(comandos: &[ComandoPath]) -> Vec<ComandoPath> {
    let mut out = Vec::with_capacity(comandos.len());
    let mut i = 0;
    while i < comandos.len() {
        // Delimitar el sub-path [i, j): arranca en MoverA, corta antes del
        // próximo MoverA. `cerrado` = hay un Cerrar al final del sub-path.
        let ComandoPath::MoverA { x: sx, y: sy } = comandos[i] else {
            // Comando suelto sin MoverA previo (path degenerado): copialo tal cual.
            out.push(comandos[i]);
            i += 1;
            continue;
        };
        let mut j = i + 1;
        while j < comandos.len() && !matches!(comandos[j], ComandoPath::MoverA { .. }) {
            j += 1;
        }
        let cerrado = matches!(comandos.get(j - 1), Some(ComandoPath::Cerrar));
        let fin_seg = if cerrado { j - 1 } else { j }; // rango de segmentos (sin Cerrar)

        // Anclas del sub-path en orden: el MoverA + el endpoint de cada segmento.
        let mut anclas = vec![(sx, sy)];
        for c in &comandos[i + 1..fin_seg] {
            match *c {
                ComandoPath::LineaA { x, y } | ComandoPath::CurvaA { x, y, .. } => {
                    anclas.push((x, y))
                }
                _ => {}
            }
        }
        // Sub-path reversado: MoverA en el último ancla, luego segmentos al revés.
        let n = anclas.len();
        out.push(ComandoPath::MoverA { x: anclas[n - 1].0, y: anclas[n - 1].1 });
        for k in (0..n - 1).rev() {
            // Segmento original que llega al ancla k+1 es comandos[i+1 + k].
            let seg = comandos[i + 1 + k];
            let destino = anclas[k];
            match seg {
                ComandoPath::LineaA { .. } => {
                    out.push(ComandoPath::LineaA { x: destino.0, y: destino.1 });
                }
                ComandoPath::CurvaA { c1x, c1y, c2x, c2y, .. } => {
                    // Invertida: los controles se intercambian.
                    out.push(ComandoPath::CurvaA {
                        c1x: c2x, c1y: c2y, c2x: c1x, c2y: c1y, x: destino.0, y: destino.1,
                    });
                }
                _ => {}
            }
        }
        if cerrado {
            out.push(ComandoPath::Cerrar);
        }
        i = j;
    }
    out
}

/// Distancia de `p` al segmento recto `a→b` y el parámetro `t ∈ [0,1]` del
/// punto más cercano (clampeado a los extremos). Pura.
fn dist_punto_segmento(a: (f32, f32), b: (f32, f32), p: (f32, f32)) -> (f32, f32) {
    let (dx, dy) = (b.0 - a.0, b.1 - a.1);
    let len2 = dx * dx + dy * dy;
    let t = if len2 <= 1e-12 {
        0.0
    } else {
        (((p.0 - a.0) * dx + (p.1 - a.1) * dy) / len2).clamp(0.0, 1.0)
    };
    let (cx, cy) = (a.0 + dx * t, a.1 + dy * t);
    let d = ((p.0 - cx).powi(2) + (p.1 - cy).powi(2)).sqrt();
    (t, d)
}

/// Distancia de `p` a la cúbica `p0-c1-c2-p3` por muestreo (32 pasos) y el
/// parámetro `t` del punto muestreado más cercano. Pura.
fn dist_punto_cubica(
    p0: (f32, f32),
    c1: (f32, f32),
    c2: (f32, f32),
    p3: (f32, f32),
    p: (f32, f32),
) -> (f32, f32) {
    const PASOS: u32 = 32;
    let eval = |t: f32| {
        let u = 1.0 - t;
        let x = u * u * u * p0.0 + 3.0 * u * u * t * c1.0 + 3.0 * u * t * t * c2.0 + t * t * t * p3.0;
        let y = u * u * u * p0.1 + 3.0 * u * u * t * c1.1 + 3.0 * u * t * t * c2.1 + t * t * t * p3.1;
        (x, y)
    };
    let mut mejor = (0.0f32, f32::INFINITY);
    for k in 0..=PASOS {
        let t = k as f32 / PASOS as f32;
        let (x, y) = eval(t);
        let d = ((p.0 - x).powi(2) + (p.1 - y).powi(2)).sqrt();
        if d < mejor.1 {
            mejor = (t, d);
        }
    }
    mejor
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum ClaseCapa {
    /// Capa con buffer Rgba8 en `contenido` (raster o derivada).
    Pixeles,
    /// Carpeta: agrupa a las capas cuyo `grupo == Some(self.id)`.
    Grupo,
    /// Capa de ajuste: aplica `op` al compuesto inferior al componer.
    Ajuste(OpLocal),
    /// Capa de texto: `contenido` lleva el texto ya rasterizado (se compone
    /// como píxeles); estos params permiten re-editar y re-rasterizar.
    Texto(ParamsTexto),
    /// Capa vectorial: `contenido` lleva el path ya rasterizado (se compone
    /// como píxeles); `ParamsVector` permite re-editar y re-rasterizar. Variante
    /// nueva → va **al final** (postcard serializa por índice).
    Vector(ParamsVector),
}

impl Default for ClaseCapa {
    fn default() -> Self {
        ClaseCapa::Pixeles
    }
}

/// Una capa del lienzo. El `id` es estable a través de regeneraciones — sirve
/// como ancla para que otras capas la apunten como madre. `contenido` es el
/// hash BLAKE3 del buffer Rgba8 (W*H*4 bytes) que vive en el almacén
/// content-addressed; `mascara` análogo para una máscara alfa opcional
/// (W*H bytes).
///
/// `clase` distingue píxeles / grupo / ajuste (ver [`ClaseCapa`]). `grupo` es
/// el `id` de la capa-grupo que contiene a ésta (`None` = nivel raíz);
/// modela la jerarquía de carpetas sin dejar de ser una lista plana. `clipping`
/// recorta esta capa a la alfa de la capa inmediatamente inferior en su mismo
/// grupo (clipping mask de Photoshop).
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Capa {
    pub id: Uuid,
    pub nombre: String,
    pub contenido: Hash,
    pub blend: ModoFusion,
    pub opacidad: f32,
    pub mascara: Option<Hash>,
    pub visible: bool,
    pub origen: OrigenCapa,
    pub clase: ClaseCapa,
    pub grupo: Option<Uuid>,
    pub clipping: bool,
}

impl Capa {
    /// Construye una capa raster a partir del hash de su buffer. Los defaults
    /// (Normal, opacidad 1.0, sin máscara, visible) son el caso "acabo de
    /// arrastrar este PNG al lienzo".
    pub fn raster(nombre: impl Into<String>, contenido: Hash) -> Self {
        Self {
            id: Uuid::new_v4(),
            nombre: nombre.into(),
            contenido,
            blend: ModoFusion::Normal,
            opacidad: 1.0,
            mascara: None,
            visible: true,
            origen: OrigenCapa::Raster,
            clase: ClaseCapa::Pixeles,
            grupo: None,
            clipping: false,
        }
    }

    /// Construye una capa-grupo (carpeta) vacía. No tiene buffer propio: su
    /// `contenido` es un hash centinela que el compositor ignora. Las capas
    /// que la integran se cuelgan poniendo su `grupo` al `id` devuelto en
    /// [`Capa::id`]. Default visible, opacidad 1.0, blend Normal.
    pub fn grupo(nombre: impl Into<String>) -> Self {
        Self {
            id: Uuid::new_v4(),
            nombre: nombre.into(),
            contenido: [0u8; 32],
            blend: ModoFusion::Normal,
            opacidad: 1.0,
            mascara: None,
            visible: true,
            origen: OrigenCapa::Raster,
            clase: ClaseCapa::Grupo,
            grupo: None,
            clipping: false,
        }
    }

    /// Construye una capa de ajuste no destructiva. No tiene buffer: al
    /// componer, `op` (per-píxel) se aplica al compuesto de lo que tenga
    /// debajo dentro de su grupo, modulada por opacidad y máscara. Sólo tienen
    /// sentido las ops per-píxel (invertir, brillo, contraste, niveles,
    /// saturación, tonalidad, curvas); las espaciales/alfa el compositor las
    /// ignora — ver [`pixel::ajustar_rgb_inplace`].
    pub fn ajuste(nombre: impl Into<String>, op: OpLocal) -> Self {
        Self {
            id: Uuid::new_v4(),
            nombre: nombre.into(),
            contenido: [0u8; 32],
            blend: ModoFusion::Normal,
            opacidad: 1.0,
            mascara: None,
            visible: true,
            origen: OrigenCapa::Raster,
            clase: ClaseCapa::Ajuste(op),
            grupo: None,
            clipping: false,
        }
    }

    /// Construye una capa derivada *stale* — el buffer todavía no se calculó.
    /// El compositor sabrá que tiene que invocar la op para llenarlo. El
    /// `contenido` inicial puede ser el hash del buffer vacío; el caller que
    /// quiera optimizar pasa el hash del último output conocido.
    pub fn derivada(
        nombre: impl Into<String>,
        madre: Uuid,
        op: TransformacionPixel,
        contenido_cache: Hash,
    ) -> Self {
        Self {
            id: Uuid::new_v4(),
            nombre: nombre.into(),
            contenido: contenido_cache,
            blend: ModoFusion::Normal,
            opacidad: 1.0,
            mascara: None,
            visible: true,
            origen: OrigenCapa::Derivada {
                madre,
                op,
                estado: Frescura::Stale,
            },
            clase: ClaseCapa::Pixeles,
            grupo: None,
            clipping: false,
        }
    }

    /// `true` si la capa es una carpeta-grupo.
    pub fn es_grupo(&self) -> bool {
        matches!(self.clase, ClaseCapa::Grupo)
    }

    /// La op de ajuste si la capa es de ajuste; `None` en otro caso.
    pub fn op_ajuste(&self) -> Option<&OpLocal> {
        match &self.clase {
            ClaseCapa::Ajuste(op) => Some(op),
            _ => None,
        }
    }

    /// `true` si la capa aporta píxeles al compuesto vía su `contenido`
    /// (raster, derivada, texto o vector rasterizado). Grupos y ajustes no.
    pub fn tiene_buffer(&self) -> bool {
        matches!(
            self.clase,
            ClaseCapa::Pixeles | ClaseCapa::Texto(_) | ClaseCapa::Vector(_)
        )
    }

    /// Los params de texto si la capa es de texto; `None` en otro caso.
    pub fn params_texto(&self) -> Option<&ParamsTexto> {
        match &self.clase {
            ClaseCapa::Texto(p) => Some(p),
            _ => None,
        }
    }

    /// Los params vectoriales si la capa es vectorial; `None` en otro caso.
    pub fn params_vector(&self) -> Option<&ParamsVector> {
        match &self.clase {
            ClaseCapa::Vector(p) => Some(p),
            _ => None,
        }
    }

    /// Construye una capa vectorial. `contenido` es el hash del buffer Rgba8 ya
    /// rasterizado (lo produce `tullpu_ops::rasterizar_vector`); `params`
    /// guarda el path editable.
    pub fn vector(nombre: impl Into<String>, contenido: Hash, params: ParamsVector) -> Self {
        Self {
            id: Uuid::new_v4(),
            nombre: nombre.into(),
            contenido,
            blend: ModoFusion::Normal,
            opacidad: 1.0,
            mascara: None,
            visible: true,
            origen: OrigenCapa::Raster,
            clase: ClaseCapa::Vector(params),
            grupo: None,
            clipping: false,
        }
    }

    /// Construye una capa de texto. `contenido` es el hash del buffer Rgba8 ya
    /// rasterizado (lo produce el frontend); `params` guarda el texto editable.
    pub fn texto(nombre: impl Into<String>, contenido: Hash, params: ParamsTexto) -> Self {
        Self {
            id: Uuid::new_v4(),
            nombre: nombre.into(),
            contenido,
            blend: ModoFusion::Normal,
            opacidad: 1.0,
            mascara: None,
            visible: true,
            origen: OrigenCapa::Raster,
            clase: ClaseCapa::Texto(params),
            grupo: None,
            clipping: false,
        }
    }

    /// `true` si esta capa tiene una operación derivada y está stale.
    pub fn esta_stale(&self) -> bool {
        matches!(
            self.origen,
            OrigenCapa::Derivada {
                estado: Frescura::Stale,
                ..
            }
        )
    }

    /// Madre directa, si la capa es derivada.
    pub fn madre(&self) -> Option<Uuid> {
        match &self.origen {
            OrigenCapa::Derivada { madre, .. } => Some(*madre),
            OrigenCapa::Raster => None,
        }
    }
}

// =============================================================================
//  Lienzo + grafo
// =============================================================================

/// El lienzo entero: dimensiones + pila de capas. El orden de `capas` es
/// pintura-de-pintor: índice 0 = fondo, último = encima.
#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub struct Lienzo {
    pub width: u32,
    pub height: u32,
    pub capas: Vec<Capa>,
}

impl Lienzo {
    /// Lienzo vacío del tamaño pedido.
    pub fn nuevo(width: u32, height: u32) -> Self {
        Self {
            width,
            height,
            capas: Vec::new(),
        }
    }

    /// Apila una capa encima (la convierte en la capa visible top).
    pub fn apilar(&mut self, capa: Capa) {
        self.capas.push(capa);
    }

    /// Índices (en orden visual fondo→tope) de las capas hijas directas de
    /// `grupo`: `None` = nivel raíz, `Some(id)` = dentro de esa carpeta. No
    /// recursa — devuelve sólo el nivel pedido. Es la primitiva que el
    /// compositor usa para recorrer la jerarquía nivel por nivel.
    pub fn hijos_directos(&self, grupo: Option<Uuid>) -> Vec<usize> {
        self.capas
            .iter()
            .enumerate()
            .filter(|(_, c)| c.grupo == grupo)
            .map(|(i, _)| i)
            .collect()
    }

    /// Mete las capas `ids` en una carpeta-grupo nueva llamada `nombre`. La
    /// carpeta se inserta en la posición de la capa más alta del conjunto y
    /// las capas pasan a colgar de ella (`grupo = Some(nuevo_id)`),
    /// preservando su orden relativo. Devuelve el `id` del grupo, o `None` si
    /// ningún `id` existe. Las capas conservan el grupo padre que tenían en
    /// común; si estaban en niveles distintos, el grupo hereda el del tope.
    pub fn agrupar(&mut self, ids: &[Uuid], nombre: impl Into<String>) -> Option<Uuid> {
        let seleccion: std::collections::HashSet<Uuid> = ids.iter().copied().collect();
        let posiciones: Vec<usize> = self
            .capas
            .iter()
            .enumerate()
            .filter(|(_, c)| seleccion.contains(&c.id))
            .map(|(i, _)| i)
            .collect();
        if posiciones.is_empty() {
            return None;
        }
        let tope = *posiciones.iter().max().unwrap();
        let padre = self.capas[tope].grupo;

        let mut grupo = Capa::grupo(nombre);
        grupo.grupo = padre;
        let nuevo_id = grupo.id;
        for &i in &posiciones {
            self.capas[i].grupo = Some(nuevo_id);
        }
        // Insertar el grupo justo encima de la capa más alta del conjunto.
        self.capas.insert(tope + 1, grupo);
        Some(nuevo_id)
    }

    /// Busca una capa por su `Uuid` y devuelve referencia mutable. La forma
    /// canónica de mutar parámetros de una capa madre y luego propagar stale.
    pub fn capa_mut(&mut self, id: Uuid) -> Option<&mut Capa> {
        self.capas.iter_mut().find(|c| c.id == id)
    }

    /// Igual pero inmutable.
    pub fn capa(&self, id: Uuid) -> Option<&Capa> {
        self.capas.iter().find(|c| c.id == id)
    }

    /// Marca *stale* todas las capas derivadas, directa o transitivamente, de
    /// `id`. Se invoca después de mutar el contenido o parámetros de una
    /// capa madre — la UI verá las conexiones punteadas y ofrecerá
    /// regenerarlas. Es BFS sobre el grafo `hija → madre` invertido.
    ///
    /// El propio `id` no se marca (es la capa que se acaba de tocar; su
    /// estado lo decide el caller).
    pub fn propagar_stale(&mut self, id: Uuid) -> usize {
        let mut frontera: Vec<Uuid> = vec![id];
        let mut afectados = 0usize;
        let mut i = 0;
        while i < frontera.len() {
            let madre_actual = frontera[i];
            for capa in self.capas.iter_mut() {
                if let OrigenCapa::Derivada {
                    madre, estado, ..
                } = &mut capa.origen
                {
                    if *madre == madre_actual && *estado == Frescura::Fresca {
                        *estado = Frescura::Stale;
                        afectados += 1;
                        frontera.push(capa.id);
                    }
                }
            }
            i += 1;
        }
        afectados
    }

    /// Sube una capa un puesto hacia el tope (mayor índice). Devuelve `true`
    /// si hubo movimiento; `false` si ya estaba en el tope o no existe. No
    /// invalida frescuras: la pila pintor-de-pintor reordena la composición,
    /// pero las dependencias madre→hija son por `Uuid` y siguen valiendo.
    pub fn mover_arriba(&mut self, id: Uuid) -> bool {
        let Some(i) = self.capas.iter().position(|c| c.id == id) else {
            return false;
        };
        if i + 1 >= self.capas.len() {
            return false;
        }
        self.capas.swap(i, i + 1);
        true
    }

    /// Baja una capa un puesto hacia el fondo (menor índice). Misma forma que
    /// [`mover_arriba`] en espejo.
    pub fn mover_abajo(&mut self, id: Uuid) -> bool {
        let Some(i) = self.capas.iter().position(|c| c.id == id) else {
            return false;
        };
        if i == 0 {
            return false;
        }
        self.capas.swap(i, i - 1);
        true
    }

    /// Duplica una capa: inserta una copia con `Uuid` nuevo justo encima de la
    /// original. Devuelve el `Uuid` recién acuñado, o `None` si `id` no
    /// existía. Una raster duplicada apunta al mismo hash de contenido (el
    /// almacén content-addressed no replica buffers). Una derivada apunta a
    /// la misma madre con la misma op y preserva su estado de frescura — el
    /// almacén ya tiene el buffer cacheado bajo ese hash. Las hijas existentes
    /// de la original siguen apuntando a la original; el clone vive su
    /// propia vida.
    pub fn duplicar(&mut self, id: Uuid) -> Option<Uuid> {
        let i = self.capas.iter().position(|c| c.id == id)?;
        let mut clon = self.capas[i].clone();
        clon.id = Uuid::new_v4();
        clon.nombre = format!("{} (copia)", clon.nombre);
        let nuevo_id = clon.id;
        self.capas.insert(i + 1, clon);
        Some(nuevo_id)
    }

    /// Marca una capa derivada como `Fresca` — lo invoca el compositor tras
    /// regenerar el buffer cacheado. Devuelve `true` si la capa existía y
    /// era derivada.
    pub fn marcar_fresca(&mut self, id: Uuid, nuevo_contenido: Hash) -> bool {
        if let Some(c) = self.capa_mut(id) {
            if let OrigenCapa::Derivada { estado, .. } = &mut c.origen {
                *estado = Frescura::Fresca;
                c.contenido = nuevo_contenido;
                return true;
            }
        }
        false
    }

    /// Orden topológico de las capas para regeneración: madres antes que
    /// hijas. Las capas raster aparecen en el orden visual (fondo→tope); las
    /// derivadas se reordenan para respetar `madre → hija`. Devuelve los
    /// `Uuid` en orden de regeneración válida.
    ///
    /// Si hay un ciclo (imposible si solo se construye con [`Capa::derivada`]
    /// que asigna `Uuid` nuevo, pero el modelo lo admite), las capas
    /// involucradas se devuelven al final en orden de inserción.
    pub fn orden_regeneracion(&self) -> Vec<Uuid> {
        use std::collections::{HashMap, HashSet};

        let ids: Vec<Uuid> = self.capas.iter().map(|c| c.id).collect();
        let id_set: HashSet<Uuid> = ids.iter().copied().collect();
        let madres: HashMap<Uuid, Option<Uuid>> = self
            .capas
            .iter()
            .map(|c| (c.id, c.madre().filter(|m| id_set.contains(m))))
            .collect();

        let mut resuelto: HashSet<Uuid> = HashSet::new();
        let mut salida: Vec<Uuid> = Vec::with_capacity(ids.len());
        let mut pendiente: Vec<Uuid> = ids.clone();

        // Kahn naïf: iteramos hasta que ningún progreso ocurra; lo no
        // resuelto va al final (cobertura defensiva ante ciclos).
        let mut hubo_progreso = true;
        while hubo_progreso {
            hubo_progreso = false;
            pendiente.retain(|id| {
                let madre = madres.get(id).copied().flatten();
                let listo = match madre {
                    None => true,
                    Some(m) => resuelto.contains(&m),
                };
                if listo {
                    salida.push(*id);
                    resuelto.insert(*id);
                    hubo_progreso = true;
                    false
                } else {
                    true
                }
            });
        }
        salida.extend(pendiente);
        salida
    }
}

// =============================================================================
//  Serialización al grafo
// =============================================================================

/// Errores del módulo. Se mantienen anchos: el caller decide cómo reportar.
#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("postcard: {0}")]
    Postcard(#[from] postcard::Error),
}

/// Vuelca un [`Lienzo`] a un [`format::Objeto`]. `datos` = postcard del
/// lienzo entero (cabecera + lista de capas con parámetros); `hijos` = hashes
/// únicos de los buffers Rgba8 y las máscaras a los que apuntan las capas,
/// en orden estable y deduplicados. El consumidor (`almacen`) ya garantiza
/// que cada hash distinto se guarda una sola vez.
pub fn lienzo_a_objeto(l: &Lienzo) -> Result<format::Objeto, Error> {
    let datos = postcard::to_allocvec(l)?;
    let mut hijos: Vec<Hash> = Vec::new();
    let mut vistos: std::collections::HashSet<Hash> = std::collections::HashSet::new();
    for c in &l.capas {
        // Grupos y ajustes no tienen buffer propio: su `contenido` es un hash
        // centinela que no apunta a nada en el almacén — no lo referenciamos.
        if c.tiene_buffer() && vistos.insert(c.contenido) {
            hijos.push(c.contenido);
        }
        if let Some(m) = c.mascara {
            if vistos.insert(m) {
                hijos.push(m);
            }
        }
    }
    Ok(format::Objeto { datos, hijos })
}

/// Recupera un [`Lienzo`] desde su [`format::Objeto`]. Los buffers a los que
/// apunta no se cargan aquí — el caller los pide al almacén por hash cuando
/// los necesita.
pub fn lienzo_desde_objeto(o: &format::Objeto) -> Result<Lienzo, Error> {
    let l = postcard::from_bytes(&o.datos)?;
    Ok(l)
}

// =============================================================================
//  Tests
// =============================================================================

#[cfg(test)]
mod tests {
    use super::*;

    fn h(byte: u8) -> Hash {
        [byte; 32]
    }

    // ----- edición de path vectorial ----------------------------------------

    #[test]
    fn puntos_ancla_lista_solo_on_curve() {
        let p = ParamsVector::rectangulo(0.0, 0.0, 10.0, 10.0, [0, 0, 0, 255]);
        // rect = M, L, L, L, Z → 4 anclas (el Cerrar no aporta).
        let anclas = p.puntos_ancla();
        assert_eq!(anclas.len(), 4);
        assert_eq!(anclas[0].1, [0.0, 0.0]);
    }

    #[test]
    fn mover_ancla_actualiza_el_punto() {
        let mut p = ParamsVector::rectangulo(0.0, 0.0, 10.0, 10.0, [0, 0, 0, 255]);
        p.mover_ancla(0, 5.0, 7.0);
        assert_eq!(p.puntos_ancla()[0].1, [5.0, 7.0]);
    }

    #[test]
    fn mover_ancla_de_curva_arrastra_controles_rigidamente() {
        // M(0,0) C(c1)(c2)(end). Mover el end +(10,0) debe mover c2 +(10,0) sin
        // cambiar la forma relativa de ese tramo.
        let mut p = ParamsVector {
            comandos: vec![
                ComandoPath::MoverA { x: 0.0, y: 0.0 },
                ComandoPath::CurvaA { c1x: 1.0, c1y: 2.0, c2x: 3.0, c2y: 4.0, x: 5.0, y: 6.0 },
            ],
            relleno: Some([0, 0, 0, 255]),
            gradiente: None,
            regla: ReglaRelleno::NoCero,
            trazo: None,
            ancho_trazo: 0.0,
            estilo_trazo: None,
        };
        p.mover_ancla(1, 15.0, 6.0); // +10 en x
        if let ComandoPath::CurvaA { c2x, c2y, x, .. } = p.comandos[1] {
            assert_eq!(x, 15.0);
            assert_eq!((c2x, c2y), (13.0, 4.0)); // c2 se trasladó +10 en x, y intacto
        } else {
            panic!("esperaba CurvaA");
        }
    }

    #[test]
    fn agregar_vertice_construye_polilinea() {
        let mut p = ParamsVector {
            comandos: vec![],
            relleno: Some([0, 0, 0, 255]),
            gradiente: None,
            regla: ReglaRelleno::NoCero,
            trazo: None,
            ancho_trazo: 0.0,
            estilo_trazo: None,
        };
        p.agregar_vertice(1.0, 1.0);
        p.agregar_vertice(2.0, 2.0);
        assert!(matches!(p.comandos[0], ComandoPath::MoverA { .. }));
        assert!(matches!(p.comandos[1], ComandoPath::LineaA { .. }));
        // Tras cerrar, un nuevo vértice se inserta ANTES del Cerrar.
        p.cerrar_path();
        p.agregar_vertice(3.0, 3.0);
        assert!(matches!(p.comandos[2], ComandoPath::LineaA { x: 3.0, .. }));
        assert!(matches!(p.comandos.last(), Some(ComandoPath::Cerrar)));
    }

    #[test]
    fn trasladar_mueve_todo_el_path() {
        let mut p = ParamsVector::rectangulo(0.0, 0.0, 10.0, 10.0, [0, 0, 0, 255]);
        p.trasladar(5.0, -3.0);
        assert_eq!(p.puntos_ancla()[0].1, [5.0, -3.0]);
    }

    #[test]
    fn transformar_escala_y_traslada() {
        let mut p = ParamsVector::rectangulo(1.0, 1.0, 2.0, 2.0, [0, 0, 0, 255]);
        // Escala x2 y traslada (10, 20): x' = 2x + 10, y' = 2y + 20.
        p.transformar([2.0, 0.0, 0.0, 2.0, 10.0, 20.0]);
        assert_eq!(p.puntos_ancla()[0].1, [12.0, 22.0]); // (1,1) → (12,22)
        assert_eq!(p.puntos_ancla()[2].1, [16.0, 26.0]); // (3,3) → (16,26)
    }

    #[test]
    fn convertir_a_curva_y_mover_control() {
        let mut p = ParamsVector {
            comandos: vec![
                ComandoPath::MoverA { x: 0.0, y: 0.0 },
                ComandoPath::LineaA { x: 9.0, y: 0.0 },
            ],
            relleno: Some([0, 0, 0, 255]),
            gradiente: None,
            regla: ReglaRelleno::NoCero,
            trazo: None,
            ancho_trazo: 0.0,
            estilo_trazo: None,
        };
        p.convertir_a_curva(1);
        // El segmento es ahora una cúbica con controles a 1/3 y 2/3.
        assert!(matches!(p.comandos[1], ComandoPath::CurvaA { .. }));
        let ctrls = p.puntos_control();
        assert_eq!(ctrls.len(), 2);
        assert_eq!(ctrls[0].2, [3.0, 0.0]); // c1 a 1/3
        assert_eq!(ctrls[1].2, [6.0, 0.0]); // c2 a 2/3
        p.mover_control(1, true, 3.0, 5.0); // bend c1 hacia arriba
        assert_eq!(p.puntos_control()[0].2, [3.0, 5.0]);
    }

    #[test]
    fn linea_es_solo_trazo() {
        let p = ParamsVector::linea(0.0, 0.0, 10.0, 5.0, [1, 2, 3, 255], 2.0);
        assert!(p.relleno.is_none());
        assert_eq!(p.trazo, Some([1, 2, 3, 255]));
        assert_eq!(p.puntos_ancla().len(), 2);
    }

    #[test]
    fn rect_redondeado_tiene_4_lineas_4_curvas_y_cierra() {
        let p = ParamsVector::rect_redondeado(0.0, 0.0, 20.0, 10.0, 3.0, [0, 0, 0, 255]);
        let lineas = p.comandos.iter().filter(|c| matches!(c, ComandoPath::LineaA { .. })).count();
        let curvas = p.comandos.iter().filter(|c| matches!(c, ComandoPath::CurvaA { .. })).count();
        assert_eq!((lineas, curvas), (4, 4));
        assert!(matches!(p.comandos.last(), Some(ComandoPath::Cerrar)));
    }

    #[test]
    fn estrella_y_poligono_regular_cuentan_vertices() {
        let e = ParamsVector::estrella(0.0, 0.0, 10.0, 4.0, 5, [0, 0, 0, 255]);
        assert_eq!(e.puntos_ancla().len(), 10); // 2 * puntas
        let h = ParamsVector::poligono_regular(0.0, 0.0, 10.0, 6, [0, 0, 0, 255]);
        assert_eq!(h.puntos_ancla().len(), 6);
    }

    #[test]
    fn convertir_a_linea_es_inverso_de_a_curva() {
        let mut p = ParamsVector {
            comandos: vec![
                ComandoPath::MoverA { x: 0.0, y: 0.0 },
                ComandoPath::LineaA { x: 9.0, y: 0.0 },
            ],
            relleno: None, gradiente: None, regla: ReglaRelleno::NoCero,
            trazo: Some([0, 0, 0, 255]), ancho_trazo: 1.0, estilo_trazo: None,
        };
        p.convertir_a_curva(1);
        assert!(matches!(p.comandos[1], ComandoPath::CurvaA { .. }));
        p.convertir_a_linea(1);
        // Vuelve a LineaA al mismo endpoint; los controles se descartaron.
        assert_eq!(p.comandos[1], ComandoPath::LineaA { x: 9.0, y: 0.0 });
        assert!(p.puntos_control().is_empty());
    }

    #[test]
    fn insertar_vertice_en_linea_parte_en_el_punto_medio() {
        let mut p = ParamsVector {
            comandos: vec![
                ComandoPath::MoverA { x: 0.0, y: 0.0 },
                ComandoPath::LineaA { x: 10.0, y: 0.0 },
            ],
            relleno: None, gradiente: None, regla: ReglaRelleno::NoCero,
            trazo: Some([0, 0, 0, 255]), ancho_trazo: 1.0, estilo_trazo: None,
        };
        p.insertar_vertice_en_segmento(1, 0.5);
        // Ahora hay 3 anclas: 0, punto medio (5,0), y el original (10,0).
        let anclas: Vec<[f32; 2]> = p.puntos_ancla().into_iter().map(|(_, xy)| xy).collect();
        assert_eq!(anclas, vec![[0.0, 0.0], [5.0, 0.0], [10.0, 0.0]]);
        // t fuera de (0,1) es no-op.
        let antes = p.comandos.len();
        p.insertar_vertice_en_segmento(1, 0.0);
        p.insertar_vertice_en_segmento(1, 1.0);
        assert_eq!(p.comandos.len(), antes);
    }

    #[test]
    fn insertar_vertice_en_curva_preserva_la_forma() {
        // Cúbica de (0,0) a (30,0) con controles arqueando hacia arriba.
        let p0 = (0.0f32, 0.0f32);
        let c1 = (10.0f32, 30.0f32);
        let c2 = (20.0f32, 30.0f32);
        let p3 = (30.0f32, 0.0f32);
        let mut p = ParamsVector {
            comandos: vec![
                ComandoPath::MoverA { x: p0.0, y: p0.1 },
                ComandoPath::CurvaA { c1x: c1.0, c1y: c1.1, c2x: c2.0, c2y: c2.1, x: p3.0, y: p3.1 },
            ],
            relleno: None, gradiente: None, regla: ReglaRelleno::NoCero,
            trazo: Some([0, 0, 0, 255]), ancho_trazo: 1.0, estilo_trazo: None,
        };
        // Evaluador de la cúbica original en t.
        let eval = |t: f32| {
            let u = 1.0 - t;
            let x = u*u*u*p0.0 + 3.0*u*u*t*c1.0 + 3.0*u*t*t*c2.0 + t*t*t*p3.0;
            let y = u*u*u*p0.1 + 3.0*u*u*t*c1.1 + 3.0*u*t*t*c2.1 + t*t*t*p3.1;
            (x, y)
        };
        let tt = 0.4;
        let esperado = eval(tt);
        p.insertar_vertice_en_segmento(1, tt);
        // Dos cúbicas ahora; el ancla intermedio está sobre la curva original.
        let curvas = p.comandos.iter().filter(|c| matches!(c, ComandoPath::CurvaA { .. })).count();
        assert_eq!(curvas, 2, "la curva se partió en dos cúbicas");
        let medio = p.puntos_ancla()[1].1; // el ancla insertado
        assert!((medio[0] - esperado.0).abs() < 1e-3, "x del punto de división sobre la curva");
        assert!((medio[1] - esperado.1).abs() < 1e-3, "y del punto de división sobre la curva");
        // El endpoint final sigue siendo P3.
        assert_eq!(p.puntos_ancla().last().unwrap().1, [p3.0, p3.1]);
    }

    #[test]
    fn segmento_mas_cercano_elige_el_correcto_y_su_t() {
        // Cuadrado 0..10: aristas superior (0,0)->(10,0), derecha, inferior, izq.
        let p = ParamsVector::rectangulo(0.0, 0.0, 10.0, 10.0, [0, 0, 0, 255]);
        // Un punto justo debajo del medio de la arista superior.
        let (idx, t, d) = p.segmento_mas_cercano(5.0, 0.3).unwrap();
        assert_eq!(idx, 1, "arista superior es el comando 1 (LineaA)");
        assert!((t - 0.5).abs() < 0.05, "t≈0.5 (medio de la arista)");
        assert!((d - 0.3).abs() < 1e-3, "distancia = 0.3");
        // Insertar ahí parte la arista en el punto (5,0).
        let mut p2 = p.clone();
        p2.insertar_vertice_en_segmento(idx, t);
        let cerca = p2.puntos_ancla().iter().any(|(_, xy)| (xy[0] - 5.0).abs() < 0.6 && xy[1].abs() < 0.6);
        assert!(cerca, "hay un ancla nuevo cerca de (5,0)");
    }

    #[test]
    fn invertir_orientacion_dos_veces_es_identidad() {
        // Path mixto: recta + curva, cerrado.
        let mut p = ParamsVector {
            comandos: vec![
                ComandoPath::MoverA { x: 0.0, y: 0.0 },
                ComandoPath::LineaA { x: 10.0, y: 0.0 },
                ComandoPath::CurvaA { c1x: 12.0, c1y: 4.0, c2x: 12.0, c2y: 8.0, x: 10.0, y: 10.0 },
                ComandoPath::Cerrar,
            ],
            relleno: Some([0, 0, 0, 255]), gradiente: None, regla: ReglaRelleno::NoCero,
            trazo: None, ancho_trazo: 0.0, estilo_trazo: None,
        };
        let orig = p.comandos.clone();
        p.invertir_orientacion();
        // El primer ancla del reversado es el último del original (10,10).
        assert_eq!(p.puntos_ancla()[0].1, [10.0, 10.0]);
        assert!(matches!(p.comandos.last(), Some(ComandoPath::Cerrar)), "sigue cerrado");
        p.invertir_orientacion();
        assert_eq!(p.comandos, orig, "invertir dos veces = identidad");
    }

    #[test]
    fn combinar_paths_fija_regla_segun_modo() {
        let a = ParamsVector::rectangulo(0.0, 0.0, 10.0, 10.0, [0, 0, 0, 255]);
        let b = ParamsVector::rectangulo(5.0, 5.0, 10.0, 10.0, [0, 0, 0, 255]);
        let na = a.comandos.len();
        let nb = b.comandos.len();
        // Unir: concatena, regla no-cero, conserva el relleno de `a`.
        let u = a.combinar_con(&b, BooleanoPath::Unir);
        assert_eq!(u.comandos.len(), na + nb);
        assert_eq!(u.regla, ReglaRelleno::NoCero);
        assert_eq!(u.relleno, a.relleno);
        // Excluir: par-impar (agujero en la intersección).
        let x = a.combinar_con(&b, BooleanoPath::Excluir);
        assert_eq!(x.regla, ReglaRelleno::ParImpar);
        assert_eq!(x.comandos.len(), na + nb);
    }

    #[test]
    fn eliminar_vertice_quita_el_ancla() {
        let mut p = ParamsVector::rectangulo(0.0, 0.0, 10.0, 10.0, [0, 0, 0, 255]);
        let antes = p.puntos_ancla().len();
        p.eliminar_vertice(0);
        assert_eq!(p.puntos_ancla().len(), antes - 1);
    }

    #[test]
    fn capa_raster_defaults_son_razonables() {
        let c = Capa::raster("fondo", h(1));
        assert_eq!(c.blend, ModoFusion::Normal);
        assert_eq!(c.opacidad, 1.0);
        assert!(c.visible);
        assert!(c.mascara.is_none());
        assert!(!c.esta_stale());
        assert!(c.madre().is_none());
    }

    #[test]
    fn capa_derivada_arranca_stale() {
        let madre = Uuid::new_v4();
        let op = TransformacionPixel::Local(OpLocal::Invertir);
        let c = Capa::derivada("inv", madre, op, h(0));
        assert!(c.esta_stale());
        assert_eq!(c.madre(), Some(madre));
    }

    #[test]
    fn propagar_stale_cubre_cono_descendiente() {
        // Cadena: A (raster) → B (derivada de A) → C (derivada de B).
        // Una hermana D (derivada de A) también debe quedar stale.
        let mut l = Lienzo::nuevo(64, 64);
        let a = Capa::raster("a", h(1));
        let id_a = a.id;
        l.apilar(a);

        let mut b = Capa::derivada("b", id_a, TransformacionPixel::Local(OpLocal::Invertir), h(0));
        // Asumimos que el compositor ya la regeneró:
        if let OrigenCapa::Derivada { estado, .. } = &mut b.origen {
            *estado = Frescura::Fresca;
        }
        let id_b = b.id;
        l.apilar(b);

        let mut c = Capa::derivada("c", id_b, TransformacionPixel::Local(OpLocal::Brillo { delta: 0.1 }), h(0));
        if let OrigenCapa::Derivada { estado, .. } = &mut c.origen {
            *estado = Frescura::Fresca;
        }
        let id_c = c.id;
        l.apilar(c);

        let mut d = Capa::derivada("d", id_a, TransformacionPixel::Local(OpLocal::Contraste { factor: 1.2 }), h(0));
        if let OrigenCapa::Derivada { estado, .. } = &mut d.origen {
            *estado = Frescura::Fresca;
        }
        let id_d = d.id;
        l.apilar(d);

        // Tocamos A.
        let afectadas = l.propagar_stale(id_a);
        assert_eq!(afectadas, 3, "B, C y D deben quedar stale");
        assert!(l.capa(id_b).unwrap().esta_stale());
        assert!(l.capa(id_c).unwrap().esta_stale());
        assert!(l.capa(id_d).unwrap().esta_stale());
    }

    #[test]
    fn marcar_fresca_actualiza_contenido() {
        let mut l = Lienzo::nuevo(8, 8);
        let a = Capa::raster("a", h(1));
        let id_a = a.id;
        l.apilar(a);
        let b = Capa::derivada("b", id_a, TransformacionPixel::Local(OpLocal::Invertir), h(0));
        let id_b = b.id;
        l.apilar(b);

        assert!(l.capa(id_b).unwrap().esta_stale());
        assert!(l.marcar_fresca(id_b, h(42)));
        let b_fresca = l.capa(id_b).unwrap();
        assert!(!b_fresca.esta_stale());
        assert_eq!(b_fresca.contenido, h(42));

        // Sobre una capa raster no aplica.
        assert!(!l.marcar_fresca(id_a, h(99)));
    }

    #[test]
    fn orden_regeneracion_respeta_madres() {
        // Construimos en orden visual: A (fondo), B↘A, C↘B, D (raster top).
        let mut l = Lienzo::nuevo(16, 16);
        let a = Capa::raster("a", h(1));
        let id_a = a.id;
        l.apilar(a);
        let b = Capa::derivada("b", id_a, TransformacionPixel::Local(OpLocal::Invertir), h(0));
        let id_b = b.id;
        l.apilar(b);
        let c = Capa::derivada("c", id_b, TransformacionPixel::Local(OpLocal::Invertir), h(0));
        let id_c = c.id;
        l.apilar(c);
        let d = Capa::raster("d", h(2));
        let id_d = d.id;
        l.apilar(d);

        let orden = l.orden_regeneracion();
        let pos = |x: Uuid| orden.iter().position(|y| *y == x).unwrap();
        assert!(pos(id_a) < pos(id_b));
        assert!(pos(id_b) < pos(id_c));
        // D no tiene madre — puede aparecer donde sea, pero está.
        assert!(orden.contains(&id_d));
        assert_eq!(orden.len(), 4);
    }

    #[test]
    fn lienzo_redondea_por_objeto() {
        let mut l = Lienzo::nuevo(32, 32);
        let a = Capa::raster("a", h(1));
        let mut b = Capa::raster("b", h(2));
        // La capa b reutiliza el mismo hash de contenido que a — dedup en hijos.
        b.contenido = h(1);
        let mut c = Capa::raster("c", h(3));
        c.mascara = Some(h(7));
        l.apilar(a);
        l.apilar(b);
        l.apilar(c);

        let obj = lienzo_a_objeto(&l).unwrap();
        // Hijos únicos: h(1), h(3), h(7).
        assert_eq!(obj.hijos.len(), 3);
        assert!(obj.hijos.contains(&h(1)));
        assert!(obj.hijos.contains(&h(3)));
        assert!(obj.hijos.contains(&h(7)));

        let l2 = lienzo_desde_objeto(&obj).unwrap();
        assert_eq!(l, l2);
    }

    #[test]
    fn hash_bytes_es_blake3() {
        let h1 = hash_bytes(b"tullpu");
        let h2 = hash_bytes(b"tullpu");
        let h3 = hash_bytes(b"pluma");
        assert_eq!(h1, h2);
        assert_ne!(h1, h3);
    }

    #[test]
    fn mover_arriba_abajo_reordena_pila() {
        let mut l = Lienzo::nuevo(8, 8);
        let a = Capa::raster("a", h(1));
        let b = Capa::raster("b", h(2));
        let c = Capa::raster("c", h(3));
        let (id_a, id_b, id_c) = (a.id, b.id, c.id);
        l.apilar(a);
        l.apilar(b);
        l.apilar(c);

        // Orden inicial: [a, b, c] (a fondo, c tope).
        assert!(l.mover_arriba(id_b)); // [a, c, b]
        let orden: Vec<Uuid> = l.capas.iter().map(|x| x.id).collect();
        assert_eq!(orden, vec![id_a, id_c, id_b]);

        // c ya está en tope tras el swap — no, b sí. Bajamos b: [a, b, c].
        assert!(l.mover_abajo(id_b));
        let orden: Vec<Uuid> = l.capas.iter().map(|x| x.id).collect();
        assert_eq!(orden, vec![id_a, id_b, id_c]);

        // a en el fondo no baja más; c en el tope no sube más.
        assert!(!l.mover_abajo(id_a));
        assert!(!l.mover_arriba(id_c));

        // Capa inexistente.
        assert!(!l.mover_arriba(Uuid::new_v4()));
        assert!(!l.mover_abajo(Uuid::new_v4()));
    }

    #[test]
    fn duplicar_inserta_clon_justo_encima() {
        let mut l = Lienzo::nuevo(8, 8);
        let a = Capa::raster("a", h(1));
        let id_a = a.id;
        l.apilar(a);
        let b = Capa::derivada("b", id_a, TransformacionPixel::Local(OpLocal::Invertir), h(0));
        let id_b = b.id;
        l.apilar(b);

        let id_clon = l.duplicar(id_a).expect("a existe");
        assert_eq!(l.capas.len(), 3);
        // Orden esperado: [a, a_clon, b]
        assert_eq!(l.capas[0].id, id_a);
        assert_eq!(l.capas[1].id, id_clon);
        assert_eq!(l.capas[2].id, id_b);

        // El clon tiene Uuid nuevo, mismo contenido, nombre con sufijo.
        assert_ne!(id_clon, id_a);
        assert_eq!(l.capas[1].contenido, h(1));
        assert_eq!(l.capas[1].nombre, "a (copia)");

        // Las hijas de la original siguen apuntando a la original, no al clon.
        assert_eq!(l.capa(id_b).unwrap().madre(), Some(id_a));

        // Duplicar una derivada copia la op y la madre.
        let id_clon_b = l.duplicar(id_b).expect("b existe");
        let clon_b = l.capa(id_clon_b).unwrap();
        assert_eq!(clon_b.madre(), Some(id_a));
        assert!(matches!(
            &clon_b.origen,
            OrigenCapa::Derivada { op: TransformacionPixel::Local(OpLocal::Invertir), .. }
        ));

        // Capa inexistente.
        assert!(l.duplicar(Uuid::new_v4()).is_none());
    }

    #[test]
    fn etiqueta_transformacion_es_legible() {
        assert_eq!(
            TransformacionPixel::Local(OpLocal::Brillo { delta: 0.2 }).etiqueta(),
            "brillo"
        );
        assert_eq!(
            TransformacionPixel::Ia {
                modelo: "sam".into(),
                prompt: None,
                params: vec![],
            }
            .etiqueta(),
            "ia:sam"
        );
    }
}
