//! Enumeración de sesiones de escritorio instaladas.
//!
//! Un display manager no inventa qué sesiones ofrecer: las lee de los
//! directorios estándar de XDG. Cada sesión es un archivo `.desktop` con
//! un `Name` legible y un `Exec` que la arranca:
//!
//! - `…/wayland-sessions/*.desktop` → sesiones Wayland (sway, Hyprland,
//!   Plasma Wayland, GNOME…).
//! - `…/xsessions/*.desktop` → sesiones X11.
//!
//! El greeter lista lo que **ya existe** en el sistema (sin instalar
//! nada), el usuario elige una, y su `Exec` viaja en el [`SessionTicket`]
//! para que el compositor la ejecute como el usuario autenticado. Si la
//! lista no trae nada de afuera, queda al menos la entrada nativa de
//! mirada (su autostart), que es `Exec` vacío.

use std::path::Path;

/// Servidor gráfico de una sesión (sólo informativo, para etiquetar).
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Kind {
    Wayland,
    X11,
}

impl Kind {
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Wayland => "wayland",
            Kind::X11 => "x11",
        }
    }
}

/// Una sesión ofrecible en el login.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Session {
    /// Nombre legible (campo `Name` del `.desktop`).
    pub name: String,
    /// Comando que la arranca (campo `Exec`, ya sin field-codes `%U`/`%i`).
    /// Vacío ⇒ sesión nativa de mirada: el compositor cae a su autostart
    /// en vez de ejecutar un comando ajeno.
    pub exec: String,
    pub kind: Kind,
    /// `true` si es un compositor **ajeno** (descubierto en `wayland-sessions`):
    /// el handoff lo lanza por `exec` soltando el DRM. `false` para las
    /// nativas de mirada (pata, autostart), que corren como clientes.
    pub foreign: bool,
}

/// Raíces XDG donde buscar sesiones, según `XDG_DATA_HOME` y
/// `XDG_DATA_DIRS` (con los defaults del spec si faltan). De cada raíz
/// `R` se miran `R/wayland-sessions` y `R/xsessions`. Honrar XDG es lo que
/// hace un DM de verdad —y permite dropear un `.desktop` en
/// `~/.local/share/wayland-sessions` para probar sin tocar `/usr`.
fn xdg_data_roots() -> Vec<String> {
    let mut roots = Vec::new();
    // XDG_DATA_HOME (default ~/.local/share) primero: gana sobre el sistema.
    match std::env::var("XDG_DATA_HOME") {
        Ok(h) if !h.is_empty() => roots.push(h),
        _ => {
            if let Ok(home) = std::env::var("HOME") {
                roots.push(format!("{home}/.local/share"));
            }
        }
    }
    let dirs = std::env::var("XDG_DATA_DIRS")
        .ok()
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "/usr/local/share:/usr/share".to_string());
    for d in dirs.split(':').filter(|s| !s.is_empty()) {
        roots.push(d.to_string());
    }
    roots
}

/// Descubre todas las sesiones del sistema. La primera entrada es siempre
/// la nativa de mirada (`Exec` vacío) para que la lista nunca esté vacía y
/// haya siempre un camino al autostart del compositor. Deduplica por
/// `(name, exec)` —la misma sesión puede aparecer en varias raíces XDG.
pub fn discover() -> Vec<Session> {
    // Built-ins nativos: corren como clientes del propio compositor (no
    // son `foreign`). «mirada» a secas ⇒ Exec vacío ⇒ autostart del
    // usuario; «mirada · pata» ⇒ arranca el marco pata (forzado a su
    // backend de ventana, que es como mirada lo acopla por app-id).
    let mut out = vec![
        Session {
            name: "mirada".to_string(),
            exec: String::new(),
            kind: Kind::Wayland,
            foreign: false,
        },
        Session {
            // pata ancla por wlr-layer-shell (su backend nativo, que mirada
            // ahora soporta): barra con zona exclusiva, sin winit ni app-id.
            name: "mirada · pata".to_string(),
            exec: "pata-llimphi".to_string(),
            kind: Kind::Wayland,
            foreign: false,
        },
    ];
    for root in xdg_data_roots() {
        collect_dir(&Path::new(&root).join("wayland-sessions"), Kind::Wayland, &mut out);
        collect_dir(&Path::new(&root).join("xsessions"), Kind::X11, &mut out);
    }
    // Las sesiones del propio mirada (mirada.desktop, mirada-pata.desktop)
    // existen para DMs externos; aquí ya están cubiertas por los built-ins,
    // así que las filtramos para no duplicar.
    out.retain(|s| !is_mirada_session(&s.exec));
    // Dedup global por (name, exec): la misma sesión puede repetirse en
    // varias raíces XDG y no quedar contigua. `dedup_by` no basta.
    let mut seen = std::collections::HashSet::new();
    out.retain(|s| seen.insert((s.name.clone(), s.exec.clone())));
    out
}

/// ¿El `Exec` arranca el propio mirada? (`mirada-session*`,
/// `mirada-compositor`). Esas sesiones las cubren los built-ins.
fn is_mirada_session(exec: &str) -> bool {
    let first = exec.split_whitespace().next().unwrap_or("");
    let base = first.rsplit('/').next().unwrap_or(first);
    base.starts_with("mirada-session") || base == "mirada-compositor"
}

/// Lee un directorio de sesiones, parsea cada `.desktop` y agrega los
/// válidos a `out` ordenados por nombre. Un directorio inexistente o
/// ilegible se ignora en silencio (no todos los sistemas tienen ambos).
fn collect_dir(dir: &Path, kind: Kind, out: &mut Vec<Session>) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    let mut found: Vec<Session> = Vec::new();
    for entry in entries.flatten() {
        let path = entry.path();
        if path.extension().and_then(|s| s.to_str()) != Some("desktop") {
            continue;
        }
        let Ok(text) = std::fs::read_to_string(&path) else {
            continue;
        };
        if let Some(session) = parse_entry(&text, kind) {
            found.push(session);
        }
    }
    found.sort_by(|a, b| a.name.to_lowercase().cmp(&b.name.to_lowercase()));
    out.extend(found);
}

/// Parsea un `.desktop`: toma `Name` y `Exec` de la sección
/// `[Desktop Entry]`. Devuelve `None` si está oculta (`Hidden`/`NoDisplay`)
/// o no trae un `Exec` ejecutable.
fn parse_entry(text: &str, kind: Kind) -> Option<Session> {
    let mut in_main = false;
    let mut name: Option<String> = None;
    let mut exec: Option<String> = None;
    let mut hidden = false;

    for raw in text.lines() {
        let line = raw.trim();
        if line.starts_with('[') && line.ends_with(']') {
            // Sólo nos interesa la sección principal; las
            // `[Desktop Action …]` se ignoran.
            in_main = line == "[Desktop Entry]";
            continue;
        }
        if !in_main || line.starts_with('#') {
            continue;
        }
        let Some((key, value)) = line.split_once('=') else {
            continue;
        };
        let value = value.trim();
        match key.trim() {
            // Sólo la clave sin locale (`Name`, no `Name[es]`): el primero
            // que aparezca gana.
            "Name" if name.is_none() => name = Some(value.to_string()),
            "Exec" if exec.is_none() => exec = Some(strip_field_codes(value)),
            "Hidden" | "NoDisplay" if value == "true" => hidden = true,
            _ => {}
        }
    }

    if hidden {
        return None;
    }
    let exec = exec?;
    if exec.is_empty() {
        return None;
    }
    Some(Session {
        name: name.unwrap_or_else(|| exec.clone()),
        exec,
        kind,
        // Toda sesión declarada en el sistema es un compositor ajeno: el
        // handoff la lanza por `exec`, no como cliente.
        foreign: true,
    })
}

/// Quita los field-codes de un `Exec` (`%U`, `%f`, `%i`, `%k`, …). El
/// spec los reserva como tokens `%x` de dos chars; en sesiones casi nunca
/// aparecen, pero los limpiamos por las dudas para no pasarle basura a
/// `sh -c`.
fn strip_field_codes(exec: &str) -> String {
    exec.split_whitespace()
        .filter(|tok| !(tok.len() == 2 && tok.starts_with('%')))
        .collect::<Vec<_>>()
        .join(" ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsea_entrada_minima() {
        let s = parse_entry("[Desktop Entry]\nName=Sway\nExec=sway\n", Kind::Wayland).unwrap();
        assert_eq!(s.name, "Sway");
        assert_eq!(s.exec, "sway");
        assert_eq!(s.kind, Kind::Wayland);
    }

    #[test]
    fn limpia_field_codes() {
        let s = parse_entry(
            "[Desktop Entry]\nName=Plasma\nExec=startplasma-wayland %U\n",
            Kind::Wayland,
        )
        .unwrap();
        assert_eq!(s.exec, "startplasma-wayland");
    }

    #[test]
    fn ignora_ocultas() {
        assert!(parse_entry(
            "[Desktop Entry]\nName=X\nExec=x\nNoDisplay=true\n",
            Kind::Wayland
        )
        .is_none());
        assert!(
            parse_entry("[Desktop Entry]\nName=X\nExec=x\nHidden=true\n", Kind::X11).is_none()
        );
    }

    #[test]
    fn ignora_otras_secciones() {
        // El `Exec` de una `[Desktop Action]` no debe colarse como el de
        // la sesión.
        let s = parse_entry(
            "[Desktop Entry]\nName=Foo\nExec=foo\n[Desktop Action New]\nExec=foo --new\n",
            Kind::Wayland,
        )
        .unwrap();
        assert_eq!(s.exec, "foo");
    }

    #[test]
    fn sin_exec_no_es_sesion() {
        assert!(parse_entry("[Desktop Entry]\nName=Solo nombre\n", Kind::Wayland).is_none());
    }

    #[test]
    fn discover_siempre_trae_mirada() {
        let v = discover();
        assert_eq!(v[0].name, "mirada");
        assert!(v[0].exec.is_empty());
    }
}
