//! Visualizador de audio (estilo CAVA) para el cava del lock.
//!
//! Espejo de `pata-llimphi/src/cava.rs`: corremos el binario **`cava`** en modo
//! *raw* (un servicio público del sistema) y leemos sus cuadros desde un hilo.
//! cava escribe a stdout una línea por cuadro con una columna por banda
//! (`v;v;…;`), valores `0..max`; el hilo los normaliza a `0..1` y publica el
//! último por un canal. El lock las pinta como barras con gradiente.
//!
//! Si `cava` no está instalado, el hilo termina en silencio y el cuadro queda
//! vacío —degrada sin romper, igual que en pata—.

use std::io::{BufRead, BufReader};
use std::sync::mpsc::{channel, Receiver};

use llimphi_ui::llimphi_raster::vello::Scene;
use llimphi_ui::PaintRect;
use llimphi_ui::llimphi_raster::peniko::Color;

/// El rango máximo que pedimos a cava (`ascii_max_range`); normalizamos contra
/// él para sacar la fracción `0..1`.
const MAX_RANGE: f32 = 100.0;

/// El visualizador corriendo en su propio hilo. Publica el último cuadro (una
/// barra por banda) por un canal; el frontend lo drena por frame.
pub struct CavaHandle {
    rx: Receiver<Vec<f32>>,
}

impl CavaHandle {
    /// Arranca cava con `bars` bandas en modo raw ascii y lanza el hilo lector.
    /// Si cava no arranca, el canal queda vacío (el cava se ve plano).
    pub fn spawn(bars: u32) -> Self {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("greeter-cava".into())
            .spawn(move || {
                if let Err(_e) = correr_cava(bars, tx) {
                    // cava ausente o caído: el hilo muere y latest() da None.
                }
            })
            .ok();
        Self { rx }
    }

    /// El último cuadro (drena la cola), o `None` si no llegó nada nuevo. No
    /// bloquea.
    pub fn latest(&self) -> Option<Vec<f32>> {
        let mut last = None;
        while let Ok(frame) = self.rx.try_recv() {
            last = Some(frame);
        }
        last
    }
}

/// Escribe el config raw, lanza `cava -p <config>` y bombea sus cuadros por `tx`
/// hasta que el proceso muere o el receptor se va.
fn correr_cava(bars: u32, tx: std::sync::mpsc::Sender<Vec<f32>>) -> std::io::Result<()> {
    let conf = config_path();
    std::fs::write(&conf, config_raw(bars))?;

    let mut child = std::process::Command::new("cava")
        .arg("-p")
        .arg(&conf)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()?;

    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| std::io::Error::other("cava sin stdout"))?;
    let reader = BufReader::new(stdout);
    for line in reader.lines() {
        let Ok(line) = line else { break };
        let frame = parse_frame(&line);
        if !frame.is_empty() && tx.send(frame).is_err() {
            break; // el frontend se fue
        }
    }
    let _ = child.kill();
    let _ = child.wait();
    Ok(())
}

/// Parsea una línea de cava raw ascii (`"12;40;0;7;"`) a fracciones `0..1`.
fn parse_frame(line: &str) -> Vec<f32> {
    line.split(';')
        .filter(|t| !t.trim().is_empty())
        .filter_map(|t| t.trim().parse::<f32>().ok())
        .map(|v| (v / MAX_RANGE).clamp(0.0, 1.0))
        .collect()
}

/// El config de cava en modo raw ascii mono, una columna por banda.
fn config_raw(bars: u32) -> String {
    format!(
        "[general]\n\
         bars = {bars}\n\
         framerate = 30\n\
         \n\
         [output]\n\
         method = raw\n\
         raw_target = /dev/stdout\n\
         data_format = ascii\n\
         ascii_max_range = {max}\n\
         channels = mono\n",
        max = MAX_RANGE as u32,
    )
}

/// Ruta del config temporal de cava. Nombre propio para no pisar el de pata
/// (ambos pueden correr cava a la vez: el panel abajo y el lock encima).
fn config_path() -> std::path::PathBuf {
    std::env::temp_dir().join("mirada-greeter-cava.conf")
}

/// Pinta `bars` (fracciones `0..1`) como barras con gradiente verde→ámbar dentro
/// de `rect`. Espejo de `dibujar_cava` de pata. Cuadro vacío ⇒ no pinta nada.
pub fn paint(scene: &mut Scene, rect: PaintRect, bars: &[f32]) {
    use llimphi_ui::llimphi_raster::kurbo::{Affine, Point, RoundedRect};
    use llimphi_ui::llimphi_raster::peniko::{Fill, Gradient};
    let n = bars.len();
    if n == 0 || rect.w <= 0.0 || rect.h <= 0.0 {
        return;
    }
    let (x, y, w, h) = (rect.x as f64, rect.y as f64, rect.w as f64, rect.h as f64);
    let gap = 2.0_f64;
    let bw = ((w - gap * (n as f64 - 1.0)) / n as f64).max(1.0);
    for (i, &v) in bars.iter().enumerate() {
        let v = v.clamp(0.0, 1.0);
        let bh = (v as f64 * h).max(2.0);
        let bx = x + i as f64 * (bw + gap);
        let by = y + h - bh;
        let rr = RoundedRect::new(bx, by, bx + bw, y + h, 1.5);
        let lo = hsv(150.0, 0.55, 0.45);
        let hi = hsv(150.0 * (1.0 - v), 0.85, 0.98);
        let g = Gradient::new_linear(Point::new(bx, y + h), Point::new(bx, by))
            .with_stops([lo, hi].as_slice());
        scene.fill(Fill::NonZero, Affine::IDENTITY, &g, None, &rr);
    }
}

/// HSV → color opaco. Copia del helper de pata (`render::widgets::hsv`).
fn hsv(h: f32, s: f32, v: f32) -> Color {
    use llimphi_ui::llimphi_raster::peniko::color::AlphaColor;
    let h = h.rem_euclid(360.0);
    let c = v * s;
    let xx = c * (1.0 - ((h / 60.0) % 2.0 - 1.0).abs());
    let m = v - c;
    let (r, g, b) = match (h / 60.0) as u32 {
        0 => (c, xx, 0.0),
        1 => (xx, c, 0.0),
        2 => (0.0, c, xx),
        3 => (0.0, xx, c),
        4 => (xx, 0.0, c),
        _ => (c, 0.0, xx),
    };
    AlphaColor::new([r + m, g + m, b + m, 1.0])
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsea_cuadro_ascii() {
        assert_eq!(parse_frame("100;50;0;"), vec![1.0, 0.5, 0.0]);
    }

    #[test]
    fn cuadro_vacio_o_basura() {
        assert!(parse_frame("").is_empty());
        assert!(parse_frame(";;;").is_empty());
    }

    #[test]
    fn clampa_fuera_de_rango() {
        assert_eq!(parse_frame("250;"), vec![1.0]);
    }

    #[test]
    fn config_lleva_las_bandas() {
        let c = config_raw(24);
        assert!(c.contains("bars = 24"));
        assert!(c.contains("method = raw"));
    }
}
