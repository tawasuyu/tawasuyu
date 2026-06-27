//! El motor: dos funciones puras que el host envuelve con la red.
//!
//! 1. [`construir_request`] — `Conversacion` + `Agente` → [`ChatRequest`]
//!    multi-turno (con system prompt y, si el agente tiene control, el menú de
//!    capacidades atipay). El host hace el `.complete()` con `pluma-llm`.
//! 2. [`interpretar_respuesta`] — el texto crudo del modelo → `Vec<BloqueSalida>`
//!    (texto / código / acción de control validada). Es la **gama de outputs**.
//!
//! Ninguna toca sockets ni `tokio`: son sync y testeables. Mismo reparto que el
//! `LlmRequest`/`LlmResult` del shell.

use crate::agente::{Agente, Capacidades};
use crate::conversacion::{AccionPropuesta, BloqueSalida, Conversacion, EstadoAccion, Rol};
use pluma_llm_core::{ChatMessage, ChatRequest};

/// Persona por defecto si el agente no fija `system_prompt`.
const SYSTEM_DEFAULT: &str = "Sos un asistente del escritorio tawasuyu. Respondé claro y conciso, \
     en el idioma del usuario. Usá bloques de código cercados (```) para comandos o código.";

/// Arma el [`ChatRequest`] multi-turno desde la conversación + el agente.
///
/// El `system` es la persona del agente (o [`SYSTEM_DEFAULT`]); si el agente
/// tiene `capacidades.control`, se le anexan las instrucciones para proponer
/// acciones del catálogo atipay. Los `messages` son **todo el historial** de la
/// conversación traducido a `user`/`assistant` — así el modelo tiene memoria.
pub fn construir_request(conv: &Conversacion, agente: &Agente) -> ChatRequest {
    let mut system = if agente.system_prompt.trim().is_empty() {
        SYSTEM_DEFAULT.to_string()
    } else {
        agente.system_prompt.clone()
    };
    if agente.capacidades.control {
        system.push_str(&instrucciones_control(&agente.capacidades));
    }

    let messages = conv
        .turnos
        .iter()
        .map(|t| {
            let content = t.texto_plano();
            match t.rol {
                Rol::Usuario => ChatMessage::user(content),
                Rol::Asistente => ChatMessage::assistant(content),
            }
        })
        .collect::<Vec<_>>();

    ChatRequest {
        system: Some(system),
        messages,
        max_tokens: agente.max_tokens,
        temperature: agente.temperatura.clamp(0.0, 1.0),
    }
}

/// Instrucciones que se anexan al system de un agente con control: cómo proponer
/// una acción (bloque cercado `accion` con JSON `{"id","args"}`) y el menú de
/// ids válidos del catálogo. El usuario aprueba; el agente nunca ejecuta.
fn instrucciones_control(_cap: &Capacidades) -> String {
    // El catálogo se identifica por `id`; el modelo elige UNO. La línea de
    // comando la arma `atipay` (validada) — el modelo no puede inventar flags.
    let menu = atipay::Catalogo::estandar().prompt_menu_ids();
    format!(
        "\n\nAdemás de charlar, podés PROPONER acciones de control del escritorio. \
         Cuando quieras ejecutar una, incluí en tu respuesta un bloque cercado con \
         la etiqueta `accion` que contenga SÓLO un objeto JSON \
         {{\"id\":\"<id de la acción>\",\"args\":{{\"<param>\":\"<valor>\"}}}} — sin \
         markdown adentro. Podés acompañarlo de texto explicando qué hace. El usuario \
         revisa y aprueba: vos NUNCA la ejecutás. Usá EXACTAMENTE estos ids:\n{menu}"
    )
}

/// Parte el texto crudo del asistente en [`BloqueSalida`]s.
///
/// Reglas:
/// - Bloque cercado ```` ```accion ```` / ```` ```atipay ```` → se resuelve con
///   atipay a una [`AccionPropuesta`] validada (o un `Error` si no encaja).
/// - Cualquier otro bloque cercado → [`BloqueSalida::Codigo`] (con su lenguaje).
/// - El texto fuera de cercos → [`BloqueSalida::Texto`] (se descartan los vacíos).
/// - Tolerancia: si el agente tiene control y la respuesta entera es un objeto
///   JSON suelto, se interpreta como acción (como hace hoy `:hacé`).
pub fn interpretar_respuesta(texto: &str, agente: &Agente) -> Vec<BloqueSalida> {
    let crudo = texto.trim();
    if crudo.is_empty() {
        return Vec::new();
    }

    // Fallback: control + JSON suelto (sin cercos) → acción.
    if agente.capacidades.control && crudo.starts_with('{') && crudo.ends_with('}') {
        return vec![resolver_accion(crudo, agente)];
    }

    let mut bloques = Vec::new();
    let mut texto_acc: Vec<&str> = Vec::new();
    let mut en_cerco = false;
    let mut info = String::new();
    let mut cuerpo: Vec<&str> = Vec::new();

    let flush_texto = |acc: &mut Vec<&str>, bloques: &mut Vec<BloqueSalida>| {
        let t = acc.join("\n");
        let t = t.trim();
        if !t.is_empty() {
            bloques.push(BloqueSalida::Texto(t.to_string()));
        }
        acc.clear();
    };

    for linea in crudo.lines() {
        let trimmed = linea.trim_start();
        if let Some(resto) = trimmed.strip_prefix("```") {
            if en_cerco {
                // Cierra el cerco actual.
                let etiqueta = info.trim().to_lowercase();
                let contenido = cuerpo.join("\n");
                if etiqueta == "accion" || etiqueta == "atipay" {
                    bloques.push(resolver_accion(&contenido, agente));
                } else {
                    let lenguaje = (!etiqueta.is_empty()).then(|| etiqueta.clone());
                    bloques.push(BloqueSalida::Codigo {
                        lenguaje,
                        codigo: contenido,
                    });
                }
                cuerpo.clear();
                info.clear();
                en_cerco = false;
            } else {
                // Abre un cerco: primero descargá el texto acumulado.
                flush_texto(&mut texto_acc, &mut bloques);
                info = resto.trim().to_string();
                en_cerco = true;
            }
        } else if en_cerco {
            cuerpo.push(linea);
        } else {
            texto_acc.push(linea);
        }
    }

    // Cerco sin cerrar: rescatá el cuerpo como código para no perder contenido.
    if en_cerco && !cuerpo.is_empty() {
        let lenguaje = (!info.trim().is_empty()).then(|| info.trim().to_lowercase());
        bloques.push(BloqueSalida::Codigo {
            lenguaje,
            codigo: cuerpo.join("\n"),
        });
    }
    flush_texto(&mut texto_acc, &mut bloques);

    if bloques.is_empty() {
        bloques.push(BloqueSalida::Texto(crudo.to_string()));
    }
    bloques
}

/// Resuelve un fragmento JSON `{"id","args"}` a una [`AccionPropuesta`] validada
/// por atipay, o a un [`BloqueSalida::Error`] legible. Respeta la lista blanca de
/// superficies del agente.
fn resolver_accion(fragmento: &str, agente: &Agente) -> BloqueSalida {
    let raw = fragmento.trim();
    if raw.is_empty() || raw.eq_ignore_ascii_case("nada") {
        return BloqueSalida::Error("ninguna acción de control encaja".to_string());
    }
    // El modelo puede colar texto alrededor; quedate con el objeto JSON.
    let json = match (raw.find('{'), raw.rfind('}')) {
        (Some(i), Some(j)) if j > i => &raw[i..=j],
        _ => return BloqueSalida::Error("no entendí la elección del modelo".to_string()),
    };
    let inv: atipay::Invocacion = match serde_json::from_str(json) {
        Ok(inv) => inv,
        Err(_) => return BloqueSalida::Error("JSON de acción inválido".to_string()),
    };

    // Lista blanca por superficie (prefijo del id: "mirada.brillo" → "mirada").
    let prefijo = inv.id.split('.').next().unwrap_or("");
    if !agente.capacidades.permite_superficie(prefijo) {
        return BloqueSalida::Error(format!(
            "el agente no tiene permitida la superficie «{prefijo}»"
        ));
    }

    match atipay::Catalogo::estandar().plan(&inv) {
        Ok(plan) => BloqueSalida::Accion(AccionPropuesta {
            id: plan.id.clone(),
            linea_comando: plan.linea_comando(),
            peligro: plan.peligro.into(),
            estado: EstadoAccion::Propuesta,
        }),
        Err(e) => BloqueSalida::Error(e.to_string()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversacion::Peligro;
    use pluma_llm_core::Role;

    fn agente_charla() -> Agente {
        Agente::nuevo("Asistente")
    }
    fn agente_control() -> Agente {
        Agente::nuevo("Control").con_control()
    }

    #[test]
    fn request_lleva_persona_e_historial() {
        let mut conv = Conversacion::nueva("a1", 0);
        conv.agregar_usuario("hola", 1);
        conv.agregar_asistente(vec![BloqueSalida::Texto("¡hola!".into())], 2);
        conv.agregar_usuario("¿qué hora es?", 3);

        let ag = agente_charla().con_persona("Sos pirata.");
        let req = construir_request(&conv, &ag);
        assert_eq!(req.system.as_deref(), Some("Sos pirata."));
        assert_eq!(req.messages.len(), 3);
        assert_eq!(req.messages[0].role, Role::User);
        assert_eq!(req.messages[1].role, Role::Assistant);
        assert_eq!(req.messages[2].content, "¿qué hora es?");
    }

    #[test]
    fn control_anexa_menu_al_system() {
        let conv = Conversacion::nueva("a1", 0);
        let req_charla = construir_request(&conv, &agente_charla());
        let req_control = construir_request(&conv, &agente_control());
        assert!(!req_charla.system.as_deref().unwrap().contains("PROPONER acciones"));
        assert!(req_control.system.as_deref().unwrap().contains("PROPONER acciones"));
    }

    #[test]
    fn interpreta_texto_y_codigo() {
        let bloques = interpretar_respuesta(
            "Probá esto:\n```sh\nls -la\n```\nY listo.",
            &agente_charla(),
        );
        assert_eq!(bloques.len(), 3);
        assert_eq!(bloques[0], BloqueSalida::Texto("Probá esto:".into()));
        assert_eq!(
            bloques[1],
            BloqueSalida::Codigo { lenguaje: Some("sh".into()), codigo: "ls -la".into() }
        );
        assert_eq!(bloques[2], BloqueSalida::Texto("Y listo.".into()));
    }

    #[test]
    fn interpreta_accion_cercada_valida() {
        // `sistema.brillo` existe en el catálogo estándar (Sistema).
        let resp = "Subo el brillo.\n```accion\n{\"id\":\"sistema.brillo\",\"args\":{\"nivel\":\"80\"}}\n```";
        let bloques = interpretar_respuesta(resp, &agente_control());
        assert_eq!(bloques.len(), 2);
        assert!(matches!(bloques[0], BloqueSalida::Texto(_)));
        match &bloques[1] {
            BloqueSalida::Accion(a) => {
                assert_eq!(a.id, "sistema.brillo");
                assert_eq!(a.estado, EstadoAccion::Propuesta);
                assert!(a.linea_comando.contains("80"));
            }
            otro => panic!("esperaba Accion, vino {otro:?}"),
        }
    }

    #[test]
    fn accion_con_id_desconocido_es_error() {
        let resp = "```accion\n{\"id\":\"inventada.cosa\"}\n```";
        let bloques = interpretar_respuesta(resp, &agente_control());
        assert!(matches!(bloques[0], BloqueSalida::Error(_)));
    }

    #[test]
    fn superficie_no_permitida_se_rechaza() {
        let mut ag = agente_control();
        ag.capacidades.superficies = vec!["mirada".into()]; // sólo mirada
        let resp = "```accion\n{\"id\":\"sistema.brillo\",\"args\":{\"nivel\":\"50\"}}\n```";
        let bloques = interpretar_respuesta(resp, &ag);
        match &bloques[0] {
            BloqueSalida::Error(e) => assert!(e.contains("sistema")),
            otro => panic!("esperaba Error, vino {otro:?}"),
        }
    }

    #[test]
    fn json_suelto_en_agente_control_es_accion() {
        let resp = "{\"id\":\"sistema.brillo\",\"args\":{\"nivel\":\"30\"}}";
        let bloques = interpretar_respuesta(resp, &agente_control());
        assert_eq!(bloques.len(), 1);
        assert!(matches!(&bloques[0], BloqueSalida::Accion(a) if a.peligro == Peligro::Seguro || a.peligro == Peligro::Reversible || a.peligro == Peligro::Disruptivo));
    }

    #[test]
    fn json_suelto_sin_control_es_texto() {
        let resp = "{\"id\":\"sistema.brillo\"}";
        let bloques = interpretar_respuesta(resp, &agente_charla());
        assert!(matches!(bloques[0], BloqueSalida::Texto(_)));
    }
}
