use super::*;

/// Rama de git activa para `cwd` — `None` si no estamos en un repo (o si
/// HEAD está detached). Implementación minimalista por archivo: sube por
/// los padres buscando `.git`, lee `HEAD` y extrae `refs/heads/<rama>`. No
/// usa libgit2 ni lanza procesos (barato de llamar por frame).
pub(crate) fn git_branch(cwd: &std::path::Path) -> Option<String> {
    let mut dir = cwd.to_path_buf();
    let git_dir = loop {
        let candidate = dir.join(".git");
        if candidate.exists() {
            break candidate;
        }
        if !dir.pop() {
            return None;
        }
    };
    // `.git` puede ser un archivo (worktrees/submódulos) con `gitdir: …`,
    // o un directorio con `HEAD` dentro.
    let head_path = if git_dir.is_file() {
        let s = std::fs::read_to_string(&git_dir).ok()?;
        let target = s.strip_prefix("gitdir:")?.trim();
        std::path::PathBuf::from(target).join("HEAD")
    } else {
        git_dir.join("HEAD")
    };
    let head = std::fs::read_to_string(head_path).ok()?;
    head.trim()
        .strip_prefix("ref: refs/heads/")
        .map(|b| b.to_string())
}

/// Marcadores de proyecto: archivos/dirs que identifican la "forma" de un
/// directorio. Gatean la predicción por estructura (no sugerir `cargo` sin
/// `Cargo.toml`).
pub(crate) const PROJECT_MARKERS: &[&str] = &[
    ".git",
    "Cargo.toml",
    "package.json",
    "go.mod",
    "Makefile",
    "pyproject.toml",
    "pom.xml",
    "build.gradle",
];

/// Marcadores de proyecto presentes en `dir`.
fn markers_in(dir: &str) -> Vec<String> {
    let base = std::path::Path::new(dir);
    PROJECT_MARKERS
        .iter()
        .filter(|m| base.join(m).exists())
        .map(|m| m.to_string())
        .collect()
}

/// Construye los `CommandRecord` de `shuma-infer` a partir del historial
/// (éxito = exit 0).
fn infer_records(s: &State) -> Vec<shuma_infer::CommandRecord> {
    let Ok(history) = s.history.lock() else {
        return Vec::new();
    };
    history
        .entries()
        .iter()
        // El historial Llimphi aún no graba el exit (siempre `None`):
        // tratamos lo desconocido como éxito para no descartar todo el
        // corpus. Si más adelante se registra el exit, los fallos
        // (`Some(c!=0)`) quedan excluidos automáticamente.
        .map(|e| {
            let ok = e.exit.map_or(true, |c| c == 0);
            shuma_infer::CommandRecord::parse(&e.line, e.cwd.clone(), ok)
        })
        .collect()
}

/// Recalcula los patrones emergentes del historial y los cachea en el
/// state. Se llama al cerrar cada comando (cuando el historial creció).
pub(crate) fn refresh_patterns(s: &mut State) {
    let records = infer_records(s);
    s.patterns = shuma_infer::detect_patterns(&records, &shuma_infer::InferConfig::default());
}

/// Condición de disparo de un patrón: los marcadores de proyecto comunes a
/// todos los directorios donde corrió.
fn pattern_trigger(p: &shuma_infer::EmergingPattern) -> Vec<String> {
    let mut dirs = p.directories.iter();
    let Some(first) = dirs.next() else {
        return Vec::new();
    };
    let mut common = markers_in(first);
    for d in dirs {
        let here = markers_in(d);
        common.retain(|m| here.contains(m));
    }
    common
}

/// Umbral de ocurrencias para ofrecer una coreografía como grupo (A1): la
/// hiciste al menos esto seguido para que valga la pena guardarla.
pub(crate) const CHOREO_OFFER_THRESHOLD: usize = 3;

/// A1 — la coreografía que vale la pena ofrecer como grupo: el patrón de
/// mayor score (los `patterns` vienen ordenados desc) con ≥ [`CHOREO_OFFER_THRESHOLD`]
/// ocurrencias que el usuario no descartó y que todavía no está guardado como
/// grupo (mismas líneas). `None` si no hay ninguno. El shell propone, el
/// usuario acepta con un click o ignora.
pub(crate) fn choreography_suggestion(s: &State) -> Option<&shuma_infer::EmergingPattern> {
    s.patterns.iter().find(|p| {
        p.occurrences >= CHOREO_OFFER_THRESHOLD
            && !s.dismissed_choreo.contains(&p.signature)
            && !s.groups.iter().any(|g| g.lines == p.example)
    })
}

/// A1 — promueve una coreografía emergente a grupo ejecutable: busca el patrón
/// por su `signature`, lo guarda como [`CommandGroup`] con su nombre sugerido y
/// las líneas reales de la última ocurrencia (`example`), y lo marca como
/// descartado para no re-ofrecerlo. Reemplaza un grupo homónimo si existe. La
/// firma queda en `dismissed_choreo` también para que el chip no reaparezca el
/// frame intermedio antes del próximo `refresh_patterns`.
pub(crate) fn accept_choreography(mut s: State, signature: &[String]) -> State {
    let Some(p) = s.patterns.iter().find(|p| p.signature == signature) else {
        return s;
    };
    let name = p.suggested_name();
    let lines = p.example.clone();
    let n = lines.len();
    if let Some(g) = s.groups.iter_mut().find(|g| g.name == name) {
        g.lines = lines;
    } else {
        s.groups.push(CommandGroup { name: name.clone(), lines });
    }
    let fkey = s
        .groups
        .iter()
        .position(|g| g.name == name)
        .map(|i| i + 1)
        .unwrap_or(0);
    s.dismissed_choreo.insert(signature.to_vec());
    s.push_output(OutputLine::notice(format!(
        "✔ coreografía «{name}» guardada como grupo ({n} comandos) — F{fkey} la ejecuta"
    )));
    s
}

/// La secuencia que el motor predice como continuación de la sesión, si la
/// hay y el cwd comparte la forma del patrón.
pub(crate) fn predicted_sequence(s: &State) -> Option<String> {
    if s.patterns.is_empty() {
        return None;
    }
    let records = infer_records(s);
    let tail = &records[records.len().saturating_sub(6)..];
    let (pi, next) = shuma_infer::predict_next(tail, &s.patterns)?;
    if next.is_empty() {
        return None;
    }
    // Disparo por estructura: no anticipar un patrón en un directorio que
    // no comparte su forma (no sugerir `cargo` sin `Cargo.toml`).
    let trigger = pattern_trigger(&s.patterns[pi]);
    if !trigger.is_empty() {
        let here = markers_in(&s.cwd.to_string_lossy());
        if !trigger.iter().all(|m| here.contains(m)) {
            return None;
        }
    }
    Some(next.join(" && "))
}

/// Sugerencia "ghost" para la línea actual — la secuencia predicha por el
/// motor de patrones (si aplica) y, tras ella, el prefijo histórico más
/// reciente que extiende el texto que ya está tipeado.
pub(crate) fn current_ghost(s: &State) -> Option<String> {
    let text = s.input.text();
    if text.is_empty() || s.input.cursor() != text.len() {
        return None;
    }
    // Corpus por prioridad: secuencia predicha primero, luego historial.
    let mut corpus: Vec<String> = Vec::new();
    if let Some(seq) = predicted_sequence(s) {
        corpus.push(seq);
    }
    if let Ok(history) = s.history.lock() {
        corpus.extend(history.entries().iter().rev().map(|e| e.line.clone()));
    }
    shuma_line::ghost_suggestion(text, &corpus)
}

#[cfg(test)]
mod a1_choreo_tests {
    use super::*;

    /// Construye un State con la coreografía `git pull → cargo build → cargo
    /// test` repetida 3 veces, **separada por comandos distintos** (como en el
    /// uso real) para que las ventanas largas solapadas no subsuman el patrón
    /// base. Queda ya inferida en `s.patterns`.
    fn state_con_patron() -> State {
        let mut s = State::new(shuma_module::Source::Local);
        let rec = |l: &str| shuma_infer::CommandRecord::parse(l, "/repo", true);
        let records = vec![
            rec("git pull"),
            rec("cargo build"),
            rec("cargo test"),
            rec("ls"), // separador 1
            rec("git pull"),
            rec("cargo build"),
            rec("cargo test"),
            rec("cd /tmp"), // separador 2 (distinto → corta ventanas largas)
            rec("git pull"),
            rec("cargo build"),
            rec("cargo test"),
        ];
        s.patterns =
            shuma_infer::detect_patterns(&records, &shuma_infer::InferConfig::default());
        s
    }

    #[test]
    fn ofrece_coreografia_sobre_umbral() {
        let s = state_con_patron();
        let sug = choreography_suggestion(&s).expect("hay sugerencia");
        assert!(sug.occurrences >= CHOREO_OFFER_THRESHOLD);
        assert_eq!(sug.suggested_name(), "git+cargo+cargo");
    }

    #[test]
    fn aceptar_crea_grupo_y_calla_la_oferta() {
        let mut s = state_con_patron();
        let sig = choreography_suggestion(&s).unwrap().signature.clone();
        s = accept_choreography(s, &sig);
        // Quedó un grupo con las líneas reales de la última ocurrencia.
        assert_eq!(s.groups.len(), 1);
        assert_eq!(
            s.groups[0].lines,
            vec!["git pull", "cargo build", "cargo test"]
        );
        // Y ya no se vuelve a ofrecer (guardado + descartado).
        assert!(choreography_suggestion(&s).is_none());
    }

    #[test]
    fn descartar_calla_la_oferta() {
        let mut s = state_con_patron();
        let sig = choreography_suggestion(&s).unwrap().signature.clone();
        s.dismissed_choreo.insert(sig);
        assert!(choreography_suggestion(&s).is_none());
    }
}
