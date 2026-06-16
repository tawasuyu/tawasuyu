//! Contexto de ejecución: runtime Tokio, Engine sandokan, siembra de demo y
//! censo host-side de apps Wawa.

use card_core::{Card, Payload, Supervision};
use sandokan::{auto_default, Engine, Intent, IsolationLevel};
use sandokan_monitor_core::{observe, MonitorSnapshot};

use super::modelo::WawaApp;

// ---------------------------------------------------------------------------
// Contexto de ejecución compartido (runtime tokio + Engine elegido).
// ---------------------------------------------------------------------------

/// El Engine es async; Llimphi es sync. Encapsulamos un runtime tokio y el
/// `Box<dyn Engine>` (que es `Send + Sync`) en un `Arc` que los hilos de
/// polling/control clonan barato.
pub(crate) struct EngineCtx {
    pub(crate) rt: tokio::runtime::Runtime,
    pub(crate) engine: Box<dyn Engine>,
}

impl EngineCtx {
    pub(crate) fn poll(&self) -> Result<MonitorSnapshot, String> {
        self.rt
            .block_on(observe(&*self.engine))
            .map_err(|e| e.to_string())
    }
}

// ---------------------------------------------------------------------------
// Arranque del Engine + siembra opcional de demo.
// ---------------------------------------------------------------------------

pub(crate) fn build_ctx() -> EngineCtx {
    let rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .expect("runtime tokio");
    let engine = rt.block_on(auto_default());
    let ctx = EngineCtx { rt, engine };
    // Si no hay init/daemon, `auto_default` cae al LocalEngine in-process y la
    // lista arranca vacía. Para que `cargo run` muestre algo vivo sin montar
    // un arje-zero, `SANDOKAN_MONITOR_SEED=1` siembra unas unidades reales
    // (procesos hijos de verdad — los observa el mismo Engine).
    if std::env::var("SANDOKAN_MONITOR_SEED").is_ok() {
        if ctx.poll().map(|s| s.is_empty()).unwrap_or(true) {
            seed_demo(&ctx);
        }
    }
    ctx
}

/// Siembra procesos reales vía el Engine (sin sandbox: `IsolationLevel::None`
/// = mismo namespace, sin root). Son `sh -c` portables: tres durmientes y un
/// "worker" que pulsa CPU para que el sparkline tenga vida.
pub(crate) fn seed_demo(ctx: &EngineCtx) {
    let specs: &[(&str, &str)] = &[
        ("reposo-α", "exec sleep 100000"),
        ("reposo-β", "exec sleep 100000"),
        ("vigía", "while :; do sleep 2; done"),
        (
            "worker-pulso",
            "while :; do dd if=/dev/zero of=/dev/null bs=1M count=64 2>/dev/null; sleep 1; done",
        ),
    ];
    for (label, script) in specs {
        let mut card = Card::new(*label);
        card.payload = Payload::Native {
            exec: "/bin/sh".into(),
            argv: vec!["sh".into(), "-c".into(), (*script).into()],
            envp: vec![],
        };
        card.supervision = Supervision::OneShot;
        let intent = Intent::new(card).with_isolation(IsolationLevel::None);
        let _ = ctx.rt.block_on(ctx.engine.run(intent));
    }
}

/// Censo host-side de las apps WASM de Wawa (lectura de los assets del
/// kernel). Es **observación del manifiesto instalado**, no del executor en
/// vivo (eso es Fase 4). Honesto y barato: un `read_dir`.
pub(crate) fn wawa_census() -> Vec<WawaApp> {
    let candidates = [
        std::env::var("SANDOKAN_WAWA_ASSETS").unwrap_or_default(),
        "03_ukupacha/wawa/wawa-kernel/assets".into(),
        "wawa-kernel/assets".into(),
    ];
    for dir in candidates.iter().filter(|d| !d.is_empty()) {
        let Ok(rd) = std::fs::read_dir(dir) else {
            continue;
        };
        let mut apps: Vec<WawaApp> = rd
            .flatten()
            .filter_map(|e| {
                let p = e.path();
                if p.extension().and_then(|x| x.to_str()) != Some("wasm") {
                    return None;
                }
                let name = p.file_stem()?.to_string_lossy().into_owned();
                let bytes = e.metadata().ok()?.len();
                Some(WawaApp { name, bytes })
            })
            .collect();
        apps.sort_by(|a, b| b.bytes.cmp(&a.bytes));
        if !apps.is_empty() {
            return apps;
        }
    }
    Vec::new()
}
