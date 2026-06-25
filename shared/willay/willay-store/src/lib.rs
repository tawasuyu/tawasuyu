//! `willay-store` — el índice compartido del centro de eventos sobre `sled`.
//!
//! Append-only y ordenado por tiempo, igual que el historial de `pata-notify`:
//! la clave es `ts_usec ++ id` big-endian, así `iter()` sale en orden temporal
//! ascendente y el `id` desempata (y deduplica: re-emitir un [`Evento`] idéntico
//! reescribe la misma clave en vez de duplicar).
//!
//! Guarda [`Evento`]s livianos que **referencian** el dato pesado en su
//! productor (el PNG en hapiy, la notif entera en el sled de `pata-notify`) — no
//! centraliza payloads. `sled` no es multi-proceso: las escrituras se embudan
//! por el **escritor único** que lo envuelve (el daemon willay, bloque futuro).
//! Ver `shared/willay/SDD.md`.

use std::path::PathBuf;

use willay_core::{Clase, Evento};

/// Acceso al índice de eventos. Clonable y barato (`sled` es `Arc` por dentro).
#[derive(Clone)]
pub struct Indice {
    // Mantiene viva la `Db` mientras exista un `Indice`; sled hace flush al drop.
    #[allow(dead_code)]
    db: sled::Db,
    tree: sled::Tree,
}

impl Indice {
    /// Abre el índice en `$XDG_DATA_HOME/willay` (persiste entre sesiones).
    pub fn open() -> anyhow::Result<Self> {
        let dir = data_dir();
        std::fs::create_dir_all(&dir)?;
        let db = sled::open(dir.join("indice"))?;
        let tree = db.open_tree("eventos")?;
        Ok(Self { db, tree })
    }

    /// Índice efímero en memoria — fallback si `open` falla y sustrato de tests.
    pub fn temporary() -> anyhow::Result<Self> {
        let db = sled::Config::new().temporary(true).open()?;
        let tree = db.open_tree("eventos")?;
        Ok(Self { db, tree })
    }

    /// Clave de un evento: `ts_usec` (8) ++ `id` (32), big-endian. Orden temporal
    /// ascendente con el id como desempate y dedup.
    fn clave(e: &Evento) -> Vec<u8> {
        let mut k = Vec::with_capacity(8 + 32);
        k.extend_from_slice(&e.ts_usec.to_be_bytes());
        k.extend_from_slice(&e.id);
        k
    }

    /// Agrega (o reescribe, si ya existía idéntico) un evento al índice.
    pub fn append(&self, e: &Evento) -> anyhow::Result<()> {
        let val = postcard::to_stdvec(e)?;
        self.tree.insert(Self::clave(e), val)?;
        Ok(())
    }

    /// Todo el índice en orden temporal **ascendente** (más viejo primero).
    pub fn listar(&self) -> anyhow::Result<Vec<Evento>> {
        let mut out = Vec::new();
        for kv in self.tree.iter() {
            let (_, v) = kv?;
            out.push(postcard::from_bytes(&v)?);
        }
        Ok(out)
    }

    /// Los `limite` eventos más **recientes**, en orden descendente (lo nuevo
    /// arriba) — el orden por defecto del feed. La iteración inversa de sled
    /// hace barato traer sólo la cola.
    pub fn recientes(&self, limite: usize) -> anyhow::Result<Vec<Evento>> {
        let mut out = Vec::with_capacity(limite.min(1024));
        for kv in self.tree.iter().rev() {
            if out.len() >= limite {
                break;
            }
            let (_, v) = kv?;
            out.push(postcard::from_bytes(&v)?);
        }
        Ok(out)
    }

    /// Los eventos de una clase, recientes primero, hasta `limite`. Faceta por tipo.
    pub fn por_clase(&self, clase: Clase, limite: usize) -> anyhow::Result<Vec<Evento>> {
        let mut out = Vec::new();
        for kv in self.tree.iter().rev() {
            if out.len() >= limite {
                break;
            }
            let (_, v) = kv?;
            let e: Evento = postcard::from_bytes(&v)?;
            if e.clase == clase {
                out.push(e);
            }
        }
        Ok(out)
    }

    /// Eventos en `[desde_usec, hasta_usec)` (orden ascendente). Faceta por tiempo:
    /// el panel arma «Hoy / Ayer / …» acotando el rango. El barrido de claves de
    /// sled es por rango, sin escanear todo el árbol.
    pub fn rango(&self, desde_usec: u64, hasta_usec: u64) -> anyhow::Result<Vec<Evento>> {
        let desde = desde_usec.to_be_bytes().to_vec();
        let hasta = hasta_usec.to_be_bytes().to_vec();
        let mut out = Vec::new();
        for kv in self.tree.range(desde..hasta) {
            let (_, v) = kv?;
            out.push(postcard::from_bytes(&v)?);
        }
        Ok(out)
    }

    /// Búsqueda **literal** (case-insensitive) sobre título/cuerpo/origen,
    /// recientes primero, hasta `limite`. Es el registro instantáneo de la
    /// búsqueda; el registro semántico (RAG) vive aparte (widget `rag`).
    pub fn buscar(&self, aguja: &str, limite: usize) -> anyhow::Result<Vec<Evento>> {
        let aguja = aguja.to_lowercase();
        let mut out = Vec::new();
        for kv in self.tree.iter().rev() {
            if out.len() >= limite {
                break;
            }
            let (_, v) = kv?;
            let e: Evento = postcard::from_bytes(&v)?;
            if e.coincide(&aguja) {
                out.push(e);
            }
        }
        Ok(out)
    }

    /// Cantidad de eventos en el índice.
    pub fn len(&self) -> usize {
        self.tree.len()
    }

    /// `true` si el índice está vacío.
    pub fn is_empty(&self) -> bool {
        self.tree.is_empty()
    }

    /// Vacía el índice.
    pub fn clear(&self) -> anyhow::Result<()> {
        self.tree.clear()?;
        Ok(())
    }
}

/// `$XDG_DATA_HOME/willay`, con fallback a `~/.local/share/willay`.
fn data_dir() -> PathBuf {
    let base = std::env::var_os("XDG_DATA_HOME")
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|h| PathBuf::from(h).join(".local/share")))
        .unwrap_or_else(std::env::temp_dir);
    base.join("willay")
}

#[cfg(test)]
mod tests {
    use super::*;
    use willay_core::Payload;

    fn ev(clase: Clase, ts: u64, titulo: &str, cuerpo: &str) -> Evento {
        Evento::nuevo(clase, ts, "test", titulo, cuerpo, Payload::Nada)
    }

    fn indice_con(eventos: &[Evento]) -> Indice {
        let ix = Indice::temporary().unwrap();
        for e in eventos {
            ix.append(e).unwrap();
        }
        ix
    }

    #[test]
    fn listar_sale_en_orden_temporal_ascendente() {
        let ix = indice_con(&[
            ev(Clase::Clip, 300, "c", ""),
            ev(Clase::Clip, 100, "a", ""),
            ev(Clase::Clip, 200, "b", ""),
        ]);
        let ts: Vec<u64> = ix.listar().unwrap().iter().map(|e| e.ts_usec).collect();
        assert_eq!(ts, vec![100, 200, 300]);
    }

    #[test]
    fn recientes_sale_descendente_y_capado() {
        let ix = indice_con(&[
            ev(Clase::Clip, 100, "a", ""),
            ev(Clase::Clip, 200, "b", ""),
            ev(Clase::Clip, 300, "c", ""),
        ]);
        let r = ix.recientes(2).unwrap();
        assert_eq!(r.len(), 2);
        assert_eq!(r[0].ts_usec, 300);
        assert_eq!(r[1].ts_usec, 200);
    }

    #[test]
    fn append_idempotente_para_evento_identico() {
        let e = ev(Clase::Clip, 100, "a", "");
        let ix = indice_con(&[e.clone(), e.clone(), e]);
        assert_eq!(ix.len(), 1, "re-emitir idéntico no duplica");
    }

    #[test]
    fn por_clase_filtra() {
        let ix = indice_con(&[
            ev(Clase::Notificacion, 100, "n", ""),
            ev(Clase::Captura, 200, "cap", ""),
            ev(Clase::Notificacion, 300, "n2", ""),
        ]);
        let notifs = ix.por_clase(Clase::Notificacion, 10).unwrap();
        assert_eq!(notifs.len(), 2);
        assert!(notifs.iter().all(|e| e.clase == Clase::Notificacion));
        // Recientes primero.
        assert_eq!(notifs[0].ts_usec, 300);
    }

    #[test]
    fn rango_acota_por_tiempo() {
        let ix = indice_con(&[
            ev(Clase::Clip, 100, "a", ""),
            ev(Clase::Clip, 200, "b", ""),
            ev(Clase::Clip, 300, "c", ""),
        ]);
        let r = ix.rango(150, 300).unwrap(); // [150, 300): incluye 200, excluye 300
        let ts: Vec<u64> = r.iter().map(|e| e.ts_usec).collect();
        assert_eq!(ts, vec![200]);
    }

    #[test]
    fn buscar_literal_sobre_campos() {
        let ix = indice_con(&[
            ev(Clase::Clip, 100, "API Key", "secreto"),
            ev(Clase::Clip, 200, "otra cosa", "cuerpo"),
        ]);
        let hits = ix.buscar("api", 10).unwrap();
        assert_eq!(hits.len(), 1);
        assert_eq!(hits[0].titulo, "API Key");
        assert_eq!(ix.buscar("", 10).unwrap().len(), 2, "vacío trae todo");
    }

    #[test]
    fn clear_vacia() {
        let ix = indice_con(&[ev(Clase::Clip, 1, "a", "")]);
        assert!(!ix.is_empty());
        ix.clear().unwrap();
        assert!(ix.is_empty());
    }
}
