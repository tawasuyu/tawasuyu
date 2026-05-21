//! `charka-ir` — la representación intermedia del transpilador.
//!
//! Tercera etapa del pipeline COBOL→Rust: toma el [`Program`] de
//! `charka-parser` (cuyo PROCEDURE division es una lista de sentencias
//! con tokens crudos) y produce un [`Ir`] donde cada sentencia ya es un
//! árbol de [`Stmt`] tipados: `MOVE`, `IF`, `PERFORM`, `COMPUTE`, los
//! verbos aritméticos, etc.
//!
//! El modelo de datos (`DATA division`) pasa tal cual — el árbol de
//! [`DataItem`] que ya armó el parser sirve de tabla de símbolos.
//!
//! El lowering es **tolerante y total**: nunca falla. Un verbo que la
//! v1 no parsea se conserva como [`Stmt::Unknown`] con sus tokens
//! crudos, listo para que el codegen (o un humano) lo revise.
//!
//! Alcance v1 — los verbos parseados a fondo: `MOVE`, `DISPLAY`,
//! `ACCEPT`, `COMPUTE` (con expresiones con precedencia), `ADD`,
//! `SUBTRACT`, `MULTIPLY`, `DIVIDE`, `IF`/`ELSE`/`END-IF` (con
//! condiciones `AND`/`OR`/`NOT`), `EVALUATE`/`WHEN`, `STRING`,
//! `UNSTRING`, `INSPECT`, `PERFORM` (fuera de línea, en línea,
//! `TIMES`, `UNTIL`, `VARYING`), `GO TO`, `STOP RUN`, `GOBACK`,
//! `EXIT`, `CONTINUE`. Fuera de alcance: E/S de ficheros, CICS y SQL.

#![forbid(unsafe_code)]

mod ast;
mod cursor;
mod expr;
mod kw;
mod model;
mod stmt;

pub use ast::*;
pub use charka_parser::Program;
pub use model::{resolve_data, ConditionName, DataModel, Field, FieldKind};

use cursor::Cursor;

/// Baja un [`Program`] parseado a la representación intermedia.
pub fn lower(program: &Program) -> Ir {
    let procedures = program
        .paragraphs
        .iter()
        .map(|p| {
            let mut body = Vec::new();
            for sentence in &p.sentences {
                let mut cur = Cursor::new(&sentence.tokens);
                body.extend(stmt::parse_statements(&mut cur, &[]));
            }
            Procedure {
                name: p.name.clone(),
                body,
            }
        })
        .collect();

    Ir {
        program_id: program.program_id.clone().unwrap_or_default(),
        data: program.data.clone(),
        model: model::resolve_data(&program.data),
        procedures,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use charka_lexer::{lex, SourceFormat};

    /// Helper: lexa, parsea y baja a IR un fuente en formato libre.
    fn ir(src: &str) -> Ir {
        let toks = lex(src, SourceFormat::Free).expect("lex OK");
        let program = charka_parser::parse(&toks).expect("parse OK");
        lower(&program)
    }

    /// Helper: el cuerpo del primer (y normalmente único) párrafo.
    fn body(src: &str) -> Vec<Stmt> {
        let prog = format!("PROCEDURE DIVISION.\nMAIN.\n{src}\n");
        ir(&prog).procedures.into_iter().next().unwrap().body
    }

    #[test]
    fn empty_program_lowers_to_default() {
        let got = lower(&charka_parser::parse(&[]).unwrap());
        assert_eq!(got, Ir::default());
    }

    #[test]
    fn move_simple() {
        let b = body("MOVE 5 TO WS-X.");
        assert_eq!(
            b,
            vec![Stmt::Move {
                from: Operand::Num("5".into()),
                to: vec![Operand::Data("WS-X".into())],
            }]
        );
    }

    #[test]
    fn move_to_several_targets() {
        let b = body("MOVE WS-A TO WS-B WS-C.");
        assert_eq!(
            b,
            vec![Stmt::Move {
                from: Operand::Data("WS-A".into()),
                to: vec![Operand::Data("WS-B".into()), Operand::Data("WS-C".into()),],
            }]
        );
    }

    #[test]
    fn indexed_operand_parses_subscript() {
        // `WS-ELEM(WS-I)` — un destino con subíndice de tabla.
        let b = body("MOVE 7 TO WS-ELEM(WS-I).");
        match &b[0] {
            Stmt::Move { to, .. } => match &to[0] {
                Operand::Indexed { name, index } => {
                    assert_eq!(name, "WS-ELEM");
                    assert_eq!(**index, Operand::Data("WS-I".into()));
                }
                other => panic!("se esperaba Indexed, vino {other:?}"),
            },
            other => panic!("se esperaba MOVE, vino {other:?}"),
        }
    }

    #[test]
    fn display_items_and_figurative() {
        let b = body("DISPLAY 'TOTAL: ' WS-TOTAL SPACES.");
        assert_eq!(
            b,
            vec![Stmt::Display {
                items: vec![
                    Operand::Str("TOTAL: ".into()),
                    Operand::Data("WS-TOTAL".into()),
                    Operand::Figurative(Figurative::Space),
                ],
            }]
        );
    }

    #[test]
    fn compute_respects_precedence() {
        let b = body("COMPUTE WS-T = WS-A + WS-B * 2.");
        let expr = match &b[0] {
            Stmt::Compute { targets, expr, .. } => {
                assert_eq!(targets, &vec![Operand::Data("WS-T".into())]);
                expr.clone()
            }
            other => panic!("se esperaba COMPUTE, vino {other:?}"),
        };
        // WS-A + (WS-B * 2)
        assert_eq!(
            expr,
            Expr::Binary {
                op: BinOp::Add,
                lhs: Box::new(Expr::Operand(Operand::Data("WS-A".into()))),
                rhs: Box::new(Expr::Binary {
                    op: BinOp::Mul,
                    lhs: Box::new(Expr::Operand(Operand::Data("WS-B".into()))),
                    rhs: Box::new(Expr::Operand(Operand::Num("2".into()))),
                }),
            }
        );
    }

    #[test]
    fn compute_rounded_flag() {
        let b = body("COMPUTE WS-T ROUNDED = WS-A / 3.");
        assert!(matches!(&b[0], Stmt::Compute { rounded: true, .. }));
    }

    #[test]
    fn add_in_place_and_giving() {
        assert_eq!(
            body("ADD 1 TO WS-CT."),
            vec![Stmt::Add {
                addends: vec![Operand::Num("1".into())],
                to: vec![Operand::Data("WS-CT".into())],
                giving: vec![],
                rounded: false,
            }]
        );
        assert_eq!(
            body("ADD WS-A WS-B GIVING WS-C."),
            vec![Stmt::Add {
                addends: vec![Operand::Data("WS-A".into()), Operand::Data("WS-B".into()),],
                to: vec![],
                giving: vec![Operand::Data("WS-C".into())],
                rounded: false,
            }]
        );
    }

    #[test]
    fn subtract_from_giving() {
        assert_eq!(
            body("SUBTRACT WS-TAX FROM WS-GROSS GIVING WS-NET."),
            vec![Stmt::Subtract {
                amounts: vec![Operand::Data("WS-TAX".into())],
                from: vec![Operand::Data("WS-GROSS".into())],
                giving: vec![Operand::Data("WS-NET".into())],
                rounded: false,
            }]
        );
    }

    #[test]
    fn divide_by_and_into() {
        assert!(matches!(
            &body("DIVIDE WS-A BY WS-B GIVING WS-C.")[0],
            Stmt::Divide { by_form: true, .. }
        ));
        assert!(matches!(
            &body("DIVIDE WS-A INTO WS-B.")[0],
            Stmt::Divide { by_form: false, .. }
        ));
    }

    #[test]
    fn if_else_end_if() {
        let b = body("IF WS-X > 0 DISPLAY 'POS' ELSE DISPLAY 'NEG' END-IF.");
        match &b[0] {
            Stmt::If {
                cond,
                then_branch,
                else_branch,
            } => {
                assert_eq!(
                    cond,
                    &Cond::Compare {
                        lhs: Operand::Data("WS-X".into()),
                        op: CmpOp::Gt,
                        rhs: Operand::Num("0".into()),
                    }
                );
                assert_eq!(then_branch.len(), 1);
                assert_eq!(else_branch.len(), 1);
            }
            other => panic!("se esperaba IF, vino {other:?}"),
        }
    }

    #[test]
    fn if_condition_with_and() {
        let b = body("IF A = 1 AND B = 2 CONTINUE END-IF.");
        match &b[0] {
            Stmt::If { cond, .. } => {
                assert!(matches!(cond, Cond::And(_, _)));
            }
            other => panic!("se esperaba IF, vino {other:?}"),
        }
    }

    #[test]
    fn if_named_condition() {
        // Un dato suelto en la condición es un nombre de condición (88).
        let b = body("IF FLAG-IS-OK MOVE 1 TO X END-IF.");
        match &b[0] {
            Stmt::If { cond, .. } => {
                assert_eq!(cond, &Cond::Named("FLAG-IS-OK".into()));
            }
            other => panic!("se esperaba IF, vino {other:?}"),
        }
    }

    #[test]
    fn perform_paragraph_and_times() {
        assert_eq!(
            body("PERFORM SUB-PARA."),
            vec![Stmt::Perform(Perform {
                target: PerformTarget::Paragraph {
                    name: "SUB-PARA".into(),
                    thru: None,
                },
                control: PerformControl::Once,
            })]
        );
        assert_eq!(
            body("PERFORM SUB-PARA 3 TIMES."),
            vec![Stmt::Perform(Perform {
                target: PerformTarget::Paragraph {
                    name: "SUB-PARA".into(),
                    thru: None,
                },
                control: PerformControl::Times(Operand::Num("3".into())),
            })]
        );
    }

    #[test]
    fn perform_inline_until() {
        let b = body("PERFORM UNTIL WS-DONE = 1 ADD 1 TO WS-CT END-PERFORM.");
        match &b[0] {
            Stmt::Perform(p) => {
                assert!(matches!(p.control, PerformControl::Until(_)));
                match &p.target {
                    PerformTarget::Inline(body) => assert_eq!(body.len(), 1),
                    other => panic!("se esperaba cuerpo en línea, vino {other:?}"),
                }
            }
            other => panic!("se esperaba PERFORM, vino {other:?}"),
        }
    }

    #[test]
    fn perform_varying_inline() {
        let b = body(
            "PERFORM VARYING WS-I FROM 1 BY 2 UNTIL WS-I > 9 \
             CONTINUE END-PERFORM.",
        );
        match &b[0] {
            Stmt::Perform(p) => match &p.control {
                PerformControl::Varying {
                    var,
                    from,
                    by,
                    until,
                } => {
                    assert_eq!(var, "WS-I");
                    assert_eq!(from, &Operand::Num("1".into()));
                    assert_eq!(by, &Operand::Num("2".into()));
                    assert!(matches!(until, Cond::Compare { .. }));
                }
                other => panic!("se esperaba Varying, vino {other:?}"),
            },
            other => panic!("se esperaba PERFORM, vino {other:?}"),
        }
    }

    #[test]
    fn evaluate_parses_whens_and_other() {
        let b = body(
            "EVALUATE WS-X \
             WHEN 1 DISPLAY 'A' \
             WHEN 2 WHEN 3 DISPLAY 'B' \
             WHEN OTHER DISPLAY 'C' \
             END-EVALUATE.",
        );
        match &b[0] {
            Stmt::Evaluate {
                subject,
                whens,
                other,
            } => {
                assert_eq!(subject, &Operand::Data("WS-X".into()));
                assert_eq!(whens.len(), 2);
                assert_eq!(whens[0].values, vec![Operand::Num("1".into())]);
                assert_eq!(
                    whens[1].values,
                    vec![Operand::Num("2".into()), Operand::Num("3".into())]
                );
                assert_eq!(other.len(), 1);
            }
            other => panic!("se esperaba EVALUATE, vino {other:?}"),
        }
    }

    #[test]
    fn string_and_unstring_parse() {
        let b = body("STRING WS-A WS-B DELIMITED BY SIZE INTO WS-OUT END-STRING.");
        match &b[0] {
            Stmt::StringConcat { sources, into } => {
                assert_eq!(sources.len(), 2);
                assert_eq!(into, &Operand::Data("WS-OUT".into()));
            }
            other => panic!("se esperaba STRING, vino {other:?}"),
        }
        let b = body("UNSTRING WS-SRC DELIMITED BY ',' INTO WS-A WS-B END-UNSTRING.");
        match &b[0] {
            Stmt::Unstring {
                source,
                delimiter,
                into,
            } => {
                assert_eq!(source, &Operand::Data("WS-SRC".into()));
                assert_eq!(delimiter, &Operand::Str(",".into()));
                assert_eq!(into.len(), 2);
            }
            other => panic!("se esperaba UNSTRING, vino {other:?}"),
        }
    }

    #[test]
    fn inspect_tallying_and_replacing_parse() {
        let b = body("INSPECT WS-T TALLYING WS-N FOR ALL 'A'.");
        match &b[0] {
            Stmt::Inspect {
                target,
                op: InspectOp::TallyingForAll { counter, search },
            } => {
                assert_eq!(target, &Operand::Data("WS-T".into()));
                assert_eq!(counter, &Operand::Data("WS-N".into()));
                assert_eq!(search, &Operand::Str("A".into()));
            }
            other => panic!("se esperaba INSPECT TALLYING, vino {other:?}"),
        }
        let b = body("INSPECT WS-T REPLACING ALL 'A' BY 'O'.");
        match &b[0] {
            Stmt::Inspect {
                op: InspectOp::ReplacingAll { from, to },
                ..
            } => {
                assert_eq!(from, &Operand::Str("A".into()));
                assert_eq!(to, &Operand::Str("O".into()));
            }
            other => panic!("se esperaba INSPECT REPLACING, vino {other:?}"),
        }
    }

    #[test]
    fn several_statements_in_one_sentence() {
        let b = body("MOVE 1 TO X DISPLAY X STOP RUN.");
        assert_eq!(b.len(), 3);
        assert!(matches!(b[0], Stmt::Move { .. }));
        assert!(matches!(b[1], Stmt::Display { .. }));
        assert_eq!(b[2], Stmt::StopRun);
    }

    #[test]
    fn unrecognized_verb_becomes_unknown() {
        let b = body("INITIALIZE WS-X WS-Y.");
        match &b[0] {
            Stmt::Unknown { verb, tokens } => {
                assert_eq!(verb, "INITIALIZE");
                assert!(!tokens.is_empty());
            }
            other => panic!("se esperaba Unknown, vino {other:?}"),
        }
    }

    #[test]
    fn full_program_lowers() {
        let program = ir("IDENTIFICATION DIVISION.\n\
             PROGRAM-ID. ADDER.\n\
             DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-A PIC 9(3) VALUE 10.\n\
             01 WS-T PIC 9(4).\n\
             PROCEDURE DIVISION.\n\
             MAIN-PARA.\n\
                 COMPUTE WS-T = WS-A + 5.\n\
                 DISPLAY WS-T.\n\
                 STOP RUN.\n");
        assert_eq!(program.program_id, "ADDER");
        assert_eq!(program.data.len(), 2);
        assert_eq!(program.procedures.len(), 1);
        assert_eq!(program.procedures[0].name, "MAIN-PARA");
        assert_eq!(program.procedures[0].body.len(), 3);
    }
}
