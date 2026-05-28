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

use agora_core::{AgoraError, Attestation, Identity, IdentityId, IdentityKind};
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
///
/// Tres ejes ortogonales:
///
/// - **`min_third_party` + `accept_self`** — eje básico, evaluable sin
///   más contexto que el [`Corroboration`]. Lo cubre [`Self::accepts`].
/// - **`min_attesters_of_kind`** — exige al menos N atestadores cuyo
///   [`IdentityKind`] coincida con el dado (p. ej. "al menos 1
///   Institution"). Requiere el grafo para resolver los kinds.
/// - **`max_age_secs`** — el claim debe haberse emitido en los últimos
///   N segundos respecto a `now`. Requiere las atestaciones (timestamps).
///
/// Para las dos extensiones, usar [`TrustGraph::is_accepted_at`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct TrustPolicy {
    /// Atestadores terceros distintos exigidos como mínimo.
    pub min_third_party: usize,
    /// Si la auto-atestación del sujeto cuenta como respaldo válido.
    pub accept_self: bool,
    /// Exige al menos `usize` atestadores cuyo [`IdentityKind`] sea el
    /// dado. `None` desactiva el chequeo. Útil para "necesito el aval
    /// de UNA institución reconocida sin importar cuántas comunidades
    /// se sumen", o vice versa.
    pub min_attesters_of_kind: Option<(IdentityKind, usize)>,
    /// Máxima edad del claim respecto a `now`. Si el claim más reciente
    /// que respalda `(subject, predicate, value)` se emitió hace más de
    /// `max_age_secs`, la política rechaza. `None` desactiva el chequeo
    /// (las atestaciones nunca caducan).
    pub max_age_secs: Option<u64>,
}

impl TrustPolicy {
    /// Política estricta: al menos `n` terceros, la auto-atestación no
    /// suma, sin chequeos extra.
    pub fn strict(min_third_party: usize) -> Self {
        Self {
            min_third_party,
            accept_self: false,
            min_attesters_of_kind: None,
            max_age_secs: None,
        }
    }

    /// Builder fluido: exige al menos `n` atestadores del kind dado.
    pub fn with_min_of_kind(mut self, kind: IdentityKind, n: usize) -> Self {
        self.min_attesters_of_kind = Some((kind, n));
        self
    }

    /// Builder fluido: exige que el claim se haya emitido en los
    /// últimos `secs` segundos.
    pub fn with_max_age(mut self, secs: u64) -> Self {
        self.max_age_secs = Some(secs);
        self
    }

    /// `true` si la evidencia satisface el eje **básico** de la política
    /// (`min_third_party` + `accept_self`). Las extensiones por kind y
    /// edad NO se evalúan aquí — para eso usar
    /// [`TrustGraph::is_accepted_at`].
    pub fn accepts(&self, c: &Corroboration) -> bool {
        if c.third_party() >= self.min_third_party {
            return true;
        }
        self.accept_self && c.self_attested && self.min_third_party == 0
    }
}

impl Default for TrustPolicy {
    /// Por defecto: un tercero basta, la auto-atestación no, sin
    /// chequeos extra.
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

    /// Iterador sobre todas las identidades registradas. Orden no
    /// garantizado (`HashMap` interno) — los consumidores que necesiten
    /// orden estable deben ordenar por `id()`.
    pub fn identities(&self) -> impl Iterator<Item = &Identity> + '_ {
        self.identities.values()
    }

    /// Cantidad de atestaciones verificadas almacenadas.
    pub fn attestation_count(&self) -> usize {
        self.attestations.len()
    }

    /// Atestaciones verificadas en orden de inserción. Habilita
    /// snapshots para persistencia y gossip.
    pub fn attestations(&self) -> &[Attestation] {
        &self.attestations
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

    /// Atajo: `true` si la `policy` acepta el claim según su eje
    /// **básico** (`min_third_party` + `accept_self`). Ignora
    /// `min_attesters_of_kind` y `max_age_secs` — para esos usar
    /// [`Self::is_accepted_at`].
    pub fn is_accepted(
        &self,
        subject: IdentityId,
        predicate: &str,
        value: &str,
        policy: &TrustPolicy,
    ) -> bool {
        policy.accepts(&self.corroboration(subject, predicate, value))
    }

    /// `true` si la `policy` acepta el claim evaluando TODOS los ejes:
    /// básico + kind + edad respecto a `now`.
    ///
    /// - El eje básico se evalúa primero.
    /// - El eje kind cuenta atestadores cuyo `IdentityKind` registrado
    ///   en el grafo coincide con el pedido. Identidades no registradas
    ///   (atestaciones de pubkeys que el grafo desconoce) NO cuentan.
    /// - El eje edad mira el timestamp del claim más nuevo que respalda
    ///   `(subject, predicate, value)`; si `now - issued_at > max_age`,
    ///   rechaza.
    ///
    /// Mantener `is_accepted` y `is_accepted_at` como dos métodos
    /// permite que callers simples sigan ignorando edad sin tener que
    /// inventar un `now`.
    pub fn is_accepted_at(
        &self,
        subject: IdentityId,
        predicate: &str,
        value: &str,
        policy: &TrustPolicy,
        now: u64,
    ) -> bool {
        let cor = self.corroboration(subject, predicate, value);
        if !policy.accepts(&cor) {
            return false;
        }
        if let Some((kind, n)) = policy.min_attesters_of_kind {
            let count = cor
                .attesters
                .iter()
                .filter(|id| {
                    self.identities
                        .get(id)
                        .map(|i| i.kind == kind)
                        .unwrap_or(false)
                })
                .count();
            if count < n {
                return false;
            }
        }
        if let Some(max_age) = policy.max_age_secs {
            // Tomamos el claim más reciente que respalda este (subject,
            // predicate, value). Si no hay ninguno, accepts() ya falló
            // arriba; acá hay por lo menos uno.
            let mas_reciente = self
                .evidence_for(subject, predicate, value)
                .iter()
                .map(|a| a.claim.issued_at)
                .max()
                .unwrap_or(0);
            if now.saturating_sub(mas_reciente) > max_age {
                return false;
            }
        }
        true
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
    fn policy_min_attesters_of_kind_exige_aval_de_tipo() {
        // Yumaira tiene dos respaldos de oficio = "partera": una
        // comunidad y una vecina (Person). Una política que exige al
        // menos 1 Institution rechaza; relajarla a Community acepta.
        let (yumaira, _, comunidad, vecina) = actors();
        let mut g = TrustGraph::new();
        g.register(yumaira.identity(IdentityKind::Person, "Yumaira"));
        g.register(comunidad.identity(IdentityKind::Community, "Vecinos del Valle"));
        g.register(vecina.identity(IdentityKind::Person, "Carmen"));
        g.add_attestation(attest(&comunidad, yumaira.identity_id(), "oficio", "partera"))
            .unwrap();
        g.add_attestation(attest(&vecina, yumaira.identity_id(), "oficio", "partera"))
            .unwrap();

        let exige_institution = TrustPolicy::strict(2)
            .with_min_of_kind(IdentityKind::Institution, 1);
        assert!(!g.is_accepted_at(
            yumaira.identity_id(),
            "oficio",
            "partera",
            &exige_institution,
            0,
        ));

        let exige_community = TrustPolicy::strict(2).with_min_of_kind(IdentityKind::Community, 1);
        assert!(g.is_accepted_at(
            yumaira.identity_id(),
            "oficio",
            "partera",
            &exige_community,
            0,
        ));
    }

    #[test]
    fn policy_max_age_rechaza_claims_viejos() {
        // Atestación emitida en t=1_000. Política con max_age = 60
        // evaluada en now=2_000 (1_000 s después) rechaza; en now=1_050
        // (50 s después) acepta.
        let (yumaira, venezuela, ..) = actors();
        let mut g = TrustGraph::new();
        g.register(yumaira.identity(IdentityKind::Person, "Yumaira"));
        g.register(venezuela.identity(IdentityKind::Institution, "Venezuela"));
        let att = Attestation::create(
            &venezuela,
            agora_core::Claim::new(yumaira.identity_id(), "nacionalidad", "venezolana", 1_000),
        );
        g.add_attestation(att).unwrap();

        let policy = TrustPolicy::strict(1).with_max_age(60);
        assert!(g.is_accepted_at(
            yumaira.identity_id(),
            "nacionalidad",
            "venezolana",
            &policy,
            1_050,
        ));
        assert!(!g.is_accepted_at(
            yumaira.identity_id(),
            "nacionalidad",
            "venezolana",
            &policy,
            2_000,
        ));
    }

    #[test]
    fn is_accepted_legacy_ignora_kind_y_edad() {
        // Política con max_age=10 y claim emitido hace mucho. El
        // método legacy `is_accepted` lo acepta igual (la edad sólo se
        // evalúa en `is_accepted_at`); `is_accepted_at` lo rechaza.
        let (yumaira, venezuela, ..) = actors();
        let mut g = TrustGraph::new();
        g.register(yumaira.identity(IdentityKind::Person, "Yumaira"));
        g.register(venezuela.identity(IdentityKind::Institution, "Venezuela"));
        let att = Attestation::create(
            &venezuela,
            agora_core::Claim::new(yumaira.identity_id(), "nacionalidad", "venezolana", 0),
        );
        g.add_attestation(att).unwrap();

        let p = TrustPolicy::strict(1).with_max_age(10);
        assert!(g.is_accepted(yumaira.identity_id(), "nacionalidad", "venezolana", &p));
        assert!(!g.is_accepted_at(
            yumaira.identity_id(),
            "nacionalidad",
            "venezolana",
            &p,
            1_000_000,
        ));
    }

    #[test]
    fn policy_can_accept_self_attestation_when_negotiated() {
        let (yumaira, ..) = actors();
        let mut g = TrustGraph::new();
        g.add_attestation(attest(&yumaira, yumaira.identity_id(), "lema", "sembrar"))
            .unwrap();
        let lax = TrustPolicy {
            min_third_party: 0,
            accept_self: true,
            min_attesters_of_kind: None,
            max_age_secs: None,
        };
        assert!(g.is_accepted(yumaira.identity_id(), "lema", "sembrar", &lax));
        // La política por defecto, sin terceros, no la acepta.
        assert!(!g.is_accepted(yumaira.identity_id(), "lema", "sembrar", &TrustPolicy::default()));
    }
}
