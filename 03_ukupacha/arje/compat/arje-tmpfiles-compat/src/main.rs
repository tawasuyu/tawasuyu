//! ente-tmpfiles-compat: aplica directivas tmpfiles.d al boot.
//!
//! Lee, en orden, los conf files de:
//!   /usr/lib/tmpfiles.d/*.conf
//!   /etc/tmpfiles.d/*.conf       (override del usuario, gana)
//!   /run/tmpfiles.d/*.conf       (efímero)
//!
//! Aplica un subset de directivas — las suficientes para el boot:
//!   d  — crear directorio (idempotente: no falla si existe)
//!   D  — crear directorio + limpiar contenido si existe
//!   f  — crear archivo (vacío, perms aplicados)
//!   L  — crear symlink (overrideable con `+L` si existe)
//!   r  — remove file (no falla si ausente)
//!   R  — remove recursivamente
//!   e  — adjust perms si existe
//!
//! Edad/cleanup (`age` field) y modos exotic (b, c, p, P) se ignoran.
//! El proceso es OneShot: corre, aplica, sale con código 0 / 1.

use std::collections::BTreeMap;
use std::fs;
use std::os::unix::fs::PermissionsExt;
use std::path::{Path, PathBuf};
use tracing::{debug, info, warn};
use tracing_subscriber::EnvFilter;

const SEARCH_DIRS: &[&str] = &[
    "/usr/lib/tmpfiles.d",
    "/etc/tmpfiles.d",
    "/run/tmpfiles.d",
];

#[derive(Debug, Clone)]
struct Directive {
    typ: char,           // d, D, f, L, r, R, e
    path: PathBuf,
    mode: Option<u32>,
    user: Option<String>,
    group: Option<String>,
    arg: Option<String>, // symlink target o content
}

fn main() {
    init_tracing();
    info!("ente-tmpfiles-compat: aplicando directivas tmpfiles.d");
    let directives = collect_directives();
    info!(count = directives.len(), "directivas a aplicar");

    let mut applied = 0;
    let mut skipped = 0;
    let mut errors = 0;
    for d in directives {
        match apply(&d) {
            Ok(true) => applied += 1,
            Ok(false) => skipped += 1,
            Err(e) => {
                warn!(?e, ?d.typ, path = %d.path.display(), "directiva falló");
                errors += 1;
            }
        }
    }
    info!(applied, skipped, errors, "tmpfiles aplicado");
    if errors > 0 { std::process::exit(1); }
}

fn collect_directives() -> Vec<Directive> {
    // Last-wins por path: /etc supera /usr/lib, /run supera /etc.
    let mut by_path: BTreeMap<(PathBuf, char), Directive> = BTreeMap::new();
    for dir in SEARCH_DIRS {
        if !Path::new(dir).exists() { continue; }
        let mut entries: Vec<_> = match fs::read_dir(dir) {
            Ok(rd) => rd.filter_map(|e| e.ok()).collect(),
            Err(_) => continue,
        };
        entries.sort_by_key(|e| e.file_name());
        for entry in entries {
            let path = entry.path();
            if path.extension().map(|e| e != "conf").unwrap_or(true) { continue; }
            match fs::read_to_string(&path) {
                Ok(content) => {
                    for (line_no, line) in content.lines().enumerate() {
                        if let Some(d) = parse_line(line) {
                            by_path.insert((d.path.clone(), d.typ), d);
                        } else if !line.trim().is_empty() && !line.trim().starts_with('#') {
                            debug!(file = %path.display(), line_no, line, "no parseable, skip");
                        }
                    }
                }
                Err(e) => warn!(?e, path = %path.display(), "read"),
            }
        }
    }
    // Orden de aplicación: removes (r/R) primero, luego creates (d/D/f/L),
    // adjusts (e) al final.
    let mut all: Vec<Directive> = by_path.into_values().collect();
    all.sort_by_key(|d| match d.typ {
        'r' | 'R' => 0,
        'd' | 'D' | 'f' | 'L' => 1,
        'e' => 2,
        _ => 3,
    });
    all
}

fn parse_line(line: &str) -> Option<Directive> {
    let line = line.trim();
    if line.is_empty() || line.starts_with('#') { return None; }
    // Formato: TYPE PATH MODE USER GROUP AGE ARGUMENT
    // Strip leading '+' (override marker) y '!' (boot-only) — los soportamos
    // implícitamente.
    let typ_str = line.chars().next()?;
    let typ = match typ_str {
        '+' | '!' => line.chars().nth(1)?,
        c => c,
    };
    if !"dDfLrRe".contains(typ) { return None; }
    // tokenize tomando en cuenta '-' como "default"
    let mut parts = line.splitn(7, char::is_whitespace).filter(|s| !s.is_empty());
    let _t = parts.next()?;
    let path = parts.next()?.to_string();
    let mode = parts.next().and_then(parse_mode);
    let user = parts.next().and_then(parse_default);
    let group = parts.next().and_then(parse_default);
    let _age = parts.next();
    let arg = parts.next().and_then(parse_default);
    Some(Directive {
        typ,
        path: PathBuf::from(path),
        mode, user, group, arg,
    })
}

fn parse_default(s: &str) -> Option<String> {
    if s == "-" { None } else { Some(s.to_string()) }
}

fn parse_mode(s: &str) -> Option<u32> {
    if s == "-" { return None; }
    u32::from_str_radix(s.trim_start_matches('~'), 8).ok()
}

fn apply(d: &Directive) -> anyhow::Result<bool> {
    match d.typ {
        'd' | 'D' => apply_d(d),
        'f' => apply_f(d),
        'L' => apply_l(d),
        'r' => apply_r(d, false),
        'R' => apply_r(d, true),
        'e' => apply_e(d),
        _ => Ok(false),
    }
}

fn apply_d(d: &Directive) -> anyhow::Result<bool> {
    fs::create_dir_all(&d.path)
        .map_err(|e| anyhow::anyhow!("mkdir {}: {e}", d.path.display()))?;
    if let Some(mode) = d.mode {
        fs::set_permissions(&d.path, fs::Permissions::from_mode(mode))?;
    }
    chown(&d.path, d.user.as_deref(), d.group.as_deref())?;
    if d.typ == 'D' {
        // Limpiar contenido (no recursivo).
        if let Ok(rd) = fs::read_dir(&d.path) {
            for entry in rd.flatten() {
                let p = entry.path();
                if p.is_dir() { let _ = fs::remove_dir_all(&p); }
                else { let _ = fs::remove_file(&p); }
            }
        }
    }
    info!(path = %d.path.display(), mode = ?d.mode, "d/D aplicado");
    Ok(true)
}

fn apply_f(d: &Directive) -> anyhow::Result<bool> {
    if !d.path.exists() {
        if let Some(parent) = d.path.parent() {
            let _ = fs::create_dir_all(parent);
        }
        let content = d.arg.clone().unwrap_or_default();
        fs::write(&d.path, content.as_bytes())?;
    }
    if let Some(mode) = d.mode {
        fs::set_permissions(&d.path, fs::Permissions::from_mode(mode))?;
    }
    chown(&d.path, d.user.as_deref(), d.group.as_deref())?;
    info!(path = %d.path.display(), mode = ?d.mode, "f aplicado");
    Ok(true)
}

fn apply_l(d: &Directive) -> anyhow::Result<bool> {
    let target = match &d.arg {
        Some(t) => t,
        None => anyhow::bail!("L sin target en {}", d.path.display()),
    };
    if d.path.exists() {
        // No sobreescribimos symlinks/files existentes (modo no-`+`).
        debug!(path = %d.path.display(), "L: existe, skip");
        return Ok(false);
    }
    if let Some(parent) = d.path.parent() {
        let _ = fs::create_dir_all(parent);
    }
    std::os::unix::fs::symlink(target, &d.path)?;
    info!(path = %d.path.display(), %target, "L aplicado");
    Ok(true)
}

fn apply_r(d: &Directive, recursive: bool) -> anyhow::Result<bool> {
    if !d.path.exists() {
        return Ok(false);
    }
    if recursive {
        fs::remove_dir_all(&d.path)?;
    } else if d.path.is_dir() {
        fs::remove_dir(&d.path)?;
    } else {
        fs::remove_file(&d.path)?;
    }
    info!(path = %d.path.display(), recursive, "remove aplicado");
    Ok(true)
}

fn apply_e(d: &Directive) -> anyhow::Result<bool> {
    if !d.path.exists() {
        return Ok(false);
    }
    if let Some(mode) = d.mode {
        fs::set_permissions(&d.path, fs::Permissions::from_mode(mode))?;
    }
    chown(&d.path, d.user.as_deref(), d.group.as_deref())?;
    info!(path = %d.path.display(), "e aplicado");
    Ok(true)
}

fn chown(path: &Path, user: Option<&str>, group: Option<&str>) -> anyhow::Result<()> {
    use std::ffi::CString;
    let uid = match user {
        Some(u) => Some(lookup_uid(u)?),
        None => None,
    };
    let gid = match group {
        Some(g) => Some(lookup_gid(g)?),
        None => None,
    };
    let (uid, gid) = (uid.unwrap_or(u32::MAX), gid.unwrap_or(u32::MAX));
    let cstr = CString::new(path.as_os_str().as_encoded_bytes())?;
    let r = unsafe { libc::chown(cstr.as_ptr(), uid, gid) };
    if r != 0 {
        let e = std::io::Error::last_os_error();
        // No-op si ya somos non-root y el chown falla con EPERM.
        if e.raw_os_error() == Some(libc::EPERM) {
            debug!(path = %path.display(), "chown EPERM (esperado sin root)");
            return Ok(());
        }
        return Err(anyhow::anyhow!("chown: {e}"));
    }
    Ok(())
}

fn lookup_uid(name: &str) -> anyhow::Result<u32> {
    if let Ok(n) = name.parse::<u32>() { return Ok(n); }
    let cstr = std::ffi::CString::new(name)?;
    let pw = unsafe { libc::getpwnam(cstr.as_ptr()) };
    if pw.is_null() { anyhow::bail!("user '{name}' no encontrado"); }
    Ok(unsafe { (*pw).pw_uid })
}

fn lookup_gid(name: &str) -> anyhow::Result<u32> {
    if let Ok(n) = name.parse::<u32>() { return Ok(n); }
    let cstr = std::ffi::CString::new(name)?;
    let gr = unsafe { libc::getgrnam(cstr.as_ptr()) };
    if gr.is_null() { anyhow::bail!("group '{name}' no encontrado"); }
    Ok(unsafe { (*gr).gr_gid })
}

fn init_tracing() {
    let filter = EnvFilter::try_from_default_env()
        .unwrap_or_else(|_| EnvFilter::new("arje_tmpfiles_compat=info"));
    tracing_subscriber::fmt().with_env_filter(filter).with_target(true).init();
}
