//! `hapiy` — el **núcleo agnóstico** de la captura de pantalla.
//!
//! `hapiy` (quechua: *agarrar / atrapar*) es el "Spectacle" de la suite: atrapa
//! lo que mirada pinta. Acá vive la lógica que **no** necesita Wayland ni GPU ni
//! UI, así que es `cargo test`-eable de punta a punta:
//!
//! - [`Shot`] — una captura: RGBA8 contiguo (sin padding), con recorte a [`Region`].
//! - codificación a PNG ([`Shot::to_png`] / [`Shot::save_png`]).
//! - el trait [`Capturer`] (lo implementa el backend Wayland; [`MockCapturer`]
//!   para tests) + [`OutputInfo`] para elegir monitor.
//! - el **handoff a tullpu** ([`tullpu_launch`]): una captura se abre en el editor
//!   de imágenes de la suite para anotar/recortar — tullpu ya abre un PNG por arg.
//!
//! Lo que sí toca el sistema (cliente `zwlr_screencopy`, lanzar tullpu) vive en el
//! binario `hapiy`, detrás de este núcleo.

use std::path::{Path, PathBuf};

/// Una captura cruda: `width*height` píxeles RGBA8, fila por fila, sin padding.
#[derive(Clone)]
pub struct Shot {
    pub width: u32,
    pub height: u32,
    /// `width*height*4` bytes (R,G,B,A por píxel).
    pub rgba: Vec<u8>,
}

/// Un rectángulo de recorte en píxeles, relativo a la esquina superior-izquierda.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Region {
    pub x: u32,
    pub y: u32,
    pub w: u32,
    pub h: u32,
}

/// Información de una salida (monitor) para elegir qué capturar.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct OutputInfo {
    /// Nombre del conector DRM (`eDP-1`, `HDMI-A-1`, …) o etiqueta del backend.
    pub name: String,
    pub width: u32,
    pub height: u32,
}

impl Shot {
    /// Construye un `Shot`, validando que el buffer tenga el largo exacto.
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Shot, String> {
        let expected = width as usize * height as usize * 4;
        if rgba.len() != expected {
            return Err(format!(
                "buffer de {} bytes, se esperaban {expected} ({width}x{height} RGBA)",
                rgba.len()
            ));
        }
        Ok(Shot { width, height, rgba })
    }

    /// La región que cubre toda la captura.
    pub fn full_region(&self) -> Region {
        Region { x: 0, y: 0, w: self.width, h: self.height }
    }

    /// Recorta a `r`, clampeando al área disponible. Devuelve `None` si la región
    /// queda vacía (fuera de la captura o de tamaño cero tras el clamp).
    pub fn crop(&self, r: Region) -> Option<Shot> {
        let x0 = r.x.min(self.width);
        let y0 = r.y.min(self.height);
        let x1 = (r.x + r.w).min(self.width);
        let y1 = (r.y + r.h).min(self.height);
        if x1 <= x0 || y1 <= y0 {
            return None;
        }
        let (cw, ch) = (x1 - x0, y1 - y0);
        let mut out = vec![0u8; cw as usize * ch as usize * 4];
        let src_stride = self.width as usize * 4;
        let dst_stride = cw as usize * 4;
        for row in 0..ch as usize {
            let sy = y0 as usize + row;
            let so = sy * src_stride + x0 as usize * 4;
            let do_ = row * dst_stride;
            out[do_..do_ + dst_stride].copy_from_slice(&self.rgba[so..so + dst_stride]);
        }
        Some(Shot { width: cw, height: ch, rgba: out })
    }

    /// Codifica la captura a PNG en memoria.
    pub fn to_png(&self) -> Result<Vec<u8>, String> {
        let img = image::RgbaImage::from_raw(self.width, self.height, self.rgba.clone())
            .ok_or_else(|| "buffer RGBA inconsistente con el tamaño".to_string())?;
        let mut buf = std::io::Cursor::new(Vec::new());
        img.write_to(&mut buf, image::ImageFormat::Png)
            .map_err(|e| format!("no se pudo codificar PNG: {e}"))?;
        Ok(buf.into_inner())
    }

    /// Codifica y escribe la captura como PNG en `path`.
    pub fn save_png(&self, path: &Path) -> Result<(), String> {
        let png = self.to_png()?;
        std::fs::write(path, png).map_err(|e| format!("no se pudo escribir «{}»: {e}", path.display()))
    }
}

/// Quién sabe leer los píxeles de la pantalla. Lo implementa el backend Wayland
/// (`zwlr_screencopy`) en el binario; [`MockCapturer`] lo cubre en tests.
pub trait Capturer {
    /// Las salidas (monitores) disponibles.
    fn outputs(&self) -> Result<Vec<OutputInfo>, String>;
    /// Captura una salida por nombre, o la primaria si `output` es `None`.
    fn capture(&self, output: Option<&str>) -> Result<Shot, String>;
}

/// El programa y los argumentos para abrir un PNG en **tullpu** (el editor de
/// imágenes de la suite). tullpu ya abre la ruta que recibe como primer arg, así
/// que el handoff es un `exec` directo — capturás con hapiy, anotás en tullpu.
pub fn tullpu_launch(path: &Path) -> (String, Vec<String>) {
    ("tullpu-app-llimphi".to_string(), vec![path.display().to_string()])
}

/// Nombre de archivo por defecto para una captura, dado un sello (timestamp u
/// otro identificador). Puro y determinista — el binario le pasa la hora real.
pub fn default_filename(stamp: &str) -> String {
    format!("hapiy-{stamp}.png")
}

/// Construye el [`willay_core::Evento`] de una captura ya guardada — la entrada
/// que hapiy emite al centro de eventos. Es **puro** (no toca el socket): el
/// binario lo arma con esto y lo manda con `willay_emit::emitir_silencioso`. El
/// payload referencia el PNG por ruta (federación: el archivo se queda en disco,
/// el índice sólo apunta).
///
/// `display` = conector del monitor capturado (o `None` si fue todo el
/// escritorio); `region` = recorte si lo hubo; `width`/`height` = tamaño del PNG.
pub fn evento_captura(
    path: &Path,
    display: Option<&str>,
    region: Option<Region>,
    width: u32,
    height: u32,
    ts_usec: u64,
) -> willay_core::Evento {
    let titulo = match (region, display) {
        (Some(r), _) => format!("Captura región {}×{}", r.w, r.h),
        (None, Some(name)) => format!("Captura {name} {width}×{height}"),
        (None, None) => format!("Captura escritorio {width}×{height}"),
    };
    let ruta = path.display().to_string();
    willay_core::Evento::nuevo(
        willay_core::Clase::Captura,
        ts_usec,
        "hapiy",
        titulo,
        // El cuerpo (lo que se busca/embebe) lleva la ruta, así una búsqueda por
        // nombre de archivo o carpeta encuentra la captura.
        ruta.clone(),
        willay_core::Payload::Archivo { ruta, mime: "image/png".to_string() },
    )
}

/// Directorio destino por defecto: `~/Pictures` si existe, si no el directorio de
/// imágenes XDG, si no el directorio actual.
pub fn default_dir() -> PathBuf {
    if let Some(u) = directories::UserDirs::new() {
        if let Some(p) = u.picture_dir() {
            if p.is_dir() {
                return p.to_path_buf();
            }
        }
    }
    std::env::current_dir().unwrap_or_else(|_| PathBuf::from("."))
}

/// Un `Capturer` de mentira para tests: devuelve una captura sólida del tamaño y
/// color dados. No toca el sistema.
pub struct MockCapturer {
    pub width: u32,
    pub height: u32,
    pub rgba_pixel: [u8; 4],
    pub output_name: String,
}

impl Default for MockCapturer {
    fn default() -> Self {
        MockCapturer { width: 8, height: 8, rgba_pixel: [10, 20, 30, 255], output_name: "MOCK-1".into() }
    }
}

impl Capturer for MockCapturer {
    fn outputs(&self) -> Result<Vec<OutputInfo>, String> {
        Ok(vec![OutputInfo { name: self.output_name.clone(), width: self.width, height: self.height }])
    }
    fn capture(&self, _output: Option<&str>) -> Result<Shot, String> {
        let rgba = self.rgba_pixel.repeat(self.width as usize * self.height as usize);
        Shot::new(self.width, self.height, rgba)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn ramp_shot(w: u32, h: u32) -> Shot {
        // Cada píxel codifica su (x,y) en R,G para verificar el recorte.
        let mut rgba = Vec::with_capacity((w * h * 4) as usize);
        for y in 0..h {
            for x in 0..w {
                rgba.extend_from_slice(&[x as u8, y as u8, 0, 255]);
            }
        }
        Shot::new(w, h, rgba).unwrap()
    }

    #[test]
    fn new_valida_el_largo() {
        assert!(Shot::new(2, 2, vec![0; 16]).is_ok());
        assert!(Shot::new(2, 2, vec![0; 15]).is_err());
    }

    #[test]
    fn crop_recorta_la_subregion_correcta() {
        let s = ramp_shot(4, 4);
        let c = s.crop(Region { x: 1, y: 2, w: 2, h: 2 }).unwrap();
        assert_eq!((c.width, c.height), (2, 2));
        // Esquina superior-izquierda del recorte = píxel (1,2) del original.
        assert_eq!(&c.rgba[0..4], &[1, 2, 0, 255]);
        // Píxel (1,0) del recorte = (2,2) del original.
        assert_eq!(&c.rgba[4..8], &[2, 2, 0, 255]);
    }

    #[test]
    fn crop_clampa_y_rechaza_vacio() {
        let s = ramp_shot(4, 4);
        // Se pasa de ancho → se clampa a 3 columnas (x 1..4).
        let c = s.crop(Region { x: 1, y: 0, w: 99, h: 1 }).unwrap();
        assert_eq!((c.width, c.height), (3, 1));
        // Fuera de la captura → None.
        assert!(s.crop(Region { x: 10, y: 10, w: 4, h: 4 }).is_none());
    }

    #[test]
    fn png_roundtrip() {
        let s = ramp_shot(6, 5);
        let png = s.to_png().unwrap();
        assert!(png.starts_with(b"\x89PNG\r\n\x1a\n"));
        let back = image::load_from_memory(&png).unwrap().to_rgba8();
        assert_eq!((back.width(), back.height()), (6, 5));
        assert_eq!(back.get_pixel(3, 4).0, [3, 4, 0, 255]);
    }

    #[test]
    fn tullpu_launch_arma_el_exec() {
        let (prog, args) = tullpu_launch(Path::new("/tmp/x.png"));
        assert_eq!(prog, "tullpu-app-llimphi");
        assert_eq!(args, vec!["/tmp/x.png".to_string()]);
    }

    #[test]
    fn default_filename_formatea() {
        assert_eq!(default_filename("20260624-160000"), "hapiy-20260624-160000.png");
    }

    #[test]
    fn evento_captura_arma_titulo_y_payload() {
        use willay_core::{Clase, Payload};
        // Monitor específico.
        let e = evento_captura(Path::new("/p/x.png"), Some("DP-1"), None, 2560, 1440, 100);
        assert_eq!(e.clase, Clase::Captura);
        assert_eq!(e.origen, "hapiy");
        assert_eq!(e.titulo, "Captura DP-1 2560×1440");
        assert_eq!(e.cuerpo, "/p/x.png", "la ruta va al cuerpo para buscar por nombre");
        assert!(matches!(e.payload, Payload::Archivo { mime, .. } if mime == "image/png"));
        // Región: el título la describe.
        let r = evento_captura(Path::new("/p/y.png"), None, Some(Region { x: 0, y: 0, w: 640, h: 480 }), 640, 480, 1);
        assert_eq!(r.titulo, "Captura región 640×480");
        // Escritorio completo.
        let d = evento_captura(Path::new("/p/z.png"), None, None, 1920, 1080, 1);
        assert_eq!(d.titulo, "Captura escritorio 1920×1080");
    }

    #[test]
    fn mock_capturer_da_un_shot() {
        let cap = MockCapturer::default();
        let outs = cap.outputs().unwrap();
        assert_eq!(outs.len(), 1);
        let s = cap.capture(None).unwrap();
        assert_eq!((s.width, s.height), (8, 8));
        assert_eq!(&s.rgba[0..4], &[10, 20, 30, 255]);
    }
}
