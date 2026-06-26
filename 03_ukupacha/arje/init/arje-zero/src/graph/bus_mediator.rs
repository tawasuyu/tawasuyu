//! Bus mediator: integración de `EnteGraph` con el bus interno.
//!
//! Responsabilidades:
//!   - Auth de Announce (verificar identidad reclamada contra SO_PEERCRED)
//!   - Registro de conexiones (`bus_connections` indexado por Ulid)
//!   - Forwarding de Invokes a proveedores
//!   - Tracking de invokes en vuelo (`pending_invokes` por seq)
//!   - Cleanup en cierre de conexión

use super::{EnteGraph, INHIBIT_TTL, SERVER_SEQ_FLAG};
use arje_bus::{
    BusMessage, BusPayload, BusRequest, BusResponse, EnteInfo, Liveness, PeerCreds, ResourceSample,
};
use arje_card::{Capability, EntityCard, Payload, WireCard};
use std::time::Instant;
use tokio::sync::{mpsc, oneshot};
use tracing::{debug, info, warn};
use ulid::Ulid;

/// Operaciones que requieren identidad verificada en el bus.
///
/// - `Announce`: establece bus_connections para forwarding.
/// - `UpdateCapabilities`: muta dynamic_provides del Ente — sólo el dueño.
///
/// Invoke, ListEntes y power-mgmt se aceptan anonymous — políticas por
/// capacidad se aplican aguas abajo, no aquí.
fn requires_auth(req: &BusRequest) -> bool {
    matches!(
        req,
        BusRequest::Announce { .. }
            | BusRequest::UpdateCapabilities { .. }
            | BusRequest::KillEnte { .. }
            | BusRequest::SpawnCardFromDisk { .. }
            | BusRequest::StopCardFromDisk { .. }
            | BusRequest::RunCard { .. }
    )
}

/// Directorio del card store. Override con `ARJE_CARDS_DIR`. Default
/// canónico `/etc/arje/cards.d`. Cada archivo es un `EntityCard` JSON.
fn cards_dir() -> std::path::PathBuf {
    std::env::var("ARJE_CARDS_DIR")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|_| std::path::PathBuf::from("/etc/arje/cards.d"))
}

/// Expande una Card cargada del store a la lista de Entes a encarnar.
///
/// Regla del **bundle de sesión**: una Card `Virtual` con `genesis` no
/// vacío representa un *conjunto*, no un proceso (p. ej. `session-gnome`
/// = los shims de `arje-compat`). El boot la materializa anexando sus
/// hijos al genesis de la Semilla (`profile::overlay_session`); en
/// runtime, spawnearla = encarnar **sus miembros**, no el envoltorio
/// `Virtual` (que no tiene proceso y dejaría los shims sin arrancar — el
/// grafo sólo encarna un nivel, no recurre `genesis`). Cualquier otra
/// Card (un único Ente, `Virtual` aislado, o con payload real) se
/// spawnea tal cual. Esto da al greeter una activación de sesión al login
/// con un solo `SpawnCardFromDisk { name: "session-gnome" }`.
fn expand_disk_bundle(card: EntityCard) -> Vec<EntityCard> {
    if matches!(card.payload, Payload::Virtual) && !card.genesis.is_empty() {
        card.genesis
    } else {
        vec![card]
    }
}

/// Lee `(memoria residente, nº de hilos)` de `/proc/<pid>`. `None` si el
/// proceso desapareció o `/proc` no es legible. RSS = 2º campo de `statm`
/// (páginas) × tamaño de página; los hilos se cuentan por entradas en `task/`.
/// El conteo de restarts NO sale de `/proc` (lo conoce el supervisor) — lo
/// agrega el handler.
fn read_proc_resources(pid: i32) -> Option<(u64, u32)> {
    const PAGE_SIZE: u64 = 4096; // x86_64 estándar
    let statm = std::fs::read_to_string(format!("/proc/{pid}/statm")).ok()?;
    let resident_pages: u64 = statm.split_whitespace().nth(1)?.parse().ok()?;
    let mem_bytes = resident_pages.saturating_mul(PAGE_SIZE);
    let nproc = std::fs::read_dir(format!("/proc/{pid}/task"))
        .map(|rd| rd.flatten().count() as u32)
        .unwrap_or(1);
    Some((mem_bytes, nproc))
}

impl EnteGraph {
    pub async fn on_bus_request(
        &mut self,
        peer: PeerCreds,
        from: Option<Ulid>,
        request: BusRequest,
        outbound: mpsc::Sender<BusMessage>,
        reply: oneshot::Sender<BusResponse>,
    ) {
        // ---- Auth: kernel-injected SO_PEERCRED vs identidad reclamada ----
        let from_authenticated = match from {
            None => None,
            Some(claimed) => {
                let expected = self.incarnated.get(&claimed).and_then(|i| i.pid);
                match expected {
                    Some(p) if p.as_raw() == peer.pid => Some(claimed),
                    Some(p) => {
                        warn!(
                            claimed = %claimed, expected_pid = p.as_raw(),
                            actual_pid = peer.pid,
                            "identity mismatch — rechazando request"
                        );
                        let _ = reply.send(BusResponse::Error("identity mismatch".into()));
                        return;
                    }
                    None => {
                        warn!(?claimed, peer_pid = peer.pid, "Ente desconocido reclamando identidad");
                        let _ = reply.send(BusResponse::Error("unknown ente claimed".into()));
                        return;
                    }
                }
            }
        };
        if requires_auth(&request) && from_authenticated.is_none() {
            let _ = reply.send(BusResponse::Error("auth required for this request".into()));
            return;
        }

        // ---- Dispatch ----
        match request {
            BusRequest::Announce { capabilities } => {
                let id = from_authenticated.expect("auth-required guarantees Some");
                let label = self.incarnated.get(&id).map(|i| i.card.label.clone())
                    .unwrap_or_else(|| "anónimo".into());
                info!(%id, %label, ?capabilities, peer_pid = peer.pid, "Announce autenticado");
                self.bus_connections.insert(id, outbound);
                let _ = reply.send(BusResponse::Ok);
            }
            BusRequest::ListEntes => {
                let entes = self.incarnated.values()
                    .map(|i| EnteInfo {
                        id: i.card.id,
                        label: i.card.label.clone(),
                        provides: i.card.provides.iter().cloned().collect(),
                        pid: i.pid.map(|p| p.as_raw()),
                    })
                    .collect();
                let _ = reply.send(BusResponse::Entes(entes));
            }
            BusRequest::PowerOff { interactive } => {
                if let Some(reasons) = self.inhibit_block_reason() {
                    warn!(?from_authenticated, ?reasons, "PowerOff denegado por inhibición");
                    let _ = reply.send(BusResponse::Error(format!("inhibited: {reasons}")));
                } else {
                    info!(?from_authenticated, interactive, peer_pid = peer.pid, "PowerOff via bus");
                    let _ = reply.send(BusResponse::Ok);
                }
            }
            BusRequest::Reboot { interactive } => {
                if let Some(reasons) = self.inhibit_block_reason() {
                    warn!(?from_authenticated, ?reasons, "Reboot denegado por inhibición");
                    let _ = reply.send(BusResponse::Error(format!("inhibited: {reasons}")));
                } else {
                    info!(?from_authenticated, interactive, "Reboot via bus");
                    let _ = reply.send(BusResponse::Ok);
                }
            }
            BusRequest::Suspend { interactive } => {
                if let Some(reasons) = self.inhibit_block_reason() {
                    warn!(?from_authenticated, ?reasons, "Suspend denegado por inhibición");
                    let _ = reply.send(BusResponse::Error(format!("inhibited: {reasons}")));
                } else {
                    info!(?from_authenticated, interactive, "Suspend via bus");
                    let _ = reply.send(BusResponse::Ok);
                }
            }
            BusRequest::Hibernate { interactive } => {
                if let Some(reasons) = self.inhibit_block_reason() {
                    warn!(?from_authenticated, ?reasons, "Hibernate denegado por inhibición");
                    let _ = reply.send(BusResponse::Error(format!("inhibited: {reasons}")));
                } else {
                    info!(?from_authenticated, interactive, "Hibernate via bus");
                    let _ = reply.send(BusResponse::Ok);
                }
            }
            BusRequest::Invoke { cap, blob } => {
                self.forward_invoke(from_authenticated, cap, blob, reply).await;
            }
            BusRequest::UpdateCapabilities { adds, removes } => {
                let id = from_authenticated.expect("auth-required guarantees Some");
                self.apply_capability_update(id, adds, removes);
                let _ = reply.send(BusResponse::Ok);
            }
            BusRequest::KillEnte { target, signal } => {
                let caller = from_authenticated.expect("auth-required guarantees Some");
                let resp = self.kill_ente(caller, target, signal);
                let _ = reply.send(resp);
            }
            BusRequest::SpawnCardFromDisk { name } => {
                let caller = from_authenticated.expect("auth-required guarantees Some");
                let resp = self.spawn_card_from_disk(caller, name).await;
                let _ = reply.send(resp);
            }
            BusRequest::StopCardFromDisk { name } => {
                let caller = from_authenticated.expect("auth-required guarantees Some");
                let resp = self.stop_card_from_disk(caller, name);
                let _ = reply.send(resp);
            }
            BusRequest::RunCard { card } => {
                let caller = from_authenticated.expect("auth-required guarantees Some");
                let resp = self.run_card(caller, card).await;
                let _ = reply.send(resp);
            }
            BusRequest::EnteStatus { target } => {
                // Observabilidad anónima. Vivo = está en el grafo; arje-zero no
                // guarda exit codes tras la muerte, así que sólo Running/Gone.
                let liveness = match self.incarnated.get(&target) {
                    Some(inc) => Liveness::Running {
                        pid: inc.pid.map(|p| p.as_raw()),
                    },
                    None => Liveness::Gone,
                };
                let _ = reply.send(BusResponse::Status(liveness));
            }
            BusRequest::EnteTelemetry { target } => {
                let resp = match self.incarnated.get(&target) {
                    Some(inc) => match inc.pid {
                        Some(pid) => match read_proc_resources(pid.as_raw()) {
                            Some((mem_bytes, nproc)) => {
                                // restarts: del supervisor, no de /proc.
                                let restarts = self
                                    .restart_state
                                    .get(&inc.card.label)
                                    .map(|s| s.restarts)
                                    .unwrap_or(0);
                                BusResponse::Telemetry(ResourceSample {
                                    mem_bytes,
                                    nproc,
                                    restarts,
                                })
                            }
                            None => BusResponse::Error(format!(
                                "telemetría no disponible para pid {}",
                                pid.as_raw()
                            )),
                        },
                        // Sin PID: Ente Virtual/Wasm (sin proceso).
                        None => BusResponse::Error("ente sin proceso (Virtual/Wasm)".into()),
                    },
                    None => BusResponse::Error("ente no vivo".into()),
                };
                let _ = reply.send(resp);
            }
            BusRequest::Subscribe => {
                // Observabilidad anónima: la conexión pasa a recibir eventos
                // de ciclo de vida. Guardamos su `outbound` —el writer task
                // de `handle_conn` los serializa al socket—. La purga de
                // suscriptores muertos ocurre perezosamente en `broadcast_lifecycle`.
                self.lifecycle_subscribers.push(outbound);
                let _ = reply.send(BusResponse::Ok);
            }
        }
    }

    /// Carga una Card desde el card store y la encarna usando la Semilla
    /// como requester. Bloquea I/O del filesystem en el bucle primordial —
    /// aceptable porque el store es local y el caller espera la respuesta.
    async fn spawn_card_from_disk(&mut self, caller: Ulid, name: String) -> BusResponse {
        // Validación de nombre: nada de `..`, `/`, ni absolutas.
        if name.is_empty() || name.contains('/') || name.contains("..") {
            warn!(%caller, %name, "SpawnCardFromDisk: nombre inválido");
            return BusResponse::Error(format!("nombre inválido: {name:?}"));
        }
        if let Some(reasons) = self.inhibit_block_reason() {
            warn!(%caller, %name, ?reasons, "SpawnCardFromDisk denegado por inhibición");
            return BusResponse::Error(format!("inhibited: {reasons}"));
        }
        let path = cards_dir().join(format!("{name}.json"));
        let blob = match std::fs::read_to_string(&path) {
            Ok(s) => s,
            Err(e) => {
                warn!(%caller, %name, path = %path.display(), ?e, "card no encontrada");
                return BusResponse::Error(format!("card {name}: {e}"));
            }
        };
        let card: arje_card::EntityCard = match serde_json::from_str(&blob) {
            Ok(c) => c,
            Err(e) => {
                warn!(%caller, %name, ?e, "card JSON inválido");
                return BusResponse::Error(format!("card {name} JSON: {e}"));
            }
        };
        info!(%caller, %name, label = %card.label, "SpawnCardFromDisk");
        let seed_id = self.seed.id;
        // Una Card `Virtual` con genesis = bundle de sesión: encarnamos sus
        // miembros (los shims), no el envoltorio. Best-effort por miembro:
        // un shim que no encarna no debe tumbar el resto de la sesión.
        let miembros = expand_disk_bundle(card);
        let total = miembros.len();
        let mut errores = Vec::new();
        let mut ya_vivos = 0usize;
        for ente in miembros {
            let label = ente.label.clone();
            // Idempotencia: re-activar un bundle (login gnome→mirada→gnome) no
            // debe re-spawnear miembros ya vivos. Sus ULIDs son fijos en el
            // fichero, así que un segundo spawn sobrescribiría la entrada del
            // grafo y dejaría el proceso viejo huérfano. Saltamos por label —
            // espeja el dedup del overlay de boot (`profile::overlay_session`).
            if self.label_is_incarnated(&label) {
                debug!(%caller, %name, %label, "miembro del bundle ya vivo — no se re-spawnea");
                ya_vivos += 1;
                continue;
            }
            if let Err(e) = self.authorize_and_spawn(ente, seed_id).await {
                warn!(%caller, %name, %label, ?e, "miembro del bundle no encarnó");
                errores.push(format!("{label}: {e}"));
            }
        }
        if errores.is_empty() {
            info!(%name, total, ya_vivos, "bundle activado");
            BusResponse::Ok
        } else {
            BusResponse::Error(format!(
                "spawn {name}: {}/{total} miembros fallaron: {}",
                errores.len(),
                errores.join("; ")
            ))
        }
    }

    /// ¿Hay algún Ente vivo con este `label`? Usado para no re-spawnear
    /// miembros de un bundle ya activo (idempotencia de la activación de
    /// sesión).
    fn label_is_incarnated(&self, label: &str) -> bool {
        self.incarnated.values().any(|i| i.card.label == label)
    }

    /// Inverso de [`spawn_card_from_disk`](Self::spawn_card_from_disk): baja
    /// los Entes vivos cuyos labels declara `{name}.json` (su raíz si es un
    /// Ente simple, o los de su `genesis` si es un bundle `Virtual`). Los
    /// marca en `stopping` y les manda SIGTERM, así su supervisor `Restart`
    /// no los revive (ver `on_death`). Idempotente: labels sin Ente vivo se
    /// ignoran. Es el teardown de una sesión (gnome → mirada).
    fn stop_card_from_disk(&mut self, caller: Ulid, name: String) -> BusResponse {
        if name.is_empty() || name.contains('/') || name.contains("..") {
            warn!(%caller, %name, "StopCardFromDisk: nombre inválido");
            return BusResponse::Error(format!("nombre inválido: {name:?}"));
        }
        let path = cards_dir().join(format!("{name}.json"));
        let card: EntityCard = match std::fs::read_to_string(&path)
            .ok()
            .and_then(|s| serde_json::from_str(&s).ok())
        {
            Some(c) => c,
            None => {
                warn!(%caller, %name, path = %path.display(), "StopCardFromDisk: card ausente o inválida");
                return BusResponse::Error(format!("card {name}: ausente o inválida"));
            }
        };
        let labels: std::collections::BTreeSet<String> = expand_disk_bundle(card)
            .into_iter()
            .map(|c| c.label)
            .collect();
        // Snapshot de objetivos vivos antes de mutar el grafo.
        let targets: Vec<(Ulid, Option<nix::unistd::Pid>, String)> = self
            .incarnated
            .values()
            .filter(|i| labels.contains(&i.card.label))
            .map(|i| (i.card.id, i.pid, i.card.label.clone()))
            .collect();
        let mut detenidos = 0usize;
        for (id, pid, label) in targets {
            match pid {
                Some(pid) => {
                    // Marcamos ANTES de matar: el SIGCHLD podría llegar antes
                    // de volver acá, y `on_death` debe ver la marca.
                    self.stopping.insert(id);
                    match nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGTERM) {
                        Ok(()) => detenidos += 1,
                        Err(e) => {
                            warn!(%caller, %name, %label, ?e, "StopCardFromDisk: kill falló");
                            self.stopping.remove(&id);
                        }
                    }
                }
                None => {
                    // Virtual/Wasm sin proceso: lo sacamos del grafo a mano.
                    if let Some(inc) = self.incarnated.remove(&id) {
                        self.unregister_provider(&inc.card);
                        if let Some(parent) = inc.card.lineage {
                            if let Some(sib) = self.children.get_mut(&parent) {
                                sib.retain(|c| c != &id);
                            }
                        }
                        detenidos += 1;
                    }
                }
            }
        }
        info!(%caller, %name, detenidos, "StopCardFromDisk");
        BusResponse::Ok
    }

    /// Encarna una Card transmitida por el bus (no del store en disco). Es el
    /// `Engine::run` de sandokan con una Card arbitraria. Modelo de confianza
    /// (distinto de [`spawn_card_from_disk`](Self::spawn_card_from_disk), que usa
    /// la Semilla): el caller debe tener `Capability::Spawn` y la Card se encarna
    /// con el **caller como requester** —hereda sus capacidades, no las de la
    /// Semilla—, así que es imposible escalar privilegios. El gate va explícito
    /// aquí para devolver un error claro: `authorize_and_spawn` deniega en
    /// silencio (`Ok(())`), lo que para una orden remota sería engañoso.
    async fn run_card(&mut self, caller: Ulid, card: WireCard) -> BusResponse {
        if !self.holder_has(caller, &Capability::Spawn) {
            warn!(%caller, "RunCard denegado: caller sin Capability::Spawn");
            return BusResponse::Error("RunCard: caller carece de Capability::Spawn".into());
        }
        if let Some(reasons) = self.inhibit_block_reason() {
            warn!(%caller, ?reasons, "RunCard denegado por inhibición");
            return BusResponse::Error(format!("inhibited: {reasons}"));
        }
        let card = EntityCard::from(card);
        if let Err(e) = card.validate() {
            warn!(%caller, label = %card.label, ?e, "RunCard: card inválida");
            return BusResponse::Error(format!("card inválida: {e}"));
        }
        info!(%caller, label = %card.label, "RunCard (card por el wire, caller como requester)");
        match self.authorize_and_spawn(card, caller).await {
            Ok(()) => BusResponse::Ok,
            Err(e) => BusResponse::Error(format!("spawn: {e}")),
        }
    }

    /// Envía `signal` al PID del Ente target. La autorización ya fue verificada
    /// vía SO_PEERCRED (caller tiene identidad en el grafo); políticas más
    /// finas (caller==parent, capability::Kill explícita) se aplicarán cuando
    /// existan — por ahora cualquier Ente autenticado puede pedir kill.
    fn kill_ente(&mut self, caller: Ulid, target: Ulid, signal: i32) -> BusResponse {
        let Some(inc) = self.incarnated.get(&target) else {
            warn!(%caller, %target, "KillEnte: target no existe en el grafo");
            return BusResponse::Error(format!("ente {target} no existe"));
        };
        let Some(pid) = inc.pid else {
            warn!(%caller, %target, label = %inc.card.label, "KillEnte: target sin PID (Virtual o Wasm)");
            return BusResponse::Error(format!("ente {target} no es matable (sin PID)"));
        };
        let sig = match nix::sys::signal::Signal::try_from(signal) {
            Ok(s) => s,
            Err(_) => {
                warn!(%caller, %target, signal, "KillEnte: signal inválido");
                return BusResponse::Error(format!("signal {signal} inválido"));
            }
        };
        info!(%caller, %target, label = %inc.card.label, pid = pid.as_raw(), ?sig, "KillEnte");
        if let Err(e) = nix::sys::signal::kill(pid, sig) {
            warn!(?e, "KillEnte: kill(2) falló");
            return BusResponse::Error(format!("kill: {e}"));
        }
        BusResponse::Ok
    }

    /// Muta `dynamic_provides` del Ente y actualiza el índice global de
    /// providers. La Card original (immutable) no se toca.
    fn apply_capability_update(
        &mut self,
        ente_id: Ulid,
        adds: Vec<Capability>,
        removes: Vec<Capability>,
    ) {
        // Adiciones: dedupe contra Card.provides + dynamic_provides existentes.
        let mut added = Vec::new();
        let mut removed = Vec::new();
        if let Some(inc) = self.incarnated.get_mut(&ente_id) {
            for cap in adds {
                if inc.card.provides.contains(&cap) || inc.dynamic_provides.contains(&cap) {
                    continue; // ya provista, no-op
                }
                inc.dynamic_provides.insert(cap.clone());
                added.push(cap);
            }
            for cap in removes {
                if inc.dynamic_provides.remove(&cap) {
                    removed.push(cap);
                }
                // Caps de la Card original no se pueden quitar — silenciosamente
                // ignoradas. Una Card es contrato; sólo el dynamic es mutable.
            }
        }
        // Actualizar índice global. Hacemos esto fuera del scope `inc` para
        // evitar el doble-borrow de self.
        for cap in &added {
            self.register_dynamic_cap(ente_id, cap.clone());
        }
        for cap in &removed {
            self.unregister_dynamic_cap(ente_id, cap);
            // Revocar grants emitidos contra esta cap por este Ente.
            let revoked: Vec<u64> = self.grants.iter()
                .filter(|(_, g)| g.provider == ente_id && &g.cap == cap)
                .map(|(t, _)| *t)
                .collect();
            for t in revoked {
                self.grants.remove(&t);
            }
        }
        info!(
            %ente_id,
            added_count = added.len(),
            removed_count = removed.len(),
            "capabilities actualizadas en runtime"
        );
    }

    /// Enruta un Invoke al proveedor real de la capacidad. Aloca un seq
    /// server-side, registra el reply oneshot en `pending_invokes`, y empuja
    /// el request por la conexión del proveedor.
    async fn forward_invoke(
        &mut self,
        from: Option<Ulid>,
        cap: Capability,
        blob: Vec<u8>,
        reply: oneshot::Sender<BusResponse>,
    ) {
        let provider_id = match self.pick_invokable_provider(&cap) {
            Some(id) => id,
            None => {
                let _ = reply.send(BusResponse::Error(format!("sin proveedor invokable para {cap:?}")));
                return;
            }
        };
        let outbound = match self.bus_connections.get(&provider_id) {
            Some(o) => o.clone(),
            None => {
                let _ = reply.send(BusResponse::Error("proveedor no conectado al bus".into()));
                return;
            }
        };
        let seq = self.alloc_invoke_seq();
        self.pending_invokes.insert(seq, reply);
        debug!(?from, ?cap, ?provider_id, seq, blob_len = blob.len(), "forwardeando Invoke");

        let msg = BusMessage {
            from: None,
            seq,
            payload: BusPayload::Request(BusRequest::Invoke { cap, blob }),
        };
        if outbound.send(msg).await.is_err() {
            if let Some(orig) = self.pending_invokes.remove(&seq) {
                let _ = orig.send(BusResponse::Error("conn del proveedor cerrada".into()));
            }
        }
    }

    fn pick_invokable_provider(&self, cap: &Capability) -> Option<Ulid> {
        // Sólo proveedores con conexión al bus pueden recibir forwards.
        // El propio Ente #0 está en `providers` para varias caps pero no
        // debe recibir forwards — se filtra implícitamente porque la Semilla
        // no tiene conexión al bus.
        self.providers.get(cap)?
            .iter()
            .find(|id| self.bus_connections.contains_key(id))
            .copied()
    }

    pub(in crate::graph) fn alloc_invoke_seq(&mut self) -> u64 {
        self.next_invoke_seq = self.next_invoke_seq.wrapping_add(1);
        SERVER_SEQ_FLAG | self.next_invoke_seq
    }

    pub async fn on_bus_response(&mut self, seq: u64, response: BusResponse) {
        if let Some(orig) = self.pending_invokes.remove(&seq) {
            let _ = orig.send(response);
        } else {
            warn!(seq, "Response sin pending invoke");
        }
    }

    pub async fn on_bus_conn_closed(&mut self, ente_id: Option<Ulid>) {
        if let Some(id) = ente_id {
            self.bus_connections.remove(&id);
            // No revocamos providers — la capacidad sigue declarada en su
            // Card. Sólo perdimos el canal de invocación.
            debug!(%id, "bus connection cerrada");
        }
    }

    /// Invoke originado por el cerebro. Fire-and-forget: empuja el BusMessage
    /// al proveedor real sin registrar `pending_invokes` (no hay reply que
    /// retornar a un peer del bus — la decisión vivió en proceso). Si no hay
    /// proveedor invokable o el canal está saturado, se registra warn.
    pub async fn forward_brain_invoke(&mut self, cap: Capability, blob: Vec<u8>) {
        if let Some(reasons) = self.inhibit_block_reason() {
            warn!(?cap, ?reasons, "brain invoke descartado por inhibición");
            return;
        }
        let Some(provider_id) = self.pick_invokable_provider(&cap) else {
            warn!(?cap, "brain invoke: sin proveedor invokable, descartado");
            return;
        };
        let Some(outbound) = self.bus_connections.get(&provider_id).cloned() else {
            warn!(?cap, %provider_id, "brain invoke: proveedor sin conexión, descartado");
            return;
        };
        let seq = self.alloc_invoke_seq();
        debug!(?cap, %provider_id, seq, blob_len = blob.len(), "brain invoke forwardeado");
        let msg = BusMessage {
            from: None,
            seq,
            payload: BusPayload::Request(BusRequest::Invoke { cap, blob }),
        };
        if outbound.send(msg).await.is_err() {
            warn!(seq, "brain invoke: outbound del proveedor cerrado");
        }
    }

    /// Notificación dirigida a un Ente por Ulid. Empaqueta el mensaje como
    /// Invoke contra `BRAIN_NOTIFY_IFACE`. Es la única vía de comunicación
    /// directa (no por capacidad anónima) que ofrece el cerebro.
    pub async fn forward_brain_notify(&mut self, target_id: Ulid, message: String) {
        if let Some(reasons) = self.inhibit_block_reason() {
            warn!(%target_id, ?reasons, "brain notify descartado por inhibición");
            return;
        }
        let Some(outbound) = self.bus_connections.get(&target_id).cloned() else {
            warn!(%target_id, "brain notify: target sin conexión al bus, descartado");
            return;
        };
        let seq = self.alloc_invoke_seq();
        debug!(%target_id, seq, len = message.len(), "brain notify forwardeado");
        let msg = BusMessage {
            from: None,
            seq,
            payload: BusPayload::Request(BusRequest::Invoke {
                cap: Capability::Endpoint {
                    interface: arje_bus::BRAIN_NOTIFY_IFACE,
                    version: 1,
                },
                blob: message.into_bytes(),
            }),
        };
        if outbound.send(msg).await.is_err() {
            warn!(seq, %target_id, "brain notify: outbound cerrado");
        }
    }

    /// Spawn originado por el cerebro. A diferencia de `SpawnRequest`
    /// (genesis, restart, Spawn-capability) este pasa por el filtro de
    /// inhibición — si la regla del cerebro entró en distress, el grafo
    /// no acepta más expansión hasta que el TTL vence.
    pub async fn forward_brain_spawn(&mut self, card: arje_card::EntityCard) {
        if let Some(reasons) = self.inhibit_block_reason() {
            warn!(label = %card.label, ?reasons, "brain spawn descartado por inhibición");
            return;
        }
        let seed_id = self.seed.id;
        if let Err(e) = self.authorize_and_spawn(card, seed_id).await {
            warn!(?e, "brain spawn falló");
        }
    }

    /// Aplica una inhibición declarada por el cerebro. Si la razón ya existe,
    /// extiende su TTL — semántica idempotente y "re-affirmable".
    pub fn apply_brain_inhibit(&mut self, reason: String) {
        let expires = Instant::now() + INHIBIT_TTL;
        let was_new = self.inhibits.insert(reason.clone(), expires).is_none();
        if was_new {
            info!(%reason, ttl_secs = INHIBIT_TTL.as_secs(), "inhibición activada");
        } else {
            debug!(%reason, "inhibición re-afirmada");
        }
    }

    /// Si hay inhibiciones vivas, devuelve un string con las razones para
    /// los mensajes de error. También purga las expiradas como side effect.
    /// `None` significa "puede proceder".
    pub(in crate::graph) fn inhibit_block_reason(&mut self) -> Option<String> {
        let now = Instant::now();
        self.inhibits.retain(|_, exp| *exp > now);
        if self.inhibits.is_empty() {
            None
        } else {
            let reasons: Vec<&str> = self.inhibits.keys().map(|s| s.as_str()).collect();
            Some(reasons.join(", "))
        }
    }
}

#[cfg(test)]
mod tests {
    use super::read_proc_resources;

    #[test]
    fn read_proc_resources_lee_el_proceso_actual() {
        // El propio runner de tests existe en /proc — debe dar RSS > 0 y ≥1 hilo.
        let pid = std::process::id() as i32;
        let (mem_bytes, nproc) = read_proc_resources(pid).expect("el proceso actual existe en /proc");
        assert!(mem_bytes > 0, "RSS debería ser > 0");
        assert!(nproc >= 1, "al menos un hilo");
    }

    #[test]
    fn read_proc_resources_pid_inexistente_es_none() {
        // PID imposible (fuera de rango) → None, sin panic.
        assert!(read_proc_resources(i32::MAX).is_none());
    }

    use arje_card::{EntityCard, Payload};

    fn native(label: &str) -> EntityCard {
        let mut c = EntityCard::new(label);
        c.payload = Payload::Native { exec: "/x".into(), argv: vec![], envp: vec![] };
        c
    }

    #[test]
    fn bundle_virtual_con_genesis_devuelve_los_miembros() {
        // session-gnome: Virtual con shims en genesis → spawnear los shims.
        let mut bundle = EntityCard::new("session-gnome");
        bundle.payload = Payload::Virtual;
        bundle.genesis = vec![native("compat-logind"), native("compat-hostnamed")];
        let out = super::expand_disk_bundle(bundle);
        assert_eq!(out.len(), 2);
        assert!(out.iter().all(|c| matches!(c.payload, Payload::Native { .. })));
    }

    #[test]
    fn bundle_card_simple_se_mantiene() {
        let out = super::expand_disk_bundle(native("un-ente"));
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].label, "un-ente");
    }

    #[tokio::test]
    async fn stopping_salta_el_restart() {
        use crate::events::ExitStatus;
        use crate::graph::{EnteGraph, Incarnated};
        use arje_card::Supervision;
        use std::time::Duration;

        let mut g = EnteGraph::new(EntityCard::new("seed"));
        let mut shim = native("compat-logind");
        shim.supervision = Supervision::Restart {
            initial: Duration::from_millis(100),
            max: Duration::from_secs(30),
        };
        let id = shim.id;
        g.incarnated.insert(
            id,
            Incarnated {
                card: shim,
                pid: None,
                dynamic_provides: Default::default(),
            },
        );
        g.stopping.insert(id);

        let (tx, _rx) = tokio::sync::mpsc::channel(4);
        g.on_death(id, ExitStatus::Killed(nix::sys::signal::Signal::SIGTERM), &tx)
            .await;

        // El ente bajó del grafo y la marca se consumió: prueba que la rama de
        // detención (early-return, sin restart) corrió. Si no se hubiera
        // chequeado `stopping`, la marca seguiría puesta.
        assert!(!g.incarnated.contains_key(&id), "el ente detenido se fue del grafo");
        assert!(g.stopping.is_empty(), "la marca de detención se consumió");
    }

    #[test]
    fn label_is_incarnated_detecta_vivos() {
        // La Semilla queda incarnada en EnteGraph::new → su label es "vivo";
        // uno cualquiera, no. Es la guarda que hace idempotente re-activar un
        // bundle de sesión.
        let seed = EntityCard::new("seed-x");
        let g = crate::graph::EnteGraph::new(seed);
        assert!(g.label_is_incarnated("seed-x"));
        assert!(!g.label_is_incarnated("compat-logind"));
    }

    #[test]
    fn bundle_virtual_sin_genesis_se_respeta() {
        // Un Virtual aislado es un nodo lógico legítimo: no se descompone.
        let mut c = EntityCard::new("aggregator");
        c.payload = Payload::Virtual;
        let out = super::expand_disk_bundle(c);
        assert_eq!(out.len(), 1);
        assert_eq!(out[0].label, "aggregator");
    }
}
