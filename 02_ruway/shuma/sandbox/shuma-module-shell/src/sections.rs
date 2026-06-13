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

/// Cómo renderizar el body de una sección.
#[derive(Debug, Clone)]
pub enum SectionKind {
    /// Líneas planas — render por defecto (text-editor / line store).
    Lines(Vec<String>),
    /// Tabla con columnas + filas. El renderer las pinta como grid con
    /// headers clickeables para ordenar.
    Table {
        columns: Vec<String>,
        rows: Vec<Vec<String>>,
    },
}

impl SectionKind {
    /// Conteo de elementos representativo (líneas o filas).
    pub fn count(&self) -> usize {
        match self {
            SectionKind::Lines(v) => v.len(),
            SectionKind::Table { rows, .. } => rows.len(),
        }
    }

    /// Acceso compatible al modo "lines" — quien renderiza por líneas
    /// virtualizadas (el path histórico) puede pedir un slice.
    pub fn as_lines(&self) -> Option<&[String]> {
        match self {
            SectionKind::Lines(v) => Some(v.as_slice()),
            _ => None,
        }
    }
}

/// Un trozo del output con un título y su body. El render lo pinta como
/// chevron + header clickeable + (si está abierto) las líneas / tabla.
#[derive(Debug, Clone)]
pub struct Section {
    pub title: String,
    pub kind: SectionKind,
}

impl Section {
    /// Helper de compatibilidad para callers que ya leían `.lines`. Si la
    /// sección es Lines, devuelve sus líneas; si es Table, las serializa
    /// como joined-text (no se usa en render — sólo para fallback).
    pub fn lines(&self) -> Vec<String> {
        match &self.kind {
            SectionKind::Lines(v) => v.clone(),
            SectionKind::Table { rows, .. } => rows.iter().map(|r| r.join("  ")).collect(),
        }
    }
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
        ":stats" => detect_stats(lines),
        "git" if tokens.get(1) == Some(&"status") => detect_git_status(&tokens[2..], lines),
        "cargo" | "rustc" => detect_cargo(lines),
        // Comandos cuya salida es una tabla con header alineado a ancho fijo:
        // `docker ps`, `podman ps`, `kubectl get`, `systemctl list-units`,
        // `ps aux`, `df -h`, `lsblk`… Las columnas se cortan por la posición
        // de inicio de cada header (left-aligned, padded al ancho de la
        // columna). El detector es seguro: si no ve un header tabular,
        // devuelve `None` y el bloque cae al render plano.
        "docker" | "podman" | "kubectl" | "systemctl" | "ps" | "df" | "lsblk" => {
            header_table(lines).map(|(columns, rows)| {
                vec![Section { title: String::new(), kind: SectionKind::Table { columns, rows } }]
            })
        }
        _ => None,
    }
}

/// Tabla con header alineado a ancho fijo (`docker ps`, `kubectl get`, `ps
/// aux`, `df`…). Toma la primera línea no vacía como header, deriva la
/// posición de inicio de cada columna (un no-espacio precedido de ≥2
/// espacios, o el inicio de línea) y corta cada fila por esas posiciones.
/// Esto maneja celdas vacías (slice vacío) y valores con espacios simples
/// (`Up 2 hours`, `2 hours ago`). `None` si no hay ≥2 columnas o ninguna
/// fila de datos.
fn header_table(lines: &[String]) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let header_idx = lines.iter().position(|l| !l.trim().is_empty())?;
    let hchars: Vec<char> = lines[header_idx].chars().collect();
    // Posiciones (en chars) donde arranca cada columna.
    let mut starts: Vec<usize> = Vec::new();
    for (i, c) in hchars.iter().enumerate() {
        if c.is_whitespace() {
            continue;
        }
        let nuevo = i == 0
            || (i >= 2 && hchars[i - 1].is_whitespace() && hchars[i - 2].is_whitespace());
        if nuevo {
            starts.push(i);
        }
    }
    if starts.len() < 2 {
        return None;
    }
    let slice = |chars: &[char], a: usize, b: Option<usize>| -> String {
        let end = b.unwrap_or(chars.len()).min(chars.len());
        let a = a.min(chars.len());
        if a >= end {
            return String::new();
        }
        chars[a..end].iter().collect::<String>().trim().to_string()
    };
    let columns: Vec<String> = (0..starts.len())
        .map(|k| slice(&hchars, starts[k], starts.get(k + 1).copied()))
        .collect();
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in &lines[header_idx + 1..] {
        if line.trim().is_empty() {
            continue;
        }
        let rc: Vec<char> = line.chars().collect();
        let cells: Vec<String> = (0..starts.len())
            .map(|k| slice(&rc, starts[k], starts.get(k + 1).copied()))
            .collect();
        if cells.iter().all(|c| c.is_empty()) {
            continue;
        }
        rows.push(cells);
    }
    if rows.is_empty() {
        None
    } else {
        Some((columns, rows))
    }
}

/// `git status`: en forma corta (`-s`/`--short`/`--porcelain`) una tabla
/// `XY · estado · archivo`; en forma larga, una sección por grupo (rama,
/// staged, modificados, sin seguimiento, conflictos).
fn detect_git_status(args: &[&str], lines: &[String]) -> Option<Vec<Section>> {
    let short = args.iter().any(|a| {
        matches!(*a, "-s" | "--short" | "--porcelain")
            || (a.starts_with('-') && !a.starts_with("--") && a.contains('s'))
    });
    if short {
        detect_git_status_short(lines)
    } else {
        detect_git_status_long(lines)
    }
}

/// Etiqueta legible para el código `XY` de `git status -s`.
fn git_xy_label(xy: &str) -> String {
    let c: Vec<char> = xy.chars().collect();
    let x = c.first().copied().unwrap_or(' ');
    let y = c.get(1).copied().unwrap_or(' ');
    if x == '?' && y == '?' {
        return "sin seguimiento".into();
    }
    if x == '!' && y == '!' {
        return "ignorado".into();
    }
    if x == 'U' || y == 'U' || (x == 'A' && y == 'A') || (x == 'D' && y == 'D') {
        return "conflicto".into();
    }
    // El staged (X) manda para el verbo; si no, el del árbol (Y).
    let code = if x != ' ' { x } else { y };
    let staged = if x != ' ' && x != '?' { " (staged)" } else { "" };
    let verbo = match code {
        'M' => "modificado",
        'A' => "agregado",
        'D' => "borrado",
        'R' => "renombrado",
        'C' => "copiado",
        'T' => "tipo cambiado",
        _ => "—",
    };
    format!("{verbo}{staged}")
}

fn detect_git_status_short(lines: &[String]) -> Option<Vec<Section>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for l in lines {
        // `-sb` antepone una línea de rama `## main...origin/main`.
        if l.starts_with("##") {
            continue;
        }
        let chars: Vec<char> = l.chars().collect();
        if chars.len() < 3 {
            continue;
        }
        let xy: String = chars[..2].iter().collect();
        let file: String = chars[3..].iter().collect::<String>().trim().to_string();
        if file.is_empty() {
            continue;
        }
        rows.push(vec![xy.clone(), git_xy_label(&xy), file]);
    }
    if rows.is_empty() {
        return None;
    }
    Some(vec![Section {
        title: String::new(),
        kind: SectionKind::Table {
            columns: vec!["XY".into(), "estado".into(), "archivo".into()],
            rows,
        },
    }])
}

fn detect_git_status_long(lines: &[String]) -> Option<Vec<Section>> {
    // Título humano para cada encabezado de grupo de `git status`.
    fn heading_of(line: &str) -> Option<&'static str> {
        let t = line.trim_start();
        if t.starts_with("Changes to be committed") {
            Some("staged")
        } else if t.starts_with("Changes not staged for commit") {
            Some("modificados")
        } else if t.starts_with("Untracked files") {
            Some("sin seguimiento")
        } else if t.starts_with("Unmerged paths") {
            Some("conflictos")
        } else {
            None
        }
    }
    let mut preamble: Vec<String> = Vec::new();
    let mut sections: Vec<Section> = Vec::new();
    let mut cur: Option<(String, Vec<String>)> = None;
    let flush = |cur: &mut Option<(String, Vec<String>)>, out: &mut Vec<Section>| {
        if let Some((title, body)) = cur.take() {
            if !body.is_empty() {
                out.push(Section { title, kind: SectionKind::Lines(body) });
            }
        }
    };
    for l in lines {
        if let Some(title) = heading_of(l) {
            flush(&mut cur, &mut sections);
            cur = Some((title.to_string(), Vec::new()));
            continue;
        }
        let t = l.trim();
        // Las líneas de pista `(use "git …")` y las vacías no son archivos.
        if t.is_empty() || t.starts_with('(') {
            continue;
        }
        match cur.as_mut() {
            Some((_, body)) => body.push(t.to_string()),
            None => preamble.push(t.to_string()),
        }
    }
    flush(&mut cur, &mut sections);
    if sections.is_empty() {
        return None;
    }
    // La preamble (rama / tracking) va primero para no perderla.
    if !preamble.is_empty() {
        sections.insert(
            0,
            Section { title: "rama".to_string(), kind: SectionKind::Lines(preamble) },
        );
    }
    Some(sections)
}

/// Salida de `cargo`/`rustc`: una sección colapsable por diagnóstico
/// (cada `error…`/`warning:` arranca uno; el preámbulo `Compiling…` va a
/// «salida»). `None` si no hay ningún diagnóstico — así un `cargo run`
/// normal cae al render plano.
fn detect_cargo(lines: &[String]) -> Option<Vec<Section>> {
    let is_diag = |l: &str| l.starts_with("error") || l.starts_with("warning:");
    if !lines.iter().any(|l| is_diag(l)) {
        return None;
    }
    let mut preamble: Vec<String> = Vec::new();
    let mut diags: Vec<Section> = Vec::new();
    let mut cur: Option<(String, Vec<String>)> = None;
    for l in lines {
        if is_diag(l) {
            if let Some((title, body)) = cur.take() {
                diags.push(Section { title, kind: SectionKind::Lines(body) });
            }
            // El título lleva la línea del diagnóstico; el body, el contexto
            // (los `-->`, el caret, las notas). Sin duplicar la primera línea.
            cur = Some((l.trim_end().to_string(), Vec::new()));
        } else if let Some((_, body)) = cur.as_mut() {
            body.push(l.clone());
        } else {
            preamble.push(l.clone());
        }
    }
    if let Some((title, body)) = cur.take() {
        diags.push(Section { title, kind: SectionKind::Lines(body) });
    }
    let mut out: Vec<Section> = Vec::new();
    let pre: Vec<String> = preamble.into_iter().filter(|l| !l.trim().is_empty()).collect();
    if !pre.is_empty() {
        out.push(Section { title: "salida".to_string(), kind: SectionKind::Lines(pre) });
    }
    out.extend(diags);
    if out.is_empty() {
        None
    } else {
        Some(out)
    }
}

/// Detecta el reporte de `:stats` (E6): líneas de resumen sin tabulador y un
/// bloque tab-separado (header + filas). Devuelve dos secciones: «resumen»
/// (las líneas sin tab) y «por comando» (la tabla ordenable). El productor es
/// [`crate::update::apply_stats`]; el delimitador `\t` no aparece en nombres
/// de binario ni en los enteros que emite, así que el ida-y-vuelta es estable.
fn detect_stats(lines: &[String]) -> Option<Vec<Section>> {
    let mut resumen: Vec<String> = Vec::new();
    let mut header: Option<Vec<String>> = None;
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in lines {
        if !line.contains('\t') {
            if !line.trim().is_empty() {
                resumen.push(line.clone());
            }
            continue;
        }
        let cells: Vec<String> = line.split('\t').map(|c| c.to_string()).collect();
        match &header {
            None => header = Some(cells),
            Some(h) if cells.len() == h.len() => rows.push(cells),
            // Fila desalineada: la ignoramos en vez de romper la tabla.
            Some(_) => {}
        }
    }
    let columns = header?;
    let mut sections = Vec::new();
    if !resumen.is_empty() {
        sections.push(Section {
            title: "resumen".to_string(),
            kind: SectionKind::Lines(resumen),
        });
    }
    sections.push(Section {
        title: "por comando".to_string(),
        kind: SectionKind::Table { columns, rows },
    });
    Some(sections)
}

/// Detecta el output de `ls` con `-l` y/o `-R` y devuelve secciones:
/// - `-R` solo: una sección por directorio, cada una con líneas planas.
/// - `-l` solo: una sección sin título con `SectionKind::Table` parseada.
/// - `-lR`: una sección por directorio, cada una con tabla.
/// Devuelve `None` si no aparece ni `-l` ni `-R`, o si el output no
/// matchea el patrón clásico.
fn detect_ls(flags: &[&str], lines: &[String]) -> Option<Vec<Section>> {
    let has_long = flags
        .iter()
        .any(|f| f.starts_with('-') && !f.starts_with("--") && f.contains('l'))
        || flags.iter().any(|f| *f == "--long" || *f == "--format=long");
    let recursive = flags
        .iter()
        .any(|f| f.starts_with('-') && !f.starts_with("--") && f.contains('R'))
        || flags.iter().any(|f| *f == "--recursive");
    if !has_long && !recursive {
        return None;
    }
    if recursive {
        // El patrón `ls -R` siempre arranca con un header `path:`.
        if !lines.first().map(|l| l.trim_end().ends_with(':')).unwrap_or(false) {
            return None;
        }
        let mut sections: Vec<Section> = Vec::new();
        let mut current_title: Option<String> = None;
        let mut current_lines: Vec<String> = Vec::new();
        let flush = |title: Option<String>, lines: Vec<String>, out: &mut Vec<Section>, long: bool| {
            if let Some(t) = title {
                let kind = if long {
                    parse_ls_long_table(&lines)
                        .map(|(cols, rows)| SectionKind::Table { columns: cols, rows })
                        .unwrap_or_else(|| SectionKind::Lines(lines.clone()))
                } else {
                    SectionKind::Lines(lines)
                };
                out.push(Section { title: t, kind });
            }
        };
        for line in lines {
            let trimmed = line.trim_end();
            if trimmed.ends_with(':')
                && !trimmed.starts_with(' ')
                && !trimmed.starts_with('\t')
            {
                flush(
                    current_title.take(),
                    std::mem::take(&mut current_lines),
                    &mut sections,
                    has_long,
                );
                current_title = Some(trimmed.trim_end_matches(':').to_string());
            } else if trimmed.is_empty() {
                continue;
            } else if current_title.is_some() {
                current_lines.push(line.clone());
            } else {
                return None;
            }
        }
        flush(current_title, current_lines, &mut sections, has_long);
        if sections.is_empty() {
            None
        } else {
            Some(sections)
        }
    } else {
        // `-l` solo: una sección única sin header con tabla.
        let (cols, rows) = parse_ls_long_table(lines)?;
        Some(vec![Section {
            title: String::new(),
            kind: SectionKind::Table { columns: cols, rows },
        }])
    }
}

/// Parser básico de líneas `ls -l`. Cada línea típica:
/// `-rw-r--r-- 1 sergio sergio 1234 mar  1 12:34 nombre con espacios`
/// Columns: perms, links, owner, group, size, date (3 tokens), name.
/// La primera línea `total N` se descarta. Devuelve `None` si no se ve el
/// patrón en al menos una línea (puede haber unas pocas no-conformes que
/// se ignoran — devolvemos `Some` si rescatamos ≥1 fila).
fn parse_ls_long_table(lines: &[String]) -> Option<(Vec<String>, Vec<Vec<String>>)> {
    let cols = vec![
        "permisos".to_string(),
        "links".to_string(),
        "owner".to_string(),
        "group".to_string(),
        "size".to_string(),
        "fecha".to_string(),
        "nombre".to_string(),
    ];
    let mut rows: Vec<Vec<String>> = Vec::new();
    for line in lines {
        let trimmed = line.trim_end();
        if trimmed.is_empty() {
            continue;
        }
        if trimmed.starts_with("total ") {
            continue;
        }
        // Tomamos los primeros 8 tokens whitespace-separated; el resto es
        // el nombre (que puede tener espacios).
        let mut it = trimmed.split_whitespace();
        let perms = match it.next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        // Sanity check: perms tiene 10 caracteres (drwxr-xr-x) o con ACL `+`.
        if perms.len() < 10 {
            continue;
        }
        let links = match it.next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        let owner = match it.next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        let group = match it.next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        let size = match it.next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        let d1 = match it.next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        let d2 = match it.next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        let d3 = match it.next() {
            Some(s) => s.to_string(),
            None => continue,
        };
        let fecha = format!("{d1} {d2} {d3}");
        let nombre = it.collect::<Vec<_>>().join(" ");
        if nombre.is_empty() {
            continue;
        }
        rows.push(vec![perms, links, owner, group, size, fecha, nombre]);
    }
    if rows.is_empty() {
        None
    } else {
        Some((cols, rows))
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
        assert_eq!(secs[0].as_lines_for_test(), Some(vec!["a  b  c".to_string()]));
        assert_eq!(secs[1].title, "./sub");
        assert_eq!(secs[1].as_lines_for_test(), Some(vec!["d  e".to_string()]));
    }

    #[test]
    fn ls_l_solo_devuelve_tabla() {
        let lines = vec![
            "total 8".to_string(),
            "-rw-r--r-- 1 u u 0 mar 1 12:00 a".to_string(),
            "-rw-r--r-- 1 u u 42 mar 1 12:00 nombre con espacios".to_string(),
        ];
        let secs = detect_sections("ls -l", &lines).expect("detect");
        assert_eq!(secs.len(), 1);
        match &secs[0].kind {
            SectionKind::Table { columns, rows } => {
                assert_eq!(columns.len(), 7);
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[1][6], "nombre con espacios");
            }
            _ => panic!("esperaba Table"),
        }
    }

    #[test]
    fn ls_sin_l_ni_R_no_se_secciona() {
        let lines = vec!["a".to_string(), "b".to_string()];
        assert!(detect_sections("ls -a", &lines).is_none());
    }

    #[test]
    fn comando_desconocido_no_secciona() {
        let lines = vec!["foo".to_string()];
        assert!(detect_sections("echo foo", &lines).is_none());
    }

    #[test]
    fn ls_lR_combinado_da_tablas_por_dir() {
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
        assert!(matches!(secs[0].kind, SectionKind::Table { .. }));
        assert!(matches!(secs[1].kind, SectionKind::Table { .. }));
        assert_eq!(secs[1].title, "./d");
    }

    #[test]
    fn stats_se_parte_en_resumen_y_tabla() {
        let lines = vec![
            "120 comandos en historial · 8 binarios distintos · 100 con código de salida".to_string(),
            "comando\tveces\tfallos\t%fallo\tp50ms\tp95ms\túltimo".to_string(),
            "cargo\t30\t2\t6\t1500\t4200\t2m".to_string(),
            "git\t12\t0\t0\t40\t90\t1h".to_string(),
        ];
        let secs = detect_sections(":stats", &lines).expect("detect");
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[0].title, "resumen");
        assert!(matches!(secs[0].kind, SectionKind::Lines(_)));
        assert_eq!(secs[1].title, "por comando");
        match &secs[1].kind {
            SectionKind::Table { columns, rows } => {
                assert_eq!(columns.len(), 7);
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0][0], "cargo");
                assert_eq!(rows[0][1], "30");
            }
            _ => panic!("esperaba Table"),
        }
    }

    #[test]
    fn docker_ps_se_parsea_como_tabla() {
        let lines = vec![
            "CONTAINER ID   IMAGE          COMMAND                  CREATED         STATUS          PORTS                    NAMES".to_string(),
            "abc123def456   nginx:1.27     \"/docker-entrypoint.…\"   2 hours ago     Up 2 hours      0.0.0.0:80->80/tcp       web".to_string(),
            "789aaa111bbb   postgres:16    \"docker-entrypoint.s…\"   3 days ago      Exited (0)                               db".to_string(),
        ];
        let secs = detect_sections("docker ps -a", &lines).expect("detect");
        assert_eq!(secs.len(), 1);
        match &secs[0].kind {
            SectionKind::Table { columns, rows } => {
                assert_eq!(columns.len(), 7);
                assert_eq!(columns[0], "CONTAINER ID");
                assert_eq!(columns[4], "STATUS");
                assert_eq!(rows.len(), 2);
                // Valores con espacio simple se mantienen unidos.
                assert_eq!(rows[0][4], "Up 2 hours");
                assert_eq!(rows[0][6], "web");
                // Celda PORTS vacía en el segundo no descoloca NAMES.
                assert_eq!(rows[1][5], "");
                assert_eq!(rows[1][6], "db");
            }
            _ => panic!("esperaba Table"),
        }
    }

    #[test]
    fn git_status_corto_da_tabla_con_estado() {
        let lines = vec![
            "## main...origin/main".to_string(),
            " M src/foo.rs".to_string(),
            "A  src/bar.rs".to_string(),
            "?? nohup.out".to_string(),
        ];
        let secs = detect_sections("git status -s", &lines).expect("detect");
        assert_eq!(secs.len(), 1);
        match &secs[0].kind {
            SectionKind::Table { columns, rows } => {
                assert_eq!(columns, &["XY", "estado", "archivo"]);
                assert_eq!(rows.len(), 3); // la línea ## se omite
                assert_eq!(rows[0][2], "src/foo.rs");
                assert_eq!(rows[0][1], "modificado");
                assert!(rows[1][1].contains("staged"));
                assert_eq!(rows[2][1], "sin seguimiento");
            }
            _ => panic!("esperaba Table"),
        }
    }

    #[test]
    fn git_status_largo_se_parte_por_grupo() {
        let lines = vec![
            "On branch main".to_string(),
            "Your branch is up to date with 'origin/main'.".to_string(),
            "".to_string(),
            "Changes to be committed:".to_string(),
            "  (use \"git restore --staged <file>...\" to unstage)".to_string(),
            "\tmodified:   a.rs".to_string(),
            "".to_string(),
            "Untracked files:".to_string(),
            "  (use \"git add <file>...\" to include)".to_string(),
            "\tnohup.out".to_string(),
        ];
        let secs = detect_sections("git status", &lines).expect("detect");
        // rama + staged + sin seguimiento.
        assert_eq!(secs.len(), 3);
        assert_eq!(secs[0].title, "rama");
        assert_eq!(secs[1].title, "staged");
        assert_eq!(secs[1].as_lines_for_test().unwrap(), vec!["modified:   a.rs"]);
        assert_eq!(secs[2].title, "sin seguimiento");
        assert_eq!(secs[2].as_lines_for_test().unwrap(), vec!["nohup.out"]);
    }

    #[test]
    fn cargo_diagnosticos_una_seccion_por_error() {
        let lines = vec![
            "   Compiling shuma v0.1.0".to_string(),
            "error[E0308]: mismatched types".to_string(),
            "  --> src/foo.rs:3:5".to_string(),
            "warning: unused variable `x`".to_string(),
            "  --> src/bar.rs:9:9".to_string(),
            "error: could not compile `shuma`".to_string(),
        ];
        let secs = detect_sections("cargo build", &lines).expect("detect");
        // salida + 3 diagnósticos.
        assert_eq!(secs.len(), 4);
        assert_eq!(secs[0].title, "salida");
        assert!(secs[1].title.starts_with("error[E0308]"));
        assert_eq!(secs[1].as_lines_for_test().unwrap(), vec!["  --> src/foo.rs:3:5"]);
        assert!(secs[2].title.starts_with("warning"));
        assert!(secs[3].title.starts_with("error: could not compile"));
    }

    #[test]
    fn cargo_sin_diagnosticos_no_secciona() {
        let lines = vec!["Hello, world!".to_string(), "   Finished in 0.1s".to_string()];
        assert!(detect_sections("cargo run", &lines).is_none());
    }

    impl Section {
        fn as_lines_for_test(&self) -> Option<Vec<String>> {
            self.kind.as_lines().map(|v| v.to_vec())
        }
    }
}
