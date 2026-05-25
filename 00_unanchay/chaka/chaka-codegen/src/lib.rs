//! `chaka_app-codegen` — emisión de Rust desde el IR del transpilador.
//!
//! Etapa final del pipeline COBOL→Rust: toma el [`Ir`] de `chaka_app-ir`
//! y produce un fuente Rust (un `String`) que, compilado contra
//! `chaka_app-runtime`, ejecuta la lógica del programa COBOL original.
//!
//! La forma del código emitido:
//!
//! - Un `struct Program` con un campo por cada dato elemental — `Num`
//!   para los numéricos, `Text` para los alfanuméricos.
//! - `Program::new()` inicializa los campos desde sus cláusulas `VALUE`.
//! - Un método `p_<párrafo>(&mut self)` por cada párrafo del PROCEDURE.
//! - `run()` los encadena en orden (el «caer» de COBOL); `main()`
//!   construye el `Program` y lo corre.
//!
//! Es **tolerante**: lo que no sabe transpilar (un `Stmt::Unknown`, un
//! dato sin resolver, `**`) se emite como un comentario `// chaka_app:` —
//! el código generado siempre compila.
//!
//! Alcance v1 — fuera: grupos como campo propio, `REDEFINES`,
//! `OCCURS`/tablas, `PERFORM ... THRU` como rango, E/S de ficheros,
//! `EVALUATE`, CICS y SQL embebido.

#![forbid(unsafe_code)]

mod emit;
mod expr;
mod stmt;
mod sym;

use chaka_ir::Ir;

use emit::Emitter;
use expr::rust_str;
use stmt::emit_stmt;
use sym::{Field, FieldKind, Symbols};

/// Transpila un [`Ir`] a un fuente Rust completo (un `main.rs`).
pub fn generate(ir: &Ir) -> String {
    let sym = Symbols::build(ir);
    let mut em = Emitter::new();
    emit_header(&mut em);
    emit_struct(&mut em, &sym);
    emit_impl(&mut em, &sym, ir);
    emit_main(&mut em);
    em.finish()
}

/// El preámbulo: doc, `allow`s, el `use` del runtime y el helper `dec`.
fn emit_header(em: &mut Emitter) {
    em.line("//! Generado por chaka_app — transpilador COBOL → Rust.");
    em.line("//! No editar a mano: regenerar desde el fuente COBOL.");
    em.blank();
    em.line(
        "#![allow(dead_code, unused_mut, unused_variables, unused_parens, \
unreachable_code, clippy::all)]",
    );
    em.blank();
    em.line("use chaka_runtime::*;");
    em.blank();
    em.line("/// Construye un `Decimal` desde un literal numérico COBOL.");
    em.line("fn dec(s: &str) -> Decimal {");
    em.line("    Decimal::parse(s).expect(\"chaka_app: literal numérico inválido\")");
    em.line("}");
    em.blank();
}

/// El `struct Program` con un campo por dato elemental.
fn emit_struct(em: &mut Emitter, sym: &Symbols) {
    em.line("/// El estado del programa: un campo por cada dato elemental.");
    em.line("struct Program {");
    em.indent();
    for f in &sym.fields {
        let elem = match f.kind {
            FieldKind::Num { .. } => "Num",
            FieldKind::Text { .. } => "Text",
        };
        let ty = match f.occurs {
            None => elem.to_string(),
            Some(_) => format!("Vec<{elem}>"),
        };
        em.line(&format!("{}: {ty},", f.ident));
    }
    for fs in &sym.files {
        em.line(&format!("{}: CobFile,", fs.ident));
    }
    em.dedent();
    em.line("}");
    em.blank();
}

/// El bloque `impl Program`: `new`, los párrafos y `run`.
fn emit_impl(em: &mut Emitter, sym: &Symbols, ir: &Ir) {
    em.line("impl Program {");
    em.indent();

    // new()
    em.line("fn new() -> Self {");
    em.indent();
    em.line("Self {");
    em.indent();
    for f in &sym.fields {
        em.line(&format!("{}: {},", f.ident, field_init(f)));
    }
    for fs in &sym.files {
        em.line(&format!(
            "{}: CobFile::new({}),",
            fs.ident,
            rust_str(&fs.path)
        ));
    }
    em.dedent();
    em.line("}");
    em.dedent();
    em.line("}");
    em.blank();

    // Un método por párrafo (en paralelo con `sym.paragraphs`).
    for (i, proc) in ir.procedures.iter().enumerate() {
        em.line(&format!("fn {}(&mut self) {{", sym.paragraphs[i].1));
        em.indent();
        for s in &proc.body {
            emit_stmt(em, sym, s);
        }
        em.dedent();
        em.line("}");
        em.blank();
    }

    // run() — encadena los párrafos en orden.
    em.line("fn run(&mut self) {");
    em.indent();
    if sym.paragraphs.is_empty() {
        em.line("// programa sin PROCEDURE division");
    }
    for (_, method) in &sym.paragraphs {
        em.line(&format!("self.{method}();"));
    }
    em.dedent();
    em.line("}");

    em.dedent();
    em.line("}");
    em.blank();
}

/// El `fn main`.
fn emit_main(em: &mut Emitter) {
    em.line("fn main() {");
    em.indent();
    em.line("Program::new().run();");
    em.dedent();
    em.line("}");
}

/// El inicializador de un campo, a partir de su `VALUE` ya
/// normalizado por `chaka_app-ir`. Una tabla (`OCCURS n`) se inicializa
/// como un `Vec` de `n` copias del valor inicial.
fn field_init(f: &Field) -> String {
    let scalar = match &f.kind {
        FieldKind::Num { int, frac, signed } => format!(
            "Num::with_value(Picture::new({int}, {frac}, {signed}), {})",
            rust_str(&f.init)
        ),
        FieldKind::Text { len } => {
            format!("Text::with_value({len}, {})", rust_str(&f.init))
        }
    };
    match f.occurs {
        None => scalar,
        Some(n) => format!("vec![{scalar}; {n}]"),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Helper: lexa, parsea, baja a IR y transpila un fuente COBOL.
    fn gen(src: &str) -> String {
        let toks = chaka_lexer::lex(src, chaka_lexer::SourceFormat::Free).expect("lex");
        let prog = chaka_parser::parse(&toks).expect("parse");
        let ir = chaka_ir::lower(&prog);
        generate(&ir)
    }

    /// Un programa COBOL de demostración, razonablemente completo.
    const DEMO: &str = "IDENTIFICATION DIVISION.\n\
         PROGRAM-ID. DEMO.\n\
         DATA DIVISION.\n\
         WORKING-STORAGE SECTION.\n\
         01 WS-A    PIC 9(3) VALUE 10.\n\
         01 WS-B    PIC 9(3).\n\
         01 WS-NAME PIC X(8) VALUE 'BOB'.\n\
         PROCEDURE DIVISION.\n\
         MAIN-PARA.\n\
             MOVE 5 TO WS-B.\n\
             COMPUTE WS-B = WS-A + WS-B.\n\
             DISPLAY 'B=' WS-B.\n\
             IF WS-B > 0 DISPLAY 'POS' END-IF.\n\
             PERFORM SUB-PARA.\n\
             STOP RUN.\n\
         SUB-PARA.\n\
             DISPLAY WS-NAME.\n";

    #[test]
    fn header_and_main_are_emitted() {
        let out = gen(DEMO);
        assert!(out.contains("use chaka_runtime::*;"));
        assert!(out.contains("fn dec(s: &str) -> Decimal {"));
        assert!(out.contains("fn main() {"));
        assert!(out.contains("Program::new().run();"));
    }

    #[test]
    fn numeric_field_becomes_num() {
        let out = gen(DEMO);
        assert!(out.contains("ws_a: Num,"));
        assert!(out.contains("Num::with_value(Picture::new(3, 0, false), \"10\")"));
    }

    #[test]
    fn alphanumeric_field_becomes_text() {
        let out = gen(DEMO);
        assert!(out.contains("ws_name: Text,"));
        assert!(out.contains("Text::with_value(8, \"BOB\")"));
    }

    #[test]
    fn move_emits_a_store() {
        assert!(gen(DEMO).contains("self.ws_b.store(dec(\"5\"));"));
    }

    #[test]
    fn compute_emits_the_expression() {
        let out = gen(DEMO);
        assert!(out.contains("self.ws_b.store((self.ws_a.value()).add(&(self.ws_b.value())));"));
    }

    #[test]
    fn display_emits_a_println() {
        let out = gen(DEMO);
        assert!(out.contains("println!(\"{}{}\", \"B=\", self.ws_b.display());"));
    }

    #[test]
    fn if_emits_a_rust_if() {
        let out = gen(DEMO);
        assert!(out.contains("if (self.ws_b.value()) > (dec(\"0\")) {"));
    }

    #[test]
    fn paragraphs_become_methods_and_run_chains_them() {
        let out = gen(DEMO);
        assert!(out.contains("fn p_main_para(&mut self) {"));
        assert!(out.contains("fn p_sub_para(&mut self) {"));
        assert!(out.contains("fn run(&mut self) {"));
        assert!(out.contains("self.p_main_para();"));
        assert!(out.contains("self.p_sub_para();"));
    }

    #[test]
    fn perform_calls_the_paragraph_method() {
        assert!(gen(DEMO).contains("self.p_sub_para();"));
    }

    #[test]
    fn stop_run_exits() {
        assert!(gen(DEMO).contains("std::process::exit(0);"));
    }

    #[test]
    fn unknown_verb_becomes_a_comment() {
        let out = gen("PROCEDURE DIVISION.\n\
             MAIN.\n\
                 CALL 'SUBPROG'.\n");
        assert!(out.contains("// chaka_app: verbo no transpilado — CALL"));
    }

    #[test]
    fn add_giving_emits_a_sum() {
        let out = gen("DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 A PIC 9(3).\n\
             01 B PIC 9(3).\n\
             01 C PIC 9(3).\n\
             PROCEDURE DIVISION.\n\
             MAIN.\n\
                 ADD A B GIVING C.\n");
        assert!(out.contains("self.c.store((self.a.value()).add(&(self.b.value())));"));
    }

    #[test]
    fn perform_times_emits_a_loop() {
        let out = gen("PROCEDURE DIVISION.\n\
             MAIN.\n\
                 PERFORM SUB 3 TIMES.\n\
             SUB.\n\
                 CONTINUE.\n");
        assert!(out.contains("for _ in 0..3usize {"));
    }

    #[test]
    fn perform_varying_emits_init_loop_and_increment() {
        let out = gen("DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-I PIC 9(2).\n\
             01 WS-N PIC 9(3).\n\
             PROCEDURE DIVISION.\n\
             MAIN.\n\
                 PERFORM VARYING WS-I FROM 1 BY 1 UNTIL WS-I > 5\n\
                     ADD 1 TO WS-N\n\
                 END-PERFORM.\n");
        assert!(out.contains("self.ws_i.store(dec(\"1\"));"));
        assert!(out.contains("while !((self.ws_i.value()) > (dec(\"5\"))) {"));
        assert!(out.contains("self.ws_i.store(self.ws_i.value().add(&(dec(\"1\"))));"));
    }

    #[test]
    fn evaluate_emits_an_if_else_chain() {
        let out = gen("DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-X PIC 9(1).\n\
             PROCEDURE DIVISION.\n\
             MAIN.\n\
                 EVALUATE WS-X\n\
                     WHEN 1 DISPLAY 'UNO'\n\
                     WHEN OTHER DISPLAY 'OTRO'\n\
                 END-EVALUATE.\n");
        assert!(out.contains("if ((self.ws_x.value()) == (dec(\"1\"))) {"));
        assert!(out.contains("} else {"));
    }

    #[test]
    fn level_88_condition_resolves_to_a_comparison() {
        let out = gen("DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-FLAG PIC X VALUE 'N'.\n\
                88 ES-SI VALUE 'Y'.\n\
             PROCEDURE DIVISION.\n\
             MAIN.\n\
                 IF ES-SI DISPLAY 'SI' END-IF.\n");
        // ES-SI equivale a `WS-FLAG = 'Y'` (comparación de texto).
        assert!(out.contains("cobol_text_cmp(self.ws_flag.display().as_str(), \"Y\").is_eq()"));
    }

    #[test]
    fn occurs_emits_a_vec_field_and_indexed_access() {
        let out = gen("DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-T.\n\
                05 WS-E PIC 9(3) OCCURS 4 TIMES.\n\
             01 WS-I PIC 9(1).\n\
             PROCEDURE DIVISION.\n\
             MAIN.\n\
                 MOVE 7 TO WS-E(WS-I).\n");
        assert!(out.contains("ws_e: Vec<Num>,"));
        assert!(out.contains("; 4]"));
        assert!(out.contains("self.ws_e["));
        assert!(out.contains(".saturating_sub(1)]"));
    }

    #[test]
    fn string_concatenates_and_unstring_splits() {
        let out = gen("DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-A PIC X(4).\n\
             01 WS-B PIC X(4).\n\
             01 WS-OUT PIC X(10).\n\
             01 WS-SRC PIC X(10).\n\
             PROCEDURE DIVISION.\n\
             MAIN.\n\
                 STRING WS-A WS-B DELIMITED BY SIZE INTO WS-OUT END-STRING.\n\
                 UNSTRING WS-SRC DELIMITED BY ',' INTO WS-A WS-B END-UNSTRING.\n");
        assert!(out.contains("self.ws_out.store(&format!("));
        assert!(out.contains("__src.split(__delim.as_str())"));
        assert!(out.contains("__it.next().unwrap_or(\"\")"));
    }

    #[test]
    fn inspect_emits_tally_and_replace() {
        let out = gen("DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-T PIC X(10).\n\
             01 WS-N PIC 9(3).\n\
             PROCEDURE DIVISION.\n\
             MAIN.\n\
                 INSPECT WS-T TALLYING WS-N FOR ALL 'X'.\n\
                 INSPECT WS-T REPLACING ALL 'X' BY 'Y'.\n");
        assert!(out.contains(".matches("));
        assert!(out.contains("Decimal::from_integer(__n)"));
        assert!(out.contains(".replace("));
    }

    #[test]
    fn evaluate_true_and_range_emit() {
        let out = gen("DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-X PIC 9(3).\n\
             PROCEDURE DIVISION.\n\
             MAIN.\n\
                 EVALUATE WS-X WHEN 1 THRU 9 DISPLAY 'R' END-EVALUATE.\n\
                 EVALUATE TRUE WHEN WS-X > 5 DISPLAY 'T' END-EVALUATE.\n");
        assert!(out.contains(">= (dec(\"1\"))"));
        assert!(out.contains("<= (dec(\"9\"))"));
        assert!(out.contains("> (dec(\"5\"))"));
    }

    #[test]
    fn initialize_resets_group_members() {
        let out = gen("DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-REC.\n\
                05 WS-A PIC 9(3).\n\
                05 WS-B PIC X(4).\n\
             PROCEDURE DIVISION.\n\
             MAIN.\n\
                 INITIALIZE WS-REC.\n");
        assert!(out.contains("self.ws_a.store(Decimal::zero());"));
        assert!(out.contains("self.ws_b.fill(' ');"));
    }

    #[test]
    fn set_to_true_moves_the_88_value() {
        let out = gen("DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-F PIC X VALUE 'N'.\n\
                88 LISTO VALUE 'S'.\n\
             PROCEDURE DIVISION.\n\
             MAIN.\n\
                 SET LISTO TO TRUE.\n");
        assert!(out.contains("self.ws_f.store(\"S\");"));
    }

    #[test]
    fn perform_thru_calls_the_paragraph_range() {
        let out = gen("PROCEDURE DIVISION.\n\
             MAIN.\n\
                 PERFORM A THRU C.\n\
             A.\n\
                 DISPLAY 'A'.\n\
             B.\n\
                 DISPLAY 'B'.\n\
             C.\n\
                 DISPLAY 'C'.\n");
        // `PERFORM A THRU C` emite la llamada a p_b dentro de p_main,
        // además de la que hace run() — de ahí >= 2 apariciones.
        assert!(out.matches("self.p_b();").count() >= 2);
    }

    #[test]
    fn file_io_emits_open_read_write_close() {
        let out = gen("ENVIRONMENT DIVISION.\n\
             INPUT-OUTPUT SECTION.\n\
             FILE-CONTROL.\n\
                 SELECT ARCH ASSIGN TO 'x.dat'.\n\
             DATA DIVISION.\n\
             FILE SECTION.\n\
             FD ARCH.\n\
             01 REG PIC X(10).\n\
             PROCEDURE DIVISION.\n\
             MAIN.\n\
                 OPEN OUTPUT ARCH.\n\
                 WRITE REG FROM 'HI'.\n\
                 CLOSE ARCH.\n\
                 OPEN INPUT ARCH.\n\
                 READ ARCH AT END CONTINUE END-READ.\n\
                 CLOSE ARCH.\n");
        assert!(out.contains("file_arch: CobFile,"));
        assert!(out.contains("CobFile::new(\"x.dat\")"));
        assert!(out.contains("self.file_arch.open_output();"));
        assert!(out.contains("self.file_arch.write("));
        assert!(out.contains("self.file_arch.read()"));
        assert!(out.contains("self.file_arch.close();"));
    }

    #[test]
    fn edited_field_is_text_and_move_formats_it() {
        let out = gen("DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-N PIC 9(5).\n\
             01 WS-E PIC Z,ZZ9.99.\n\
             PROCEDURE DIVISION.\n\
             MAIN.\n\
                 MOVE WS-N TO WS-E.\n");
        // El campo de edición se materializa como texto de presentación.
        assert!(out.contains("ws_e: Text,"));
        // El MOVE pasa por `format_edited` con la PICTURE de edición.
        assert!(out.contains("self.ws_e.store(&format_edited(self.ws_n.value(), \"Z,ZZ9.99\"));"));
    }

    #[test]
    fn empty_program_still_compiles_shape() {
        let out = gen("");
        assert!(out.contains("struct Program {"));
        assert!(out.contains("fn main() {"));
        assert!(out.contains("fn run(&mut self) {"));
    }
}
