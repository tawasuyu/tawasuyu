//! Los tipos del IR: el programa COBOL con su PROCEDURE division ya
//! parseada a instrucciones tipadas.

pub use chaka_parser::{DataItem, FileEntry, Token};

/// Un programa COBOL en representación intermedia.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Ir {
    /// El `PROGRAM-ID` ("" si el programa no lo declara).
    pub program_id: String,
    /// El árbol de [`DataItem`] tal cual lo produjo `chaka-parser`,
    /// con su estructura de grupos.
    pub data: Vec<DataItem>,
    /// El modelo de datos resuelto: los datos elementales aplanados y
    /// los nombres de condición (nivel 88).
    pub model: crate::model::DataModel,
    /// Los ficheros declarados (`SELECT` + `FD`).
    pub files: Vec<FileEntry>,
    /// Los párrafos del PROCEDURE, con sus statements ya tipados.
    pub procedures: Vec<Procedure>,
}

/// Un párrafo del PROCEDURE: un nombre y un cuerpo de statements.
#[derive(Debug, Clone, PartialEq, Default, serde::Serialize, serde::Deserialize)]
pub struct Procedure {
    /// Nombre del párrafo en mayúsculas; "" para el párrafo implícito.
    pub name: String,
    /// Los statements del párrafo, en orden.
    pub body: Vec<Stmt>,
}

/// Un operando: lo que puede ir donde se espera un valor.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Operand {
    /// Referencia a un dato, por nombre (en mayúsculas).
    Data(String),
    /// Referencia a un elemento de tabla: `name(index)`. El subíndice
    /// es 1-based, como en COBOL.
    Indexed { name: String, index: Box<Operand> },
    /// Literal numérico (texto, posiblemente con signo).
    Num(String),
    /// Literal de texto.
    Str(String),
    /// Constante figurativa (`ZERO`, `SPACE`...).
    Figurative(Figurative),
}

/// Las constantes figurativas de COBOL.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum Figurative {
    Zero,
    Space,
    HighValue,
    LowValue,
    Quote,
    Null,
}

/// Una expresión aritmética (la parte derecha de un `COMPUTE`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Expr {
    /// Un operando hoja.
    Operand(Operand),
    /// Negación unaria.
    Neg(Box<Expr>),
    /// Operación binaria.
    Binary {
        op: BinOp,
        lhs: Box<Expr>,
        rhs: Box<Expr>,
    },
}

/// Operador aritmético binario.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    /// Exponenciación (`**`).
    Pow,
}

/// Una condición (la guarda de un `IF` o de un `PERFORM UNTIL`).
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Cond {
    /// Comparación relacional `lhs op rhs`.
    Compare {
        lhs: Operand,
        op: CmpOp,
        rhs: Operand,
    },
    /// Un nombre de condición (un dato de nivel 88) usado solo.
    Named(String),
    /// Negación lógica.
    Not(Box<Cond>),
    /// Conjunción lógica.
    And(Box<Cond>, Box<Cond>),
    /// Disyunción lógica.
    Or(Box<Cond>, Box<Cond>),
}

/// Operador relacional.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

/// Un statement del PROCEDURE division.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum Stmt {
    /// `MOVE from TO to...`
    Move { from: Operand, to: Vec<Operand> },
    /// `DISPLAY items...`
    Display { items: Vec<Operand> },
    /// `ACCEPT into`
    Accept { into: Operand },
    /// `COMPUTE targets... [ROUNDED] = expr`
    Compute {
        targets: Vec<Operand>,
        rounded: bool,
        expr: Expr,
    },
    /// `ADD addends... TO to... [GIVING giving...]`
    Add {
        addends: Vec<Operand>,
        to: Vec<Operand>,
        giving: Vec<Operand>,
        rounded: bool,
    },
    /// `SUBTRACT amounts... FROM from... [GIVING giving...]`
    Subtract {
        amounts: Vec<Operand>,
        from: Vec<Operand>,
        giving: Vec<Operand>,
        rounded: bool,
    },
    /// `MULTIPLY left BY by [GIVING giving...]`
    Multiply {
        left: Operand,
        by: Operand,
        giving: Vec<Operand>,
        rounded: bool,
    },
    /// `DIVIDE left {BY|INTO} right [GIVING giving...]`. `by_form` es
    /// true para `BY` (`left/right`), false para `INTO` (`right/left`).
    Divide {
        left: Operand,
        right: Operand,
        by_form: bool,
        giving: Vec<Operand>,
        rounded: bool,
    },
    /// `IF cond [THEN] then_branch [ELSE else_branch] [END-IF]`
    If {
        cond: Cond,
        then_branch: Vec<Stmt>,
        else_branch: Vec<Stmt>,
    },
    /// `EVALUATE subject WHEN ... [WHEN OTHER ...] END-EVALUATE` — el
    /// `case` de COBOL. Una rama se elige si `subject` es igual a
    /// alguno de sus valores; sin caída entre ramas.
    Evaluate {
        subject: Operand,
        whens: Vec<WhenBranch>,
        /// El cuerpo de `WHEN OTHER` (vacío si no hay).
        other: Vec<Stmt>,
    },
    /// `STRING sources... DELIMITED BY SIZE INTO into` — concatena el
    /// texto de los `sources` en `into`.
    StringConcat {
        sources: Vec<Operand>,
        into: Operand,
    },
    /// `UNSTRING source DELIMITED BY delimiter INTO into...` — parte el
    /// texto de `source` por `delimiter` y reparte los trozos.
    Unstring {
        source: Operand,
        delimiter: Operand,
        into: Vec<Operand>,
    },
    /// `INSPECT target ...` — cuenta o reemplaza caracteres.
    Inspect { target: Operand, op: InspectOp },
    /// `INITIALIZE targets...` — pone cada dato (o grupo) en su valor
    /// por defecto: 0 los numéricos, espacios los alfanuméricos.
    Initialize { targets: Vec<Operand> },
    /// `SET cond-name... TO TRUE` — hace verdaderos esos nombres de
    /// condición (nivel 88): asigna a su dato padre el valor del 88.
    SetTrue { conditions: Vec<String> },
    /// `SET name... TO value` — asigna `value` a cada `name` (copia
    /// directa, sin involucrar nombres de condición 88).
    SetTo { targets: Vec<String>, value: Operand },
    /// `SET name... UP BY n` / `SET name... DOWN BY n` — incrementa o
    /// decrementa cada `name` en `by` unidades.
    SetAdjust {
        targets: Vec<String>,
        by: Operand,
        up: bool,
    },
    /// `OPEN {INPUT|OUTPUT} files...`
    Open { mode: FileMode, files: Vec<String> },
    /// `CLOSE files...`
    Close { files: Vec<String> },
    /// `READ file [AT END at_end] [NOT AT END not_at_end] [END-READ]`
    Read {
        file: String,
        at_end: Vec<Stmt>,
        not_at_end: Vec<Stmt>,
    },
    /// `WRITE record [FROM from]`
    Write {
        record: String,
        from: Option<Operand>,
    },
    /// `REWRITE record [FROM from] [INVALID KEY ...] [NOT INVALID KEY ...]
    /// [END-REWRITE]`. La v1 reescribe sobre un fichero line-sequential
    /// como un `WRITE` más; `INVALID KEY` no se dispara nunca, así que
    /// la rama `NOT INVALID KEY` siempre corre.
    Rewrite {
        record: String,
        from: Option<Operand>,
        invalid_key: Vec<Stmt>,
        not_invalid_key: Vec<Stmt>,
    },
    /// `DELETE file [INVALID KEY ...] [NOT INVALID KEY ...] [END-DELETE]`.
    /// La v1 trata `DELETE` como un no-op sobre un fichero secuencial;
    /// dispara siempre la rama `NOT INVALID KEY`.
    Delete {
        file: String,
        invalid_key: Vec<Stmt>,
        not_invalid_key: Vec<Stmt>,
    },
    /// `START file [KEY {= | > | >= | < | <=} k] [INVALID KEY ...]
    /// [NOT INVALID KEY ...] [END-START]`. La v1 trata `START` como un
    /// no-op (sólo soporta acceso secuencial); dispara siempre la rama
    /// `NOT INVALID KEY`.
    Start {
        file: String,
        invalid_key: Vec<Stmt>,
        not_invalid_key: Vec<Stmt>,
    },
    /// `PERFORM ...` — ver [`Perform`].
    Perform(Perform),
    /// `SORT sort-file [ON {ASCENDING|DESCENDING} KEY k...] USING in...
    /// GIVING out...`. La v1 ignora las claves: lee todas las líneas de
    /// los `using`, las ordena lexicográficamente y las vuelca a cada
    /// `giving`. `INPUT/OUTPUT PROCEDURE` queda fuera de alcance.
    Sort {
        sort_file: String,
        using: Vec<String>,
        giving: Vec<String>,
    },
    /// `MERGE sort-file [ON KEY ...] USING in... GIVING out...`. Idem
    /// `SORT` en la v1: las entradas suelen estar ya ordenadas, así que
    /// un re-sort lexicográfico da el mismo resultado.
    Merge {
        sort_file: String,
        using: Vec<String>,
        giving: Vec<String>,
    },
    /// `SEARCH table VARYING idx AT END ... WHEN cond ... END-SEARCH` —
    /// búsqueda lineal sobre una tabla `OCCURS`. La v1 sólo modela la
    /// forma `VARYING idx`: el bucle incrementa `idx` desde su valor
    /// actual hasta agotar la tabla, evalúa cada `WHEN` en cada vuelta y
    /// dispara `AT END` si ninguna se cumplió.
    Search {
        table: String,
        varying: String,
        at_end: Vec<Stmt>,
        whens: Vec<SearchBranch>,
    },
    /// `CALL program USING args... [ON OVERFLOW ...] [NOT ON OVERFLOW ...]`.
    /// La v1 modela un sub-programa cuyo nombre coincide con un párrafo
    /// del mismo programa: si lo encuentra, lo ejecuta como un `PERFORM`.
    /// Si no lo encuentra, dispara `on_overflow` (la rama de fallo).
    Call {
        program: Operand,
        using: Vec<Operand>,
        on_overflow: Vec<Stmt>,
        not_on_overflow: Vec<Stmt>,
    },
    /// `GO TO target`
    GoTo { target: String },
    /// `STOP RUN`
    StopRun,
    /// `GOBACK`
    Goback,
    /// `EXIT` (y sus variantes `EXIT PROGRAM`/`PARAGRAPH`...).
    Exit,
    /// `CONTINUE` — el no-op de COBOL.
    Continue,
    /// Un verbo que la v1 no parsea: se conserva crudo para que las
    /// etapas siguientes (o un humano) lo revisen.
    Unknown { verb: String, tokens: Vec<Token> },
}

/// El modo de apertura de un fichero.
#[derive(Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
pub enum FileMode {
    /// `OPEN INPUT` — para lectura.
    Input,
    /// `OPEN OUTPUT` — para escritura (crea el fichero de cero).
    Output,
}

/// La operación de un `INSPECT`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum InspectOp {
    /// `TALLYING counter FOR ALL search` — suma a `counter` la cantidad
    /// de apariciones de `search` en el destino.
    TallyingForAll { counter: Operand, search: Operand },
    /// `TALLYING counter FOR LEADING search` — cuenta sólo las
    /// apariciones consecutivas de `search` desde el inicio del campo.
    TallyingForLeading { counter: Operand, search: Operand },
    /// `REPLACING ALL from BY to` — reemplaza las apariciones de `from`.
    ReplacingAll { from: Operand, to: Operand },
    /// `CONVERTING from TO to` — traduce carácter a carácter: el carácter
    /// en `from[i]` se reemplaza por el de `to[i]`.
    Converting { from: Operand, to: Operand },
}

/// Cómo una rama `WHEN` decide si se dispara.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum WhenTest {
    /// El sujeto es igual a este valor.
    Value(Operand),
    /// El sujeto está en el rango `[lo, hi]` (`WHEN lo THRU hi`).
    Range(Operand, Operand),
    /// La condición se cumple — la forma `EVALUATE TRUE WHEN cond`.
    Cond(Cond),
}

/// Una rama `WHEN` de un `EVALUATE`: las pruebas que la disparan
/// (varios `WHEN` apilados comparten cuerpo) y el cuerpo a ejecutar.
/// La rama se dispara si **alguna** de sus pruebas pasa.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct WhenBranch {
    pub tests: Vec<WhenTest>,
    pub body: Vec<Stmt>,
}

/// Una rama `WHEN` de un `SEARCH`: la condición que la dispara y el
/// cuerpo a ejecutar al encontrarla. El bucle se corta al disparar una.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct SearchBranch {
    pub cond: Cond,
    pub body: Vec<Stmt>,
}

/// Un statement `PERFORM`: a quién ejecuta y cuántas veces.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub struct Perform {
    pub target: PerformTarget,
    pub control: PerformControl,
}

/// El cuerpo que un `PERFORM` ejecuta.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PerformTarget {
    /// `PERFORM PARA [THRU PARA2]` — ejecuta uno o un rango de párrafos.
    Paragraph { name: String, thru: Option<String> },
    /// `PERFORM ... statements ... END-PERFORM` — cuerpo en línea.
    Inline(Vec<Stmt>),
}

/// Cuántas veces se ejecuta el cuerpo de un `PERFORM`.
#[derive(Debug, Clone, PartialEq, serde::Serialize, serde::Deserialize)]
pub enum PerformControl {
    /// Una sola vez.
    Once,
    /// `n TIMES`.
    Times(Operand),
    /// `UNTIL cond`.
    Until(Cond),
    /// `VARYING var FROM from BY by UNTIL until` — el bucle con
    /// variable de control: `var = from`; mientras no se cumpla
    /// `until`, ejecuta el cuerpo y hace `var = var + by`.
    Varying {
        var: String,
        from: Operand,
        by: Operand,
        until: Cond,
    },
}
