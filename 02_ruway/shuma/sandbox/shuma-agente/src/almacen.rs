//! Persistencia de agentes y conversaciones en sled.
//!
//! Dos árboles: `agentes` y `conversaciones`, ambos clave=`id` →
//! valor=JSON. JSON (no postcard) porque el contenido evoluciona campo a campo
//! con `serde(default)` y conviene poder inspeccionarlo a mano. El volumen es
//! chico (charlas de un usuario), así que listar deserializando todo el árbol
//! es de sobra.

use crate::agente::Agente;
use crate::conversacion::Conversacion;
use std::path::Path;
use thiserror::Error;

/// Errores de almacenamiento.
#[derive(Debug, Error)]
pub enum AlmacenError {
    #[error("sled: {0}")]
    Sled(#[from] sled::Error),
    #[error("serializar/deserializar: {0}")]
    Json(#[from] serde_json::Error),
}

/// El almacén persistente de la IA conversacional.
pub struct Almacen {
    agentes: sled::Tree,
    conversaciones: sled::Tree,
    _db: sled::Db,
}

impl Almacen {
    /// Abre (o crea) el almacén en `path`.
    pub fn abrir(path: impl AsRef<Path>) -> Result<Self, AlmacenError> {
        let db = sled::open(path)?;
        let agentes = db.open_tree("agentes")?;
        let conversaciones = db.open_tree("conversaciones")?;
        Ok(Self { agentes, conversaciones, _db: db })
    }

    // ── Agentes ──────────────────────────────────────────────────────────

    /// Inserta o actualiza un agente (clave = `agente.id`).
    pub fn guardar_agente(&self, a: &Agente) -> Result<(), AlmacenError> {
        self.agentes.insert(a.id.as_bytes(), serde_json::to_vec(a)?)?;
        Ok(())
    }

    /// Lee un agente por id.
    pub fn agente(&self, id: &str) -> Result<Option<Agente>, AlmacenError> {
        match self.agentes.get(id.as_bytes())? {
            Some(v) => Ok(Some(serde_json::from_slice(&v)?)),
            None => Ok(None),
        }
    }

    /// Todos los agentes, ordenados por nombre.
    pub fn agentes(&self) -> Result<Vec<Agente>, AlmacenError> {
        let mut out = Vec::new();
        for kv in self.agentes.iter() {
            let (_, v) = kv?;
            out.push(serde_json::from_slice::<Agente>(&v)?);
        }
        out.sort_by(|a, b| a.nombre.to_lowercase().cmp(&b.nombre.to_lowercase()));
        Ok(out)
    }

    /// Borra un agente. Las conversaciones que lo apuntaban quedan huérfanas
    /// (la UI las muestra como «agente eliminado»); no se borran en cascada.
    pub fn borrar_agente(&self, id: &str) -> Result<(), AlmacenError> {
        self.agentes.remove(id.as_bytes())?;
        Ok(())
    }

    /// Si no hay ningún agente, siembra dos por defecto («Asistente» de charla y
    /// «Control» con acciones del escritorio) y los devuelve. Idempotente: si ya
    /// hay agentes, no toca nada y devuelve los existentes.
    pub fn sembrar_defaults(&self) -> Result<Vec<Agente>, AlmacenError> {
        let existentes = self.agentes()?;
        if !existentes.is_empty() {
            return Ok(existentes);
        }
        // Por defecto pegan a Claude vía el CLI `claude` (Claude Code) — usa la
        // suscripción Pro/Max del usuario sin API key. Si `claude` no está
        // logueado, el host cae al `[ai.llm]` global o reporta el error.
        let claude = || wawa_config::LlmSettings {
            backend: "claude-cli".to_string(),
            ..Default::default()
        };
        let asistente = Agente::nuevo("Asistente")
            .con_descripcion("Charla general; sin tocar el sistema.")
            .con_backend(claude());
        let control = Agente::nuevo("Control")
            .con_descripcion("Maneja el escritorio: propone acciones que vos aprobás.")
            .con_persona(
                "Sos el controlador del escritorio tawasuyu. Ayudás al usuario a manejar la \
                 suite proponiendo acciones de control cuando hace falta.",
            )
            .con_backend(claude())
            .con_control();
        self.guardar_agente(&asistente)?;
        self.guardar_agente(&control)?;
        self.agentes()
    }

    // ── Conversaciones ───────────────────────────────────────────────────

    /// Inserta o actualiza una conversación (clave = `conv.id`).
    pub fn guardar_conversacion(&self, c: &Conversacion) -> Result<(), AlmacenError> {
        self.conversaciones.insert(c.id.as_bytes(), serde_json::to_vec(c)?)?;
        Ok(())
    }

    /// Lee una conversación por id.
    pub fn conversacion(&self, id: &str) -> Result<Option<Conversacion>, AlmacenError> {
        match self.conversaciones.get(id.as_bytes())? {
            Some(v) => Ok(Some(serde_json::from_slice(&v)?)),
            None => Ok(None),
        }
    }

    /// Todas las conversaciones, **más recientes primero** (por `actualizada`) —
    /// el orden del sidebar de las apps de IA.
    pub fn conversaciones(&self) -> Result<Vec<Conversacion>, AlmacenError> {
        let mut out = Vec::new();
        for kv in self.conversaciones.iter() {
            let (_, v) = kv?;
            out.push(serde_json::from_slice::<Conversacion>(&v)?);
        }
        out.sort_by(|a, b| b.actualizada.cmp(&a.actualizada));
        Ok(out)
    }

    /// Borra una conversación.
    pub fn borrar_conversacion(&self, id: &str) -> Result<(), AlmacenError> {
        self.conversaciones.remove(id.as_bytes())?;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::conversacion::{BloqueSalida, Conversacion};

    fn almacen_tmp() -> Almacen {
        // Path único por test sin tocar el reloj ni random: usa el nombre del
        // árbol temporal del propio sled en memoria via `Config::temporary`.
        let db = sled::Config::new().temporary(true).open().unwrap();
        let agentes = db.open_tree("agentes").unwrap();
        let conversaciones = db.open_tree("conversaciones").unwrap();
        Almacen { agentes, conversaciones, _db: db }
    }

    #[test]
    fn round_trip_agente() {
        let a = almacen_tmp();
        let ag = Agente::nuevo("DevOps").con_control();
        a.guardar_agente(&ag).unwrap();
        let leido = a.agente(&ag.id).unwrap().unwrap();
        assert_eq!(leido, ag);
        assert_eq!(a.agentes().unwrap().len(), 1);
        a.borrar_agente(&ag.id).unwrap();
        assert!(a.agente(&ag.id).unwrap().is_none());
    }

    #[test]
    fn sembrar_defaults_es_idempotente() {
        let a = almacen_tmp();
        let primera = a.sembrar_defaults().unwrap();
        assert_eq!(primera.len(), 2);
        let segunda = a.sembrar_defaults().unwrap();
        assert_eq!(segunda.len(), 2); // no duplica
    }

    #[test]
    fn conversaciones_ordenan_recientes_primero() {
        let a = almacen_tmp();
        let mut vieja = Conversacion::nueva("ag", 100);
        vieja.agregar_usuario("vieja", 100);
        let mut nueva = Conversacion::nueva("ag", 200);
        nueva.agregar_usuario("nueva", 200);
        a.guardar_conversacion(&vieja).unwrap();
        a.guardar_conversacion(&nueva).unwrap();
        let lista = a.conversaciones().unwrap();
        assert_eq!(lista[0].id, nueva.id);
        assert_eq!(lista[1].id, vieja.id);
    }

    #[test]
    fn round_trip_conversacion_con_bloques() {
        let a = almacen_tmp();
        let mut c = Conversacion::nueva("ag", 1);
        c.agregar_usuario("hola", 1);
        c.agregar_asistente(
            vec![BloqueSalida::Codigo { lenguaje: Some("rs".into()), codigo: "fn main(){}".into() }],
            2,
        );
        a.guardar_conversacion(&c).unwrap();
        assert_eq!(a.conversacion(&c.id).unwrap().unwrap(), c);
    }
}
