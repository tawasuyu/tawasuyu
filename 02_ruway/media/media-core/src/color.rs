//! color — ajustes de color de video (brillo / contraste / gamma /
//! saturación) como procesador por-frame. V4 de `PARIDAD.md`: lo que VLC
//! trae en "Efectos y filtros → Ajustes de imagen" y mpv en
//! `--brightness/--contrast/--gamma/--saturation`.
//!
//! Calca el molde del ecualizador ([`crate::eq`]): un procesador puro y
//! testeable ([`ColorAdjust`]) y un wrapper de [`FrameSource`]
//! ([`ColorVideo`]) gobernado por un [`ColorControl`] compartido, que
//! compone en la cadena de video igual que `PausableVideo`. Cero
//! dependencias — sólo aritmética `f32` sobre el buffer RGBA, así que
//! corre en CI sin GPU.
//!
//! Cadena típica de video:
//!
//! ```text
//! <decoder> → ColorVideo → PausableVideo → surface
//! ```
//!
//! ## Orden de las operaciones
//!
//! Por píxel, sobre canales normalizados `0..1` (el alfa no se toca):
//!
//! 1. **Contraste** alrededor del gris medio: `(c - 0.5) * contrast + 0.5`.
//! 2. **Brillo** como offset aditivo: `+ brightness`.
//! 3. **Saturación** mezclando contra la luma Rec.709:
//!    `luma + saturation * (c - luma)`.
//! 4. **Matiz** (hue): rotación del vector cromático en espacio YIQ por un
//!    ángulo en grados (lo que hace el control "hue" de NTSC y `--hue` de
//!    mpv). Con ángulo 0 el round-trip YIQ→RGB es la identidad.
//! 5. **Gamma** como potencia: `c^(1/gamma)`.
//!
//! Cada parámetro tiene su identidad (contraste 1, brillo 0, saturación 1,
//! matiz 0, gamma 1); con todos en identidad el wrapper hace **bypass real**
//! (no recorre el buffer).

use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use crate::FrameSource;

/// Coeficientes de luma Rec.709 (los mismos que usa el video HD).
const LUMA_R: f32 = 0.2126;
const LUMA_G: f32 = 0.7152;
const LUMA_B: f32 = 0.0722;

/// Rota el matiz de un píxel `rgb` (canales `0..1`) por un ángulo dado vía
/// su `cos`/`sin` ya calculados. Convierte a YIQ (NTSC), gira el plano
/// cromático `(I, Q)` por el ángulo y vuelve a RGB. Con `cos=1, sin=0`
/// (ángulo 0) el round-trip es la identidad, por lo que el caller se lo
/// saltea. La luma `Y` no se toca: rotar el matiz no cambia el brillo.
fn rotate_hue(rgb: [f32; 3], cos: f32, sin: f32) -> [f32; 3] {
    let [r, g, b] = rgb;
    // RGB → YIQ.
    let y = 0.299 * r + 0.587 * g + 0.114 * b;
    let i = 0.595_716 * r - 0.274_453 * g - 0.321_263 * b;
    let q = 0.211_456 * r - 0.522_591 * g + 0.311_135 * b;
    // Giro del vector cromático.
    let i2 = i * cos - q * sin;
    let q2 = i * sin + q * cos;
    // YIQ → RGB.
    [
        y + 0.955_69 * i2 + 0.619_86 * q2,
        y - 0.272_122 * i2 - 0.647_368 * q2,
        y - 1.106_890 * i2 + 1.704_614 * q2,
    ]
}

/// Parámetros de ajuste de color. La identidad ([`Default`]) deja la
/// imagen intacta. Los rangos son sugerencias para la UI; [`ColorAdjust`]
/// clampea la salida a `0..1` de todos modos.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ColorParams {
    /// Offset de brillo, sumado tras el contraste. `0.0` = sin cambio;
    /// rango útil `-1.0..1.0`.
    pub brightness: f32,
    /// Escala de contraste alrededor del gris medio. `1.0` = sin cambio,
    /// `0.0` = gris plano; rango útil `0.0..4.0`.
    pub contrast: f32,
    /// Gamma de salida (`c^(1/gamma)`). `1.0` = sin cambio, `>1` aclara
    /// medios tonos, `<1` los oscurece; rango útil `0.1..5.0`.
    pub gamma: f32,
    /// Saturación: mezcla entre escala de grises (`0.0`) y la imagen
    /// (`1.0`); `>1` sobresatura. Rango útil `0.0..4.0`.
    pub saturation: f32,
    /// Rotación de matiz en **grados**. `0.0` = sin cambio; rango útil
    /// `-180.0..180.0` (envuelve). Rota el vector cromático en YIQ.
    pub hue: f32,
}

impl Default for ColorParams {
    fn default() -> Self {
        ColorParams {
            brightness: 0.0,
            contrast: 1.0,
            gamma: 1.0,
            saturation: 1.0,
            hue: 0.0,
        }
    }
}

impl ColorParams {
    /// `true` si todos los parámetros están en su identidad — el wrapper lo
    /// usa para saltarse el procesado entero (bypass sin costo).
    pub fn is_identity(&self) -> bool {
        self.brightness == 0.0
            && self.contrast == 1.0
            && self.gamma == 1.0
            && self.saturation == 1.0
            && self.hue == 0.0
    }
}

/// Procesador puro: aplica unos [`ColorParams`] a un buffer RGBA in-place.
/// Sin estado entre frames (cada píxel es independiente), así que es
/// trivialmente testeable y paralelizable.
#[derive(Debug, Clone, Copy, Default)]
pub struct ColorAdjust {
    params: ColorParams,
}

impl ColorAdjust {
    pub fn new(params: ColorParams) -> Self {
        ColorAdjust { params }
    }

    pub fn params(&self) -> ColorParams {
        self.params
    }

    pub fn set_params(&mut self, params: ColorParams) {
        self.params = params;
    }

    /// Procesa `buf` (RGBA8 intercalado) in-place. No-op si los parámetros
    /// son la identidad. El alfa (cada 4º byte) se preserva.
    pub fn process(&self, buf: &mut [u8]) {
        if self.params.is_identity() {
            return;
        }
        let ColorParams {
            brightness,
            contrast,
            gamma,
            saturation,
            hue,
        } = self.params;
        // Gamma 0 sería división por cero; clampeamos a un mínimo sano.
        let inv_gamma = 1.0 / gamma.max(1e-3);
        // Pre-cómputo del coseno/seno del ángulo de matiz (una vez por frame).
        let hue_rot = (hue != 0.0).then(|| {
            let rad = hue.to_radians();
            (rad.cos(), rad.sin())
        });

        for px in buf.chunks_exact_mut(4) {
            let mut rgb = [
                px[0] as f32 / 255.0,
                px[1] as f32 / 255.0,
                px[2] as f32 / 255.0,
            ];
            // 1) contraste alrededor de 0.5, 2) brillo.
            for c in &mut rgb {
                *c = (*c - 0.5) * contrast + 0.5 + brightness;
            }
            // 3) saturación contra la luma Rec.709.
            if saturation != 1.0 {
                let luma = LUMA_R * rgb[0] + LUMA_G * rgb[1] + LUMA_B * rgb[2];
                for c in &mut rgb {
                    *c = luma + saturation * (*c - luma);
                }
            }
            // 4) matiz: rotación del vector cromático en YIQ.
            if let Some((cos, sin)) = hue_rot {
                rgb = rotate_hue(rgb, cos, sin);
            }
            // 5) gamma + clamp final → u8.
            for (i, c) in rgb.iter().enumerate() {
                let v = c.clamp(0.0, 1.0).powf(inv_gamma);
                px[i] = (v * 255.0 + 0.5) as u8;
            }
        }
    }
}

// ============================================================
// Control compartido (mismo patrón que EqControl)
// ============================================================

#[derive(Debug)]
struct ColorShared {
    params: ColorParams,
    enabled: bool,
}

/// Handle compartido y barato de clonar (sólo `Arc`s) para gobernar un
/// [`ColorVideo`] en vivo desde la UI. El wrapper compara un contador de
/// versión atómico y sólo re-sincroniza cuando algo cambió.
#[derive(Clone)]
pub struct ColorControl {
    shared: Arc<Mutex<ColorShared>>,
    version: Arc<AtomicU64>,
}

impl Default for ColorControl {
    fn default() -> Self {
        ColorControl::new(ColorParams::default())
    }
}

impl ColorControl {
    pub fn new(params: ColorParams) -> Self {
        ColorControl {
            shared: Arc::new(Mutex::new(ColorShared {
                params,
                enabled: true,
            })),
            version: Arc::new(AtomicU64::new(0)),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, ColorShared> {
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

    pub fn params(&self) -> ColorParams {
        self.lock().params
    }

    pub fn set_params(&self, params: ColorParams) {
        self.lock().params = params;
        self.bump();
    }

    /// Suma `delta` al brillo (clampea a `-1.0..1.0`).
    pub fn add_brightness(&self, delta: f32) {
        {
            let mut g = self.lock();
            g.params.brightness = (g.params.brightness + delta).clamp(-1.0, 1.0);
        }
        self.bump();
    }

    /// Multiplica/ajusta contraste sumando `delta` (clampea a `0.0..4.0`).
    pub fn add_contrast(&self, delta: f32) {
        {
            let mut g = self.lock();
            g.params.contrast = (g.params.contrast + delta).clamp(0.0, 4.0);
        }
        self.bump();
    }

    /// Suma `delta` a la gamma (clampea a `0.1..5.0`).
    pub fn add_gamma(&self, delta: f32) {
        {
            let mut g = self.lock();
            g.params.gamma = (g.params.gamma + delta).clamp(0.1, 5.0);
        }
        self.bump();
    }

    /// Suma `delta` a la saturación (clampea a `0.0..4.0`).
    pub fn add_saturation(&self, delta: f32) {
        {
            let mut g = self.lock();
            g.params.saturation = (g.params.saturation + delta).clamp(0.0, 4.0);
        }
        self.bump();
    }

    /// Suma `delta` grados al matiz, envolviendo a `-180.0..180.0`.
    pub fn add_hue(&self, delta: f32) {
        {
            let mut g = self.lock();
            // Normaliza a (-180, 180] para que girar de a poco no se trabe
            // en los topes (a diferencia de un clamp duro).
            let h = (g.params.hue + delta + 180.0).rem_euclid(360.0) - 180.0;
            g.params.hue = h;
        }
        self.bump();
    }

    /// Vuelve todos los parámetros a la identidad.
    pub fn reset(&self) {
        self.lock().params = ColorParams::default();
        self.bump();
    }

    pub fn set_enabled(&self, enabled: bool) {
        self.lock().enabled = enabled;
        self.bump();
    }

    pub fn is_enabled(&self) -> bool {
        self.lock().enabled
    }
}

/// Wrapper de [`FrameSource`] que aplica un [`ColorAdjust`] gobernado por un
/// [`ColorControl`] compartido. Lee la versión atómica en cada frame; si
/// cambió (o es la primera vez), resincroniza los parámetros y el on/off.
/// El camino común (sin cambios) es lock-free; con todo en identidad o
/// deshabilitado no recorre el buffer.
pub struct ColorVideo<S> {
    inner: S,
    control: ColorControl,
    adjust: ColorAdjust,
    last_version: u64,
    enabled: bool,
    needs_init: bool,
}

impl<S> ColorVideo<S> {
    pub fn new(inner: S, control: ColorControl) -> Self {
        let adjust = ColorAdjust::new(control.params());
        let enabled = control.is_enabled();
        ColorVideo {
            inner,
            control,
            adjust,
            last_version: u64::MAX,
            enabled,
            needs_init: true,
        }
    }

    pub fn control(&self) -> ColorControl {
        self.control.clone()
    }

    fn sync(&mut self) {
        let v = self.control.version();
        if self.needs_init || v != self.last_version {
            self.adjust.set_params(self.control.params());
            self.enabled = self.control.is_enabled();
            self.last_version = v;
            self.needs_init = false;
        }
    }
}

impl<S: FrameSource> FrameSource for ColorVideo<S> {
    fn tick(&mut self, dt: std::time::Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
        let dims = self.inner.tick(dt, buf)?;
        self.sync();
        if self.enabled {
            self.adjust.process(buf);
        }
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

    fn px(r: u8, g: u8, b: u8) -> Vec<u8> {
        vec![r, g, b, 255]
    }

    #[test]
    fn identidad_no_toca_el_buffer() {
        let adj = ColorAdjust::default();
        let mut buf = px(10, 128, 250);
        adj.process(&mut buf);
        assert_eq!(buf, px(10, 128, 250));
        assert!(ColorParams::default().is_identity());
    }

    #[test]
    fn brillo_aclara_y_preserva_alfa() {
        let adj = ColorAdjust::new(ColorParams {
            brightness: 0.5,
            ..Default::default()
        });
        let mut buf = vec![100, 100, 100, 77];
        adj.process(&mut buf);
        // +0.5 sobre 100/255≈0.392 → ≈0.892 → ≈227.
        assert!(buf[0] > 220 && buf[0] < 234, "fue {}", buf[0]);
        // El alfa no se toca.
        assert_eq!(buf[3], 77);
    }

    #[test]
    fn brillo_clampea_sin_desbordar() {
        let adj = ColorAdjust::new(ColorParams {
            brightness: 1.0,
            ..Default::default()
        });
        let mut buf = px(200, 200, 200);
        adj.process(&mut buf);
        assert_eq!(&buf[0..3], &[255, 255, 255]);
    }

    #[test]
    fn saturacion_cero_da_gris_de_luma() {
        let adj = ColorAdjust::new(ColorParams {
            saturation: 0.0,
            ..Default::default()
        });
        // Rojo puro → luma Rec.709 = 0.2126 → ≈54 en los tres canales.
        let mut buf = px(255, 0, 0);
        adj.process(&mut buf);
        let l = (LUMA_R * 255.0 + 0.5) as u8;
        assert_eq!(buf[0], buf[1]);
        assert_eq!(buf[1], buf[2]);
        assert!((buf[0] as i32 - l as i32).abs() <= 1, "luma {} vs {}", buf[0], l);
    }

    #[test]
    fn contraste_aleja_del_gris_medio() {
        let adj = ColorAdjust::new(ColorParams {
            contrast: 2.0,
            ..Default::default()
        });
        // Un valor por encima de 0.5 sube; uno por debajo baja.
        let mut alto = px(200, 200, 200);
        adj.process(&mut alto);
        assert!(alto[0] > 200);
        let mut bajo = px(50, 50, 50);
        adj.process(&mut bajo);
        assert!(bajo[0] < 50);
    }

    #[test]
    fn gamma_mayor_aclara_medios_tonos() {
        let adj = ColorAdjust::new(ColorParams {
            gamma: 2.0,
            ..Default::default()
        });
        let mut buf = px(128, 128, 128);
        adj.process(&mut buf);
        // 0.5^(1/2) ≈ 0.707 → ≈180, más claro que 128.
        assert!(buf[0] > 170 && buf[0] < 190, "fue {}", buf[0]);
    }

    #[test]
    fn control_clampea_a_rangos() {
        let c = ColorControl::default();
        c.add_brightness(5.0);
        assert_eq!(c.params().brightness, 1.0);
        c.add_brightness(-9.0);
        assert_eq!(c.params().brightness, -1.0);
        c.add_contrast(-9.0);
        assert_eq!(c.params().contrast, 0.0);
        c.add_gamma(99.0);
        assert_eq!(c.params().gamma, 5.0);
        c.add_saturation(99.0);
        assert_eq!(c.params().saturation, 4.0);
        c.reset();
        assert!(c.params().is_identity());
    }

    #[test]
    fn matiz_cero_es_identidad() {
        let adj = ColorAdjust::new(ColorParams {
            hue: 0.0,
            ..Default::default()
        });
        assert!(adj.params().is_identity());
        let mut buf = px(200, 64, 30);
        adj.process(&mut buf);
        // hue=0 ⇒ bypass exacto.
        assert_eq!(&buf[0..3], &[200, 64, 30]);
    }

    #[test]
    fn matiz_no_toca_grises() {
        // Un gris (R=G=B) no tiene croma: rotar el matiz no debe cambiarlo.
        let adj = ColorAdjust::new(ColorParams {
            hue: 90.0,
            ..Default::default()
        });
        let mut buf = px(120, 120, 120);
        adj.process(&mut buf);
        for c in &buf[0..3] {
            assert!((*c as i32 - 120).abs() <= 1, "gris derivó a {}", c);
        }
    }

    #[test]
    fn matiz_preserva_la_luma() {
        // Rotar el matiz conserva la luma NTSC (`Y`). Lo verificamos sobre el
        // helper puro para que el clamp a gamut + el redondeo a u8 no
        // contaminen la igualdad (un color saturado puede salirse del cubo
        // RGB tras rotar, y ahí sí cambia la luma del píxel final).
        let luma = |[r, g, b]: [f32; 3]| 0.299 * r + 0.587 * g + 0.114 * b;
        let rgb = [0.6, 0.3, 0.45];
        for deg in [30.0f32, 120.0, -90.0, 200.0] {
            let rad = deg.to_radians();
            let out = rotate_hue(rgb, rad.cos(), rad.sin());
            assert!(
                (luma(rgb) - luma(out)).abs() < 1e-4,
                "deg {deg}: luma {} → {}",
                luma(rgb),
                luma(out)
            );
        }
    }

    #[test]
    fn matiz_mueve_el_color() {
        // Un color en gamut tras rotar debe cambiar (no es bypass).
        let adj = ColorAdjust::new(ColorParams {
            hue: 120.0,
            ..Default::default()
        });
        let mut buf = px(150, 110, 130);
        adj.process(&mut buf);
        assert!(buf[0..3] != [150, 110, 130], "no movió el color");
    }

    #[test]
    fn control_matiz_envuelve() {
        let c = ColorControl::default();
        c.add_hue(200.0);
        // 200 envuelve a -160 (rango (-180, 180]).
        assert!((c.params().hue - (-160.0)).abs() < 1e-3, "fue {}", c.params().hue);
        c.add_hue(-100.0);
        // -260 envuelve a 100.
        assert!((c.params().hue - 100.0).abs() < 1e-3, "fue {}", c.params().hue);
    }

    struct Solid(u8);
    impl FrameSource for Solid {
        fn tick(&mut self, _dt: Duration, buf: &mut Vec<u8>) -> Option<(u32, u32)> {
            *buf = vec![self.0, self.0, self.0, 255];
            Some((1, 1))
        }
    }

    #[test]
    fn wrapper_bypass_y_aplica_en_vivo() {
        let ctrl = ColorControl::default();
        let mut vid = ColorVideo::new(Solid(100), ctrl.clone());
        let mut buf = Vec::new();
        // Identidad: pasa sin tocar.
        assert_eq!(vid.tick(Duration::ZERO, &mut buf), Some((1, 1)));
        assert_eq!(buf[0], 100);
        // Subimos brillo en vivo: el próximo frame ya viene más claro.
        ctrl.add_brightness(0.5);
        vid.tick(Duration::ZERO, &mut buf);
        assert!(buf[0] > 200, "fue {}", buf[0]);
        // Bypass por enabled=false aunque haya parámetros no-identidad.
        ctrl.set_enabled(false);
        vid.tick(Duration::ZERO, &mut buf);
        assert_eq!(buf[0], 100);
    }
}
