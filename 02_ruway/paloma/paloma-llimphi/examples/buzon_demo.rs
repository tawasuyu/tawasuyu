//! `buzon_demo` — paloma corriendo sobre un `MockBackend` sembrado, sin red.
//!
//! Tres paneles reales (buzones · hilos · lectura) + redacción. Sirve para
//! ejercitar el frontend sin credenciales: un INBOX con varias conversaciones
//! hiladas (una con tres mensajes), no-leídos, y un `Sent` que se puebla al
//! enviar.
//!
//! Atajos: `c` redacta · `r` responde al hilo abierto · `F5` refresca ·
//! Tab cicla campos del compositor · Esc cierra · ⏎/botón envía.
//!
//! Corre con: `cargo run -p paloma-llimphi --example buzon_demo --release`.

use llimphi_theme::Theme;
use llimphi_ui::{App, Handle, KeyEvent, Modifiers, View, WheelDelta};

use paloma_core::{Address, Flags, Message, MessageId, MockBackend};
use paloma_llimphi::{Model, Msg};

/// Un timestamp base (2026-05-25 12:00 UTC) + offset en horas, para fechar
/// los mensajes del demo sin arrastrar un crate de tiempo.
fn ts(hours: i64) -> i64 {
    1_748_174_400 + hours * 3_600
}

fn msg(
    id: &str,
    from: Address,
    subject: &str,
    body: &str,
    hours: i64,
    seen: bool,
    in_reply_to: Option<&str>,
    references: &[&str],
) -> Message {
    Message {
        id: MessageId(id.into()),
        from,
        to: vec![Address::named("Sergio", "sergio@jlsoltech.com")],
        cc: vec![],
        bcc: vec![],
        subject: subject.into(),
        date: ts(hours),
        in_reply_to: in_reply_to.map(|s| MessageId(s.into())),
        references: references.iter().map(|s| MessageId((*s).into())).collect(),
        body_text: body.into(),
        body_html: None,
        flags: Flags { seen, ..Default::default() },
        mailbox: "INBOX".into(),
    }
}

fn seed() -> MockBackend {
    let ana = Address::named("Ana Pérez", "ana@ejemplo.com");
    let bruno = Address::named("Bruno Díaz", "bruno@empresa.com");
    let lista = Address::named("Lista Rust", "anuncios@rust-es.org");

    let inbox = vec![
        // Hilo de tres mensajes (conversación viva, último sin leer).
        msg(
            "<p1@ejemplo.com>",
            ana.clone(),
            "Propuesta de integración",
            "Hola Sergio,\n\nTe paso la propuesta para integrar paloma con nuestro \
             servidor IMAP. ¿Tenés un rato esta semana para revisarla?\n\nSaludos,\nAna",
            -50,
            true,
            None,
            &[],
        ),
        msg(
            "<p2@jlsoltech.com>",
            Address::named("Sergio", "sergio@jlsoltech.com"),
            "Re: Propuesta de integración",
            "Ana, me parece muy bien. El jueves a la tarde me queda cómodo.",
            -40,
            true,
            Some("<p1@ejemplo.com>"),
            &["<p1@ejemplo.com>"],
        ),
        msg(
            "<p3@ejemplo.com>",
            ana,
            "Re: Propuesta de integración",
            "Perfecto, jueves 16hs entonces. Te mando el link de la llamada.",
            -2,
            false,
            Some("<p2@jlsoltech.com>"),
            &["<p1@ejemplo.com>", "<p2@jlsoltech.com>"],
        ),
        // Mensaje suelto sin leer.
        msg(
            "<f1@empresa.com>",
            bruno,
            "Factura de mayo",
            "Buenas, adjunto la factura del mes. Cualquier duda quedo a las órdenes.",
            -10,
            false,
            None,
            &[],
        ),
        // Boletín leído.
        msg(
            "<n1@rust-es.org>",
            lista,
            "Novedades de Rust 1.90",
            "Esta semana: nuevas APIs estabilizadas, mejoras en cargo y el \
             roadmap del próximo trimestre.",
            -28,
            true,
            None,
            &[],
        ),
    ];

    MockBackend::new(inbox)
}

struct Demo;

impl App for Demo {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "paloma"
    }

    fn initial_size() -> (u32, u32) {
        (1180, 720)
    }

    fn init(_handle: &Handle<Msg>) -> Model {
        let me = Address::named("Sergio", "sergio@jlsoltech.com");
        Model::new(Box::new(seed()), me, Theme::dark())
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        paloma_llimphi::update(model, msg, handle)
    }

    fn view(model: &Model) -> View<Msg> {
        paloma_llimphi::view(model)
    }

    fn view_overlay(model: &Model) -> Option<View<Msg>> {
        paloma_llimphi::view_overlay(model)
    }

    fn on_key(model: &Model, event: &KeyEvent) -> Option<Msg> {
        paloma_llimphi::on_key(model, event)
    }

    fn on_wheel(model: &Model, delta: WheelDelta, cursor: (f32, f32), mods: Modifiers) -> Option<Msg> {
        paloma_llimphi::on_wheel(model, delta, cursor, mods)
    }
}

fn main() {
    llimphi_ui::run::<Demo>();
}
