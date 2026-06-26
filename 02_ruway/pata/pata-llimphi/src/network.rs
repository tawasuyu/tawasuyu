//! Estado de red para el widget `network` (el applet de Wi-Fi/Ethernet).
//!
//! Como el clima o el tray, es **dato del host** que el frontend muestrea aparte
//! del view-model de core: corre en su **propio hilo** (consultar al
//! NetworkManager puede tardar) y publica la última lectura por un canal. La
//! fuente es `nmcli` (NetworkManager), invocado en modo terse (`-t`), sin agregar
//! una dependencia de D-Bus al árbol — mismo patrón defensivo que `weather` con
//! `curl` o el sampler con `wpctl`. Si `nmcli` no está, el widget queda en
//! [`NetStatus::Sin`] (icono tenue) sin romper la barra.
//!
//! El render traduce el [`NetState`] a un **dibujo del nivel de señal** (barras
//! ascendentes) y el popup lista los SSID disponibles para conectarse.
//!
//! **Alcance**: enumera redes, conecta a una guardada/abierta, desconecta y
//! conmuta la radio. La **entrada de contraseña** para una red segura nueva queda
//! pendiente (necesita un campo de texto con foco de teclado, como el menú de
//! inicio); hoy una red segura sin perfil guardado depende del agente de secretos
//! del sistema (nm-applet/polkit).

use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

/// La conexión activa, derivada de lo que reporta NetworkManager.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub enum NetStatus {
    /// Cable conectado.
    Ethernet,
    /// Wi-Fi conectado a `ssid` con intensidad `signal` (0..=100).
    Wifi {
        /// El SSID de la red activa.
        ssid: String,
        /// Intensidad de señal 0..=100.
        signal: u8,
    },
    /// Radio Wi-Fi apagada (rfkill / `nmcli radio wifi off`).
    WifiOff,
    /// Sin conexión (radio encendida pero sin asociar, y sin cable).
    #[default]
    Desconectado,
    /// No hay NetworkManager (nmcli ausente o sin responder).
    Sin,
}

/// Un punto de acceso Wi-Fi visible, para el popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WifiAp {
    /// El nombre de la red.
    pub ssid: String,
    /// Intensidad de señal 0..=100.
    pub signal: u8,
    /// `true` si la red pide credenciales (no es abierta).
    pub secure: bool,
    /// `true` si es la red a la que estamos conectados.
    pub active: bool,
}

/// La foto de la red que el hilo publica: estado actual + radio + lista de redes.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct NetState {
    /// La conexión activa.
    pub status: NetStatus,
    /// `true` si la radio Wi-Fi está habilitada.
    pub wifi_enabled: bool,
    /// Redes Wi-Fi visibles, la activa primero, luego por señal descendente.
    pub networks: Vec<WifiAp>,
}

// ============================================================
// Parsers puros (testeables sin red)
// ============================================================

/// Parte una línea terse de `nmcli -t` respetando el escape `\:` (un SSID puede
/// contener `:`). NetworkManager escapa `:` y `\` como `\:` y `\\`.
fn split_terse(line: &str) -> Vec<String> {
    let mut campos = Vec::new();
    let mut actual = String::new();
    let mut chars = line.chars();
    while let Some(c) = chars.next() {
        match c {
            '\\' => {
                // El siguiente carácter es literal (`\:` → `:`, `\\` → `\`).
                if let Some(n) = chars.next() {
                    actual.push(n);
                }
            }
            ':' => {
                campos.push(std::mem::take(&mut actual));
            }
            _ => actual.push(c),
        }
    }
    campos.push(actual);
    campos
}

/// Parsea la salida de `nmcli -t -f ACTIVE,SSID,SIGNAL,SECURITY device wifi`.
/// Deduplica por SSID quedándose con la entrada de mayor señal (o la activa);
/// descarta SSID vacíos (redes ocultas). Ordena activa primero, luego por señal.
pub fn parse_wifi_list(out: &str) -> Vec<WifiAp> {
    let mut aps: Vec<WifiAp> = Vec::new();
    for line in out.lines() {
        let line = line.trim();
        if line.is_empty() {
            continue;
        }
        let campos = split_terse(line);
        if campos.len() < 4 {
            continue;
        }
        let active = campos[0].trim().eq_ignore_ascii_case("yes");
        let ssid = campos[1].trim().to_string();
        if ssid.is_empty() {
            continue;
        }
        let signal: u8 = campos[2].trim().parse().unwrap_or(0).min(100);
        // SECURITY vacío (o "--") = red abierta.
        let sec = campos[3].trim();
        let secure = !sec.is_empty() && sec != "--";
        // Dedup: si ya está el SSID, conservamos el mejor (activo o más fuerte).
        if let Some(prev) = aps.iter_mut().find(|a| a.ssid == ssid) {
            if active || signal > prev.signal {
                prev.signal = signal.max(prev.signal);
                prev.active = prev.active || active;
                prev.secure = secure;
            }
            continue;
        }
        aps.push(WifiAp {
            ssid,
            signal,
            secure,
            active,
        });
    }
    aps.sort_by(|a, b| {
        b.active
            .cmp(&a.active)
            .then(b.signal.cmp(&a.signal))
            .then(a.ssid.cmp(&b.ssid))
    });
    aps
}

/// Parsea `nmcli -t radio wifi` → `true` si dice `enabled`.
pub fn parse_radio(out: &str) -> bool {
    out.trim().eq_ignore_ascii_case("enabled")
}

/// Parsea `nmcli -t -f TYPE,STATE device status` → `true` si hay un `ethernet`
/// en estado `connected`.
pub fn parse_ethernet_connected(out: &str) -> bool {
    out.lines().any(|l| {
        let campos = split_terse(l.trim());
        campos.len() >= 2
            && campos[0].trim().eq_ignore_ascii_case("ethernet")
            && campos[1].trim().starts_with("connected")
    })
}

/// Deriva el [`NetStatus`] a partir de las tres lecturas: radio, cable y APs.
/// Prioridad: radio apagada → `WifiOff`; Wi-Fi asociado → `Wifi`; cable →
/// `Ethernet`; si no → `Desconectado`.
pub fn derive_status(wifi_enabled: bool, ethernet: bool, aps: &[WifiAp]) -> NetStatus {
    if let Some(activo) = aps.iter().find(|a| a.active) {
        return NetStatus::Wifi {
            ssid: activo.ssid.clone(),
            signal: activo.signal,
        };
    }
    if ethernet {
        return NetStatus::Ethernet;
    }
    if !wifi_enabled {
        return NetStatus::WifiOff;
    }
    NetStatus::Desconectado
}

// ============================================================
// Acciones (fire-and-forget, como spawn_cmd)
// ============================================================

/// Conecta a la red `ssid` (`nmcli device wifi connect`). Usa el perfil guardado
/// o el agente de secretos del sistema para la contraseña; no bloquea.
pub fn connect(ssid: &str) {
    let _ = std::process::Command::new("nmcli")
        .args(["device", "wifi", "connect", ssid])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Conecta a `ssid` con una contraseña explícita
/// (`nmcli device wifi connect <ssid> password <pw>`). Con `pw` vacío cae a
/// [`connect`] (perfil guardado / agente). No bloquea; la contraseña va por
/// argumentos al subproceso nmcli (no por la shell), sin quoting frágil.
pub fn connect_with(ssid: &str, pw: &str) {
    if pw.is_empty() {
        return connect(ssid);
    }
    let _ = std::process::Command::new("nmcli")
        .args(["device", "wifi", "connect", ssid, "password", pw])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Baja la conexión activa con ese `ssid` (`nmcli connection down`). No bloquea.
pub fn disconnect(ssid: &str) {
    let _ = std::process::Command::new("nmcli")
        .args(["connection", "down", "id", ssid])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

/// Enciende/apaga la radio Wi-Fi (`nmcli radio wifi on|off`). No bloquea.
pub fn set_wifi_radio(on: bool) {
    let _ = std::process::Command::new("nmcli")
        .args(["radio", "wifi", if on { "on" } else { "off" }])
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

// ============================================================
// Muestreo síncrono (corre en el hilo)
// ============================================================

/// Corre `nmcli <args>` con un tope de tiempo y devuelve su stdout, o `None` si
/// nmcli no está, falla, o se pasa del plazo (la red puede colgar).
fn run_nmcli(args: &[&str]) -> Option<String> {
    use std::io::Read;
    use std::time::Instant;
    const PLAZO: Duration = Duration::from_secs(6);
    let mut child = std::process::Command::new("nmcli")
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

/// Una lectura completa del estado de la red. `None` si nmcli no responde.
fn sample() -> Option<NetState> {
    // La lista de Wi-Fi y la radio son las consultas esenciales; el cable es
    // best-effort (su ausencia no invalida la lectura).
    let radio_out = run_nmcli(&["-t", "radio", "wifi"])?;
    let wifi_enabled = parse_radio(&radio_out);
    let wifi_out = run_nmcli(&["-t", "-f", "ACTIVE,SSID,SIGNAL,SECURITY", "device", "wifi"])
        .unwrap_or_default();
    let networks = parse_wifi_list(&wifi_out);
    let eth_out = run_nmcli(&["-t", "-f", "TYPE,STATE", "device", "status"]).unwrap_or_default();
    let ethernet = parse_ethernet_connected(&eth_out);
    Some(NetState {
        status: derive_status(wifi_enabled, ethernet, &networks),
        wifi_enabled,
        networks,
    })
}

/// El feed de red corriendo en su propio hilo. Publica la última lectura por un
/// canal; el frontend la drena con [`NetworkHandle::latest`] por frame.
pub struct NetworkHandle {
    rx: Receiver<NetState>,
}

impl NetworkHandle {
    /// Arranca el hilo. Refresca cada ~5 s. Si nmcli no responde, publica una
    /// lectura `Sin` (icono tenue) y reintenta más espaciado.
    pub fn spawn() -> Self {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("pata-network".into())
            .spawn(move || loop {
                let (estado, espera) = match sample() {
                    Some(s) => (s, Duration::from_secs(5)),
                    None => (
                        NetState {
                            status: NetStatus::Sin,
                            ..Default::default()
                        },
                        Duration::from_secs(15),
                    ),
                };
                if tx.send(estado).is_err() {
                    break; // la app se fue
                }
                std::thread::sleep(espera);
            })
            .ok();
        Self { rx }
    }

    /// La lectura más reciente (drena la cola), o `None` si no llegó nada nuevo.
    /// No bloquea.
    pub fn latest(&self) -> Option<NetState> {
        let mut last = None;
        while let Ok(s) = self.rx.try_recv() {
            last = Some(s);
        }
        last
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn split_respeta_escape() {
        assert_eq!(split_terse("yes:MiRed:72:WPA2"), vec!["yes", "MiRed", "72", "WPA2"]);
        // Un SSID con `:` viene escapado como `\:`.
        assert_eq!(
            split_terse(r"no:Red\:rara:40:WPA2"),
            vec!["no", "Red:rara", "40", "WPA2"]
        );
    }

    #[test]
    fn parsea_lista_wifi_dedup_y_orden() {
        let out = "\
yes:CasaWifi:65:WPA2
no:Vecino:80:WPA2
no:CasaWifi:50:WPA2
no::55:WPA2
no:Abierta:30:";
        let aps = parse_wifi_list(out);
        // La oculta (SSID vacío) se descarta.
        assert_eq!(aps.len(), 3);
        // La activa va primero pese a menor señal.
        assert_eq!(aps[0].ssid, "CasaWifi");
        assert!(aps[0].active);
        // La abierta no es segura.
        let abierta = aps.iter().find(|a| a.ssid == "Abierta").unwrap();
        assert!(!abierta.secure);
        // El resto, por señal descendente.
        assert_eq!(aps[1].ssid, "Vecino");
    }

    #[test]
    fn radio_enabled() {
        assert!(parse_radio("enabled\n"));
        assert!(!parse_radio("disabled"));
        assert!(!parse_radio("missing"));
    }

    #[test]
    fn ethernet_conectado() {
        assert!(parse_ethernet_connected("ethernet:connected\nwifi:disconnected"));
        assert!(parse_ethernet_connected("ethernet:connected (externally)"));
        assert!(!parse_ethernet_connected("ethernet:unavailable\nwifi:connected"));
        assert!(!parse_ethernet_connected("wifi:connected"));
    }

    #[test]
    fn deriva_estado() {
        let activa = vec![WifiAp {
            ssid: "X".into(),
            signal: 70,
            secure: true,
            active: true,
        }];
        assert_eq!(
            derive_status(true, false, &activa),
            NetStatus::Wifi { ssid: "X".into(), signal: 70 }
        );
        // Sin Wi-Fi asociado pero con cable.
        assert_eq!(derive_status(true, true, &[]), NetStatus::Ethernet);
        // Radio apagada y sin cable.
        assert_eq!(derive_status(false, false, &[]), NetStatus::WifiOff);
        // Radio encendida, sin asociar, sin cable.
        assert_eq!(derive_status(true, false, &[]), NetStatus::Desconectado);
    }
}
