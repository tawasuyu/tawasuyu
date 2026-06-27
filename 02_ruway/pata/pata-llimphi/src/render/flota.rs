//! Render del diente **«Flota»** (matilda): vista *read-only* del inventario
//! baremetal — hosts, contenedores y vhosts declarados. Sin SSH ni discover (eso
//! es fase posterior): sólo muestra lo que el inventario dice que **debe** existir.
//! Es el segundo brazo del control center de sistema + flota, montado global en
//! pata aunque matilda viva del lado de shuma.

use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::prelude::{
    auto, length, percent, AlignItems, FlexDirection, JustifyContent, Size, Style,
};
use llimphi_ui::llimphi_layout::taffy::Rect as TaffyRect;
use llimphi_ui::View;

use matilda_core::{Inventory, Upstream};

use super::panels::panel_box_flow;
use crate::Msg;

/// El panel de la flota, de alto completo. Lista hosts / contenedores / vhosts del
/// inventario, o un aviso si no hay inventario cargado.
pub fn flota_view(inv: Option<&Inventory>, panel_h: f32, theme: &Theme) -> View<Msg> {
    let titulo = View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(24.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text("Flota".to_string(), 14.0, theme.fg_text);

    let mut hijos = vec![titulo];
    match inv {
        Some(inv) if !inv.is_empty() => {
            // Hosts: nombre · dirección (+ tags tenues).
            let mut h_filas = vec![encabezado("Hosts", theme)];
            for h in inv.hosts() {
                let detalle = if h.tags.is_empty() {
                    h.address.clone()
                } else {
                    format!("{} · {}", h.address, h.tags.join(", "))
                };
                h_filas.push(fila(&h.name, &detalle, theme));
            }
            if h_filas.len() > 1 {
                hijos.push(panel_box_flow(h_filas, theme));
            }

            // Contenedores: nombre · imagen (+ puertos tenues).
            let mut c_filas = vec![encabezado("Contenedores", theme)];
            for c in inv.containers() {
                let puertos: Vec<String> =
                    c.ports.iter().map(|p| format!("{}→{}", p.host, p.container)).collect();
                let detalle = if puertos.is_empty() {
                    c.image.clone()
                } else {
                    format!("{}  [{}]", c.image, puertos.join(" "))
                };
                c_filas.push(fila(&c.name, &detalle, theme));
            }
            if c_filas.len() > 1 {
                hijos.push(panel_box_flow(c_filas, theme));
            }

            // VHosts: dominio · upstream (TLS marcado).
            let mut v_filas = vec![encabezado("VHosts", theme)];
            for v in inv.vhosts() {
                let up = match &v.upstream {
                    Upstream::Address(a) => a.clone(),
                    Upstream::Container { name, port } => format!("{name}:{port}"),
                };
                let candado = if v.tls { "🔒 " } else { "" };
                v_filas.push(fila(&v.domain, &format!("{candado}{up}"), theme));
            }
            if v_filas.len() > 1 {
                hijos.push(panel_box_flow(v_filas, theme));
            }
        }
        _ => {
            hijos.push(aviso(theme));
        }
    }

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: percent(1.0_f32), height: length(panel_h) },
        padding: TaffyRect {
            left: length(8.0_f32),
            right: length(8.0_f32),
            top: length(10.0_f32),
            bottom: length(10.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(10.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .children(hijos)
}

/// Encabezado tenue de una sección.
fn encabezado(t: &str, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        ..Default::default()
    })
    .text(t.to_string(), 12.0, theme.fg_muted)
}

/// Una fila: nombre (acento del texto) a la izquierda, detalle tenue a la derecha.
fn fila(nombre: &str, detalle: &str, theme: &Theme) -> View<Msg> {
    let izq = View::new(Style {
        size: Size { width: auto(), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        flex_shrink: 0.0,
        ..Default::default()
    })
    .text(nombre.to_string(), 12.0, theme.fg_text);
    let der = View::new(Style {
        flex_grow: 1.0,
        size: Size { width: percent(0.0_f32), height: length(20.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::FlexEnd),
        ..Default::default()
    })
    .text(detalle.to_string(), 11.0, theme.fg_muted);
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(22.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(8.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![izq, der])
}

/// Aviso cuando no hay inventario (o está vacío).
fn aviso(theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(60.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        padding: TaffyRect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .text(
        "Sin inventario de flota. Poné uno en ~/.config/tawasuyu/flota/inventory.json".to_string(),
        12.0,
        theme.fg_muted,
    )
}
