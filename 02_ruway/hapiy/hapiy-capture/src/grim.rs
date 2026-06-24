//! Backend de captura **grim**: delega en el binario `grim` (cliente
//! `zwlr_screencopy` de wlroots que mirada ya permite). Fiable y mínimo; es el
//! camino por defecto mientras el backend nativo (`wayland.rs`) se verifica.

use hapiy_core::{Capturer, OutputInfo, Shot};
use std::process::Command;

/// Captura corriendo `grim -` (PNG a stdout) y lo decodifica a un [`Shot`].
pub struct GrimCapturer;

impl Capturer for GrimCapturer {
    fn outputs(&self) -> Result<Vec<OutputInfo>, String> {
        Err("grim no enumera salidas; pasá el nombre con --display o usá el backend nativo".into())
    }

    fn capture(&self, output: Option<&str>) -> Result<Shot, String> {
        let mut cmd = Command::new("grim");
        if let Some(o) = output {
            cmd.arg("-o").arg(o);
        }
        cmd.arg("-"); // PNG a stdout
        let out = cmd
            .output()
            .map_err(|e| format!("no se pudo ejecutar grim ({e}); instalalo o usá --backend native"))?;
        if !out.status.success() {
            return Err(format!("grim falló: {}", String::from_utf8_lossy(&out.stderr).trim()));
        }
        let img = image::load_from_memory(&out.stdout)
            .map_err(|e| format!("grim no devolvió una imagen válida: {e}"))?
            .to_rgba8();
        Shot::new(img.width(), img.height(), img.into_raw())
    }
}
