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
        "git" if tokens.get(1) == Some(&"diff") => detect_diff(lines),
        "git" if tokens.get(1) == Some(&"log") => detect_git_log(lines),
        "diff" => detect_diff(lines),
        "env" | "printenv" => detect_env(lines),
        "cargo" | "rustc" => detect_cargo(lines),
        // `ip addr`/`ip link` (e `ifconfig`): un volcado denso de interfaces.
        // Lo partimos en una sección por interfaz (como `git status` largo).
        // `ip route`/`ip neigh` NO aplican (no son por-interfaz) → quedan fuera.
        "ip" if matches!(
            tokens.get(1).copied(),
            Some("addr" | "a" | "address" | "link" | "l")
        ) =>
        {
            detect_net_interfaces(lines)
        }
        "ifconfig" => detect_net_interfaces(lines),
        "mount" => detect_mount(lines),
        "du" => detect_du(lines),
        // `free` no va por `header_table`: su columna de etiqueta (`Mem:`,
        // `Swap:`) no tiene header, así que se perdería. Parser dedicado.
        "free" => detect_free(lines),
        // Comandos cuya salida es una tabla con header alineado a ancho fijo:
        // `docker ps`, `podman ps`, `kubectl get`, `systemctl list-units`,
        // `ps aux`, `df -h`, `lsblk`… Las columnas se cortan por la posición
        // de inicio de cada header (left-aligned, padded al ancho de la
        // columna). El detector es seguro: si no ve un header tabular,
        // devuelve `None` y el bloque cae al render plano.
        "docker" | "podman" | "kubectl" | "systemctl" | "ps" | "df" | "lsblk" | "ss"
        | "netstat" => header_table(lines)
            .map(|(columns, rows)| {
                vec![Section { title: String::new(), kind: SectionKind::Table { columns, rows } }]
            })
            // `docker inspect`/`kubectl … -o json` emiten JSON, no tabla: si el
            // header_table no enganchó, probamos pretty-print de JSON.
            .or_else(|| detect_json(lines)),
        // Fallback por contenido: si la salida es un blob JSON compacto
        // (típico de `docker inspect`, `kubectl -o json`, una API), lo
        // pretty-printeamos para que sea legible. Gated y barato (ver guardas).
        _ => detect_json(lines),
    }
}

/// Tope de bytes para intentar parsear JSON (corre por frame en el render —
/// acotamos el costo). Más grande que esto cae al plano.
const JSON_MAX_BYTES: usize = 64 * 1024;

/// Detecta un **blob JSON compacto** y lo pretty-printea como sección `json`.
/// Sólo se mete cuando aporta: salida que arranca con `{`/`[`, ≤4 líneas no
/// vacías (ya multilínea = ya formateada, se respeta), bajo `JSON_MAX_BYTES`,
/// y que parsea a objeto/array. `None` en cualquier otro caso (cae al plano).
fn detect_json(lines: &[String]) -> Option<Vec<Section>> {
    let no_vacias = lines.iter().filter(|l| !l.trim().is_empty()).count();
    if no_vacias == 0 || no_vacias > 4 {
        return None; // vacío, o ya viene en varias líneas (formateado)
    }
    let joined = lines.join("\n");
    let t = joined.trim();
    if t.len() > JSON_MAX_BYTES || !(t.starts_with('{') || t.starts_with('[')) {
        return None;
    }
    let val: serde_json::Value = serde_json::from_str(t).ok()?;
    // Sólo contenedores (un escalar `"hola"` o `42` no gana nada con esto).
    if !val.is_object() && !val.is_array() {
        return None;
    }
    let pretty = serde_json::to_string_pretty(&val).ok()?;
    let body: Vec<String> = pretty.lines().map(String::from).collect();
    Some(vec![Section { title: "json".into(), kind: SectionKind::Lines(body) }])
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

/// `git diff` / `diff` unificado: una sección colapsable por archivo. Corta
/// en cada `diff --git a/… b/…` (preferido) o, si no hay, en cada par
/// `--- …`. El título es el path del archivo. `None` si no se ve estructura.
fn detect_diff(lines: &[String]) -> Option<Vec<Section>> {
    let starts_file = |l: &str| l.starts_with("diff --git ") || l.starts_with("diff -");
    let has_git_headers = lines.iter().any(|l| starts_file(l));
    let mut sections: Vec<Section> = Vec::new();
    let mut preamble: Vec<String> = Vec::new();
    let mut cur: Option<(String, Vec<String>)> = None;
    let flush = |cur: &mut Option<(String, Vec<String>)>, out: &mut Vec<Section>| {
        if let Some((title, body)) = cur.take() {
            out.push(Section { title, kind: SectionKind::Lines(body) });
        }
    };
    for l in lines {
        // Header de archivo: `diff --git a/x b/x` → título = el path b/.
        let is_header = if has_git_headers {
            starts_file(l)
        } else {
            // Sin `diff --git`: cortamos en `+++ b/path` (segunda mitad del par).
            l.starts_with("+++ ")
        };
        if is_header {
            flush(&mut cur, &mut sections);
            let title = diff_title(l);
            cur = Some((title, vec![l.clone()]));
        } else if let Some((_, body)) = cur.as_mut() {
            body.push(l.clone());
        } else {
            preamble.push(l.clone());
        }
    }
    flush(&mut cur, &mut sections);
    if sections.is_empty() {
        return None;
    }
    let pre: Vec<String> = preamble.into_iter().filter(|l| !l.trim().is_empty()).collect();
    if !pre.is_empty() {
        sections.insert(0, Section { title: "resumen".into(), kind: SectionKind::Lines(pre) });
    }
    Some(sections)
}

/// Extrae el path de un header de diff (`diff --git a/x b/x` o `+++ b/x`).
fn diff_title(line: &str) -> String {
    if let Some(rest) = line.strip_prefix("diff --git ") {
        // `a/path b/path` → preferimos el lado b (destino).
        if let Some((_, b)) = rest.split_once(" b/") {
            return b.trim().to_string();
        }
        return rest.trim().to_string();
    }
    if let Some(rest) = line.strip_prefix("+++ ") {
        return rest.trim().trim_start_matches("b/").to_string();
    }
    line.trim().to_string()
}

/// `git log` (formato completo): una sección colapsable por commit (corta en
/// cada línea `commit <hash>`). En formato `--oneline` (`<hash> asunto`) cae a
/// una tabla `hash · asunto`. `None` si no se ve ninguno de los dos.
fn detect_git_log(lines: &[String]) -> Option<Vec<Section>> {
    let first = lines.iter().find(|l| !l.trim().is_empty())?;
    if first.starts_with("commit ") {
        // Formato completo: secciones por commit.
        let mut sections: Vec<Section> = Vec::new();
        let mut cur: Option<(String, Vec<String>)> = None;
        for l in lines {
            if l.starts_with("commit ") {
                if let Some((title, body)) = cur.take() {
                    sections.push(Section { title, kind: SectionKind::Lines(body) });
                }
                let short = l.strip_prefix("commit ").unwrap_or("").trim();
                let short = short.get(..short.len().min(10)).unwrap_or(short);
                cur = Some((format!("commit {short}"), vec![l.clone()]));
            } else if let Some((_, body)) = cur.as_mut() {
                body.push(l.clone());
            }
        }
        if let Some((title, body)) = cur.take() {
            sections.push(Section { title, kind: SectionKind::Lines(body) });
        }
        return if sections.is_empty() { None } else { Some(sections) };
    }
    // Formato --oneline: `<hash> asunto`. Tabla hash · asunto.
    let mut rows: Vec<Vec<String>> = Vec::new();
    for l in lines {
        let l = l.trim();
        if l.is_empty() {
            continue;
        }
        let Some((hash, subject)) = l.split_once(char::is_whitespace) else {
            return None;
        };
        let is_hash = (7..=40).contains(&hash.len())
            && hash.chars().all(|c| c.is_ascii_hexdigit());
        if !is_hash {
            return None;
        }
        rows.push(vec![hash.to_string(), subject.trim().to_string()]);
    }
    if rows.is_empty() {
        return None;
    }
    Some(vec![Section {
        title: String::new(),
        kind: SectionKind::Table { columns: vec!["hash".into(), "asunto".into()], rows },
    }])
}

/// `env` / `printenv`: tabla `variable · valor` partiendo cada línea por el
/// primer `=`. Las líneas de continuación (valores multilínea) se ignoran.
fn detect_env(lines: &[String]) -> Option<Vec<Section>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut no_vacias = 0usize;
    for l in lines {
        if l.trim().is_empty() {
            continue;
        }
        no_vacias += 1;
        let Some((key, val)) = l.split_once('=') else {
            continue;
        };
        // Una clave de env válida no tiene espacios — filtra basura.
        if key.is_empty() || key.contains(char::is_whitespace) {
            continue;
        }
        rows.push(vec![key.to_string(), val.to_string()]);
    }
    // Guarda contra falsos positivos (`env FOO=bar cmd` cuya salida no es
    // KEY=VAL): exigimos que la mayoría de las líneas sean asignaciones.
    if rows.is_empty() || rows.len() * 5 < no_vacias * 3 {
        return None;
    }
    Some(vec![Section {
        title: String::new(),
        kind: SectionKind::Table { columns: vec!["variable".into(), "valor".into()], rows },
    }])
}

/// Extrae el nombre de interfaz del encabezado de un bloque de `ip addr`
/// (`1: eth0: <…>`) o `ifconfig` (`eth0: flags=…`). Cae al primer token si
/// el formato no matchea.
fn iface_name(line: &str) -> String {
    let t = line.trim_end();
    // `ip addr`: "N: name: <flags> …" — el primer campo es el índice numérico.
    if let Some((head, rest)) = t.split_once(": ") {
        if !head.is_empty() && head.chars().all(|c| c.is_ascii_digit()) {
            return rest.split(':').next().unwrap_or(rest).trim().to_string();
        }
    }
    // `ifconfig`: "name: flags=…" o "name  Link encap:…" — el nombre va antes
    // del primer ':' o espacio.
    t.split([':', ' ']).next().unwrap_or(t).trim().to_string()
}

/// `ip addr`/`ip link`/`ifconfig`: un volcado plano de varias interfaces.
/// Cada encabezado **no indentado** abre una interfaz; sus líneas indentadas
/// (link/inet/inet6/…) son su cuerpo. Devuelve una sección por interfaz, con
/// el nombre como título navegable. `None` si no hay ≥2 interfaces (entonces
/// no vale la pena estructurar — cae al render plano).
fn detect_net_interfaces(lines: &[String]) -> Option<Vec<Section>> {
    let mut sections: Vec<Section> = Vec::new();
    let mut cur: Option<(String, Vec<String>)> = None;
    let flush = |cur: &mut Option<(String, Vec<String>)>, out: &mut Vec<Section>| {
        if let Some((title, body)) = cur.take() {
            out.push(Section { title, kind: SectionKind::Lines(body) });
        }
    };
    for line in lines {
        if line.trim().is_empty() {
            continue; // las blancas separan bloques; no aportan al cuerpo
        }
        let indented = line.starts_with(char::is_whitespace);
        if !indented {
            // Encabezado de interfaz nueva.
            flush(&mut cur, &mut sections);
            cur = Some((iface_name(line), vec![line.clone()]));
        } else if let Some((_, body)) = cur.as_mut() {
            body.push(line.clone());
        } else {
            // Línea indentada antes de cualquier encabezado: preámbulo suelto.
            cur = Some((String::new(), vec![line.clone()]));
        }
    }
    flush(&mut cur, &mut sections);
    (sections.len() >= 2).then_some(sections)
}

/// `mount`: cada línea es `DISPOSITIVO on MONTAJE type FS (opciones)`. Lo
/// volcamos a una tabla ordenable (dispositivo · montaje · tipo · opciones).
/// Las líneas que no matchean el patrón se saltan; `None` si ninguna lo hace.
fn detect_mount(lines: &[String]) -> Option<Vec<Section>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    for l in lines {
        let t = l.trim();
        if t.is_empty() {
            continue;
        }
        let Some((dev, rest)) = t.split_once(" on ") else {
            continue;
        };
        let Some((mnt, rest2)) = rest.split_once(" type ") else {
            continue;
        };
        // `rest2` = "ext4 (rw,relatime)" → tipo + opciones entre paréntesis.
        let (fstype, opts) = match rest2.split_once(" (") {
            Some((fs, o)) => (fs.trim(), o.trim_end_matches(')')),
            None => (rest2.trim(), ""),
        };
        rows.push(vec![
            dev.to_string(),
            mnt.to_string(),
            fstype.to_string(),
            opts.to_string(),
        ]);
    }
    if rows.is_empty() {
        return None;
    }
    Some(vec![Section {
        title: String::new(),
        kind: SectionKind::Table {
            columns: vec!["dispositivo".into(), "montaje".into(), "tipo".into(), "opciones".into()],
            rows,
        },
    }])
}

/// `du`/`du -h`: cada línea es `TAMAÑO<sep>RUTA`. Tabla ordenable (tamaño ·
/// ruta) — ordenar por tamaño es el caso de uso. El tamaño debe empezar con
/// dígito (filtra ruido); `None` si la mayoría de las líneas no son `du`.
fn detect_du(lines: &[String]) -> Option<Vec<Section>> {
    let mut rows: Vec<Vec<String>> = Vec::new();
    let mut no_vacias = 0usize;
    for l in lines {
        let t = l.trim_end();
        if t.trim().is_empty() {
            continue;
        }
        no_vacias += 1;
        let mut it = t.split_whitespace();
        let Some(size) = it.next() else { continue };
        // Un tamaño de `du` empieza con dígito (`4.0K`, `4096`, `1.2G`).
        if !size.starts_with(|c: char| c.is_ascii_digit()) {
            continue;
        }
        let path = it.collect::<Vec<_>>().join(" ");
        if path.is_empty() {
            continue;
        }
        rows.push(vec![size.to_string(), path]);
    }
    // Guarda contra falsos positivos: la mayoría de las líneas deben matchear.
    if rows.is_empty() || rows.len() * 5 < no_vacias * 3 {
        return None;
    }
    Some(vec![Section {
        title: String::new(),
        kind: SectionKind::Table {
            columns: vec!["tamaño".into(), "ruta".into()],
            rows,
        },
    }])
}

/// `free`/`free -h`: el header (`total used free shared buff/cache available`)
/// no tiene columna para las etiquetas de fila (`Mem:`, `Swap:`), así que
/// `header_table` las perdería. Acá prependeamos una columna vacía y leemos
/// cada fila como `etiqueta + valores`. `None` si el header no parece de `free`.
fn detect_free(lines: &[String]) -> Option<Vec<Section>> {
    let header_idx = lines.iter().position(|l| !l.trim().is_empty())?;
    let header_cols: Vec<String> = lines[header_idx].split_whitespace().map(String::from).collect();
    // Confianza: el header de `free` lleva `total` y `used`.
    if !header_cols.iter().any(|c| c == "total") || !header_cols.iter().any(|c| c == "used") {
        return None;
    }
    let mut columns = vec![String::new()]; // columna de la etiqueta de fila
    columns.extend(header_cols);
    let mut rows: Vec<Vec<String>> = Vec::new();
    for l in &lines[header_idx + 1..] {
        if l.trim().is_empty() {
            continue;
        }
        let mut cells: Vec<String> = l.split_whitespace().map(String::from).collect();
        if cells.is_empty() {
            continue;
        }
        // Pad/trunca al ancho de columnas (Swap: trae menos campos que Mem:).
        cells.resize(columns.len(), String::new());
        rows.push(cells);
    }
    if rows.is_empty() {
        return None;
    }
    Some(vec![Section {
        title: String::new(),
        kind: SectionKind::Table { columns, rows },
    }])
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
    fn ip_addr_se_parte_por_interfaz() {
        let lines = vec![
            "1: lo: <LOOPBACK,UP,LOWER_UP> mtu 65536 qdisc noqueue state UNKNOWN".to_string(),
            "    link/loopback 00:00:00:00:00:00 brd 00:00:00:00:00:00".to_string(),
            "    inet 127.0.0.1/8 scope host lo".to_string(),
            "2: eth0: <BROADCAST,MULTICAST,UP,LOWER_UP> mtu 1500 qdisc fq state UP".to_string(),
            "    link/ether aa:bb:cc:dd:ee:ff brd ff:ff:ff:ff:ff:ff".to_string(),
            "    inet 192.168.1.10/24 brd 192.168.1.255 scope global eth0".to_string(),
        ];
        let secs = detect_sections("ip addr", &lines).expect("detect");
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[0].title, "lo");
        assert_eq!(secs[1].title, "eth0");
        // El cuerpo conserva encabezado + líneas indentadas de la interfaz.
        let body0 = secs[0].as_lines_for_test().unwrap();
        assert_eq!(body0.len(), 3);
        assert!(body0[2].contains("127.0.0.1/8"));
    }

    #[test]
    fn ifconfig_se_parte_por_interfaz() {
        let lines = vec![
            "eth0: flags=4163<UP,BROADCAST,RUNNING,MULTICAST>  mtu 1500".to_string(),
            "        inet 192.168.1.10  netmask 255.255.255.0".to_string(),
            "        ether aa:bb:cc:dd:ee:ff  txqueuelen 1000".to_string(),
            "lo: flags=73<UP,LOOPBACK,RUNNING>  mtu 65536".to_string(),
            "        inet 127.0.0.1  netmask 255.0.0.0".to_string(),
        ];
        let secs = detect_sections("ifconfig", &lines).expect("detect");
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[0].title, "eth0");
        assert_eq!(secs[1].title, "lo");
    }

    #[test]
    fn ip_route_no_se_estructura() {
        // `ip route` no es por-interfaz → el detector ni se invoca.
        let lines = vec![
            "default via 192.168.1.1 dev eth0".to_string(),
            "192.168.1.0/24 dev eth0 proto kernel scope link".to_string(),
        ];
        assert!(detect_sections("ip route", &lines).is_none());
    }

    #[test]
    fn una_sola_interfaz_no_vale_la_pena() {
        let lines = vec![
            "1: lo: <LOOPBACK,UP> mtu 65536".to_string(),
            "    inet 127.0.0.1/8 scope host lo".to_string(),
        ];
        assert!(detect_sections("ip addr", &lines).is_none());
    }

    #[test]
    fn ss_se_lee_como_tabla() {
        let lines = vec![
            "Netid  State   Recv-Q  Send-Q  Local-Address  Peer-Address".to_string(),
            "tcp    LISTEN  0       128     0.0.0.0:22     0.0.0.0:*".to_string(),
            "tcp    ESTAB   0       0       10.0.0.1:22    10.0.0.5:5051".to_string(),
        ];
        let secs = detect_sections("ss -tn", &lines).expect("detect");
        match &secs[0].kind {
            SectionKind::Table { columns, rows } => {
                assert!(columns.len() >= 4);
                assert_eq!(rows.len(), 2);
            }
            _ => panic!("esperaba Table"),
        }
    }

    #[test]
    fn json_compacto_se_pretty_printea() {
        let lines = vec![r#"{"name":"web","ports":[80,443],"tls":true}"#.to_string()];
        let secs = detect_sections("docker inspect web", &lines).expect("detect");
        assert_eq!(secs[0].title, "json");
        let body = secs[0].as_lines_for_test().unwrap();
        // Pretty-print: varias líneas indentadas.
        assert!(body.len() > 3);
        assert!(body.iter().any(|l| l.contains("\"name\": \"web\"")));
    }

    #[test]
    fn json_array_tambien() {
        let lines = vec![r#"[{"a":1},{"a":2}]"#.to_string()];
        let secs = detect_sections("curl -s http://x/api", &lines).expect("detect");
        assert_eq!(secs[0].title, "json");
    }

    #[test]
    fn json_no_estructurado_o_escalar_cae_al_plano() {
        // No-JSON.
        assert!(detect_sections("echo hola", &[String::from("hola mundo")]).is_none());
        // Escalar JSON: no gana nada.
        assert!(detect_sections("echo", &[String::from("42")]).is_none());
        // Ya multilínea (formateado): se respeta, no se re-envuelve.
        let pretty = vec![
            "{".to_string(),
            "  \"a\": 1,".to_string(),
            "  \"b\": 2,".to_string(),
            "  \"c\": 3,".to_string(),
            "  \"d\": 4".to_string(),
            "}".to_string(),
        ];
        assert!(detect_sections("cat x.json", &pretty).is_none());
    }

    #[test]
    fn mount_se_lee_como_tabla() {
        let lines = vec![
            "/dev/sda1 on / type ext4 (rw,relatime)".to_string(),
            "proc on /proc type proc (rw,nosuid,nodev,noexec)".to_string(),
            "tmpfs on /run type tmpfs (rw,nosuid,nodev)".to_string(),
        ];
        let secs = detect_sections("mount", &lines).expect("detect");
        match &secs[0].kind {
            SectionKind::Table { columns, rows } => {
                assert_eq!(columns.len(), 4);
                assert_eq!(rows.len(), 3);
                assert_eq!(rows[0], vec!["/dev/sda1", "/", "ext4", "rw,relatime"]);
            }
            _ => panic!("esperaba Table"),
        }
    }

    #[test]
    fn du_se_lee_como_tabla_tamaño_ruta() {
        let lines = vec![
            "4.0K\t./a".to_string(),
            "12K\t./carpeta con espacios".to_string(),
            "16K\t.".to_string(),
        ];
        let secs = detect_sections("du -h", &lines).expect("detect");
        match &secs[0].kind {
            SectionKind::Table { columns, rows } => {
                assert_eq!(columns, &vec!["tamaño".to_string(), "ruta".to_string()]);
                assert_eq!(rows.len(), 3);
                assert_eq!(rows[1], vec!["12K", "./carpeta con espacios"]);
            }
            _ => panic!("esperaba Table"),
        }
    }

    #[test]
    fn du_descarta_si_no_parece_du() {
        // Salida que no es `du` (no empieza con tamaño) → cae al plano.
        let lines = vec![
            "hola mundo".to_string(),
            "esto no es du".to_string(),
        ];
        assert!(detect_sections("du", &lines).is_none());
    }

    #[test]
    fn free_conserva_las_etiquetas_de_fila() {
        let lines = vec![
            "              total        used        free      shared  buff/cache   available".to_string(),
            "Mem:           15Gi       8.0Gi       2.0Gi       500Mi       5.0Gi       6.0Gi".to_string(),
            "Swap:         8.0Gi       1.0Gi       7.0Gi".to_string(),
        ];
        let secs = detect_sections("free -h", &lines).expect("detect");
        match &secs[0].kind {
            SectionKind::Table { columns, rows } => {
                // Columna 0 = etiqueta (sin header) + las 6 de free.
                assert_eq!(columns.len(), 7);
                assert_eq!(columns[1], "total");
                assert_eq!(rows[0][0], "Mem:");
                assert_eq!(rows[0][1], "15Gi");
                // Swap trae menos campos → se rellena al ancho.
                assert_eq!(rows[1][0], "Swap:");
                assert_eq!(rows[1].len(), 7);
            }
            _ => panic!("esperaba Table"),
        }
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

    #[test]
    fn git_diff_una_seccion_por_archivo() {
        let lines = vec![
            "diff --git a/src/foo.rs b/src/foo.rs".to_string(),
            "index 111..222 100644".to_string(),
            "@@ -1,3 +1,4 @@".to_string(),
            "+nueva línea".to_string(),
            "diff --git a/README.md b/README.md".to_string(),
            "@@ -10,2 +10,2 @@".to_string(),
            "-vieja".to_string(),
            "+nueva".to_string(),
        ];
        let secs = detect_sections("git diff", &lines).expect("detect");
        assert_eq!(secs.len(), 2);
        assert_eq!(secs[0].title, "src/foo.rs");
        assert_eq!(secs[1].title, "README.md");
    }

    #[test]
    fn git_log_oneline_es_tabla_y_full_secciones() {
        let oneline = vec![
            "a1b2c3d arregla el parser".to_string(),
            "9f8e7d6 agrega tests".to_string(),
        ];
        let secs = detect_sections("git log --oneline", &oneline).expect("detect");
        match &secs[0].kind {
            SectionKind::Table { columns, rows } => {
                assert_eq!(columns, &["hash", "asunto"]);
                assert_eq!(rows.len(), 2);
                assert_eq!(rows[0][1], "arregla el parser");
            }
            _ => panic!("oneline esperaba tabla"),
        }
        let full = vec![
            "commit a1b2c3d4e5f6a7b8".to_string(),
            "Author: Sergio <x@y>".to_string(),
            "    arregla el parser".to_string(),
            "commit 0011223344556677".to_string(),
            "    otro commit".to_string(),
        ];
        let secs = detect_sections("git log", &full).expect("detect");
        assert_eq!(secs.len(), 2);
        assert!(secs[0].title.starts_with("commit a1b2c3d4"));
    }

    #[test]
    fn env_es_tabla_y_no_secciona_salida_libre() {
        let lines = vec![
            "PATH=/usr/bin:/bin".to_string(),
            "HOME=/home/u".to_string(),
            "SHELL=/bin/zsh".to_string(),
        ];
        let secs = detect_sections("env", &lines).expect("detect");
        match &secs[0].kind {
            SectionKind::Table { columns, rows } => {
                assert_eq!(columns, &["variable", "valor"]);
                assert_eq!(rows.len(), 3);
                assert_eq!(rows[0], vec!["PATH", "/usr/bin:/bin"]);
            }
            _ => panic!("esperaba tabla"),
        }
        // `env FOO=bar prog` cuya salida es texto libre → no se secciona.
        let libre = vec![
            "FOO=bar".to_string(),
            "esto es salida del programa".to_string(),
            "otra línea cualquiera de log".to_string(),
            "y otra más".to_string(),
        ];
        assert!(detect_sections("env", &libre).is_none());
    }

    impl Section {
        fn as_lines_for_test(&self) -> Option<Vec<String>> {
            self.kind.as_lines().map(|v| v.to_vec())
        }
    }
}
