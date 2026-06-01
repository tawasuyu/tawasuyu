//! Datos sembrados para correr paloma sin red: un `MockBackend` con un INBOX
//! de ejemplo. Lo usan el `examples/buzon_demo` y el fallback de `paloma-app`
//! cuando no hay cuenta/credenciales configuradas — una sola fuente de verdad
//! para los datos de demostración.

use paloma_core::{Address, Flags, Message, MessageId, MockBackend};

/// Un timestamp base (2026-05-25 12:00 UTC) + offset en horas, para fechar los
/// mensajes del demo sin arrastrar un crate de tiempo.
fn ts(hours: i64) -> i64 {
    1_748_174_400 + hours * 3_600
}

#[allow(clippy::too_many_arguments)]
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

/// Construye un `MockBackend` con un INBOX poblado: un hilo de tres mensajes
/// (con el último sin leer), un mensaje suelto sin leer y un boletín leído.
pub fn backend() -> MockBackend {
    let ana = Address::named("Ana Pérez", "ana@ejemplo.com");
    let bruno = Address::named("Bruno Díaz", "bruno@empresa.com");
    let lista = Address::named("Lista Rust", "anuncios@rust-es.org");

    let inbox = vec![
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
