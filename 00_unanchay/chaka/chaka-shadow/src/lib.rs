//! `chaka-shadow` — el validador en sombra del transpilador.
//!
//! Certifica que el pipeline de chaka (lexer → parser → IR → codegen)
//! preserva la semántica del programa COBOL original. Lo hace con una
//! **ejecución sombra**: un intérprete que corre el [`Ir`] directamente
//! sobre los tipos de `chaka-runtime`, sin compilar nada.
//!
//! El intérprete es una segunda ruta de ejecución, independiente del
//! código que emite `chaka-codegen`. Si la sombra y el transpilado
//! produjeran salidas distintas, eso delataría un bug del codegen.
//!
//! - [`interpret`] — ejecuta un `Ir` y devuelve su salida.
//! - [`run_source`] — el pipeline completo, de fuente COBOL a salida.
//!
//! La referencia contra la que se comparan los resultados es, en la
//! v1, un conjunto de salidas esperadas verificadas a mano (el corpus
//! en `crates/modules/chaka/corpus/`). Cuando haya un GnuCOBOL
//! disponible, un modo futuro podrá diferenciar contra el compilador
//! de COBOL real — la validación «original vs transpilado» plena.
//!
//! El intérprete tiene un tope de pasos: un bucle que no termina se
//! corta con [`Halt::StepLimit`] en vez de colgarse.

#![forbid(unsafe_code)]

pub mod cobc;
mod field;
mod interp;

use chaka_ir::Ir;
use interp::Machine;

/// Cómo terminó una ejecución sombra.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Halt {
    /// Cayó por el final del PROCEDURE division.
    Normal,
    /// Un `STOP RUN` o `GOBACK`.
    StopRun,
    /// Se agotó el tope de pasos (un bucle que no termina).
    StepLimit,
}

/// El resultado de una ejecución sombra.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Outcome {
    /// Las líneas que el programa emitió por `DISPLAY`.
    pub lines: Vec<String>,
    /// Cómo terminó.
    pub halt: Halt,
}

/// Falla del pipeline previo al intérprete.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
pub enum ShadowError {
    #[error("error de léxico: {0}")]
    Lex(#[from] chaka_lexer::LexError),
    #[error("error de parseo: {0}")]
    Parse(#[from] chaka_parser::ParseError),
}

/// Ejecuta un [`Ir`] en sombra y captura su salida.
pub fn interpret(ir: &Ir) -> Outcome {
    let mut machine = Machine::new(ir);
    machine.run();
    let halt = if machine.step_limit_hit {
        Halt::StepLimit
    } else if machine.stopped {
        Halt::StopRun
    } else {
        Halt::Normal
    };
    Outcome {
        lines: machine.output,
        halt,
    }
}

/// Corre el pipeline completo: fuente COBOL (format libre) → salida.
pub fn run_source(cobol: &str) -> Result<Outcome, ShadowError> {
    let tokens = chaka_lexer::lex(cobol, chaka_lexer::SourceFormat::Free)?;
    let program = chaka_parser::parse(&tokens)?;
    let ir = chaka_ir::lower(&program);
    Ok(interpret(&ir))
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Verifica un programa del corpus contra su salida esperada. La
    /// comparación ignora los espacios finales de cada línea.
    fn check(cobol: &str, expected: &str) {
        let outcome = run_source(cobol).expect("el pipeline no debe fallar");
        let got: Vec<&str> = outcome.lines.iter().map(|l| l.trim_end()).collect();
        let want: Vec<&str> = expected.lines().map(|l| l.trim_end()).collect();
        assert_eq!(got, want, "salida sombra distinta de la esperada");
    }

    /// Declara un test que corre un programa del corpus.
    macro_rules! corpus_test {
        ($name:ident, $file:literal) => {
            #[test]
            fn $name() {
                check(
                    include_str!(concat!("../../corpus/", $file, ".cob")),
                    include_str!(concat!("../../corpus/", $file, ".expected")),
                );
            }
        };
    }

    corpus_test!(corpus_01_hola, "01-hola");
    corpus_test!(corpus_02_aritmetica, "02-aritmetica");
    corpus_test!(corpus_03_condicional, "03-condicional");
    corpus_test!(corpus_04_bucle, "04-bucle");
    corpus_test!(corpus_05_factorial, "05-factorial");
    corpus_test!(corpus_06_nomina, "06-nomina");
    corpus_test!(corpus_07_clasificar, "07-clasificar");
    corpus_test!(corpus_08_varying, "08-varying");
    corpus_test!(corpus_09_evaluar, "09-evaluar");
    corpus_test!(corpus_10_condicion, "10-condicion");
    corpus_test!(corpus_11_tabla, "11-tabla");
    corpus_test!(corpus_12_cadenas, "12-cadenas");
    corpus_test!(corpus_13_inspeccion, "13-inspeccion");
    corpus_test!(corpus_14_clasifica, "14-clasifica");
    corpus_test!(corpus_15_resetear, "15-resetear");
    corpus_test!(corpus_16_bandera, "16-bandera");
    corpus_test!(corpus_17_rangopar, "17-rangopar");
    corpus_test!(corpus_18_fichero, "18-fichero");
    corpus_test!(corpus_19_reporte, "19-reporte");
    corpus_test!(corpus_20_call, "20-call");
    corpus_test!(corpus_21_search, "21-search");
    corpus_test!(corpus_22_sort, "22-sort");
    corpus_test!(corpus_23_fileops, "23-fileops");
    corpus_test!(corpus_25_inspect_set, "25-inspect-set");
    corpus_test!(corpus_26_indexed, "26-indexed");
    corpus_test!(corpus_27_relative, "27-relative");

    #[test]
    fn corpus_24_copy() {
        // El copybook se siembra en /tmp para que el `COPY '/tmp/...'.`
        // del .cob lo encuentre desde cualquier directorio de trabajo.
        std::fs::write(
            "/tmp/chaka-corpus-24.cpy",
            include_str!("../../corpus/24-copy.cpy"),
        )
        .expect("escribir copybook");
        check(
            include_str!("../../corpus/24-copy.cob"),
            include_str!("../../corpus/24-copy.expected"),
        );
    }

    #[test]
    fn empty_source_runs_clean() {
        let outcome = run_source("").expect("pipeline OK");
        assert!(outcome.lines.is_empty());
        assert_eq!(outcome.halt, Halt::Normal);
    }

    #[test]
    fn stop_run_is_reported() {
        let outcome = run_source("PROCEDURE DIVISION.\nMAIN.\n DISPLAY 'X'.\n STOP RUN.\n")
            .expect("pipeline OK");
        assert_eq!(outcome.lines, vec!["X".to_string()]);
        assert_eq!(outcome.halt, Halt::StopRun);
    }

    #[test]
    fn endless_loop_is_cut_by_the_step_limit() {
        // `PERFORM UNTIL 1 = 0` nunca se cumple — el tope lo corta.
        let outcome = run_source(
            "PROCEDURE DIVISION.\n\
             MAIN.\n\
                 PERFORM UNTIL 1 = 0\n\
                     CONTINUE\n\
                 END-PERFORM.\n",
        )
        .expect("pipeline OK");
        assert_eq!(outcome.halt, Halt::StepLimit);
    }

    #[test]
    fn perform_varying_out_of_line() {
        // `PERFORM CONTAR VARYING ...` — el párrafo es el cuerpo del bucle.
        // WS-I = 1, 3, 5, 7, 9 (FROM 1 BY 2 UNTIL > 9) → 5 iteraciones.
        let outcome = run_source(
            "DATA DIVISION.\n\
             WORKING-STORAGE SECTION.\n\
             01 WS-I PIC 9(2) VALUE 0.\n\
             01 WS-N PIC 9(3) VALUE 0.\n\
             PROCEDURE DIVISION.\n\
             MAIN.\n\
                 PERFORM CONTAR VARYING WS-I FROM 1 BY 2 UNTIL WS-I > 9.\n\
                 DISPLAY WS-N.\n\
                 STOP RUN.\n\
             CONTAR.\n\
                 ADD 1 TO WS-N.\n",
        )
        .expect("pipeline OK");
        assert_eq!(outcome.lines, vec!["005".to_string()]);
    }

    #[test]
    fn lex_error_surfaces() {
        let err = run_source("PROCEDURE DIVISION.\nMAIN.\n DISPLAY 'sin cerrar.\n").unwrap_err();
        assert!(matches!(err, ShadowError::Lex(_)));
    }
}
