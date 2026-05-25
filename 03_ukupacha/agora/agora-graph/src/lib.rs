//! `agora_app-graph` — la red de confianza del ágora.
//!
//! Acumula [`Attestation`]s **verificadas** (una atestación con firma
//! rota nunca entra) y responde preguntas de corroboración: *¿quién
//! respalda este claim?*
//!
//! El grafo deliberadamente **no** emite un veredicto. La verdad del
//! ágora no es absoluta: depende de cuánto peso le dé a cada atestador
//! quien la consulta. Por eso [`TrustGraph::corroboration`] devuelve la
//! evidencia cruda y [`TrustPolicy`] —un umbral *negociado*— la traduce
//! a un sí/no. Dos consumidores con políticas distintas pueden mirar la
//! misma red y discrepar legítimamente.

#![forbid(unsafe_code)]

use std::collections::HashMap;

use agora_core::{AgoraError, Attestation, Identity, IdentityId};
use serde::{Deserialize, Serialize};

/// Evidencia acumulada a favor de un claim concreto.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Corroboration {
    /// Atestadores distintos que respaldan exactamente este claim.
    pub attesters: Vec<IdentityId>,
    /// El propio sujeto figura entre los atestadores.
    pub self_attested: bool,
}

impl Corroboration {
    /// `true` si nadie respalda el claim.
    pub fn is_empty(&self) -> bool {
        self.attesters.is_empty()
    }

    /// Atestadores totales (incluido el sujeto si se auto-atestó).
    pub fn total(&self) -> usize {
        self.attesters.len()
    }

    /// Atestadores que no son el sujeto — la evidencia independiente.
    pub fn third_party(&self) -> usize {
        self.total() - usize::from(self.self_attested)
    }
}

/// Umbral *negociado* de aceptación de un claim. No es una verdad del
/// sistema: cada consumidor adopta el suyo según lo que pacte.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustPolicy {
    /// Atestadores terceros distintos exigidos como mínimo.
    pub min_third_party: usize,
    /// Si la auto-atestación del sujeto cuenta como respaldo válido.
    pub accept_self: bool,
}

impl TrustPolicy {
    /// Política estricta: al menos `n` terceros, la auto-atestación no
    /// suma.
    pub fn strict(min_third_party: usize) -> Self {
        Self { min_third_party, accept_self: false }
    }

    /// `true` si la evidencia satisface la política.
    pub fn accepts(&self, c: &Corroboration) -> bool {
        if c.third_party() >= self.min_third_party {
            return true;
        }
        self.accept_self && c.self_attested && self.min_third_party == 0
    }
}

impl Default for TrustPolicy {
    /// Por defecto: un tercero basta, la auto-atestación no.
    fn default() -> Self {
        Self::strict(1)
    }
}

/// La red de confianza: identidades conocidas + atestaciones verificadas.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustGraph {
    identities: HashMap<IdentityId, Identity>,
    /// Atestaciones verificadas, en orden de inserción.
    attestations: Vec<Attestation>,
}

impl TrustGraph {
    pub fn new() -> Self {
        Self::default()
    }

    /// Registra (o actualiza) una identidad conocida.
    pub fn register(&mut self, identity: Identity) {
        self.identities.insert(identity.id(), identity);
    }

    /// Identidad registrada con ese id, si la hay.
    pub fn identity(&self, id: IdentityId) -> Option<&Identity> {
        self.identities.get(&id)
    }

    /// Cantidad de identidades registradas.
    pub fn identity_count(&self) -> usize {
        self.identities.len()
    }

    /// Cantidad de atestaciones verificadas almacenadas.
    pub fn attestation_count(&self) -> usize {
        self.attestations.len()
    }

    /// Verifica una atestación y, si es válida y no es duplicada exacta,
    /// la incorpora. Una firma rota se rechaza con error — la red sólo
    /// guarda evidencia comprobable.
    pub fn add_attestation(&mut self, att: Attestation) -> Result<(), AgoraError> {
        att.verify()?;
        if !self.attestations.contains(&att) {
            self.attestations.push(att);
        }
        Ok(())
    }

    /// Atestaciones cuyo claim trata sobre `subject`.
    pub fn attestations_about(&self, subject: IdentityId) -> Vec<&Attestation> {
        self.attestations
            .iter()
            .filter(|a| a.claim.subject == subject)
            .collect()
    }

    /// Atestaciones emitidas por `attester`.
    pub fn attestations_by(&self, attester: IdentityId) -> Vec<&Attestation> {
        self.attestations
            .iter()
            .filter(|a| a.attester == attester)
            .collect()
    }

    /// Atestaciones que respaldan exactamente el claim
    /// `subject · predicate = value` (la marca de tiempo no importa).
    pub fn evidence_for(
        &self,
        subject: IdentityId,
        predicate: &str,
        value: &str,
    ) -> Vec<&Attestation> {
        self.attestations
            .iter()
            .filter(|a| {
                a.claim.subject == subject
                    && a.claim.predicate == predicate
                    && a.claim.value == value
            })
            .collect()
    }

    /// Resume la corroboración de un claim: atestadores distintos y si
    /// el sujeto se lo auto-atestó. El veredicto lo pone una
    /// [`TrustPolicy`].
    pub fn corroboration(
        &self,
        subject: IdentityId,
        predicate: &str,
        value: &str,
    ) -> Corroboration {
        let mut attesters: Vec<IdentityId> = Vec::new();
        let mut self_attested = false;
        for att in self.evidence_for(subject, predicate, value) {
            if att.is_self_attested() {
                self_attested = true;
            }
            if !attesters.contains(&att.attester) {
                attesters.push(att.attester);
            }
        }
        Corroboration { attesters, self_attested }
    }

    /// Atajo: `true` si la `policy` acepta el claim dada la evidencia.
    pub fn is_accepted(
        &self,
        subject: IdentityId,
        predicate: &str,
        value: &str,
        policy: &TrustPolicy,
    ) -> bool {
        policy.accepts(&self.corroboration(subject, predicate, value))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use agora_core::{Attestation, Claim, IdentityKind, Keypair};

    /// Mundo de prueba: Yumaira (persona) + tres atestadores.
    fn actors() -> (Keypair, Keypair, Keypair, Keypair) {
        (
            Keypair::from_seed([20; 32]), // yumaira
            Keypair::from_seed([10; 32]), // venezuela (institución)
            Keypair::from_seed([30; 32]), // comunidad
            Keypair::from_seed([40; 32]), // vecina
        )
    }

    fn attest(by: &Keypair, subject: IdentityId, pred: &str, val: &str) -> Attestation {
        Attestation::create(by, Claim::new(subject, pred, val, 1_700_000_000))
    }

    #[test]
    fn rejects_attestation_with_broken_signature() {
        let (yumaira, venezuela, ..) = actors();
        let mut att = attest(&venezuela, yumaira.identity_id(), "nacionalidad", "venezolana");
        att.claim.value = "falsa".into(); // rompe la firma
        let mut g = TrustGraph::new();
        assert!(g.add_attestation(att).is_err());
        assert_eq!(g.attestation_count(), 0);
    }

    #[test]
    fn stores_and_queries_verified_attestations() {
        let (yumaira, venezuela, comunidad, _) = actors();
        let mut g = TrustGraph::new();
        g.register(venezuela.identity(IdentityKind::Institution, "Venezuela"));
        g.add_attestation(attest(&venezuela, yumaira.identity_id(), "nacionalidad", "venezolana"))
            .unwrap();
        g.add_attestation(attest(&comunidad, yumaira.identity_id(), "miembro-de", "Valle"))
            .unwrap();
        assert_eq!(g.attestations_about(yumaira.identity_id()).len(), 2);
        assert_eq!(g.attestations_by(venezuela.identity_id()).len(), 1);
        assert_eq!(g.identity_count(), 1);
    }

    #[test]
    fn duplicate_attestation_is_ignored() {
        let (yumaira, venezuela, ..) = actors();
        let att = attest(&venezuela, yumaira.identity_id(), "nacionalidad", "venezolana");
        let mut g = TrustGraph::new();
        g.add_attestation(att.clone()).unwrap();
        g.add_attestation(att).unwrap();
        assert_eq!(g.attestation_count(), 1);
    }

    #[test]
    fn corroboration_counts_distinct_attesters() {
        let (yumaira, venezuela, comunidad, vecina) = actors();
        let mut g = TrustGraph::new();
        for who in [&venezuela, &comunidad, &vecina] {
            g.add_attestation(attest(who, yumaira.identity_id(), "nacionalidad", "venezolana"))
                .unwrap();
        }
        let c = g.corroboration(yumaira.identity_id(), "nacionalidad", "venezolana");
        assert_eq!(c.total(), 3);
        assert_eq!(c.third_party(), 3);
        assert!(!c.self_attested);
    }

    #[test]
    fn self_attestation_is_distinguished_from_third_party() {
        let (yumaira, venezuela, ..) = actors();
        let mut g = TrustGraph::new();
        g.add_attestation(attest(&yumaira, yumaira.identity_id(), "habilidad", "soldadura"))
            .unwrap();
        g.add_attestation(attest(&venezuela, yumaira.identity_id(), "habilidad", "soldadura"))
            .unwrap();
        let c = g.corroboration(yumaira.identity_id(), "habilidad", "soldadura");
        assert_eq!(c.total(), 2);
        assert_eq!(c.third_party(), 1);
        assert!(c.self_attested);
    }

    #[test]
    fn negotiated_policy_decides_acceptance() {
        let (yumaira, venezuela, comunidad, _) = actors();
        let mut g = TrustGraph::new();
        g.add_attestation(attest(&venezuela, yumaira.identity_id(), "oficio", "partera"))
            .unwrap();
        g.add_attestation(attest(&comunidad, yumaira.identity_id(), "oficio", "partera"))
            .unwrap();
        let (sub, pred, val) = (yumaira.identity_id(), "oficio", "partera");
        // Dos terceros: una política laxa acepta, una exigente no.
        assert!(g.is_accepted(sub, pred, val, &TrustPolicy::strict(2)));
        assert!(!g.is_accepted(sub, pred, val, &TrustPolicy::strict(3)));
    }

    #[test]
    fn empty_corroboration_for_unknown_claim() {
        let (yumaira, ..) = actors();
        let g = TrustGraph::new();
        let c = g.corroboration(yumaira.identity_id(), "nada", "nada");
        assert!(c.is_empty() && c.third_party() == 0);
    }

    #[test]
    fn policy_can_accept_self_attestation_when_negotiated() {
        let (yumaira, ..) = actors();
        let mut g = TrustGraph::new();
        g.add_attestation(attest(&yumaira, yumaira.identity_id(), "lema", "sembrar"))
            .unwrap();
        let lax = TrustPolicy { min_third_party: 0, accept_self: true };
        assert!(g.is_accepted(yumaira.identity_id(), "lema", "sembrar", &lax));
        // La política por defecto, sin terceros, no la acepta.
        assert!(!g.is_accepted(yumaira.identity_id(), "lema", "sembrar", &TrustPolicy::default()));
    }
}
