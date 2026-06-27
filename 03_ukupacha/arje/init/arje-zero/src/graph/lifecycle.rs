//! Encarnación, muerte y supervisión.
//!
//! Aquí vive el flujo: Card → autorizar → soma::incarnate / wasm → registro
//! en el grafo → SIGCHLD → on_death → Restart/OneShot/Delegate.

use super::{EnteGraph, Incarnated};
use crate::events::{ExitStatus, GraphEvent};
use arje_bus::{BusEvent, BusMessage, BusPayload, BusRequest, LifecycleStatus};
use arje_card::{Capability, EntityCard, Payload, Supervision};
use sandokan_lifecycle::Backoff;
use std::time::Instant;
use tokio::sync::mpsc;
use tracing::{info, warn};
use ulid::Ulid;

impl EnteGraph {
    /// Encarna las dependencias declaradas en la Semilla. Único punto donde
    /// el Init "decide": después sólo reacciona.
    pub async fn instantiate_seed_dependencies(
        &mut self,
        _tx: &mpsc::Sender<GraphEvent>,
    ) -> anyhow::Result<()> {
        let cards = std::mem::take(&mut self.pending_genesis);
        if cards.is_empty() {
            info!(seed = %self.seed.label, "semilla sin genesis cards");
            return Ok(());
        }
        info!(seed = %self.seed.label, count = cards.len(), "instanciando genesis");
        let seed_id = self.seed.id;

        // Plan de arranque: ordena por contratos de dependencia (grafo
        // topológico), detecta ciclos y descarta las insatisfacibles. Antes el
        // genesis se spawneaba en orden de declaración con un gate de `requires`
        // por presencia; ahora respeta `Any`/`Quorum`/`Conflicts`/`After` y
        // arranca cada proveedor antes que sus consumidores.
        let external = self.available_caps();
        let plan = super::resolve::plan_spawn(&cards, &external);
        for (idx, reason) in &plan.rejected {
            warn!(
                label = %cards[*idx].label, ?reason,
                "genesis card descartada por contrato/ciclo"
            );
        }
        for &idx in &plan.order {
            if let Err(e) = self.authorize_and_spawn(cards[idx].clone(), seed_id).await {
                warn!(?e, label = %cards[idx].label, "genesis card falló");
            }
        }
        Ok(())
    }

    /// Spawn solicitado por un Ente con `Capability::Spawn`. Verifica auth,
    /// requires del grafo, y delega la encarnación al backend correspondiente
    /// (`arje_soma` para procesos, `arje_wasm` para Wasm).
    pub async fn authorize_and_spawn(
        &mut self,
        mut card: EntityCard,
        requester: Ulid,
    ) -> anyhow::Result<()> {
        if !self.holder_has(requester, &Capability::Spawn) {
            warn!(?requester, "spawn denied: lacks Capability::Spawn");
            return Ok(());
        }
        if let Err(e) = card.validate() {
            warn!(?e, label = %card.label, "card inválida, spawn rechazado");
            return Ok(());
        }
        // Falla rápida sobre los contratos de dependencia (requires AND +
        // Any/Quorum/Conflicts) contra las capacidades disponibles — mejor que
        // daemons en bucle. `After` es sólo orden (lo respeta el planificador
        // del genesis), no gatea acá. Fuente única: `Card::deps_satisfied`.
        let available = self.available_caps();
        if let Err(unmet) = card.deps_satisfied(&available) {
            warn!(?unmet, label = %card.label, "contrato de dependencia no satisfecho");
            return Ok(());
        }
        // Lineage por defecto = quien pidió el spawn.
        if card.lineage.is_none() {
            card.lineage = Some(requester);
        }

        let pid = match &card.payload {
            Payload::Virtual => None,
            Payload::Native { .. } | Payload::Legacy { .. } => {
                Some(arje_soma::incarnate(&card)?)
            }
            Payload::Wasm { module_sha256, entry } => {
                // Wasm: hilo dedicado, sin PID. Su muerte se observa por
                // estado del runtime, no por SIGCHLD.
                let bytes = arje_cas::resolve(module_sha256)
                    .map_err(|e| anyhow::anyhow!("CAS resolve para {}: {e}", card.label))?;
                arje_wasm::incarnate_wasm(&card, bytes, entry.clone())?;
                None
            }
        };

        if let Some(p) = pid {
            self.by_pid.insert(p.as_raw(), card.id);
        }
        self.register_provider(&card);
        if let Some(parent) = card.lineage {
            self.children.entry(parent).or_default().push(card.id);
        }
        info!(label = %card.label, ?pid, lineage = ?card.lineage, "Ente encarnado");
        // Sello de tiempo para el supervisor: marca el inicio del "intento"
        // actual. Si el Ente sobrevive ≥ Supervision::max, el contador de
        // reintentos se reseteará en on_death.
        if matches!(card.supervision, Supervision::Restart { .. }) {
            self.restart_state
                .entry(card.label.clone())
                .or_default()
                .last_started_at = Some(Instant::now());
        }
        self.incarnated.insert(card.id, Incarnated {
            card, pid,
            dynamic_provides: std::collections::BTreeSet::new(),
        });
        Ok(())
    }

    pub async fn on_death(
        &mut self,
        id: Ulid,
        status: ExitStatus,
        tx: &mpsc::Sender<GraphEvent>,
    ) {
        let Some(inc) = self.incarnated.remove(&id) else { return };
        if let Some(p) = inc.pid {
            self.by_pid.remove(&p.as_raw());
        }
        self.unregister_provider(&inc.card);
        if let Some(parent) = inc.card.lineage {
            if let Some(siblings) = self.children.get_mut(&parent) {
                siblings.retain(|c| c != &id);
            }
        }
        info!(label = %inc.card.label, ?status, "Ente disuelto");

        // Detención a pedido (teardown de bundle de sesión): la limpieza del
        // grafo ya ocurrió arriba. Saltamos el restart —y el broadcast de
        // crash— para que la sesión baje de verdad: un shim `Restart` matado
        // por SIGTERM, si no, reviviría. Es la mitad de "desactivar" simétrica
        // a `SpawnCardFromDisk`. Emite `EnteExited` (cierre limpio).
        if self.stopping.remove(&id) {
            info!(label = %inc.card.label, "Ente detenido a pedido — sin restart");
            self.broadcast_lifecycle(BusEvent::EnteExited {
                id,
                label: inc.card.label.clone(),
            });
            return;
        }

        // Vocabulario de ciclo de vida para los suscriptores del bus (la capa
        // de IA de hammer reacciona a `EnteCrashed` → su propio `CRASHED`).
        // Capturamos label/status antes de mover `inc.card` en el Restart.
        let label = inc.card.label.clone();
        let wire_status = match &status {
            ExitStatus::Exit(code) => LifecycleStatus::Exited(*code),
            ExitStatus::Killed(sig) => LifecycleStatus::Killed(*sig as i32),
        };
        if wire_status.is_crash() {
            self.broadcast_lifecycle(BusEvent::EnteCrashed {
                id,
                label: label.clone(),
                status: wire_status.clone(),
            });
        }

        match inc.card.supervision.clone() {
            Supervision::Restart { initial, max } => {
                // Política: si el Ente sobrevivió al menos `max` (su propio cap
                // de backoff), lo consideramos estable y reseteamos el backoff.
                // Si murió antes, escala. La matemática del backoff vive en
                // `sandokan-lifecycle` (fuente única; ver SDD §5 Fase 1) — acá
                // sólo queda la *política* de cuándo resetear.
                let st = self.restart_state.entry(inc.card.label.clone()).or_default();
                let stable = st
                    .last_started_at
                    .map(|t| t.elapsed() >= max)
                    .unwrap_or(false);
                if stable {
                    st.restarts = 0;
                }
                st.restarts = st.restarts.saturating_add(1);
                let backoff = st.backoff.get_or_insert_with(|| Backoff::new(initial, max));
                if stable {
                    backoff.reset();
                }
                let delay = backoff.next_delay();
                let delay_ms = delay.as_millis() as u64;
                info!(
                    label = %inc.card.label, delay_ms,
                    "Restart programado"
                );
                self.broadcast_lifecycle(BusEvent::EnteRestarting {
                    id,
                    label: label.clone(),
                    delay_ms,
                });
                // No bloquear el bucle primordial: el restart vuelve como
                // SpawnRequest tras el delay. El requester es la Semilla
                // (autorizada para Capability::Spawn).
                let new_card = EntityCard { id: Ulid::new(), ..inc.card };
                let tx = tx.clone();
                let requester = self.seed.id;
                tokio::spawn(async move {
                    tokio::time::sleep(delay).await;
                    if tx.send(GraphEvent::SpawnRequest { card: new_card, requester })
                        .await
                        .is_err()
                    {
                        warn!("restart: graph_tx cerrado, abortando reintento");
                    }
                });
            }
            Supervision::OneShot => {
                if !wire_status.is_crash() {
                    self.broadcast_lifecycle(BusEvent::EnteExited { id, label });
                }
            }
            Supervision::Delegate => {
                self.notify_lineage_of_death(&inc, &status);
                if !wire_status.is_crash() {
                    self.broadcast_lifecycle(BusEvent::EnteExited { id, label });
                }
            }
        }
    }

    /// Difunde un evento de ciclo de vida a las conexiones suscritas
    /// (`BusRequest::Subscribe`). Fire-and-forget: empuja por el `outbound` de
    /// cada suscriptor y **purga** los que ya cerraron su extremo. Un canal
    /// lleno (suscriptor lento) NO se purga —se prefiere perder un frame a
    /// desconectar a un observador transitoriamente atascado—. `seq = 0`
    /// porque los eventos no se correlacionan con ninguna request.
    pub(in crate::graph) fn broadcast_lifecycle(&mut self, ev: BusEvent) {
        if self.lifecycle_subscribers.is_empty() {
            return;
        }
        let msg = BusMessage {
            from: None,
            seq: 0,
            payload: BusPayload::Event(ev),
        };
        self.lifecycle_subscribers.retain(|tx| {
            !matches!(
                tx.try_send(msg.clone()),
                Err(mpsc::error::TrySendError::Closed(_))
            )
        });
    }

    /// Fire-and-forget: si el parent tiene conexión al bus, le forwardeamos
    /// un Invoke con la muerte del hijo. Sin retry, sin backpressure.
    fn notify_lineage_of_death(&mut self, inc: &Incarnated, status: &ExitStatus) {
        let Some(parent) = inc.card.lineage else { return };
        info!(
            child = %inc.card.id, parent = %parent, label = %inc.card.label,
            ?status,
            "Supervision::Delegate — muerte notificada al lineage"
        );
        if let Some(out) = self.bus_connections.get(&parent).cloned() {
            let blob = format!("{}:{:?}", inc.card.id, status);
            let seq = self.alloc_invoke_seq();
            let msg = BusMessage {
                from: None,
                seq,
                payload: BusPayload::Request(BusRequest::Invoke {
                    cap: Capability::Endpoint {
                        interface: arje_card::InterfaceId([0xde; 16]),
                        version: 1,
                    },
                    blob: blob.into_bytes(),
                }),
            };
            let _ = out.try_send(msg);
        }
    }
}

#[cfg(test)]
mod tests {
    use sandokan_lifecycle::Backoff;
    use std::time::Duration;

    // La matemática del backoff ahora vive en `sandokan-lifecycle` (y tiene
    // sus propios tests). Acá verificamos que el `Backoff` canónico reproduce
    // la secuencia que antes daba el `backoff_delay` propio de arje-zero
    // (initial, ×2, …, capeado a max) y la política de reset al estabilizarse.

    #[test]
    fn backoff_reproduce_secuencia_previa() {
        let mut b = Backoff::new(Duration::from_millis(100), Duration::from_secs(60));
        assert_eq!(b.next_delay(), Duration::from_millis(100)); // 1er reintento
        assert_eq!(b.next_delay(), Duration::from_millis(200));
        assert_eq!(b.next_delay(), Duration::from_millis(400));
        assert_eq!(b.next_delay(), Duration::from_millis(800));
    }

    #[test]
    fn backoff_satura_en_max() {
        let mut b = Backoff::new(Duration::from_millis(100), Duration::from_secs(1));
        for _ in 0..10 {
            b.next_delay();
        }
        assert_eq!(b.next_delay(), Duration::from_secs(1)); // capeado a max
    }

    #[test]
    fn reset_al_estabilizarse_vuelve_a_initial() {
        // Política de arje-zero: si el Ente vive ≥ max, reset → próximo
        // reintento arranca de nuevo en initial.
        let mut b = Backoff::new(Duration::from_millis(100), Duration::from_secs(60));
        b.next_delay();
        b.next_delay();
        b.reset();
        assert_eq!(b.next_delay(), Duration::from_millis(100));
    }
}

#[cfg(test)]
mod broadcast_tests {
    //! La fuente real del `CRASHED` (hammer Fase 5 / B.2): `on_death` difunde
    //! `BusEvent`s a los suscriptores del bus. Verificamos el mapeo
    //! muerte→evento sin levantar el bus completo: insertamos un Ente en el
    //! grafo a mano, enchufamos un suscriptor y disparamos `on_death`.
    use crate::events::ExitStatus;
    use crate::graph::{EnteGraph, Incarnated};
    use arje_bus::{BusEvent, BusMessage, BusPayload, LifecycleStatus};
    use arje_card::{EntityCard, Supervision};
    use nix::sys::signal::Signal;
    use std::collections::BTreeSet;
    use std::time::Duration;
    use tokio::sync::mpsc;
    use ulid::Ulid;

    fn grafo_con_suscriptor() -> (EnteGraph, mpsc::Receiver<BusMessage>) {
        let seed = EntityCard::new("seed-test");
        let mut g = EnteGraph::new(seed);
        let (tx, rx) = mpsc::channel::<BusMessage>(16);
        g.lifecycle_subscribers.push(tx);
        (g, rx)
    }

    /// Inserta un Ente "vivo" (sin proceso) directamente en el grafo y
    /// devuelve su id, para luego matarlo con `on_death`.
    fn encarnar(g: &mut EnteGraph, label: &str, sup: Supervision) -> Ulid {
        let mut card = EntityCard::new(label);
        card.supervision = sup;
        let id = card.id;
        g.incarnated.insert(id, Incarnated {
            card,
            pid: None,
            dynamic_provides: BTreeSet::new(),
        });
        id
    }

    fn evento(msg: BusMessage) -> BusEvent {
        match msg.payload {
            BusPayload::Event(ev) => ev,
            other => panic!("esperaba BusPayload::Event, fue {other:?}"),
        }
    }

    #[tokio::test]
    async fn restart_killed_difunde_crashed_y_restarting() {
        let (mut g, mut rx) = grafo_con_suscriptor();
        let sup = Supervision::Restart {
            initial: Duration::from_millis(10),
            max: Duration::from_secs(60),
        };
        let id = encarnar(&mut g, "demonio", sup);
        let (gtx, _grx) = mpsc::channel(16);

        g.on_death(id, ExitStatus::Killed(Signal::SIGSEGV), &gtx).await;

        // Primero el crash, luego el reinicio programado.
        match evento(rx.try_recv().expect("debía haber EnteCrashed")) {
            BusEvent::EnteCrashed { id: i, label, status } => {
                assert_eq!(i, id);
                assert_eq!(label, "demonio");
                assert_eq!(status, LifecycleStatus::Killed(Signal::SIGSEGV as i32));
                assert!(status.is_crash());
            }
            other => panic!("esperaba EnteCrashed, fue {other:?}"),
        }
        match evento(rx.try_recv().expect("debía haber EnteRestarting")) {
            BusEvent::EnteRestarting { id: i, label, delay_ms } => {
                assert_eq!(i, id);
                assert_eq!(label, "demonio");
                assert_eq!(delay_ms, 10);
            }
            other => panic!("esperaba EnteRestarting, fue {other:?}"),
        }
    }

    #[tokio::test]
    async fn oneshot_exit_limpio_difunde_exited_y_nunca_crashed() {
        let (mut g, mut rx) = grafo_con_suscriptor();
        let id = encarnar(&mut g, "tarea", Supervision::OneShot);
        let (gtx, _grx) = mpsc::channel(16);

        g.on_death(id, ExitStatus::Exit(0), &gtx).await;

        match evento(rx.try_recv().expect("debía haber EnteExited")) {
            BusEvent::EnteExited { id: i, label } => {
                assert_eq!(i, id);
                assert_eq!(label, "tarea");
            }
            other => panic!("esperaba EnteExited, fue {other:?}"),
        }
        assert!(rx.try_recv().is_err(), "exit limpio no debe emitir nada más");
    }

    #[tokio::test]
    async fn oneshot_exit_anomalo_difunde_crashed() {
        let (mut g, mut rx) = grafo_con_suscriptor();
        let id = encarnar(&mut g, "tarea-rota", Supervision::OneShot);
        let (gtx, _grx) = mpsc::channel(16);

        g.on_death(id, ExitStatus::Exit(42), &gtx).await;

        match evento(rx.try_recv().expect("debía haber EnteCrashed")) {
            BusEvent::EnteCrashed { status, .. } => {
                assert_eq!(status, LifecycleStatus::Exited(42));
            }
            other => panic!("esperaba EnteCrashed, fue {other:?}"),
        }
        // OneShot anómalo: sólo el crash, sin reinicio.
        assert!(rx.try_recv().is_err());
    }

    #[tokio::test]
    async fn suscriptor_cerrado_se_purga() {
        let (mut g, rx) = grafo_con_suscriptor();
        drop(rx); // el receptor se va: el sender quedó muerto.
        assert_eq!(g.lifecycle_subscribers.len(), 1);

        let id = encarnar(&mut g, "x", Supervision::OneShot);
        let (gtx, _grx) = mpsc::channel(16);
        g.on_death(id, ExitStatus::Exit(0), &gtx).await;

        assert!(
            g.lifecycle_subscribers.is_empty(),
            "el suscriptor con receptor cerrado debió purgarse"
        );
    }
}
