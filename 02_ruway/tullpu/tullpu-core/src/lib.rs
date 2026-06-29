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
    /// (raster, derivada o texto rasterizado). Grupos y ajustes no.
    pub fn tiene_buffer(&self) -> bool {
        matches!(self.clase, ClaseCapa::Pixeles | ClaseCapa::Texto(_))
    }

    /// Los params de texto si la capa es de texto; `None` en otro caso.
    pub fn params_texto(&self) -> Option<&ParamsTexto> {
        match &self.clase {
            ClaseCapa::Texto(p) => Some(p),
            _ => None,
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
