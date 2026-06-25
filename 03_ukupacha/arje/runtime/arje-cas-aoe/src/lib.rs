//! `arje-cas-aoe` — bridge entre el CAS compartido (`arje-cas`) y el protocolo
//! **Akasha Over Ether** (AoE). Sirve y trae blobs direccionados por contenido
//! (BLAKE3) sobre Ethernet crudo, reusando `wawa-explorer-aoe` (transporte raw
//! socket `AF_PACKET`) y `arje-cas` (almacén en disco).
//!
//! ## Por qué encaja sin fricción
//!
//! El invariante es el MISMO en los dos mundos: un objeto se identifica por
//! `blake3(bytes)`. AoE pide/provee `(id, bytes)` y el receptor verifica
//! `blake3(bytes) == id`; `arje-cas` guarda `bytes` bajo `blake3(bytes)`. Así un
//! blob del CAS de arje/hammer viaja a una wawa (o a otro nodo Linux) y se
//! verifica de punta a punta — transferencia **content-agnostic**: no importa si
//! el blob es un `.wasm`, un manifiesto de seed firmado, o un nodo del grafo. Es
//! el «CAS compartido» de la coordinación arje↔hammer puesto sobre el cable, sin
//! reimplementar el socket: lo aporta `wawa-explorer-aoe`.
//!
//! ## Capas
//!
//! - **Núcleo puro y testeable** (sin socket): [`objetos_del_cas`] (vuelca el CAS
//!   a un mapa `id→bytes` listo para servir) y [`absorber`] (verifica
//!   `blake3==id` y guarda en el CAS).
//! - **Funciones de red** (necesitan `CAP_NET_RAW`/root): [`servir_cas`],
//!   [`traer_al_cas`], [`anunciar`] — glue fino sobre `wawa_explorer_aoe::ClienteAoE`.

use std::collections::HashMap;
use std::time::Duration;

use wawa_explorer_aoe::{ClienteAoE, EstadisticasServir};

/// Identificador de objeto = BLAKE3 de su contenido. Igual que `arje_cas` y que
/// `akasha::ObjectId` (un alias de `[u8; 32]`), así que viaja sin conversión.
pub type ObjectId = [u8; 32];

/// Vuelca TODO el CAS local a un mapa `id → bytes`, listo para
/// [`ClienteAoE::servir`]. El CAS de arje/hammer son binarios/manifiestos/
/// bytecodes — un volumen modesto, no un dataset masivo, así que mantenerlo en
/// RAM mientras se sirve es razonable. (Un servir perezoso por-pedido contra el
/// disco queda como optimización futura si el CAS creciera.)
pub fn objetos_del_cas() -> anyhow::Result<HashMap<ObjectId, Vec<u8>>> {
    let mut mapa = HashMap::new();
    for sha in arje_cas::list_all_shas()? {
        let bytes = arje_cas::resolve(&sha)?;
        mapa.insert(sha, bytes);
    }
    Ok(mapa)
}

/// Verifica que `blake3(bytes) == id` y, si casa, lo guarda en el CAS local.
/// Devuelve el hash almacenado (`== id`). Rechaza un payload cuyo hash no case —
/// el MISMO chequeo que hace el kernel wawa al absorber: garantiza que lo que
/// guardamos es lo que **pedimos**, no lo que un peer adversario quiso colarnos.
pub fn absorber(id: &ObjectId, bytes: &[u8]) -> anyhow::Result<ObjectId> {
    let real = arje_cas::blake3_of(bytes);
    anyhow::ensure!(
        &real == id,
        "hash no casa: pedí {} y recibí {}",
        arje_cas::hex(id),
        arje_cas::hex(&real),
    );
    arje_cas::store(bytes)
}

/// Sirve TODO el CAS local por AoE durante `duracion` en la interfaz `iface`.
/// Responde cada `SolicitarObjeto(id)` cuyo `id` esté en el CAS (fragmentando
/// los objetos grandes). Requiere `CAP_NET_RAW`/root. Bloquea el hilo `duracion`.
pub fn servir_cas(iface: &str, duracion: Duration) -> anyhow::Result<EstadisticasServir> {
    let objetos = objetos_del_cas()?;
    let cliente = ClienteAoE::nuevo(iface).map_err(|e| anyhow::anyhow!("AoE socket: {e}"))?;
    cliente
        .servir(&objetos, duracion)
        .map_err(|e| anyhow::anyhow!("AoE servir: {e}"))
}

/// Pide el objeto `id` a la red AoE (con los reintentos del transporte) y, si
/// llega, lo guarda en el CAS local verificando `blake3==id` ([`absorber`]).
/// `Some(id)` si se absorbió, `None` si nadie respondió dentro de `timeout`.
/// Requiere `CAP_NET_RAW`/root.
pub fn traer_al_cas(
    iface: &str,
    id: &ObjectId,
    timeout: Duration,
) -> anyhow::Result<Option<ObjectId>> {
    let cliente = ClienteAoE::nuevo(iface).map_err(|e| anyhow::anyhow!("AoE socket: {e}"))?;
    match cliente
        .solicitar(*id, timeout)
        .map_err(|e| anyhow::anyhow!("AoE solicitar: {e}"))?
    {
        Some(bytes) => Ok(Some(absorber(id, &bytes)?)),
        None => Ok(None),
    }
}

/// Difunde `AnunciarRaiz(id)` — un faro AoE diciendo «tengo este objeto,
/// pedímelo». Útil tras publicar un release/manifiesto en el CAS. Requiere
/// `CAP_NET_RAW`/root.
pub fn anunciar(iface: &str, id: ObjectId) -> anyhow::Result<()> {
    let cliente = ClienteAoE::nuevo(iface).map_err(|e| anyhow::anyhow!("AoE socket: {e}"))?;
    cliente
        .anunciar_raiz(id)
        .map_err(|e| anyhow::anyhow!("AoE anunciar: {e}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Núcleo del bridge sin socket: volcar el CAS a un mapa servible y absorber
    /// un objeto entrante verificando su hash. Aísla el CAS en un dir temporal
    /// por `ENTE_CAS_ROOT` (un solo test → sin carrera con el env global).
    #[test]
    fn cas_a_mapa_y_absorber_verificado() {
        let tmp = std::env::temp_dir().join(format!("arje-cas-aoe-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&tmp);
        std::env::set_var("ENTE_CAS_ROOT", &tmp);

        // Dos blobs en el CAS → objetos_del_cas los vuelca con su id correcto.
        let a = arje_cas::store(b"objeto alfa").unwrap();
        let b = arje_cas::store(b"objeto beta").unwrap();
        let mapa = objetos_del_cas().unwrap();
        assert_eq!(mapa.len(), 2);
        assert_eq!(mapa.get(&a).unwrap().as_slice(), b"objeto alfa");
        assert_eq!(mapa.get(&b).unwrap().as_slice(), b"objeto beta");

        // absorber con hash correcto → guarda y devuelve el id; resoluble luego.
        let bytes = b"un objeto que llega por el cable AoE";
        let id = arje_cas::blake3_of(bytes);
        assert_eq!(absorber(&id, bytes).unwrap(), id);
        assert_eq!(arje_cas::resolve(&id).unwrap().as_slice(), bytes.as_slice());

        // absorber con un id que NO casa con los bytes → error, no se guarda.
        let id_falso = [0u8; 32];
        assert!(absorber(&id_falso, bytes).is_err());
        assert!(arje_cas::resolve(&id_falso).is_err());

        std::env::remove_var("ENTE_CAS_ROOT");
        let _ = std::fs::remove_dir_all(&tmp);
    }
}
