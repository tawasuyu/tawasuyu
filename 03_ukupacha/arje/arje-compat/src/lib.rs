//! `arje-compat` — shims D-Bus que traducen interfaces de systemd al
//! bus interno de `arje`, en un solo crate con un binario por servicio.
//!
//! Cada binario (`arje-{hostnamed,localed,logind,…}-compat`) es una
//! cáscara D-Bus que recibe llamadas systemd y las reenvía como cards
//! `arje-bus`. La lógica pura compartida —parseo `KEY=value`, escritura
//! atómica, validadores— vive en esta lib y se testea en un solo lugar.
//!
//! Antes vivía como 14 crates aislados bajo `arje/compat/*` + el lib
//! `arje-compat-common`. Cada uno duplicaba parseo y escritura sin un
//! solo test. La consolidación a un crate con `[[bin]]` por servicio
//! deja el código duplicado fuera y respeta la regla del PLAN.md §6.2
//! "un dominio = un crate raíz + subcrates plugin, sin proliferación".

#![forbid(unsafe_code)]

use std::io;
use std::path::Path;

/// Escribe `content` en `path` de forma atómica: vuelca a un archivo
/// temporal vecino, hace `fsync`, y renombra sobre el destino —de modo
/// que un lector nunca ve un archivo a medio escribir—. Permisos 0644.
/// Crea el directorio padre si falta.
pub fn atomic_write(path: impl AsRef<Path>, content: &[u8]) -> io::Result<()> {
    use io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let p = path.as_ref();
    if let Some(parent) = p.parent() {
        let _ = std::fs::create_dir_all(parent);
    }
    let tmp = p.with_extension("tmp");
    {
        let mut f = std::fs::OpenOptions::new()
            .create(true)
            .write(true)
            .truncate(true)
            .mode(0o644)
            .open(&tmp)?;
        f.write_all(content)?;
        f.sync_all()?;
    }
    std::fs::rename(&tmp, p)
}

/// Busca `key` en un contenido de líneas `KEY=value` y devuelve su
/// valor, trimeado y sin las comillas envolventes. Ignora líneas en
/// blanco y comentarios (`#`). `None` si la clave no aparece.
pub fn parse_kv(content: &str, key: &str) -> Option<String> {
    for line in content.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }
        if let Some((k, v)) = line.split_once('=') {
            if k.trim() == key {
                return Some(v.trim().trim_matches('"').to_string());
            }
        }
    }
    None
}

/// Actualiza —o inserta, si no existe— la clave `key` con `value` en un
/// contenido de líneas `KEY=value`. Conserva el resto de las líneas y
/// su orden. El resultado termina en `\n`.
pub fn merge_kv(existing: &str, key: &str, value: &str) -> String {
    let mut out = String::new();
    let mut found = false;
    for line in existing.lines() {
        if let Some((k, _)) = line.split_once('=') {
            if k.trim() == key {
                out.push_str(&format!("{key}={value}\n"));
                found = true;
                continue;
            }
        }
        out.push_str(line);
        out.push('\n');
    }
    if !found {
        out.push_str(&format!("{key}={value}\n"));
    }
    out
}

/// Las entradas significativas de un archivo de config tipo
/// `locale.conf`: las líneas no vacías y no comentadas, trimeadas.
pub fn conf_entries(content: &str) -> Vec<String> {
    content
        .lines()
        .map(|l| l.trim())
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
        .map(String::from)
        .collect()
}

/// `true` si `s` es un hostname válido: ASCII alfanumérico más `-`,
/// `.`, `_`; longitud `1..=253`; sin espacios ni caracteres de control.
pub fn is_valid_hostname(s: &str) -> bool {
    if s.is_empty() || s.len() > 253 {
        return false;
    }
    s.chars()
        .all(|c| c.is_ascii_alphanumeric() || c == '-' || c == '.' || c == '_')
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parse_kv_encuentra_trimea_y_quita_comillas() {
        let c = "# comentario\nLANG=en_US.UTF-8\n  LC_TIME = \"es_AR.UTF-8\" \n";
        assert_eq!(parse_kv(c, "LANG").as_deref(), Some("en_US.UTF-8"));
        assert_eq!(parse_kv(c, "LC_TIME").as_deref(), Some("es_AR.UTF-8"));
        assert_eq!(parse_kv(c, "AUSENTE"), None);
    }

    #[test]
    fn parse_kv_ignora_comentarios_y_blancos() {
        assert_eq!(parse_kv("#LANG=x\n\nLANG=real\n", "LANG").as_deref(), Some("real"));
    }

    #[test]
    fn merge_kv_actualiza_la_clave_existente_y_conserva_el_resto() {
        let antes = "LANG=C\nKEYMAP=us\n";
        let despues = merge_kv(antes, "KEYMAP", "es");
        assert!(despues.contains("LANG=C\n"), "conserva otras líneas");
        assert!(despues.contains("KEYMAP=es\n"));
        assert!(!despues.contains("KEYMAP=us"));
    }

    #[test]
    fn merge_kv_inserta_la_clave_nueva_al_final() {
        let despues = merge_kv("LANG=C\n", "KEYMAP", "us");
        assert!(despues.contains("LANG=C\n"));
        assert!(despues.ends_with("KEYMAP=us\n"));
    }

    #[test]
    fn merge_kv_sobre_contenido_vacio() {
        assert_eq!(merge_kv("", "KEYMAP", "us"), "KEYMAP=us\n");
    }

    #[test]
    fn conf_entries_descarta_blancos_y_comentarios() {
        let c = "\n# header\nLANG=en\n   \nLC_TIME=es\n";
        assert_eq!(conf_entries(c), vec!["LANG=en", "LC_TIME=es"]);
    }

    #[test]
    fn is_valid_hostname_acepta_y_rechaza() {
        assert!(is_valid_hostname("arje-host"));
        assert!(is_valid_hostname("host.local"));
        assert!(!is_valid_hostname(""), "vacío");
        assert!(!is_valid_hostname("con espacio"));
        assert!(!is_valid_hostname("tab\tinside"));
        assert!(!is_valid_hostname(&"x".repeat(254)), "demasiado largo");
    }

    #[test]
    fn atomic_write_escribe_y_sobrescribe() {
        let dir = std::env::temp_dir();
        let path = dir.join(format!("arje-compat-test-{}.conf", std::process::id()));
        atomic_write(&path, b"primero\n").expect("escribe");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "primero\n");
        atomic_write(&path, b"segundo\n").expect("sobrescribe");
        assert_eq!(std::fs::read_to_string(&path).unwrap(), "segundo\n");
        let _ = std::fs::remove_file(&path);
    }
}
