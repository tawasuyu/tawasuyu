//! Puente vCard (RFC 6350 / 2426) ↔ `Contact` nativo.
//!
//! CardDAV entrega objetos `VCARD`; acá los parseamos al [`Contact`] de
//! `raymi-core` y serializamos de vuelta para los `PUT`. Subconjunto práctico:
//! `UID`, `FN`, `EMAIL`, `TEL`, `ORG`, `NOTE`.

use raymi_core::Contact;

use crate::text::{escape, split_line, unescape, unfold};

/// Parsea uno o varios `VCARD` de `text` como [`Contact`]s de `address_book`.
pub fn parse_vcards(text: &str, address_book: &str) -> Vec<Contact> {
    let lines = unfold(text);
    let mut out = Vec::new();
    let mut cur: Option<Vec<(String, String, String)>> = None;

    for line in &lines {
        let upper = line.to_ascii_uppercase();
        if upper == "BEGIN:VCARD" {
            cur = Some(Vec::new());
        } else if upper == "END:VCARD" {
            if let Some(props) = cur.take() {
                if let Some(c) = contact_from_props(&props, address_book) {
                    out.push(c);
                }
            }
        } else if let Some(props) = cur.as_mut() {
            if let Some(parsed) = split_line(line) {
                props.push(parsed);
            }
        }
    }
    out
}

/// Serializa un [`Contact`] como `VCARD` 3.0, listo para `PUT`.
pub fn write_vcard(c: &Contact) -> String {
    let mut s = String::from("BEGIN:VCARD\r\nVERSION:3.0\r\n");
    s.push_str(&format!("UID:{}\r\n", c.uid));
    s.push_str(&format!("FN:{}\r\n", escape(&c.full_name)));
    for e in &c.emails {
        s.push_str(&format!("EMAIL:{e}\r\n"));
    }
    for t in &c.phones {
        s.push_str(&format!("TEL:{t}\r\n"));
    }
    if let Some(org) = &c.org {
        s.push_str(&format!("ORG:{}\r\n", escape(org)));
    }
    if !c.note.is_empty() {
        s.push_str(&format!("NOTE:{}\r\n", escape(&c.note)));
    }
    s.push_str("END:VCARD\r\n");
    s
}

fn contact_from_props(props: &[(String, String, String)], address_book: &str) -> Option<Contact> {
    let mut uid = String::new();
    let mut full_name = String::new();
    let mut emails = Vec::new();
    let mut phones = Vec::new();
    let mut org = None;
    let mut note = String::new();
    // `N` (apellido;nombre;…) como respaldo si no hay `FN`.
    let mut n_fallback = String::new();

    for (name, _params, value) in props {
        match name.as_str() {
            "UID" => uid = value.trim_start_matches("urn:uuid:").to_string(),
            "FN" => full_name = unescape(value),
            "EMAIL" => emails.push(value.trim().to_string()),
            "TEL" => phones.push(value.trim().to_string()),
            "ORG" => org = Some(unescape(value).replace(';', " ").trim().to_string()),
            "NOTE" => note = unescape(value),
            "N" => {
                // "Apellido;Nombre;…" → "Nombre Apellido"
                let parts: Vec<&str> = value.split(';').collect();
                let last = parts.first().copied().unwrap_or("").trim();
                let first = parts.get(1).copied().unwrap_or("").trim();
                n_fallback = format!("{first} {last}").trim().to_string();
            }
            _ => {}
        }
    }

    if full_name.is_empty() {
        full_name = n_fallback;
    }
    if full_name.is_empty() && emails.is_empty() {
        return None; // tarjeta vacía
    }
    if uid.is_empty() {
        uid = format!("raymi-{}@local", emails.first().cloned().unwrap_or_else(|| full_name.clone()));
    }
    Some(Contact {
        uid,
        full_name,
        emails,
        phones,
        org,
        note,
        address_book: address_book.to_string(),
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    const SAMPLE: &str = "BEGIN:VCARD\r\nVERSION:3.0\r\nUID:u-1\r\nFN:Ana Pérez\r\n\
EMAIL;TYPE=home:ana@x.com\r\nEMAIL;TYPE=work:ana@trabajo.com\r\n\
TEL:+58 412 555\r\nORG:Acme;Ventas\r\nNOTE:Nota libre\r\nEND:VCARD\r\n";

    #[test]
    fn parsea_vcard() {
        let cs = parse_vcards(SAMPLE, "def");
        assert_eq!(cs.len(), 1);
        let c = &cs[0];
        assert_eq!(c.uid, "u-1");
        assert_eq!(c.full_name, "Ana Pérez");
        assert_eq!(c.emails, vec!["ana@x.com", "ana@trabajo.com"]);
        assert_eq!(c.phones, vec!["+58 412 555"]);
        assert_eq!(c.org.as_deref(), Some("Acme Ventas"));
        assert_eq!(c.note, "Nota libre");
        assert_eq!(c.address_book, "def");
    }

    #[test]
    fn fn_cae_a_n_si_falta() {
        let raw = "BEGIN:VCARD\r\nN:Díaz;Bruno;;;\r\nEMAIL:b@x.com\r\nEND:VCARD\r\n";
        let c = &parse_vcards(raw, "def")[0];
        assert_eq!(c.full_name, "Bruno Díaz");
    }

    #[test]
    fn roundtrip_write_parse() {
        let c = &parse_vcards(SAMPLE, "def")[0];
        let serial = write_vcard(c);
        let back = &parse_vcards(&serial, "def")[0];
        assert_eq!(back.full_name, c.full_name);
        assert_eq!(back.emails, c.emails);
        assert_eq!(back.uid, c.uid);
    }

    #[test]
    fn varias_tarjetas() {
        let raw = format!("{SAMPLE}{}", "BEGIN:VCARD\r\nUID:u2\r\nFN:Bob\r\nEND:VCARD\r\n");
        assert_eq!(parse_vcards(&raw, "def").len(), 2);
    }
}
