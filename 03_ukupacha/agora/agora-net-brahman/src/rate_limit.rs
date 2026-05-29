//! Rate limit por peer para el lado pasivo del gossip.
//!
//! Token bucket clásico: cada peer tiene un balde de `capacity` tokens
//! que se rellena a `refill_per_sec`. Aceptar una sesión cuesta 1
//! token. Si el balde está vacío, el accept loop descarta la sesión
//! (cierra la stream sin atender) — el peer hostil ve un EOF inmediato
//! y no consigue laburo del receptor.
//!
//! ## Por qué por sesión, no por byte
//!
//! Un bundle puede ser grande, sí, pero el costo dominante es CPU del
//! `verify` de cada atestación dentro del bundle. Token-per-byte es
//! más fino pero pedirías al peer un "hold this token before sending
//! more" que rompe el protocolo simple. Token-per-session captura el
//! ataque dominante (peer abriendo sesiones en loop) sin complicar
//! el wire.
//!
//! ## Default: sin límite
//!
//! `AgoraNet::sharing` / `::standalone` siguen aceptando todo —
//! retrocompat con código existente. Activar via
//! `AgoraNet::with_rate_limit(cfg)`. La razón es que rate limiting es
//! una decisión de despliegue (ágora doméstico ≠ ágora público), no
//! una propiedad del protocolo.

use std::collections::HashMap;
use std::time::Instant;

use card_net::PeerId;

/// Configuración del rate limiter. Valores chicos = permisivo.
#[derive(Debug, Clone, Copy)]
pub struct RateLimitConfig {
    /// Capacidad del balde — burst máximo aceptado de un peer.
    pub burst: u32,
    /// Tasa de reposición en tokens por segundo. Fractional para que
    /// "una sesión cada 2 segundos" se escriba como `0.5`.
    pub refill_per_sec: f64,
}

impl RateLimitConfig {
    /// Default razonable para un nodo doméstico: hasta 8 sesiones de
    /// burst, reposición de 1 sesión cada 2 segundos. Un peer hostil
    /// que abra streams en loop queda capeado en ~30/min después del
    /// burst inicial.
    pub fn permisivo() -> Self {
        Self { burst: 8, refill_per_sec: 0.5 }
    }

    /// Default estricto para nodos expuestos a Internet: burst 4, una
    /// sesión cada 5 segundos. Suficiente para sync periódico sin abrir
    /// la puerta a abuso.
    pub fn estricto() -> Self {
        Self { burst: 4, refill_per_sec: 0.2 }
    }
}

#[derive(Debug, Clone, Copy)]
struct Bucket {
    tokens: f64,
    last_refill: Instant,
}

/// Estado interno: un balde por peer. `RateLimiter` se comparte detrás
/// de `Arc` y se mutea con `Mutex<...>` async porque las decisiones se
/// toman desde el accept loop.
pub struct RateLimiter {
    cfg: RateLimitConfig,
    buckets: tokio::sync::Mutex<HashMap<PeerId, Bucket>>,
}

impl RateLimiter {
    pub fn new(cfg: RateLimitConfig) -> Self {
        Self {
            cfg,
            buckets: tokio::sync::Mutex::new(HashMap::new()),
        }
    }

    pub fn config(&self) -> RateLimitConfig {
        self.cfg
    }

    /// Intenta consumir 1 token del balde del peer. `true` si pasa
    /// (continuar atendiendo), `false` si está vacío (descartar la
    /// sesión).
    ///
    /// Refill perezoso: el balde se actualiza al momento de la consulta
    /// usando el `Instant` actual. No corre un task en background.
    pub async fn try_take(&self, peer: PeerId) -> bool {
        self.try_take_at(peer, Instant::now()).await
    }

    /// Variante con `Instant` explícito — usada en los tests para
    /// avanzar el reloj sin esperar tiempo real.
    pub async fn try_take_at(&self, peer: PeerId, now: Instant) -> bool {
        let mut g = self.buckets.lock().await;
        let bucket = g.entry(peer).or_insert(Bucket {
            tokens: self.cfg.burst as f64,
            last_refill: now,
        });
        let elapsed = now.saturating_duration_since(bucket.last_refill).as_secs_f64();
        let cap = self.cfg.burst as f64;
        bucket.tokens = (bucket.tokens + elapsed * self.cfg.refill_per_sec).min(cap);
        bucket.last_refill = now;
        if bucket.tokens >= 1.0 {
            bucket.tokens -= 1.0;
            true
        } else {
            false
        }
    }

    /// Cantidad de tokens (puede ser fraccional) que le quedan al peer.
    /// Útil para tests / observabilidad. `0.0` si nunca vimos al peer.
    pub async fn tokens_for(&self, peer: PeerId) -> f64 {
        let g = self.buckets.lock().await;
        g.get(&peer).map(|b| b.tokens).unwrap_or(0.0)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn fake_peer(byte: u8) -> PeerId {
        // Derivado de una pubkey ed25519 — no necesita ser válida para
        // el rate limiter (sólo se usa como clave del HashMap).
        use libp2p::identity::Keypair;
        let mut seed = [0u8; 32];
        seed[0] = byte;
        let kp = Keypair::ed25519_from_bytes(seed).unwrap();
        kp.public().to_peer_id()
    }

    #[tokio::test]
    async fn burst_drena_balde_luego_refill_lo_repone() {
        let limiter = RateLimiter::new(RateLimitConfig { burst: 3, refill_per_sec: 1.0 });
        let peer = fake_peer(7);
        let t0 = Instant::now();
        // Burst inicial: 3 tokens, 3 takes pasan.
        assert!(limiter.try_take_at(peer, t0).await);
        assert!(limiter.try_take_at(peer, t0).await);
        assert!(limiter.try_take_at(peer, t0).await);
        assert!(!limiter.try_take_at(peer, t0).await, "balde drenado");
        // Tras 2 segundos: refill 2.0 tokens.
        let t2 = t0 + Duration::from_secs(2);
        assert!(limiter.try_take_at(peer, t2).await);
        assert!(limiter.try_take_at(peer, t2).await);
        assert!(!limiter.try_take_at(peer, t2).await);
    }

    #[tokio::test]
    async fn peers_distintos_tienen_baldes_independientes() {
        let limiter = RateLimiter::new(RateLimitConfig { burst: 1, refill_per_sec: 0.0 });
        let a = fake_peer(1);
        let b = fake_peer(2);
        let t0 = Instant::now();
        assert!(limiter.try_take_at(a, t0).await);
        assert!(!limiter.try_take_at(a, t0).await, "A drenado");
        // B sigue con su token entero.
        assert!(limiter.try_take_at(b, t0).await);
    }

    #[tokio::test]
    async fn refill_no_supera_el_burst() {
        let limiter = RateLimiter::new(RateLimitConfig { burst: 2, refill_per_sec: 10.0 });
        let peer = fake_peer(3);
        let t0 = Instant::now();
        // No tomamos nada por 5 segundos — el balde no puede crecer más
        // de `burst`.
        let t5 = t0 + Duration::from_secs(5);
        // Forzamos el primer take para crear el bucket en t0.
        assert!(limiter.try_take_at(peer, t0).await);
        let tokens_antes_de_refill = limiter.tokens_for(peer).await;
        assert!((tokens_antes_de_refill - 1.0).abs() < 1e-6);
        // 5s después, refill = 5 * 10 = 50 tokens; debería capear en 2.
        assert!(limiter.try_take_at(peer, t5).await);
        let tokens = limiter.tokens_for(peer).await;
        assert!(tokens <= 2.0 + 1e-6, "burst capea el refill, got {tokens}");
    }
}
