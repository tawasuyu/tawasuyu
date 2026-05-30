//! Sub-bootstraps JS-puro. JsRuntime::new() los eval_raw en orden:
//! dependencias semánticas marcadas en cada módulo. Para agregar features
//! grandes, preferí UN módulo nuevo a engordar uno existente.

mod console;
mod timers;
mod dom_events;
mod event_class;
mod window_scroll;
mod window_events;
mod url;
mod streams;
mod blob;
mod file;
mod objecturl;
mod urlsearchparams;
mod urlclass;
mod textcodec;
mod base64;
mod formdata;
mod body;
mod response;
mod request;
mod fetch;
mod headers;
mod abort;
mod visibility;
mod observers;
mod xhr;
mod computed_style;

pub(crate) use console::CONSOLE_BOOTSTRAP;
pub(crate) use timers::TIMERS_BOOTSTRAP;
pub(crate) use dom_events::DOM_EVENTS_BOOTSTRAP;
pub(crate) use event_class::EVENT_CLASS_BOOTSTRAP;
pub(crate) use window_scroll::WINDOW_SCROLL_BOOTSTRAP;
pub(crate) use window_events::WINDOW_EVENTS_BOOTSTRAP;
pub(crate) use url::URL_BOOTSTRAP;
pub(crate) use streams::STREAMS_BOOTSTRAP;
pub(crate) use blob::BLOB_BOOTSTRAP;
pub(crate) use file::FILE_BOOTSTRAP;
pub(crate) use objecturl::OBJECT_URL_BOOTSTRAP;
pub(crate) use urlsearchparams::URLSEARCHPARAMS_BOOTSTRAP;
pub(crate) use urlclass::URLCLASS_BOOTSTRAP;
pub(crate) use textcodec::TEXTCODEC_BOOTSTRAP;
pub(crate) use base64::BASE64_BOOTSTRAP;
pub(crate) use formdata::FORMDATA_BOOTSTRAP;
pub(crate) use body::BODY_BOOTSTRAP;
pub(crate) use response::RESPONSE_BOOTSTRAP;
pub(crate) use request::REQUEST_BOOTSTRAP;
pub(crate) use fetch::FETCH_BOOTSTRAP;
pub(crate) use headers::HEADERS_BOOTSTRAP;
pub(crate) use abort::ABORT_BOOTSTRAP;
pub(crate) use visibility::VISIBILITY_BOOTSTRAP;
pub(crate) use observers::OBSERVERS_BOOTSTRAP;
pub(crate) use xhr::XHR_BOOTSTRAP;
pub(crate) use computed_style::COMPUTED_STYLE_BOOTSTRAP;

/// Lista ordenada — JsRuntime::new() corre eval_raw sobre cada elemento.
pub(crate) const ALL: &[&str] = &[
    CONSOLE_BOOTSTRAP,
    TIMERS_BOOTSTRAP,
    DOM_EVENTS_BOOTSTRAP,
    EVENT_CLASS_BOOTSTRAP,
    WINDOW_SCROLL_BOOTSTRAP,
    WINDOW_EVENTS_BOOTSTRAP,
    URL_BOOTSTRAP,
    STREAMS_BOOTSTRAP,
    BLOB_BOOTSTRAP,
    FILE_BOOTSTRAP,
    OBJECT_URL_BOOTSTRAP,
    URLSEARCHPARAMS_BOOTSTRAP,
    URLCLASS_BOOTSTRAP,
    TEXTCODEC_BOOTSTRAP,
    BASE64_BOOTSTRAP,
    FORMDATA_BOOTSTRAP,
    BODY_BOOTSTRAP,
    RESPONSE_BOOTSTRAP,
    REQUEST_BOOTSTRAP,
    FETCH_BOOTSTRAP,
    HEADERS_BOOTSTRAP,
    ABORT_BOOTSTRAP,
    VISIBILITY_BOOTSTRAP,
    OBSERVERS_BOOTSTRAP,
    XHR_BOOTSTRAP,
    COMPUTED_STYLE_BOOTSTRAP,
];
