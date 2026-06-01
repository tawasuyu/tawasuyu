//! Estado de los **editores** de evento y contacto: borradores con campos de
//! texto, ciclo de foco con Tab y la conversión borrador → modelo nativo.
//!
//! La UI los pinta como un modal (`view::editor_overlay`); el `update` los
//! aplica al backend y al `CalStore`. Mantenerlos aquí deja `lib.rs` con sólo
//! las transiciones y `view.rs` con sólo el dibujo.

use llimphi_widget_text_input::TextInputState;

use raymi_core::recur::{self, Freq, Recurrence};
use raymi_core::time::{self, CivilDate};
use raymi_core::{Contact, Event};

/// Qué editor está abierto sobre el cuerpo (o ninguno).
pub enum Editor {
    None,
    Event(EventDraft),
    Contact(ContactDraft),
}

impl Editor {
    pub fn is_open(&self) -> bool {
        !matches!(self, Editor::None)
    }
}

// ── Evento ──────────────────────────────────────────────────────────────────

/// Campo de texto enfocado del editor de evento (orden del ciclo Tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EventField {
    Summary,
    Date,
    Start,
    End,
    Interval,
    Count,
    Until,
    Location,
    Description,
}

impl EventField {
    /// Siguiente campo en el ciclo (plano; el formulario oculta los irrelevantes).
    pub fn next(self) -> Self {
        use EventField::*;
        match self {
            Summary => Date,
            Date => Start,
            Start => End,
            End => Interval,
            Interval => Count,
            Count => Until,
            Until => Location,
            Location => Description,
            Description => Summary,
        }
    }
}

/// Cadencia de repetición elegida en la UI (`Freq` + “no se repite”).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Repeat {
    None,
    Daily,
    Weekly,
    Monthly,
    Yearly,
}

impl Repeat {
    pub fn next(self) -> Self {
        use Repeat::*;
        match self {
            None => Daily,
            Daily => Weekly,
            Weekly => Monthly,
            Monthly => Yearly,
            Yearly => None,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Repeat::None => "No se repite",
            Repeat::Daily => "Diariamente",
            Repeat::Weekly => "Semanalmente",
            Repeat::Monthly => "Mensualmente",
            Repeat::Yearly => "Anualmente",
        }
    }

    /// Etiqueta de la unidad del intervalo (“cada N …”).
    pub fn unit(self) -> &'static str {
        match self {
            Repeat::None => "",
            Repeat::Daily => "día(s)",
            Repeat::Weekly => "semana(s)",
            Repeat::Monthly => "mes(es)",
            Repeat::Yearly => "año(s)",
        }
    }

    fn to_freq(self) -> Option<Freq> {
        match self {
            Repeat::None => None,
            Repeat::Daily => Some(Freq::Daily),
            Repeat::Weekly => Some(Freq::Weekly),
            Repeat::Monthly => Some(Freq::Monthly),
            Repeat::Yearly => Some(Freq::Yearly),
        }
    }

    fn from_freq(f: Freq) -> Self {
        match f {
            Freq::Daily => Repeat::Daily,
            Freq::Weekly => Repeat::Weekly,
            Freq::Monthly => Repeat::Monthly,
            Freq::Yearly => Repeat::Yearly,
        }
    }
}

/// Condición de término de la repetición.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RepeatEnd {
    Never,
    Count,
    Until,
}

impl RepeatEnd {
    pub fn next(self) -> Self {
        use RepeatEnd::*;
        match self {
            Never => Count,
            Count => Until,
            Until => Never,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            RepeatEnd::Never => "Sin fin",
            RepeatEnd::Count => "Tras N veces",
            RepeatEnd::Until => "Hasta fecha",
        }
    }
}

/// Borrador de un evento en edición. `uid = None` es uno nuevo; al editar uno
/// existente se preservan los campos que el formulario no toca (recurrencia,
/// organizador, invitados).
pub struct EventDraft {
    pub uid: Option<String>,
    pub calendar: String,
    pub summary: TextInputState,
    pub date: TextInputState,
    pub start_hm: TextInputState,
    pub end_hm: TextInputState,
    pub location: TextInputState,
    pub description: TextInputState,
    pub all_day: bool,
    // Recurrencia (editable).
    pub repeat: Repeat,
    pub interval: TextInputState,
    /// Días marcados para `WEEKLY;BYDAY` (índice 0 = lunes … 6 = domingo). Si
    /// ninguno está marcado, la regla repite en el mismo día de la semana del inicio.
    pub byday: [bool; 7],
    pub repeat_end: RepeatEnd,
    pub count: TextInputState,
    pub until: TextInputState,
    pub focus: EventField,
    // Campos preservados de un evento existente (no editables en el formulario).
    /// `RRULE` cruda preservada sólo si no la sabemos representar (no parsea).
    keep_rrule: Option<String>,
    keep_organizer: Option<raymi_core::Address>,
    keep_attendees: Vec<raymi_core::Address>,
}

impl EventDraft {
    /// Nuevo evento en `calendar`, anclado al día `day_ts` (medianoche UTC),
    /// 09:00–10:00 por defecto.
    pub fn new(calendar: String, day_ts: i64) -> Self {
        let date = time::civil_from_days(day_ts.div_euclid(time::DAY));
        Self {
            uid: None,
            calendar,
            summary: TextInputState::new(),
            date: input(&fmt_date(date)),
            start_hm: input("09:00"),
            end_hm: input("10:00"),
            location: TextInputState::new(),
            description: TextInputState::new(),
            all_day: false,
            repeat: Repeat::None,
            interval: input("1"),
            byday: [false; 7],
            repeat_end: RepeatEnd::Never,
            count: input("10"),
            until: TextInputState::new(),
            focus: EventField::Summary,
            keep_rrule: None,
            keep_organizer: None,
            keep_attendees: Vec::new(),
        }
    }

    /// Edita un evento existente: vuelca sus campos al borrador. La `RRULE` se
    /// descompone en los controles si la sabemos parsear; si no, se preserva
    /// cruda y el formulario muestra “No se repite”.
    pub fn from_event(e: &Event) -> Self {
        let (date, h, mi, _) = time::to_civil(e.start);
        let (_, eh, emi, _) = time::to_civil(e.end);

        let parsed = e.rrule.as_deref().filter(|s| !s.trim().is_empty()).map(|s| (s, recur::parse(s)));
        let mut repeat = Repeat::None;
        let mut interval = input("1");
        let mut byday = [false; 7];
        let mut repeat_end = RepeatEnd::Never;
        let mut count = input("10");
        let mut until = TextInputState::new();
        let mut keep_rrule = None;
        match parsed {
            Some((_, Some(r))) => {
                repeat = Repeat::from_freq(r.freq);
                interval = input(&r.interval.to_string());
                for &d in &r.byday {
                    if (d as usize) < 7 {
                        byday[d as usize] = true;
                    }
                }
                if let Some(c) = r.count {
                    repeat_end = RepeatEnd::Count;
                    count = input(&c.to_string());
                } else if let Some(u) = r.until {
                    repeat_end = RepeatEnd::Until;
                    let (ud, _, _, _) = time::to_civil(u);
                    until = input(&fmt_date(ud));
                }
            }
            // RRULE presente pero no representable → preservarla cruda.
            Some((raw, None)) => keep_rrule = Some(raw.to_string()),
            None => {}
        }

        Self {
            uid: Some(e.uid.clone()),
            calendar: e.calendar.clone(),
            summary: input(&e.summary),
            date: input(&fmt_date(date)),
            start_hm: input(&format!("{h:02}:{mi:02}")),
            end_hm: input(&format!("{eh:02}:{emi:02}")),
            location: input(&e.location),
            description: input(&e.description),
            all_day: e.all_day,
            repeat,
            interval,
            byday,
            repeat_end,
            count,
            until,
            focus: EventField::Summary,
            keep_rrule,
            keep_organizer: e.organizer.clone(),
            keep_attendees: e.attendees.clone(),
        }
    }

    pub fn focused_mut(&mut self) -> &mut TextInputState {
        match self.focus {
            EventField::Summary => &mut self.summary,
            EventField::Date => &mut self.date,
            EventField::Start => &mut self.start_hm,
            EventField::End => &mut self.end_hm,
            EventField::Interval => &mut self.interval,
            EventField::Count => &mut self.count,
            EventField::Until => &mut self.until,
            EventField::Location => &mut self.location,
            EventField::Description => &mut self.description,
        }
    }

    /// Compone la `RRULE` desde los controles, o `None` si “No se repite”.
    /// Devuelve `None` también si la cadencia no parsea a una `Freq`.
    fn build_rrule(&self) -> Option<String> {
        let freq = self.repeat.to_freq()?;
        let interval = self.interval.text().trim().parse::<u32>().ok().filter(|&n| n >= 1).unwrap_or(1);
        let byday: Vec<u32> = if matches!(self.repeat, Repeat::Weekly) {
            (0..7).filter(|&i| self.byday[i as usize]).collect()
        } else {
            Vec::new()
        };
        let (count, until) = match self.repeat_end {
            RepeatEnd::Never => (None, None),
            RepeatEnd::Count => (self.count.text().trim().parse::<u32>().ok().filter(|&n| n >= 1), None),
            RepeatEnd::Until => (None, parse_date(&self.until.text()).map(|d| time::to_unix(d, 23, 59, 59))),
        };
        Some(Recurrence { freq, interval, count, until, byday }.to_rrule())
    }

    /// Construye el `Event` final con el `uid` dado. `None` si la fecha o las
    /// horas no parsean. Para día completo se ancla a medianoche y dura un día.
    pub fn build(&self, uid: String) -> Option<Event> {
        let date = parse_date(&self.date.text())?;
        let (start, end) = if self.all_day {
            let s = time::to_unix(date, 0, 0, 0);
            (s, s + time::DAY)
        } else {
            let (sh, sm) = parse_hm(&self.start_hm.text())?;
            let (eh, em) = parse_hm(&self.end_hm.text())?;
            let s = time::to_unix(date, sh, sm, 0);
            let mut e = time::to_unix(date, eh, em, 0);
            if e <= s {
                e = s + 3600; // fin ≤ inicio → una hora
            }
            (s, e)
        };
        let summary = self.summary.text();
        // La regla compuesta gana; si “No se repite” pero había una cruda no
        // representable, se preserva.
        let rrule = self.build_rrule().or_else(|| self.keep_rrule.clone());
        Some(Event {
            uid,
            summary: if summary.trim().is_empty() { "(sin título)".to_string() } else { summary },
            description: self.description.text(),
            location: self.location.text(),
            start,
            end,
            all_day: self.all_day,
            rrule,
            organizer: self.keep_organizer.clone(),
            attendees: self.keep_attendees.clone(),
            calendar: self.calendar.clone(),
        })
    }
}

// ── Contacto ────────────────────────────────────────────────────────────────

/// Campo enfocado del editor de contacto (orden del ciclo Tab).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ContactField {
    Name,
    Emails,
    Phones,
    Org,
    Note,
}

impl ContactField {
    pub fn next(self) -> Self {
        use ContactField::*;
        match self {
            Name => Emails,
            Emails => Phones,
            Phones => Org,
            Org => Note,
            Note => Name,
        }
    }
}

/// Borrador de contacto. Correos y teléfonos se editan como una línea separada
/// por comas; se parten al construir el `Contact`.
pub struct ContactDraft {
    pub uid: Option<String>,
    pub address_book: String,
    pub name: TextInputState,
    pub emails: TextInputState,
    pub phones: TextInputState,
    pub org: TextInputState,
    pub note: TextInputState,
    pub focus: ContactField,
}

impl ContactDraft {
    pub fn new(address_book: String) -> Self {
        Self {
            uid: None,
            address_book,
            name: TextInputState::new(),
            emails: TextInputState::new(),
            phones: TextInputState::new(),
            org: TextInputState::new(),
            note: TextInputState::new(),
            focus: ContactField::Name,
        }
    }

    pub fn from_contact(c: &Contact) -> Self {
        Self {
            uid: Some(c.uid.clone()),
            address_book: c.address_book.clone(),
            name: input(&c.full_name),
            emails: input(&c.emails.join(", ")),
            phones: input(&c.phones.join(", ")),
            org: input(c.org.as_deref().unwrap_or("")),
            note: input(&c.note),
            focus: ContactField::Name,
        }
    }

    pub fn focused_mut(&mut self) -> &mut TextInputState {
        match self.focus {
            ContactField::Name => &mut self.name,
            ContactField::Emails => &mut self.emails,
            ContactField::Phones => &mut self.phones,
            ContactField::Org => &mut self.org,
            ContactField::Note => &mut self.note,
        }
    }

    /// Construye el `Contact` final. `None` si el nombre queda vacío.
    pub fn build(&self, uid: String) -> Option<Contact> {
        let full_name = self.name.text().trim().to_string();
        if full_name.is_empty() {
            return None;
        }
        let org = self.org.text().trim().to_string();
        Some(Contact {
            uid,
            full_name,
            emails: split_list(&self.emails.text()),
            phones: split_list(&self.phones.text()),
            org: if org.is_empty() { None } else { Some(org) },
            note: self.note.text(),
            address_book: self.address_book.clone(),
        })
    }
}

// ── helpers ─────────────────────────────────────────────────────────────────

fn input(s: &str) -> TextInputState {
    let mut t = TextInputState::new();
    t.set_text(s);
    t
}

/// Parte una línea separada por comas en valores no vacíos y recortados.
fn split_list(s: &str) -> Vec<String> {
    s.split(',').map(|p| p.trim().to_string()).filter(|p| !p.is_empty()).collect()
}

/// `"AAAA-MM-DD"` → fecha civil. Tolerante a espacios; exige los tres campos.
fn parse_date(s: &str) -> Option<CivilDate> {
    let mut it = s.trim().split('-');
    let year: i64 = it.next()?.trim().parse().ok()?;
    let month: u32 = it.next()?.trim().parse().ok()?;
    let day: u32 = it.next()?.trim().parse().ok()?;
    if it.next().is_some() || !(1..=12).contains(&month) || day < 1 || day > time::days_in_month(year, month) {
        return None;
    }
    Some(CivilDate { year, month, day })
}

/// `"HH:MM"` → (hora, minuto), validados a rango.
fn parse_hm(s: &str) -> Option<(u32, u32)> {
    let mut it = s.trim().split(':');
    let h: u32 = it.next()?.trim().parse().ok()?;
    let m: u32 = it.next()?.trim().parse().ok()?;
    if it.next().is_some() || h > 23 || m > 59 {
        return None;
    }
    Some((h, m))
}

/// Fecha civil → `"AAAA-MM-DD"`.
fn fmt_date(d: CivilDate) -> String {
    format!("{:04}-{:02}-{:02}", d.year, d.month, d.day)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn fechas_y_horas_roundtrip() {
        let d = parse_date("2026-06-01").unwrap();
        assert_eq!(fmt_date(d), "2026-06-01");
        assert!(parse_date("2026-13-01").is_none(), "mes inválido");
        assert!(parse_date("2026-02-30").is_none(), "día inexistente");
        assert!(parse_date("nope").is_none());
        assert_eq!(parse_hm("09:30"), Some((9, 30)));
        assert!(parse_hm("24:00").is_none());
        assert!(parse_hm("10:60").is_none());
    }

    #[test]
    fn draft_evento_construye_y_ordena_fin() {
        let mut d = EventDraft::new("personal".into(), time::days_from_civil(2026, 6, 1) * time::DAY);
        d.summary.set_text("Reunión");
        d.start_hm.set_text("15:00");
        d.end_hm.set_text("14:00"); // fin antes del inicio → +1h
        let e = d.build("u1".into()).unwrap();
        assert_eq!(e.summary, "Reunión");
        assert_eq!(e.calendar, "personal");
        assert_eq!(e.end - e.start, 3600);
    }

    #[test]
    fn draft_evento_compone_rrule_semanal() {
        let mut d = EventDraft::new("personal".into(), 0);
        d.repeat = Repeat::Weekly;
        d.interval.set_text("2");
        d.byday[0] = true; // lunes
        d.byday[2] = true; // miércoles
        d.repeat_end = RepeatEnd::Count;
        d.count.set_text("8");
        let e = d.build("u1".into()).unwrap();
        assert_eq!(e.rrule.as_deref(), Some("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE;COUNT=8"));
    }

    #[test]
    fn draft_evento_sin_repeticion_no_pone_rrule() {
        let d = EventDraft::new("personal".into(), 0);
        assert!(d.build("u1".into()).unwrap().rrule.is_none());
    }

    #[test]
    fn from_event_descompone_rrule_y_roundtrip() {
        let base = Event {
            uid: "u1".into(),
            summary: "Standup".into(),
            description: String::new(),
            location: String::new(),
            start: time::to_unix(CivilDate { year: 2026, month: 6, day: 1 }, 9, 0, 0),
            end: time::to_unix(CivilDate { year: 2026, month: 6, day: 1 }, 9, 30, 0),
            all_day: false,
            rrule: Some("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE".into()),
            organizer: None,
            attendees: vec![],
            calendar: "personal".into(),
        };
        let d = EventDraft::from_event(&base);
        assert_eq!(d.repeat, Repeat::Weekly);
        assert_eq!(d.interval.text(), "2");
        assert!(d.byday[0] && d.byday[2]);
        // y reconstruye la misma regla
        assert_eq!(d.build("u1".into()).unwrap().rrule.as_deref(), Some("FREQ=WEEKLY;INTERVAL=2;BYDAY=MO,WE"));
    }

    #[test]
    fn draft_evento_dia_completo() {
        let mut d = EventDraft::new("personal".into(), time::days_from_civil(2026, 6, 1) * time::DAY);
        d.all_day = true;
        let e = d.build("u1".into()).unwrap();
        assert!(e.all_day);
        assert_eq!(e.end - e.start, time::DAY);
    }

    #[test]
    fn draft_evento_titulo_vacio_es_sin_titulo() {
        let d = EventDraft::new("personal".into(), 0);
        let e = d.build("u1".into()).unwrap();
        assert_eq!(e.summary, "(sin título)");
    }

    #[test]
    fn draft_contacto_parte_listas() {
        let mut d = ContactDraft::new("def".into());
        d.name.set_text("Ana Pérez");
        d.emails.set_text("ana@x.com, ana@work.com ,");
        d.phones.set_text("123");
        let c = d.build("u1".into()).unwrap();
        assert_eq!(c.full_name, "Ana Pérez");
        assert_eq!(c.emails, vec!["ana@x.com", "ana@work.com"]);
        assert_eq!(c.phones, vec!["123"]);
    }

    #[test]
    fn draft_contacto_sin_nombre_no_construye() {
        let d = ContactDraft::new("def".into());
        assert!(d.build("u1".into()).is_none());
    }
}
