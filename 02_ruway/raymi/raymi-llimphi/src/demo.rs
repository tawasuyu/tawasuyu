//! Datos sembrados para correr raymi sin red: un `MockBackend` con dos
//! calendarios, eventos (incluyendo recurrentes) anclados alrededor de `now`, y
//! una libreta de contactos. Lo usan el `examples/agenda_demo` y, a futuro, el
//! fallback de `raymi-app`.

use raymi_core::time::{self, CivilDate, DAY};
use raymi_core::{AddressBook, Calendar, Contact, Event, MockBackend};

/// Crea un evento de un día relativo a `today0` (medianoche del día base),
/// a `day_offset` días, de `hour:00` por `dur_h` horas.
fn ev(uid: &str, cal: &str, summary: &str, today0: i64, day_offset: i64, hour: i64, dur_h: i64, rrule: Option<&str>) -> Event {
    let start = today0 + day_offset * DAY + hour * 3600;
    Event {
        uid: uid.into(),
        summary: summary.into(),
        description: String::new(),
        location: String::new(),
        start,
        end: start + dur_h * 3600,
        all_day: false,
        rrule: rrule.map(str::to_string),
        exdates: vec![],
        organizer: None,
        attendees: vec![],
        calendar: cal.into(),
    }
}

fn all_day(uid: &str, cal: &str, summary: &str, date: CivilDate, rrule: Option<&str>) -> Event {
    let start = time::to_unix(date, 0, 0, 0);
    Event {
        uid: uid.into(),
        summary: summary.into(),
        description: String::new(),
        location: String::new(),
        start,
        end: start + DAY,
        all_day: true,
        rrule: rrule.map(str::to_string),
        exdates: vec![],
        organizer: None,
        attendees: vec![],
        calendar: cal.into(),
    }
}

/// Construye un `MockBackend` sembrado con eventos alrededor de `now` (s Unix).
pub fn backend(now: i64) -> MockBackend {
    let cals = vec![
        Calendar::new("personal", "Personal").with_color("#3b82f6"),
        Calendar::new("trabajo", "Trabajo").with_color("#ef4444"),
    ];
    let books = vec![AddressBook::new("def", "Personales")];
    let mock = MockBackend::new(cals, books);

    let today0 = time::start_of_day(now);

    // Reunión con clientes: con invitados (cruce con la libreta).
    let mut clientes = ev("clientes", "trabajo", "Reunión con clientes", today0, 2, 11, 2, None);
    clientes.location = "Sala 2".into();
    clientes.attendees = vec![
        raymi_core::Address::named("Ana Pérez", "ana@ejemplo.com"),
        raymi_core::Address::named("Bruno Díaz", "bruno@empresa.com"),
    ];
    mock.seed_events(
        "trabajo",
        vec![
            // Daily standup hábil 9:00.
            ev("standup", "trabajo", "Daily standup", today0, 0, 9, 0, Some("FREQ=WEEKLY;BYDAY=MO,TU,WE,TH,FR")),
            // Revisión hoy a la tarde.
            ev("review", "trabajo", "Revisión de sprint", today0, 0, 15, 1, None),
            clientes,
        ],
    );
    mock.seed_events(
        "personal",
        vec![
            ev("almuerzo", "personal", "Almuerzo con Ana", today0, 1, 13, 1, None),
            ev("gym", "personal", "Gimnasio", today0, 0, 19, 1, Some("FREQ=WEEKLY;BYDAY=MO,WE,FR")),
            // Cumpleaños anual (de día completo) el día 20 de este mes.
            all_day(
                "cumple",
                "personal",
                "🎂 Cumpleaños de Bruno",
                CivilDate { year: time::to_civil(today0).0.year, month: time::to_civil(today0).0.month, day: 20 },
                Some("FREQ=YEARLY"),
            ),
        ],
    );

    mock.seed_contacts(
        "def",
        vec![
            contact("u1", "Ana Pérez", &["ana@ejemplo.com"], &["+58 412 555 0101"], Some("Acme S.A."), "Compañera de proyecto."),
            contact("u2", "Bruno Díaz", &["bruno@empresa.com"], &["+58 414 555 0102"], Some("Empresa C.A."), ""),
            contact("u3", "Carla Soto", &["carla@correo.com", "carla.soto@trabajo.com"], &[], None, "Diseñadora."),
        ],
    );

    mock
}

fn contact(uid: &str, name: &str, emails: &[&str], phones: &[&str], org: Option<&str>, note: &str) -> Contact {
    Contact {
        uid: uid.into(),
        full_name: name.into(),
        emails: emails.iter().map(|s| s.to_string()).collect(),
        phones: phones.iter().map(|s| s.to_string()).collect(),
        org: org.map(str::to_string),
        note: note.into(),
        address_book: "def".into(),
    }
}
