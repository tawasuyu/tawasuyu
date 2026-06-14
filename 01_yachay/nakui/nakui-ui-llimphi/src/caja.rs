//! Área **Caja** — la terminal del cajero (POS): grilla de **botones
//! grandes** de productos pensada para pantallas viejas / táctiles, el
//! ticket en curso al costado con su total, y botones grandes de cobro.
//!
//! No es una vista meta-driven: es una pantalla con estado (el carrito vive
//! en el `Model`). Opera sobre el módulo activo si es un POS —convención de
//! entidades `Producto` (nombre/precio/stock), `Venta` (total/metodo/…) y
//! `LineaVenta` (venta/producto/cantidad/importe)—. "Cobrar" siembra la
//! Venta y sus líneas y descuenta el stock vía el backend.

use super::*;
use std::time::{SystemTime, UNIX_EPOCH};

/// Una línea del ticket en curso (carrito).
#[derive(Clone)]
pub(crate) struct CartLine {
    pub product_id: Uuid,
    pub name: String,
    pub price: f64,
    pub qty: u32,
}

/// `true` si el módulo tiene forma de POS (entidades Producto/Venta/LineaVenta).
pub(crate) fn module_is_pos(module: &Module) -> bool {
    let has = |n: &str| module.entities.iter().any(|e| e.name == n);
    has("Producto") && has("Venta") && has("LineaVenta")
}

/// Métodos de pago ofrecidos en la barra del ticket.
pub(crate) const METODOS: [&str; 3] = ["efectivo", "tarjeta", "transferencia"];

fn money(v: f64) -> String {
    if v.fract() == 0.0 {
        format!("${}", v as i64)
    } else {
        format!("${v:.2}")
    }
}

/// Fecha de hoy en ISO `YYYY-MM-DD` (algoritmo civil-from-days, sin deps).
fn today_iso() -> String {
    let secs = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map(|d| d.as_secs() as i64)
        .unwrap_or(0);
    let z = secs.div_euclid(86_400) + 719_468;
    let era = z.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = doy - (153 * mp + 2) / 5 + 1;
    let m = if mp < 10 { mp + 3 } else { mp - 9 };
    let y = if m <= 2 { y + 1 } else { y };
    format!("{y:04}-{m:02}-{d:02}")
}

/// Cobra el ticket del `Model`. Delega en [`charge_cart`].
pub(crate) fn charge(m: &Model) -> (bool, Toast) {
    charge_cart(&m.backend, &m.cart, &m.caja_method)
}

/// Cobra un carrito: siembra la Venta + sus LineaVenta y descuenta el stock.
/// Devuelve `(ok, toast)`; en éxito el caller limpia el carrito.
pub(crate) fn charge_cart(
    backend: &Arc<Mutex<NakuiBackend>>,
    cart: &[CartLine],
    method: &str,
) -> (bool, Toast) {
    if cart.is_empty() {
        return (
            false,
            Toast { kind: BannerKind::Warning, text: "el ticket está vacío".into() },
        );
    }
    let total: f64 = cart.iter().map(|l| l.price * l.qty as f64).sum();
    let Ok(mut backend) = backend.lock() else {
        return (false, Toast { kind: BannerKind::Error, text: "backend ocupado".into() });
    };

    // 1. La Venta (cabecera del ticket).
    let mut venta = serde_json::Map::new();
    venta.insert("fecha".into(), Value::String(today_iso()));
    venta.insert("total".into(), Value::from(total));
    venta.insert("metodo".into(), Value::String(method.to_string()));
    venta.insert("pagado".into(), Value::Bool(true));
    venta.insert("estado".into(), Value::String("cerrada".into()));
    let venta_id = match backend.seed("Venta", venta) {
        Ok(o) => o.id,
        Err(e) => return (false, Toast { kind: BannerKind::Error, text: format!("no pude cobrar: {e}") }),
    };

    // 2. Una LineaVenta por ítem + descuento de stock.
    for line in cart {
        let mut lv = serde_json::Map::new();
        if let Some(vid) = venta_id {
            lv.insert("venta".into(), Value::String(vid.to_string()));
        }
        lv.insert("producto".into(), Value::String(line.product_id.to_string()));
        lv.insert("cantidad".into(), Value::from(line.qty));
        lv.insert("importe".into(), Value::from(line.price * line.qty as f64));
        let _ = backend.seed("LineaVenta", lv);

        // Descontar stock (best-effort).
        if let Some(rec) = backend.load_record("Producto", line.product_id) {
            let stock = rec.get("stock").and_then(|v| v.as_f64()).unwrap_or(0.0);
            let mut set = serde_json::Map::new();
            set.insert("stock".into(), Value::from(stock - line.qty as f64));
            let _ = backend.update("Producto", line.product_id, set, Vec::new());
        }
    }

    (
        true,
        Toast {
            kind: BannerKind::Success,
            text: format!("ticket cobrado · {} ({method})", money(total)),
        },
    )
}

/// Construye la pantalla del cajero.
pub(crate) fn build_caja(model: &Model, theme: &Theme) -> View<Msg> {
    let module = match model.selected_module.and_then(|i| model.modules.get(i)) {
        Some(mdl) if module_is_pos(mdl) => mdl,
        Some(_) => {
            return empty_panel(theme, "este módulo no es un Punto de Venta (faltan Producto/Venta/LineaVenta).");
        }
        None => return empty_panel(theme, "elegí un módulo POS en el panel de navegación."),
    };

    let products = grid(model, theme);
    let ticket = ticket_panel(model, theme);

    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
        flex_grow: 1.0,
        gap: Size { width: length(12.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![
        // Grilla de productos (ocupa el resto).
        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
            ..Default::default()
        })
        .children(vec![
            text_line(format!("{} · Caja", module.label), 16.0, theme.fg_text),
            products,
        ]),
        ticket,
    ])
}

/// Grilla de botones grandes, uno por producto.
fn grid(model: &Model, theme: &Theme) -> View<Msg> {
    let records = model
        .backend
        .lock()
        .ok()
        .map(|b| b.list_records("Producto"))
        .unwrap_or_default();

    let mut buttons: Vec<View<Msg>> = Vec::new();
    for (id, rec) in &records {
        let name = rec.get("nombre").and_then(|v| v.as_str()).unwrap_or("¿?").to_string();
        let price = rec.get("precio").and_then(|v| v.as_f64()).unwrap_or(0.0);
        let stock = rec.get("stock").and_then(|v| v.as_f64()).unwrap_or(0.0);
        buttons.push(product_button(*id, name, price, stock, theme));
    }
    if buttons.is_empty() {
        return empty_panel(theme, "no hay productos cargados — agregá alguno en '+ Producto'.");
    }

    View::new(Style {
        flex_direction: FlexDirection::Row,
        flex_wrap: llimphi_ui::llimphi_layout::taffy::FlexWrap::Wrap,
        size: Size { width: percent(1.0_f32), height: auto() },
        flex_grow: 1.0,
        align_content: Some(llimphi_ui::llimphi_layout::taffy::AlignContent::Start),
        align_items: Some(AlignItems::FlexStart),
        gap: Size { width: length(10.0_f32), height: length(10.0_f32) },
        ..Default::default()
    })
    .children(buttons)
}

/// Un botón grande de producto (toca para sumar al ticket).
fn product_button(id: Uuid, name: String, price: f64, stock: f64, theme: &Theme) -> View<Msg> {
    let agotado = stock <= 0.0;
    let mut card = View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(156.0_f32), height: length(92.0_f32) },
        flex_shrink: 0.0,
        justify_content: Some(JustifyContent::Center),
        gap: Size { width: length(0.0_f32), height: length(4.0_f32) },
        padding: Rect {
            left: length(12.0_f32),
            right: length(12.0_f32),
            top: length(8.0_f32),
            bottom: length(8.0_f32),
        },
        ..Default::default()
    })
    .fill(theme.bg_panel)
    .radius(10.0)
    .children(vec![
        text_line(name.clone(), 16.0, theme.fg_text),
        text_line(money(price), 18.0, theme.accent),
        text_line(
            if agotado { "sin stock".into() } else { format!("stock {}", stock as i64) },
            10.5,
            if agotado { theme.fg_destructive } else { theme.fg_muted },
        ),
    ]);
    if !agotado {
        card = card
            .hover_fill(theme.bg_row_hover)
            .on_click(Msg::CajaAddProduct { id, name, price });
    }
    card
}

/// Panel del ticket en curso: líneas, total y botones grandes de cobro.
fn ticket_panel(model: &Model, theme: &Theme) -> View<Msg> {
    let mut children: Vec<View<Msg>> = vec![text_line("Ticket".into(), 16.0, theme.fg_text)];

    if model.cart.is_empty() {
        children.push(text_line("tocá un producto para empezar".into(), 11.5, theme.fg_muted));
    } else {
        for (i, line) in model.cart.iter().enumerate() {
            children.push(ticket_row(i, line, theme));
        }
    }

    // Total.
    let total: f64 = model.cart.iter().map(|l| l.price * l.qty as f64).sum();
    children.push(
        View::new(Style {
            flex_direction: FlexDirection::Row,
            size: Size { width: percent(1.0_f32), height: length(44.0_f32) },
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::SpaceBetween),
            margin: Rect { left: length(0.0), right: length(0.0), top: length(8.0), bottom: length(0.0) },
            ..Default::default()
        })
        .children(vec![
            text_line("TOTAL".into(), 16.0, theme.fg_muted),
            View::new(Style {
                size: Size { width: auto(), height: length(34.0_f32) },
                align_items: Some(AlignItems::Center),
                ..Default::default()
            })
            .text_aligned(money(total), 28.0, theme.accent, Alignment::End),
        ]),
    );

    // Selector de método de pago (3 pastillas).
    children.push(method_row(model, theme));

    // Botones grandes de cobro / vaciar.
    children.push(big_button("COBRAR", theme.accent, theme.bg_app, Msg::CajaCharge));
    children.push(big_button("Vaciar", theme.bg_panel, theme.fg_muted, Msg::CajaClear));

    View::new(Style {
        flex_direction: FlexDirection::Column,
        size: Size { width: length(320.0_f32), height: percent(1.0_f32) },
        flex_shrink: 0.0,
        padding: Rect {
            left: length(14.0_f32),
            right: length(14.0_f32),
            top: length(12.0_f32),
            bottom: length(12.0_f32),
        },
        gap: Size { width: length(0.0_f32), height: length(8.0_f32) },
        ..Default::default()
    })
    .fill(theme.bg_panel_alt)
    .radius(10.0)
    .children(children)
}

/// Una fila del ticket: nombre, − cantidad +, importe.
fn ticket_row(i: usize, line: &CartLine, theme: &Theme) -> View<Msg> {
    let importe = line.price * line.qty as f64;
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(40.0_f32) },
        align_items: Some(AlignItems::Center),
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(vec![
        // Nombre (flex).
        View::new(Style {
            size: Size { width: percent(1.0_f32), height: percent(1.0_f32) },
            flex_grow: 1.0,
            align_items: Some(AlignItems::Center),
            ..Default::default()
        })
        .text_aligned(line.name.clone(), 13.0, theme.fg_text, Alignment::Start),
        qty_button("−", Msg::CajaDec(i), theme),
        View::new(Style {
            size: Size { width: length(26.0_f32), height: percent(1.0_f32) },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::Center),
            ..Default::default()
        })
        .text_aligned(line.qty.to_string(), 14.0, theme.fg_text, Alignment::Center),
        qty_button("+", Msg::CajaInc(i), theme),
        // Importe.
        View::new(Style {
            size: Size { width: length(64.0_f32), height: percent(1.0_f32) },
            flex_shrink: 0.0,
            align_items: Some(AlignItems::Center),
            justify_content: Some(JustifyContent::FlexEnd),
            ..Default::default()
        })
        .text_aligned(money(importe), 13.0, theme.fg_muted, Alignment::End),
    ])
}

fn qty_button(label: &str, msg: Msg, theme: &Theme) -> View<Msg> {
    View::new(Style {
        size: Size { width: length(30.0_f32), height: length(30.0_f32) },
        flex_shrink: 0.0,
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(theme.bg_button)
    .radius(6.0)
    .hover_fill(theme.bg_button_hover)
    .text_aligned(label.to_string(), 18.0, theme.fg_text, Alignment::Center)
    .on_click(msg)
}

fn method_row(model: &Model, theme: &Theme) -> View<Msg> {
    let mut chips: Vec<View<Msg>> = Vec::new();
    for met in METODOS {
        let active = model.caja_method == met;
        let (bg, fg) = if active { (theme.accent, theme.bg_app) } else { (theme.bg_button, theme.fg_muted) };
        chips.push(
            View::new(Style {
                size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
                flex_grow: 1.0,
                align_items: Some(AlignItems::Center),
                justify_content: Some(JustifyContent::Center),
                ..Default::default()
            })
            .fill(bg)
            .radius(6.0)
            .hover_fill(if active { bg } else { theme.bg_button_hover })
            .text_aligned(met.to_string(), 11.5, fg, Alignment::Center)
            .on_click(Msg::CajaSetMethod(met.to_string())),
        );
    }
    View::new(Style {
        flex_direction: FlexDirection::Row,
        size: Size { width: percent(1.0_f32), height: length(30.0_f32) },
        gap: Size { width: length(6.0_f32), height: length(0.0_f32) },
        ..Default::default()
    })
    .children(chips)
}

fn big_button(label: &str, bg: Color, fg: Color, msg: Msg) -> View<Msg> {
    View::new(Style {
        size: Size { width: percent(1.0_f32), height: length(48.0_f32) },
        align_items: Some(AlignItems::Center),
        justify_content: Some(JustifyContent::Center),
        ..Default::default()
    })
    .fill(bg)
    .radius(8.0)
    .text_aligned(label.to_string(), 18.0, fg, Alignment::Center)
    .on_click(msg)
}

/// Resumen del carrito para el panel Inspector.
pub(crate) fn inspector(model: &Model, theme: &Theme) -> Vec<View<Msg>> {
    let units: u32 = model.cart.iter().map(|l| l.qty).sum();
    let total: f64 = model.cart.iter().map(|l| l.price * l.qty as f64).sum();
    vec![
        text_line(format!("Líneas: {}", model.cart.len()), 12.0, theme.fg_muted),
        text_line(format!("Unidades: {units}"), 12.0, theme.fg_muted),
        text_line(format!("Total: {}", money(total)), 13.0, theme.accent),
        text_line(format!("Pago: {}", model.caja_method), 11.5, theme.fg_muted),
    ]
}
