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

use agora_core::{
    AgoraError, Attestation, Identity, IdentityId, IdentityKind, KeyRotation, MultiSigError,
    RevReason, Revocation,
};
use serde::{Deserialize, Serialize};

/// Predicado reservado con el que una identidad declara a sus guardianes: una
/// auto-atestación `subject=yo, attester=yo, predicate="guardian", value=hex(G)`
/// por cada guardián `G`. El set de guardianes es la autoridad que puede
/// revocar la clave de la identidad por compromiso (M-of-N) en el plano social
/// —el análogo del `AGORA_AUTH_RING` en el de control—. Se declaran ANTES de un
/// compromiso: [`TrustGraph::guardians_of`] los resuelve a la hora de revocar.
pub const PREDICATE_GUARDIAN: &str = "guardian";

/// Parsea 64 chars hex a un `IdentityId` (32 bytes). `None` si el largo o los
/// dígitos no cuadran — un `value` de guardián mal formado simplemente no suma.
fn parse_id_hex(s: &str) -> Option<IdentityId> {
    if s.len() != 64 {
        return None;
    }
    let mut bytes = [0u8; 32];
    for (i, b) in bytes.iter_mut().enumerate() {
        *b = u8::from_str_radix(&s[i * 2..i * 2 + 2], 16).ok()?;
    }
    Some(IdentityId::from_bytes(bytes))
}

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

/// Cuántas cosas removió [`TrustGraph::remove_identity`]. Útil para
/// reportes "borré una identidad y N atestaciones huérfanas".
#[derive(Debug, Clone, Copy, Default)]
pub struct RemoveStats {
    /// `true` si la identidad estaba registrada y se eliminó.
    pub identity: bool,
    /// Cantidad de atestaciones purgadas (mencionaban al id como
    /// attester o como claim.subject).
    pub attestations: usize,
}

/// La red de confianza: identidades conocidas + atestaciones verificadas.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct TrustGraph {
    identities: HashMap<IdentityId, Identity>,
    /// Atestaciones verificadas, en orden de inserción.
    attestations: Vec<Attestation>,
    /// Revocaciones de clave verificadas — tombstones de primera clase. Una
    /// clave revocada (vigente a `now`) deja de contar como atestador. Se
    /// guardan APARTE de las atestaciones para que el filtrado sea en tiempo de
    /// consulta: un re-gossip de lo revocado NO lo resucita. `serde(default)`
    /// preserva la lectura de snapshots viejos (sin estos campos).
    #[serde(default)]
    revocations: Vec<Revocation>,
    /// Rotaciones de clave verificadas (doble-firmadas). Encadenan vieja→nueva;
    /// [`TrustGraph::current_key_at`] sigue la cadena hasta la punta viva.
    #[serde(default)]
    rotations: Vec<KeyRotation>,
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

    // =========================================================================
    //  Ciclo de vida de claves — rotación y revocación (SDD #4)
    // =========================================================================

    /// Snapshot de las revocaciones almacenadas (persistencia / gossip).
    pub fn revocations(&self) -> &[Revocation] {
        &self.revocations
    }

    /// Snapshot de las rotaciones almacenadas (persistencia / gossip).
    pub fn rotations(&self) -> &[KeyRotation] {
        &self.rotations
    }

    /// Resuelve los guardianes de una identidad: las identidades `G` que ella
    /// misma declaró vía una auto-atestación [`PREDICATE_GUARDIAN`]. Es la
    /// autoridad que un consumidor del plano social pasa como `allowed` a
    /// [`Self::add_revocation`]. Distintos, en orden de declaración.
    pub fn guardians_of(&self, id: IdentityId) -> Vec<IdentityId> {
        let mut out: Vec<IdentityId> = Vec::new();
        for a in &self.attestations {
            if a.claim.subject == id
                && a.attester == id
                && a.claim.predicate == PREDICATE_GUARDIAN
            {
                if let Some(g) = parse_id_hex(&a.claim.value) {
                    if !out.contains(&g) {
                        out.push(g);
                    }
                }
            }
        }
        out
    }

    /// Incorpora una rotación de clave tras verificar que AMBAS firmas
    /// (vieja + nueva) cierran. Una rotación con una firma rota se rechaza —
    /// como con las atestaciones, la red sólo guarda lo comprobable. Duplicados
    /// exactos se ignoran.
    pub fn add_rotation(&mut self, rot: KeyRotation) -> Result<(), AgoraError> {
        rot.verify()?;
        if !self.rotations.contains(&rot) {
            self.rotations.push(rot);
        }
        Ok(())
    }

    /// Incorpora una revocación tras verificar que ≥ `min` firmantes DISTINTOS
    /// del set `allowed` la respaldan. El consumidor decide la autoridad: en el
    /// plano social `allowed = guardians_of(target)`; en el de control, el
    /// anillo. El grafo es mecanismo, no veredicto — pero NO guarda una
    /// revocación que su autoridad declarada no respalde (si no, cualquiera
    /// inyectaría tombstones como negación de servicio). Duplicados se ignoran.
    pub fn add_revocation(
        &mut self,
        rev: Revocation,
        min: usize,
        allowed: &[IdentityId],
    ) -> Result<(), MultiSigError> {
        rev.verify(min, allowed)?;
        if !self.revocations.contains(&rev) {
            self.revocations.push(rev);
        }
        Ok(())
    }

    /// `true` si `key` está revocada y la revocación rige en `now`. Cualquier
    /// motivo cuenta para suprimir evidencia.
    pub fn is_revoked_at(&self, key: IdentityId, now: u64) -> bool {
        self.revocations
            .iter()
            .any(|r| r.target_id() == key && r.is_active_at(now))
    }

    /// La clave VIVA que una identidad controla en `now`, siguiendo la cadena
    /// de rotaciones desde `start`. `None` si la línea está muerta: una
    /// revocación por COMPROMISO en cualquier eslabón corta la cadena ahí (no
    /// seguimos rotaciones firmadas por una clave en manos hostiles — esa es la
    /// precedencia "la revocación gana sobre la rotación"), y una punta revocada
    /// por cualquier motivo tampoco devuelve clave viva.
    pub fn current_key_at(&self, start: IdentityId, now: u64) -> Option<IdentityId> {
        let mut current = start;
        // Cota dura contra datos cíclicos (un grafo honesto es acíclico).
        for _ in 0..1024 {
            // Un compromiso vigente mata la línea: las rotaciones que salgan de
            // esta clave son indignas de confianza.
            let comprometida = self.revocations.iter().any(|r| {
                r.target_id() == current
                    && r.reason == RevReason::Compromised
                    && r.is_active_at(now)
            });
            if comprometida {
                return None;
            }
            // El sucesor: la rotación más reciente que sale de `current`.
            let siguiente = self
                .rotations
                .iter()
                .filter(|r| r.old_id() == current)
                .max_by_key(|r| r.issued_at)
                .map(|r| r.new_id());
            match siguiente {
                Some(n) if n != current => current = n,
                // Sin sucesor: la punta. Vale sólo si no está revocada.
                _ => {
                    return if self.is_revoked_at(current, now) {
                        None
                    } else {
                        Some(current)
                    };
                }
            }
        }
        None
    }

    /// Cambia el `display_name` de una identidad ya registrada. Devuelve
    /// `false` si la identidad no existe. El `display_name` es local y
    /// no autoritativo — el id sigue siendo el mismo, lo que cambia es
    /// cómo se presenta. Las atestaciones existentes no se tocan (su
    /// `subject`/`attester` son ids, no nombres).
    pub fn set_display_name(&mut self, id: IdentityId, name: impl Into<String>) -> bool {
        match self.identities.get_mut(&id) {
            Some(ident) => {
                ident.display_name = name.into();
                true
            }
            None => false,
        }
    }

    /// Saca una identidad del grafo y purga toda atestación que la
    /// mencione (como `attester` o como `claim.subject`). Devuelve la
    /// cuenta de cosas removidas — `removed.identity == false` significa
    /// que no había nada con ese id; las atestaciones se purgan igual
    /// (caso degenerado: hubo identidad antes y dejó huérfanos).
    ///
    /// **No** toca el keystore — eso es responsabilidad del caller.
    /// Pensado para `agora-cli identidad remove` sobre seeds propias;
    /// para identidades ajenas sólo borra la visión local del grafo.
    pub fn remove_identity(&mut self, id: IdentityId) -> RemoveStats {
        let identity = self.identities.remove(&id).is_some();
        let before = self.attestations.len();
        self.attestations
            .retain(|a| a.attester != id && a.claim.subject != id);
        let removed_attestations = before - self.attestations.len();
        RemoveStats {
            identity,
            attestations: removed_attestations,
        }
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

    /// Como [`Self::evidence_for`] pero EXCLUYE atestaciones cuyo atestador esté
    /// revocado (vigente a `now`). El filtro es en tiempo de consulta — un
    /// re-gossip de una atestación de clave revocada no la resucita.
    pub fn evidence_for_at(
        &self,
        subject: IdentityId,
        predicate: &str,
        value: &str,
        now: u64,
    ) -> Vec<&Attestation> {
        self.attestations
            .iter()
            .filter(|a| {
                a.claim.subject == subject
                    && a.claim.predicate == predicate
                    && a.claim.value == value
                    && !self.is_revoked_at(a.attester, now)
            })
            .collect()
    }

    /// Como [`Self::corroboration`] pero sobre la evidencia NO revocada a `now`.
    /// La que usa [`Self::is_accepted_at`] para que una clave revocada deje de
    /// sostener el claim que respaldaba.
    pub fn corroboration_at(
        &self,
        subject: IdentityId,
        predicate: &str,
        value: &str,
        now: u64,
    ) -> Corroboration {
        let mut attesters: Vec<IdentityId> = Vec::new();
        let mut self_attested = false;
        for att in self.evidence_for_at(subject, predicate, value, now) {
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
        // Evidencia NO revocada a `now`: una clave revocada deja de respaldar.
        let cor = self.corroboration_at(subject, predicate, value, now);
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
            // El claim más reciente que respalda (subject, predicate, value)
            // ENTRE los no revocados. Si no hay ninguno, accepts() ya falló
            // arriba; acá hay por lo menos uno.
            let mas_reciente = self
                .evidence_for_at(subject, predicate, value, now)
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

    #[test]
    fn set_display_name_replaces_label_keeping_id() {
        let (yumaira, ..) = actors();
        let mut g = TrustGraph::new();
        g.register(yumaira.identity(IdentityKind::Person, "Yumi"));
        let id = yumaira.identity_id();
        assert_eq!(g.identity(id).unwrap().display_name, "Yumi");
        assert!(g.set_display_name(id, "Yumaira"));
        assert_eq!(g.identity(id).unwrap().display_name, "Yumaira");
        // Identidad inexistente: false, sin crear nada.
        let (_, _, _, vecina) = actors();
        assert!(!g.set_display_name(vecina.identity_id(), "Carmen"));
        assert!(g.identity(vecina.identity_id()).is_none());
    }

    #[test]
    fn remove_identity_purges_related_attestations() {
        let (yumaira, venezuela, comunidad, vecina) = actors();
        let mut g = TrustGraph::new();
        g.register(yumaira.identity(IdentityKind::Person, "Yumaira"));
        g.register(venezuela.identity(IdentityKind::Institution, "Venezuela"));
        g.register(vecina.identity(IdentityKind::Person, "Carmen"));
        // Venezuela atestigua sobre Yumaira → al borrar Venezuela esta
        // atestación cae (mencionaba al attester).
        g.add_attestation(attest(&venezuela, yumaira.identity_id(), "nacionalidad", "venezolana"))
            .unwrap();
        // Yumaira atestigua sobre Carmen → al borrar Yumaira cae.
        g.add_attestation(attest(&yumaira, vecina.identity_id(), "vecindad", "valle"))
            .unwrap();
        // Comunidad NO está registrada pero atestigua sobre Carmen — la
        // atestación entra al graph igual; debe sobrevivir al remove de
        // Venezuela.
        g.add_attestation(attest(&comunidad, vecina.identity_id(), "miembro-de", "Valle"))
            .unwrap();
        assert_eq!(g.attestation_count(), 3);

        let stats = g.remove_identity(venezuela.identity_id());
        assert!(stats.identity);
        assert_eq!(stats.attestations, 1);
        assert_eq!(g.attestation_count(), 2);
        assert!(g.identity(venezuela.identity_id()).is_none());

        // Doble remove no rompe; identity:false, attestations:0.
        let again = g.remove_identity(venezuela.identity_id());
        assert!(!again.identity);
        assert_eq!(again.attestations, 0);
    }

    // =========================================================================
    //  Ciclo de vida — rotación y revocación (SDD #4 fase 2)
    // =========================================================================

    fn id_hex(id: IdentityId) -> String {
        id.as_bytes().iter().map(|b| format!("{b:02x}")).collect()
    }

    #[test]
    fn revocacion_suprime_evidencia_en_is_accepted_at() {
        // Venezuela respalda la nacionalidad de Yumaira; con un tercero basta.
        // Tras revocar la clave de Venezuela (M-of-N de su anillo de guardianes),
        // la atestación deja de contar y el claim ya no se acepta.
        let (yumaira, venezuela, ..) = actors();
        let g1 = Keypair::from_seed([71; 32]);
        let g2 = Keypair::from_seed([72; 32]);
        let mut g = TrustGraph::new();
        g.add_attestation(attest(&venezuela, yumaira.identity_id(), "nacionalidad", "venezolana"))
            .unwrap();

        let pol = TrustPolicy::strict(1);
        let now = 1_700_000_500;
        assert!(g.is_accepted_at(yumaira.identity_id(), "nacionalidad", "venezolana", &pol, now));

        let allowed = [g1.identity_id(), g2.identity_id()];
        let rev = Revocation::create(
            venezuela.public_key(),
            RevReason::Compromised,
            1_700_000_400,
            None,
            &[&g1, &g2],
        );
        g.add_revocation(rev, 2, &allowed).unwrap();

        // Antes de que rija la revocación, la evidencia seguía contando…
        assert!(g.is_accepted_at(yumaira.identity_id(), "nacionalidad", "venezolana", &pol, 1_700_000_300));
        // …a partir de issued_at, suprimida.
        assert!(!g.is_accepted_at(yumaira.identity_id(), "nacionalidad", "venezolana", &pol, now));
        // El legacy `is_accepted` (sin `now`) NO filtra revocaciones — sigue viendo la evidencia.
        assert!(g.is_accepted(yumaira.identity_id(), "nacionalidad", "venezolana", &pol));
    }

    #[test]
    fn add_revocation_rechaza_quorum_insuficiente() {
        let g1 = Keypair::from_seed([71; 32]);
        let g2 = Keypair::from_seed([72; 32]);
        let target = Keypair::from_seed([99; 32]).public_key();
        let allowed = [g1.identity_id(), g2.identity_id()];
        // Sólo g1 firma.
        let rev = Revocation::create(target, RevReason::Compromised, 100, None, &[&g1]);
        let mut g = TrustGraph::new();
        assert!(g.clone().add_revocation(rev.clone(), 2, &allowed).is_err());
        // Bajar el umbral a 1 la acepta y la almacena.
        g.add_revocation(rev, 1, &allowed).unwrap();
        assert_eq!(g.revocations().len(), 1);
    }

    #[test]
    fn rotacion_resuelve_clave_actual_en_cadena() {
        let v1 = Keypair::from_seed([1; 32]);
        let v2 = Keypair::from_seed([2; 32]);
        let v3 = Keypair::from_seed([3; 32]);
        let mut g = TrustGraph::new();
        g.add_rotation(KeyRotation::create(&v1, &v2, 100)).unwrap();
        g.add_rotation(KeyRotation::create(&v2, &v3, 200)).unwrap();
        // Desde la original, la punta viva es v3.
        assert_eq!(g.current_key_at(v1.identity_id(), 1_000), Some(v3.identity_id()));
        // Una clave sin rotaciones es su propia clave actual.
        let suelta = Keypair::from_seed([5; 32]);
        assert_eq!(
            g.current_key_at(suelta.identity_id(), 0),
            Some(suelta.identity_id())
        );
    }

    #[test]
    fn compromiso_corta_la_cadena_de_rotacion() {
        // v1→v2, pero v1 se revoca por COMPROMISO: no seguimos una rotación que
        // pudo firmar el atacante. La línea queda muerta (None).
        let v1 = Keypair::from_seed([1; 32]);
        let v2 = Keypair::from_seed([2; 32]);
        let g1 = Keypair::from_seed([71; 32]);
        let g2 = Keypair::from_seed([72; 32]);
        let allowed = [g1.identity_id(), g2.identity_id()];
        let mut g = TrustGraph::new();
        g.add_rotation(KeyRotation::create(&v1, &v2, 100)).unwrap();
        let rev = Revocation::create(v1.public_key(), RevReason::Compromised, 150, None, &[&g1, &g2]);
        g.add_revocation(rev, 2, &allowed).unwrap();
        // Antes del compromiso, la cadena resolvía a v2.
        assert_eq!(g.current_key_at(v1.identity_id(), 120), Some(v2.identity_id()));
        // Vigente el compromiso, muerta.
        assert_eq!(g.current_key_at(v1.identity_id(), 200), None);
    }

    #[test]
    fn guardians_of_resuelve_autodeclaracion() {
        let yo = Keypair::from_seed([20; 32]);
        let g1 = Keypair::from_seed([71; 32]);
        let g2 = Keypair::from_seed([72; 32]);
        let mut g = TrustGraph::new();
        g.add_attestation(attest(&yo, yo.identity_id(), PREDICATE_GUARDIAN, &id_hex(g1.identity_id())))
            .unwrap();
        g.add_attestation(attest(&yo, yo.identity_id(), PREDICATE_GUARDIAN, &id_hex(g2.identity_id())))
            .unwrap();
        // Una declaración de guardián AJENA (otro firma "mis guardianes") no cuenta.
        let intruso = Keypair::from_seed([88; 32]);
        g.add_attestation(attest(&intruso, yo.identity_id(), PREDICATE_GUARDIAN, &id_hex(intruso.identity_id())))
            .unwrap();

        let guardianes = g.guardians_of(yo.identity_id());
        assert_eq!(guardianes.len(), 2);
        assert!(guardianes.contains(&g1.identity_id()));
        assert!(guardianes.contains(&g2.identity_id()));
        assert!(!guardianes.contains(&intruso.identity_id()));
    }

    #[test]
    fn revocacion_temporal_caduca_y_la_evidencia_revive() {
        // Una suspensión temporal (expires_at) suprime la evidencia DENTRO de la
        // ventana y la deja revivir después — la caducidad estricta del modelo.
        let (yumaira, venezuela, ..) = actors();
        let g1 = Keypair::from_seed([71; 32]);
        let allowed = [g1.identity_id()];
        let mut g = TrustGraph::new();
        g.add_attestation(attest(&venezuela, yumaira.identity_id(), "oficio", "partera"))
            .unwrap();
        let rev = Revocation::create(venezuela.public_key(), RevReason::Retired, 100, Some(200), &[&g1]);
        g.add_revocation(rev, 1, &allowed).unwrap();

        let pol = TrustPolicy::strict(1);
        assert!(!g.is_accepted_at(yumaira.identity_id(), "oficio", "partera", &pol, 150)); // suspendida
        assert!(g.is_accepted_at(yumaira.identity_id(), "oficio", "partera", &pol, 250)); // ya caducó
    }
}
