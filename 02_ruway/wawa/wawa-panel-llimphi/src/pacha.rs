//! Diente **Contextos (pacha)** del panel de control: configura los *modos de
//! uso con nombre* (`pachas.ron`) y muestra el estado del versionado/cifrado de
//! dotfiles (Fases 1–5 de `pacha-dotfiles`).
//!
//! Es un *diente-de-app*: edita el catálogo de [`pacha_core`] (id · nombre ·
//! qué pasa al dejar el contexto) sin que el panel sepa de cgroups ni
//! namespaces. El versionado de dotfiles se opera con la CLI `pacha dotfiles …`;
//! acá se **muestra** su estado: si la identidad está desbloqueada (cifrado
//! activo) y tu clave pública para que otros te publiquen sets.

use std::path::PathBuf;

use allichay::{EnumOption, Field, FieldPath, FieldValue, Schema, Section};
use directories::ProjectDirs;
use pacha_core::{Catalog, OnLeave, Pacha};
use pacha_llavero::{Llavero, LlaveroKernel};

/// Nombre de la seed de identidad en el llavero de sesión (Fase 3).
const SEED_KEY: &str = "id:default";

/// Opciones de «al dejar el contexto» (espeja [`OnLeave`]).
const ON_LEAVE: &[(&str, &str)] = &[
    ("background", "En segundo plano (vuelta instantánea)"),
    ("pause", "Pausado (congelado, 0% CPU)"),
    ("close", "Cerrado (máximo ahorro)"),
];

/// Estado del diente: el catálogo de contextos + dónde persiste + cuál se edita
/// + el estado (cacheado) de la identidad/cifrado.
pub struct PachaState {
    pub catalog: Catalog,
    path: Option<PathBuf>,
    /// Contexto seleccionado (el que editan las secciones de detalle).
    sel: String,
    /// Passphrase tipeada para desbloquear (transitoria, NO se persiste).
    pass: String,
    /// ¿La identidad está desbloqueada (seed en el session keyring)? Cacheado:
    /// se recalcula en `load` y tras crear/desbloquear, NO en cada render.
    desbloqueada: bool,
    /// Clave pública X25519 (hex) si está desbloqueada; vacío si no.
    pubkey: String,
}

/// Lee el estado de identidad UNA vez (consulta el session keyring). `(desbloqueada,
/// pubkey_hex)`.
fn estado_identidad() -> (bool, String) {
    match LlaveroKernel::new().recuperar(SEED_KEY) {
        Ok(Some(seed)) => (true, hex(&pacha_dotfiles::clave_publica_de_seed(&seed))),
        _ => (false, String::new()),
    }
}

/// Lo que `route` devuelve al `update` del panel: si persistir + texto de estado.
pub struct PachaAction {
    pub dirty: bool,
    pub status: String,
}

impl PachaAction {
    fn dirty(status: impl Into<String>) -> Self {
        Self { dirty: true, status: status.into() }
    }
    fn clean(status: impl Into<String>) -> Self {
        Self { dirty: false, status: status.into() }
    }
}

/// `~/.config/pacha/pachas.ron`.
fn pacha_path() -> Option<PathBuf> {
    ProjectDirs::from("", "", "pacha").map(|d| d.config_dir().join("pachas.ron"))
}

impl PachaState {
    /// Carga el catálogo de contextos. Sin config dir / archivo, arranca vacío
    /// (el panel igual abre).
    pub fn load() -> Self {
        let path = pacha_path();
        let catalog = path
            .as_ref()
            .and_then(|p| std::fs::read_to_string(p).ok())
            .and_then(|s| Catalog::from_ron(&s).ok())
            .unwrap_or_default();
        let sel = catalog.iter().next().map(|p| p.id.clone()).unwrap_or_default();
        let (desbloqueada, pubkey) = estado_identidad();
        Self { catalog, path, sel, pass: String::new(), desbloqueada, pubkey }
    }

    /// Recalcula el estado de identidad (tras crear/desbloquear).
    fn refrescar_identidad(&mut self) {
        let (d, p) = estado_identidad();
        self.desbloqueada = d;
        self.pubkey = p;
    }

    /// Persiste el catálogo a `pachas.ron`.
    pub fn save(&self) -> Result<(), String> {
        let path = self.path.as_ref().ok_or_else(|| "sin dir de config de pacha".to_string())?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(|e| e.to_string())?;
        }
        let ron = self.catalog.to_ron().map_err(|e| e.to_string())?;
        std::fs::write(path, ron).map_err(|e| format!("pacha pachas.ron: {e}"))
    }
}

// =====================================================================
// Schema (lo que se pinta)
// =====================================================================

/// Arma el schema del diente: lista de contextos + detalle del seleccionado +
/// estado del versionado/cifrado de dotfiles.
pub fn schema(state: &PachaState) -> Schema {
    let mut schema = Schema::new().section(contextos_section(state));
    if let Some(p) = state.catalog.get(&state.sel) {
        schema = schema.section(contexto_section(p));
    }
    schema.section(identidad_section(state)).section(dotfiles_section(state))
}

/// Sección «Contextos»: selector del que se edita (● seleccionado) + alta / baja.
fn contextos_section(state: &PachaState) -> Section {
    let opts: Vec<EnumOption> = state
        .catalog
        .iter()
        .map(|p| {
            let label = if p.id == state.sel {
                format!("● {} ({})", p.label, p.id)
            } else {
                format!("{} ({})", p.label, p.id)
            };
            EnumOption::new(p.id.clone(), label)
        })
        .collect();
    let mut sec = Section::new("pacha::contextos", "Contextos").icon("◴").help(
        "Los modos de uso con nombre (pachas). Cada contexto compone config + apps + \
         política de recursos + dotfiles. Escribí un id en «Nuevo contexto» para crear \
         uno; las demás secciones editan el seleccionado (●). Cambiar de contexto en \
         vivo es `pacha switch <id>`.",
    );
    if !opts.is_empty() {
        sec = sec.field(Field::radio("usar", "Contexto seleccionado", state.sel.clone(), opts));
    }
    sec.field(Field::text("crear", "Nuevo contexto (id)…", ""))
        .field(Field::button("eliminar", "Eliminar el contexto seleccionado"))
}

/// Sección de detalle del contexto seleccionado: nombre + qué pasa al dejarlo +
/// recuento de apps/dotfiles (informativo; la receta fina se edita en `pachas.ron`).
fn contexto_section(p: &Pacha) -> Section {
    let opts: Vec<EnumOption> =
        ON_LEAVE.iter().map(|(v, l)| EnumOption::new(v.to_string(), l.to_string())).collect();
    Section::new("pacha::contexto", format!("Contexto «{}»", p.label))
        .icon("◷")
        .field(Field::text("label", "Nombre visible", p.label.clone()))
        .field(Field::dropdown("on_leave", "Al dejarlo", on_leave_str(p.on_leave), opts))
        .field(Field::display("apps", "Apps en la receta", p.apps.len().to_string()))
        .field(Field::display("dotfiles", "Sets de dotfiles", p.dotfiles.len().to_string()))
}

/// Sección «Identidad»: estado del desbloqueo + crear/desbloquear (vía `agora-cli`).
/// Desbloquear cachea la seed en el session keyring → habilita el cifrado de pacha.
fn identidad_section(state: &PachaState) -> Section {
    let estado = if state.desbloqueada {
        "● desbloqueada — el cifrado de dotfiles está activo en esta sesión".to_string()
    } else {
        "○ bloqueada — creá/desbloqueá tu identidad para cifrar y compartir".to_string()
    };
    let mut sec = Section::new("pacha::identidad", "Identidad (agora)").icon("🪪").help(
        "Tu identidad soberana Ed25519 (agora). Al desbloquearla, su seed queda en el \
         session keyring del kernel y `pacha` la usa para cifrar/descifrar tus dotfiles sin \
         re-pedir la frase en cada conmutación. La frase se toma del campo de abajo (no se \
         guarda) y se pasa a `agora-cli` por `AGORA_PASSPHRASE`. Al cerrar sesión, el kernel \
         olvida la seed. Operación equivalente por shell: `agora-cli desbloquear`.",
    );
    sec = sec.field(Field::display("estado", "Estado", estado));
    if !state.desbloqueada {
        sec = sec
            .field(Field::text("passphrase", "Frase de la identidad (no se guarda)", ""))
            .field(Field::button("desbloquear", "Desbloquear con la frase"))
            .field(Field::button("crear", "Crear una identidad nueva"));
    }
    sec
}

/// Sección «Dotfiles / Secretos»: estado del cifrado en reposo + clave pública.
fn dotfiles_section(state: &PachaState) -> Section {
    let cifrado = if state.desbloqueada {
        "activo — sellado AEAD con tu identidad".to_string()
    } else {
        "inactivo — el store va en claro hasta desbloquear la identidad".to_string()
    };
    let pubkey =
        if state.pubkey.is_empty() { "(identidad bloqueada)".to_string() } else { state.pubkey.clone() };
    let almacen = ProjectDirs::from("", "", "pacha")
        .map(|d| d.data_dir().join("dotfiles").display().to_string())
        .unwrap_or_default();
    Section::new("pacha::dotfiles", "Dotfiles / Secretos").icon("🔑").help(
        "Versionado y cifrado de tus dotfiles por contexto (`pacha dotfiles add/snapshot/\
         restore/publish/push`). El almacén se cifra en reposo con tu identidad; tu clave \
         pública sirve para que otros te publiquen sets cifrados. El aislamiento de FS por \
         contexto (tmpfs/bind por app) lo arma el incarnator desde el perfil de cada app.",
    )
    .field(Field::display("cifrado", "Cifrado en reposo", cifrado))
    .field(Field::display("pubkey", "Tu clave pública (compartir)", pubkey))
    .field(Field::display("almacen", "Almacén", almacen))
}

// =====================================================================
// Lectura de valores (para campos de texto)
// =====================================================================

pub fn text_value(state: &PachaState, rel: &FieldPath) -> Option<String> {
    let section = rel.segments().first().map(String::as_str)?;
    let leaf = rel.leaf()?;
    match section {
        // En la lista, «crear» arranca vacío (escribís el id nuevo).
        "contextos" => Some(String::new()),
        "contexto" => {
            let p = state.catalog.get(&state.sel)?;
            Some(match leaf {
                "label" => p.label.clone(),
                _ => String::new(),
            })
        }
        "identidad" => Some(match leaf {
            "passphrase" => state.pass.clone(),
            _ => String::new(),
        }),
        _ => None,
    }
}

// =====================================================================
// Ruteo de cambios
// =====================================================================

/// Aplica un cambio del diente. `rel` ya viene sin el prefijo `pacha::`.
pub fn route(state: &mut PachaState, rel: &FieldPath, value: FieldValue) -> PachaAction {
    let section = rel.segments().first().cloned().unwrap_or_default();
    match section.as_str() {
        "contextos" => route_contextos(state, rel, value),
        "contexto" => route_contexto(state, rel, value),
        "identidad" => route_identidad(state, rel, value),
        // «dotfiles» es informativa (display): no rutea.
        _ => PachaAction::clean(String::new()),
    }
}

/// Acciones de identidad: tipear la frase, desbloquear (cachea la seed) o crear.
/// Shell-out a `agora-cli` (evita acoplar el panel a `agora-keystore`).
fn route_identidad(state: &mut PachaState, rel: &FieldPath, value: FieldValue) -> PachaAction {
    match rel.leaf() {
        Some("passphrase") => {
            if let Some(v) = value.as_str() {
                state.pass = v.to_string();
            }
            // No persiste nada; sólo guarda la frase tipeada para el botón.
            PachaAction::clean(String::new())
        }
        Some("desbloquear") if value.as_bool() == Some(true) => {
            let out = std::process::Command::new("agora-cli")
                .arg("desbloquear")
                .env("AGORA_PASSPHRASE", &state.pass)
                .output();
            state.pass.clear();
            match out {
                Ok(o) if o.status.success() => {
                    state.refrescar_identidad();
                    PachaAction::clean("identidad desbloqueada — cifrado activo".to_string())
                }
                Ok(o) => PachaAction::clean(format!(
                    "no se pudo desbloquear: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                )),
                Err(e) => PachaAction::clean(format!("agora-cli no disponible: {e}")),
            }
        }
        Some("crear") if value.as_bool() == Some(true) => {
            let out = std::process::Command::new("agora-cli")
                .args(["identidad", "nueva", "--name", "yo"])
                .env("AGORA_PASSPHRASE", if state.pass.is_empty() { "agora-dev" } else { &state.pass })
                .output();
            match out {
                Ok(o) if o.status.success() => {
                    PachaAction::clean("identidad creada — ahora desbloqueala con tu frase".to_string())
                }
                Ok(o) => PachaAction::clean(format!(
                    "no se pudo crear: {}",
                    String::from_utf8_lossy(&o.stderr).trim()
                )),
                Err(e) => PachaAction::clean(format!("agora-cli no disponible: {e}")),
            }
        }
        _ => PachaAction::clean(String::new()),
    }
}

fn route_contextos(state: &mut PachaState, rel: &FieldPath, value: FieldValue) -> PachaAction {
    match rel.leaf() {
        Some("usar") => {
            if let Some(id) = value.as_str() {
                state.sel = id.to_string();
                return PachaAction::clean(format!("contexto: {id}"));
            }
        }
        Some("crear") => {
            if let Some(id) = value.as_str().map(str::trim).filter(|s| !s.is_empty()) {
                if state.catalog.contains(id) {
                    return PachaAction::clean(format!("«{id}» ya existe"));
                }
                state.catalog.upsert(Pacha::new(id, id));
                state.sel = id.to_string();
                return PachaAction::dirty(format!("contexto «{id}» creado"));
            }
        }
        Some("eliminar") if value.as_bool() == Some(true) => {
            let sel = state.sel.clone();
            if sel.is_empty() {
                return PachaAction::clean("nada seleccionado".to_string());
            }
            state.catalog.remove(&sel);
            state.sel = state.catalog.iter().next().map(|p| p.id.clone()).unwrap_or_default();
            return PachaAction::dirty(format!("contexto «{sel}» eliminado"));
        }
        _ => {}
    }
    PachaAction::clean(String::new())
}

fn route_contexto(state: &mut PachaState, rel: &FieldPath, value: FieldValue) -> PachaAction {
    let sel = state.sel.clone();
    let Some(p) = state.catalog.get_mut(&sel) else {
        return PachaAction::clean(String::new());
    };
    match rel.leaf() {
        Some("label") => {
            if let Some(v) = value.as_str() {
                p.label = v.to_string();
                return PachaAction::dirty("nombre actualizado");
            }
        }
        Some("on_leave") => {
            if let Some(v) = value.as_str() {
                p.on_leave = on_leave_from(v);
                return PachaAction::dirty(format!("al dejar: {v}"));
            }
        }
        _ => {}
    }
    PachaAction::clean(String::new())
}

fn on_leave_str(o: OnLeave) -> String {
    match o {
        OnLeave::Background => "background",
        OnLeave::Pause => "pause",
        OnLeave::Close => "close",
    }
    .to_string()
}

fn on_leave_from(s: &str) -> OnLeave {
    match s {
        "pause" => OnLeave::Pause,
        "close" => OnLeave::Close,
        _ => OnLeave::Background,
    }
}

fn hex(h: &[u8; 32]) -> String {
    h.iter().map(|b| format!("{b:02x}")).collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn rel(section: &str, leaf: &str) -> FieldPath {
        FieldPath(vec![section.to_string(), leaf.to_string()])
    }

    #[test]
    fn crear_seleccionar_editar_y_borrar_contexto() {
        let mut st = PachaState {
            catalog: Catalog::new(),
            path: None,
            sel: String::new(),
            pass: String::new(),
            desbloqueada: false,
            pubkey: String::new(),
        };
        // crear
        let a = route(&mut st, &rel("contextos", "crear"), FieldValue::Text("oficina".into()));
        assert!(a.dirty && st.catalog.contains("oficina") && st.sel == "oficina");
        // editar nombre + on_leave del seleccionado
        route(&mut st, &rel("contexto", "label"), FieldValue::Text("Trabajo".into()));
        route(&mut st, &rel("contexto", "on_leave"), FieldValue::Enum("close".into()));
        let p = st.catalog.get("oficina").unwrap();
        assert_eq!(p.label, "Trabajo");
        assert_eq!(p.on_leave, OnLeave::Close);
        // el schema se arma sin panic: lista + detalle + identidad + dotfiles
        assert_eq!(schema(&st).sections.len(), 4);
        // borrar
        let d = route(&mut st, &rel("contextos", "eliminar"), FieldValue::Bool(true));
        assert!(d.dirty && !st.catalog.contains("oficina"));
    }

    #[test]
    fn crear_duplicado_no_pisa() {
        let mut st = PachaState {
            catalog: Catalog::new(),
            path: None,
            sel: String::new(),
            pass: String::new(),
            desbloqueada: false,
            pubkey: String::new(),
        };
        route(&mut st, &rel("contextos", "crear"), FieldValue::Text("x".into()));
        st.catalog.get_mut("x").unwrap().label = "Editado".into();
        let a = route(&mut st, &rel("contextos", "crear"), FieldValue::Text("x".into()));
        assert!(!a.dirty, "no debe recrear");
        assert_eq!(st.catalog.get("x").unwrap().label, "Editado", "no pisa el existente");
    }
}
