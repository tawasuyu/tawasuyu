//! Test diferencial: cada fixture del corpus pasa por el pipeline real
//! del transpilador (lexer → parser → IR → `chaka-codegen::generate`),
//! se compila con `cargo` contra `chaka-runtime`, se ejecuta, y su
//! stdout se compara contra el `.expected` verificado a mano (mismo
//! convenio `trim_end` que los tests sombra de `chaka-shadow`).
//!
//! Esto cierra el lazo prometido en el README: «si la sombra y el
//! transpilado produjeran salidas distintas, eso delataría un bug del
//! codegen». Hasta ahora sólo se testeaba la sombra.
//!
//! Marcado `#[ignore]` porque lanza `cargo build` por cada fixture y
//! tarda decenas de segundos en frío. Correr explícitamente con:
//!
//! ```sh
//! cargo test -p chaka-app --test corpus_e2e --release -- --ignored
//! ```
//!
//! Los workdirs persisten en `$TMPDIR/chaka-e2e/<fixture>/` para que
//! corridas sucesivas reutilicen la caché incremental de cargo.

use std::path::{Path, PathBuf};
use std::process::Command;

fn corpus_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../corpus")
}

fn runtime_dir() -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../chaka-runtime")
        .canonicalize()
        .expect("chaka-runtime existe y es canonicalizable")
}

/// Siembra los recursos externos que algunos fixtures esperan en /tmp.
fn seed_external_inputs(name: &str, corpus: &Path) {
    if name == "24-copy" {
        let src = corpus.join("24-copy.cpy");
        std::fs::copy(&src, "/tmp/chaka-corpus-24.cpy").expect("sembrar copybook 24");
    }
}

fn run_e2e(name: &str) {
    let corpus = corpus_dir();
    let source = std::fs::read_to_string(corpus.join(format!("{name}.cob")))
        .expect("leer fuente COBOL");
    let expected = std::fs::read_to_string(corpus.join(format!("{name}.expected")))
        .expect("leer salida esperada");

    seed_external_inputs(name, &corpus);

    let tokens =
        chaka_lexer::lex_with_base(&source, chaka_lexer::SourceFormat::Free, Some(&corpus))
            .expect("lex");
    let program = chaka_parser::parse(&tokens).expect("parse");
    let ir = chaka_ir::lower(&program);
    let rust = chaka_codegen::generate(&ir);

    let crate_name = format!("chaka_e2e_{}", name.replace('-', "_"));
    let workdir = std::env::temp_dir().join("chaka-e2e").join(name);
    std::fs::create_dir_all(workdir.join("src")).expect("crear workdir");
    std::fs::write(workdir.join("src/main.rs"), &rust).expect("escribir main.rs");

    let runtime = runtime_dir();
    let toml = format!(
        "[package]\n\
         name = \"{crate_name}\"\n\
         version = \"0.1.0\"\n\
         edition = \"2021\"\n\
         \n\
         [[bin]]\n\
         name = \"{crate_name}\"\n\
         path = \"src/main.rs\"\n\
         \n\
         [dependencies]\n\
         chaka-runtime = {{ path = \"{}\" }}\n\
         \n\
         [workspace]\n",
        runtime.display()
    );
    std::fs::write(workdir.join("Cargo.toml"), toml).expect("escribir Cargo.toml");

    let cargo = std::env::var("CARGO").unwrap_or_else(|_| "cargo".into());
    let build = Command::new(&cargo)
        .args(["build", "--quiet", "--release", "--manifest-path"])
        .arg(workdir.join("Cargo.toml"))
        .output()
        .expect("cargo build se ejecuta");
    assert!(
        build.status.success(),
        "cargo build falló para {name}:\n{}",
        String::from_utf8_lossy(&build.stderr)
    );

    let bin = workdir.join("target/release").join(&crate_name);
    let run = Command::new(&bin).output().expect("ejecutar binario");
    assert!(
        run.status.success(),
        "el binario falló para {name}:\n{}",
        String::from_utf8_lossy(&run.stderr)
    );

    let got_raw = String::from_utf8(run.stdout).expect("stdout es UTF-8");
    let got: Vec<&str> = got_raw.lines().map(|l| l.trim_end()).collect();
    let want: Vec<&str> = expected.lines().map(|l| l.trim_end()).collect();
    assert_eq!(
        got, want,
        "el transpilado de {name} difiere de la salida esperada"
    );
}

macro_rules! e2e {
    ($test:ident, $fixture:literal) => {
        #[test]
        #[ignore]
        fn $test() {
            run_e2e($fixture);
        }
    };
}

e2e!(corpus_01_hola, "01-hola");
e2e!(corpus_02_aritmetica, "02-aritmetica");
e2e!(corpus_03_condicional, "03-condicional");
e2e!(corpus_04_bucle, "04-bucle");
e2e!(corpus_05_factorial, "05-factorial");
e2e!(corpus_06_nomina, "06-nomina");
e2e!(corpus_07_clasificar, "07-clasificar");
e2e!(corpus_08_varying, "08-varying");
e2e!(corpus_09_evaluar, "09-evaluar");
e2e!(corpus_10_condicion, "10-condicion");
e2e!(corpus_11_tabla, "11-tabla");
e2e!(corpus_12_cadenas, "12-cadenas");
e2e!(corpus_13_inspeccion, "13-inspeccion");
e2e!(corpus_14_clasifica, "14-clasifica");
e2e!(corpus_15_resetear, "15-resetear");
e2e!(corpus_16_bandera, "16-bandera");
e2e!(corpus_17_rangopar, "17-rangopar");
e2e!(corpus_18_fichero, "18-fichero");
e2e!(corpus_19_reporte, "19-reporte");
e2e!(corpus_20_call, "20-call");
e2e!(corpus_21_search, "21-search");
e2e!(corpus_22_sort, "22-sort");
e2e!(corpus_23_fileops, "23-fileops");
e2e!(corpus_24_copy, "24-copy");
e2e!(corpus_25_inspect_set, "25-inspect-set");
