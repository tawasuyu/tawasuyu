//! Agrupado de mensajes en **hilos** (conversaciones).
//!
//! Algoritmo (JWZ simplificado): un union-find sobre el universo de
//! `Message-ID`s — los propios más los referenciados por `In-Reply-To` y
//! `References`. Dos mensajes terminan en el mismo hilo si comparten,
//! transitivamente, cualquier ancestro — aun si ese ancestro no está entre
//! los mensajes que tenemos (caso típico: dos respuestas a un mensaje que no
//! descargamos). No se hilea por asunto (evita fusionar conversaciones
//! distintas con el mismo "Re: …"); el asunto sólo da el título del hilo.

use std::collections::HashMap;

use crate::message::{Message, MessageId};

/// Un hilo: el conjunto de mensajes de una conversación.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Thread {
    /// Asunto normalizado (sin `Re:`/`Fwd:`) del mensaje más viejo del hilo.
    pub subject: String,
    /// IDs de los mensajes del hilo, **ordenados por fecha ascendente**.
    pub message_ids: Vec<MessageId>,
    /// Fecha del mensaje más reciente (para ordenar hilos en la bandeja).
    pub last_date: i64,
    /// Cantidad de mensajes sin leer en el hilo.
    pub unread: usize,
}

/// Construye los hilos a partir de un conjunto de mensajes. El resultado va
/// ordenado por `last_date` **descendente** (lo más reciente primero), que es
/// el orden natural de una bandeja de entrada.
pub fn build_threads(messages: &[Message]) -> Vec<Thread> {
    let mut uf = UnionFind::default();
    for m in messages {
        uf.make(&m.id.0);
        if let Some(irt) = &m.in_reply_to {
            uf.union(&m.id.0, &irt.0);
        }
        for r in &m.references {
            uf.union(&m.id.0, &r.0);
        }
    }

    // Agrupá los índices de mensaje por su raíz en el union-find.
    let mut groups: HashMap<String, Vec<usize>> = HashMap::new();
    for (i, m) in messages.iter().enumerate() {
        let root = uf.find(&m.id.0);
        groups.entry(root).or_default().push(i);
    }

    let mut threads: Vec<Thread> = groups
        .into_values()
        .map(|mut idx| {
            idx.sort_by_key(|&i| messages[i].date);
            let subject = normalize_subject(&messages[idx[0]].subject);
            let last_date = idx.iter().map(|&i| messages[i].date).max().unwrap_or(0);
            let unread = idx.iter().filter(|&&i| messages[i].is_unread()).count();
            let message_ids = idx.iter().map(|&i| messages[i].id.clone()).collect();
            Thread { subject, message_ids, last_date, unread }
        })
        .collect();

    // Más reciente primero; empate desestabilizado por asunto para que el
    // orden sea determinista entre corridas.
    threads.sort_by(|a, b| b.last_date.cmp(&a.last_date).then(a.subject.cmp(&b.subject)));
    threads
}

/// Quita prefijos de respuesta/reenvío (`Re:`, `RE:`, `Fwd:`, `Fw:`, `RV:`)
/// repetidos y espacios sobrantes. `"Re: Fwd: Hola"` → `"Hola"`.
pub fn normalize_subject(subject: &str) -> String {
    let mut s = subject.trim();
    loop {
        let lower = s.to_ascii_lowercase();
        let stripped = ["re:", "fwd:", "fw:", "rv:"]
            .iter()
            .find_map(|p| lower.starts_with(p).then(|| s[p.len()..].trim_start()));
        match stripped {
            Some(rest) => s = rest,
            None => break,
        }
    }
    s.to_string()
}

/// Union-find sobre strings (los `Message-ID`s). Path-halving + union por
/// tamaño implícito; suficiente para los volúmenes de un buzón.
#[derive(Default)]
struct UnionFind {
    parent: HashMap<String, String>,
}

impl UnionFind {
    fn make(&mut self, x: &str) {
        self.parent.entry(x.to_string()).or_insert_with(|| x.to_string());
    }

    fn find(&mut self, x: &str) -> String {
        self.make(x);
        let mut cur = x.to_string();
        loop {
            let p = self.parent[&cur].clone();
            if p == cur {
                return cur;
            }
            // path-halving: apuntá al abuelo.
            let gp = self.parent[&p].clone();
            self.parent.insert(cur.clone(), gp.clone());
            cur = p;
        }
    }

    fn union(&mut self, a: &str, b: &str) {
        let ra = self.find(a);
        let rb = self.find(b);
        if ra != rb {
            self.parent.insert(ra, rb);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::Address;
    use crate::message::{Flags, Message, MessageId, SignatureStatus};

    fn m(id: &str, subject: &str, date: i64, irt: Option<&str>, refs: &[&str]) -> Message {
        Message {
            id: MessageId(id.into()),
            from: Address::new("a@x.com"),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: subject.into(),
            date,
            in_reply_to: irt.map(|s| MessageId(s.into())),
            references: refs.iter().map(|s| MessageId((*s).into())).collect(),
            body_text: String::new(),
            body_html: None,
            flags: Flags::default(),
            signature: SignatureStatus::Unsigned,
            mailbox: "INBOX".into(),
            cuerpos: Vec::new(),
        }
    }

    #[test]
    fn normalize_quita_prefijos_repetidos() {
        assert_eq!(normalize_subject("Re: Fwd: Hola"), "Hola");
        assert_eq!(normalize_subject("RE: RV: Asunto"), "Asunto");
        assert_eq!(normalize_subject("Sin prefijo"), "Sin prefijo");
    }

    #[test]
    fn cadena_de_respuestas_es_un_hilo() {
        let msgs = vec![
            m("<1@x>", "Propuesta", 100, None, &[]),
            m("<2@x>", "Re: Propuesta", 200, Some("<1@x>"), &["<1@x>"]),
            m("<3@x>", "Re: Propuesta", 300, Some("<2@x>"), &["<1@x>", "<2@x>"]),
        ];
        let threads = build_threads(&msgs);
        assert_eq!(threads.len(), 1);
        assert_eq!(threads[0].subject, "Propuesta");
        assert_eq!(threads[0].message_ids.len(), 3);
        assert_eq!(threads[0].last_date, 300);
        // Orden interno ascendente por fecha.
        assert_eq!(threads[0].message_ids[0].0, "<1@x>");
    }

    #[test]
    fn dos_respuestas_a_un_padre_ausente_se_unen() {
        // No tenemos <0@x> (el root), pero dos respuestas lo referencian.
        let msgs = vec![
            m("<a@x>", "Re: Tema", 100, Some("<0@x>"), &["<0@x>"]),
            m("<b@x>", "Re: Tema", 200, Some("<0@x>"), &["<0@x>"]),
        ];
        let threads = build_threads(&msgs);
        assert_eq!(threads.len(), 1, "deberían unirse por el ancestro ausente");
        assert_eq!(threads[0].message_ids.len(), 2);
    }

    #[test]
    fn conversaciones_distintas_no_se_mezclan() {
        let msgs = vec![
            m("<1@x>", "Tema A", 100, None, &[]),
            m("<2@x>", "Tema B", 200, None, &[]),
        ];
        let threads = build_threads(&msgs);
        assert_eq!(threads.len(), 2);
        // Más reciente primero.
        assert_eq!(threads[0].subject, "Tema B");
    }

    #[test]
    fn cuenta_no_leidos_del_hilo() {
        let mut a = m("<1@x>", "T", 100, None, &[]);
        let mut b = m("<2@x>", "Re: T", 200, Some("<1@x>"), &["<1@x>"]);
        a.flags.seen = true; // leído
        b.flags.seen = false; // sin leer
        let threads = build_threads(&[a, b]);
        assert_eq!(threads[0].unread, 1);
    }
}
