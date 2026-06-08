//! Sub-collapsables inteligentes dentro del bloque de un comando.
//!
//! Un comando puede emitir output con estructura conocida (un árbol `ls -R`,
//! un log de `claude code` con secciones, un diff con hunks…). Este módulo
//! mira el comando y, si conoce el patrón, parte la salida en `Section`s —
//! cada una se renderiza con su propio header colapsable dentro del card.
//!
//! Cuando ningún detector matchea, retorna `None` y el block cae al render
//! por defecto (text-editor virtualizado de líneas planas).
//!
//! El estado de qué sub-collapsable está plegado vive en
//! [`crate::State::section_collapsed`] como `(block, section_idx)`.

/// Un trozo del output con un título y sus líneas. El render lo pinta como
/// chevron + header clickeable + (si está abierto) las líneas tabuladas.
#[derive(Debug, Clone)]
pub struct Section {
    pub title: String,
    pub lines: Vec<String>,
}

/// Detecta si `cmd` tiene un patrón conocido y devuelve la lista de
/// secciones derivadas de `lines`. Si no aplica, retorna `None`.
pub fn detect_sections(cmd: &str, lines: &[String]) -> Option<Vec<Section>> {
    let cmd_trimmed = cmd.trim_start().trim_start_matches('$').trim_start();
    let tokens: Vec<&str> = cmd_trimmed.split_whitespace().collect();
    if tokens.is_empty() {
        return None;
    }
    match tokens[0] {
        "ls" => detect_ls(&tokens[1..], lines),
        _ => None,
    }
}

/// Detecta el output de `ls -R` (o `ls -lR` etc.): bloques separados por
/// línea en blanco, cada uno con un header `path:` y luego las entradas.
/// Devuelve `None` si `ls` no es recursivo o si no se ve el patrón.
fn detect_ls(flags: &[&str], lines: &[String]) -> Option<Vec<Section>> {
    let recursive = flags
        .iter()
        .any(|f| f.starts_with('-') && !f.starts_with("--") && f.contains('R'))
        || flags.iter().any(|f| *f == "--recursive");
    if !recursive {
        return None;
    }
    // El patrón `ls -R` siempre arranca con un header `path:` (a veces `.:`
    // para el cwd). Si no se ve, asumimos que el output viene roto o que
    // hay solo un directorio (output corto) y cae al render normal.
    if !lines.first().map(|l| l.trim_end().ends_with(':')).unwrap_or(false) {
        return None;
    }
    let mut sections: Vec<Section> = Vec::new();
    let mut current: Option<Section> = None;
    for line in lines {
        let trimmed = line.trim_end();
        if trimmed.ends_with(':') && !trimmed.starts_with(' ') && !trimmed.starts_with('\t') {
            // Header de directorio. Cierra el actual y abre uno nuevo.
            if let Some(s) = current.take() {
                sections.push(s);
            }
            current = Some(Section {
                title: trimmed.trim_end_matches(':').to_string(),
                lines: Vec::new(),
            });
        } else if trimmed.is_empty() {
            // Línea en blanco = separador entre dirs en `ls -R`. La
            // tragamos sin agregarla al body de la sección actual.
            continue;
        } else if let Some(s) = current.as_mut() {
            s.lines.push(line.clone());
        } else {
            // Línea antes del primer header — output no clásico, abort.
            return None;
        }
    }
    if let Some(s) = current {
        sections.push(s);
    }
    if sections.is_empty() {
        None
    } else {
        Some(sections)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ls_r_clasico_se_parte_por_directorio() {
        let lines = vec![
            ".:".to_string(),
            "a  b  c".to_string(),
            "".to_string(),
            "./sub:".to_string(),
            "d  e".to_string(),
        ];
        let secs = detect_sections("ls -R", &lines).expect("detect");
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[0].title, ".");
        assert_eq!(secs[0].lines, vec!["a  b  c"]);
        assert_eq!(secs[1].title, "./sub");
        assert_eq!(secs[1].lines, vec!["d  e"]);
    }

    #[test]
    fn ls_sin_R_no_se_secciona() {
        let lines = vec!["a".to_string(), "b".to_string()];
        assert!(detect_sections("ls -la", &lines).is_none());
    }

    #[test]
    fn comando_desconocido_no_secciona() {
        let lines = vec!["foo".to_string()];
        assert!(detect_sections("echo foo", &lines).is_none());
    }

    #[test]
    fn ls_lR_combinado_se_parte() {
        let lines = vec![
            ".:".to_string(),
            "total 8".to_string(),
            "-rw-r--r-- 1 u u 0 mar 1 12:00 a".to_string(),
            "".to_string(),
            "./d:".to_string(),
            "total 4".to_string(),
            "-rw-r--r-- 1 u u 0 mar 1 12:00 b".to_string(),
        ];
        let secs = detect_sections("ls -lR", &lines).expect("detect");
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[1].title, "./d");
        assert_eq!(secs[1].lines.len(), 2);
    }
}
