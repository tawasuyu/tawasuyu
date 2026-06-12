use super::*;

/// Aplica un Tab:
/// - popup abierto: cicla al siguiente candidato (no toca el texto, así el
///   rango de reemplazo del `Completion` guardado sigue válido).
/// - popup cerrado: 0 candidatos → nada; 1 → lo inserta directo; ≥2 → abre
///   el popup con el primero resaltado (sin tocar el texto todavía).
pub(crate) fn apply_completion_msg(mut s: State) -> State {
    if let Some(comp) = &s.completion {
        let n = comp.candidates.len();
        if n > 0 {
            s.completion_index = (s.completion_index + 1) % n;
        }
        return s;
    }
    let comp = s.input.complete(s.completion_source.as_ref());
    if comp.is_empty() {
        return s;
    }
    if comp.candidates.len() == 1 {
        let candidate = comp.candidates[0].clone();
        s.input.apply_completion(&comp, &candidate);
        return s;
    }
    s.completion = Some(comp);
    s.completion_index = 0;
    s
}

/// Cierra el popup de completado sin aplicar nada.
pub(crate) fn close_completion(s: &mut State) {
    s.completion = None;
    s.completion_index = 0;
}

/// Refresca el popup de completado **en vivo** (as-you-type): lo abre cuando
/// hay un prefijo a completar y candidatos, lo cierra si no. Rankea los
/// comandos por uso (frecuencia en el historial) — los más usados primero.
pub(crate) fn refresh_completion(s: &mut State) {
    let mut comp = s.input.complete(s.completion_source.as_ref());
    if comp.candidates.is_empty() || comp.replace_end <= comp.replace_start {
        s.completion = None;
        s.completion_index = 0;
        return;
    }
    rank_completion_by_usage(s, &mut comp);
    s.completion = Some(comp);
    s.completion_index = 0;
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

/// Cicla el candidato resaltado del popup (`delta` ±1, con wrap). No-op si
/// el popup está cerrado.
pub(crate) fn cycle_completion(mut s: State, delta: i32) -> State {
    if let Some(comp) = &s.completion {
        let n = comp.candidates.len() as i32;
        if n > 0 {
            s.completion_index = (s.completion_index as i32 + delta).rem_euclid(n) as usize;
        }
    }
    s
}

/// Acepta el candidato resaltado del popup, lo inserta y cierra el popup.
pub(crate) fn accept_completion(mut s: State) -> State {
    if let Some(comp) = s.completion.take() {
        if let Some(candidate) = comp.candidates.get(s.completion_index) {
            s.input.apply_completion(&comp, candidate);
        }
    }
    s.completion_index = 0;
    s
}
