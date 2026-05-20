//! Qué orquestar: la intención de ejecución y su contexto.

use brahman_card::Card;
use serde::{Deserialize, Serialize};
use std::time::{Duration, SystemTime};
use ulid::Ulid;

/// Nivel de aislamiento pedido para una encarnación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, Serialize, Deserialize)]
pub enum IsolationLevel {
    /// Sin sandbox — mismo namespace que el orquestador.
    None,
    /// Namespaces estándar (pid/mount/net/...) según `Card.soma`.
    #[default]
    Standard,
    /// Namespaces + rootfs aislado (`pivot_root` + OverlayFS).
    Sealed,
}

/// Contexto de ejecución: ajustes sobre cómo encarnar la Card.
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct ExecContext {
    /// Aislamiento pedido. `None` = derivar de `Card.soma`.
    pub isolation: Option<IsolationLevel>,
    /// Variables de entorno adicionales (sobre las del Card).
    pub env: Vec<(String, String)>,
    /// Time-to-live opcional: si se setea, el orquestador detiene la
    /// entidad al vencer.
    pub ttl: Option<Duration>,
}

/// Una intención de ejecución: la `Card` a encarnar + su contexto.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Intent {
    /// La Card se serializa vía `WireCard` (proyección postcard-friendly):
    /// el campo `extensions` de `Card` usa `#[serde(flatten)]`, que no es
    /// compatible con formatos no auto-descriptivos como postcard.
    #[serde(with = "card_wire")]
    pub card: Card,
    #[serde(default)]
    pub context: ExecContext,
}

/// Serde adapter: `Card` ↔ `WireCard` en el límite de serialización.
/// Las `extensions` locales de la Card se descartan al cruzar el wire.
mod card_wire {
    use brahman_card::{Card, WireCard};
    use serde::{Deserialize, Deserializer, Serialize, Serializer};

    pub fn serialize<S: Serializer>(card: &Card, s: S) -> Result<S::Ok, S::Error> {
        WireCard::from(card.clone()).serialize(s)
    }

    pub fn deserialize<'de, D: Deserializer<'de>>(d: D) -> Result<Card, D::Error> {
        Ok(Card::from(WireCard::deserialize(d)?))
    }
}

impl Intent {
    /// Intención mínima: encarnar una Card con el contexto por defecto.
    pub fn new(card: Card) -> Self {
        Self { card, context: ExecContext::default() }
    }

    /// Builder: fija el nivel de aislamiento.
    pub fn with_isolation(mut self, level: IsolationLevel) -> Self {
        self.context.isolation = Some(level);
        self
    }

    /// Builder: fija un TTL.
    pub fn with_ttl(mut self, ttl: Duration) -> Self {
        self.context.ttl = Some(ttl);
        self
    }

    /// El id de la Card que esta intención encarna.
    pub fn card_id(&self) -> Ulid {
        self.card.id
    }
}

/// Referencia a una entidad encarnada por el orquestador.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ExecHandle {
    /// Id de la Card encarnada (identidad estable en el fractal).
    pub card_id: Ulid,
    /// Label humano-legible (copiado de `Card.label`).
    pub label: String,
    /// Cuándo arrancó.
    pub started_at: SystemTime,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn isolation_default_is_standard() {
        assert_eq!(IsolationLevel::default(), IsolationLevel::Standard);
    }

    #[test]
    fn intent_builders_compose() {
        let card = Card::new("demo");
        let intent = Intent::new(card)
            .with_isolation(IsolationLevel::Sealed)
            .with_ttl(Duration::from_secs(30));
        assert_eq!(intent.context.isolation, Some(IsolationLevel::Sealed));
        assert_eq!(intent.context.ttl, Some(Duration::from_secs(30)));
    }
}
