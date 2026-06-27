//! Demo del **pipeline completo upstream**: frames de audio → VAD → STT → máquina.
//!
//! El `escucha_mock` arranca de transcripts ya listos; éste muestra la pieza que
//! va *antes*: el [`Vad`] segmenta utterances de un flujo de frames (gated por
//! energía, determinista) y sólo entonces corre el STT del fragmento. Es el
//! gemelo en código del diagrama de `VOZ.md`:
//!
//! ```text
//!   frames → VAD (det.) → [utterance] → STT del fragmento → Maquina → Reacción
//! ```
//!
//! Todo sin micrófono ni modelo: los frames son sintéticos (silencio = ceros,
//! voz = amplitud alta) y el STT es un mock por utterance. Se certifica leyendo
//! la salida.
//!
//! ```sh
//! cargo run -p rimay-voz --example pipeline_vad
//! ```

use rimay_voz::{
    ConfigVad, ConfigVoz, DetectorEnergia, Evento, Maquina, Reaccion, SalidaVad, Transcriptor,
    TranscriptorMock, Vad,
};

const HZ: u32 = 16_000;
const MUESTRAS_POR_FRAME: usize = 480; // 30 ms a 16 kHz

/// Una utterance simulada: cuántos frames de voz dura y qué diría el STT de
/// ella. El host real obtendría el texto de whisper; acá lo fijamos por mock.
struct Utterance {
    frames_voz: usize,
    texto_stt: &'static str,
}

const GUION: &[Utterance] = &[
    Utterance { frames_voz: 5, texto_stt: "cargo build release" }, // ruido: no es el llamado
    Utterance { frames_voz: 4, texto_stt: "shuma" },               // el llamado → despierta
    Utterance { frames_voz: 8, texto_stt: "listá los archivos" },  // dictado
    Utterance { frames_voz: 6, texto_stt: "shuma abrí cosmos" },   // llamado + cola → dicta ya
];

#[tokio::main]
async fn main() {
    // Detector de energía + segmentador con colgado corto para el demo.
    let cfg = ConfigVad { umbral: 0.5, arranque: 2, colgado: 3 };
    let mut vad = Vad::new(DetectorEnergia::default(), cfg, HZ);
    let mut escucha = Maquina::new(ConfigVoz::default());

    let voz = vec![20_000i16; MUESTRAS_POR_FRAME]; // sobre el techo de energía
    let silencio = vec![0i16; MUESTRAS_POR_FRAME];

    println!("== pipeline upstream: frames → VAD → STT → máquina ==\n");

    for (i, u) in GUION.iter().enumerate() {
        println!("utterance {}: {} frames de voz", i + 1, u.frames_voz);

        // El host empuja los frames de voz...
        for _ in 0..u.frames_voz {
            if let SalidaVad::Empezo = vad.empujar(&voz) {
                // VAD detectó inicio → la máquina marca arranque de voz.
                escucha.avanzar(Evento::VozEmpieza);
                println!("  · VAD: empezó la utterance");
            }
        }
        // ...y luego el silencio que la cierra (colgado de 3 frames).
        let mut fragmento = None;
        for _ in 0..cfg.colgado {
            if let SalidaVad::Termino(audio) = vad.empujar(&silencio) {
                fragmento = Some(audio);
            }
        }

        let Some(audio) = fragmento else {
            println!("  · VAD: no cerró utterance (¿muy corta?)\n");
            continue;
        };
        println!("  · VAD: cerró → {:.2}s de audio al STT", audio.duracion_s());

        // Sólo ahora corre el STT, sobre el fragmento exacto que aisló el VAD.
        let stt = TranscriptorMock::con_texto(u.texto_stt);
        let texto = stt.transcribir(&audio).await.unwrap().texto;

        match escucha.avanzar(Evento::Transcript(texto)) {
            Reaccion::Nada => println!("  · STT «{}» → (ignorado)   [{:?}]\n", u.texto_stt, escucha.estado()),
            Reaccion::Desperto => println!("  · STT «{}» → despertó      [{:?}]\n", u.texto_stt, escucha.estado()),
            Reaccion::Dictar(t) => println!("  · STT «{}» → DICTAR: {t:?}   [{:?}]\n", u.texto_stt, escucha.estado()),
            Reaccion::SeDurmio => println!("  · STT «{}» → se durmió     [{:?}]\n", u.texto_stt, escucha.estado()),
        }
    }

    println!("== fin: nada pesado corrió hasta que el VAD vio voz ==");
}
