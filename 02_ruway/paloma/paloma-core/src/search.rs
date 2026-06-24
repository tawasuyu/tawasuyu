//! Búsqueda de texto sobre los mensajes cacheados.
//!
//! Una búsqueda simple, local e instantánea: parte la consulta en términos y
//! exige que **todos** aparezcan (AND) en algún campo del mensaje, sin importar
//! mayúsculas. Cada término puntúa según dónde matchea —asunto pesa más que
//! remitente, y remitente más que cuerpo— y la suma ordena los resultados. No
//! es semántica (eso lo dará `rimay` cuando haya un daemon de embeddings); es la
//! búsqueda exacta que cubre el 90% de "¿dónde estaba ese correo?".

use crate::message::Message;

/// Peso por campo donde matchea un término.
const W_SUBJECT: i32 = 3;
const W_FROM: i32 = 2;
const W_BODY: i32 = 1;

/// Normaliza la consulta a términos en minúsculas (separados por espacios),
/// descartando vacíos. Una consulta sin términos no matchea nada.
pub fn terms(query: &str) -> Vec<String> {
    query.split_whitespace().map(|t| t.to_lowercase()).collect()
}

/// Puntúa un mensaje contra `terms` (ya en minúsculas). Devuelve `Some(score)`
/// sólo si **todos** los términos aparecen en algún campo; `None` si falta uno
/// o si `terms` está vacío. El score suma el mejor peso de campo por término.
pub fn score(message: &Message, terms: &[String]) -> Option<i32> {
    if terms.is_empty() {
        return None;
    }
    let subject = message.subject.to_lowercase();
    let from = format!(
        "{} {}",
        message.from.display_name().to_lowercase(),
        message.from.email.to_lowercase()
    );
    let body = message.body_text.to_lowercase();

    let mut total = 0;
    for t in terms {
        let here = if subject.contains(t.as_str()) {
            W_SUBJECT
        } else if from.contains(t.as_str()) {
            W_FROM
        } else if body.contains(t.as_str()) {
            W_BODY
        } else {
            return None; // término ausente → no es un hit
        };
        total += here;
    }
    Some(total)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::address::Address;
    use crate::message::{Flags, MessageId, SignatureStatus};

    fn m(subject: &str, from_name: &str, from_email: &str, body: &str) -> Message {
        Message {
            id: MessageId("<x@x>".into()),
            from: Address::named(from_name, from_email),
            to: vec![],
            cc: vec![],
            bcc: vec![],
            subject: subject.into(),
            date: 0,
            in_reply_to: None,
            references: vec![],
            body_text: body.into(),
            body_html: None,
            flags: Flags::default(),
            signature: SignatureStatus::Unsigned,
            mailbox: "INBOX".into(),
            cuerpos: Vec::new(),
            signer: None,
        }
    }

    #[test]
    fn terms_normaliza_y_descarta_vacios() {
        assert_eq!(terms("  Hola   MUNDO "), vec!["hola", "mundo"]);
        assert!(terms("   ").is_empty());
    }

    #[test]
    fn matchea_en_asunto_pesa_mas() {
        let msg = m("Factura mayo", "Ana", "ana@x.com", "el cuerpo");
        assert_eq!(score(&msg, &terms("factura")), Some(W_SUBJECT));
        assert_eq!(score(&msg, &terms("ana")), Some(W_FROM));
        assert_eq!(score(&msg, &terms("cuerpo")), Some(W_BODY));
    }

    #[test]
    fn exige_todos_los_terminos() {
        let msg = m("Propuesta", "Ana", "ana@x.com", "reunión el jueves");
        // ambos presentes (asunto + cuerpo) → suma
        assert_eq!(score(&msg, &terms("propuesta jueves")), Some(W_SUBJECT + W_BODY));
        // uno ausente → no matchea
        assert_eq!(score(&msg, &terms("propuesta viernes")), None);
    }

    #[test]
    fn consulta_vacia_no_matchea() {
        let msg = m("x", "y", "y@z.com", "w");
        assert_eq!(score(&msg, &terms("")), None);
    }

    #[test]
    fn es_insensible_a_mayusculas() {
        let msg = m("INFORME Anual", "Bruno", "bruno@x.com", "");
        assert_eq!(score(&msg, &terms("informe ANUAL")), Some(W_SUBJECT * 2));
    }
}
