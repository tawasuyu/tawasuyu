//! Sub-bootstraps JS-puro. JsRuntime::new() los eval_raw en orden:
//! dependencias semánticas marcadas en cada módulo. Para agregar features
//! grandes, preferí UN módulo nuevo a engordar uno existente.

mod window_alias;
mod console;
mod timers;
mod microtask;
mod performance;
mod dom_events;
mod event_class;
mod event_target;
mod typed_events;
mod domexception;
mod window_scroll;
mod window_events;
mod error_events;
mod url;
mod streams;
mod blob;
mod file;
mod filereader;
mod objecturl;
mod urlsearchparams;
mod urlclass;
mod location;
mod history;
mod textcodec;
mod base64;
mod crypto;
mod crypto_subtle;
mod structuredclone;
mod formdata;
mod body;
mod response;
mod request;
mod fetch;
mod navigator;
mod connection;
mod websocket;
mod eventsource;
mod broadcastchannel;
mod messagechannel;
mod headers;
mod abort;
mod visibility;
mod observers;
mod xhr;
mod cookies;
mod cachestorage;
mod storage_event;
mod permissions;
mod notification;
mod geolocation;
mod clipboard;
mod share;
mod matchmedia;
mod screen;
mod serviceworker;
mod mediadevices;
mod battery;
mod wakelock;
mod storagemanager;
mod locks;
mod useractivation;
mod mediasession;
mod vibration;
mod gamepad;
mod credentials;
mod badging;
mod deviceorientation;
mod payment;
mod speech;
mod storageaccess;
mod eyedropper;
mod idledetector;
mod contacts;
mod midi;
mod serial;
mod hid;
mod usb;
mod computed_style;

pub(crate) use window_alias::WINDOW_ALIAS_BOOTSTRAP;
pub(crate) use console::CONSOLE_BOOTSTRAP;
pub(crate) use timers::TIMERS_BOOTSTRAP;
pub(crate) use microtask::MICROTASK_BOOTSTRAP;
pub(crate) use performance::PERFORMANCE_BOOTSTRAP;
pub(crate) use dom_events::DOM_EVENTS_BOOTSTRAP;
pub(crate) use event_class::EVENT_CLASS_BOOTSTRAP;
pub(crate) use event_target::EVENT_TARGET_BOOTSTRAP;
pub(crate) use typed_events::TYPED_EVENTS_BOOTSTRAP;
pub(crate) use domexception::DOMEXCEPTION_BOOTSTRAP;
pub(crate) use window_scroll::WINDOW_SCROLL_BOOTSTRAP;
pub(crate) use window_events::WINDOW_EVENTS_BOOTSTRAP;
pub(crate) use error_events::ERROR_EVENTS_BOOTSTRAP;
pub(crate) use url::URL_BOOTSTRAP;
pub(crate) use streams::STREAMS_BOOTSTRAP;
pub(crate) use blob::BLOB_BOOTSTRAP;
pub(crate) use file::FILE_BOOTSTRAP;
pub(crate) use filereader::FILEREADER_BOOTSTRAP;
pub(crate) use objecturl::OBJECT_URL_BOOTSTRAP;
pub(crate) use urlsearchparams::URLSEARCHPARAMS_BOOTSTRAP;
pub(crate) use urlclass::URLCLASS_BOOTSTRAP;
pub(crate) use location::LOCATION_BOOTSTRAP;
pub(crate) use history::HISTORY_BOOTSTRAP;
pub(crate) use textcodec::TEXTCODEC_BOOTSTRAP;
pub(crate) use base64::BASE64_BOOTSTRAP;
pub(crate) use crypto::CRYPTO_BOOTSTRAP;
pub(crate) use crypto_subtle::CRYPTO_SUBTLE_BOOTSTRAP;
pub(crate) use structuredclone::STRUCTURED_CLONE_BOOTSTRAP;
pub(crate) use formdata::FORMDATA_BOOTSTRAP;
pub(crate) use body::BODY_BOOTSTRAP;
pub(crate) use response::RESPONSE_BOOTSTRAP;
pub(crate) use request::REQUEST_BOOTSTRAP;
pub(crate) use fetch::FETCH_BOOTSTRAP;
pub(crate) use navigator::NAVIGATOR_BOOTSTRAP;
pub(crate) use connection::CONNECTION_BOOTSTRAP;
pub(crate) use websocket::WEBSOCKET_BOOTSTRAP;
pub(crate) use eventsource::EVENTSOURCE_BOOTSTRAP;
pub(crate) use broadcastchannel::BROADCAST_CHANNEL_BOOTSTRAP;
pub(crate) use messagechannel::MESSAGE_CHANNEL_BOOTSTRAP;
pub(crate) use headers::HEADERS_BOOTSTRAP;
pub(crate) use abort::ABORT_BOOTSTRAP;
pub(crate) use visibility::VISIBILITY_BOOTSTRAP;
pub(crate) use observers::OBSERVERS_BOOTSTRAP;
pub(crate) use xhr::XHR_BOOTSTRAP;
pub(crate) use cookies::COOKIES_BOOTSTRAP;
pub(crate) use cachestorage::CACHE_STORAGE_BOOTSTRAP;
pub(crate) use storage_event::STORAGE_EVENT_BOOTSTRAP;
pub(crate) use permissions::PERMISSIONS_BOOTSTRAP;
pub(crate) use notification::NOTIFICATION_BOOTSTRAP;
pub(crate) use geolocation::GEOLOCATION_BOOTSTRAP;
pub(crate) use clipboard::CLIPBOARD_BOOTSTRAP;
pub(crate) use share::SHARE_BOOTSTRAP;
pub(crate) use matchmedia::MATCHMEDIA_BOOTSTRAP;
pub(crate) use screen::SCREEN_BOOTSTRAP;
pub(crate) use serviceworker::SERVICEWORKER_BOOTSTRAP;
pub(crate) use mediadevices::MEDIADEVICES_BOOTSTRAP;
pub(crate) use battery::BATTERY_BOOTSTRAP;
pub(crate) use wakelock::WAKELOCK_BOOTSTRAP;
pub(crate) use storagemanager::STORAGEMANAGER_BOOTSTRAP;
pub(crate) use locks::LOCKS_BOOTSTRAP;
pub(crate) use useractivation::USER_ACTIVATION_BOOTSTRAP;
pub(crate) use mediasession::MEDIA_SESSION_BOOTSTRAP;
pub(crate) use vibration::VIBRATION_BOOTSTRAP;
pub(crate) use gamepad::GAMEPAD_BOOTSTRAP;
pub(crate) use credentials::CREDENTIALS_BOOTSTRAP;
pub(crate) use badging::BADGING_BOOTSTRAP;
pub(crate) use deviceorientation::DEVICE_ORIENTATION_BOOTSTRAP;
pub(crate) use payment::PAYMENT_BOOTSTRAP;
pub(crate) use speech::SPEECH_BOOTSTRAP;
pub(crate) use storageaccess::STORAGE_ACCESS_BOOTSTRAP;
pub(crate) use eyedropper::EYEDROPPER_BOOTSTRAP;
pub(crate) use idledetector::IDLEDETECTOR_BOOTSTRAP;
pub(crate) use contacts::CONTACTS_BOOTSTRAP;
pub(crate) use midi::MIDI_BOOTSTRAP;
pub(crate) use serial::SERIAL_BOOTSTRAP;
pub(crate) use hid::HID_BOOTSTRAP;
pub(crate) use usb::USB_BOOTSTRAP;
pub(crate) use computed_style::COMPUTED_STYLE_BOOTSTRAP;

/// Lista ordenada — JsRuntime::new() corre eval_raw sobre cada elemento.
pub(crate) const ALL: &[&str] = &[
    WINDOW_ALIAS_BOOTSTRAP,
    CONSOLE_BOOTSTRAP,
    TIMERS_BOOTSTRAP,
    MICROTASK_BOOTSTRAP,
    PERFORMANCE_BOOTSTRAP,
    DOM_EVENTS_BOOTSTRAP,
    EVENT_CLASS_BOOTSTRAP,
    EVENT_TARGET_BOOTSTRAP,
    TYPED_EVENTS_BOOTSTRAP,
    DOMEXCEPTION_BOOTSTRAP,
    WINDOW_SCROLL_BOOTSTRAP,
    WINDOW_EVENTS_BOOTSTRAP,
    ERROR_EVENTS_BOOTSTRAP,
    URL_BOOTSTRAP,
    STREAMS_BOOTSTRAP,
    BLOB_BOOTSTRAP,
    FILE_BOOTSTRAP,
    FILEREADER_BOOTSTRAP,
    OBJECT_URL_BOOTSTRAP,
    URLSEARCHPARAMS_BOOTSTRAP,
    URLCLASS_BOOTSTRAP,
    LOCATION_BOOTSTRAP,
    HISTORY_BOOTSTRAP,
    TEXTCODEC_BOOTSTRAP,
    BASE64_BOOTSTRAP,
    CRYPTO_BOOTSTRAP,
    CRYPTO_SUBTLE_BOOTSTRAP,
    STRUCTURED_CLONE_BOOTSTRAP,
    FORMDATA_BOOTSTRAP,
    BODY_BOOTSTRAP,
    RESPONSE_BOOTSTRAP,
    REQUEST_BOOTSTRAP,
    FETCH_BOOTSTRAP,
    NAVIGATOR_BOOTSTRAP,
    CONNECTION_BOOTSTRAP,
    WEBSOCKET_BOOTSTRAP,
    EVENTSOURCE_BOOTSTRAP,
    BROADCAST_CHANNEL_BOOTSTRAP,
    MESSAGE_CHANNEL_BOOTSTRAP,
    HEADERS_BOOTSTRAP,
    ABORT_BOOTSTRAP,
    VISIBILITY_BOOTSTRAP,
    OBSERVERS_BOOTSTRAP,
    XHR_BOOTSTRAP,
    COOKIES_BOOTSTRAP,
    CACHE_STORAGE_BOOTSTRAP,
    STORAGE_EVENT_BOOTSTRAP,
    PERMISSIONS_BOOTSTRAP,
    NOTIFICATION_BOOTSTRAP,
    GEOLOCATION_BOOTSTRAP,
    CLIPBOARD_BOOTSTRAP,
    SHARE_BOOTSTRAP,
    MATCHMEDIA_BOOTSTRAP,
    SCREEN_BOOTSTRAP,
    SERVICEWORKER_BOOTSTRAP,
    MEDIADEVICES_BOOTSTRAP,
    BATTERY_BOOTSTRAP,
    WAKELOCK_BOOTSTRAP,
    STORAGEMANAGER_BOOTSTRAP,
    LOCKS_BOOTSTRAP,
    USER_ACTIVATION_BOOTSTRAP,
    MEDIA_SESSION_BOOTSTRAP,
    VIBRATION_BOOTSTRAP,
    GAMEPAD_BOOTSTRAP,
    CREDENTIALS_BOOTSTRAP,
    BADGING_BOOTSTRAP,
    DEVICE_ORIENTATION_BOOTSTRAP,
    PAYMENT_BOOTSTRAP,
    SPEECH_BOOTSTRAP,
    STORAGE_ACCESS_BOOTSTRAP,
    EYEDROPPER_BOOTSTRAP,
    IDLEDETECTOR_BOOTSTRAP,
    CONTACTS_BOOTSTRAP,
    MIDI_BOOTSTRAP,
    SERIAL_BOOTSTRAP,
    HID_BOOTSTRAP,
    USB_BOOTSTRAP,
    COMPUTED_STYLE_BOOTSTRAP,
];
