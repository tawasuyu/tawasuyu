//! Demo de la **compuerta wake-word (F1)** — sin micrófono, certificado por texto.
//!
//! Muestra la diferencia con F0: con un detector de llamado enrolado, estando
//! dormida, una utterance que NO suena al llamado **no se transcribe** — aunque
//! el VAD haya visto sonido y el STT fuese a decir «shuma». Sólo la que matchea
//! cruza al STT y despierta.
//!
//! ```sh
//! cargo run -p rimay-voz-host --example wake_gateo
//! ```

use std::sync::Arc;

use rimay_voz::{Audio, ConfigVad, ConfigVoz, DetectorPlantilla, ParamsLlamado, TranscriptorMock};
use rimay_voz_host::Lazo;

const FRAME: usize = 480;

/// Patrón "llamado": alterna signo (zcr alto), como una sílaba con energía.
fn como_llamado(n_frames: usize) -> Vec<i16> {
    (0..FRAME * n_frames)
        .map(|i| if i % 2 == 0 { 18_000 } else { -18_000 })
        .collect()
}
/// Patrón "otra cosa": constante (zcr 0) — tiene energía (el VAD lo ve como
/// voz) pero NO suena al llamado.
fn otra_cosa(n_frames: usize) -> Vec<i16> {
    vec![6_000; FRAME * n_frames]
}
fn silencio(n_frames: usize) -> Vec<i16> {
    vec![0; FRAME * n_frames]
}

#[tokio::main]
async fn main() {
    let params = ParamsLlamado::default();
    // Enrolamos el detector con una grabación del "llamado" (acá sintética; en
    // metal serían unas grabaciones reales de «shuma»).
    let plantilla = Audio::new(como_llamado(8), 16_000);
    let detector = Arc::new(DetectorPlantilla::enrolar(&[plantilla], 0.5, params));

    // El STT mock diría «shuma» para CUALQUIER audio: así se ve que lo que
    // filtra es la compuerta, no el STT.
    let stt = Arc::new(TranscriptorMock::con_texto("shuma"));
    // VAD con colgado corto para cerrar utterances con 3 frames de silencio.
    let cfg_vad = ConfigVad { umbral: 0.5, arranque: 2, colgado: 3 };
    let mut lazo = Lazo::con_config(stt, 16_000, FRAME, cfg_vad, ConfigVoz::default())
        .con_detector_llamado(detector.clone());

    println!("== compuerta wake-word (F1), umbral 0.5 ==\n");

    // Utterance 1: NO suena al llamado → la compuerta la frena, no se transcribe.
    let dist_otra = detector.distancia(&Audio::new(otra_cosa(5), 16_000));
    let mut evs = lazo.empujar(&otra_cosa(5)).await;
    evs.extend(lazo.empujar(&silencio(3)).await);
    println!("«otra cosa» (con energía, pero no es el llamado):");
    println!("  distancia al llamado: {dist_otra:.2}  (> 0.5 → rechazada)");
    println!("  eventos: {evs:?}");
    println!("  estado:  {:?}  → el STT NO la vio\n", lazo.estado());

    // Utterance 2: suena al llamado → cruza el STT → despierta.
    let dist_lla = detector.distancia(&Audio::new(como_llamado(5), 16_000));
    let mut evs = lazo.empujar(&como_llamado(5)).await;
    evs.extend(lazo.empujar(&silencio(3)).await);
    println!("«llamado» (matchea la plantilla):");
    println!("  distancia al llamado: {dist_lla:.2}  (≤ 0.5 → pasa)");
    println!("  eventos: {evs:?}");
    println!("  estado:  {:?}  → cruzó al STT y despertó\n", lazo.estado());

    println!("== con F1, la nube sólo ve lo que suena a «shuma» ==");
}
