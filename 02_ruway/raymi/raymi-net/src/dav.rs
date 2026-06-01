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

    /// `PROPFIND` con cuerpo XML y profundidad (`0` = el recurso, `1` = sus hijos).
    /// Devuelve el cuerpo `multistatus` crudo para que el llamante lo parsee según
    /// qué propiedad pidió.
    pub fn propfind(&self, url: &str, body: &str, depth: &str) -> Result<String, CalError> {
        self.send("PROPFIND", url, Some(body), &[("Depth", depth), ("Content-Type", DISCOVERY_CT)])
    }

    /// **Autodescubrimiento** completo desde una URL base: principal →
    /// home-sets (`calendar-home-set`/`addressbook-home-set`) → enumeración de
    /// colecciones (PROPFIND `Depth: 1`). Devuelve las colecciones encontradas
    /// (calendarios y libretas, ya etiquetadas por tipo y con URL absoluta).
    ///
    /// Tolerante: si falta el principal o un home-set, sigue con lo que haya en
    /// vez de abortar (servidores que exponen la colección directo en la base).
    pub fn discover(&self, base_url: &str) -> Result<Vec<DavCollection>, CalError> {
        let mut out = Vec::new();

        // 1. principal del usuario actual.
        let principal = match parse_current_user_principal(&self.propfind(base_url, PRINCIPAL_PROP, "0")?) {
            Some(href) => resolve(base_url, &href),
            None => base_url.to_string(), // sin principal: probamos la base como home.
        };

        // 2. home-sets de calendario y de contactos.
        let homes_xml = self.propfind(&principal, HOME_SET_PROP, "0")?;
        let mut homes = Vec::new();
        if let Some(h) = parse_home_set(&homes_xml, "calendar-home-set") {
            homes.push(resolve(base_url, &h));
        }
        if let Some(h) = parse_home_set(&homes_xml, "addressbook-home-set") {
            homes.push(resolve(base_url, &h));
        }
        if homes.is_empty() {
            homes.push(principal); // sin home-set: enumeramos el principal mismo.
        }

        // 3. enumerar colecciones bajo cada home; dedup por URL absoluta.
        for home in homes {
            let xml = self.propfind(&home, COLLECTIONS_PROP, "1")?;
            for mut c in parse_collections(&xml) {
                c.href = resolve(base_url, &c.href);
                if !out.iter().any(|o: &DavCollection| o.href == c.href) {
                    out.push(c);
                }
            }
        }
        Ok(out)
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
pub const DISCOVERY_CT: &str = "application/xml; charset=utf-8";

/// `PROPFIND` que pide el principal del usuario autenticado (paso 1 del
/// autodescubrimiento). Se manda sobre la URL base con `Depth: 0`.
pub const PRINCIPAL_PROP: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:">
  <d:prop><d:current-user-principal/></d:prop>
</d:propfind>"#;

/// `PROPFIND` sobre el principal que pide ambos home-sets (paso 2). Pide los dos
/// de una para no hacer dos viajes cuando el servidor sirve calendario y
/// contactos juntos (Nextcloud, Radicale).
pub const HOME_SET_PROP: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="urn:ietf:params:xml:ns:carddav">
  <d:prop><c:calendar-home-set/><a:addressbook-home-set/></d:prop>
</d:propfind>"#;

/// `PROPFIND` `Depth: 1` sobre un home-set que enumera las colecciones con su
/// tipo, nombre visible y color (paso 3). El color usa la extensión de Apple.
pub const COLLECTIONS_PROP: &str = r#"<?xml version="1.0" encoding="utf-8"?>
<d:propfind xmlns:d="DAV:" xmlns:ic="http://apple.com/ns/ical/">
  <d:prop><d:resourcetype/><d:displayname/><ic:calendar-color/></d:prop>
</d:propfind>"#;

/// Tipo de una colección descubierta, según su `resourcetype`.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CollectionKind {
    Calendar,
    AddressBook,
    /// Una colección plana (el home-set mismo) u otro recurso — se descarta.
    Other,
}

/// Una colección descubierta por PROPFIND: su URL (absoluta tras `resolve`), el
/// nombre visible y color opcionales, y su tipo.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DavCollection {
    pub href: String,
    pub display_name: Option<String>,
    pub color: Option<String>,
    pub kind: CollectionKind,
}

/// El `href` dentro del `current-user-principal` del `multistatus` (paso 1).
pub fn parse_current_user_principal(xml: &str) -> Option<String> {
    href_inside(xml, "current-user-principal")
}

/// El `href` dentro del home-set con nombre local `local` (`calendar-home-set` o
/// `addressbook-home-set`) del `multistatus` (paso 2).
pub fn parse_home_set(xml: &str, local: &str) -> Option<String> {
    href_inside(xml, local)
}

/// Las colecciones de un `multistatus` de enumeración (paso 3): una por
/// `response`, con su `href`, `displayname`, color y tipo inferido del
/// `resourcetype`.
pub fn parse_collections(xml: &str) -> Vec<DavCollection> {
    let doc = match roxmltree::Document::parse(xml) {
        Ok(d) => d,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for resp in doc.descendants().filter(|n| n.has_tag_name_local("response")) {
        let mut href = None;
        let mut display_name = None;
        let mut color = None;
        let mut kind = CollectionKind::Other;
        for n in resp.descendants() {
            match n.tag_name().name() {
                "href" if href.is_none() => href = n.text().map(|t| t.trim().to_string()),
                "displayname" => {
                    let t = n.text().map(|t| t.trim().to_string()).filter(|s| !s.is_empty());
                    if t.is_some() {
                        display_name = t;
                    }
                }
                "calendar-color" => {
                    color = n.text().map(|t| normalize_color(t.trim())).filter(|s| !s.is_empty());
                }
                "resourcetype" => {
                    for child in n.children().filter(|c| c.is_element()) {
                        match child.tag_name().name() {
                            "calendar" => kind = CollectionKind::Calendar,
                            "addressbook" => kind = CollectionKind::AddressBook,
                            _ => {}
                        }
                    }
                }
                _ => {}
            }
        }
        if let Some(href) = href {
            out.push(DavCollection { href, display_name, color, kind });
        }
    }
    out
}

/// Primer `href` que cuelga del primer elemento con nombre local `local`.
fn href_inside(xml: &str, local: &str) -> Option<String> {
    let doc = roxmltree::Document::parse(xml).ok()?;
    let node = doc.descendants().find(|n| n.has_tag_name_local(local))?;
    node.descendants()
        .find(|n| n.has_tag_name_local("href"))
        .and_then(|h| h.text())
        .map(|t| t.trim().to_string())
}

/// Normaliza un color CalDAV a `#rrggbb`: Apple suele mandar `#rrggbbaa` (con
/// alfa) — recortamos el canal alfa; lo que no parezca hex se deja como vino.
fn normalize_color(c: &str) -> String {
    if c.starts_with('#') && c.len() >= 7 {
        c[..7].to_string()
    } else {
        c.to_string()
    }
}

/// El origen (`esquema://autoridad`) de una URL: hasta la primera `/` después de
/// `://`. Para resolver `href`s absolutos al servidor que vino en la base.
fn origin(url: &str) -> &str {
    match url.find("://") {
        Some(i) => {
            let after = i + 3;
            match url[after..].find('/') {
                Some(j) => &url[..after + j],
                None => url,
            }
        }
        None => url,
    }
}

/// Resuelve un `href` del servidor contra la URL base: deja los absolutos
/// (`http(s)://…`) tal cual, prefija el origen a los que arrancan con `/`, y
/// concatena los relativos a la base.
fn resolve(base: &str, href: &str) -> String {
    if href.starts_with("http://") || href.starts_with("https://") {
        href.to_string()
    } else if href.starts_with('/') {
        format!("{}{}", origin(base), href)
    } else {
        format!("{}/{}", base.trim_end_matches('/'), href)
    }
}

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

    const PRINCIPAL: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:">
  <d:response><d:href>/remote.php/dav/</d:href><d:propstat><d:prop>
    <d:current-user-principal><d:href>/remote.php/dav/principals/users/ana/</d:href></d:current-user-principal>
  </d:prop></d:propstat></d:response>
</d:multistatus>"#;

    const HOMES: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:a="urn:ietf:params:xml:ns:carddav">
  <d:response><d:href>/remote.php/dav/principals/users/ana/</d:href><d:propstat><d:prop>
    <c:calendar-home-set><d:href>/remote.php/dav/calendars/ana/</d:href></c:calendar-home-set>
    <a:addressbook-home-set><d:href>/remote.php/dav/addressbooks/users/ana/</d:href></a:addressbook-home-set>
  </d:prop></d:propstat></d:response>
</d:multistatus>"#;

    const COLLECTIONS: &str = r#"<?xml version="1.0"?>
<d:multistatus xmlns:d="DAV:" xmlns:c="urn:ietf:params:xml:ns:caldav" xmlns:card="urn:ietf:params:xml:ns:carddav" xmlns:ic="http://apple.com/ns/ical/">
  <d:response>
    <d:href>/remote.php/dav/calendars/ana/</d:href>
    <d:propstat><d:prop><d:resourcetype><d:collection/></d:resourcetype></d:prop></d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/calendars/ana/personal/</d:href>
    <d:propstat><d:prop>
      <d:resourcetype><d:collection/><c:calendar/></d:resourcetype>
      <d:displayname>Personal</d:displayname>
      <ic:calendar-color>#3b82f6ff</ic:calendar-color>
    </d:prop></d:propstat>
  </d:response>
  <d:response>
    <d:href>/remote.php/dav/addressbooks/users/ana/contacts/</d:href>
    <d:propstat><d:prop>
      <d:resourcetype><d:collection/><card:addressbook/></d:resourcetype>
      <d:displayname>Contactos</d:displayname>
    </d:prop></d:propstat>
  </d:response>
</d:multistatus>"#;

    #[test]
    fn descubre_principal_y_home_sets() {
        assert_eq!(
            parse_current_user_principal(PRINCIPAL).as_deref(),
            Some("/remote.php/dav/principals/users/ana/")
        );
        assert_eq!(
            parse_home_set(HOMES, "calendar-home-set").as_deref(),
            Some("/remote.php/dav/calendars/ana/")
        );
        assert_eq!(
            parse_home_set(HOMES, "addressbook-home-set").as_deref(),
            Some("/remote.php/dav/addressbooks/users/ana/")
        );
    }

    #[test]
    fn enumera_colecciones_por_tipo() {
        let cs = parse_collections(COLLECTIONS);
        // El home-set plano (sólo <collection/>) es Other; un calendario y una libreta.
        assert_eq!(cs.len(), 3);
        let cal = cs.iter().find(|c| c.kind == CollectionKind::Calendar).unwrap();
        assert_eq!(cal.display_name.as_deref(), Some("Personal"));
        assert_eq!(cal.color.as_deref(), Some("#3b82f6")); // alfa recortado
        let book = cs.iter().find(|c| c.kind == CollectionKind::AddressBook).unwrap();
        assert_eq!(book.display_name.as_deref(), Some("Contactos"));
        assert!(cs.iter().any(|c| c.kind == CollectionKind::Other));
    }

    #[test]
    fn resolve_y_origin() {
        assert_eq!(origin("https://nube.org/remote.php/dav/"), "https://nube.org");
        assert_eq!(origin("https://nube.org:8443/x"), "https://nube.org:8443");
        assert_eq!(
            resolve("https://nube.org/remote.php/dav/", "/remote.php/dav/calendars/ana/"),
            "https://nube.org/remote.php/dav/calendars/ana/"
        );
        assert_eq!(
            resolve("https://nube.org/base", "https://otra.org/abs/"),
            "https://otra.org/abs/"
        );
    }

    #[test]
    fn normaliza_color() {
        assert_eq!(normalize_color("#aabbccdd"), "#aabbcc");
        assert_eq!(normalize_color("#aabbcc"), "#aabbcc");
        assert_eq!(normalize_color("rojo"), "rojo");
    }
}
