//! Parser incremental del wire-format de **Server-Sent Events** (el stream que
//! consume `EventSource`). Vive acá, junto a `fetch`, porque es parseo de
//! protocolo puro — el worker del chrome lo alimenta con los bytes que llegan
//! del socket y va sacando eventos ya formados.
//!
//! Formato (WHATWG): líneas `campo: valor` (un espacio tras `:` opcional),
//! líneas que arrancan con `:` son comentarios, una línea en blanco "despacha"
//! el evento acumulado. Campos: `data` (se acumula, separado por `\n`),
//! `event` (tipo; default `message`), `id` (último id, persiste entre eventos),
//! `retry` (ms de reconexión). Saltos `\n`, `\r` y `\r\n` se aceptan.

/// Un evento SSE ya formado, listo para reinyectar al `EventSource` JS.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct SseEvent {
    /// Tipo del evento — `message` si el stream no mandó `event:`.
    pub event_type: String,
    /// Cuerpo (`data`), con el `\n` final ya recortado.
    pub data: String,
    /// `lastEventId` vigente al despachar (el último `id:` visto, persiste).
    pub last_id: String,
}

/// Acumulador incremental: se le da `feed(chunk)` con lo que llega del socket
/// (puede partir líneas a la mitad) y devuelve los eventos completados.
#[derive(Default)]
pub struct SseParser {
    /// Bytes recibidos que aún no forman una línea completa (sin `\n` final).
    pending: String,
    /// Buffer de `data` del evento en curso (con `\n` por cada línea `data:`).
    data: String,
    /// Buffer del tipo de evento en curso (`event:`).
    event_type: String,
    /// Último `id:` visto — persiste entre eventos (spec).
    last_id: String,
    /// Último `retry:` parseado (ms de reconexión) — lo lee el worker.
    retry: Option<u64>,
}

impl SseParser {
    pub fn new() -> Self {
        Self::default()
    }

    /// Ms de reconexión pedidos por el server (`retry:`), si los mandó.
    pub fn retry(&self) -> Option<u64> {
        self.retry
    }

    /// Alimenta un trozo de stream. Extrae todas las líneas completas
    /// (hasta el último `\n`) y deja la cola incompleta en `pending`.
    pub fn feed(&mut self, chunk: &str) -> Vec<SseEvent> {
        self.pending.push_str(chunk);
        let mut events = Vec::new();
        while let Some(nl) = self.pending.find('\n') {
            let mut line: String = self.pending.drain(..=nl).collect();
            line.pop(); // quita el '\n'
            if line.ends_with('\r') {
                line.pop(); // \r\n
            }
            if let Some(ev) = self.process_line(&line) {
                events.push(ev);
            }
        }
        events
    }

    fn process_line(&mut self, line: &str) -> Option<SseEvent> {
        // Línea en blanco → despacha el evento acumulado (si hay data).
        if line.is_empty() {
            if self.data.is_empty() {
                // Sin data: spec dice resetear tipo y no despachar.
                self.event_type.clear();
                return None;
            }
            let mut data = std::mem::take(&mut self.data);
            if data.ends_with('\n') {
                data.pop();
            }
            let event_type = if self.event_type.is_empty() {
                "message".to_string()
            } else {
                std::mem::take(&mut self.event_type)
            };
            self.event_type.clear();
            return Some(SseEvent { event_type, data, last_id: self.last_id.clone() });
        }
        // Comentario.
        if line.starts_with(':') {
            return None;
        }
        // `campo: valor` (un espacio tras `:` se recorta); sin `:` → campo solo.
        let (field, value) = match line.split_once(':') {
            Some((f, v)) => (f, v.strip_prefix(' ').unwrap_or(v)),
            None => (line, ""),
        };
        match field {
            "data" => {
                self.data.push_str(value);
                self.data.push('\n');
            }
            "event" => self.event_type = value.to_string(),
            "id" => {
                // El spec descarta `id` con NUL.
                if !value.contains('\0') {
                    self.last_id = value.to_string();
                }
            }
            "retry" => {
                if let Ok(ms) = value.parse::<u64>() {
                    self.retry = Some(ms);
                }
            }
            _ => {}
        }
        None
    }
}

/// Ms de reconexión por defecto si el server no manda `retry:` (igual que los
/// browsers, ~3s).
const DEFAULT_RETRY_MS: u64 = 3000;

/// Maneja una conexión `EventSource` COMPLETA, con reconexión: abre el stream
/// `text/event-stream`, parsea con [`SseParser`] y llama `on_open` / `on_event`
/// / `on_error`. Al cortarse (EOF o error de red) reconecta tras `retry` ms
/// (default 3000, o el `retry:` que mandó el server) reenviando el
/// `Last-Event-ID`. Termina cuando `cancelled()` da `true` (se chequea entre
/// lecturas y durante la espera de reconexión, gracias a un read-timeout
/// corto). **Bloquea** — pensado para un thread dedicado del chrome.
pub fn run_eventsource(
    url: &str,
    cancelled: &dyn Fn() -> bool,
    mut on_open: impl FnMut(),
    mut on_event: impl FnMut(&SseEvent),
    mut on_error: impl FnMut(),
) {
    use std::io::Read;
    use std::time::Duration;

    let mut last_id = String::new();
    let mut retry_ms = DEFAULT_RETRY_MS;
    while !cancelled() {
        // Read-timeout corto: un read que vence devuelve error de timeout, que
        // tratamos como "re-chequear cancel y seguir leyendo" — así un stream
        // ocioso no bloquea la cancelación ni se da por muerto.
        let agent = ureq::AgentBuilder::new()
            .timeout_read(Duration::from_secs(2))
            .build();
        let mut req = agent.get(url).set("Accept", "text/event-stream");
        if !last_id.is_empty() {
            req = req.set("Last-Event-ID", &last_id);
        }
        let connected = match req.call() {
            Ok(resp) => {
                on_open();
                let mut reader = resp.into_reader();
                let mut parser = SseParser::new();
                let mut buf = [0u8; 4096];
                loop {
                    if cancelled() {
                        return;
                    }
                    match reader.read(&mut buf) {
                        Ok(0) => break, // EOF → reconectar
                        Ok(n) => {
                            let chunk = String::from_utf8_lossy(&buf[..n]);
                            for ev in parser.feed(&chunk) {
                                last_id = ev.last_id.clone();
                                on_event(&ev);
                            }
                            if let Some(r) = parser.retry() {
                                retry_ms = r;
                            }
                        }
                        Err(e) if is_timeout(&e) => continue,
                        Err(_) => break, // error de red → reconectar
                    }
                }
                true
            }
            Err(_) => false, // no conectó
        };
        let _ = connected;
        if cancelled() {
            return;
        }
        on_error();
        // Espera de reconexión, chequeando cancel cada 100ms.
        let mut waited = 0u64;
        while waited < retry_ms {
            if cancelled() {
                return;
            }
            std::thread::sleep(Duration::from_millis(100));
            waited += 100;
        }
    }
}

/// ¿El error de lectura es un timeout (no un fallo real de la conexión)?
fn is_timeout(e: &std::io::Error) -> bool {
    matches!(
        e.kind(),
        std::io::ErrorKind::WouldBlock | std::io::ErrorKind::TimedOut
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn evento_simple_message() {
        let mut p = SseParser::new();
        let evs = p.feed("data: hola\n\n");
        assert_eq!(evs, vec![SseEvent { event_type: "message".into(), data: "hola".into(), last_id: String::new() }]);
    }

    #[test]
    fn evento_con_tipo_e_id() {
        let mut p = SseParser::new();
        let evs = p.feed("event: ping\nid: 42\ndata: x\n\n");
        assert_eq!(evs, vec![SseEvent { event_type: "ping".into(), data: "x".into(), last_id: "42".into() }]);
        // El id persiste para el evento siguiente sin id propio.
        let evs2 = p.feed("data: y\n\n");
        assert_eq!(evs2[0].last_id, "42");
        assert_eq!(evs2[0].event_type, "message");
    }

    #[test]
    fn data_multilinea_se_une_con_newline() {
        let mut p = SseParser::new();
        let evs = p.feed("data: a\ndata: b\ndata: c\n\n");
        assert_eq!(evs[0].data, "a\nb\nc");
    }

    #[test]
    fn comentarios_y_blanco_sin_data_no_despachan() {
        let mut p = SseParser::new();
        assert!(p.feed(": keep-alive\n\n").is_empty());
        assert!(p.feed("\n").is_empty());
    }

    #[test]
    fn chunks_partidos_a_la_mitad_se_buffean() {
        let mut p = SseParser::new();
        assert!(p.feed("data: ho").is_empty()); // línea incompleta
        assert!(p.feed("la\n").is_empty()); // línea completa, falta el blanco
        let evs = p.feed("\n"); // ahora sí despacha
        assert_eq!(evs[0].data, "hola");
    }

    #[test]
    fn crlf_y_retry() {
        let mut p = SseParser::new();
        let evs = p.feed("retry: 5000\r\ndata: z\r\n\r\n");
        assert_eq!(evs[0].data, "z");
        assert_eq!(p.retry(), Some(5000));
    }

    #[test]
    fn campo_sin_espacio_tras_colon() {
        // `data:x` (sin espacio) es válido — sólo se recorta UN espacio si hay.
        let mut p = SseParser::new();
        let evs = p.feed("data:x\n\n");
        assert_eq!(evs[0].data, "x");
    }
}
