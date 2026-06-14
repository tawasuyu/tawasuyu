use super::*;

/// Copia el bloque entero al clipboard: el comando (`$ …`) seguido de su
/// salida completa (stdout y stderr en orden). Es el "copiar comando + salida"
/// estilo terminal moderna; lo dispara el botón ⧉ del header del bloque en la
/// superficie de terminal. No-op si el bloque no tiene ni comando ni cuerpo.
///
/// (Lo que queda de la vieja maquinaria del cuerpo IDE per-comando: el resto
/// —selección por-card, doble-click, menú legacy— se borró en la Fase 5 del
/// SDD-TERMINAL cuando la superficie virtualizada pasó a ser la única vía. La
/// selección/copia/menú del stream ahora viven en `update/surface.rs`.)
pub(crate) fn copy_command_block(s: &State, block: u64) {
    let mut partes: Vec<String> = Vec::new();
    if let Some(cmd) = s.block_command.get(&block) {
        // `block_command` guarda el texto ya con el prefijo "$ ".
        partes.push(cmd.clone());
    }
    partes.extend(body_lines_for_block(s, block));
    if partes.is_empty() {
        return;
    }
    set_clipboard(&partes.join("\n"));
}

/// Rango `[start, end)` (en columnas/chars) de la palabra en `line_text`
/// que contiene la columna `col` — alfanumérico + `_`, igual que el
/// text-editor. Si `col` cae sobre un no-word-char, devuelve un rango
/// vacío en `col` (no selecciona). Lo usa el doble-click de la superficie
/// de terminal (`update/surface.rs`) para seleccionar la palabra.
pub(crate) fn word_range_at(line_text: &str, col: usize) -> (usize, usize) {
    let chars: Vec<char> = line_text.chars().collect();
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    if col >= chars.len() || !is_word(chars[col]) {
        // Permití también el caso "el cursor quedó justo después de la
        // última letra de la palabra" (col == len o sobre separador): mirá
        // el char anterior.
        if col > 0 && col <= chars.len() && is_word(chars[col - 1]) {
            let mut start = col;
            while start > 0 && is_word(chars[start - 1]) {
                start -= 1;
            }
            return (start, col);
        }
        return (col, col);
    }
    let mut start = col;
    while start > 0 && is_word(chars[start - 1]) {
        start -= 1;
    }
    let mut end = col;
    while end < chars.len() && is_word(chars[end]) {
        end += 1;
    }
    (start, end)
}
