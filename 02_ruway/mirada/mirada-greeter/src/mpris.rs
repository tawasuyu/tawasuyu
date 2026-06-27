//! Estado del reproductor activo (MPRIS) para el cuadro de música del lock.
//!
//! Espejo de `pata-llimphi/src/mpris.rs`, con un campo de más: **quién** suena
//! (el nombre del reproductor — `playerName`), que es lo que el lock muestra en
//! grande («Spotify», «mpv»…). Como el clima o la red, es dato del host en su
//! propio hilo: sondea `playerctl` (el cliente MPRIS de línea de comandos), sin
//! sumar un cliente D-Bus al árbol. Sin `playerctl` o sin reproductor el estado
//! queda sin player y el cuadro se oculta.

use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

/// El estado del reproductor activo que el hilo publica.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct MediaState {
    /// `true` si hay un reproductor MPRIS respondiendo.
    pub has_player: bool,
    /// `true` si está reproduciendo (vs pausado/detenido).
    pub playing: bool,
    /// **Quién** suena: el nombre del reproductor, capitalizado (`Spotify`,
    /// `Mpv`…). Vacío si `playerctl` no lo da.
    pub player: String,
    /// Texto de la pista (`artista — título`, o lo que haya).
    pub title: String,
}

/// Parsea `playerctl status` → `Some(true)` si `Playing`, `Some(false)` si
/// `Paused`/`Stopped`, `None` si la salida no es un estado conocido.
pub fn parse_status(s: &str) -> Option<bool> {
    match s.trim() {
        "Playing" => Some(true),
        "Paused" | "Stopped" => Some(false),
        _ => None,
    }
}

/// Capitaliza la primera letra del nombre del reproductor (`spotify` → `Spotify`).
pub fn nice_player(raw: &str) -> String {
    let raw = raw.trim();
    let mut chars = raw.chars();
    match chars.next() {
        Some(first) => format!("{}{}", first.to_uppercase(), chars.as_str()),
        None => String::new(),
    }
}

/// El feed MPRIS corriendo en su propio hilo. Publica la última lectura por un
/// canal; el frontend la drena con [`MprisHandle::latest`] por frame.
pub struct MprisHandle {
    rx: Receiver<MediaState>,
}

impl MprisHandle {
    /// Arranca el hilo. Refresca cada ~1.5 s.
    pub fn spawn() -> Self {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("greeter-mpris".into())
            .spawn(move || loop {
                if tx.send(sample()).is_err() {
                    break; // la app se fue
                }
                std::thread::sleep(Duration::from_millis(1500));
            })
            .ok();
        Self { rx }
    }

    /// La lectura más reciente (drena la cola), o `None` si no llegó nada nuevo.
    pub fn latest(&self) -> Option<MediaState> {
        let mut last = None;
        while let Ok(s) = self.rx.try_recv() {
            last = Some(s);
        }
        last
    }
}

/// Una lectura del reproductor activo vía `playerctl`. Sin player → `has_player`
/// en falso (el cuadro se oculta). Pedimos `playerName|artista — título` en una
/// sola llamada y partimos por el primer `|` (el nombre del player no lo lleva).
fn sample() -> MediaState {
    let Some(status_out) = run(&["status"]) else {
        return MediaState::default();
    };
    let Some(playing) = parse_status(&status_out) else {
        return MediaState::default();
    };
    let meta = run(&["metadata", "--format", "{{playerName}}|{{artist}} — {{title}}"])
        .unwrap_or_default();
    let (player, title) = match meta.trim().split_once('|') {
        Some((p, t)) => (nice_player(p), t.trim().trim_matches('—').trim().to_string()),
        None => (String::new(), String::new()),
    };
    let title = if title.is_empty() { "♪".to_string() } else { title };
    MediaState {
        has_player: true,
        playing,
        player,
        title,
    }
}

/// Play/pausa del reproductor activo (desacoplado, no espera).
pub fn play_pause() {
    spawn(&["play-pause"]);
}

/// Pista siguiente.
pub fn next() {
    spawn(&["next"]);
}

/// Pista anterior.
pub fn previous() {
    spawn(&["previous"]);
}

/// Corre `playerctl <args>` con tope de tiempo y devuelve su stdout, o `None` si
/// no está / falla / se pasa del plazo. Mismo patrón defensivo que en pata.
fn run(args: &[&str]) -> Option<String> {
    use std::io::Read;
    use std::time::Instant;
    const PLAZO: Duration = Duration::from_secs(3);
    let mut child = std::process::Command::new("playerctl")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::piped())
        .stderr(std::process::Stdio::null())
        .spawn()
        .ok()?;
    let inicio = Instant::now();
    loop {
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return None;
                }
                let mut buf = String::new();
                child.stdout.take()?.read_to_string(&mut buf).ok()?;
                return Some(buf);
            }
            Ok(None) => {
                if inicio.elapsed() >= PLAZO {
                    let _ = child.kill();
                    let _ = child.wait();
                    return None;
                }
                std::thread::sleep(Duration::from_millis(20));
            }
            Err(_) => return None,
        }
    }
}

/// Lanza `playerctl <args>` sin esperar (transporte: play/pausa/siguiente).
fn spawn(args: &[&str]) {
    let _ = std::process::Command::new("playerctl")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn estado_conocido() {
        assert_eq!(parse_status("Playing\n"), Some(true));
        assert_eq!(parse_status("Paused"), Some(false));
        assert_eq!(parse_status("Stopped"), Some(false));
        assert_eq!(parse_status("No players found"), None);
        assert_eq!(parse_status(""), None);
    }

    #[test]
    fn nombre_capitalizado() {
        assert_eq!(nice_player("spotify"), "Spotify");
        assert_eq!(nice_player("mpv"), "Mpv");
        assert_eq!(nice_player(""), "");
    }
}
