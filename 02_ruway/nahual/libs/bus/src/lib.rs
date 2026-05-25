//! `nahual_bus` — `AppBus` + `AppEvent` para comunicación cross-widget.
//!
//! Es un `Entity<AppBus>` que emite [`AppEvent`]. Cualquier widget se
//! subscribe con `cx.subscribe(&bus, |this, _, ev, cx| { ... })`. La
//! Shell crea exactamente un AppBus al boot y lo distribuye:
//!
//! - **Productores** (FileExplorer, DatabaseExplorer): el LayoutHost los
//!   subscribe individualmente y reenvía sus eventos tipados al bus,
//!   normalizando al format `{provider, id, …}` agnóstico.
//! - **Consumidores** (TextViewer, ImageViewer, …): reciben el handle del
//!   bus en su constructor y se subscriben directo.
//!
//! Por qué un bus y no `cx.subscribe` directo entre productor y consumidor:
//! los viewers no saben qué explorers existen (ni viceversa). El bus
//! desacopla — puede haber 0, 1 o N explorers de distintos providers, y
//! varios viewers en paralelo viendo el mismo evento.

use gpui::EventEmitter;

/// Eventos cross-widget. Diseñados para ser agnósticos del dominio:
/// `provider` es el id (string) del DataProvider que sabe interpretar el
/// `id`. `provider_path` es el contexto opcional (ej. el .sqlite del
/// DatabaseExplorer) que el viewer necesita para construir su provider.
#[derive(Clone, Debug)]
pub enum AppEvent {
    /// Una entidad fue seleccionada (single click). Suele triggerear un
    /// preview en el viewer activo.
    EntitySelected {
        provider: String,
        provider_path: Option<String>,
        id: String,
    },
    /// Una entidad fue ejecutada (doble click u "Open" del menú).
    EntityOpened {
        provider: String,
        provider_path: Option<String>,
        id: String,
    },
}

#[derive(Default)]
pub struct AppBus;

impl EventEmitter<AppEvent> for AppBus {}
