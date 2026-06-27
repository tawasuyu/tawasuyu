//! ente-timer-compat: scheduler estilo cron + systemd .timer.
//!
//! Lee config en JSON desde `/etc/ente/timers.json` (override env
//! `ENTE_TIMERS_FILE`):
//!
//! ```json
//! [
//!   {
//!     "name": "daily-cleanup",
//!     "schedule": "0 4 * * *",
//!     "card": {
//!       "id": "01KQ_TIMER_CLEANUP_0000000",
//!       "label": "daily-cleanup-job",
//!       "schema_version": 1,
//!       "soma": {"namespaces": {}, "rlimits": {}, "cgroup": {"path": ""}},
//!       "payload": {"Native": {"exec": "/usr/local/bin/cleanup", "argv": [], "envp": []}},
//!       "supervision": "OneShot",
//!       "provides": [], "requires": []
//!     }
//!   }
//! ]
//! ```
//!
//! Schedule: cron 5-fields `min hour dom mon dow` (DOM/DOW como en cron
//! tradicional). `*` y `*/N` soportados, listas no.
//!
//! Cuando un timer dispara, si la entry trae `card` (nombre del store), el
//! shim envía `BusRequest::SpawnCardFromDisk { name }` al bus interno;
//! arje-zero carga `$ARJE_CARDS_DIR/<name>.json` y encarna con la Semilla
//! como requester. Sin `card` el fire es sólo log estructurado.

use arje_bus::{BusClient, BusRequest, BusResponse};
use arje_card::Capability;
use serde::Deserialize;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::signal::unix::{signal, SignalKind};
use tracing::{info, warn};
use tracing_subscriber::EnvFilter;

#[derive(Debug, Clone, Deserialize)]
struct TimerConfig {
    name: String,
    /// Cron 5-field: `min hour dom mon dow`. `*`, `N`, `*/N` soportados.
    schedule: String,
    /// Nombre de la Card en el store (`<ARJE_CARDS_DIR>/<card>.json`). El
    /// fire envía `SpawnCardFromDisk { name: card }` al bus interno. None
    /// = fire silencioso (sólo log).
    #[serde(default)]
    card: Option<String>,
}

#[derive(Debug)]
struct Cron {
    min: CronField,
    hour: CronField,
    dom: CronField,
    mon: CronField,
    dow: CronField,
}

#[derive(Debug)]
enum CronField {
    Any,
    Exact(u32),
    Step(u32),  // */N
}

impl CronField {
    fn parse(s: &str) -> Option<Self> {
        if s == "*" { return Some(CronField::Any); }
        if let Some(n) = s.strip_prefix("*/") {
            return n.parse().ok().map(CronField::Step);
        }
        s.parse().ok().map(CronField::Exact)
    }
    fn matches(&self, v: u32) -> bool {
        match self {
            CronField::Any => true,
            CronField::Exact(n) => *n == v,
            CronField::Step(n) if *n > 0 => v % n == 0,
            _ => false,
        }
    }
}

impl Cron {
    fn parse(s: &str) -> Option<Self> {
        let parts: Vec<&str> = s.split_whitespace().collect();
        if parts.len() != 5 { return None; }
        Some(Self {
            min:  CronField::parse(parts[0])?,
            hour: CronField::parse(parts[1])?,
            dom:  CronField::parse(parts[2])?,
            mon:  CronField::parse(parts[3])?,
            dow:  CronField::parse(parts[4])?,
        })
    }
    fn matches(&self, t: &TimeBits) -> bool {
        self.min.matches(t.min)
            && self.hour.matches(t.hour)
            && self.dom.matches(t.dom)
            && self.mon.matches(t.mon)
            && self.dow.matches(t.dow)
    }
}

#[derive(Debug)]
struct TimeBits {
    min: u32, hour: u32, dom: u32, mon: u32, dow: u32,
}

/// Decompose epoch_secs en componentes UTC. Algoritmo simple (Howard Hinnant).
fn time_bits_utc(epoch_secs: i64) -> TimeBits {
    let secs_per_day = 86400i64;
    let days_since_epoch = epoch_secs.div_euclid(secs_per_day);
    let secs_in_day = epoch_secs.rem_euclid(secs_per_day);
    let hour = (secs_in_day / 3600) as u32;
    let min = ((secs_in_day % 3600) / 60) as u32;

    // dow: 1970-01-01 fue jueves (4); cron usa 0-6 con 0=domingo.
    let dow = ((days_since_epoch + 4).rem_euclid(7)) as u32;

    // Conversión a y/m/d (Howard Hinnant Civil from days).
    let z = days_since_epoch + 719_468;
    let era = z.div_euclid(146_097);
    let doe = (z - era * 146_097) as u64;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe as i64 + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let d = (doy - (153 * mp + 2) / 5 + 1) as u32;
    let m = (if mp < 10 { mp + 3 } else { mp - 9 }) as u32;
    let _y = y + if m <= 2 { 1 } else { 0 };
    TimeBits { min, hour, dom: d, mon: m, dow }
}

#[tokio::main(flavor = "current_thread")]
async fn main() -> anyhow::Result<()> {
    bitacora::abrir("arje");
    init_tracing();
    info!("ente-timer-compat: arrancando");
    announce_to_fractal().await;

    let timers = load_timers();
    info!(count = timers.len(), "timers cargados");
    for t in &timers {
        info!(name = %t.name, schedule = %t.schedule, "timer activo");
    }

    let parsed: Vec<(TimerConfig, Cron)> = timers.into_iter()
        .filter_map(|t| {
            let cron = Cron::parse(&t.schedule)?;
            Some((t, cron))
        })
        .collect();

    let mut term = signal(SignalKind::terminate())?;
    let mut int_ = signal(SignalKind::interrupt())?;
    let mut tick = tokio::time::interval(std::time::Duration::from_secs(60));
    // Alinear al próximo minuto entero.
    let now_ms = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_millis() as u64;
    let to_next_min = 60_000 - (now_ms % 60_000);
    tokio::time::sleep(std::time::Duration::from_millis(to_next_min)).await;
    tick.tick().await; // descartar primer tick post-alignment

    info!("scheduler activo (cron 5-field UTC)");
    loop {
        tokio::select! {
            _ = tick.tick() => {
                let now = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_secs() as i64;
                let bits = time_bits_utc(now);
                for (cfg, cron) in &parsed {
                    if cron.matches(&bits) {
                        fire(cfg).await;
                    }
                }
            }
            _ = term.recv() => { info!("SIGTERM"); return Ok(()); }
            _ = int_.recv() => { info!("SIGINT"); return Ok(()); }
        }
    }
}

async fn fire(cfg: &TimerConfig) {
    info!(name = %cfg.name, "TIMER FIRE");
    let Some(card_name) = cfg.card.as_deref() else { return };
    let mut client = match BusClient::from_env().await {
        Ok(c) => c,
        Err(e) => {
            warn!(?e, name = %cfg.name, "no bus client al fire");
            return;
        }
    };
    let req = BusRequest::SpawnCardFromDisk { name: card_name.to_string() };
    match client.call(req).await {
        Ok(BusResponse::Ok) => {
            info!(name = %cfg.name, card = card_name, "card spawn aplicado");
        }
        Ok(other) => {
            warn!(name = %cfg.name, ?other, "spawn rechazado por el bus");
        }
        Err(e) => {
            warn!(?e, name = %cfg.name, "bus call falló");
        }
    }
}

fn load_timers() -> Vec<TimerConfig> {
    let path = std::env::var("ENTE_TIMERS_FILE")
        .unwrap_or_else(|_| "/etc/ente/timers.json".into());
    match std::fs::read_to_string(&path) {
        Ok(content) => serde_json::from_str(&content).unwrap_or_else(|e| {
            warn!(?e, path, "timers.json inválido — sin timers");
            vec![]
        }),
        Err(_) => {
            info!(path, "timers.json ausente — scheduler inactivo");
            vec![]
        }
    }
}

async fn announce_to_fractal() {
    if let Ok(mut client) = BusClient::from_env().await {
        let req = BusRequest::Announce {
            capabilities: vec![Capability::Endpoint {
                interface: arje_card::InterfaceId([0xa8; 16]),
                version: 1,
            }],
        };
        match client.call(req).await {
            Ok(BusResponse::Ok) => info!("Announce → bus interno OK"),
            Ok(other) => warn!(?other, "Announce respuesta inesperada"),
            Err(e) => warn!(?e, "Announce falló"),
        }
    }
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("arje_timer_compat=info"));
    // try_init: bitacora::abrir ya puede haber instalado el subscriber global.
    let _ = tracing_subscriber::fmt().with_env_filter(filter).with_target(true).try_init();
}
