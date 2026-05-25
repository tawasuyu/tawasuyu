//! `cosmos_app-web` — cdylib WASM que renderiza la rueda
//! astrológica desde el browser, sin round-trip al server por cada
//! interacción.
//!
//! ## Flujo
//!
//! 1. El cliente JS hace `await fetch('/api/sky')` o
//!    `/api/charts/:id/render?...` y recibe un `RenderModel` JSON.
//! 2. JS llama `render_model_to_svg(json)` (exportado desde WASM) que
//!    deserializa + corre `cosmos_render::compose_wheel` +
//!    serializa SVG.
//! 3. JS hace `wheelContainer.innerHTML = svg`.
//!
//! ## Build
//!
//! ```bash
//! cargo install wasm-pack          # una vez
//! cd crates/modules/cosmos_app/cosmos_app-web
//! wasm-pack build --target web --out-dir ../../../../apps/cosmos_app-server/static/wasm
//! ```
//!
//! Esto produce un módulo ES6 (`cosmos_web.js` +
//! `cosmobiologia_web_bg.wasm`) que el `index.html` del server
//! importa con `import init, { render_model_to_svg } from
//! '/static/wasm/cosmos_web.js';`.

#![forbid(unsafe_code)]
#![warn(rust_2018_idioms)]

// La API pública SOLO se expone con `wasm-bindgen` en target
// wasm32. En nativo (rlib) el crate compila para validar la
// signature pero no exporta nada — los tests del render ya viven
// en `cosmos_app-render::math`.
#[cfg(target_arch = "wasm32")]
mod wasm {
    use cosmos_render::{
        compose_wheel, draw_commands_to_svg, CompositionOpts, Palette, RenderModel,
    };
    use wasm_bindgen::prelude::*;

    /// Renderea un `RenderModel` (JSON string) como SVG. El JSON sale
    /// de `/api/sky` o `/api/charts/:id/render` del server.
    ///
    /// `size` es el lado del cuadrado contenedor en px (default 600).
    /// `rot_offset_deg` permite rotar la vista (jog-dial / preview).
    #[wasm_bindgen]
    pub fn render_model_to_svg(
        json: &str,
        size: f32,
        rot_offset_deg: f32,
    ) -> Result<String, JsValue> {
        render_with_opts(json, size, rot_offset_deg, true)
    }

    /// Variante con palette explícita (dark = `true` por default, light
    /// = `false`). El JS pasa el modo según preferencia/tema del UA.
    #[wasm_bindgen]
    pub fn render_model_to_svg_themed(
        json: &str,
        size: f32,
        rot_offset_deg: f32,
        dark: bool,
    ) -> Result<String, JsValue> {
        render_with_opts(json, size, rot_offset_deg, dark)
    }

    fn render_with_opts(
        json: &str,
        size: f32,
        rot_offset_deg: f32,
        dark: bool,
    ) -> Result<String, JsValue> {
        let model: RenderModel = serde_json::from_str(json)
            .map_err(|e| JsValue::from_str(&format!("parse RenderModel: {}", e)))?;
        let opts = CompositionOpts {
            size: if size > 0.0 { size } else { 600.0 },
            rot_offset_deg,
            palette: if dark { Palette::dark() } else { Palette::light() },
            ..Default::default()
        };
        let cmds = compose_wheel(&model, &opts);
        Ok(draw_commands_to_svg(&cmds, opts.size))
    }

    /// Hook de inicialización opcional — wasm_pack lo invoca al
    /// cargar el módulo. Útil para instalar un panic hook hacia
    /// `console.error`. Por ahora no-op.
    #[wasm_bindgen(start)]
    pub fn main_js() {}
}

#[cfg(not(target_arch = "wasm32"))]
pub fn _native_marker() {
    // Sin target wasm32, el crate solo expone el render como
    // transitivo. Esta función vive para que `cargo check -p
    // cosmos_app-web` valide la compilación nativa sin
    // wasm-bindgen — útil en CI y en desarrollo desktop.
    let _ = std::any::type_name::<cosmos_render::RenderModel>();
}
