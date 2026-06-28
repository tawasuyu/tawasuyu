//! Demo **en metal**: abre el micrófono real y escucha manos-libres contra el
//! STT mock. Imprime cada evento de escucha. Decí «shuma» y después algo.
//!
//! No se certifica por CI (necesita un micrófono y tu voz); es la verificación
//! manual del driver `microfono`. El lazo en sí está testeado aparte, sin
//! hardware (ver `lazo::tests`).
//!
//! ```sh
//! cargo run -p rimay-voz-host --example escuchar_microfono
//! ```
//!
//! Nota: el STT es mock (transcribe siempre `"shuma"`), así que vas a ver
//! `Desperto` ante cualquier utterance. Para dictado real, cableá un
//! `Transcriptor` de daemon/nube en lugar de `stt_mock()`.

#[tokio::main]
async fn main() {
    let stt = rimay_voz::stt_mock();

    let (_guardia, mut eventos) = match rimay_voz_host::escuchar(stt) {
        Ok(par) => par,
        Err(e) => {
            eprintln!("no se pudo abrir el micrófono: {e}");
            return;
        }
    };

    println!("escuchando… (hablá; Ctrl-C para salir)\n");
    while let Some(ev) = eventos.recv().await {
        println!("· {ev:?}");
    }
}
