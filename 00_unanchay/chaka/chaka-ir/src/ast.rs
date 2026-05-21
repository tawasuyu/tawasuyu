//! Los tipos del IR: el programa COBOL con su PROCEDURE division ya
//! parseada a instrucciones tipadas.

pub use charka_parser::{DataItem, Token};

/// Un programa COBOL en representación intermedia.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Ir {
    /// El `PROGRAM-ID` ("" si el programa no lo declara).
    pub program_id: String,
    /// El árbol de [`DataItem`] tal cual lo produjo `charka-parser`,
    /// con su estructura de grupos.
    pub data: Vec<DataItem>,
    /// El modelo de datos resuelto: los datos elementales aplanados y
    /// los nombres de condición (nivel 88).
    pub model: crate::model::DataModel,
    /// Los párrafos del PROCEDURE, con sus statements ya tipados.
    pub procedures: Vec<Procedure>,
}

/// Un párrafo del PROCEDURE: un nombre y un cuerpo de statements.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct Procedure {
    /// Nombre del párrafo en mayúsculas; "" para el párrafo implícito.
    pub name: String,
    /// Los statements del párrafo, en orden.
    pub body: Vec<Stmt>,
}

/// Un operando: lo que puede ir donde se espera un valor.
#[derive(Debug, Clone, PartialEq)]
pub enum Operand {
    /// Referencia a un dato, por nombre (en mayúsculas).
    Data(String),
    /// Literal numérico (texto, posiblemente con signo).
    Num(String),
    /// Literal de texto.
    Str(String),
    /// Constante figurativa (`ZERO`, `SPACE`...).
    Figurative(Figurative),
}

/// Las constantes figurativas de COBOL.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Figurative {
    Zero,
    Space,
    HighValue,
    LowValue,
    Quote,
    Null,
}

/// Una expresión aritmética (la parte derecha de un `COMPUTE`).
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BinOp {
    Add,
    Sub,
    Mul,
    Div,
    /// Exponenciación (`**`).
    Pow,
}

/// Una condición (la guarda de un `IF` o de un `PERFORM UNTIL`).
#[derive(Debug, Clone, PartialEq)]
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
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CmpOp {
    Eq,
    Ne,
    Lt,
    Gt,
    Le,
    Ge,
}

/// Un statement del PROCEDURE division.
#[derive(Debug, Clone, PartialEq)]
pub enum Stmt {
    /// `MOVE from TO to...`
    Move { from: Operand, to: Vec<String> },
    /// `DISPLAY items...`
    Display { items: Vec<Operand> },
    /// `ACCEPT into`
    Accept { into: String },
    /// `COMPUTE targets... [ROUNDED] = expr`
    Compute {
        targets: Vec<String>,
        rounded: bool,
        expr: Expr,
    },
    /// `ADD addends... TO to... [GIVING giving...]`
    Add {
        addends: Vec<Operand>,
        to: Vec<String>,
        giving: Vec<String>,
        rounded: bool,
    },
    /// `SUBTRACT amounts... FROM from... [GIVING giving...]`
    Subtract {
        amounts: Vec<Operand>,
        from: Vec<String>,
        giving: Vec<String>,
        rounded: bool,
    },
    /// `MULTIPLY left BY by [GIVING giving...]`
    Multiply {
        left: Operand,
        by: Operand,
        giving: Vec<String>,
        rounded: bool,
    },
    /// `DIVIDE left {BY|INTO} right [GIVING giving...]`. `by_form` es
    /// true para `BY` (`left/right`), false para `INTO` (`right/left`).
    Divide {
        left: Operand,
        right: Operand,
        by_form: bool,
        giving: Vec<String>,
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
    /// `PERFORM ...` — ver [`Perform`].
    Perform(Perform),
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

/// Una rama `WHEN` de un `EVALUATE`: los valores que la disparan
/// (varios `WHEN` apilados comparten cuerpo) y el cuerpo a ejecutar.
#[derive(Debug, Clone, PartialEq)]
pub struct WhenBranch {
    pub values: Vec<Operand>,
    pub body: Vec<Stmt>,
}

/// Un statement `PERFORM`: a quién ejecuta y cuántas veces.
#[derive(Debug, Clone, PartialEq)]
pub struct Perform {
    pub target: PerformTarget,
    pub control: PerformControl,
}

/// El cuerpo que un `PERFORM` ejecuta.
#[derive(Debug, Clone, PartialEq)]
pub enum PerformTarget {
    /// `PERFORM PARA [THRU PARA2]` — ejecuta uno o un rango de párrafos.
    Paragraph { name: String, thru: Option<String> },
    /// `PERFORM ... statements ... END-PERFORM` — cuerpo en línea.
    Inline(Vec<Stmt>),
}

/// Cuántas veces se ejecuta el cuerpo de un `PERFORM`.
#[derive(Debug, Clone, PartialEq)]
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
