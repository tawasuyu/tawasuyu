//! `nahual-thumb-core` — pipeline de miniaturas, **agnóstico de Llimphi**.
//!
//! Tres piezas, todas testeables sin gráficos ni threads:
//!
//! 1. **Generación** ([`generar_thumb_de_bytes`] / [`generar_thumb_de_archivo`]):
//!    decodifica una imagen y la reduce a un lado máximo, devolviendo un
//!    [`ThumbRgba`] (buffer Rgba8 crudo — el frontend lo envuelve en
//!    `peniko::Image`). No conoce el motor gráfico.
//! 2. **Cache RAM** ([`CacheThumbs`]): mapa `ClaveThumb → ThumbRgba` con la
//!    clave atada a `(path, mtime, size, lado)` — si el archivo cambia en
//!    disco, la clave cambia y el thumb viejo no se reusa. El backing en
//!    disco (paso 3 del plan) se enchufa detrás de esta misma API.
//! 3. **Planificador** ([`Planificador`]): gobierna *qué* generar para no
//!    disparar miles de threads de golpe. Mantiene una cola priorizada (lo
//!    visible primero), deduplica, limita la concurrencia y olvida lo que
//!    salió de la ventana al scrollear. Sin threads: el frontend pregunta
//!    [`Planificador::proximos`] y por cada path hace `Handle::spawn`.
//!
//! El bucle del frontend (p. ej. `nahual-gallery-llimphi`):
//!
//! ```ignore
//! // al cambiar la ventana visible:
//! for (i, path) in visibles.iter().enumerate() {
//!     if !cache.contiene_archivo(path, LADO) {
//!         plan.solicitar(path.clone(), i as u64);   // prioridad = orden
//!     }
//! }
//! plan.olvidar_excepto(&visibles_set);
//! for path in plan.proximos() {
//!     handle.spawn(move || match generar_thumb_de_archivo(&path, LADO) {
//!         Ok(t)  => Msg::ThumbListo(path, t),
//!         Err(e) => Msg::ThumbFallo(path, e),
//!     });
//! }
//! // al recibir Msg::ThumbListo(path, thumb):
//! //   plan.completar(&path); cache.insertar(clave, thumb); // y pedir más
//! ```

#![forbid(unsafe_code)]

use std::collections::{HashMap, HashSet};
use std::io::Cursor;
use std::path::{Path, PathBuf};

/// Miniatura cruda: buffer Rgba8 (`w*h*4` bytes) listo para envolver en un
/// `peniko::Image` desde el frontend. Agnóstico del motor gráfico.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThumbRgba {
    pub w: u32,
    pub h: u32,
    pub rgba: Vec<u8>,
}

/// Fallo al generar una miniatura. Diferenciamos formato-no-soportado de
/// error de IO/decode para que el frontend pinte distinto (un ícono "?" vs
/// un ícono de error roto).
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ThumbError {
    Io(String),
    Decode(String),
    FormatoNoSoportado,
}

impl std::fmt::Display for ThumbError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            ThumbError::Io(e) => write!(f, "io: {e}"),
            ThumbError::Decode(e) => write!(f, "decode: {e}"),
            ThumbError::FormatoNoSoportado => write!(f, "formato no soportado"),
        }
    }
}

impl std::error::Error for ThumbError {}

/// Decodifica una imagen en memoria y la reduce a un cuadro de `lado_max`
/// px (preservando aspect ratio). Usa `DynamicImage::thumbnail`, que es un
/// downscale rápido — suficiente para miniaturas. **Función pura.**
pub fn generar_thumb_de_bytes(bytes: &[u8], lado_max: u32) -> Result<ThumbRgba, ThumbError> {
    let reader = image::ImageReader::new(Cursor::new(bytes))
        .with_guessed_format()
        .map_err(|e| ThumbError::Io(e.to_string()))?;
    // `format()` es `None` si el formato detectado no está habilitado por
    // feature del crate `image` (hoy: png/jpeg/webp).
    if reader.format().is_none() {
        return Err(ThumbError::FormatoNoSoportado);
    }
    let img = reader.decode().map_err(|e| ThumbError::Decode(e.to_string()))?;
    let lado = lado_max.max(1) as f32;
    let (ow, oh) = (img.width(), img.height());
    // Sólo reducir: `thumbnail` también agranda, pero upscalear una
    // miniatura desperdicia RAM y se ve borrosa. Si ya entra en el cuadro,
    // se deja igual.
    let escala = (lado / ow as f32).min(lado / oh as f32).min(1.0);
    let rgba = if escala < 1.0 {
        let tw = ((ow as f32 * escala).round() as u32).max(1);
        let th = ((oh as f32 * escala).round() as u32).max(1);
        img.thumbnail(tw, th).to_rgba8()
    } else {
        img.to_rgba8()
    };
    Ok(ThumbRgba {
        w: rgba.width(),
        h: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

/// Lee un archivo y genera su miniatura. Para el MVP lee el archivo entero
/// a memoria antes de decodificar; el paso 4 del plan (thumb embebido
/// EXIF + downscale-on-decode) evitará decodificar el full-res.
pub fn generar_thumb_de_archivo(path: &Path, lado_max: u32) -> Result<ThumbRgba, ThumbError> {
    let bytes = std::fs::read(path).map_err(|e| ThumbError::Io(e.to_string()))?;
    generar_thumb_de_bytes(&bytes, lado_max)
}

/// Clave de cache atada al estado del archivo en disco. Si el archivo se
/// edita (cambia `mtime` o `size`) la clave cambia y el thumb viejo deja
/// de matchear — invalidación automática sin watcher.
#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct ClaveThumb {
    pub path: PathBuf,
    pub mtime_ns: u128,
    pub size: u64,
    pub lado: u32,
}

impl ClaveThumb {
    /// Construye la clave consultando `metadata` del archivo. `mtime_ns` es
    /// 0 si la plataforma no expone mtime (no rompe: sólo desactiva la
    /// invalidación por tiempo, queda la de tamaño).
    pub fn de_archivo(path: &Path, lado: u32) -> std::io::Result<Self> {
        let meta = std::fs::metadata(path)?;
        let mtime_ns = meta
            .modified()
            .ok()
            .and_then(|t| t.duration_since(std::time::UNIX_EPOCH).ok())
            .map(|d| d.as_nanos())
            .unwrap_or(0);
        Ok(Self {
            path: path.to_path_buf(),
            mtime_ns,
            size: meta.len(),
            lado,
        })
    }
}

/// Cache de miniaturas en RAM. La clave incluye el estado del archivo, así
/// que reabrir una carpeta sin cambios reusa todo. El paso 3 (cache disco)
/// se monta detrás de esta API sin tocar el frontend.
#[derive(Debug, Default)]
pub struct CacheThumbs {
    mapa: HashMap<ClaveThumb, ThumbRgba>,
}

impl CacheThumbs {
    pub fn nuevo() -> Self {
        Self::default()
    }

    pub fn insertar(&mut self, clave: ClaveThumb, thumb: ThumbRgba) {
        self.mapa.insert(clave, thumb);
    }

    pub fn obtener(&self, clave: &ClaveThumb) -> Option<&ThumbRgba> {
        self.mapa.get(clave)
    }

    /// ¿Hay un thumb fresco para este archivo? Hace `stat` para construir la
    /// clave actual; devuelve `false` si el archivo no existe o cambió.
    pub fn contiene_archivo(&self, path: &Path, lado: u32) -> bool {
        match ClaveThumb::de_archivo(path, lado) {
            Ok(clave) => self.mapa.contains_key(&clave),
            Err(_) => false,
        }
    }

    pub fn len(&self) -> usize {
        self.mapa.len()
    }

    pub fn is_empty(&self) -> bool {
        self.mapa.is_empty()
    }

    /// Descarta entradas que no satisfacen el predicado — para acotar el
    /// uso de RAM (p. ej. conservar sólo las claves de archivos aún en la
    /// carpeta actual).
    pub fn retener<F: FnMut(&ClaveThumb) -> bool>(&mut self, mut pred: F) {
        self.mapa.retain(|k, _| pred(k));
    }
}

/// Planificador de la cola de generación. No corre threads: decide *qué*
/// generar y *cuántos a la vez*. Prioridad: **menor número = más urgente**
/// (el frontend usa la distancia al viewport, p. ej. el orden de la fila).
#[derive(Debug)]
pub struct Planificador {
    max_en_vuelo: usize,
    en_vuelo: HashSet<PathBuf>,
    pendientes: Vec<(u64, PathBuf)>,
}

impl Planificador {
    /// `max_en_vuelo`: cuántas generaciones concurrentes permitir (= cuántos
    /// `Handle::spawn` vivos a la vez). Un valor sano es ~núcleos.
    pub fn nuevo(max_en_vuelo: usize) -> Self {
        Self {
            max_en_vuelo: max_en_vuelo.max(1),
            en_vuelo: HashSet::new(),
            pendientes: Vec::new(),
        }
    }

    /// Encola `path` con `prioridad`. No-op si ya está en vuelo. Si ya está
    /// pendiente, se queda con la prioridad más urgente (menor) de las dos
    /// — así re-solicitar al scrollear no duplica ni degrada.
    pub fn solicitar(&mut self, path: PathBuf, prioridad: u64) {
        if self.en_vuelo.contains(&path) {
            return;
        }
        if let Some(slot) = self.pendientes.iter_mut().find(|(_, p)| *p == path) {
            slot.0 = slot.0.min(prioridad);
            return;
        }
        self.pendientes.push((prioridad, path));
    }

    /// Saca los próximos paths a generar (hasta llenar el cupo de
    /// concurrencia), los de mayor prioridad primero, y los marca en vuelo.
    /// El frontend hace un `Handle::spawn` por cada uno.
    pub fn proximos(&mut self) -> Vec<PathBuf> {
        let cupo = self.max_en_vuelo.saturating_sub(self.en_vuelo.len());
        if cupo == 0 || self.pendientes.is_empty() {
            return Vec::new();
        }
        // Orden estable por prioridad ascendente; los empates conservan el
        // orden de inserción (FIFO dentro de la misma prioridad).
        self.pendientes.sort_by_key(|(p, _)| *p);
        let n = cupo.min(self.pendientes.len());
        let salientes: Vec<PathBuf> = self.pendientes.drain(0..n).map(|(_, p)| p).collect();
        for p in &salientes {
            self.en_vuelo.insert(p.clone());
        }
        salientes
    }

    /// Marca un path como terminado (éxito o fallo): libera un cupo de
    /// concurrencia. El frontend lo llama al recibir el Msg de resultado.
    pub fn completar(&mut self, path: &Path) {
        self.en_vuelo.remove(path);
    }

    /// Descarta los pendientes que ya no están en la ventana visible. Los
    /// en vuelo se dejan terminar (ya pagaron el decode); se sueltan solos
    /// al `completar`. Esto evita generar miniaturas que el usuario
    /// scrolleó de largo.
    pub fn olvidar_excepto(&mut self, visibles: &HashSet<PathBuf>) {
        self.pendientes.retain(|(_, p)| visibles.contains(p));
    }

    pub fn en_vuelo(&self) -> usize {
        self.en_vuelo.len()
    }

    pub fn pendientes(&self) -> usize {
        self.pendientes.len()
    }
}

// ============================================================================
//  Cache en disco (paso 3) — reabrir una carpeta sin re-decodificar.
// ============================================================================

/// Hash FNV-1a de 64 bits. Determinista entre ejecuciones (a diferencia del
/// `DefaultHasher` de std, que randomiza la semilla) — imprescindible para
/// que el nombre del archivo cache sea estable de un arranque al siguiente.
fn fnv1a(bytes: &[u8]) -> u64 {
    let mut h: u64 = 0xcbf2_9ce4_8422_2325;
    for &b in bytes {
        h ^= b as u64;
        h = h.wrapping_mul(0x0000_0100_0000_01b3);
    }
    h
}

/// Cache de miniaturas en disco. Persiste cada thumb como PNG bajo un
/// nombre que **codifica la clave entera** (`hash(path)_mtime_size_lado`):
/// si el archivo de origen cambia, su `mtime`/`size` cambian, el nombre
/// cambia, y el thumb viejo simplemente no se encuentra (queda huérfano
/// para un GC futuro). Así reabrir una carpeta sin cambios reusa todo el
/// trabajo de decodificación de la sesión anterior — el pilar de gThumb /
/// FastStone junto a la virtualización.
#[derive(Debug, Clone)]
pub struct CacheDisco {
    dir: PathBuf,
}

impl CacheDisco {
    /// Usa (y crea) `dir` como carpeta del cache.
    pub fn en(dir: PathBuf) -> std::io::Result<Self> {
        std::fs::create_dir_all(&dir)?;
        Ok(Self { dir })
    }

    /// Cache en la ubicación estándar: `$XDG_CACHE_HOME/nahual/thumbs`
    /// (cae a `~/.cache/nahual/thumbs`, y a `tmp` si no hay HOME).
    pub fn por_defecto() -> std::io::Result<Self> {
        let base = std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".cache")))
            .unwrap_or_else(std::env::temp_dir);
        Self::en(base.join("nahual").join("thumbs"))
    }

    /// Carpeta donde vive el cache.
    pub fn dir(&self) -> &Path {
        &self.dir
    }

    fn nombre(clave: &ClaveThumb) -> String {
        let h = fnv1a(clave.path.to_string_lossy().as_bytes());
        format!("{h:016x}_{}_{}_{}.png", clave.mtime_ns, clave.size, clave.lado)
    }

    fn ruta(&self, clave: &ClaveThumb) -> PathBuf {
        self.dir.join(Self::nombre(clave))
    }

    /// Devuelve el thumb cacheado para esta clave, o `None` si no está en
    /// disco (o el archivo está corrupto / ilegible).
    pub fn cargar(&self, clave: &ClaveThumb) -> Option<ThumbRgba> {
        let bytes = std::fs::read(self.ruta(clave)).ok()?;
        decodificar_png(&bytes)
    }

    /// Escribe el thumb a disco de forma atómica (tmp + rename) para que un
    /// corte a mitad de escritura no deje un PNG truncado que luego se lea
    /// como válido.
    pub fn guardar(&self, clave: &ClaveThumb, thumb: &ThumbRgba) -> std::io::Result<()> {
        let bytes = codificar_png(thumb)?;
        let ruta = self.ruta(clave);
        let tmp = ruta.with_extension("png.tmp");
        std::fs::write(&tmp, &bytes)?;
        std::fs::rename(&tmp, &ruta)?;
        Ok(())
    }
}

fn decodificar_png(bytes: &[u8]) -> Option<ThumbRgba> {
    let img = image::load_from_memory_with_format(bytes, image::ImageFormat::Png).ok()?;
    let rgba = img.to_rgba8();
    Some(ThumbRgba {
        w: rgba.width(),
        h: rgba.height(),
        rgba: rgba.into_raw(),
    })
}

fn codificar_png(thumb: &ThumbRgba) -> std::io::Result<Vec<u8>> {
    let buf = image::RgbaImage::from_raw(thumb.w, thumb.h, thumb.rgba.clone()).ok_or_else(|| {
        std::io::Error::new(std::io::ErrorKind::InvalidData, "buffer de thumb inválido")
    })?;
    let mut out = Vec::new();
    image::DynamicImage::ImageRgba8(buf)
        .write_to(&mut Cursor::new(&mut out), image::ImageFormat::Png)
        .map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e.to_string()))?;
    Ok(out)
}

/// Obtiene el thumb de `path` pasando por el cache en disco: si ya está
/// (clave fresca) lo carga; si no, lo genera y lo guarda. Pensada para
/// correr en el thread de `Handle::spawn`. El guardado es best-effort —
/// un fallo de disco no impide devolver el thumb recién generado.
pub fn obtener_o_generar(
    cache: &CacheDisco,
    path: &Path,
    lado: u32,
) -> Result<ThumbRgba, ThumbError> {
    let clave = ClaveThumb::de_archivo(path, lado).map_err(|e| ThumbError::Io(e.to_string()))?;
    if let Some(t) = cache.cargar(&clave) {
        return Ok(t);
    }
    let t = generar_thumb_de_archivo(path, lado)?;
    let _ = cache.guardar(&clave, &t);
    Ok(t)
}

#[cfg(test)]
mod pruebas {
    use super::*;

    /// Codifica un PNG sólido `w×h` del color dado, en memoria.
    fn png_solido(w: u32, h: u32, color: [u8; 4]) -> Vec<u8> {
        let img = image::RgbaImage::from_pixel(w, h, image::Rgba(color));
        let mut bytes = Vec::new();
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut Cursor::new(&mut bytes), image::ImageFormat::Png)
            .unwrap();
        bytes
    }

    #[test]
    fn genera_thumb_downscale_preserva_aspecto() {
        // 200×100 → thumb dentro de 64×64: el lado mayor queda en 64, el
        // menor a la mitad (32), preservando el aspecto 2:1.
        let png = png_solido(200, 100, [10, 20, 30, 255]);
        let t = generar_thumb_de_bytes(&png, 64).unwrap();
        assert_eq!(t.w, 64);
        assert_eq!(t.h, 32);
        assert_eq!(t.rgba.len() as u32, t.w * t.h * 4);
        // Color sólido se conserva (downscale de plano = plano).
        assert_eq!(&t.rgba[0..4], &[10, 20, 30, 255]);
    }

    #[test]
    fn imagen_chica_no_se_agranda() {
        // thumbnail() sólo reduce: una imagen menor al lado se deja igual.
        let png = png_solido(20, 20, [0, 0, 0, 255]);
        let t = generar_thumb_de_bytes(&png, 64).unwrap();
        assert_eq!((t.w, t.h), (20, 20));
    }

    #[test]
    fn bytes_basura_dan_error_no_panic() {
        let err = generar_thumb_de_bytes(b"no soy una imagen", 64).unwrap_err();
        // Formato no reconocido → FormatoNoSoportado (o Io según el guess).
        assert!(matches!(
            err,
            ThumbError::FormatoNoSoportado | ThumbError::Io(_) | ThumbError::Decode(_)
        ));
    }

    #[test]
    fn cache_guarda_y_recupera() {
        let mut cache = CacheThumbs::nuevo();
        let clave = ClaveThumb {
            path: PathBuf::from("/tmp/a.png"),
            mtime_ns: 123,
            size: 456,
            lado: 64,
        };
        let thumb = ThumbRgba {
            w: 2,
            h: 2,
            rgba: vec![1; 16],
        };
        assert!(cache.obtener(&clave).is_none());
        cache.insertar(clave.clone(), thumb.clone());
        assert_eq!(cache.obtener(&clave), Some(&thumb));
        assert_eq!(cache.len(), 1);
    }

    #[test]
    fn cache_clave_distinta_por_mtime() {
        // Mismo path/size/lado pero distinto mtime ⇒ clave distinta ⇒ miss.
        let mut cache = CacheThumbs::nuevo();
        let base = ClaveThumb {
            path: PathBuf::from("/tmp/a.png"),
            mtime_ns: 1,
            size: 10,
            lado: 64,
        };
        cache.insertar(
            base.clone(),
            ThumbRgba {
                w: 1,
                h: 1,
                rgba: vec![0; 4],
            },
        );
        let editado = ClaveThumb {
            mtime_ns: 2,
            ..base.clone()
        };
        assert!(cache.obtener(&editado).is_none(), "mtime nuevo invalida");
    }

    #[test]
    fn cache_retener_descarta() {
        let mut cache = CacheThumbs::nuevo();
        for i in 0..5 {
            cache.insertar(
                ClaveThumb {
                    path: PathBuf::from(format!("/tmp/{i}.png")),
                    mtime_ns: 0,
                    size: i,
                    lado: 64,
                },
                ThumbRgba {
                    w: 1,
                    h: 1,
                    rgba: vec![0; 4],
                },
            );
        }
        // Conservar sólo size par.
        cache.retener(|k| k.size % 2 == 0);
        assert_eq!(cache.len(), 3); // 0,2,4
    }

    #[test]
    fn planificador_limita_concurrencia() {
        let mut plan = Planificador::nuevo(2);
        for i in 0..5 {
            plan.solicitar(PathBuf::from(format!("/{i}")), i);
        }
        assert_eq!(plan.pendientes(), 5);
        let lote = plan.proximos();
        assert_eq!(lote.len(), 2, "sólo 2 en vuelo a la vez");
        assert_eq!(plan.en_vuelo(), 2);
        // Sin cupo libre, no salen más.
        assert!(plan.proximos().is_empty());
        // Al completar uno, se libera un cupo.
        plan.completar(&lote[0]);
        assert_eq!(plan.proximos().len(), 1);
    }

    #[test]
    fn planificador_prioriza_menor_numero() {
        let mut plan = Planificador::nuevo(1);
        plan.solicitar(PathBuf::from("/lejos"), 100);
        plan.solicitar(PathBuf::from("/cerca"), 1);
        let lote = plan.proximos();
        assert_eq!(lote, vec![PathBuf::from("/cerca")], "lo urgente primero");
    }

    #[test]
    fn planificador_dedup_y_mejora_prioridad() {
        let mut plan = Planificador::nuevo(4);
        plan.solicitar(PathBuf::from("/a"), 50);
        plan.solicitar(PathBuf::from("/a"), 5); // mismo path, más urgente
        assert_eq!(plan.pendientes(), 1, "no duplica");
        // Otro con prioridad intermedia para verificar el orden.
        plan.solicitar(PathBuf::from("/b"), 10);
        let lote = plan.proximos();
        assert_eq!(
            lote,
            vec![PathBuf::from("/a"), PathBuf::from("/b")],
            "/a tomó la prioridad 5"
        );
    }

    #[test]
    fn planificador_no_re_solicita_en_vuelo() {
        let mut plan = Planificador::nuevo(4);
        plan.solicitar(PathBuf::from("/a"), 1);
        let _ = plan.proximos(); // /a queda en vuelo
        plan.solicitar(PathBuf::from("/a"), 1); // debe ignorarse
        assert_eq!(plan.pendientes(), 0);
        assert_eq!(plan.en_vuelo(), 1);
    }

    #[test]
    fn planificador_olvida_fuera_de_ventana() {
        let mut plan = Planificador::nuevo(1);
        for n in ["/a", "/b", "/c"] {
            plan.solicitar(PathBuf::from(n), 1);
        }
        let visibles: HashSet<PathBuf> = [PathBuf::from("/b")].into_iter().collect();
        plan.olvidar_excepto(&visibles);
        assert_eq!(plan.pendientes(), 1);
        assert_eq!(plan.proximos(), vec![PathBuf::from("/b")]);
    }

    /// Carpeta temporal única para un test de cache (sin crate externo).
    fn tmpdir(tag: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("nahual_thumb_cache_{}_{}", std::process::id(), tag));
        let _ = std::fs::remove_dir_all(&d);
        d
    }

    fn thumb_demo() -> ThumbRgba {
        ThumbRgba {
            w: 3,
            h: 2,
            rgba: (0..3 * 2 * 4).map(|i| i as u8).collect(),
        }
    }

    #[test]
    fn fnv1a_es_determinista() {
        // Estabilidad cross-run: el mismo input siempre da el mismo hash
        // (a diferencia de DefaultHasher). Valor fijo conocido de FNV-1a.
        assert_eq!(fnv1a(b""), 0xcbf2_9ce4_8422_2325);
        assert_eq!(fnv1a(b"a"), fnv1a(b"a"));
        assert_ne!(fnv1a(b"a"), fnv1a(b"b"));
    }

    #[test]
    fn cache_disco_roundtrip() {
        let dir = tmpdir("roundtrip");
        let cache = CacheDisco::en(dir.clone()).unwrap();
        let clave = ClaveThumb {
            path: PathBuf::from("/fotos/a.png"),
            mtime_ns: 42,
            size: 99,
            lado: 64,
        };
        let thumb = thumb_demo();
        assert!(cache.cargar(&clave).is_none(), "miss antes de guardar");
        cache.guardar(&clave, &thumb).unwrap();
        let recuperado = cache.cargar(&clave).expect("hit tras guardar");
        // PNG es lossless sobre Rgba8 → bit-a-bit idéntico.
        assert_eq!(recuperado, thumb);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn cache_disco_nombre_codifica_la_clave() {
        // Distinto mtime ⇒ archivo distinto ⇒ el thumb viejo no se reusa.
        let dir = tmpdir("nombre");
        let cache = CacheDisco::en(dir.clone()).unwrap();
        let base = ClaveThumb {
            path: PathBuf::from("/fotos/a.png"),
            mtime_ns: 1,
            size: 10,
            lado: 64,
        };
        cache.guardar(&base, &thumb_demo()).unwrap();
        let editado = ClaveThumb {
            mtime_ns: 2,
            ..base.clone()
        };
        assert!(
            cache.cargar(&editado).is_none(),
            "mtime nuevo ⇒ nombre nuevo ⇒ miss"
        );
        assert!(cache.cargar(&base).is_some(), "el original sigue ahí");
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn obtener_o_generar_usa_y_puebla_el_disco() {
        let dir = tmpdir("obtener");
        let cache = CacheDisco::en(dir.clone()).unwrap();
        // PNG real en disco para generar desde él.
        let src = dir.join("origen.png");
        std::fs::write(&src, png_solido(100, 50, [9, 8, 7, 255])).unwrap();

        // Primera vez: miss de cache → genera + guarda.
        let clave = ClaveThumb::de_archivo(&src, 32).unwrap();
        assert!(cache.cargar(&clave).is_none());
        let t1 = obtener_o_generar(&cache, &src, 32).unwrap();
        assert_eq!((t1.w, t1.h), (32, 16));
        // Ahora está en disco.
        let t2 = cache.cargar(&clave).expect("poblado tras obtener_o_generar");
        assert_eq!(t1, t2);

        // Segunda vez: hit de cache, mismo resultado (sin re-generar).
        let t3 = obtener_o_generar(&cache, &src, 32).unwrap();
        assert_eq!(t1, t3);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn genera_thumb_de_archivo_roundtrip() {
        // Escribe un PNG temporal y genera su thumb desde disco.
        let dir = std::env::temp_dir();
        let path = dir.join("nahual_thumb_core_test.png");
        std::fs::write(&path, png_solido(120, 60, [200, 100, 50, 255])).unwrap();
        let t = generar_thumb_de_archivo(&path, 48).unwrap();
        assert_eq!((t.w, t.h), (48, 24));
        // La clave del archivo se construye y matchea tras insertar.
        let clave = ClaveThumb::de_archivo(&path, 48).unwrap();
        let mut cache = CacheThumbs::nuevo();
        cache.insertar(clave, t);
        assert!(cache.contiene_archivo(&path, 48));
        let _ = std::fs::remove_file(&path);
    }
}
