//! `pluma-cotejo-llm` — el resumidor **IA** de diferencias de un cotejo.
//!
//! `pluma-cotejo` produce, por cada sección, una línea determinista
//! (`ResumidorTextual`: "≈ reformulado · 78% en común"). Este crate la
//! reemplaza, para las secciones que de verdad cambiaron, por una frase que un
//! modelo redacta describiendo *qué* cambió en sentido o matiz — no el diff
//! literal. Es el gemelo de `pluma-transform-llm`: aísla la dependencia de
//! `pluma-llm` del núcleo `pluma-cotejo`, que sigue sin saber de modelos.
//!
//! El flujo es asíncrono (una request por sección cambiada). El caller (la app)
//! lo corre en un hilo aparte con su runtime y, al terminar, vuelca las líneas
//! sobre los átomos del lienzo de diferencias. Las secciones idénticas,
//! agregadas o eliminadas **no** llaman al modelo: su línea es determinista
//! (no hay nada que interpretar, o el "qué cambió" es el propio texto).

#![forbid(unsafe_code)]

use std::collections::HashMap;

use pluma_cotejo::{ClaseCambio, SeccionCotejo};
use pluma_core::NarrativeAtom;
use pluma_llm_core::{ChatClient, ChatRequest};
use uuid::Uuid;

/// Lo mínimo que el resumidor necesita de una sección: su clase, su fuerza de
/// coincidencia y los textos de ambos lados (los que existan). Es *owned* para
/// poder moverse a un hilo de trabajo sin atar referencias al modelo.
#[derive(Debug, Clone, PartialEq)]
pub struct ItemDiff {
    pub clase: ClaseCambio,
    pub similitud: f32,
    pub izq: Option<String>,
    pub der: Option<String>,
}

/// Arma los [`ItemDiff`] de un cotejo a partir de sus secciones y un índice de
/// átomos. El orden se preserva 1:1 con `secciones` (y con el lienzo de
/// diferencias), así las líneas resultantes se vuelcan por posición.
pub fn items_desde_secciones(
    secciones: &[SeccionCotejo],
    atoms: &HashMap<Uuid, NarrativeAtom>,
) -> Vec<ItemDiff> {
    let texto = |id: Option<Uuid>| -> Option<String> {
        id.and_then(|i| atoms.get(&i)).map(|a| a.content.to_string())
    };
    secciones
        .iter()
        .map(|s| ItemDiff {
            clase: s.clase,
            similitud: s.similitud,
            izq: texto(s.izq),
            der: texto(s.der),
        })
        .collect()
}

/// `true` si la sección requiere al modelo: sólo los pares cambiados (similar /
/// divergente). Idénticas, agregadas y eliminadas se resuelven sin red.
fn necesita_modelo(clase: ClaseCambio) -> bool {
    matches!(clase, ClaseCambio::Similar | ClaseCambio::Divergente)
}

const SYSTEM: &str = "Compará dos versiones de un mismo párrafo. En UNA frase \
breve en español (máximo 14 palabras), describí QUÉ cambió en sentido, matiz o \
información — no enumeres el diff literal palabra por palabra. Respondé sólo la \
frase: sin comillas, sin prefijos, sin punto final.";

/// Resume cada sección. Devuelve una línea por ítem, en el mismo orden. Las
/// secciones cambiadas consultan al modelo; las demás llevan su línea
/// determinista (mismo lenguaje de glifos que `pluma_cotejo::ResumidorTextual`).
///
/// Las consultas a las secciones cambiadas se disparan **en paralelo** con
/// `join_all` (las deterministas resuelven al instante). `join_all` conserva el
/// orden de entrada, así las líneas siguen alineadas 1:1 con las secciones. Si
/// alguna consulta falla, devuelve el primer `Err` — el caller conserva las
/// líneas textuales que ya tenía y avisa, sin perder el cotejo.
///
/// La concurrencia no está acotada: dispara una request por sección cambiada a
/// la vez. Para documentos con muchísimos cambios y un backend con rate-limit
/// estricto convendría un `buffered(N)`; para tamaños normales, `join_all` va.
pub async fn resumir_diferencias(
    items: &[ItemDiff],
    chat: &dyn ChatClient,
) -> Result<Vec<String>, String> {
    let resultados = futures::future::join_all(items.iter().map(|it| resumir_item(it, chat))).await;
    // Colecta a `Result<Vec<_>, _>`: corta en el primer error, preserva orden.
    resultados.into_iter().collect()
}

/// Resume un ítem: consulta al modelo si la sección cambió, o devuelve la línea
/// determinista. Aislado para poder lanzarlo concurrentemente con `join_all`.
async fn resumir_item(it: &ItemDiff, chat: &dyn ChatClient) -> Result<String, String> {
    if !necesita_modelo(it.clase) {
        return Ok(linea_textual(it));
    }
    let izq = it.izq.as_deref().unwrap_or("");
    let der = it.der.as_deref().unwrap_or("");
    let user = format!("Original:\n«{izq}»\n\nNueva:\n«{der}»");
    let req = ChatRequest::una_vuelta(user, 96)
        .con_sistema(SYSTEM)
        .con_temperatura(0.2);
    let resp = chat.complete(&req).await.map_err(|e| format!("LLM: {e:?}"))?;
    let txt = limpiar(&resp.content);
    if txt.is_empty() {
        return Ok(linea_textual(it));
    }
    let glifo = if matches!(it.clase, ClaseCambio::Similar) { "≈" } else { "✗" };
    Ok(format!("{glifo} {txt}"))
}

/// Línea determinista de respaldo — espejo de `pluma_cotejo::ResumidorTextual`
/// para las secciones que no van al modelo (o cuando éste devuelve vacío).
fn linea_textual(it: &ItemDiff) -> String {
    let pct = (it.similitud * 100.0).round() as i32;
    match it.clase {
        ClaseCambio::Identica => "≡ sin cambios".to_string(),
        ClaseCambio::Similar => format!("≈ reformulado · {pct}% en común"),
        ClaseCambio::Divergente => format!("✗ reescrito · {pct}% en común"),
        ClaseCambio::Agregada => format!("＋ agregado: {}", recorte(it.der.as_deref().unwrap_or(""))),
        ClaseCambio::Eliminada => {
            format!("－ eliminado: {}", recorte(it.izq.as_deref().unwrap_or("")))
        }
    }
}

/// Limpia la respuesta del modelo: colapsa saltos, recorta comillas envolventes
/// y espacios, y acota a una línea legible. Devuelve `""` si queda vacía.
fn limpiar(s: &str) -> String {
    let mut t = s.trim().replace('\n', " ");
    // Quita comillas envolventes si las puso el modelo pese a la instrucción.
    for (a, b) in [('«', '»'), ('"', '"'), ('\'', '\''), ('“', '”')] {
        if t.starts_with(a) && t.ends_with(b) && t.chars().count() >= 2 {
            let inner: String = t.chars().skip(1).take(t.chars().count() - 2).collect();
            t = inner.trim().to_string();
        }
    }
    // Punto final colgante → fuera (la instrucción lo pide, pero por las dudas).
    while t.ends_with('.') {
        t.pop();
    }
    recorte(t.trim())
}

/// Recorta a un preview de una línea sin romper UTF-8.
fn recorte(s: &str) -> String {
    const LIM: usize = 90;
    let mut t = s.replace('\n', " ");
    if t.chars().count() > LIM {
        t = t.chars().take(LIM).collect::<String>();
        t.push('…');
    }
    t
}

#[cfg(test)]
mod pruebas {
    use super::*;
    use pluma_llm_mock::MockChatClient;

    fn item(clase: ClaseCambio, sim: f32, izq: Option<&str>, der: Option<&str>) -> ItemDiff {
        ItemDiff {
            clase,
            similitud: sim,
            izq: izq.map(|s| s.to_string()),
            der: der.map(|s| s.to_string()),
        }
    }

    #[tokio::test]
    async fn una_linea_por_item_y_orden_preservado() {
        let items = vec![
            item(ClaseCambio::Identica, 1.0, Some("a"), Some("a")),
            item(ClaseCambio::Similar, 0.7, Some("el gato come"), Some("el gato comió")),
            item(ClaseCambio::Agregada, 0.0, None, Some("párrafo nuevo")),
        ];
        let chat = MockChatClient::default()
            .con_respuesta("comió", "cambia el verbo a pasado");
        let lineas = resumir_diferencias(&items, &chat).await.unwrap();
        assert_eq!(lineas.len(), 3);
        // Idéntica: determinista, sin modelo.
        assert_eq!(lineas[0], "≡ sin cambios");
        // Similar: glifo ≈ + texto del modelo.
        assert_eq!(lineas[1], "≈ cambia el verbo a pasado");
        // Agregada: determinista.
        assert!(lineas[2].starts_with("＋ agregado:"));
    }

    #[tokio::test]
    async fn solo_las_cambiadas_consultan_al_modelo() {
        // Si el modelo se invocara para una idéntica, el eco "mock:: " saldría
        // en la línea. Como no lo hace, la idéntica queda determinista.
        let items = vec![
            item(ClaseCambio::Identica, 1.0, Some("x"), Some("x")),
            item(ClaseCambio::Eliminada, 0.0, Some("se fue"), None),
        ];
        let chat = MockChatClient::default(); // sólo eco
        let lineas = resumir_diferencias(&items, &chat).await.unwrap();
        assert_eq!(lineas[0], "≡ sin cambios");
        assert!(lineas[1].starts_with("－ eliminado:"));
        assert!(!lineas[0].contains("mock::"));
        assert!(!lineas[1].contains("mock::"));
    }

    #[tokio::test]
    async fn divergente_usa_glifo_de_reescritura() {
        let items = vec![item(
            ClaseCambio::Divergente,
            0.1,
            Some("corría sobre GPUI"),
            Some("corre sobre Llimphi"),
        )];
        let chat = MockChatClient::default()
            .con_respuesta("Llimphi", "reemplaza el motor GPUI por Llimphi");
        let lineas = resumir_diferencias(&items, &chat).await.unwrap();
        assert_eq!(lineas[0], "✗ reemplaza el motor GPUI por Llimphi");
    }

    #[tokio::test]
    async fn join_all_preserva_el_orden_con_varias_cambiadas() {
        // Varias secciones cambiadas intercaladas con idénticas: el resultado
        // debe quedar 1:1 con la entrada pese a correr en paralelo.
        let items = vec![
            item(ClaseCambio::Divergente, 0.1, Some("uno viejo"), Some("uno ALFA")),
            item(ClaseCambio::Identica, 1.0, Some("medio"), Some("medio")),
            item(ClaseCambio::Similar, 0.6, Some("tres viejo"), Some("tres BETA")),
            item(ClaseCambio::Divergente, 0.2, Some("cuatro viejo"), Some("cuatro GAMMA")),
        ];
        let chat = MockChatClient::default()
            .con_respuesta("ALFA", "primero")
            .con_respuesta("BETA", "tercero")
            .con_respuesta("GAMMA", "cuarto");
        let lineas = resumir_diferencias(&items, &chat).await.unwrap();
        assert_eq!(lineas.len(), 4);
        assert_eq!(lineas[0], "✗ primero");
        assert_eq!(lineas[1], "≡ sin cambios");
        assert_eq!(lineas[2], "≈ tercero");
        assert_eq!(lineas[3], "✗ cuarto");
    }

    #[test]
    fn limpiar_quita_comillas_y_punto() {
        assert_eq!(limpiar("«hola mundo.»"), "hola mundo");
        assert_eq!(limpiar("  texto simple  "), "texto simple");
        assert_eq!(limpiar("\"con comillas\""), "con comillas");
    }

    #[test]
    fn items_desde_secciones_resuelve_textos_y_orden() {
        let a = NarrativeAtom::new("texto izquierdo", "a");
        let b = NarrativeAtom::new("texto derecho", "b");
        let mut atoms = HashMap::new();
        atoms.insert(a.id, a.clone());
        atoms.insert(b.id, b.clone());
        let secciones = vec![SeccionCotejo {
            izq: Some(a.id),
            der: Some(b.id),
            similitud: 0.5,
            clase: ClaseCambio::Divergente,
        }];
        let items = items_desde_secciones(&secciones, &atoms);
        assert_eq!(items.len(), 1);
        assert_eq!(items[0].izq.as_deref(), Some("texto izquierdo"));
        assert_eq!(items[0].der.as_deref(), Some("texto derecho"));
        assert_eq!(items[0].clase, ClaseCambio::Divergente);
    }
}
