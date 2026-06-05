//! `chaka-parser` — parser de COBOL'85 (subconjunto) a AST.
//!
//! Segunda etapa del transpilador COBOL→Rust: consume los [`Token`] del
//! `chaka-lexer` y produce un [`Program`]. El alcance de la v1 es el
//! **esqueleto del programa**:
//!
//! - Las cuatro divisiones (`IDENTIFICATION`, `ENVIRONMENT`, `DATA`,
//!   `PROCEDURE`) y el `PROGRAM-ID`.
//! - La **DATA division** como un árbol de [`DataItem`]: número de
//!   nivel, nombre, cláusula `PICTURE` reensamblada y `VALUE`.
//! - La **PROCEDURE division** como una lista de [`Paragraph`], cada
//!   uno con sus [`Sentence`] (los tokens crudos de cada sentencia).
//!
//! Lo que **no** hace todavía: el parseo a nivel de statement (cada
//! sentencia queda como `Vec<Token>` — es trabajo de `chaka-ir`), la
//! ENVIRONMENT division, CICS, SQL embebido y los dialectos IBM. Son
//! las etapas siguientes del plan.
//!
//! El parser es tolerante: salta lo que no entiende (encabezados de
//! `SECTION`, entradas `FD`/`SD`, cláusulas de datos que no sean
//! `PICTURE`/`VALUE`) en vez de fallar. Sólo emite [`ParseError`] ante
//! un número de nivel inválido o un dato sin nombre.

#![forbid(unsafe_code)]

use thiserror::Error;

pub use chaka_lexer::{Token, TokenKind};

/// Un programa COBOL parseado: el esqueleto de sus cuatro divisiones.
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct Program {
    /// El `PROGRAM-ID` de la IDENTIFICATION division, si está presente.
    pub program_id: Option<String>,
    /// Los ítems raíz de la DATA division (cada `01`/`77` con su árbol).
    pub data: Vec<DataItem>,
    /// Los ficheros declarados (`SELECT` + `FD`).
    pub files: Vec<FileEntry>,
    /// Los párrafos de la PROCEDURE division, en orden de aparición.
    pub paragraphs: Vec<Paragraph>,
}

/// La organización física de un fichero (`ORGANIZATION`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum FileOrg {
    /// `LINE SEQUENTIAL` — cada registro es una línea de texto. Es el
    /// default histórico de chaka y el único soportado antes de las
    /// organizaciones por clave.
    #[default]
    LineSequential,
    /// `SEQUENTIAL` — registros de longitud fija, acceso secuencial.
    /// chaka lo trata como `LINE SEQUENTIAL` (una línea por registro).
    Sequential,
    /// `INDEXED` — registros con `RECORD KEY`; acceso por clave o
    /// secuencial en orden de clave.
    Indexed,
    /// `RELATIVE` — registros numerados; la `RELATIVE KEY` es el número
    /// de registro (1-based).
    Relative,
}

/// El modo de acceso declarado (`ACCESS MODE`).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub enum AccessMode {
    /// `SEQUENTIAL` — sólo lectura/escritura en orden.
    #[default]
    Sequential,
    /// `RANDOM` — sólo acceso directo por clave.
    Random,
    /// `DYNAMIC` — mezcla acceso directo y secuencial (`READ NEXT`).
    Dynamic,
}

/// Un fichero declarado: su nombre lógico, la ruta a la que se asigna
/// (`ASSIGN TO`), el dato de registro asociado (el `01` bajo su `FD`) y
/// las cláusulas de organización del `SELECT` (`ORGANIZATION`, `ACCESS
/// MODE`, `RECORD KEY`, `RELATIVE KEY`).
#[derive(Debug, Clone, PartialEq, Eq, Default, serde::Serialize, serde::Deserialize)]
pub struct FileEntry {
    pub name: String,
    pub path: String,
    pub record: String,
    /// Organización física (`LINE SEQUENTIAL` por defecto).
    pub organization: FileOrg,
    /// Modo de acceso (`SEQUENTIAL` por defecto).
    pub access: AccessMode,
    /// Nombre COBOL del campo `RECORD KEY` (sólo `INDEXED`), en mayúsculas.
    pub record_key: Option<String>,
    /// Nombre COBOL del campo `RELATIVE KEY` (sólo `RELATIVE`), en mayúsculas.
    pub relative_key: Option<String>,
}

/// Un ítem de datos de la DATA division: un número de nivel, un nombre
/// y, opcionalmente, las cláusulas `PICTURE` y `VALUE`. Los ítems de
/// mayor nivel numérico cuelgan como `children` del que los contiene.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct DataItem {
    /// Número de nivel: 01-49 (jerárquico), 66 (`RENAMES`), 77
    /// (elemental independiente) u 88 (nombre de condición).
    pub level: u8,
    /// Nombre del dato, normalizado a mayúsculas (`FILLER` incluido).
    pub name: String,
    /// Cláusula `PICTURE` reensamblada (`S9(5)V99`), si la hay.
    pub picture: Option<String>,
    /// Cláusula `VALUE`: literal numérico (con signo), constante
    /// figurativa en mayúsculas, o literal de texto entre comillas.
    pub value: Option<String>,
    /// Cláusula `OCCURS n [TIMES]`: el dato es una tabla de `n`
    /// elementos. `None` si es un dato escalar.
    pub occurs: Option<u32>,
    /// Ítems subordinados (de nivel numérico mayor).
    pub children: Vec<DataItem>,
}

/// Un párrafo de la PROCEDURE division. El párrafo implícito que
/// agrupa las sentencias previas al primer encabezado tiene `name` "".
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Paragraph {
    /// Nombre del párrafo en mayúsculas; "" para el párrafo implícito.
    pub name: String,
    /// Sentencias del párrafo, en orden.
    pub sentences: Vec<Sentence>,
}

/// Una sentencia: los tokens entre dos puntos terminadores. La v1 no
/// parsea a nivel de statement — eso es trabajo de `chaka-ir`.
#[derive(Debug, Clone, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub struct Sentence {
    pub tokens: Vec<Token>,
}

/// Error de parseo, con la línea del token donde se detectó.
#[derive(Debug, Clone, PartialEq, Eq, Error)]
pub enum ParseError {
    #[error("línea {line}: número de nivel inválido {found:?} (esperado 01-49, 66, 77 u 88)")]
    BadLevel { line: u32, found: String },
    #[error("línea {line}: se esperaba el nombre de un dato tras el número de nivel")]
    ExpectedDataName { line: u32 },
}

/// Las cuatro divisiones de un programa COBOL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DivKind {
    Identification,
    Environment,
    Data,
    Procedure,
}

/// Parsea una secuencia de tokens COBOL en un [`Program`].
pub fn parse(tokens: &[Token]) -> Result<Program, ParseError> {
    let divs = find_divisions(tokens);
    let mut program = Program::default();
    for (idx, &(kind, start)) in divs.iter().enumerate() {
        // El cuerpo de la división va desde el punto que cierra su
        // encabezado hasta el comienzo de la división siguiente.
        let lo = match period_after(tokens, start) {
            Some(p) => p + 1,
            None => tokens.len(),
        };
        let next = divs.get(idx + 1).map(|&(_, s)| s).unwrap_or(tokens.len());
        let body = &tokens[lo..next.max(lo)];
        match kind {
            DivKind::Identification => parse_identification(body, &mut program),
            DivKind::Environment => parse_environment(body, &mut program),
            DivKind::Data => {
                let (data, fd_records) = parse_data(body)?;
                program.data = data;
                // Asocia cada `FD` con el registro `01` que le sigue.
                for (fd, record) in fd_records {
                    if let Some(f) = program.files.iter_mut().find(|f| f.name == fd) {
                        f.record = record;
                    }
                }
            }
            DivKind::Procedure => program.paragraphs = parse_procedure(body),
        }
    }
    Ok(program)
}

/// Localiza los encabezados de división (`X DIVISION`) y devuelve, en
/// orden, su clase y el índice del token de la palabra de división.
fn find_divisions(tokens: &[Token]) -> Vec<(DivKind, usize)> {
    let mut divs = Vec::new();
    let mut i = 0;
    while i + 1 < tokens.len() {
        if kw(tokens.get(i + 1)).as_deref() == Some("DIVISION") {
            let kind = match kw(tokens.get(i)).as_deref() {
                Some("IDENTIFICATION") | Some("ID") => Some(DivKind::Identification),
                Some("ENVIRONMENT") => Some(DivKind::Environment),
                Some("DATA") => Some(DivKind::Data),
                Some("PROCEDURE") => Some(DivKind::Procedure),
                _ => None,
            };
            if let Some(k) = kind {
                divs.push((k, i));
                i += 2;
                continue;
            }
        }
        i += 1;
    }
    divs
}

/// Extrae el `PROGRAM-ID` del cuerpo de la IDENTIFICATION division.
fn parse_identification(body: &[Token], program: &mut Program) {
    for (i, t) in body.iter().enumerate() {
        if kw(Some(t)).as_deref() == Some("PROGRAM-ID") {
            // `PROGRAM-ID. NAME.` — el punto tras `PROGRAM-ID` es opcional.
            let mut j = i + 1;
            if body.get(j).map(|t| t.kind) == Some(TokenKind::Period) {
                j += 1;
            }
            if let Some(name) = body.get(j) {
                program.program_id = Some(match name.kind {
                    TokenKind::Word => name.text.to_uppercase(),
                    TokenKind::String => name.text.clone(),
                    _ => continue,
                });
            }
            return;
        }
    }
}

/// Parsea el cuerpo de la DATA division en un árbol de [`DataItem`].
/// Parsea la cláusula `FILE-CONTROL` de la ENVIRONMENT division: cada
/// `SELECT name ASSIGN TO "ruta"` se registra como un fichero.
fn parse_environment(body: &[Token], program: &mut Program) {
    for sent in split_sentences(body) {
        if kw(sent.first()).as_deref() != Some("SELECT") {
            continue;
        }
        let Some(name_tok) = sent.get(1) else {
            continue;
        };
        if name_tok.kind != TokenKind::Word {
            continue;
        }
        let mut path = String::new();
        let mut organization = FileOrg::default();
        let mut access = AccessMode::default();
        let mut record_key = None;
        let mut relative_key = None;
        let mut i = 2;
        while i < sent.len() {
            match kw(sent.get(i)).as_deref() {
                Some("ASSIGN") => {
                    i += 1;
                    if kw(sent.get(i)).as_deref() == Some("TO") {
                        i += 1;
                    }
                    if let Some(t) = sent.get(i) {
                        path = t.text.clone();
                    }
                }
                Some("ORGANIZATION") => {
                    i += 1;
                    if kw(sent.get(i)).as_deref() == Some("IS") {
                        i += 1;
                    }
                    organization = match kw(sent.get(i)).as_deref() {
                        Some("INDEXED") => FileOrg::Indexed,
                        Some("RELATIVE") => FileOrg::Relative,
                        Some("SEQUENTIAL") => FileOrg::Sequential,
                        // `LINE [SEQUENTIAL]`: la palabra `SEQUENTIAL` la
                        // consume la próxima vuelta del bucle, sin efecto.
                        Some("LINE") => FileOrg::LineSequential,
                        _ => organization,
                    };
                }
                Some("ACCESS") => {
                    i += 1;
                    if kw(sent.get(i)).as_deref() == Some("MODE") {
                        i += 1;
                    }
                    if kw(sent.get(i)).as_deref() == Some("IS") {
                        i += 1;
                    }
                    access = match kw(sent.get(i)).as_deref() {
                        Some("RANDOM") => AccessMode::Random,
                        Some("DYNAMIC") => AccessMode::Dynamic,
                        Some("SEQUENTIAL") => AccessMode::Sequential,
                        _ => access,
                    };
                }
                // `RECORD KEY [IS] name` (la cláusula `RECORD CONTAINS`
                // no lleva `KEY`, así que se distingue por la 2ª palabra).
                Some("RECORD") if kw(sent.get(i + 1)).as_deref() == Some("KEY") => {
                    i += 2;
                    if kw(sent.get(i)).as_deref() == Some("IS") {
                        i += 1;
                    }
                    record_key = kw(sent.get(i));
                }
                // `RELATIVE KEY [IS] name`.
                Some("RELATIVE") if kw(sent.get(i + 1)).as_deref() == Some("KEY") => {
                    i += 2;
                    if kw(sent.get(i)).as_deref() == Some("IS") {
                        i += 1;
                    }
                    relative_key = kw(sent.get(i));
                }
                _ => {}
            }
            i += 1;
        }
        program.files.push(FileEntry {
            name: name_tok.text.to_uppercase(),
            path,
            record: String::new(),
            organization,
            access,
            record_key,
            relative_key,
        });
    }
}

/// El resultado de parsear la DATA division: el árbol de datos y las
/// parejas `(FD, registro)` — el `01` que sigue a cada `FD`.
type DataResult = (Vec<DataItem>, Vec<(String, String)>);

/// Parsea la DATA division.
fn parse_data(body: &[Token]) -> Result<DataResult, ParseError> {
    let mut flat = Vec::new();
    let mut fd_records = Vec::new();
    let mut pending_fd: Option<String> = None;
    for sent in split_sentences(body) {
        let Some(first) = sent.first() else { continue };
        // Las sentencias que arrancan con un número de nivel son
        // entradas de datos; las demás son encabezados de SECTION o
        // de `FD`/`SD`.
        if first.kind != TokenKind::Number {
            if matches!(kw(Some(first)).as_deref(), Some("FD") | Some("SD")) {
                pending_fd = sent
                    .get(1)
                    .filter(|t| t.kind == TokenKind::Word)
                    .map(|t| t.text.to_uppercase());
            }
            continue;
        }
        let level = parse_level(first)?;
        let entry = parse_data_entry(level, &sent)?;
        // El primer `01` tras un `FD` es su registro.
        if level == 1 {
            if let Some(fd) = pending_fd.take() {
                fd_records.push((fd, entry.name.clone()));
            }
        }
        flat.push(entry);
    }
    Ok((build_tree(flat), fd_records))
}

/// Valida que el token sea un número de nivel COBOL (01-49, 66, 77, 88).
fn parse_level(t: &Token) -> Result<u8, ParseError> {
    let bad = || ParseError::BadLevel {
        line: t.line,
        found: t.text.clone(),
    };
    let n: u8 = t.text.parse().map_err(|_| bad())?;
    if (1..=49).contains(&n) || n == 66 || n == 77 || n == 88 {
        Ok(n)
    } else {
        Err(bad())
    }
}

/// Parsea una sola entrada de datos (una sentencia ya sin el punto).
fn parse_data_entry(level: u8, sent: &[Token]) -> Result<DataItem, ParseError> {
    let line = sent[0].line;
    let name_tok = sent.get(1).ok_or(ParseError::ExpectedDataName { line })?;
    if name_tok.kind != TokenKind::Word {
        return Err(ParseError::ExpectedDataName {
            line: name_tok.line,
        });
    }
    let name = name_tok.text.to_uppercase();

    let mut picture = None;
    let mut value = None;
    let mut occurs = None;
    let mut i = 2;
    while i < sent.len() {
        match kw(sent.get(i)).as_deref() {
            Some("OCCURS") => {
                i += 1;
                if let Some(t) = sent.get(i) {
                    if t.kind == TokenKind::Number {
                        if occurs.is_none() {
                            occurs = t.text.parse::<u32>().ok();
                        }
                        i += 1;
                    }
                }
                if kw(sent.get(i)).as_deref() == Some("TIMES") {
                    i += 1;
                }
            }
            Some("PIC") | Some("PICTURE") => {
                i += 1;
                if kw(sent.get(i)).as_deref() == Some("IS") {
                    i += 1;
                }
                let (pic, next) = collect_picture(sent, i);
                if picture.is_none() && !pic.is_empty() {
                    picture = Some(pic);
                }
                i = next;
            }
            Some("VALUE") => {
                i += 1;
                if matches!(kw(sent.get(i)).as_deref(), Some("IS") | Some("ARE")) {
                    i += 1;
                }
                let (val, next) = collect_value(sent, i);
                if value.is_none() {
                    value = val;
                }
                i = next;
            }
            _ => i += 1,
        }
    }

    Ok(DataItem {
        level,
        name,
        picture,
        value,
        occurs,
        children: Vec::new(),
    })
}

/// Reensambla la cláusula PICTURE: concatena el texto de los tokens
/// (`S9` `(` `5` `)` `V99` → `S9(5)V99`) hasta toparse con otra
/// cláusula de datos. El resultado va en mayúsculas.
fn collect_picture(sent: &[Token], start: usize) -> (String, usize) {
    let mut s = String::new();
    let mut i = start;
    while let Some(t) = sent.get(i) {
        if let Some(w) = kw(Some(t)) {
            if is_data_clause_kw(&w) {
                break;
            }
        }
        s.push_str(&t.text);
        i += 1;
    }
    (s.to_uppercase(), i)
}

/// Captura el literal de una cláusula VALUE: un signo opcional seguido
/// de un número, una constante figurativa o un literal de texto (que se
/// devuelve entre comillas simples para distinguirlo de una palabra).
fn collect_value(sent: &[Token], start: usize) -> (Option<String>, usize) {
    let mut i = start;
    let mut s = String::new();
    if let Some(t) = sent.get(i) {
        if t.kind == TokenKind::Symbol && (t.text == "+" || t.text == "-") {
            s.push_str(&t.text);
            i += 1;
        }
    }
    if let Some(t) = sent.get(i) {
        match t.kind {
            TokenKind::Number => {
                s.push_str(&t.text);
                i += 1;
            }
            TokenKind::String => {
                s.push('\'');
                s.push_str(&t.text);
                s.push('\'');
                i += 1;
            }
            TokenKind::Word => {
                // Constante figurativa: ZERO, ZEROS, SPACE, SPACES,
                // HIGH-VALUE, LOW-VALUE, QUOTES, NULL...
                s.push_str(&t.text.to_uppercase());
                i += 1;
            }
            TokenKind::Period | TokenKind::Symbol => {}
        }
    }
    if s.is_empty() {
        (None, i)
    } else {
        (Some(s), i)
    }
}

/// ¿Es `w` una palabra que abre una cláusula de datos? Marca el final
/// de la cláusula PICTURE.
fn is_data_clause_kw(w: &str) -> bool {
    matches!(
        w,
        "VALUE"
            | "OCCURS"
            | "REDEFINES"
            | "RENAMES"
            | "USAGE"
            | "COMP"
            | "COMP-1"
            | "COMP-2"
            | "COMP-3"
            | "COMP-4"
            | "COMP-5"
            | "COMPUTATIONAL"
            | "COMPUTATIONAL-1"
            | "COMPUTATIONAL-2"
            | "COMPUTATIONAL-3"
            | "COMPUTATIONAL-4"
            | "COMPUTATIONAL-5"
            | "BINARY"
            | "PACKED-DECIMAL"
            | "SIGN"
            | "JUSTIFIED"
            | "JUST"
            | "BLANK"
            | "SYNCHRONIZED"
            | "SYNC"
            | "DISPLAY"
    )
}

/// Convierte la lista plana de ítems en un árbol según los niveles: un
/// ítem cuelga del anterior cuyo nivel sea estrictamente menor. Los
/// niveles 01 y 77 son siempre raíces.
fn build_tree(flat: Vec<DataItem>) -> Vec<DataItem> {
    let mut roots: Vec<DataItem> = Vec::new();
    let mut stack: Vec<DataItem> = Vec::new();
    for item in flat {
        if item.level == 1 || item.level == 77 {
            while let Some(done) = stack.pop() {
                attach(&mut stack, &mut roots, done);
            }
        } else {
            // Cerrar los hermanos y descendientes ya completos.
            while stack.last().is_some_and(|top| top.level >= item.level) {
                let done = stack.pop().unwrap();
                attach(&mut stack, &mut roots, done);
            }
        }
        stack.push(item);
    }
    while let Some(done) = stack.pop() {
        attach(&mut stack, &mut roots, done);
    }
    roots
}

/// Engancha un ítem completo: como hijo del que esté en el tope de la
/// pila, o como raíz si la pila está vacía.
fn attach(stack: &mut [DataItem], roots: &mut Vec<DataItem>, item: DataItem) {
    if let Some(parent) = stack.last_mut() {
        parent.children.push(item);
    } else {
        roots.push(item);
    }
}

/// Parsea el cuerpo de la PROCEDURE division en párrafos.
fn parse_procedure(body: &[Token]) -> Vec<Paragraph> {
    let mut paras: Vec<Paragraph> = Vec::new();
    let mut current = Paragraph {
        name: String::new(),
        sentences: Vec::new(),
    };
    let mut current_named = false;
    for sent in split_sentences(body) {
        if let Some(name) = paragraph_header_name(&sent) {
            // Cerrar el párrafo en curso si tiene identidad o contenido.
            if current_named || !current.sentences.is_empty() {
                paras.push(std::mem::replace(
                    &mut current,
                    Paragraph {
                        name: String::new(),
                        sentences: Vec::new(),
                    },
                ));
            }
            current.name = name;
            current_named = true;
        } else {
            current.sentences.push(Sentence { tokens: sent });
        }
    }
    if current_named || !current.sentences.is_empty() {
        paras.push(current);
    }
    paras
}

/// ¿Es esta sentencia un encabezado de párrafo o de sección? Lo es si
/// es una sola palabra (un nombre de párrafo) o `NOMBRE SECTION`. Se
/// excluyen `EXIT`/`GOBACK`/`CONTINUE`, que de una sola palabra son
/// statements, no encabezados.
fn paragraph_header_name(sent: &[Token]) -> Option<String> {
    match sent {
        [w] if w.kind == TokenKind::Word => {
            let name = w.text.to_uppercase();
            if matches!(name.as_str(), "EXIT" | "GOBACK" | "CONTINUE") {
                None
            } else {
                Some(name)
            }
        }
        [w, s] if w.kind == TokenKind::Word && kw(Some(s)).as_deref() == Some("SECTION") => {
            Some(w.text.to_uppercase())
        }
        _ => None,
    }
}

/// Parte un tramo de tokens en sentencias por el punto terminador. El
/// punto se descarta; las sentencias vacías no se emiten.
fn split_sentences(body: &[Token]) -> Vec<Vec<Token>> {
    let mut out = Vec::new();
    let mut cur = Vec::new();
    for t in body {
        if t.kind == TokenKind::Period {
            if !cur.is_empty() {
                out.push(std::mem::take(&mut cur));
            }
        } else {
            cur.push(t.clone());
        }
    }
    if !cur.is_empty() {
        out.push(cur);
    }
    out
}

/// El índice del primer `Period` en `tokens` a partir de `start`.
fn period_after(tokens: &[Token], start: usize) -> Option<usize> {
    tokens
        .iter()
        .enumerate()
        .skip(start)
        .find(|(_, t)| t.kind == TokenKind::Period)
        .map(|(i, _)| i)
}

/// Si el token es una palabra, su texto en mayúsculas (COBOL es
/// case-insensitive). `None` para cualquier otra clase de token o
/// posición ausente.
fn kw(t: Option<&Token>) -> Option<String> {
    match t {
        Some(t) if t.kind == TokenKind::Word => Some(t.text.to_uppercase()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use chaka_lexer::{lex, SourceFormat};

    /// Helper: lexa el fuente en format libre y lo parsea.
    fn parse_src(src: &str) -> Program {
        let toks = lex(src, SourceFormat::Free).expect("lex OK");
        parse(&toks).expect("parse OK")
    }

    #[test]
    fn empty_input_yields_empty_program() {
        let p = parse(&[]).unwrap();
        assert_eq!(p, Program::default());
        assert!(p.program_id.is_none());
        assert!(p.data.is_empty());
        assert!(p.paragraphs.is_empty());
    }

    #[test]
    fn captures_program_id() {
        let p = parse_src(
            "IDENTIFICATION DIVISION.\n\
             PROGRAM-ID. PAYROLL.\n",
        );
        assert_eq!(p.program_id.as_deref(), Some("PAYROLL"));
    }

    #[test]
    fn id_division_short_form_and_case_normalized() {
        let p = parse_src("ID DIVISION.\nPROGRAM-ID. hello.\n");
        assert_eq!(p.program_id.as_deref(), Some("HELLO"));
    }

    #[test]
    fn flat_data_items() {
        let p = parse_src(
            "DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             77 WS-COUNT PIC 9(3) VALUE 0.\n\
             77 WS-NAME  PIC X(20).\n",
        );
        assert_eq!(p.data.len(), 2);
        assert_eq!(p.data[0].name, "WS-COUNT");
        assert_eq!(p.data[0].level, 77);
        assert_eq!(p.data[0].picture.as_deref(), Some("9(3)"));
        assert_eq!(p.data[0].value.as_deref(), Some("0"));
        assert_eq!(p.data[1].name, "WS-NAME");
        assert_eq!(p.data[1].picture.as_deref(), Some("X(20)"));
        assert!(p.data[1].value.is_none());
    }

    #[test]
    fn data_item_nesting() {
        let p = parse_src(
            "DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 CUSTOMER-RECORD.\n\
                05 CUST-NAME PIC X(20).\n\
                05 CUST-ADDR.\n\
                   10 STREET PIC X(30).\n\
                   10 CITY   PIC X(20).\n",
        );
        assert_eq!(p.data.len(), 1);
        let rec = &p.data[0];
        assert_eq!(rec.name, "CUSTOMER-RECORD");
        assert_eq!(rec.children.len(), 2);
        assert_eq!(rec.children[0].name, "CUST-NAME");
        let addr = &rec.children[1];
        assert_eq!(addr.name, "CUST-ADDR");
        assert_eq!(addr.children.len(), 2);
        assert_eq!(addr.children[0].name, "STREET");
        assert_eq!(addr.children[1].name, "CITY");
    }

    #[test]
    fn picture_clause_reassembled() {
        let p = parse_src(
            "DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-AMOUNT PIC S9(5)V99.\n",
        );
        assert_eq!(p.data[0].picture.as_deref(), Some("S9(5)V99"));
    }

    #[test]
    fn picture_and_name_lowercase_normalized() {
        let p = parse_src(
            "DATA DIVISION.\n\
             01 ws-x pic x(4).\n",
        );
        assert_eq!(p.data[0].name, "WS-X");
        assert_eq!(p.data[0].picture.as_deref(), Some("X(4)"));
    }

    #[test]
    fn value_variants() {
        let p = parse_src(
            "DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-TEXT  PIC X(5)  VALUE 'BOB'.\n\
             01 WS-ZERO  PIC 9(3)  VALUE ZERO.\n\
             01 WS-NEG   PIC S9(3) VALUE -5.\n",
        );
        assert_eq!(p.data[0].value.as_deref(), Some("'BOB'"));
        assert_eq!(p.data[1].value.as_deref(), Some("ZERO"));
        assert_eq!(p.data[2].value.as_deref(), Some("-5"));
    }

    #[test]
    fn level_88_condition_names_nest() {
        let p = parse_src(
            "DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-STATUS PIC X.\n\
                88 STATUS-OK  VALUE 'O'.\n\
                88 STATUS-BAD VALUE 'B'.\n",
        );
        let st = &p.data[0];
        assert_eq!(st.name, "WS-STATUS");
        assert_eq!(st.children.len(), 2);
        assert_eq!(st.children[0].level, 88);
        assert_eq!(st.children[0].name, "STATUS-OK");
        assert_eq!(st.children[0].value.as_deref(), Some("'O'"));
        assert_eq!(st.children[1].name, "STATUS-BAD");
    }

    #[test]
    fn filler_data_items() {
        let p = parse_src(
            "DATA DIVISION.\n\
             01 HEADER-LINE.\n\
                05 FILLER PIC X(10) VALUE 'NAME'.\n\
                05 FILLER PIC X(10) VALUE 'AGE'.\n",
        );
        assert_eq!(p.data[0].children.len(), 2);
        assert_eq!(p.data[0].children[0].name, "FILLER");
        assert_eq!(p.data[0].children[1].name, "FILLER");
    }

    #[test]
    fn select_and_fd_captured() {
        let p = parse_src(
            "ENVIRONMENT DIVISION.\n\
             INPUT-OUTPUT SECTION.\n\
             FILE-CONTROL.\n\
                 SELECT CLIENTES ASSIGN TO 'clientes.dat'.\n\
             DATA DIVISION.\n\
             FILE SECTION.\n\
             FD CLIENTES.\n\
             01 REG-CLIENTE PIC X(40).\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-FIN PIC X.\n",
        );
        assert_eq!(p.files.len(), 1);
        assert_eq!(p.files[0].name, "CLIENTES");
        assert_eq!(p.files[0].path, "clientes.dat");
        assert_eq!(p.files[0].record, "REG-CLIENTE");
        // Sin cláusulas, los defaults: line-sequential, acceso secuencial.
        assert_eq!(p.files[0].organization, FileOrg::LineSequential);
        assert_eq!(p.files[0].access, AccessMode::Sequential);
        assert_eq!(p.files[0].record_key, None);
    }

    #[test]
    fn select_indexed_captures_organization_and_key() {
        let p = parse_src(
            "ENVIRONMENT DIVISION.\n\
             INPUT-OUTPUT SECTION.\n\
             FILE-CONTROL.\n\
                 SELECT CLIENTES ASSIGN TO 'clientes.idx'\n\
                     ORGANIZATION IS INDEXED\n\
                     ACCESS MODE IS DYNAMIC\n\
                     RECORD KEY IS CLI-ID.\n\
             DATA DIVISION.\n\
             FILE SECTION.\n\
             FD CLIENTES.\n\
             01 REG-CLIENTE.\n\
                05 CLI-ID PIC X(5).\n\
                05 CLI-NOMBRE PIC X(35).\n",
        );
        assert_eq!(p.files[0].organization, FileOrg::Indexed);
        assert_eq!(p.files[0].access, AccessMode::Dynamic);
        assert_eq!(p.files[0].record_key.as_deref(), Some("CLI-ID"));
        assert_eq!(p.files[0].relative_key, None);
    }

    #[test]
    fn select_relative_captures_relative_key() {
        let p = parse_src(
            "ENVIRONMENT DIVISION.\n\
             INPUT-OUTPUT SECTION.\n\
             FILE-CONTROL.\n\
                 SELECT MOV ASSIGN TO 'mov.rel'\n\
                     ORGANIZATION IS RELATIVE\n\
                     ACCESS MODE IS RANDOM\n\
                     RELATIVE KEY IS MOV-NUM.\n",
        );
        assert_eq!(p.files[0].organization, FileOrg::Relative);
        assert_eq!(p.files[0].access, AccessMode::Random);
        assert_eq!(p.files[0].relative_key.as_deref(), Some("MOV-NUM"));
        assert_eq!(p.files[0].record_key, None);
    }

    #[test]
    fn occurs_clause_captured() {
        let p = parse_src(
            "DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-TABLA.\n\
                05 WS-ELEM PIC 9(3) OCCURS 10 TIMES.\n",
        );
        let elem = &p.data[0].children[0];
        assert_eq!(elem.name, "WS-ELEM");
        assert_eq!(elem.occurs, Some(10));
        assert_eq!(elem.picture.as_deref(), Some("9(3)"));
    }

    #[test]
    fn bad_level_number_is_error() {
        let toks = lex(
            "DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             99 WS-X PIC X.\n",
            SourceFormat::Free,
        )
        .unwrap();
        let err = parse(&toks).unwrap_err();
        assert!(matches!(err, ParseError::BadLevel { found, .. } if found == "99"));
    }

    #[test]
    fn procedure_paragraphs() {
        let p = parse_src(
            "PROCEDURE DIVISION.\n\
             MAIN-PARA.\n\
                 DISPLAY 'HELLO'.\n\
                 PERFORM SUB-PARA.\n\
                 STOP RUN.\n\
             SUB-PARA.\n\
                 DISPLAY 'SUB'.\n",
        );
        assert_eq!(p.paragraphs.len(), 2);
        assert_eq!(p.paragraphs[0].name, "MAIN-PARA");
        assert_eq!(p.paragraphs[0].sentences.len(), 3);
        assert_eq!(p.paragraphs[1].name, "SUB-PARA");
        assert_eq!(p.paragraphs[1].sentences.len(), 1);
        // La sentencia conserva sus tokens crudos.
        assert_eq!(p.paragraphs[0].sentences[0].tokens[0].text, "DISPLAY");
    }

    #[test]
    fn implicit_paragraph_for_leading_statements() {
        let p = parse_src(
            "PROCEDURE DIVISION.\n\
                 DISPLAY 'FIRST'.\n\
             MAIN-PARA.\n\
                 STOP RUN.\n",
        );
        assert_eq!(p.paragraphs.len(), 2);
        assert_eq!(p.paragraphs[0].name, "");
        assert_eq!(p.paragraphs[0].sentences.len(), 1);
        assert_eq!(p.paragraphs[1].name, "MAIN-PARA");
        assert_eq!(p.paragraphs[1].sentences.len(), 1);
    }

    #[test]
    fn procedure_section_header_recognized() {
        let p = parse_src(
            "PROCEDURE DIVISION.\n\
             INIT-SECTION SECTION.\n\
                 DISPLAY 'X'.\n",
        );
        assert_eq!(p.paragraphs.len(), 1);
        assert_eq!(p.paragraphs[0].name, "INIT-SECTION");
        assert_eq!(p.paragraphs[0].sentences.len(), 1);
    }

    #[test]
    fn full_program_end_to_end() {
        let p = parse_src(
            "IDENTIFICATION DIVISION.\n\
             PROGRAM-ID. ADDER.\n\
             ENVIRONMENT DIVISION.\n\
             DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-A     PIC 9(3) VALUE 10.\n\
             01 WS-B     PIC 9(3) VALUE 32.\n\
             01 WS-TOTAL PIC 9(4).\n\
             PROCEDURE DIVISION.\n\
             MAIN-PARA.\n\
                 ADD WS-A TO WS-B GIVING WS-TOTAL.\n\
                 DISPLAY WS-TOTAL.\n\
                 STOP RUN.\n",
        );
        assert_eq!(p.program_id.as_deref(), Some("ADDER"));
        assert_eq!(p.data.len(), 3);
        assert_eq!(p.data[0].value.as_deref(), Some("10"));
        assert_eq!(p.data[2].name, "WS-TOTAL");
        assert!(p.data[2].value.is_none());
        assert_eq!(p.paragraphs.len(), 1);
        assert_eq!(p.paragraphs[0].name, "MAIN-PARA");
        assert_eq!(p.paragraphs[0].sentences.len(), 3);
    }
}
