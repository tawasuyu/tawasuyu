//! Predicados sobre las palabras clave de COBOL. COBOL no termina los
//! statements con un símbolo: una sentencia es una secuencia de
//! statements que se delimitan por la aparición del siguiente verbo.
//! Estos predicados le dicen al parser dónde corta un statement.

/// ¿Es `w` un verbo que inicia un statement? (En mayúsculas.)
pub(crate) fn is_verb(w: &str) -> bool {
    matches!(
        w,
        "ACCEPT"
            | "ADD"
            | "ALTER"
            | "CALL"
            | "CANCEL"
            | "CLOSE"
            | "COMPUTE"
            | "CONTINUE"
            | "DELETE"
            | "DISPLAY"
            | "DIVIDE"
            | "EVALUATE"
            | "EXIT"
            | "GO"
            | "GOBACK"
            | "IF"
            | "INITIALIZE"
            | "INSPECT"
            | "MERGE"
            | "MOVE"
            | "MULTIPLY"
            | "OPEN"
            | "PERFORM"
            | "READ"
            | "RELEASE"
            | "RETURN"
            | "REWRITE"
            | "SEARCH"
            | "SET"
            | "SORT"
            | "START"
            | "STOP"
            | "STRING"
            | "SUBTRACT"
            | "UNSTRING"
            | "WRITE"
    )
}

/// ¿Es `w` un terminador de ámbito (`END-IF`, `ELSE`, `WHEN`...)?
pub(crate) fn is_terminator(w: &str) -> bool {
    matches!(
        w,
        "ELSE"
            | "WHEN"
            | "END-IF"
            | "END-PERFORM"
            | "END-EVALUATE"
            | "END-ADD"
            | "END-SUBTRACT"
            | "END-MULTIPLY"
            | "END-DIVIDE"
            | "END-COMPUTE"
            | "END-READ"
            | "END-WRITE"
            | "END-CALL"
            | "END-STRING"
            | "END-UNSTRING"
            | "END-SEARCH"
            | "END-START"
            | "END-DELETE"
            | "END-REWRITE"
            | "END-RETURN"
    )
}

/// ¿Es `w` una palabra de conexión de una cláusula (`TO`, `GIVING`...)?
fn is_connector(w: &str) -> bool {
    matches!(
        w,
        "TO" | "FROM"
            | "GIVING"
            | "BY"
            | "INTO"
            | "THRU"
            | "THROUGH"
            | "UNTIL"
            | "TIMES"
            | "ROUNDED"
            | "THEN"
            | "WITH"
            | "UPON"
            | "REMAINDER"
            | "VARYING"
    )
}

/// ¿Marca `w` el final de una lista de operandos o de nombres? Es así
/// para todo verbo, terminador o conector — ninguno puede ser un dato.
pub(crate) fn is_boundary(w: &str) -> bool {
    is_verb(w) || is_terminator(w) || is_connector(w)
}
