use super::*;

use crate::types::{SugKind, Suggestion};

/// Cuántas líneas completas del historial (tier 2) ofrecer como máximo.
const MAX_LINE_SUGGESTIONS: usize = 5;
/// Cuántos grupos / coreografías (tier 3) ofrecer como máximo.
const MAX_GROUP_SUGGESTIONS: usize = 4;
/// Ventana reciente del historial a mirar para las líneas completas — acota el
/// costo por keystroke (igual espíritu que el corpus del ghost).
const LINE_SUGGEST_WINDOW: usize = 2000;

/// Total de filas navegables del popup (tier 1 + tiers 2/3).
pub(crate) fn completion_total(s: &State) -> usize {
    s.completion.as_ref().map(|c| c.candidates.len()).unwrap_or(0) + s.completion_extra.len()
}

/// Aplica un Tab:
/// - popup abierto: cicla al siguiente candidato (no toca el texto, así el
///   rango de reemplazo del `Completion` guardado sigue válido).
/// - popup cerrado: lo abre con el completado **en capas** (tokens +
///   líneas completas + grupos). Si hay exactamente un candidato, lo inserta
///   directo; con ≥2, abre el popup con el primero resaltado.
pub(crate) fn apply_completion_msg(mut s: State) -> State {
    if s.completion.is_some() {
        return cycle_completion(s, 1);
    }
    // Tab fuerza la apertura del popup en capas aunque no haya token a
    // completar (para que las líneas/grupos aparezcan a pedido).
    if !populate_completion(&mut s, true) {
        return s;
    }
    if completion_total(&s) == 1 {
        return accept_completion(s);
    }
    s
}

/// Cierra el popup de completado sin aplicar nada.
pub(crate) fn close_completion(s: &mut State) {
    s.completion = None;
    s.completion_extra.clear();
    s.completion_index = 0;
}

/// Refresca el popup de completado **en vivo** (as-you-type): lo abre cuando
/// hay un prefijo de token a completar, anexando bajo los candidatos las
/// líneas completas del historial y los grupos que extienden lo tipeado. No
/// fuerza la apertura por líneas/grupos solos — eso es a pedido (Tab).
pub(crate) fn refresh_completion(s: &mut State) {
    populate_completion(s, false);
}

/// Núcleo compartido por `refresh_completion` (as-you-type) y
/// `apply_completion_msg` (Tab). Calcula el tier 1 (tokens) y, si abre, los
/// tiers 2/3 (líneas/grupos). `force` abre el popup aunque el tier 1 esté
/// vacío (Tab). Devuelve `true` si quedó un popup abierto.
fn populate_completion(s: &mut State, force: bool) -> bool {
    let mut comp = s.input.complete(s.completion_source.as_ref());
    let has_token = !comp.candidates.is_empty() && comp.replace_end > comp.replace_start;
    if has_token {
        rank_completion_by_usage(s, &mut comp);
    } else {
        comp.candidates.clear();
    }
    // El tier 1 manda la apertura as-you-type; con `force` (Tab) basta que
    // haya algo en cualquier tier.
    let extra = if has_token || force {
        build_extra_suggestions(s)
    } else {
        Vec::new()
    };
    if comp.candidates.is_empty() && extra.is_empty() {
        close_completion(s);
        return false;
    }
    s.completion = Some(comp);
    s.completion_extra = extra;
    s.completion_index = 0;
    true
}

/// Tiers 2 y 3 del completado en capas: líneas completas del historial que
/// extienden lo tipeado, y grupos / coreografías cuya secuencia empieza con el
/// texto. Vacío si el cursor no está al final o el texto está vacío (no hay
/// prefijo de línea que extender).
pub(crate) fn build_extra_suggestions(s: &State) -> Vec<Suggestion> {
    let text = s.input.text();
    if text.is_empty() || s.input.cursor() != text.len() {
        return Vec::new();
    }
    let mut out: Vec<Suggestion> = Vec::new();
    let mut seen: std::collections::HashSet<String> = std::collections::HashSet::new();
    seen.insert(text.to_string());
    let span = (0usize, text.len());

    // ── Tier 3 — grupos guardados + coreografías emergentes ──────────────
    // (Van primero: un grupo entero es la sugerencia de mayor "altura".)
    let mut groups: Vec<Suggestion> = Vec::new();
    for g in &s.groups {
        let line = g.lines.join(" && ");
        let matches = g.name.starts_with(text)
            || line.starts_with(text)
            || g.lines.first().is_some_and(|l| l.starts_with(text));
        if matches && line != text && seen.insert(line.clone()) {
            groups.push(Suggestion {
                display: format!("⊞ {} · {} comando{}", g.name, g.lines.len(),
                    if g.lines.len() == 1 { "" } else { "s" }),
                insert: line,
                replace_start: span.0,
                replace_end: span.1,
                kind: SugKind::Group,
            });
        }
    }
    for (name, line, occ) in applicable_sequences(s) {
        if groups.len() >= MAX_GROUP_SUGGESTIONS {
            break;
        }
        let matches = name.starts_with(text)
            || line.starts_with(text)
            || line.split(" && ").next().is_some_and(|l| l.starts_with(text));
        if matches && line != text && seen.insert(line.clone()) {
            groups.push(Suggestion {
                display: format!("↻ {name} · ×{occ}"),
                insert: line,
                replace_start: span.0,
                replace_end: span.1,
                kind: SugKind::Group,
            });
        }
    }
    groups.truncate(MAX_GROUP_SUGGESTIONS);
    out.extend(groups);

    // ── Tier 2 — líneas completas del historial ──────────────────────────
    // Local al cwd antes que global, y dentro de cada tramo lo más reciente
    // primero (mismo orden de prioridad que el ghost).
    if let Ok(history) = s.history.lock() {
        let base = s.cwd.to_string_lossy();
        let entries = history.entries();
        let recent = &entries[entries.len().saturating_sub(LINE_SUGGEST_WINDOW)..];
        let mut local: Vec<Suggestion> = Vec::new();
        let mut global: Vec<Suggestion> = Vec::new();
        for e in recent.iter().rev() {
            if local.len() + global.len() >= MAX_LINE_SUGGESTIONS * 3 {
                break; // tope de escaneo; el truncate final acota la salida
            }
            if e.line.len() <= text.len() || !e.line.starts_with(text) {
                continue;
            }
            if !seen.insert(e.line.clone()) {
                continue;
            }
            let sug = Suggestion {
                display: format!("↪ {}", e.line),
                insert: e.line.clone(),
                replace_start: span.0,
                replace_end: span.1,
                kind: SugKind::Line,
            };
            if crate::update::cwd_within(&e.cwd, &base) {
                local.push(sug);
            } else {
                global.push(sug);
            }
        }
        local.extend(global);
        local.truncate(MAX_LINE_SUGGESTIONS);
        out.extend(local);
    }
    out
}

/// Reordena los candidatos de comando por frecuencia de uso en el historial
/// (desc), con desempate alfabético — "ordenado por prioridad y uso". Sólo
/// aplica a completados de comando; paths/flags quedan como vienen.
pub(crate) fn rank_completion_by_usage(s: &State, comp: &mut shuma_line::Completion) {
    if comp.kind != shuma_line::CompletionKind::Command {
        return;
    }
    let mut freq: std::collections::HashMap<String, usize> = std::collections::HashMap::new();
    if let Ok(h) = s.history.lock() {
        for e in h.entries() {
            if let Some(w) = e.line.split_whitespace().next() {
                *freq.entry(w.to_string()).or_insert(0) += 1;
            }
        }
    }
    comp.candidates.sort_by(|a, b| {
        let fa = freq.get(a).copied().unwrap_or(0);
        let fb = freq.get(b).copied().unwrap_or(0);
        fb.cmp(&fa).then_with(|| a.cmp(b))
    });
}

/// Cicla el candidato resaltado del popup (`delta` ±1, con wrap) sobre el total
/// en capas. No-op si el popup está cerrado.
pub(crate) fn cycle_completion(mut s: State, delta: i32) -> State {
    let n = completion_total(&s) as i32;
    if s.completion.is_some() && n > 0 {
        s.completion_index = (s.completion_index as i32 + delta).rem_euclid(n) as usize;
    }
    s
}

/// Acepta el candidato resaltado del popup, lo inserta y cierra el popup. El
/// índice global elige entre un candidato de token (tier 1) o una sugerencia
/// de línea/grupo (tiers 2/3), cada una con su propio rango de reemplazo.
pub(crate) fn accept_completion(mut s: State) -> State {
    let n_tok = s.completion.as_ref().map(|c| c.candidates.len()).unwrap_or(0);
    let idx = s.completion_index;
    if idx < n_tok {
        if let Some(comp) = s.completion.take() {
            if let Some(candidate) = comp.candidates.get(idx) {
                s.input.apply_completion(&comp, candidate);
            }
        }
    } else if let Some(sug) = s.completion_extra.get(idx - n_tok).cloned() {
        // Reusa la maquinaria de reemplazo de `LineState` armando un
        // `Completion` sintético con el rango propio de la sugerencia.
        let synthetic = shuma_line::Completion {
            kind: shuma_line::CompletionKind::Command,
            candidates: vec![sug.insert.clone()],
            replace_start: sug.replace_start,
            replace_end: sug.replace_end,
        };
        s.input.apply_completion(&synthetic, &sug.insert);
    }
    close_completion(&mut s);
    s
}
