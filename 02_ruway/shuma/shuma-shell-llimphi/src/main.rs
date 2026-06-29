//! `shuma-shell-llimphi` (bin) — entrypoint fino.
//!
//! Toda la lógica (Model/update/view/App, sesiones, chrome) vive en la **lib**
//! homónima para que sea un frontend sobre core agnóstico (Regla 2) y la pueda
//! hospedar también pata. Este bin sólo arranca la app de ventana.

#![forbid(unsafe_code)]

fn main() {
    bitacora::abrir("shuma");
    let args: Vec<String> = std::env::args().skip(1).collect();

    // `-e/--exec <cmd…>`: corre ese comando como primer bloque al arrancar
    // (estilo `xterm -e`). Lo consume el resto de argv. Lo usa, p. ej., el
    // launcher de apps WASM (`llimphi-wasm-open`) para mostrar la salida de una
    // app WASI de consola DENTRO de shuma en vez de en una ventana de consola
    // propia. Se pasa por env (mismo patrón que `SHUMA_DOCK`), que `init` lee.
    if let Some(pos) = args.iter().position(|a| a == "-e" || a == "--exec") {
        let cmd = shell_join(&args[pos + 1..]);
        if !cmd.is_empty() {
            std::env::set_var("SHUMA_EXEC", cmd);
        }
    }

    // `--dock` arranca shuma como barra wlr-layer-shell (modo dock); sin flag,
    // como ventana normal.
    if args.iter().any(|a| a == "--dock") {
        shuma_shell_llimphi::run_dock();
    } else {
        shuma_shell_llimphi::run();
    }
}

/// Une `args` en una línea de shell, citando con comillas simples los que
/// tengan espacios o caracteres especiales (escapando comillas simples
/// internas con el truco `'\''`). Así `-e prog --flag "un arg"` se reconstruye
/// como una línea ejecutable sin romperse por los espacios.
fn shell_join(args: &[String]) -> String {
    fn quote(a: &str) -> String {
        if !a.is_empty()
            && a.bytes()
                .all(|b| b.is_ascii_alphanumeric() || b"-_./=:@%+".contains(&b))
        {
            a.to_string()
        } else {
            format!("'{}'", a.replace('\'', "'\\''"))
        }
    }
    args.iter().map(|a| quote(a)).collect::<Vec<_>>().join(" ")
}
