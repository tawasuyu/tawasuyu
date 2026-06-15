//! thumbs — **miniaturas de la Cola** (carátula de audio o frame de video).
//!
//! Caché por ruta, llenada en **background** (decode/extract son pesados, regla
//! del repo: no bloquear el hilo de UI). La extracción real vive en
//! `shared/foreign-av::extract_frame` (frame de video) y en
//! `media-core::metadata` (carátula embebida) — acá sólo el cacheo + el worker
//! + la clasificación por extensión. El render lo hace `vista_config` con
//! `View::image`. El instante representativo del frame de video lo da el grid
//! puro `media-core::thumbnail` (sin pedir probe de duración: offset fijo).

use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::OnceLock;
use std::time::Duration;

use llimphi_image::Image;
use llimphi_ui::Handle;
use media_core::thumbnail::ThumbGrid;
use parking_lot::Mutex;

use crate::tipos::Msg;

/// Ancho de la miniatura extraída (px). Alto proporcional.
const THUMB_W: u32 = 96;
/// Tope de miniaturas a cargar por tanda (evita N spawns de ffmpeg en colas
/// gigantes). Si la cola excede esto, el resto queda con ícono — se loguea.
const MAX_THUMBS: usize = 80;
/// Instante del frame de video a muestrear (los primeros segundos suelen ser
/// intro/negro; 3 s es un compromiso barato sin probe de duración).
const FRAME_AT: Duration = Duration::from_secs(3);

/// Caché ruta → miniatura. **Ausente** = no intentada; `Some(None)` = intentada
/// sin resultado (no reintentar); `Some(Some(img))` = lista.
fn cache() -> &'static Mutex<HashMap<String, Option<Image>>> {
    static SLOT: OnceLock<Mutex<HashMap<String, Option<Image>>>> = OnceLock::new();
    SLOT.get_or_init(|| Mutex::new(HashMap::new()))
}

/// Miniatura cacheada para `key` (ruta), o `None` si no hay / no se intentó aún.
pub(crate) fn get(key: &str) -> Option<Image> {
    cache().lock().get(key).cloned().flatten()
}

fn ext_lower(path: &Path) -> Option<String> {
    path.extension()
        .and_then(|s| s.to_str())
        .map(str::to_ascii_lowercase)
}

/// Computa la miniatura de `path` (pesado → background):
/// - audio → carátula embebida (ID3/FLAC) decodificada;
/// - imagen → la propia imagen decodificada (primer frame en GIF);
/// - video → un frame extraído con ffmpeg.
fn compute(path: &Path) -> Option<Image> {
    match ext_lower(path).as_deref() {
        Some("wav" | "mp3" | "opus" | "ogg" | "flac" | "m4a" | "aac") => {
            let meta = crate::media_io::load_media_metadata(path);
            meta.cover
                .as_ref()
                .and_then(|c| llimphi_image::decode_bytes(&c.data).ok())
        }
        Some("png" | "jpg" | "jpeg" | "webp" | "bmp" | "tiff" | "gif") => {
            std::fs::read(path)
                .ok()
                .and_then(|bytes| llimphi_image::decode_bytes(&bytes).ok())
        }
        // Video (mp4/webm/mkv/mov/avi/flv/m4v/ogv/ivf…).
        _ => match foreign_av::extract_frame(path, FRAME_AT, THUMB_W) {
            Ok(png) => llimphi_image::decode_bytes(&png).ok(),
            Err(_) => None,
        },
    }
}

/// Carga en background las miniaturas de `paths` aún no intentadas (capeadas a
/// [`MAX_THUMBS`]), y dispara `Msg::ThumbsReady` de a poco para que la Cola se
/// repinte a medida que aparecen. No-op si no hay pendientes.
pub(crate) fn spawn_load(handle: &Handle<Msg>, paths: Vec<PathBuf>) {
    let pending: Vec<PathBuf> = {
        let c = cache().lock();
        paths
            .into_iter()
            .filter(|p| !c.contains_key(&p.to_string_lossy().into_owned()))
            .collect()
    };
    if pending.is_empty() {
        return;
    }
    let total = pending.len();
    let pending: Vec<PathBuf> = pending.into_iter().take(MAX_THUMBS).collect();
    if total > MAX_THUMBS {
        eprintln!(
            "media-app: miniaturas capeadas a {MAX_THUMBS} de {total} pistas (resto con ícono)"
        );
    }
    let h = handle.clone();
    handle.spawn(move || {
        for (i, p) in pending.iter().enumerate() {
            let key = p.to_string_lossy().into_owned();
            let img = compute(p);
            cache().lock().insert(key, img);
            // Repinta de a tandas para que vayan apareciendo sin saturar.
            if i % 4 == 3 {
                h.dispatch(Msg::ThumbsReady);
            }
        }
        Msg::ThumbsReady
    });
}

/// Clave de caché de un frame de **hover** del timeline: ruta + bucket del grid
/// (cuantiza el cursor para no extraer un frame por píxel).
fn hover_key(path: &str, bucket: u32) -> String {
    format!("{path}#h{bucket}")
}

/// Frame de hover ya extraído para `path` en `fraction` del timeline, o `None`
/// si aún no está. El bucketing es estable (mismo cursor → misma clave).
pub(crate) fn hover_frame(path: &str, fraction: f32) -> Option<Image> {
    let bucket = ThumbGrid::default().bucket_for_fraction(fraction);
    get(&hover_key(path, bucket))
}

/// Extrae en background el frame de hover de `path` para `fraction` (instante =
/// centro del bucket sobre `dur`), si no se intentó ya. Sólo video (extract_frame
/// falla en audio → sin preview). Dispara `Msg::ThumbsReady` al terminar.
pub(crate) fn spawn_hover_frame(
    handle: &Handle<Msg>,
    path: PathBuf,
    dur: Duration,
    fraction: f32,
) {
    let grid = ThumbGrid::default();
    let bucket = grid.bucket_for_fraction(fraction);
    let key = hover_key(&path.to_string_lossy(), bucket);
    {
        let mut c = cache().lock();
        if c.contains_key(&key) {
            return; // ya extraído o en curso
        }
        c.insert(key.clone(), None); // marca "en curso" para no relanzar
    }
    let instant = grid.instant_for_bucket(bucket, dur);
    handle.spawn(move || {
        let img = foreign_av::extract_frame(&path, instant, THUMB_W)
            .ok()
            .and_then(|png| llimphi_image::decode_bytes(&png).ok());
        cache().lock().insert(key, img);
        Msg::ThumbsReady
    });
}
