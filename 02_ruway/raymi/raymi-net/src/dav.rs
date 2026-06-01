//! Cliente HTTP CalDAV/CardDAV mínimo sobre `ureq`.
//!
//! Cubre lo necesario para sincronizar colecciones ya conocidas: `REPORT`
//! (calendar-query / addressbook-query), `PUT` y `DELETE`, con autenticación
//! Basic. El *descubrimiento* (PROPFIND de `calendar-home-set`, etc.) queda para
//! una sub-fase; hoy las URLs de colección se configuran explícitamente. El
//! parseo del `multistatus` usa `roxmltree` (DOM de sólo lectura), matcheando
//! por nombre local de etiqueta para no pelear con los namespaces.

use base64::Engine;
use raymi_core::CalError;

/// Un recurso de un `multistatus`: su `href`, su `ETag` y el dato embebido
/// (`calendar-data` o `address-data`), si vino.
#[derive(Debug, Clone)]
pub struct DavResource {
    pub href: String,
    pub etag: Option<String>,
    pub data: Option<String>,
}

/// Cliente con credenciales Basic precomputadas.
pub struct DavClient {
    auth: String,
}

impl DavClient {
    pub fn new(username: &str, password: &str) -> Self {
        let token = base64::engine::general_purpose::STANDARD.encode(format!("{username}:{password}"));
        Self { auth: format!("Basic {token}") }
    }

    /// `REPORT` con cuerpo XML; devuelve los recursos del `multistatus`.
    pub fn report(&self, url: &str, body: &str, content_type: &str) -> Result<Vec<DavResource>, CalError> {
        let xml = self.send("REPORT", url, Some(body), &[("Depth", "1"), ("Content-Type", content_type)])?;
        Ok(parse_multistatus(&xml))
    }

    /// `PUT` de un objeto (iCalendar/vCard) en `url`.
    pub fn put(&self, url: &str, body: &str, content_type: &str) -> Result<(), CalError> {
        self.send("PUT", url, Some(body), &[("Content-Type", content_type)]).map(|_| ())
    }

    /// `DELETE` de un objeto.
    pub fn delete(&self, url: &str) -> Result<(), CalError> {
        self.send("DELETE", url, None, &[]).map(|_| ())
    }

    fn send(
        &self,
        method: &str,
        url: &str,
        body: Option<&str>,
        headers: &[(&str, &str)],
    ) -> Result<String, CalError> {
        let mut req = ureq::request(method, url).set("Authorization", &self.auth);
        for (k, v) in headers {
            req = req.set(k, v);
        }
        let resp = match body {
            Some(b) => req.send_string(b),
            None => req.call(),
        };
        match resp {
            Ok(r) => r.into_string().map_err(|e| CalError::Transport(e.to_string())),
            Err(ureq::Error::Status(401, _)) | Err(ureq::Error::Status(403, _)) => Err(CalError::Auth),
            Err(ureq::Error::Status(code, r)) => {
                Err(CalError::Transport(format!("HTTP {code}: {}", r.status_text())))
            }
            Err(e) => Err(CalError::Transport(e.to_string())),
        }
    }
}

/// Cuerpo `REPORT` calendar-query que pide `getetag` + `calendar-data` de todos
/// los `VEVENT`.
pub const CALENDAR_QUERY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<c:calendar-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:prop><d:getetag/><c:calendar-data/></d:prop>
  <c:filter><c:comp-filter name="VCALENDAR"><c:comp-filter name="VEVENT"/></c:comp-filter></c:filter>
</c:calendar-query>"#;

/// Cuerpo `REPORT` addressbook-query que pide `getetag` + `address-data`.
pub const ADDRESSBOOK_QUERY: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<c:addressbook-query xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:carddav">
  <d:prop><d:getetag/><c:address-data/></d:prop>
</c:addressbook-query>"#;

pub const CALENDAR_CT: &str = "application/xml; charset=utf-8";
pub const ICAL_CT: &str = "text/calendar; charset=utf-8";
pub const VCARD_CT: &str = "text/vcard; charset=utf-8";

/// Parsea un `multistatus` DAV en recursos. Tolerante: matchea por nombre local
/// (`href`, `getetag`, `calendar-data`, `address-data`) ignorando el namespace.
pub fn parse_multistatus(xml: &str) -> Vec<DavResource> {
    let doc = match roxmltree::Document::parse(xml) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.has_tag_name_local("response")) {
        let mut href = None;
        let mut etag = None;
        let mut data = None;
        for n in resp.descendants() {
            match n.tag_name().name() {
                "href" if href.is_none() => href = n.text().map(|t| t.trim().to_string()),
                "getetag" => etag = n.text().map(|t| t.trim().trim_matches('"').to_string()),
                "calendar-data" | "address-data" => data = n.text().map(|t| t.to_string()),
                _ => {}
            }
        }
        if let Some(href) = href {
            out.push(DavResource { href, etag, data });
        }
    }
    out
}

/// Pequeña extensión: matchear por nombre local de etiqueta.
trait LocalName {
    fn has_tag_name_local(&self, local: &str) -> bool;
}
impl LocalName for roxmltree::Node<'_, '_> {
    fn has_tag_name_local(&self, local: &str) -> bool {
        self.is_element() && self.tag_name().name() == local
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const MULTISTATUS: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav">
  <d:response>
    <d:href>/cal/personal/ev1.ics</d:href>
    <d:propstat><d:prop>
      <d:getetag>"abc123"</d:getetag>
      <c:calendar-data>BEGIN:VCALENDAR&#10;END:VCALENDAR</c:calendar-data>
    </d:prop></d:propstat>
  </d:response>
  <d:response>
    <d:href>/cal/personal/ev2.ics</d:href>
    <d:propstat><d:prop><d:getetag>"def"</d:getetag></d:prop></d:propstat>
  </d:response>
</d:multistatus>"#;

    #[test]
    fn parsea_multistatus() {
        let rs = parse_multistatus(MULTISTATUS);
        assert_eq!(rs.len(), 2);
        assert_eq!(rs[0].href, "/cal/personal/ev1.ics");
        assert_eq!(rs[0].etag.as_deref(), Some("abc123"));
        assert!(rs[0].data.as_deref().unwrap().contains("VCALENDAR"));
        assert_eq!(rs[1].etag.as_deref(), Some("def"));
        assert!(rs[1].data.is_none());
    }

    #[test]
    fn xml_invalido_no_panica() {
        assert!(parse_multistatus("no es xml <<<").is_empty());
    }

    #[test]
    fn auth_basic_se_codifica() {
        let c = DavClient::new("ana", "secreto");
        // "ana:secreto" en base64
        assert_eq!(c.auth, "Basic YW5hOnNlY3JldG8=");
    }
}
