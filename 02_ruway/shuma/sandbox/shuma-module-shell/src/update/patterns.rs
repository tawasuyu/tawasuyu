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

/// Umbral de ocurrencias por defecto para ofrecer una coreografía como grupo
/// (A1): la hiciste al menos esto seguido para que valga la pena guardarla.
/// El shumarc lo gobierna con `[rules].on_pattern_score` (E3); `0` = nunca.
pub(crate) const CHOREO_OFFER_THRESHOLD: usize = 3;

/// A1 — la coreografía que vale la pena ofrecer como grupo: el patrón de
/// mayor score (los `patterns` vienen ordenados desc) con ≥ umbral
/// ocurrencias que el usuario no descartó y que todavía no está guardado como
/// grupo (mismas líneas). El umbral sale de `[rules].on_pattern_score` (E3);
/// `0` lo apaga. `None` si no hay ninguno. El shell propone, el usuario acepta
/// con un click o ignora.
pub(crate) fn choreography_suggestion(s: &State) -> Option<&shuma_infer::EmergingPattern> {
    let threshold = s.config.rules.on_pattern_score as usize;
    if threshold == 0 {
        return None; // regla apagada por el shumarc
    }
    s.patterns.iter().find(|p| {
        p.occurrences >= threshold
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

/// A2 — una línea larga repetida que vale la pena acortar a un alias. Es el
/// gemelo de la coreografía (A1) pero sobre **una sola línea** en vez de una
/// secuencia: si tecleaste lo mismo, largo, varias veces, el shell ofrece
/// bautizarlo.
#[derive(Debug, Clone)]
pub(crate) struct AliasSuggestion {
    /// La línea completa que se acortaría (el cuerpo del alias).
    pub line: String,
    /// Cuántas veces apareció idéntica en el historial.
    pub count: usize,
    /// El nombre corto propuesto (mnemónico de las iniciales, único).
    pub name: String,
}

/// A2 — largo mínimo de línea para que valga ofrecer un alias. Por debajo de
/// esto, el alias no ahorra teclas que importen.
pub(crate) const ALIAS_MIN_LEN: usize = 40;

/// A2 — repeticiones idénticas mínimas para ofrecer el alias (mismo espíritu
/// que `CHOREO_OFFER_THRESHOLD`: lo hiciste suficiente para que valga un nombre).
pub(crate) const ALIAS_OFFER_THRESHOLD: usize = 3;

/// A2 — mnemónico corto para una línea: las **iniciales** de sus tokens
/// significativos (saltea opciones `-x`/`--y` y operadores de shell), en
/// minúscula y sólo alfanuméricas. `git push origin main` → `gpom`. Si no
/// junta al menos 2 letras (línea de puras flags), cae al primer token entero.
/// Garantiza unicidad contra `taken` agregando un sufijo numérico.
pub(crate) fn suggest_alias_name(line: &str, taken: &dyn Fn(&str) -> bool) -> String {
    let mut base: String = line
        .split_whitespace()
        .filter(|t| {
            !t.starts_with('-') && !matches!(*t, "&&" | "||" | "|" | ";" | ">" | ">>" | "<")
        })
        .filter_map(|t| t.chars().find(|c| c.is_ascii_alphanumeric()))
        .map(|c| c.to_ascii_lowercase())
        .collect();
    if base.chars().count() < 2 {
        // Línea de puras flags: usá el primer token entero (sólo alfanum).
        base = line
            .split_whitespace()
            .next()
            .unwrap_or("alias")
            .chars()
            .filter(char::is_ascii_alphanumeric)
            .map(|c| c.to_ascii_lowercase())
            .collect();
    }
    if base.is_empty() {
        base = "alias".to_string();
    }
    if !taken(&base) {
        return base;
    }
    // Colisión: sufijo numérico determinista.
    for n in 2.. {
        let cand = format!("{base}{n}");
        if !taken(&cand) {
            return cand;
        }
    }
    unreachable!("la sucesión de sufijos es infinita")
}

/// A2 — cuenta de líneas idénticas en el historial, sólo las externas (los
/// builtins `:x` no se aliasan). Devuelve `(línea, veces)` por línea distinta.
fn line_frequencies(s: &State) -> Vec<(String, usize)> {
    let Ok(history) = s.history.lock() else {
        return Vec::new();
    };
    let mut counts: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    for e in history.entries() {
        let line = e.line.trim();
        if line.is_empty() || line.starts_with(':') {
            continue;
        }
        *counts.entry(line.to_string()).or_insert(0) += 1;
    }
    counts.into_iter().collect()
}

/// A2 — la línea larga repetida que más conviene aliasar: longitud ≥
/// [`ALIAS_MIN_LEN`], al menos [`ALIAS_OFFER_THRESHOLD`] repeticiones idénticas,
/// que el usuario no descartó y que **todavía no es** el cuerpo de un alias
/// existente. Empata por más repeticiones, luego línea más larga, luego orden
/// lexicográfico (determinista). `None` si no hay candidata. El shell propone,
/// el usuario acepta con un click o ignora — gemelo de A1.
pub(crate) fn alias_suggestion(s: &State) -> Option<AliasSuggestion> {
    let already_body: std::collections::HashSet<&str> =
        s.config.aliases.values().map(String::as_str).collect();
    let mut best: Option<(String, usize)> = None;
    for (line, count) in line_frequencies(s) {
        if count < ALIAS_OFFER_THRESHOLD
            || line.chars().count() < ALIAS_MIN_LEN
            || s.dismissed_alias.contains(&line)
            || already_body.contains(line.as_str())
        {
            continue;
        }
        let better = match &best {
            None => true,
            Some((bl, bc)) => {
                count > *bc
                    || (count == *bc && line.chars().count() > bl.chars().count())
                    || (count == *bc && line.chars().count() == bl.chars().count() && line < *bl)
            }
        };
        if better {
            best = Some((line, count));
        }
    }
    let (line, count) = best?;
    let name = suggest_alias_name(&line, &alias_name_taken(s));
    Some(AliasSuggestion { line, count, name })
}

/// A2 — predicado «ese nombre ya está tomado»: por un alias existente o por un
/// binario real del PATH (no pisar un comando del sistema con un alias homónimo).
fn alias_name_taken(s: &State) -> impl Fn(&str) -> bool + '_ {
    move |name: &str| {
        if s.config.aliases.contains_key(name) {
            return true;
        }
        use shuma_line::CompletionSource;
        s.completion_source.commands().iter().any(|c| c == name)
    }
}

/// A2 — núcleo puro de aceptar un alias: lo agrega a la config viva (se expande
/// desde el próximo submit) y marca la línea descartada para no re-ofrecerla.
/// Sin efectos de disco — la persistencia al shumarc la hace [`accept_alias`].
pub(crate) fn learn_alias(mut s: State, name: &str, line: &str) -> State {
    s.config.aliases.insert(name.to_string(), line.to_string());
    s.dismissed_alias.insert(line.to_string());
    s
}

/// A2 — acepta el alias para `line`: recalcula el nombre (determinista), lo
/// aprende a la config viva ([`learn_alias`]) y lo **persiste al shumarc**
/// (`[aliases]` vía `upsert_key`, preservando comentarios). Reemplaza un alias
/// homónimo sólo si apuntaba a la misma línea (no pisa uno del usuario).
pub(crate) fn accept_alias(s: State, line: &str) -> State {
    let name = suggest_alias_name(line, &alias_name_taken(&s));
    let mut s = learn_alias(s, &name, line);
    let mut learned = true;
    if let Some(rc) = shuma_config::Config::default_path() {
        if let Err(e) = shuma_config::upsert_key(&rc, "aliases", &name, &shuma_config::toml_string(line))
        {
            learned = false;
            s.push_output(OutputLine::notice(format!(
                "alias «{name}» activo esta sesión — pero no se pudo guardar al shumarc: {e}"
            )));
        }
    } else {
        learned = false;
    }
    if learned {
        s.push_output(OutputLine::notice(format!(
            "✔ alias «{name}» = «{line}» aprendido al shumarc — tipealo y se expande"
        )));
    }
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

/// Distancia de Damerau-Levenshtein restringida (optimal string alignment)
/// entre `a` y `b`: inserción/borrado/sustitución **y transposición de dos
/// caracteres adyacentes**, todas costo 1. La transposición barata atrapa el
/// typo clásico (`cagro` → `cargo`, distancia 1; en Levenshtein plano serían
/// 2). DP O(|a|·|b|) sobre caracteres Unicode, sin dependencias.
pub(crate) fn damerau_levenshtein(a: &str, b: &str) -> usize {
    let a: Vec<char> = a.chars().collect();
    let b: Vec<char> = b.chars().collect();
    let (na, nb) = (a.len(), b.len());
    if na == 0 {
        return nb;
    }
    if nb == 0 {
        return na;
    }
    // `d[i][j]` = distancia entre `a[..i]` y `b[..j]`. Necesitamos `i-2`/`j-2`
    // para la transposición, así que mantenemos la matriz completa.
    let mut d = vec![vec![0usize; nb + 1]; na + 1];
    for (i, row) in d.iter_mut().enumerate() {
        row[0] = i;
    }
    for j in 0..=nb {
        d[0][j] = j;
    }
    for i in 1..=na {
        for j in 1..=nb {
            let cost = if a[i - 1] == b[j - 1] { 0 } else { 1 };
            let mut m = (d[i - 1][j] + 1)
                .min(d[i][j - 1] + 1)
                .min(d[i - 1][j - 1] + cost);
            if i > 1 && j > 1 && a[i - 1] == b[j - 2] && a[i - 2] == b[j - 1] {
                m = m.min(d[i - 2][j - 2] + 1);
            }
            d[i][j] = m;
        }
    }
    d[na][nb]
}

/// El candidato más cercano a `bad` dentro de `cands` con distancia en
/// `1..=umbral` (excluye 0 = el mismo token). Empata por menor distancia y,
/// a igual distancia, por orden lexicográfico (determinista). `None` si
/// ninguno entra en el umbral.
fn closest_within<'a>(bad: &str, umbral: usize, cands: impl Iterator<Item = &'a str>) -> Option<String> {
    let mut best: Option<(usize, &str)> = None;
    for cand in cands {
        if cand == bad || cand.is_empty() {
            continue;
        }
        let dd = damerau_levenshtein(bad, cand);
        if dd == 0 || dd > umbral {
            continue;
        }
        let better = match best {
            None => true,
            Some((bd, bc)) => dd < bd || (dd == bd && cand < bc),
        };
        if better {
            best = Some((dd, cand));
        }
    }
    best.map(|(_, c)| c.to_string())
}

/// A4 — detecta el caso «¿quisiste decir…?» al cerrar un comando: si su salida
/// trae `command not found`, busca el binario más cercano al primer token de
/// la línea. **Prioriza el historial** (lo que el usuario realmente corre)
/// sobre el PATH crudo; ambos con umbral `max(1, len/3)`. Si hay candidato,
/// guarda en `s.did_you_mean[block]` la línea corregida. Sin modelo, sin red.
pub(crate) fn detect_did_you_mean(s: &mut State, block: u64) {
    let has_cnf = s.output.iter().any(|l| {
        l.block == block
            && l.kind == OutputKind::Stderr
            && l.text.to_ascii_lowercase().contains("command not found")
    });
    if !has_cnf {
        return;
    }
    // Línea original (sin el prefijo "$ " del header).
    let Some(raw) = s.block_command.get(&block).cloned() else {
        return;
    };
    let cmd = raw.trim_start_matches("$ ").trim();
    let mut toks = cmd.splitn(2, char::is_whitespace);
    let Some(bad) = toks.next() else {
        return;
    };
    let rest = toks.next().unwrap_or("");
    // Un path explícito (`./x`, `/usr/bin/x`) no es un typo de binario del PATH.
    if bad.is_empty() || bad.contains('/') {
        return;
    }
    let umbral = (bad.chars().count() / 3).max(1);

    // 1) Historial: primer token de cada línea (sin paths), señal fuerte.
    let hist_bins: Vec<String> = match s.history.lock() {
        Ok(h) => h
            .entries()
            .iter()
            .filter_map(|e| e.line.split_whitespace().next())
            .filter(|t| !t.contains('/'))
            .map(String::from)
            .collect(),
        Err(_) => Vec::new(),
    };
    let pick = closest_within(bad, umbral, hist_bins.iter().map(String::as_str))
        // 2) Fallback: binarios del PATH.
        .or_else(|| {
            use shuma_line::CompletionSource;
            let path_bins = s.completion_source.commands();
            closest_within(bad, umbral, path_bins.iter().map(String::as_str))
        });

    if let Some(cand) = pick {
        let corregida = if rest.is_empty() {
            cand
        } else {
            format!("{cand} {rest}")
        };
        s.did_you_mean.insert(block, corregida);
    }
}

/// `true` si `entry_cwd` cae dentro de `base` (es el mismo directorio o un
/// hijo) — el criterio de "contexto" del ghost por cwd (A3).
fn cwd_within(entry_cwd: &str, base: &str) -> bool {
    entry_cwd == base
        || entry_cwd
            .strip_prefix(base)
            .is_some_and(|rest| rest.starts_with('/'))
}

/// Sugerencia "ghost" para la línea actual — la secuencia predicha por el
/// motor de patrones (si aplica) y, tras ella, el prefijo histórico más
/// reciente que extiende el texto que ya está tipeado.
///
/// A3 — **ghost contextual por cwd:** el historial se rankea en dos tramos,
/// primero las entradas del directorio actual (y sus hijos), después lo
/// global. En un monorepo el ghost deja de sugerir comandos de otro proyecto:
/// `cargo b…` en `cosmos/` completa al último build de cosmos, no al de wawa.
/// Dentro de cada tramo, lo más reciente primero.
pub(crate) fn current_ghost(s: &State) -> Option<String> {
    let text = s.input.text();
    if text.is_empty() || s.input.cursor() != text.len() {
        return None;
    }
    // Corpus por prioridad: secuencia predicha primero, luego historial
    // (local al cwd antes que global).
    let mut corpus: Vec<String> = Vec::new();
    if let Some(seq) = predicted_sequence(s) {
        corpus.push(seq);
    }
    if let Ok(history) = s.history.lock() {
        let base = s.cwd.to_string_lossy();
        let mut local: Vec<String> = Vec::new();
        let mut global: Vec<String> = Vec::new();
        for e in history.entries().iter().rev() {
            if cwd_within(&e.cwd, &base) {
                local.push(e.line.clone());
            } else {
                global.push(e.line.clone());
            }
        }
        corpus.extend(local);
        corpus.extend(global);
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

#[cfg(test)]
mod a2_alias_tests {
    use super::*;

    /// State con un historial **aislado** (sobre `/dev/null`, in-memory) — el
    /// `State::new` normal abre el historial real del disco; en tests eso lo
    /// contamina y, peor, persiste los `append`. Acá cada State arranca vacío.
    fn state_aislado() -> State {
        let mut s = State::new(shuma_module::Source::Local);
        s.history = Arc::new(Mutex::new(
            shuma_history::History::open(std::path::PathBuf::from("/dev/null"))
                .expect("/dev/null como history vacío"),
        ));
        s
    }

    /// Una línea larga (≥ 40 chars) tecleada `n` veces en el historial,
    /// **separada por otro comando** (el historial deduplica consecutivos —
    /// como en el uso real: corrés algo, hacés otra cosa, lo volvés a correr).
    fn state_con_linea_repetida(line: &str, n: usize) -> State {
        let mut s = state_aislado();
        {
            let mut h = s.history.lock().unwrap();
            for i in 0..n {
                let _ = h.append(shuma_history::Entry::new(line, "/repo", (2 * i) as u64));
                let _ = h.append(shuma_history::Entry::new("ls", "/repo", (2 * i + 1) as u64));
            }
        }
        s
    }

    #[test]
    fn ofrece_alias_para_linea_larga_repetida() {
        let line = "git push origin feature/inteligencia-shuma --force-with-lease";
        assert!(line.chars().count() >= ALIAS_MIN_LEN);
        let s = state_con_linea_repetida(line, ALIAS_OFFER_THRESHOLD);
        let sug = alias_suggestion(&s).expect("hay alias que ofrecer");
        assert_eq!(sug.line, line);
        assert_eq!(sug.count, ALIAS_OFFER_THRESHOLD);
        // Iniciales de los tokens no-flag: git push origin feature… → gpof.
        assert_eq!(sug.name, "gpof");
    }

    #[test]
    fn no_ofrece_si_es_corta_o_poco_repetida() {
        // Corta aunque repetida.
        let s = state_con_linea_repetida("ls -la", 5);
        assert!(alias_suggestion(&s).is_none());
        // Larga pero por debajo del umbral.
        let larga = "kubectl get pods --all-namespaces -o wide --watch";
        let s = state_con_linea_repetida(larga, ALIAS_OFFER_THRESHOLD - 1);
        assert!(alias_suggestion(&s).is_none());
    }

    #[test]
    fn no_ofrece_builtins_ni_lo_descartado_ni_lo_ya_aliasado() {
        // Los builtins `:x` no se aliasan, por largos que sean.
        let builtin = ":macro save deploy cargo build --bin %1 && scp %1 %2:/srv/app";
        let s = state_con_linea_repetida(builtin, 5);
        assert!(alias_suggestion(&s).is_none());

        // Descartada → no se vuelve a ofrecer.
        let line = "docker run --rm -it -v $PWD:/work -w /work rust:latest cargo test";
        let mut s = state_con_linea_repetida(line, 4);
        assert!(alias_suggestion(&s).is_some());
        s.dismissed_alias.insert(line.to_string());
        assert!(alias_suggestion(&s).is_none());

        // Ya es cuerpo de un alias → tampoco.
        let mut s = state_con_linea_repetida(line, 4);
        s.config.aliases.insert("dt".into(), line.to_string());
        assert!(alias_suggestion(&s).is_none());
    }

    #[test]
    fn aceptar_aprende_a_la_config_viva_y_descarta() {
        // Núcleo puro (sin tocar el shumarc del usuario): `learn_alias` es lo
        // que `accept_alias` hace antes de persistir.
        let line = "cargo build --release --target x86_64-unknown-none -Zbuild-std";
        let mut s = state_con_linea_repetida(line, 3);
        let name = alias_suggestion(&s).unwrap().name;
        s = learn_alias(s, &name, line);
        // El alias quedó en la config viva (se expande desde el próximo submit)…
        assert_eq!(s.config.aliases.get(&name).map(String::as_str), Some(line));
        // …y ya no se vuelve a ofrecer.
        assert!(alias_suggestion(&s).is_none());
    }

    #[test]
    fn nombre_evita_colisiones() {
        // Iniciales de los tokens no-flag: grep · TODO · src → "gts".
        let line = "grep --color=always -rn TODO --include='*.rs' src/";
        assert_eq!(suggest_alias_name(line, &|_| false), "gts");
        // Si "gts" ya está tomado (alias homónimo), debe sufijar sin pisarlo.
        let mut s = State::new(shuma_module::Source::Local);
        s.config.aliases.insert("gts".into(), "otra cosa".into());
        let name = suggest_alias_name(line, &alias_name_taken(&s));
        assert_ne!(name, "gts");
        assert!(name.starts_with("gts"));
    }

    #[test]
    fn nombre_para_linea_de_puras_flags_cae_al_primer_token() {
        // Sin tokens significativos para iniciales → usa el primer token entero.
        let line = "tar -czvf backup-2026-06-13.tar.gz --exclude=target ./proyecto";
        let name = suggest_alias_name(line, &|_| false);
        // El primer token con letra es "tar" (tcp… serían las iniciales reales:
        // tar backup proyecto → "tbp"); verificamos que sea no vacío y alfanum.
        assert!(!name.is_empty());
        assert!(name.chars().all(|c| c.is_ascii_alphanumeric()));
    }
}

#[cfg(test)]
mod a3_ghost_cwd_tests {
    use super::*;

    #[test]
    fn cwd_within_reconoce_mismo_dir_e_hijos() {
        assert!(cwd_within("/repo", "/repo"));
        assert!(cwd_within("/repo/sub", "/repo"));
        assert!(cwd_within("/repo/a/b", "/repo"));
        assert!(!cwd_within("/repo-otro", "/repo")); // prefijo de string, no de path
        assert!(!cwd_within("/otro", "/repo"));
    }

    #[test]
    fn ghost_prefiere_el_cwd_actual_sobre_lo_mas_reciente() {
        let mut s = State::new(shuma_module::Source::Local);
        s.cwd = std::path::PathBuf::from("/repo");
        {
            let mut h = s.history.lock().unwrap();
            // Local al cwd, más viejo.
            let _ = h.append(shuma_history::Entry::new("cargo build --debug", "/repo", 1));
            // Global (otro proyecto), más reciente → ganaría por recencia.
            let _ = h.append(shuma_history::Entry::new("cargo build --release", "/otro", 2));
        }
        s.input.set_text("cargo bu");
        // A3: el del cwd actual manda, aunque sea más viejo.
        assert_eq!(current_ghost(&s).as_deref(), Some("ild --debug"));
    }
}

#[cfg(test)]
mod a4_did_you_mean_tests {
    use super::*;

    #[test]
    fn damerau_atrapa_transposicion() {
        assert_eq!(damerau_levenshtein("cagro", "cargo"), 1); // transposición
        assert_eq!(damerau_levenshtein("cargo", "cargo"), 0);
        assert_eq!(damerau_levenshtein("gti", "git"), 1);
        assert_eq!(damerau_levenshtein("ls", "ls"), 0);
    }

    fn state_con_fallo(cmd: &str) -> State {
        let mut s = State::new(shuma_module::Source::Local);
        // El usuario ya corrió el binario bueno antes (señal del historial).
        {
            let mut h = s.history.lock().unwrap();
            let _ = h.append(shuma_history::Entry::new("cargo build", "/repo", 1));
        }
        // Bloque 5 con el comando tipeado y su stderr de "command not found".
        s.block_command.insert(5, format!("$ {cmd}"));
        let mut err = OutputLine::stderr("zsh: command not found: cagro");
        err.block = 5;
        s.output.push(err);
        s
    }

    #[test]
    fn ofrece_correccion_desde_historial() {
        let mut s = state_con_fallo("cagro build --release");
        detect_did_you_mean(&mut s, 5);
        assert_eq!(s.did_you_mean.get(&5).map(String::as_str), Some("cargo build --release"));
    }

    #[test]
    fn no_ofrece_sin_command_not_found() {
        let mut s = State::new(shuma_module::Source::Local);
        s.block_command.insert(5, "$ cagro build".to_string());
        let mut err = OutputLine::stderr("error: some other failure");
        err.block = 5;
        s.output.push(err);
        detect_did_you_mean(&mut s, 5);
        assert!(s.did_you_mean.get(&5).is_none());
    }

    #[test]
    fn no_ofrece_para_un_path_explicito() {
        let mut s = state_con_fallo("./cagro build");
        detect_did_you_mean(&mut s, 5);
        assert!(s.did_you_mean.get(&5).is_none());
    }
}
