//! Estado de Bluetooth para el widget `bluetooth` (gemelo del applet de red).
//!
//! Como la red o el clima, es **dato del host** en su **propio hilo**: sondea el
//! controlador y los dispositivos emparejados y publica la foto por un canal. La
//! fuente es `bluetoothctl` (BlueZ) en modo no interactivo, sin sumar un cliente
//! D-Bus al árbol — mismo patrón defensivo que `network` con `nmcli`. Si no está,
//! el widget queda en `available=false` (icono tenue) sin romper la barra.
//!
//! Alcance: enciende/apaga el controlador, conecta y desconecta dispositivos
//! **emparejados**. El emparejamiento de uno nuevo (scan + pair) queda fuera (es
//! un flujo con PIN/confirmación; hoy se hace una vez con `bluetoothctl`).

use std::collections::HashSet;
use std::sync::mpsc::{channel, Receiver};
use std::time::Duration;

/// Un dispositivo Bluetooth emparejado, para el popup.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BtDevice {
    /// La dirección MAC (clave para conectar/desconectar).
    pub mac: String,
    /// Nombre legible.
    pub name: String,
    /// `true` si está conectado ahora.
    pub connected: bool,
}

/// La foto del Bluetooth que el hilo publica.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct BtState {
    /// `true` si `bluetoothctl` respondió (hay controlador/BlueZ).
    pub available: bool,
    /// `true` si el controlador está encendido.
    pub powered: bool,
    /// Dispositivos emparejados, los conectados primero.
    pub devices: Vec<BtDevice>,
}

// ============================================================
// Parsers puros (testeables sin BlueZ)
// ============================================================

/// `true` si `bluetoothctl show` reporta `Powered: yes`.
pub fn parse_powered(out: &str) -> bool {
    out.lines().any(|l| {
        let l = l.trim();
        l.starts_with("Powered:") && l.ends_with("yes")
    })
}

/// Parsea `bluetoothctl devices [Paired]` → `(mac, nombre)` por línea
/// `Device <MAC> <Nombre>`. Descarta líneas que no calzan.
pub fn parse_devices(out: &str) -> Vec<(String, String)> {
    let mut v = Vec::new();
    for l in out.lines() {
        let l = l.trim();
        let Some(rest) = l.strip_prefix("Device ") else {
            continue;
        };
        let mut it = rest.splitn(2, ' ');
        let Some(mac) = it.next() else { continue };
        if mac.is_empty() {
            continue;
        }
        let name = it.next().unwrap_or(mac).trim().to_string();
        v.push((mac.to_string(), name));
    }
    v
}

/// El conjunto de MAC conectadas, de `bluetoothctl devices Connected`.
pub fn parse_connected(out: &str) -> HashSet<String> {
    parse_devices(out).into_iter().map(|(mac, _)| mac).collect()
}

/// Arma la lista ordenada (conectados primero, luego por nombre) a partir de los
/// emparejados y el conjunto de conectados.
pub fn build_devices(paired: Vec<(String, String)>, connected: &HashSet<String>) -> Vec<BtDevice> {
    let mut devs: Vec<BtDevice> = paired
        .into_iter()
        .map(|(mac, name)| {
            let connected = connected.contains(&mac);
            BtDevice { mac, name, connected }
        })
        .collect();
    devs.sort_by(|a, b| {
        b.connected
            .cmp(&a.connected)
            .then(a.name.to_lowercase().cmp(&b.name.to_lowercase()))
    });
    devs
}

// ============================================================
// Acciones (fire-and-forget)
// ============================================================

/// Conecta el dispositivo `mac` (`bluetoothctl connect`). No bloquea.
pub fn connect(mac: &str) {
    spawn(&["connect", mac]);
}

/// Desconecta el dispositivo `mac` (`bluetoothctl disconnect`). No bloquea.
pub fn disconnect(mac: &str) {
    spawn(&["disconnect", mac]);
}

/// Enciende/apaga el controlador (`bluetoothctl power on|off`). No bloquea.
pub fn set_power(on: bool) {
    spawn(&["power", if on { "on" } else { "off" }]);
}

fn spawn(args: &[&str]) {
    let _ = std::process::Command::new("bluetoothctl")
        .args(args)
        .stdin(std::process::Stdio::null())
        .stdout(std::process::Stdio::null())
        .stderr(std::process::Stdio::null())
        .spawn();
}

// ============================================================
// Muestreo en el hilo
// ============================================================

/// Corre `bluetoothctl <args>` con tope de tiempo; `None` si no está o falla.
fn run(args: &[&str]) -> Option<String> {
    use std::io::Read;
    use std::time::Instant;
    const PLAZO: Duration = Duration::from_secs(5);
    let mut child = std::process::Command::new("bluetoothctl")
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

/// Una lectura completa. `None` si bluetoothctl no responde.
fn sample() -> Option<BtState> {
    let show = run(&["show"])?;
    let powered = parse_powered(&show);
    // `devices Paired`/`Connected` existen en BlueZ moderno; si el filtro no se
    // soporta, `devices` sin filtro lista todo (y `Connected` queda vacío).
    let paired = parse_devices(&run(&["devices", "Paired"]).or_else(|| run(&["devices"])).unwrap_or_default());
    let connected = parse_connected(&run(&["devices", "Connected"]).unwrap_or_default());
    Some(BtState {
        available: true,
        powered,
        devices: build_devices(paired, &connected),
    })
}

/// El feed de Bluetooth corriendo en su propio hilo.
pub struct BluetoothHandle {
    rx: Receiver<BtState>,
}

impl BluetoothHandle {
    /// Arranca el hilo. Refresca cada ~5 s (15 s si bluetoothctl no responde).
    pub fn spawn() -> Self {
        let (tx, rx) = channel();
        std::thread::Builder::new()
            .name("pata-bluetooth".into())
            .spawn(move || loop {
                let (estado, espera) = match sample() {
                    Some(s) => (s, Duration::from_secs(5)),
                    None => (BtState::default(), Duration::from_secs(15)),
                };
                if tx.send(estado).is_err() {
                    break;
                }
                std::thread::sleep(espera);
            })
            .ok();
        Self { rx }
    }

    /// La lectura más reciente (drena la cola), o `None` si no llegó nada nuevo.
    pub fn latest(&self) -> Option<BtState> {
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
    fn powered_de_show() {
        assert!(parse_powered("Controller AA\n\tPowered: yes\n\tDiscoverable: no"));
        assert!(!parse_powered("\tPowered: no"));
        assert!(!parse_powered("sin nada"));
    }

    #[test]
    fn devices_y_conectados() {
        let dev = "Device AA:BB:CC:DD:EE:FF Sony WH-1000XM4\nDevice 11:22:33:44:55:66 Magic Mouse\nbasura";
        let paired = parse_devices(dev);
        assert_eq!(paired.len(), 2);
        assert_eq!(paired[0], ("AA:BB:CC:DD:EE:FF".into(), "Sony WH-1000XM4".into()));
        let connected = parse_connected("Device 11:22:33:44:55:66 Magic Mouse");
        let built = build_devices(paired, &connected);
        // El conectado (Magic Mouse) va primero pese al orden alfabético.
        assert!(built[0].connected);
        assert_eq!(built[0].name, "Magic Mouse");
        assert!(!built[1].connected);
    }
}
