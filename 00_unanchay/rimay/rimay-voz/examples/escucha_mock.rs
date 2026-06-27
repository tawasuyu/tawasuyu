//! Demo canónico del lazo de voz manos-libres contra los backends mock.
//!
//! Es el uso de referencia que cualquier host (shuma, mirada…) copia: el VAD/STT
//! del host transcribe fragmentos → la [`Maquina`] decide → ante un dictado, la
//! política de [`lectura`] elige qué se vocaliza y el [`Locutor`] lo sintetiza.
//! Todo determinista, sin micrófono ni modelo — se certifica leyendo la salida.
//!
//! ```sh
//! cargo run -p rimay-voz --example escucha_mock
//! ```

use rimay_voz::{
    debe_leer, ConfigVoz, Evento, Maquina, Reaccion, TipoBloque, Transcriptor, TranscriptorMock,
};

/// Lo que el host capturaría del micrófono, ya como fragmentos con voz. Cada
/// uno simula lo que el STT real (whisper) devolvería para ese audio.
const GUION: &[&str] = &[
    "cargo build --release", // ruido: no es el llamado → ignorado
    "shuma",                 // el llamado solo → despierta
    "listá los archivos",    // dictado
    "<silencio>",            // 8 ticks → se vuelve a dormir
    "shuma abrí cosmos",     // llamado + cola → dicta de una
];

#[tokio::main]
async fn main() {
    let tts = rimay_voz::tts_mock();
    let mut escucha = Maquina::new(ConfigVoz::default());

    println!("== lazo de voz manos-libres (mock) ==\n");

    for frase in GUION {
        if *frase == "<silencio>" {
            // El host marca el paso del tiempo; tras paciencia_dictado re-duerme.
            for _ in 0..8 {
                if let Reaccion::SeDurmio = escucha.avanzar(Evento::Tick) {
                    println!("  · (silencio) → se durmió   [{:?}]", escucha.estado());
                }
            }
            continue;
        }

        // El STT del host transcribe el fragmento (acá: un mock por utterance).
        let stt = TranscriptorMock::con_texto(*frase);
        let audio = rimay_voz::Audio::new(vec![0; 16_000], 16_000);
        let texto = stt.transcribir(&audio).await.unwrap().texto;

        match escucha.avanzar(Evento::Transcript(texto)) {
            Reaccion::Nada => println!("«{frase}» → (ignorado)        [{:?}]", escucha.estado()),
            Reaccion::Desperto => println!("«{frase}» → despertó         [{:?}]", escucha.estado()),
            Reaccion::Dictar(t) => println!("«{frase}» → DICTAR: {t:?}   [{:?}]", escucha.estado()),
            Reaccion::SeDurmio => println!("«{frase}» → se durmió        [{:?}]", escucha.estado()),
        }
    }

    // Lectura discriminada: el agente respondió con prosa + un bloque de código;
    // sólo la prosa se vocaliza.
    println!("\n== lectura discriminada (TTS sólo la prosa) ==");
    for (tipo, contenido) in [
        (TipoBloque::Texto, "Listo, abrí cosmos."),
        (TipoBloque::Codigo, "fn main() { ... }"),
        (TipoBloque::Accion, "abrir-app cosmos"),
    ] {
        if debe_leer(tipo) {
            let audio = tts.sintetizar(contenido).await.unwrap();
            println!("  {tipo:?}: se lee  → {:.2}s de audio", audio.duracion_s());
        } else {
            println!("  {tipo:?}: NO se lee");
        }
    }
}
