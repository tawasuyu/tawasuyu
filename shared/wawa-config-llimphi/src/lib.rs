//! `wawa-config-llimphi` — adaptador Llimphi del bus `wawa-config`.
//!
//! El crate base [`wawa_config`] es UI-agnóstico: no depende de
//! `llimphi-theme` para que herramientas no-gráficas (CLIs, daemons,
//! tests del propio bus) lo puedan usar sin arrastrar el toolkit
//! entero. Este crate provee la sola pieza que necesita una app
//! Llimphi para consumir la config: armar el `Theme` efectivo a
//! partir del `WawaConfig`.
//!
//! Patrón canónico en una app Llimphi:
//!
//! ```ignore
//! use wawa_config::{ConfigWatcher, WawaConfig};
//! use wawa_config_llimphi::theme_from_wawa;
//!
//! // En init:
//! let cfg = WawaConfig::load();
//! let theme = theme_from_wawa(&cfg, &Theme::dark());
//! let handle = handle.clone();
//! let _w = ConfigWatcher::spawn(move |c| {
//!     handle.dispatch(Msg::WawaConfigChanged(Box::new(c)));
//! })?;
//!
//! // En update::Msg::WawaConfigChanged(cfg):
//! m.theme = theme_from_wawa(&cfg, &m.theme);
//! ```
//!
//! Antes vivía duplicado en `gioser-edit` y `dominium-app-llimphi`;
//! al integrar `cosmos-app-llimphi` y `nakui-explorer-llimphi` como
//! consumidores tres más, el duplicado se factorizó acá.

#![forbid(unsafe_code)]

use llimphi_theme::{Color, Theme};
use wawa_config::WawaConfig;

/// Construye el `Theme` efectivo a partir de la config del bus.
///
/// 1. Toma `cfg.theme_variant`, lo canonicaliza (`"dark"` → `"Dark"`)
///    y lo busca en los presets de Llimphi. Si no existe, devuelve
///    `fallback` — esto evita que un variant desconocido (typo,
///    versión nueva del JSON, etc.) tire la app al theme dark "duro".
/// 2. Si `cfg.accent` no es `"default"` y tiene paleta conocida,
///    sobreescribe `theme.accent` y `theme.border_focus`.
///
/// Acepta `&Theme` (no `Theme`) porque las apps suelen tener un
/// theme actual válido al que volver — pasarlo por valor obligaría a
/// `clone()` en los callers, lo que es ruido en hot path
/// (`Msg::WawaConfigChanged` se dispara con cada save del panel).
pub fn theme_from_wawa(cfg: &WawaConfig, fallback: &Theme) -> Theme {
    let mut t = wawa_config::canonical_theme_name(&cfg.theme_variant)
        .and_then(Theme::by_name)
        .unwrap_or(*fallback);
    if let Some([r, g, b]) = wawa_config::accent_rgb(&cfg.accent) {
        let c = Color::from_rgba8(r, g, b, 255);
        t.accent = c;
        t.border_focus = c;
    }
    t
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn variant_dark_basic() {
        let cfg = WawaConfig::default();
        let t = theme_from_wawa(&cfg, &Theme::light());
        assert_eq!(t.name, "Dark");
        // accent del preset Dark — no debe estar pisado.
        assert_eq!(t.accent, Theme::dark().accent);
    }

    #[test]
    fn variant_aurora_with_accent_override() {
        let mut cfg = WawaConfig::default();
        cfg.theme_variant = "aurora".into();
        cfg.accent = "ukupacha".into();
        let t = theme_from_wawa(&cfg, &Theme::dark());
        assert_eq!(t.name, "Aurora");
        // accent: paleta ukupacha (verde oliva)
        let expected = Color::from_rgba8(0x8F, 0xB5, 0x8C, 255);
        assert_eq!(t.accent, expected);
        assert_eq!(t.border_focus, expected);
    }

    #[test]
    fn unknown_variant_falls_back() {
        let mut cfg = WawaConfig::default();
        cfg.theme_variant = "hyperdark-3000".into();
        let t = theme_from_wawa(&cfg, &Theme::sunset());
        // No es Dark, no es Aurora — conserva el fallback (Sunset).
        assert_eq!(t.name, "Sunset");
    }

    #[test]
    fn accent_default_does_not_override() {
        let mut cfg = WawaConfig::default();
        cfg.theme_variant = "aurora".into();
        cfg.accent = "default".into();
        let t = theme_from_wawa(&cfg, &Theme::dark());
        // accent del preset Aurora, no override.
        assert_eq!(t.accent, Theme::aurora().accent);
    }
}
