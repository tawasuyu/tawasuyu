//! `llimphi-widget-avatar` — círculo de identidad con inicial.
//!
//! Genera un avatar **determinista** de un nombre: el color de fondo
//! viene de un hash del nombre, mapeado a una paleta limitada de 8
//! tonos (para que dos usuarios distintos no acaben con colores que
//! se confundan). La inicial es la primera letra del nombre (uppercase),
//! pintada centrada en blanco-cálido.
//!
//! Útil para chats (ayni), authorship en pluma, presencia en
//! herramientas colaborativas. Una sola función — sin state, sin
//! animación, sin paleta configurable (la consistencia importa más
//! que la personalización).

#![forbid(unsafe_code)]

use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, Size, Style},
    AlignItems, JustifyContent,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::llimphi_text::Alignment;
use llimphi_ui::View;

/// Construye el avatar de `name` con diámetro `size_px`.
pub fn avatar_view<Msg: Clone + 'static>(name: &str, size_px: f32) -> View<Msg> {
    let bg = color_for(name);
    let initial = name
        .chars()
        .next()
        .map(|c| c.to_uppercase().next().unwrap_or(c))
        .unwrap_or('·');
    let fg = Color::from_rgba8(248, 248, 250, 255);
    let font = (size_px * 0.42).max(8.0);

    View::new(Style {
        size: Size {
            width: length(size_px),
            height: length(size_px),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .fill(bg)
    .radius((size_px * 0.5) as f64)
    .paint_with(move |scene, _ts, rect| {
        // Highlight radial en el cuadrante superior — el avatar se lee
        // como esfera. paint_with corre entre el fill y la inicial, así
        // que la luz se suma al color del nombre sin tapar el texto.
        // Mismo patrón dot-badge / switch-thumb (P6/P7).
        use llimphi_ui::llimphi_raster::kurbo::{Affine, Circle};
        use llimphi_ui::llimphi_raster::peniko::Fill;
        if rect.w <= 0.0 || rect.h <= 0.0 {
            return;
        }
        let cx = (rect.x + rect.w * 0.5) as f64;
        let cy = (rect.y + rect.h * 0.30) as f64;
        let r = (rect.w as f64 * 0.18).max(1.0);
        let highlight = Color::from_rgba8(255, 255, 255, 60);
        scene.fill(
            Fill::NonZero,
            Affine::IDENTITY,
            highlight,
            None,
            &Circle::new((cx, cy), r),
        );
    })
    .text_aligned(initial.to_string(), font, fg, Alignment::Center)
}

/// Paleta tonal limitada — 8 colores HSL-ish elegidos para destacar
/// sobre fondos oscuros sin ser estridentes.
const PALETTE: &[Color] = &[
    Color::from_rgba8(96, 130, 220, 255),  // azul
    Color::from_rgba8(110, 180, 130, 255), // verde aurora
    Color::from_rgba8(220, 140, 80, 255),  // naranja sunset
    Color::from_rgba8(160, 110, 220, 255), // púrpura
    Color::from_rgba8(80, 180, 180, 255),  // aqua
    Color::from_rgba8(220, 120, 160, 255), // rosa
    Color::from_rgba8(180, 170, 90, 255),  // mostaza
    Color::from_rgba8(130, 150, 175, 255), // gris-azul
];

/// Hash FNV-1a simple sobre los bytes del nombre, mod paleta. No
/// requiere crypto — sólo necesitamos que mismo input dé mismo color.
fn color_for(name: &str) -> Color {
    let mut h: u32 = 0x811c9dc5;
    for b in name.bytes() {
        h ^= b as u32;
        h = h.wrapping_mul(0x01000193);
    }
    PALETTE[(h as usize) % PALETTE.len()]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn color_for_is_deterministic() {
        assert_eq!(color_for("sergio").components, color_for("sergio").components);
        assert_eq!(color_for("calcetin").components, color_for("calcetin").components);
    }

    #[test]
    fn different_names_can_have_different_colors() {
        let names = ["a", "b", "c", "d", "e", "f", "g", "h"];
        let colors: Vec<_> = names.iter().map(|n| color_for(n)).collect();
        // Al menos 2 colores distintos en 8 nombres — el hash es trivial,
        // colisiones esperadas, no garantizamos 8 distintos.
        let unique: std::collections::HashSet<_> =
            colors.iter().map(|c| c.components.map(|x| (x * 255.0) as u8)).collect();
        assert!(unique.len() >= 2);
    }
}
