//! llimphi-hal — Puente al Silicio.
//!
//! Aísla el motor del sistema operativo. Pinta en ventana Wayland (vía
//! mirada) o framebuffer directo (vía wawa). Trait `Surface` abstracto.
//!
//! Fase 1: pendiente — `wgpu` + `winit` + ventana gris plomo a 144 Hz.

/// Superficie gráfica donde llimphi pinta. Implementaciones esperadas:
/// `WinitSurface` (dev en Linux) y `WawaFramebufferSurface` (bare metal).
pub trait Surface {
    fn width(&self) -> u32;
    fn height(&self) -> u32;
    fn present(&mut self);
}
