//! Mediación de capabilities: emisión, renovación, revocación de tokens.
//!
//! Los grants tienen TTL (`DEFAULT_GRANT_TTL`). El cliente debe renovarlos
//! periódicamente con `renew_grant(token)`; en caso contrario, el background
//! task `purge_expired_grants` los revoca al vencimiento.

use super::{quota_for_capability, ttl_for_capability, EnteGraph, GrantedCapability};
use crate::events::CapabilityGrant;
use arje_card::Capability;
use std::time::Instant;
use tokio::sync::oneshot;
use tracing::debug;
use ulid::Ulid;

impl EnteGraph {
    pub async fn mediate_capability(
        &mut self,
        from: Ulid,
        cap: Capability,
        reply: oneshot::Sender<CapabilityGrant>,
    ) {
        let grant = match self.providers.get(&cap).and_then(|s| s.iter().next().copied()) {
            None => CapabilityGrant::NoProvider,
            Some(provider) => {
                // Quota: contar tokens vivos para (from, cap). Si excede,
                // rechazar antes de emitir uno nuevo.
                let limit = quota_for_capability(&cap);
                let active = self.active_tokens_for(from, &cap);
                if active >= limit {
                    CapabilityGrant::QuotaExceeded { active, limit }
                } else {
                    let token = self.next_token;
                    self.next_token += 1;
                    let ttl = ttl_for_capability(&cap);
                    let expires_at = Instant::now() + ttl;
                    self.grants.insert(token, GrantedCapability {
                        cap: cap.clone(),
                        provider,
                        holder: from,
                        expires_at,
                    });
                    CapabilityGrant::Granted { token }
                }
            }
        };
        let _ = reply.send(grant);
    }

    /// Cuenta tokens vivos (no expirados) emitidos a un holder para una cap.
    pub fn active_tokens_for(&self, holder: Ulid, cap: &Capability) -> u32 {
        let now = Instant::now();
        self.grants.values()
            .filter(|g| g.holder == holder && &g.cap == cap && g.expires_at > now)
            .count() as u32
    }

    /// Extiende un grant existente. Devuelve `true` si renovó. Si el token
    /// no existe o ya expiró, `false` (el cliente debe re-acquire).
    /// Usa el TTL específico de la cap del grant.
    ///
    /// Reservado para el flujo de capability renewal (no cableado todavía).
    #[allow(dead_code)]
    pub fn renew_grant(&mut self, token: u64) -> bool {
        let now = Instant::now();
        if let Some(g) = self.grants.get_mut(&token) {
            if g.expires_at > now {
                g.expires_at = now + ttl_for_capability(&g.cap);
                return true;
            }
            // Expired — purgamos aquí mismo.
            self.grants.remove(&token);
        }
        false
    }

    /// GC: elimina grants vencidos. Devuelve cuántos fueron purgados.
    pub fn purge_expired_grants(&mut self) -> usize {
        let now = Instant::now();
        let expired: Vec<u64> = self.grants.iter()
            .filter(|(_, g)| g.expires_at <= now)
            .map(|(t, _)| *t)
            .collect();
        for t in &expired {
            self.grants.remove(t);
        }
        if !expired.is_empty() {
            debug!(count = expired.len(), "grants expirados purgados");
        }
        expired.len()
    }

    /// Cuenta de grants vivos (no expirados). Usado por métricas.
    pub fn active_grants_count(&self) -> usize {
        let now = Instant::now();
        self.grants.values().filter(|g| g.expires_at > now).count()
    }
}

