//! La conversación: un hilo multi-turno contra un agente, y la gama de bloques
//! de salida que un turno del asistente puede contener.

use serde::{Deserialize, Serialize};

/// Quién habló en un turno.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Rol {
    Usuario,
    Asistente,
}

/// Espejo local de `atipay::Peligro` — serializable y desacoplado del enum de
/// atipay (que el núcleo no necesita re-exportar).
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum Peligro {
    Seguro,
    Reversible,
    Disruptivo,
}

impl From<atipay::Peligro> for Peligro {
    fn from(p: atipay::Peligro) -> Self {
        match p {
            atipay::Peligro::Seguro => Peligro::Seguro,
            atipay::Peligro::Reversible => Peligro::Reversible,
            atipay::Peligro::Disruptivo => Peligro::Disruptivo,
        }
    }
}

impl Peligro {
    /// Etiqueta corta para la UI.
    pub fn etiqueta(self) -> &'static str {
        match self {
            Peligro::Seguro => "seguro",
            Peligro::Reversible => "reversible",
            Peligro::Disruptivo => "⚠ disruptivo",
        }
    }
}

/// Ciclo de vida de una acción propuesta por el agente. Arranca `Propuesta`; el
/// usuario la aprueba/rechaza; el host la ejecuta y reporta el desenlace.
#[derive(Clone, Copy, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub enum EstadoAccion {
    /// El agente la propuso; espera revisión del usuario.
    Propuesta,
    /// El usuario la aprobó; el host puede ejecutarla.
    Aprobada,
    /// El usuario la descartó.
    Rechazada,
    /// El host la corrió OK.
    Ejecutada,
    /// El host la corrió y falló.
    Fallida,
}

/// Una acción de control que el agente quiere ejecutar. La **línea de comando
/// la arma y valida atipay** a partir del `id` + args elegidos por el modelo —
/// imposible que el modelo invente flags. Nunca se auto-ejecuta.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq, Eq)]
pub struct AccionPropuesta {
    /// Id de la capacidad en el catálogo atipay.
    pub id: String,
    /// Línea de comando exacta, ya validada por atipay.
    pub linea_comando: String,
    /// Nivel de peligro reportado por el catálogo.
    pub peligro: Peligro,
    /// Estado del ciclo de vida.
    pub estado: EstadoAccion,
}

/// Un bloque de salida dentro de un turno. Es la **gama de outputs**: el texto
/// crudo del modelo se interpreta a esta lista (ver [`crate::motor`]).
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub enum BloqueSalida {
    /// Prosa (markdown). El grueso de una respuesta conversacional.
    Texto(String),
    /// Bloque de código con lenguaje opcional (de un cerco ```lang).
    Codigo {
        lenguaje: Option<String>,
        codigo: String,
    },
    /// Una acción de control propuesta (validada por atipay).
    Accion(AccionPropuesta),
    /// Algo no se pudo interpretar (JSON de acción inválido, id desconocido…).
    Error(String),
}

impl BloqueSalida {
    /// El texto que este bloque aporta al historial enviado al modelo en el
    /// próximo turno (para que recuerde lo que dijo). Las acciones se serializan
    /// de forma compacta y legible.
    pub fn texto_para_historial(&self) -> String {
        match self {
            BloqueSalida::Texto(t) => t.clone(),
            BloqueSalida::Codigo { lenguaje, codigo } => {
                let l = lenguaje.as_deref().unwrap_or("");
                format!("```{l}\n{codigo}\n```")
            }
            BloqueSalida::Accion(a) => format!("[acción: {} → {}]", a.id, a.linea_comando),
            BloqueSalida::Error(e) => format!("[error: {e}]"),
        }
    }
}

/// Un turno de la conversación.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Turno {
    pub rol: Rol,
    /// Para el usuario: normalmente un solo [`BloqueSalida::Texto`]. Para el
    /// asistente: los bloques interpretados de su respuesta.
    pub bloques: Vec<BloqueSalida>,
    /// Epoch en milisegundos. Lo fija el caller — el núcleo no lee el reloj.
    pub ts: u64,
}

impl Turno {
    /// Turno de usuario con texto plano.
    pub fn usuario(texto: impl Into<String>, ts: u64) -> Self {
        Self {
            rol: Rol::Usuario,
            bloques: vec![BloqueSalida::Texto(texto.into())],
            ts,
        }
    }

    /// Turno del asistente con bloques ya interpretados.
    pub fn asistente(bloques: Vec<BloqueSalida>, ts: u64) -> Self {
        Self {
            rol: Rol::Asistente,
            bloques,
            ts,
        }
    }

    /// El texto plano del turno, para reconstruir el historial del próximo
    /// `ChatRequest`.
    pub fn texto_plano(&self) -> String {
        self.bloques
            .iter()
            .map(|b| b.texto_para_historial())
            .collect::<Vec<_>>()
            .join("\n\n")
    }

    /// Las acciones propuestas en este turno, con su índice de bloque (para que
    /// el host pueda mutar su estado al aprobar/ejecutar).
    pub fn acciones(&self) -> impl Iterator<Item = (usize, &AccionPropuesta)> {
        self.bloques
            .iter()
            .enumerate()
            .filter_map(|(i, b)| match b {
                BloqueSalida::Accion(a) => Some((i, a)),
                _ => None,
            })
    }
}

/// Un hilo de conversación contra un agente.
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct Conversacion {
    /// Id estable (uuid v4).
    pub id: String,
    /// Qué agente la responde.
    pub agente_id: String,
    /// Título visible (se auto-deriva del primer mensaje si queda vacío).
    pub titulo: String,
    /// Los turnos, en orden cronológico.
    pub turnos: Vec<Turno>,
    /// Epoch ms de creación.
    pub creada: u64,
    /// Epoch ms del último turno.
    pub actualizada: u64,
}

impl Conversacion {
    /// Conversación vacía contra `agente_id`, marcada con `ahora` (epoch ms).
    pub fn nueva(agente_id: impl Into<String>, ahora: u64) -> Self {
        Self {
            id: uuid::Uuid::new_v4().to_string(),
            agente_id: agente_id.into(),
            titulo: String::new(),
            turnos: Vec::new(),
            creada: ahora,
            actualizada: ahora,
        }
    }

    /// Agrega un turno de usuario. Si la conversación no tenía título, lo deriva
    /// del texto (primeras palabras). Devuelve el índice del turno.
    pub fn agregar_usuario(&mut self, texto: impl Into<String>, ts: u64) -> usize {
        let texto = texto.into();
        if self.titulo.trim().is_empty() {
            self.titulo = derivar_titulo(&texto);
        }
        self.turnos.push(Turno::usuario(texto, ts));
        self.actualizada = ts;
        self.turnos.len() - 1
    }

    /// Agrega un turno del asistente con sus bloques ya interpretados.
    pub fn agregar_asistente(&mut self, bloques: Vec<BloqueSalida>, ts: u64) -> usize {
        self.turnos.push(Turno::asistente(bloques, ts));
        self.actualizada = ts;
        self.turnos.len() - 1
    }
}

/// Deriva un título corto de la primera línea de texto (hasta ~6 palabras).
fn derivar_titulo(texto: &str) -> String {
    let limpio = texto.trim().lines().next().unwrap_or("").trim();
    let recorte: String = limpio.split_whitespace().take(6).collect::<Vec<_>>().join(" ");
    if recorte.is_empty() {
        "Conversación".to_string()
    } else if recorte.chars().count() < limpio.chars().count() {
        format!("{recorte}…")
    } else {
        recorte
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn titulo_se_deriva_del_primer_mensaje() {
        let mut c = Conversacion::nueva("a1", 0);
        c.agregar_usuario("hola quiero listar archivos grandes del home", 10);
        assert_eq!(c.titulo, "hola quiero listar archivos grandes del…");
        assert_eq!(c.actualizada, 10);
        // El segundo mensaje no pisa el título.
        c.agregar_usuario("y ahora borralos", 20);
        assert_eq!(c.titulo, "hola quiero listar archivos grandes del…");
    }

    #[test]
    fn texto_plano_reconstruye_bloques() {
        let t = Turno::asistente(
            vec![
                BloqueSalida::Texto("probá esto:".into()),
                BloqueSalida::Codigo {
                    lenguaje: Some("sh".into()),
                    codigo: "ls -la".into(),
                },
            ],
            0,
        );
        assert_eq!(t.texto_plano(), "probá esto:\n\n```sh\nls -la\n```");
    }

    #[test]
    fn acciones_se_enumeran_con_indice() {
        let t = Turno::asistente(
            vec![
                BloqueSalida::Texto("subo el brillo".into()),
                BloqueSalida::Accion(AccionPropuesta {
                    id: "mirada.brillo".into(),
                    linea_comando: "mirada-ctl brillo 80".into(),
                    peligro: Peligro::Seguro,
                    estado: EstadoAccion::Propuesta,
                }),
            ],
            0,
        );
        let acc: Vec<_> = t.acciones().collect();
        assert_eq!(acc.len(), 1);
        assert_eq!(acc[0].0, 1);
        assert_eq!(acc[0].1.id, "mirada.brillo");
    }
}
