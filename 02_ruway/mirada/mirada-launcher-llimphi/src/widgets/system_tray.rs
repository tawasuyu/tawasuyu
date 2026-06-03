//! Widget de **bandeja del sistema** (system tray) sobre StatusNotifierItem.
//!
//! Posee un [`TrayHandle`] (hilo D-Bus aparte; ver [`crate::tray`]). En cada
//! `tick` relee el snapshot de items y `view` pinta un chip clickeable por item:
//! ícono si la app lo proveyó (pixmap embebido o PNG por nombre), o la etiqueta de
//! texto si no. El click emite [`Msg::TrayActivate`] con la `key` del item; la app
//! loop lo rutea de vuelta a este widget para activar el item por D-Bus.
//!
//! Si no hay bus de sesión (o ya hay otro watcher), el handle queda vacío y el
//! widget no pinta nada — la barra sigue viva.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{auto, length, AlignItems, JustifyContent, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::{Blob, Image, ImageFormat};
use llimphi_ui::View;

use crate::config::WidgetSpec;
use crate::tray::{TrayHandle, TrayIcon, TrayItem};
use crate::widget::{Msg, Widget};

/// Tamaño del ícono del tray en la barra (px).
const TRAY_ICON_PX: f32 = 18.0;
/// Largo máximo de la etiqueta de texto (cuando no hay ícono).
const TRAY_LABEL_MAX: usize = 14;

pub struct SystemTray {
    /// Handle al hilo del tray (`None` si no se pudo lanzar el hilo).
    handle: Option<TrayHandle>,
    /// Snapshot actual de items, refrescado por `tick`.
    items: Vec<TrayItem>,
}

impl SystemTray {
    pub fn from_spec(_spec: &WidgetSpec) -> Self {
        Self {
            handle: TrayHandle::spawn(),
            items: Vec::new(),
        }
    }

    /// Activa el item con esa `key` (llamado por la app loop al recibir
    /// [`Msg::TrayActivate`]).
    pub fn activate(&self, key: &str) {
        if let Some(h) = &self.handle {
            h.activate(key.to_string());
        }
    }
}

impl Widget for SystemTray {
    fn as_any_mut(&mut self) -> &mut dyn std::any::Any {
        self
    }
    fn as_any(&self) -> &dyn std::any::Any {
        self
    }

    fn tick(&mut self) {
        if let Some(h) = &self.handle {
            self.items = h.items();
        }
    }

    fn view(&self, theme: &Theme) -> View<Msg> {
        let chips: Vec<View<Msg>> = self.items.iter().map(|it| chip_view(it, theme)).collect();
        View::new(Style {
            size: Size {
                width: auto(),
                height: length(22.0_f32),
            },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            gap: Size {
                width: length(6.0_f32),
                height: length(0.0_f32),
            },
            ..Default::default()
        })
        .children(chips)
    }
}

/// Un chip clickeable por item: ícono (si lo hay) o etiqueta de texto. Resalta los
/// que piden atención (`NeedsAttention`) con el color de acento.
fn chip_view(it: &TrayItem, theme: &Theme) -> View<Msg> {
    let base = View::new(Style {
        size: Size {
            width: auto(),
            height: length(22.0_f32),
        },
        padding: Rect {
            left: length(6.0_f32),
            right: length(6.0_f32),
            top: length(0.0_f32),
            bottom: length(0.0_f32),
        },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(6.0)
    .hover_fill(theme.bg_button_hover)
    .on_click(Msg::TrayActivate(it.key.clone()));

    match &it.icon {
        Some(icon) => base.children(vec![tray_icon_node(icon)]),
        None => {
            let fg = if it.status == "NeedsAttention" {
                theme.accent
            } else {
                theme.fg_text
            };
            base.text(recortar(&it.label, TRAY_LABEL_MAX), 12.0, fg)
        }
    }
}

/// Un nodo cuadrado de [`TRAY_ICON_PX`] con el ícono del item. Arma la
/// `peniko::Image` desde los bytes RGBA que el hilo del tray ya decodificó.
fn tray_icon_node(icon: &TrayIcon) -> View<Msg> {
    let blob = Blob::from(icon.rgba.clone());
    let img = Image::new(blob, ImageFormat::Rgba8, icon.width, icon.height);
    View::new(Style {
        size: Size {
            width: length(TRAY_ICON_PX),
            height: length(TRAY_ICON_PX),
        },
        ..Default::default()
    })
    .image(img)
}

/// Recorta una cadena a `max` caracteres, agregando `…` si sobró.
fn recortar(s: &str, max: usize) -> String {
    if s.chars().count() <= max {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(max.saturating_sub(1)).collect();
        out.push('…');
        out
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recortar_respeta_el_tope() {
        assert_eq!(recortar("nm-applet", 14), "nm-applet");
        assert_eq!(recortar("una-etiqueta-larguísima", 6), "una-e…");
    }

    #[test]
    fn from_spec_no_panica_sin_dbus() {
        // Sin bus de sesión el handle puede ser None o un hilo que termina; en
        // cualquier caso, construir el widget no debe panicar y arranca vacío.
        let spec = WidgetSpec {
            kind: "system_tray".into(),
            props: std::collections::HashMap::new(),
        };
        let w = SystemTray::from_spec(&spec);
        assert!(w.items.is_empty());
    }
}
