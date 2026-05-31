//! `arje-card-llimphi` — card de escritorio con el estado vivo del init
//! arje: las **capacidades de aislamiento** que el sistema puede otorgar
//! a un proceso encarnado (namespaces, cgroups, privilegios).
//!
//! El README de arje promete una "card escritorio (estado de arje)";
//! `arje-card` nunca lo fue (quedó como alias de tipos de `card-core`).
//! Esta es esa card, sobre Llimphi.
//!
//! La fuente de verdad es [`arje_incarnate::caps::CapabilitySet::detect`],
//! la misma rutina que `Incarnator::new` corre antes de encarnar una Card.
//! No requiere daemon ni privilegios: lee `/proc` y reporta qué se puede
//! aislar AQUÍ Y AHORA (los sysctl/LSM cambian entre boots, por eso se
//! re-detecta por polling, no se cachea).
//!
//! Stack visual idéntico a `minga-explorer-llimphi`: llimphi-theme +
//! app-header + banner + stat-card, theme reactivo a `wawa-config`.
//!
//! Uso:
//! ```sh
//! cargo run -p arje-card-llimphi
//! ```

#![forbid(unsafe_code)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use arje_brain::introspect::{call, IntrospectRequest, IntrospectResponse};
use arje_incarnate::caps::{CapabilitySet, CgroupStatus, NsKind, UserNsStatus};
use card_core::{Card, Payload, Supervision};
use llimphi_theme::Theme;
use llimphi_ui::llimphi_layout::taffy::{
    prelude::{length, percent, AlignItems, Dimension, FlexDirection, Size, Style},
    Rect,
};
use llimphi_ui::llimphi_raster::peniko::Color;
use llimphi_ui::{App, Handle, View};
use llimphi_widget_app_header::{app_header, AppHeaderPalette};
use llimphi_widget_banner::{banner_view, BannerKind};
use llimphi_widget_button::{button_view, ButtonPalette};
use llimphi_widget_stat_card::{stat_card_view, StatCardPalette};

const REFRESH_INTERVAL: Duration = Duration::from_secs(2);

/// Los 7 namespaces que `CapabilitySet::can_create_ns` sabe evaluar, en
/// el orden en que se muestran. (NsKind no expone un `all()`.)
const NAMESPACES: [NsKind; 7] = [
    NsKind::User,
    NsKind::Mount,
    NsKind::Pid,
    NsKind::Net,
    NsKind::Uts,
    NsKind::Ipc,
    NsKind::Cgroup,
];

/// Snapshot derivado de un `CapabilitySet`, ya en forma presentable
/// (strings + bools) para que la `view` no toque los enums del dominio.
#[derive(Clone)]
struct CapsSnapshot {
    kernel: (u32, u32, u32),
    root: bool,
    user_ns: &'static str,
    cgroup_v2: &'static str,
    cgroup_delegated: bool,
    max_user_ns: Option<u64>,
    our_cgroup: Option<String>,
    /// (nombre del namespace, ¿creable aquí?).
    ns_can: Vec<(&'static str, bool)>,
    /// Cuántos de los 7 namespaces son creables.
    ns_creatable: usize,
}

impl CapsSnapshot {
    fn detect() -> Self {
        let caps = CapabilitySet::detect();
        let ns_can: Vec<(&'static str, bool)> = NAMESPACES
            .iter()
            .map(|&k| (k.name(), caps.can_create_ns(k)))
            .collect();
        let ns_creatable = ns_can.iter().filter(|(_, ok)| *ok).count();
        Self {
            kernel: caps.kernel_version,
            root: caps.has_cap_sys_admin,
            user_ns: human_user_ns(&caps.user_ns),
            cgroup_v2: human_cgroup(&caps.cgroup_v2),
            cgroup_delegated: caps.cgroup_delegated,
            max_user_ns: caps.max_user_namespaces,
            our_cgroup: caps.our_cgroup.map(|p| p.display().to_string()),
            ns_can,
            ns_creatable,
        }
    }
}

fn human_user_ns(s: &UserNsStatus) -> &'static str {
    match s {
        UserNsStatus::Allowed => "permitidos",
        UserNsStatus::DisabledBySysctl => "deshabilitados (sysctl)",
        UserNsStatus::RestrictedByLsm => "restringidos (LSM)",
        UserNsStatus::Unknown => "desconocido",
    }
}

fn human_cgroup(s: &CgroupStatus) -> &'static str {
    match s {
        CgroupStatus::Unified => "v2 unificado",
        CgroupStatus::Hybrid => "híbrido (v1+v2)",
        CgroupStatus::Legacy => "v1 legacy",
        CgroupStatus::NotMounted => "sin montar",
    }
}

/// Snapshot del brain (motor de reglas + observador + audit) leído por su
/// socket introspect. Sólo los datos que la card muestra.
#[derive(Clone)]
struct BrainSnapshot {
    /// Reglas vivas en el motor.
    rules: usize,
    /// Entropía de Shannon de la distribución de eventos observados.
    entropy_bits: f64,
    /// Eventos muestreados en la ventana del observador.
    sample_size: u64,
    /// Tipos de evento distintos vistos.
    distinct_kinds: usize,
    /// Seq del head del audit log (None si está vacío).
    head_seq: Option<u64>,
    /// Últimas entradas del audit, más recientes primero.
    recent_audit: Vec<String>,
}

/// Estado del brain en el modelo: aún consultando, caído/no-corriendo, o vivo.
/// El brain es opcional — la card de aislamiento sirve igual sin él.
#[derive(Clone)]
enum BrainStatus {
    Consultando,
    Offline(String),
    Live(BrainSnapshot),
}

/// Path del socket introspect del brain. Misma convención que arje-zero:
/// `$ENTE_BRAIN_SOCK`, o `$XDG_RUNTIME_DIR/ente-brain.sock` (fallback
/// `$TMPDIR`, `/tmp`).
fn brain_path() -> PathBuf {
    if let Ok(p) = std::env::var("ENTE_BRAIN_SOCK") {
        return p.into();
    }
    let runtime = std::env::var("XDG_RUNTIME_DIR")
        .unwrap_or_else(|_| std::env::var("TMPDIR").unwrap_or_else(|_| "/tmp".into()));
    format!("{runtime}/ente-brain.sock").into()
}

/// Consulta el brain por su socket introspect con un runtime tokio efímero
/// (current-thread). Pensado para correr fuera del hilo de UI vía
/// `Handle::spawn`. Cualquier fallo de conexión/protocolo → `Err`: la UI lo
/// pinta como "brain offline" y nunca tumba la card.
fn query_brain(path: &Path) -> Result<BrainSnapshot, String> {
    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .map_err(|e| format!("runtime tokio: {e}"))?;
    rt.block_on(async {
        let rules = match call(path, IntrospectRequest::ListRules).await {
            Ok(IntrospectResponse::Rules(v)) => v.len(),
            Ok(IntrospectResponse::Error(e)) => return Err(e),
            Ok(_) => return Err("respuesta inesperada a ListRules".into()),
            Err(e) => return Err(e.to_string()),
        };
        let (entropy_bits, sample_size, distinct_kinds) =
            match call(path, IntrospectRequest::EntropySnapshot).await {
                Ok(IntrospectResponse::Entropy {
                    value_bits,
                    sample_size,
                    distinct_kinds,
                    ..
                }) => (value_bits, sample_size, distinct_kinds),
                Ok(IntrospectResponse::Error(e)) => return Err(e),
                Ok(_) => return Err("respuesta inesperada a EntropySnapshot".into()),
                Err(e) => return Err(e.to_string()),
            };
        let recent = match call(
            path,
            IntrospectRequest::ListAudit {
                limit: 6,
                filter: Default::default(),
            },
        )
        .await
        {
            Ok(IntrospectResponse::AuditEntries(v)) => v,
            Ok(IntrospectResponse::Error(e)) => return Err(e),
            Ok(_) => return Err("respuesta inesperada a ListAudit".into()),
            Err(e) => return Err(e.to_string()),
        };
        let head_seq = recent.iter().map(|e| e.seq).max();
        let recent_audit = recent
            .iter()
            .rev()
            .map(|e| format!("#{}  {}", e.seq, e.action.kind().as_str()))
            .collect();
        Ok(BrainSnapshot {
            rules,
            entropy_bits,
            sample_size,
            distinct_kinds,
            head_seq,
            recent_audit,
        })
    })
}

/// Pide al brain verificar la integridad de la cadena del audit log
/// (`VerifyAudit`: recorre `prev_sha` hasta el génesis validando cada
/// entry contra el CAS). Operación **read-only**. Devuelve `(ok, mensaje)`
/// listo para un banner. Runtime tokio efímero, fuera del hilo de UI.
fn verify_audit(path: &Path) -> (bool, String) {
    let rt = match tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
    {
        Ok(rt) => rt,
        Err(e) => return (false, format!("runtime tokio: {e}")),
    };
    rt.block_on(async {
        match call(path, IntrospectRequest::VerifyAudit).await {
            Ok(IntrospectResponse::AuditVerified(r)) => {
                if let Some(seq) = r.broken_at_seq {
                    (
                        false,
                        format!(
                            "audit ROTO en seq {seq}: {}",
                            r.error.unwrap_or_else(|| "sin detalle".into())
                        ),
                    )
                } else if let Some(e) = r.error {
                    (false, format!("audit con error: {e}"))
                } else {
                    (
                        true,
                        format!("audit íntegro — {} entries verificadas", r.verified),
                    )
                }
            }
            Ok(IntrospectResponse::Error(e)) => (false, format!("brain: {e}")),
            Ok(_) => (false, "respuesta inesperada a VerifyAudit".into()),
            Err(e) => (false, e.to_string()),
        }
    })
}

/// Una unidad del card store: una Card `.json` invocable por el Init
/// (equivalente fractal de un `.service` de systemd).
#[derive(Clone)]
struct UnitRow {
    /// `label` de la Card, o el stem del archivo si no parsea.
    label: String,
    /// Tipo de payload: wasm / nativo / virtual / legacy / "?" si no parsea.
    payload: &'static str,
    /// Política de supervisión: restart / oneshot / delegada / "?".
    supervision: &'static str,
    /// `true` si la Card parseó bien; `false` = `.json` ilegible/corrupto.
    ok: bool,
}

/// Estado del card store leído del filesystem.
#[derive(Clone)]
struct UnitsSnapshot {
    dir: String,
    units: Vec<UnitRow>,
}

/// Directorio del card store. Misma convención que `arje_compat::cards_dir`
/// (replicada para no arrastrar el árbol de deps de arje-compat por 4
/// líneas): `$ARJE_CARDS_DIR`, default `/etc/arje/cards.d`.
fn cards_dir() -> PathBuf {
    std::env::var("ARJE_CARDS_DIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| PathBuf::from("/etc/arje/cards.d"))
}

fn payload_kind(p: &Payload) -> &'static str {
    match p {
        Payload::Wasm { .. } => "wasm",
        Payload::Native { .. } => "nativo",
        Payload::Virtual => "virtual",
        Payload::Legacy { .. } => "legacy",
    }
}

fn supervision_kind(s: &Supervision) -> &'static str {
    match s {
        Supervision::Restart { .. } => "restart",
        Supervision::OneShot => "oneshot",
        Supervision::Delegate => "delegada",
    }
}

/// Lee el card store del filesystem y arma el snapshot de unidades. Una
/// Card `.json` ilegible no rompe el listado — entra con `ok=false` y el
/// stem del archivo como label, igual que `systemctl` muestra unidades
/// dañadas. Dir ausente → lista vacía (no es error: puede no haber store).
fn detect_units() -> UnitsSnapshot {
    let dir = cards_dir();
    let mut units = Vec::new();
    if let Ok(rd) = std::fs::read_dir(&dir) {
        for entry in rd.flatten() {
            let name = entry.file_name();
            let Some(name) = name.to_str() else { continue };
            let Some(stem) = name.strip_suffix(".json") else {
                continue;
            };
            let parsed = std::fs::read_to_string(entry.path())
                .ok()
                .and_then(|s| Card::from_json(&s).ok());
            units.push(match parsed {
                Some(card) => UnitRow {
                    label: card.label,
                    payload: payload_kind(&card.payload),
                    supervision: supervision_kind(&card.supervision),
                    ok: true,
                },
                None => UnitRow {
                    label: stem.to_string(),
                    payload: "?",
                    supervision: "?",
                    ok: false,
                },
            });
        }
    }
    units.sort_by(|a, b| a.label.cmp(&b.label));
    UnitsSnapshot {
        dir: dir.display().to_string(),
        units,
    }
}

struct Model {
    theme: Theme,
    snapshot: CapsSnapshot,
    units: UnitsSnapshot,
    last_detect_ms: u64,
    brain: BrainStatus,
    /// Último resultado de "Verificar audit": `(ok, mensaje)`. `None` = aún
    /// no se pidió.
    verify: Option<(bool, String)>,
    /// Mantiene vivo el watcher de wawa-config (su thread muere al dropear).
    _wawa_watcher: Option<wawa_config::ConfigWatcher>,
}

#[derive(Clone)]
enum Msg {
    /// Tick del scheduler: re-detecta capacidades y relanza la consulta al brain.
    Tick,
    /// Resultado de una consulta al brain (vivo o caído).
    BrainRefresh(Result<BrainSnapshot, String>),
    /// Click en "Verificar audit": dispara la verificación de la cadena.
    VerifyAudit,
    /// Resultado de la verificación: `(ok, mensaje)`.
    VerifyDone(bool, String),
    /// El bus de wawa-config cambió: re-aplicar theme/accent.
    WawaChanged(wawa_config::WawaConfig),
}

struct ArjeCard;

impl App for ArjeCard {
    type Model = Model;
    type Msg = Msg;

    fn title() -> &'static str {
        "arje — estado del init"
    }

    fn initial_size() -> (u32, u32) {
        (760, 540)
    }

    fn init(handle: &Handle<Msg>) -> Model {
        // Re-detección periódica: los sysctl/LSM/cgroup delegation cambian
        // entre boots y a veces en caliente, así que no se cachea.
        handle.spawn_periodic(REFRESH_INTERVAL, || Msg::Tick);

        let initial_cfg = wawa_config::WawaConfig::load();
        let theme = theme_from_wawa(&initial_cfg);

        let handle_clone = handle.clone();
        let watcher = wawa_config::ConfigWatcher::spawn(move |cfg| {
            handle_clone.dispatch(Msg::WawaChanged(cfg));
        })
        .ok();

        // Consulta inicial al brain en background (no esperar al primer tick).
        handle.spawn(move || Msg::BrainRefresh(query_brain(&brain_path())));

        Model {
            theme,
            snapshot: CapsSnapshot::detect(),
            units: detect_units(),
            last_detect_ms: 0,
            brain: BrainStatus::Consultando,
            verify: None,
            _wawa_watcher: watcher,
        }
    }

    fn update(model: Model, msg: Msg, handle: &Handle<Msg>) -> Model {
        let mut m = model;
        match msg {
            Msg::Tick => {
                // detect() lee unos cuantos archivos de /proc — microsegundos,
                // no bloquea el hilo de UI, no necesita spawn.
                let started = std::time::Instant::now();
                m.snapshot = CapsSnapshot::detect();
                m.units = detect_units();
                m.last_detect_ms = started.elapsed().as_micros() as u64 / 1000;
                // El brain sí es socket I/O: fuera del hilo de UI.
                handle.spawn(move || Msg::BrainRefresh(query_brain(&brain_path())));
            }
            Msg::BrainRefresh(res) => {
                m.brain = match res {
                    Ok(snap) => BrainStatus::Live(snap),
                    Err(e) => BrainStatus::Offline(e),
                };
            }
            Msg::VerifyAudit => {
                m.verify = Some((true, "verificando…".into()));
                handle.spawn(move || {
                    let (ok, txt) = verify_audit(&brain_path());
                    Msg::VerifyDone(ok, txt)
                });
            }
            Msg::VerifyDone(ok, txt) => {
                m.verify = Some((ok, txt));
            }
            Msg::WawaChanged(cfg) => {
                m.theme = theme_from_wawa(&cfg);
            }
        }
        m
    }

    fn view(model: &Model) -> View<Msg> {
        let theme = &model.theme;
        let header_palette = AppHeaderPalette::from_theme(theme);
        let stat_palette = StatCardPalette::from_theme(theme);
        let snap = &model.snapshot;

        // Acentos semánticos: aislamiento azul, privilegios ámbar, cgroups verde.
        let accent_iso = Color::from_rgba8(0x88, 0xc0, 0xd0, 0xff);
        let accent_priv = Color::from_rgba8(0xeb, 0xcb, 0x8b, 0xff);
        let accent_cgroup = Color::from_rgba8(0xa3, 0xbe, 0x8c, 0xff);

        let (ka, kb, kc) = snap.kernel;
        let header_text = format!(
            "Linux {ka}.{kb}.{kc}  ·  detección {} ms",
            model.last_detect_ms
        );
        // Acción "Verificar audit" sólo tiene sentido con el brain vivo.
        let mut actions: Vec<View<Msg>> = Vec::new();
        if matches!(model.brain, BrainStatus::Live(_)) {
            let btn_palette = ButtonPalette::from_theme(theme);
            actions.push(button_view::<Msg>(
                "Verificar audit",
                &btn_palette,
                Msg::VerifyAudit,
            ));
        }
        let header = app_header::<Msg>(header_text, actions, &header_palette);

        let mut body_children: Vec<View<Msg>> = Vec::new();

        // Banner del último resultado de "Verificar audit".
        if let Some((ok, txt)) = &model.verify {
            let kind = if *ok {
                BannerKind::Success
            } else {
                BannerKind::Error
            };
            body_children.push(banner_view::<Msg>(kind, txt.clone()));
        }

        // Banner de advertencia si no se puede aislar nada: el init no podrá
        // encarnar Cards con los namespaces que pidan.
        if snap.ns_creatable == 0 {
            body_children.push(banner_view::<Msg>(
                BannerKind::Warning,
                "Ningún namespace es creable aquí: las Cards que requieran \
                 aislamiento no podrán encarnarse sin CAP_SYS_ADMIN o user-ns."
                    .to_string(),
            ));
        }

        // Card 1 — aislamiento: namespaces creables.
        let ns_items: Vec<String> = snap
            .ns_can
            .iter()
            .map(|(name, ok)| format!("{}  {name}", if *ok { "✓" } else { "✗" }))
            .collect();
        body_children.push(stat_card_view::<Msg>(
            "Aislamiento",
            format!("{}/7", snap.ns_creatable),
            "namespaces creables para un proceso encarnado",
            accent_iso,
            &ns_items,
            &stat_palette,
        ));

        // Card 2 — privilegios.
        let priv_items = vec![
            format!(
                "CAP_SYS_ADMIN  {}",
                if snap.root { "sí (root)" } else { "no" }
            ),
            format!("user namespaces  {}", snap.user_ns),
            format!(
                "max_user_namespaces  {}",
                snap.max_user_ns
                    .map(|v| v.to_string())
                    .unwrap_or_else(|| "—".into())
            ),
        ];
        body_children.push(stat_card_view::<Msg>(
            "Privilegios",
            if snap.root { "root" } else { "usuario" },
            "de qué dispone el supervisor para aislar",
            accent_priv,
            &priv_items,
            &stat_palette,
        ));

        // Card 3 — cgroups.
        let cgroup_items = vec![
            format!(
                "delegación  {}",
                if snap.cgroup_delegated { "sí" } else { "no" }
            ),
            format!(
                "nuestro cgroup  {}",
                snap.our_cgroup.as_deref().unwrap_or("—")
            ),
        ];
        body_children.push(stat_card_view::<Msg>(
            "cgroups",
            snap.cgroup_v2,
            "control de recursos disponible",
            accent_cgroup,
            &cgroup_items,
            &stat_palette,
        ));

        // Card — unidades del card store (Cards .json invocables por el Init,
        // equivalente fractal de los .service de systemd).
        let accent_units = Color::from_rgba8(0x81, 0xa1, 0xc1, 0xff);
        let unit_items: Vec<String> = model
            .units
            .units
            .iter()
            .map(|u| {
                let mark = if u.ok { "" } else { "✗ " };
                format!("{mark}{}  ·  {}  ·  {}", u.label, u.payload, u.supervision)
            })
            .collect();
        body_children.push(stat_card_view::<Msg>(
            "Unidades",
            model.units.units.len().to_string(),
            &model.units.dir,
            accent_units,
            &unit_items,
            &stat_palette,
        ));

        // Sección brain — opcional. El brain corre como daemon aparte; si no
        // está, la card de aislamiento sirve igual.
        match &model.brain {
            BrainStatus::Consultando => {}
            BrainStatus::Offline(e) => {
                body_children.push(banner_view::<Msg>(
                    BannerKind::Info,
                    format!("brain no disponible ({e})"),
                ));
            }
            BrainStatus::Live(b) => {
                let accent_brain = Color::from_rgba8(0xb4, 0x8e, 0xad, 0xff);
                let accent_audit = Color::from_rgba8(0xd0, 0x87, 0x70, 0xff);

                let brain_items = vec![
                    format!("entropía  {:.2} bits", b.entropy_bits),
                    format!("muestras  {}", b.sample_size),
                    format!("tipos de evento  {}", b.distinct_kinds),
                ];
                body_children.push(stat_card_view::<Msg>(
                    "Brain",
                    b.rules.to_string(),
                    "reglas vivas en el motor",
                    accent_brain,
                    &brain_items,
                    &stat_palette,
                ));

                body_children.push(stat_card_view::<Msg>(
                    "Audit log",
                    b.head_seq
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "—".into()),
                    "seq del head — cadena de decisiones del brain",
                    accent_audit,
                    &b.recent_audit,
                    &stat_palette,
                ));
            }
        }

        let body = View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: Dimension::auto(),
            },
            flex_grow: 1.0,
            padding: Rect {
                left: length(16.0_f32),
                right: length(16.0_f32),
                top: length(12.0_f32),
                bottom: length(16.0_f32),
            },
            gap: Size {
                width: length(0.0_f32),
                height: length(8.0_f32),
            },
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(body_children);

        View::new(Style {
            flex_direction: FlexDirection::Column,
            size: Size {
                width: percent(1.0_f32),
                height: percent(1.0_f32),
            },
            align_items: Some(AlignItems::Stretch),
            ..Default::default()
        })
        .fill(theme.bg_app)
        .children(vec![header, body])
    }
}

/// Construye un `Theme` desde la config wawa (mismo helper que
/// minga-explorer-llimphi): variant canónico → `Theme::by_name`, accent
/// si está definido, fallback dark sin romper.
fn theme_from_wawa(cfg: &wawa_config::WawaConfig) -> Theme {
    let mut t = wawa_config::canonical_theme_name(&cfg.theme_variant)
        .and_then(Theme::by_name)
        .unwrap_or_else(Theme::dark);
    if let Some([r, g, b]) = wawa_config::accent_rgb(&cfg.accent) {
        let c = Color::from_rgba8(r, g, b, 0xff);
        t.accent = c;
        t.border_focus = c;
    }
    t
}

fn main() {
    llimphi_ui::run::<ArjeCard>();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn detect_snapshot_no_panic() {
        let snap = CapsSnapshot::detect();
        // Siempre evaluamos los 7 namespaces.
        assert_eq!(snap.ns_can.len(), 7);
        assert!(snap.ns_creatable <= 7);
    }

    #[test]
    fn human_labels_cubren_variantes() {
        assert_eq!(human_user_ns(&UserNsStatus::Allowed), "permitidos");
        assert_eq!(human_cgroup(&CgroupStatus::Unified), "v2 unificado");
    }

    #[test]
    fn query_brain_offline_es_err_no_panic() {
        // Sin daemon en ese path, query_brain degrada a Err (la card lo pinta
        // como "brain offline"), nunca paniquea.
        let res = query_brain(Path::new("/nonexistent/arje-card-test.sock"));
        assert!(res.is_err());
    }

    #[test]
    fn brain_path_respeta_env() {
        // Variable explícita gana sobre el fallback de runtime dir.
        std::env::set_var("ENTE_BRAIN_SOCK", "/tmp/mi-brain.sock");
        assert_eq!(brain_path(), PathBuf::from("/tmp/mi-brain.sock"));
        std::env::remove_var("ENTE_BRAIN_SOCK");
    }

    #[test]
    fn verify_audit_offline_es_falso_no_panic() {
        let (ok, _txt) = verify_audit(Path::new("/nonexistent/arje-card-test.sock"));
        assert!(!ok);
    }

    #[test]
    fn detect_units_parsea_y_tolera_corrupto() {
        // Store temporal con una Card válida y un .json corrupto. El corrupto
        // entra con ok=false (no rompe el listado); los no-.json se ignoran.
        let dir = std::env::temp_dir().join(format!("arje-card-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let card = Card::new("demo");
        std::fs::write(dir.join("demo.json"), card.to_json_pretty().unwrap()).unwrap();
        std::fs::write(dir.join("roto.json"), b"{ no soy json").unwrap();
        std::fs::write(dir.join("ignorame.txt"), b"x").unwrap();

        std::env::set_var("ARJE_CARDS_DIR", &dir);
        let snap = detect_units();
        std::env::remove_var("ARJE_CARDS_DIR");
        let _ = std::fs::remove_dir_all(&dir);

        assert_eq!(snap.units.len(), 2); // demo + roto, no el .txt
        let roto = snap.units.iter().find(|u| u.label == "roto").unwrap();
        assert!(!roto.ok);
        let demo = snap.units.iter().find(|u| u.label == "demo").unwrap();
        assert!(demo.ok);
        assert_eq!(demo.payload, "virtual"); // default de Card::new
    }
}
