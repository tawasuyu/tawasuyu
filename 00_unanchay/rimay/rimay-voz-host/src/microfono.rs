//! Driver de micrófono real — la única parte que toca hardware (cpal).
//!
//! El `cpal::Stream` es `!Send`, así que **no** puede vivir en una task de
//! tokio. El patrón canónico: el micrófono se abre en un **hilo dedicado** que
//! lo mantiene vivo, prepara el audio (mono + 16 kHz + `i16`) y manda los
//! bloques por un canal `Send` a una task async que corre el [`Lazo`] (el STT
//! es async) y emite [`EventoEscucha`] a la app.
//!
//! ```text
//!   hilo audio: MicSource(cpal) → a_mono → remuestreo → a_i16 ──┐  (Vec<i16>)
//!                                                                ▼
//!   task tokio: Lazo.empujar (VAD→STT→Maquina) + tick ──► EventoEscucha ─► app
//! ```
//!
//! La app recibe los eventos por un `mpsc::UnboundedReceiver` y los dispatcha
//! como `Msg` a su update Elm. Soltar la [`GuardiaEscucha`] para la captura.

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::time::Duration;

use media_core::AudioSource;
use media_source_capture::{MicError, MicSource};
use rimay_voz::Transcriptor;
use tokio::sync::mpsc;

use crate::lazo::{EventoEscucha, Lazo};
use crate::prep::{a_i16, a_mono, Remuestreador};

/// Tasa a la que el lazo (y whisper) quieren el audio.
const HZ_OBJETIVO: u32 = 16_000;
/// Cada cuánto el hilo de audio drena el micrófono.
const POLL: Duration = Duration::from_millis(20);
/// Cada cuánto la task manda un `tick` a la máquina (re-dormida por silencio).
const TICK: Duration = Duration::from_millis(250);

/// Mantiene viva la captura: soltarla corta el hilo de audio y la task.
pub struct GuardiaEscucha {
    parar: Arc<AtomicBool>,
    tarea: tokio::task::JoinHandle<()>,
}

impl Drop for GuardiaEscucha {
    fn drop(&mut self) {
        // El hilo de audio sale solo al ver la bandera (no lo joineamos para no
        // bloquear el Drop); la task async se aborta.
        self.parar.store(true, Ordering::Relaxed);
        self.tarea.abort();
    }
}

/// Arranca la escucha manos-libres sobre `stt`. Devuelve el receptor de eventos
/// y la guardia que la mantiene viva. Debe llamarse dentro de un runtime tokio.
///
/// El error de «no hay micrófono» llega sincrónico: abrimos el dispositivo en el
/// hilo de audio y reportamos el resultado antes de devolver.
pub fn escuchar(
    stt: Arc<dyn Transcriptor>,
) -> Result<(GuardiaEscucha, mpsc::UnboundedReceiver<EventoEscucha>), MicError> {
    let parar = Arc::new(AtomicBool::new(false));
    let (tx_audio, mut rx_audio) = mpsc::unbounded_channel::<Vec<i16>>();
    let (tx_abierto, rx_abierto) = std::sync::mpsc::channel::<Result<(), MicError>>();

    // --- Hilo de audio: dueño del MicSource (!Send). ---
    let parar_hilo = parar.clone();
    std::thread::Builder::new()
        .name("voz-microfono".into())
        .spawn(move || {
            let mut mic = match MicSource::open_default() {
                Ok(m) => {
                    let _ = tx_abierto.send(Ok(()));
                    m
                }
                Err(e) => {
                    let _ = tx_abierto.send(Err(e));
                    return;
                }
            };
            let de_hz = mic.sample_rate();
            let canales = mic.channels();
            let mut remu = Remuestreador::new(de_hz, HZ_OBJETIVO);
            // Buffer de ~POLL ms de audio nativo (intercalado).
            let n = (de_hz as usize / 50).max(1) * canales.max(1) as usize;
            let mut buf = vec![0f32; n];

            while !parar_hilo.load(Ordering::Relaxed) {
                mic.fill(&mut buf, de_hz, canales);
                let mono = a_mono(&buf, canales);
                let remuestreado = remu.procesar(&mono);
                if !remuestreado.is_empty() && tx_audio.send(a_i16(&remuestreado)).is_err() {
                    break; // el consumidor se fue
                }
                std::thread::sleep(POLL);
            }
        })
        .map_err(|e| MicError::Build(format!("spawn hilo de audio: {e}")))?;

    // Esperamos el resultado de abrir el dispositivo (rápido).
    match rx_abierto.recv() {
        Ok(Ok(())) => {}
        Ok(Err(e)) => return Err(e),
        Err(_) => return Err(MicError::Build("el hilo de audio no reportó apertura".into())),
    }

    // --- Task async: corre el Lazo (STT async) + el reloj de re-dormida. ---
    let (tx_ev, rx_ev) = mpsc::unbounded_channel::<EventoEscucha>();
    let tarea = tokio::spawn(async move {
        let mut lazo = Lazo::new(stt);
        let mut reloj = tokio::time::interval(TICK);
        loop {
            tokio::select! {
                bloque = rx_audio.recv() => {
                    let Some(muestras) = bloque else { break }; // hilo de audio terminó
                    for ev in lazo.empujar(&muestras).await {
                        if tx_ev.send(ev).is_err() {
                            return; // app se fue
                        }
                    }
                }
                _ = reloj.tick() => {
                    if let Some(ev) = lazo.tick() {
                        if tx_ev.send(ev).is_err() {
                            return;
                        }
                    }
                }
            }
        }
    });

    Ok((GuardiaEscucha { parar, tarea }, rx_ev))
}
