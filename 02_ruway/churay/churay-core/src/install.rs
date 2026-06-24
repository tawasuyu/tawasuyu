//! El motor de instalación: resolver el binario de una unidad desde una
//! [`Source`], escribirlo **atómicamente** en `<prefix>/bin`, sembrar su
//! `.desktop`, y registrar el resultado en [`InstalledState`].
//!
//! Dos lados, una sola UI (decisión "a y b"):
//!   * **A — bundle**: el binario ya viene compilado en `<bundle>/bin/<prog>`.
//!     Se copia y, si el manifiesto trae `bin_hash`, se verifica BLAKE3.
//!   * **B — fuente**: `cargo build --release --bin <prog>` en el workspace y
//!     se copia de `target/release/<prog>`. Sólo en dev (repo presente).
//!
//! Dos modos de destino (decisión root/local):
//!   * [`InstallMode::System`] → `/usr/local` (pide root; incluye `arje`).
//!   * [`InstallMode::Local`]  → `~/.local` (sin root; sólo apps).

use std::path::{Path, PathBuf};

use crate::hash::ArtifactHash;
use crate::manifest::{Scope, Unit};
use crate::state::InstalledState;

/// Dónde y con qué privilegios se instala.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InstallMode {
    /// Sistema entero: `/usr/local`, root, incluye componentes `System`.
    System,
    /// Sólo para el usuario: `~/.local`, sin root, sólo apps.
    Local,
}

impl InstallMode {
    /// Prefix por defecto del modo.
    pub fn default_prefix(self) -> PathBuf {
        match self {
            InstallMode::System => PathBuf::from("/usr/local"),
            InstallMode::Local => directories::BaseDirs::new()
                .map(|b| b.home_dir().join(".local"))
                .unwrap_or_else(|| PathBuf::from("/usr/local")),
        }
    }

    /// `true` si el modo admite instalar una unidad de este alcance.
    pub fn admits(self, scope: Scope) -> bool {
        match self {
            InstallMode::System => true,
            InstallMode::Local => scope == Scope::App,
        }
    }
}

/// Configuración de una corrida de instalación.
#[derive(Debug, Clone)]
pub struct InstallConfig {
    pub mode: InstallMode,
    /// Raíz donde se instala (`<prefix>/bin`, `<prefix>/share/...`).
    pub prefix: PathBuf,
    /// Directorio del bundle precompilado (lado A), si existe.
    pub bundle_dir: Option<PathBuf>,
    /// Raíz del workspace para compilar desde fuente (lado B), si existe.
    pub workspace_root: Option<PathBuf>,
    /// URL base de un repo remoto firmado (actualizador), si está configurado.
    pub remote_base_url: Option<String>,
    /// Caché local de blobs descargados del repo remoto.
    pub cache_dir: PathBuf,
}

impl InstallConfig {
    /// Config para un modo, autodetectando bundle, workspace y repo del entorno.
    pub fn detect(mode: InstallMode) -> Self {
        Self {
            mode,
            prefix: mode.default_prefix(),
            bundle_dir: detect_bundle_dir(),
            workspace_root: detect_workspace_root(),
            remote_base_url: std::env::var("CHURAY_REPO").ok().filter(|s| !s.is_empty()),
            cache_dir: default_cache_dir(),
        }
    }

    /// `true` si el prefix existe y no es escribible por el usuario actual
    /// (⇒ hace falta root). Si el prefix aún no existe, mira su ancestro más
    /// cercano que sí exista.
    pub fn needs_root(&self) -> bool {
        let mut p: &Path = &self.prefix;
        loop {
            if p.exists() {
                return !is_writable(p);
            }
            match p.parent() {
                Some(parent) => p = parent,
                None => return true,
            }
        }
    }
}

/// Etapa de instalación de una unidad — para streamear progreso a la UI.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Resolviendo,
    Descargando,
    Compilando,
    Copiando,
    Desktop,
    Hecho,
}

/// Errores de instalación.
#[derive(Debug, thiserror::Error)]
pub enum InstallError {
    #[error("la unidad '{0}' no se puede instalar en este modo (alcance no admitido)")]
    AlcanceNoAdmitido(String),
    #[error("no hay binario para '{program}': ni bundle ni workspace disponibles")]
    SinFuente { program: String },
    #[error("el binario de '{program}' no quedó en {path}")]
    BinarioAusente { program: String, path: PathBuf },
    #[error("hash del binario no coincide (esperado {esperado}, obtuvo {obtuvo})")]
    HashNoCoincide { esperado: String, obtuvo: String },
    #[error("falló `cargo build --bin {program}`: {detalle}")]
    BuildFallo { program: String, detalle: String },
    #[error("descarga del repo remoto: {0}")]
    Descarga(String),
    #[error("E/S: {0}")]
    Io(#[from] std::io::Error),
}

/// De dónde sale el binario de una unidad. Trait objeto-seguro para poder
/// elegir bundle vs build en runtime (y, mañana, un repo remoto).
pub trait Source {
    /// Deja el binario ejecutable de `unit` en `dest` (ruta final del binario).
    /// Debe escribir de forma **atómica** (`.tmp` + rename) y reportar etapas.
    fn provide(
        &self,
        unit: &Unit,
        dest: &Path,
        on: &mut dyn FnMut(Step, f32),
    ) -> Result<ArtifactHash, InstallError>;
}

/// Lado A: copia un binario precompilado del bundle.
pub struct BundleSource {
    pub dir: PathBuf,
}

impl Source for BundleSource {
    fn provide(
        &self,
        unit: &Unit,
        dest: &Path,
        on: &mut dyn FnMut(Step, f32),
    ) -> Result<ArtifactHash, InstallError> {
        on(Step::Copiando, 0.0);
        let src = self.dir.join("bin").join(&unit.program);
        if !src.exists() {
            return Err(InstallError::BinarioAusente {
                program: unit.program.clone(),
                path: src,
            });
        }
        let hash = atomic_install_bin(&src, dest)?;
        // Si el manifiesto declara un hash esperado, verificarlo.
        if let Some(expected) = &unit.bin_hash {
            if &hash != expected {
                let _ = std::fs::remove_file(dest);
                return Err(InstallError::HashNoCoincide {
                    esperado: expected.to_string(),
                    obtuvo: hash.to_string(),
                });
            }
        }
        on(Step::Copiando, 1.0);
        Ok(hash)
    }
}

/// Lado B: compila desde el workspace y copia de `target/release`.
pub struct BuildSource {
    pub workspace_root: PathBuf,
}

impl Source for BuildSource {
    fn provide(
        &self,
        unit: &Unit,
        dest: &Path,
        on: &mut dyn FnMut(Step, f32),
    ) -> Result<ArtifactHash, InstallError> {
        on(Step::Compilando, 0.0);
        // `--bin <program>` no necesita el nombre del crate: cargo encuentra el
        // binario en el workspace (error claro si fuera ambiguo).
        let out = std::process::Command::new("cargo")
            .current_dir(&self.workspace_root)
            .args(["build", "--release", "--bin", &unit.program])
            .output()
            .map_err(|e| InstallError::BuildFallo {
                program: unit.program.clone(),
                detalle: e.to_string(),
            })?;
        if !out.status.success() {
            let detalle = String::from_utf8_lossy(&out.stderr)
                .lines()
                .rev()
                .take(4)
                .collect::<Vec<_>>()
                .join(" | ");
            return Err(InstallError::BuildFallo { program: unit.program.clone(), detalle });
        }
        on(Step::Copiando, 0.5);
        let built = self
            .workspace_root
            .join("target")
            .join("release")
            .join(&unit.program);
        if !built.exists() {
            return Err(InstallError::BinarioAusente {
                program: unit.program.clone(),
                path: built,
            });
        }
        let hash = atomic_install_bin(&built, dest)?;
        on(Step::Copiando, 1.0);
        Ok(hash)
    }
}

/// Elige la fuente para `unit`, por prioridad: **bundle** local (instantáneo) →
/// **repo remoto** firmado (descarga, si la unidad trae hash) → **compilar**
/// del workspace. `None` si ninguna está disponible.
pub fn resolve_source(cfg: &InstallConfig, unit: &Unit) -> Option<Box<dyn Source>> {
    if let Some(dir) = &cfg.bundle_dir {
        if dir.join("bin").join(&unit.program).exists() {
            return Some(Box::new(BundleSource { dir: dir.clone() }));
        }
    }
    if let Some(url) = &cfg.remote_base_url {
        // El repo remoto sólo puede servir lo que declara por hash.
        if unit.bin_hash.is_some() {
            return Some(Box::new(crate::repo::RemoteRepo::curl(
                url.clone(),
                cfg.cache_dir.clone(),
            )));
        }
    }
    if let Some(ws) = &cfg.workspace_root {
        return Some(Box::new(BuildSource { workspace_root: ws.clone() }));
    }
    None
}

/// Instala una unidad de punta a punta: resuelve la fuente, escribe el binario,
/// siembra el `.desktop`, copia el ícono (si el bundle lo trae) y actualiza el
/// registro `state`. Reporta cada etapa por `on`.
pub fn install_unit(
    cfg: &InstallConfig,
    unit: &Unit,
    state: &mut InstalledState,
    on: &mut dyn FnMut(Step, f32),
) -> Result<(), InstallError> {
    if !cfg.mode.admits(unit.scope) {
        return Err(InstallError::AlcanceNoAdmitido(unit.id.clone()));
    }
    on(Step::Resolviendo, 0.0);
    let source = resolve_source(cfg, unit)
        .ok_or_else(|| InstallError::SinFuente { program: unit.program.clone() })?;

    let dest = cfg.prefix.join("bin").join(&unit.program);
    std::fs::create_dir_all(dest.parent().unwrap())?;
    let hash = source.provide(unit, &dest, on)?;

    on(Step::Desktop, 0.0);
    write_desktop_entry(&cfg.prefix, unit, &dest)?;
    copy_icon_if_present(cfg, unit)?;

    state.upsert(unit.id.clone(), unit.version.clone(), hash);
    state.save(&cfg.prefix)?;
    on(Step::Hecho, 1.0);
    Ok(())
}

/// Desinstala una unidad: borra binario, `.desktop` y registro. Idempotente.
pub fn uninstall_unit(
    cfg: &InstallConfig,
    unit: &Unit,
    state: &mut InstalledState,
) -> Result<(), InstallError> {
    let _ = std::fs::remove_file(cfg.prefix.join("bin").join(&unit.program));
    let _ = std::fs::remove_file(desktop_path(&cfg.prefix, unit));
    state.remove(&unit.id);
    state.save(&cfg.prefix)?;
    Ok(())
}

/// Copia `src` a `dest` de forma atómica (`dest.tmp` + rename) y le pone el bit
/// de ejecución. Devuelve el hash del binario instalado.
pub(crate) fn atomic_install_bin(src: &Path, dest: &Path) -> Result<ArtifactHash, InstallError> {
    use std::os::unix::fs::PermissionsExt;
    let tmp = dest.with_extension("tmp-install");
    std::fs::copy(src, &tmp)?;
    let mut perms = std::fs::metadata(&tmp)?.permissions();
    perms.set_mode(0o755);
    std::fs::set_permissions(&tmp, perms)?;
    std::fs::rename(&tmp, dest)?; // atómico dentro del mismo filesystem
    ArtifactHash::of_file(dest).map_err(InstallError::Io)
}

fn desktop_path(prefix: &Path, unit: &Unit) -> PathBuf {
    prefix
        .join("share")
        .join("applications")
        .join(format!("tawasuyu-{}.desktop", unit.id))
}

/// Escribe el `.desktop` freedesktop de la unidad apuntando al binario absoluto
/// (así funciona aunque `<prefix>/bin` no esté en el PATH).
pub fn write_desktop_entry(prefix: &Path, unit: &Unit, bin: &Path) -> Result<(), InstallError> {
    let path = desktop_path(prefix, unit);
    std::fs::create_dir_all(path.parent().unwrap())?;
    // Glyph unicode no sirve como Icon= (espera nombre/ruta); usamos el nombre
    // del programa, que casa con el PNG que el bundle pueda haber instalado.
    let icon_name = &unit.program;
    let categories = freedesktop_categories(&unit.category);
    let entry = format!(
        "[Desktop Entry]\n\
         Type=Application\n\
         Name={label}\n\
         Comment={desc}\n\
         Exec={exec} %f\n\
         Icon={icon}\n\
         Terminal=false\n\
         Categories={cats}\n\
         X-Tawasuyu-Id={id}\n",
        label = unit.label,
        desc = unit.description,
        exec = bin.display(),
        icon = icon_name,
        cats = categories,
        id = unit.id,
    );
    std::fs::write(&path, entry)?;
    Ok(())
}

/// Mapea el cuadrante de la suite a categorías freedesktop razonables.
fn freedesktop_categories(category: &str) -> &'static str {
    match category {
        "unanchay" => "Graphics;Office;",
        "yachay" => "Science;Education;",
        "ruway" => "Utility;AudioVideo;",
        "ukupacha" => "System;Settings;",
        "sistema" => "System;",
        _ => "Utility;",
    }
}

/// Si el bundle trae `share/icons/<program>.png`, lo copia al theme hicolor del
/// prefix para que el `.desktop` lo encuentre. Silencioso si no hay ícono.
fn copy_icon_if_present(cfg: &InstallConfig, unit: &Unit) -> Result<(), InstallError> {
    let Some(bundle) = &cfg.bundle_dir else {
        return Ok(());
    };
    let src = bundle
        .join("share")
        .join("icons")
        .join(format!("{}.png", unit.program));
    if !src.exists() {
        return Ok(());
    }
    let dst_dir = cfg.prefix.join("share/icons/hicolor/256x256/apps");
    std::fs::create_dir_all(&dst_dir)?;
    std::fs::copy(&src, dst_dir.join(format!("{}.png", unit.program)))?;
    Ok(())
}

// ---- detección de entorno ----

/// Busca el bundle precompilado: `$CHURAY_BUNDLE`, o `bundle/` junto al binario.
fn detect_bundle_dir() -> Option<PathBuf> {
    if let Some(env) = std::env::var_os("CHURAY_BUNDLE") {
        let p = PathBuf::from(env);
        if p.join("bin").is_dir() {
            return Some(p);
        }
    }
    if let Ok(exe) = std::env::current_exe() {
        let cands = [
            exe.parent().map(|d| d.join("bundle")),
            exe.parent().and_then(|d| d.parent()).map(|d| d.join("bundle")),
        ];
        for cand in cands.into_iter().flatten() {
            if cand.join("bin").is_dir() {
                return Some(cand);
            }
        }
    }
    None
}

/// Busca la raíz del workspace (lado B): `$CHURAY_WORKSPACE`, o subiendo desde
/// el cwd hasta encontrar un `Cargo.toml` con `[workspace]`.
fn detect_workspace_root() -> Option<PathBuf> {
    if let Some(env) = std::env::var_os("CHURAY_WORKSPACE") {
        let p = PathBuf::from(env);
        if p.join("Cargo.toml").exists() {
            return Some(p);
        }
    }
    let start = std::env::current_dir().ok()?;
    let mut cur: &Path = &start;
    loop {
        let manifest = cur.join("Cargo.toml");
        if manifest.exists() {
            if let Ok(txt) = std::fs::read_to_string(&manifest) {
                if txt.contains("[workspace]") {
                    return Some(cur.to_path_buf());
                }
            }
        }
        cur = cur.parent()?;
    }
}

/// Caché de blobs del repo remoto: `~/.cache/tawasuyu/churay/blobs`.
fn default_cache_dir() -> PathBuf {
    directories::BaseDirs::new()
        .map(|b| b.cache_dir().join("tawasuyu").join("churay").join("blobs"))
        .unwrap_or_else(|| std::env::temp_dir().join("churay-cache"))
}

fn is_writable(p: &Path) -> bool {
    // Heurística portable sin libc: intentar crear un archivo temporal dentro
    // (si es dir) o chequear permisos de escritura del metadata.
    if p.is_dir() {
        let probe = p.join(".churay-write-probe");
        match std::fs::File::create(&probe) {
            Ok(_) => {
                let _ = std::fs::remove_file(&probe);
                true
            }
            Err(_) => false,
        }
    } else {
        std::fs::metadata(p)
            .map(|m| !m.permissions().readonly())
            .unwrap_or(false)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::os::unix::fs::PermissionsExt;

    fn unidad(program: &str) -> Unit {
        Unit {
            id: program.into(),
            label: program.into(),
            version: "0.1.0".into(),
            category: "ruway".into(),
            icon: "≡".into(),
            description: "demo".into(),
            program: program.into(),
            scope: Scope::App,
            bin_hash: None,
            size_bytes: None,
        }
    }

    /// Bundle falso + prefix temporal: instala, y certifica binario ejecutable,
    /// `.desktop` y registro en disco — sin compilar ni renderizar nada.
    #[test]
    fn instala_desde_bundle_y_certifica_artefactos() {
        let base = std::env::temp_dir().join(format!("churay-it-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let bundle = base.join("bundle");
        let prefix = base.join("prefix");
        std::fs::create_dir_all(bundle.join("bin")).unwrap();
        let fake = bundle.join("bin").join("demo-app");
        std::fs::write(&fake, b"#!/bin/sh\necho hola\n").unwrap();
        let mut perms = std::fs::metadata(&fake).unwrap().permissions();
        perms.set_mode(0o755);
        std::fs::set_permissions(&fake, perms).unwrap();

        let cfg = InstallConfig {
            mode: InstallMode::Local,
            prefix: prefix.clone(),
            bundle_dir: Some(bundle.clone()),
            workspace_root: None,
            remote_base_url: None,
            cache_dir: std::env::temp_dir().join("churay-test-cache"),
        };
        let mut state = InstalledState::default();
        let mut pasos = Vec::new();
        let u = unidad("demo-app");
        install_unit(&cfg, &u, &mut state, &mut |step, _| pasos.push(step)).unwrap();

        let inst = prefix.join("bin").join("demo-app");
        assert!(inst.exists(), "binario instalado");
        assert!(std::fs::metadata(&inst).unwrap().permissions().mode() & 0o111 != 0);
        let desk = prefix.join("share/applications/tawasuyu-demo-app.desktop");
        let dtxt = std::fs::read_to_string(&desk).unwrap();
        assert!(dtxt.contains(&format!("Exec={} %f", inst.display())));
        assert!(dtxt.contains("X-Tawasuyu-Id=demo-app"));
        let st = InstalledState::load(&prefix);
        assert!(st.is_installed("demo-app"));
        assert!(pasos.contains(&Step::Hecho));

        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn modo_local_rechaza_unidad_de_sistema() {
        let cfg = InstallConfig {
            mode: InstallMode::Local,
            prefix: std::env::temp_dir().join("churay-noop"),
            bundle_dir: None,
            workspace_root: None,
            remote_base_url: None,
            cache_dir: std::env::temp_dir().join("churay-test-cache"),
        };
        let mut u = unidad("arje");
        u.scope = Scope::System;
        let mut state = InstalledState::default();
        let err = install_unit(&cfg, &u, &mut state, &mut |_, _| {}).unwrap_err();
        assert!(matches!(err, InstallError::AlcanceNoAdmitido(_)));
    }

    #[test]
    fn verifica_hash_del_bundle_y_acepta_si_coincide() {
        let base = std::env::temp_dir().join(format!("churay-okhash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let bundle = base.join("bundle");
        std::fs::create_dir_all(bundle.join("bin")).unwrap();
        let contenido = b"binario real y correcto";
        std::fs::write(bundle.join("bin").join("demo-app"), contenido).unwrap();

        let cfg = InstallConfig {
            mode: InstallMode::Local,
            prefix: base.join("prefix"),
            bundle_dir: Some(bundle),
            workspace_root: None,
            remote_base_url: None,
            cache_dir: std::env::temp_dir().join("churay-test-cache"),
        };
        let mut u = unidad("demo-app");
        // El manifiesto declara exactamente el hash del binario presente.
        u.bin_hash = Some(ArtifactHash::of_bytes(contenido));
        let mut state = InstalledState::default();
        install_unit(&cfg, &u, &mut state, &mut |_, _| {}).expect("hash coincide ⇒ instala");
        // Y lo registrado es ese mismo hash.
        let st = InstalledState::load(&cfg.prefix);
        assert_eq!(st.get("demo-app").unwrap().hash, ArtifactHash::of_bytes(contenido));
        let _ = std::fs::remove_dir_all(&base);
    }

    #[test]
    fn verifica_hash_del_bundle_y_falla_si_no_coincide() {
        let base = std::env::temp_dir().join(format!("churay-hash-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        let bundle = base.join("bundle");
        std::fs::create_dir_all(bundle.join("bin")).unwrap();
        std::fs::write(bundle.join("bin").join("demo-app"), b"contenido real").unwrap();

        let cfg = InstallConfig {
            mode: InstallMode::Local,
            prefix: base.join("prefix"),
            bundle_dir: Some(bundle),
            workspace_root: None,
            remote_base_url: None,
            cache_dir: std::env::temp_dir().join("churay-test-cache"),
        };
        let mut u = unidad("demo-app");
        u.bin_hash = Some(ArtifactHash::of_bytes(b"otro contenido"));
        let mut state = InstalledState::default();
        let err = install_unit(&cfg, &u, &mut state, &mut |_, _| {}).unwrap_err();
        assert!(matches!(err, InstallError::HashNoCoincide { .. }));
        let _ = std::fs::remove_dir_all(&base);
    }
}
