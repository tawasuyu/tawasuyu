//! transform — rotación y espejado (flip) del frame de video. V3 de
//! `PARIDAD.md`: el `--video-rotate` / `--vf hflip,vflip` de mpv y la
//! "Rotación / Voltear" de VLC.
//!
//! Calca el molde de [`crate::color`]: un transform puro y testeable
//! ([`Transform`] + [`transform_rgba`]) y un wrapper de [`FrameSource`]
//! ([`TransformVideo`]) gobernado por un [`TransformControl`] compartido,
//! que compone en la cadena de video. Cero dependencias — sólo reubica
//! píxeles del buffer RGBA, así que corre en CI sin GPU.
//!
//! A diferencia del color (per-pixel, in-place), rotar 90°/270°
//! **intercambia ancho y alto**, así que el resultado va a un buffer
//! aparte: el wrapper lleva un scratch y lo intercambia con el buffer de
//! salida (`tick` devuelve las dimensiones nuevas). El surface ya sube
//! texturas de dimensión arbitraria (cada fuente trae la suya), así que un
//! cambio de orientación en vivo se maneja como un cambio de tamaño más.
//!
//! Cadena típica: `<decoder> → ColorVideo → TransformVideo → surface`.

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::FrameSource;

/// Rotación en múltiplos de 90° (sentido horario).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Rotation {
    /// Sin rotar.
    #[default]
    None,
    /// 90° horario.
    Cw90,
    /// 180°.
    Half,
    /// 270° horario (= 90° antihorario).
    Cw270,
}

impl Rotation {
    /// Cicla un paso de 90°: `dir > 0` horario, `dir < 0` antihorario.
    pub fn step(self, dir: i32) -> Rotation {
        use Rotation::*;
        let order = [None, Cw90, Half, Cw270];
        let cur = order.iter().position(|&r| r == self).unwrap_or(0) as i32;
        let next = (cur + dir.signum()).rem_euclid(4) as usize;
        order[next]
    }

    /// Etiqueta humana corta.
    pub fn label(self) -> &'static str {
        match self {
            Rotation::None => "0°",
            Rotation::Cw90 => "90°",
            Rotation::Half => "180°",
            Rotation::Cw270 => "270°",
        }
    }

    fn swaps_dims(self) -> bool {
        matches!(self, Rotation::Cw90 | Rotation::Cw270)
    }
}

/// Orientación del frame: rotación + espejado en cada eje.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Transform {
    pub rotation: Rotation,
    pub flip_h: bool,
    pub flip_v: bool,
}

impl Transform {
    /// `true` si no cambia nada — el wrapper hace bypass (no copia).
    pub fn is_identity(self) -> bool {
        self.rotation == Rotation::None && !self.flip_h && !self.flip_v
    }

    /// Dimensiones de salida para una entrada `(w, h)`: 90°/270°
    /// intercambian ancho y alto.
    pub fn out_dims(self, w: u32, h: u32) -> (u32, u32) {
        if self.rotation.swaps_dims() {
            (h, w)
        } else {
            (w, h)
        }
    }
}

/// Aplica `t` a `src` (RGBA8, `w×h`) y escribe el resultado en `dst`,
/// redimensionándolo. Devuelve las dimensiones de salida. Mapeo forward
/// (cada píxel fuente cae en exactamente uno de salida): primero el flip
/// en espacio de origen, luego la rotación horaria. El alfa viaja con su
/// píxel (se copian los 4 bytes juntos).
pub fn transform_rgba(src: &[u8], w: u32, h: u32, t: Transform, dst: &mut Vec<u8>) -> (u32, u32) {
    let (ow, oh) = t.out_dims(w, h);
    let needed = (ow as usize) * (oh as usize) * 4;
    if dst.len() != needed {
        dst.resize(needed, 0);
    }
    if w == 0 || h == 0 {
        return (ow, oh);
    }
    for sy in 0..h {
        for sx in 0..w {
            // 1) flip en espacio de origen.
            let fx = if t.flip_h { w - 1 - sx } else { sx };
            let fy = if t.flip_v { h - 1 - sy } else { sy };
            // 2) rotación horaria → coords de destino.
            let (dx, dy) = match t.rotation {
                Rotation::None => (fx, fy),
                Rotation::Cw90 => (h - 1 - fy, fx),
                Rotation::Half => (w - 1 - fx, h - 1 - fy),
                Rotation::Cw270 => (fy, w - 1 - fx),
            };
            let si = ((sy * w + sx) * 4) as usize;
            let di = ((dy * ow + dx) * 4) as usize;
            dst[di..di + 4].copy_from_slice(&src[si..si + 4]);
        }
    }
    (ow, oh)
}

// ============================================================
// Control compartido (mismo patrón que ColorControl)
// ============================================================

#[derive(Debug)]
struct TransformShared {
    transform: Transform,
}

/// Handle compartido y barato de clonar para gobernar un
/// [`TransformVideo`] en vivo. El wrapper compara un contador de versión
/// atómico y sólo resincroniza cuando algo cambió.
#[derive(Clone)]
pub struct TransformControl {
    shared: Arc<Mutex<TransformShared>>,
    version: Arc<AtomicU64>,
}

impl Default for TransformControl {
    fn default() -> Self {
        TransformControl::new(Transform::default())
    }
}

impl TransformControl {
    pub fn new(transform: Transform) -> Self {
        TransformControl {
            shared: Arc::new(Mutex::new(TransformShared { transform })),
            version: Arc::new(AtomicU64::new(0)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, TransformShared> {
        match self.shared.lock() {
            Ok(g) => g,
            Err(p) => p.into_inner(),
        }
    }

    fn bump(&self) {
        self.version.fetch_add(1, Ordering::Release);
    }

    pub fn version(&self) -> u64 {
        self.version.load(Ordering::Acquire)
    }

    pub fn transform(&self) -> Transform {
        self.lock().transform
    }

    /// Rota un paso de 90° (`dir > 0` horario, `dir < 0` antihorario).
    pub fn rotate(&self, dir: i32) {
        {
            let mut g = self.lock();
            g.transform.rotation = g.transform.rotation.step(dir);
        }
        self.bump();
    }

    /// Alterna el espejado horizontal.
    pub fn toggle_flip_h(&self) {
        {
            let mut g = self.lock();
            g.transform.flip_h = !g.transform.flip_h;
        }
        self.bump();
    }

    /// Alterna el espejado vertical.
    pub fn toggle_flip_v(&self) {
        {
            let mut g = self.lock();
            g.transform.flip_v = !g.transform.flip_v;
        }
        self.bump();
    }

    /// Vuelve a la orientación original (sin rotar ni espejar).
    pub fn reset(&self) {
        self.lock().transform = Transform::default();
        self.bump();
    }
}

/// Wrapper de [`FrameSource`] que aplica una [`Transform`] gobernada por un
/// [`TransformControl`] compartido. Lee la versión atómica en cada frame y
/// resincroniza si cambió. Con la identidad hace bypass (devuelve el frame
/// del inner sin copiar); si no, transforma a un scratch y lo intercambia
/// con el buffer de salida.
pub struct TransformVideo<S> {
    inner: S,
    control: TransformControl,
    transform: Transform,
    last_version: u64,
    needs_init: bool,
    scratch: Vec<u8>,
}

impl<S> TransformVideo<S> {
    pub fn new(inner: S, control: TransformControl) -> Self {
        let transform = control.transform();
        TransformVideo {
            inner,
            control,
            transform,
            last_version: u64::MAX,
            needs_init: true,
            scratch: Vec::new(),
        }
    }

    pub fn control(&self) -> TransformControl {
        self.control.clone()
    }

    fn sync(&mut self) {
        let v = self.control.version();
        if self.needs_init || v != self.last_version {
            self.transform = self.control.transform();
            self.last_version = v;
            self.needs_init = false;
        }
    }
}

impl<S: FrameSource> FrameSource for TransformVideo<S> {
    fn tick(&mut self, dt: std::time::Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        let (w, h) = self.inner.tick(dt, buf)?;
        self.sync();
        if self.transform.is_identity() {
            return Some((w, h));
        }
        let dims = transform_rgba(buf, w, h, self.transform, &mut self.scratch);
        // `buf` se queda con la salida; `scratch` conserva la capacidad vieja.
        std::mem::swap(buf, &mut self.scratch);
        Some(dims)
    }

    fn pts(&self) -> Option<std::time::Duration> {
        self.inner.pts()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    /// Imagen de grises `w×h` a partir de los valores de cada píxel (en
    /// orden de filas). Alfa = 255.
    fn img(vals: &[u8], w: u32, h: u32) -> Vec<u8> {
        assert_eq!(vals.len(), (w * h) as usize);
        let mut b = Vec::with_capacity(vals.len() * 4);
        for &v in vals {
            b.extend_from_slice(&[v, v, v, 255]);
        }
        b
    }

    /// Extrae los valores del canal R (= gris) en orden de filas.
    fn grays(buf: &[u8]) -> Vec<u8> {
        buf.chunks_exact(4).map(|p| p[0]).collect()
    }

    #[test]
    fn identidad_no_cambia() {
        let src = img(&[1, 2, 3, 4], 2, 2);
        let mut dst = Vec::new();
        let dims = transform_rgba(&src, 2, 2, Transform::default(), &mut dst);
        assert_eq!(dims, (2, 2));
        assert_eq!(grays(&dst), vec![1, 2, 3, 4]);
    }

    #[test]
    fn flip_horizontal_invierte_columnas() {
        // 2x1: [A B] → [B A].
        let src = img(&[10, 20], 2, 1);
        let mut dst = Vec::new();
        let t = Transform { flip_h: true, ..Default::default() };
        let dims = transform_rgba(&src, 2, 1, t, &mut dst);
        assert_eq!(dims, (2, 1));
        assert_eq!(grays(&dst), vec![20, 10]);
    }

    #[test]
    fn flip_vertical_invierte_filas() {
        // 1x2 (una columna): fila0=A, fila1=B → B, A.
        let src = img(&[10, 20], 1, 2);
        let mut dst = Vec::new();
        let t = Transform { flip_v: true, ..Default::default() };
        let dims = transform_rgba(&src, 1, 2, t, &mut dst);
        assert_eq!(dims, (1, 2));
        assert_eq!(grays(&dst), vec![20, 10]);
    }

    #[test]
    fn rotacion_90_horaria_intercambia_dims() {
        // 2x1 [A B] rotado 90° CW → 1x2 con A arriba, B abajo.
        let src = img(&[10, 20], 2, 1);
        let mut dst = Vec::new();
        let t = Transform { rotation: Rotation::Cw90, ..Default::default() };
        let dims = transform_rgba(&src, 2, 1, t, &mut dst);
        assert_eq!(dims, (1, 2));
        assert_eq!(grays(&dst), vec![10, 20]);
    }

    #[test]
    fn rotacion_180_invierte_todo() {
        // 2x2 [[1,2],[3,4]] → [[4,3],[2,1]].
        let src = img(&[1, 2, 3, 4], 2, 2);
        let mut dst = Vec::new();
        let t = Transform { rotation: Rotation::Half, ..Default::default() };
        let dims = transform_rgba(&src, 2, 2, t, &mut dst);
        assert_eq!(dims, (2, 2));
        assert_eq!(grays(&dst), vec![4, 3, 2, 1]);
    }

    #[test]
    fn rotacion_270_es_inversa_de_90() {
        // 2x1 [A B] rotado 270° CW → 1x2 con B arriba, A abajo.
        let src = img(&[10, 20], 2, 1);
        let mut dst = Vec::new();
        let t = Transform { rotation: Rotation::Cw270, ..Default::default() };
        let dims = transform_rgba(&src, 2, 1, t, &mut dst);
        assert_eq!(dims, (1, 2));
        assert_eq!(grays(&dst), vec![20, 10]);
    }

    #[test]
    fn rotacion_90_de_imagen_2x2() {
        // [[1,2],[3,4]] rotado 90° CW → [[3,1],[4,2]].
        let src = img(&[1, 2, 3, 4], 2, 2);
        let mut dst = Vec::new();
        let t = Transform { rotation: Rotation::Cw90, ..Default::default() };
        transform_rgba(&src, 2, 2, t, &mut dst);
        assert_eq!(grays(&dst), vec![3, 1, 4, 2]);
    }

    #[test]
    fn step_cicla_en_ambos_sentidos() {
        assert_eq!(Rotation::None.step(1), Rotation::Cw90);
        assert_eq!(Rotation::Cw270.step(1), Rotation::None);
        assert_eq!(Rotation::None.step(-1), Rotation::Cw270);
    }

    struct Ramp; // 2x1 fijo [10, 20].
    impl FrameSource for Ramp {
        fn tick(&mut self, _dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
            *buf = img(&[10, 20], 2, 1);
            Some((2, 1))
        }
    }

    #[test]
    fn wrapper_bypass_y_rota_en_vivo() {
        let ctrl = TransformControl::default();
        let mut vid = TransformVideo::new(Ramp, ctrl.clone());
        let mut buf = Vec::new();
        // Identidad: dims y bytes intactos.
        assert_eq!(vid.tick(Duration::ZERO, &mut buf), Some((2, 1)));
        assert_eq!(grays(&buf), vec![10, 20]);
        // Rotamos 90° en vivo: el próximo frame sale 1x2.
        ctrl.rotate(1);
        let dims = vid.tick(Duration::ZERO, &mut buf);
        assert_eq!(dims, Some((1, 2)));
        assert_eq!(grays(&buf), vec![10, 20]);
        // Reset vuelve a la identidad.
        ctrl.reset();
        assert_eq!(vid.tick(Duration::ZERO, &mut buf), Some((2, 1)));
    }
}
