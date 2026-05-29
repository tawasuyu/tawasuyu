//! `tinkuy-dsl` — DSL matemático mínimo para fuerzas pairwise.
//!
//! Capa 3 del roadmap (D1-D5, cerrada). Este crate aporta gramática +
//! lexer + parser Pratt → AST (`lib.rs`), compilador a bytecode stack-machine
//! con eval cero-alloc (`bytecode.rs`) y optimizer fold+algebraico
//! (`optimize.rs`). `tinkuy-forces::DslForce` lo enchufa como `Force` sobre
//! `tinkuy-core::World`.
//!
//! **Gramática** (EBNF-ish):
//!
//! ```text
//! expr     := add
//! add      := mul (('+' | '-') mul)*
//! mul      := unary (('*' | '/') unary)*
//! unary    := '-' unary | atom
//! atom     := number | var | call | '(' expr ')'
//! call     := func '(' (expr (',' expr)*)? ')'
//! number   := [0-9]+ ('.' [0-9]+)? (('e'|'E') ('+'|'-')? [0-9]+)?
//! var      := 'r' | 'r2' | 'eps' | 'sigma'
//!           | 'qi' | 'qj' | 'mi' | 'mj'
//!           | 'dx' | 'dy' | 'dz'
//! func     := 'pow' | 'inv' | 'sqrt'
//! ```
//!
//! Sin lambdas, sin control de flujo, sin operadores de comparación. Los
//! literales que un solver de partículas necesita y nada más — el DSL existe
//! para que un nodo de UI o un archivo .tnk describa F(r) sin recompilar.
//!
//! Convenciones:
//!   - `r2 = |r_ij|²` está disponible para evitar el `sqrt` cuando no hace
//!     falta (todas las fuerzas tipo LJ lo prefieren).
//!   - `dx, dy, dz` son las componentes de `(r_i − r_j)`. Útiles si el caller
//!     quiere recomponer el vector fuerza desde una magnitud escalar.
//!   - El parser es estricto con idents desconocidos: error en parse, no en
//!     eval. Así nodos visuales del futuro garantizan que cada compilación
//!     produce un programa válido o un mensaje claro.

#![cfg_attr(not(feature = "std"), no_std)]

extern crate alloc;

use alloc::boxed::Box;
use alloc::string::String;
use alloc::vec::Vec;

pub mod bytecode;
pub mod optimize;
pub use bytecode::{compile, eval_with_stack, Bytecode, CompileError, EvalError, Op, VarBindings};
pub use optimize::optimize;

// ─── AST ────────────────────────────────────────────────────────────────────

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Var {
    R, R2,
    Eps, Sigma,
    Qi, Qj, Mi, Mj,
    Dx, Dy, Dz,
}

impl Var {
    /// Tabla maestra: ident léxico → variable. Mantenerla aquí garantiza una
    /// única fuente de verdad para lexer, parser y (D2) compilador.
    pub fn from_ident(s: &str) -> Option<Var> {
        Some(match s {
            "r"     => Var::R,
            "r2"    => Var::R2,
            "eps"   => Var::Eps,
            "sigma" => Var::Sigma,
            "qi"    => Var::Qi,
            "qj"    => Var::Qj,
            "mi"    => Var::Mi,
            "mj"    => Var::Mj,
            "dx"    => Var::Dx,
            "dy"    => Var::Dy,
            "dz"    => Var::Dz,
            _ => return None,
        })
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum BinOp { Add, Sub, Mul, Div }

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum Func { Pow, Inv, Sqrt }

impl Func {
    pub fn from_ident(s: &str) -> Option<Func> {
        Some(match s {
            "pow"  => Func::Pow,
            "inv"  => Func::Inv,
            "sqrt" => Func::Sqrt,
            _ => return None,
        })
    }

    /// Aridad esperada. Crítico para validar `call` en parse-time.
    pub fn arity(self) -> usize {
        match self {
            Func::Pow  => 2,
            Func::Inv  => 1,
            Func::Sqrt => 1,
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Expr {
    Num(f32),
    Var(Var),
    Neg(Box<Expr>),
    Bin(BinOp, Box<Expr>, Box<Expr>),
    Call(Func, Vec<Expr>),
}

// ─── Errores ────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum ParseError {
    UnexpectedChar { pos: usize, ch: char },
    UnknownIdent { pos: usize, ident: String },
    UnexpectedToken { pos: usize, what: &'static str },
    BadNumber { pos: usize, raw: String },
    ArityMismatch { pos: usize, func: Func, expected: usize, got: usize },
    UnexpectedEof { what: &'static str },
}

// ─── Lexer ──────────────────────────────────────────────────────────────────

#[derive(Clone, Debug, PartialEq)]
pub enum Tok {
    Num(f32),
    Var(Var),
    Func(Func),
    LParen, RParen, Comma,
    Plus, Minus, Star, Slash,
}

#[derive(Clone, Debug)]
pub struct Token {
    pub tok: Tok,
    pub pos: usize,
}

pub fn lex(src: &str) -> Result<Vec<Token>, ParseError> {
    let bytes = src.as_bytes();
    let mut i = 0;
    let mut out = Vec::new();
    while i < bytes.len() {
        let c = bytes[i] as char;
        match c {
            ' ' | '\t' | '\n' | '\r' => { i += 1; }
            // Comentarios `#` hasta fin de línea. Útiles en archivos `.tnk`
            // para anotar fórmulas y convenciones sin interferir con el AST.
            '#' => {
                while i < bytes.len() && bytes[i] != b'\n' { i += 1; }
            }
            '(' => { out.push(Token { tok: Tok::LParen, pos: i }); i += 1; }
            ')' => { out.push(Token { tok: Tok::RParen, pos: i }); i += 1; }
            ',' => { out.push(Token { tok: Tok::Comma,  pos: i }); i += 1; }
            '+' => { out.push(Token { tok: Tok::Plus,   pos: i }); i += 1; }
            '-' => { out.push(Token { tok: Tok::Minus,  pos: i }); i += 1; }
            '*' => { out.push(Token { tok: Tok::Star,   pos: i }); i += 1; }
            '/' => { out.push(Token { tok: Tok::Slash,  pos: i }); i += 1; }
            c if c.is_ascii_digit() || c == '.' => {
                let start = i;
                // entero opcional + parte decimal opcional + exponente opcional.
                // No deja consumir dos puntos (no exigimos AST de error, pero
                // `f32::from_str` capturará "1.2.3" y devolverá BadNumber).
                while i < bytes.len() && (bytes[i].is_ascii_digit() || bytes[i] == b'.') {
                    i += 1;
                }
                if i < bytes.len() && (bytes[i] == b'e' || bytes[i] == b'E') {
                    i += 1;
                    if i < bytes.len() && (bytes[i] == b'+' || bytes[i] == b'-') {
                        i += 1;
                    }
                    while i < bytes.len() && bytes[i].is_ascii_digit() {
                        i += 1;
                    }
                }
                let raw = &src[start..i];
                match raw.parse::<f32>() {
                    Ok(v) if v.is_finite() => out.push(Token { tok: Tok::Num(v), pos: start }),
                    _ => return Err(ParseError::BadNumber { pos: start, raw: String::from(raw) }),
                }
            }
            c if c.is_ascii_alphabetic() || c == '_' => {
                let start = i;
                while i < bytes.len()
                    && (bytes[i].is_ascii_alphanumeric() || bytes[i] == b'_')
                {
                    i += 1;
                }
                let ident = &src[start..i];
                // Resolución determinista: Func primero, Var después. No hay
                // colisiones en la tabla actual; si las hubiera en el futuro,
                // este orden las hace explícitas.
                if let Some(f) = Func::from_ident(ident) {
                    out.push(Token { tok: Tok::Func(f), pos: start });
                } else if let Some(v) = Var::from_ident(ident) {
                    out.push(Token { tok: Tok::Var(v), pos: start });
                } else {
                    return Err(ParseError::UnknownIdent {
                        pos: start, ident: String::from(ident),
                    });
                }
            }
            _ => return Err(ParseError::UnexpectedChar { pos: i, ch: c }),
        }
    }
    Ok(out)
}

// ─── Parser Pratt ───────────────────────────────────────────────────────────

struct Parser<'a> {
    toks: &'a [Token],
    pos: usize,
}

impl<'a> Parser<'a> {
    fn new(toks: &'a [Token]) -> Self { Self { toks, pos: 0 } }
    fn peek(&self) -> Option<&'a Token> { self.toks.get(self.pos) }
    fn advance(&mut self) -> Option<&'a Token> {
        let t = self.toks.get(self.pos);
        if t.is_some() { self.pos += 1; }
        t
    }
    /// Posición textual del cursor para mensajes de error. Usa la posición del
    /// próximo token si existe; si estamos al final, del último.
    fn here_pos(&self) -> usize {
        self.toks.get(self.pos)
            .or_else(|| self.toks.last())
            .map(|t| t.pos).unwrap_or(0)
    }

    fn parse_add(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_mul()?;
        loop {
            let op = match self.peek().map(|t| &t.tok) {
                Some(Tok::Plus)  => { self.advance(); BinOp::Add }
                Some(Tok::Minus) => { self.advance(); BinOp::Sub }
                _ => break,
            };
            let rhs = self.parse_mul()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_mul(&mut self) -> Result<Expr, ParseError> {
        let mut lhs = self.parse_unary()?;
        loop {
            let op = match self.peek().map(|t| &t.tok) {
                Some(Tok::Star)  => { self.advance(); BinOp::Mul }
                Some(Tok::Slash) => { self.advance(); BinOp::Div }
                _ => break,
            };
            let rhs = self.parse_unary()?;
            lhs = Expr::Bin(op, Box::new(lhs), Box::new(rhs));
        }
        Ok(lhs)
    }

    fn parse_unary(&mut self) -> Result<Expr, ParseError> {
        if let Some(Token { tok: Tok::Minus, .. }) = self.peek() {
            self.advance();
            let inner = self.parse_unary()?;
            return Ok(Expr::Neg(Box::new(inner)));
        }
        self.parse_atom()
    }

    fn parse_atom(&mut self) -> Result<Expr, ParseError> {
        let Some(tk) = self.advance() else {
            return Err(ParseError::UnexpectedEof { what: "expresión" });
        };
        match &tk.tok {
            Tok::Num(v) => Ok(Expr::Num(*v)),
            Tok::Var(v) => Ok(Expr::Var(*v)),
            Tok::LParen => {
                let e = self.parse_add()?;
                match self.advance().map(|t| &t.tok) {
                    Some(Tok::RParen) => Ok(e),
                    _ => Err(ParseError::UnexpectedToken {
                        pos: self.here_pos(), what: "se esperaba ')'",
                    }),
                }
            }
            Tok::Func(f) => {
                let f = *f;
                let call_pos = tk.pos;
                match self.advance().map(|t| &t.tok) {
                    Some(Tok::LParen) => {}
                    _ => return Err(ParseError::UnexpectedToken {
                        pos: self.here_pos(), what: "se esperaba '(' tras función",
                    }),
                }
                let mut args = Vec::new();
                // Permite 0 args (no usado por ninguna func actual, pero el
                // parser no debe panicar si llegan).
                if !matches!(self.peek().map(|t| &t.tok), Some(Tok::RParen)) {
                    loop {
                        args.push(self.parse_add()?);
                        match self.peek().map(|t| &t.tok) {
                            Some(Tok::Comma) => { self.advance(); }
                            Some(Tok::RParen) => break,
                            _ => return Err(ParseError::UnexpectedToken {
                                pos: self.here_pos(),
                                what: "se esperaba ',' o ')' en argumentos",
                            }),
                        }
                    }
                }
                match self.advance().map(|t| &t.tok) {
                    Some(Tok::RParen) => {}
                    _ => return Err(ParseError::UnexpectedToken {
                        pos: self.here_pos(), what: "se esperaba ')' al cerrar call",
                    }),
                }
                if args.len() != f.arity() {
                    return Err(ParseError::ArityMismatch {
                        pos: call_pos, func: f, expected: f.arity(), got: args.len(),
                    });
                }
                Ok(Expr::Call(f, args))
            }
            other => Err(ParseError::UnexpectedToken {
                pos: tk.pos,
                what: match other {
                    Tok::RParen => "')' inesperado",
                    Tok::Comma  => "',' inesperada",
                    Tok::Plus | Tok::Minus | Tok::Star | Tok::Slash => "operador inesperado",
                    _ => "token inesperado",
                },
            }),
        }
    }
}

/// Parsea una expresión completa. Falla si sobra entrada tras la expresión.
pub fn parse(src: &str) -> Result<Expr, ParseError> {
    let toks = lex(src)?;
    let mut p = Parser::new(&toks);
    let e = p.parse_add()?;
    if p.peek().is_some() {
        return Err(ParseError::UnexpectedToken {
            pos: p.here_pos(),
            what: "tokens sobrantes tras la expresión",
        });
    }
    Ok(e)
}

// ─── Tests ──────────────────────────────────────────────────────────────────

#[cfg(test)]
mod tests {
    use super::*;
    use alloc::vec;

    fn n(v: f32) -> Expr { Expr::Num(v) }
    fn v(var: Var) -> Expr { Expr::Var(var) }
    fn bin(op: BinOp, a: Expr, b: Expr) -> Expr { Expr::Bin(op, Box::new(a), Box::new(b)) }
    fn neg(a: Expr) -> Expr { Expr::Neg(Box::new(a)) }
    fn call(f: Func, args: Vec<Expr>) -> Expr { Expr::Call(f, args) }

    #[test]
    fn lex_number_formats() {
        assert_eq!(lex("1").unwrap()[0].tok, Tok::Num(1.0));
        assert_eq!(lex("1.5").unwrap()[0].tok, Tok::Num(1.5));
        assert_eq!(lex(".5").unwrap()[0].tok, Tok::Num(0.5));
        assert_eq!(lex("1e3").unwrap()[0].tok, Tok::Num(1000.0));
        assert_eq!(lex("2.5E-2").unwrap()[0].tok, Tok::Num(0.025));
    }

    #[test]
    fn lex_idents_known_and_unknown() {
        let t = lex("r2 eps pow").unwrap();
        assert_eq!(t.len(), 3);
        assert_eq!(t[0].tok, Tok::Var(Var::R2));
        assert_eq!(t[1].tok, Tok::Var(Var::Eps));
        assert_eq!(t[2].tok, Tok::Func(Func::Pow));

        let bad = lex("foo").unwrap_err();
        assert!(matches!(bad, ParseError::UnknownIdent { .. }));
    }

    #[test]
    fn precedence_mul_over_add() {
        // 1 + 2 * 3 → 1 + (2 * 3), no (1 + 2) * 3
        let e = parse("1 + 2 * 3").unwrap();
        assert_eq!(e, bin(BinOp::Add, n(1.0), bin(BinOp::Mul, n(2.0), n(3.0))));
    }

    #[test]
    fn precedence_unary_neg() {
        // -2 * 3 → (-2) * 3 (neg vincula más fuerte que *)
        let e = parse("-2 * 3").unwrap();
        assert_eq!(e, bin(BinOp::Mul, neg(n(2.0)), n(3.0)));
    }

    #[test]
    fn left_associative_subtraction() {
        // 5 - 2 - 1 → (5 - 2) - 1 = 2, no 5 - (2 - 1) = 4
        let e = parse("5 - 2 - 1").unwrap();
        assert_eq!(e, bin(BinOp::Sub, bin(BinOp::Sub, n(5.0), n(2.0)), n(1.0)));
    }

    #[test]
    fn parens_override_precedence() {
        let e = parse("(1 + 2) * 3").unwrap();
        assert_eq!(e, bin(BinOp::Mul, bin(BinOp::Add, n(1.0), n(2.0)), n(3.0)));
    }

    #[test]
    fn lennard_jones_force_compiles() {
        // F_LJ_radial = 24·eps · (2·pow(sigma/r,12) − pow(sigma/r,6)) · inv(r2)
        let src = "24 * eps * (2 * pow(sigma / r, 12) - pow(sigma / r, 6)) * inv(r2)";
        let e = parse(src).expect("LJ debe parsear");
        // Forma esperada: la raíz es una multiplicación cuya última operación
        // a la derecha es inv(r2) (precedencia mul left-associative).
        if let Expr::Bin(BinOp::Mul, _, rhs) = &e {
            assert_eq!(**rhs, call(Func::Inv, vec![v(Var::R2)]));
        } else {
            panic!("estructura LJ inesperada: {e:?}");
        }
    }

    #[test]
    fn coulomb_force_compiles() {
        // F_C = qi · qj · inv(r2) · sqrt(r2) — equivalente a qi·qj/r³
        let src = "qi * qj * inv(r2) * sqrt(r2)";
        let _ = parse(src).expect("Coulomb debe parsear");
    }

    #[test]
    fn hooke_compiles() {
        // F_H = -k·(r − r0), pero `k` y `r0` no son vars del DSL → constantes
        let src = "-100.0 * (r - 1.5)";
        let e = parse(src).expect("Hooke debe parsear");
        // Confirma que el unario al inicio vincula a toda la expresión que sigue
        // sólo hasta el primer factor (luego la mul/sub continúan): −100 · (...).
        assert!(matches!(e, Expr::Bin(BinOp::Mul, _, _)));
    }

    #[test]
    fn arity_mismatch_is_detected() {
        let err = parse("pow(r)").unwrap_err();
        assert!(matches!(
            err,
            ParseError::ArityMismatch { func: Func::Pow, expected: 2, got: 1, .. }
        ));
    }

    #[test]
    fn trailing_garbage_is_rejected() {
        let err = parse("r2 +").unwrap_err();
        assert!(matches!(err, ParseError::UnexpectedEof { .. }));
        let err2 = parse("r2 r").unwrap_err();
        assert!(matches!(err2, ParseError::UnexpectedToken { .. }));
    }

    #[test]
    fn line_comments_are_ignored() {
        let src = "# esto es un comentario\n1 + 2 # cola de línea\n";
        assert_eq!(parse(src).unwrap(), bin(BinOp::Add, n(1.0), n(2.0)));
    }

    /// Los archivos `.tnk` de `examples/` son la especificación viva de D5.
    /// Si alguno deja de parsear+compilar+optimizar es una regresión real.
    #[test]
    fn example_tnk_files_all_compile() {
        use crate::{compile, optimize};
        let cases: &[(&str, &str)] = &[
            ("lj.tnk",      include_str!("../examples/lj.tnk")),
            ("coulomb.tnk", include_str!("../examples/coulomb.tnk")),
            ("hooke.tnk",   include_str!("../examples/hooke.tnk")),
        ];
        for (name, src) in cases {
            let ast = parse(src).unwrap_or_else(|e| panic!("parse {name}: {e:?}"));
            let opt = optimize(ast);
            compile(&opt).unwrap_or_else(|e| panic!("compile {name}: {e:?}"));
        }
    }

    #[test]
    fn empty_input_is_rejected() {
        let err = parse("").unwrap_err();
        assert!(matches!(err, ParseError::UnexpectedEof { .. }));
    }
}
