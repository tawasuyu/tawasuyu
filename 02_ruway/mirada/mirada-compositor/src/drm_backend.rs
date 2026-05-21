//! `drm_backend` — el Cuerpo del compositor sobre **DRM/KMS**, sin
//! sesión gráfica anfitriona: corre directo sobre una TTY, como tu
//! escritorio de verdad.
//!
//! Se construye por fases para poder verificarlo en hardware real
//! paso a paso:
//!
//! - **Fase 1 — bring-up** (esto): abre la sesión (`libseat`), encuentra
//!   la GPU, abre el dispositivo DRM y enumera las salidas físicas. No
//!   compone nada todavía; sólo comprueba —y registra en el log— que la
//!   ruta de hardware funciona en tu máquina.
//! - **Fase 2** (siguiente): GBM + EGL + `GlesRenderer`, un
//!   `DrmCompositor` por salida, `libinput` para el teclado, y el bucle
//!   `calloop` que compone de verdad.
//!
//! Todo va instrumentado con `println!`/`eprintln!` para que, al
//! correrlo, se pueda copiar la salida y diagnosticar sin tener el
//! hardware delante.

use std::error::Error;

use smithay::backend::drm::{DrmDevice, DrmDeviceFd};
use smithay::backend::session::libseat::LibSeatSession;
use smithay::backend::session::Session;
use smithay::backend::udev;
use smithay::reexports::drm::control::connector::State as ConnectorState;
use smithay::reexports::drm::control::Device as ControlDevice;
use smithay::reexports::rustix::fs::OFlags;
use smithay::utils::DeviceFd;

/// Arranca el Cuerpo sobre DRM/KMS — **fase 1: bring-up**.
///
/// Abre la sesión, localiza la GPU, abre el dispositivo DRM y enumera
/// las salidas, dejando constancia de todo en el log. La composición
/// real es la fase 2.
pub fn run() -> Result<(), Box<dyn Error>> {
    println!("mirada-compositor · backend DRM — fase 1 (bring-up).");
    println!("──────────────────────────────────────────────────");

    // 1 · La sesión. `libseat` nos da acceso a DRM y a los dispositivos
    //     de entrada sin ser root — habla con `seatd` o con `logind`.
    println!("[1/4] abriendo la sesión (libseat) …");
    let (mut session, _notifier) = LibSeatSession::new().map_err(|e| {
        format!(
            "no pude abrir la sesión libseat: {e}\n       \
             ¿estás en una TTY de verdad (Ctrl+Alt+F3), con `seatd` o \
             `logind` corriendo?"
        )
    })?;
    let seat_name = session.seat();
    println!("      sesión abierta · seat «{seat_name}»");

    // 2 · La GPU primaria del seat.
    println!("[2/4] buscando la GPU primaria …");
    let gpu = udev::primary_gpu(&seat_name)
        .map_err(|e| format!("error consultando udev: {e}"))?
        .ok_or("no encontré ninguna GPU — ¿existe algún /dev/dri/card*?")?;
    println!("      GPU primaria: {}", gpu.display());

    // 3 · Abrir el dispositivo DRM a través de la sesión.
    println!("[3/4] abriendo el dispositivo DRM …");
    let fd = session
        .open(&gpu, OFlags::RDWR | OFlags::CLOEXEC | OFlags::NONBLOCK)
        .map_err(|e| format!("no pude abrir {}: {e}", gpu.display()))?;
    let drm_fd = DrmDeviceFd::new(DeviceFd::from(fd));
    let (drm, _drm_notifier) =
        DrmDevice::new(drm_fd, true).map_err(|e| format!("DrmDevice::new falló: {e}"))?;
    println!("      dispositivo DRM listo.");

    // 4 · Enumerar conectores: cada uno conectado es una salida física.
    println!("[4/4] enumerando salidas …");
    let resources = drm
        .resource_handles()
        .map_err(|e| format!("no pude leer los recursos DRM: {e}"))?;
    let mut connected = 0usize;
    for &handle in resources.connectors() {
        let info = match drm.get_connector(handle, false) {
            Ok(info) => info,
            Err(e) => {
                eprintln!("      conector {handle:?}: error al leerlo: {e}");
                continue;
            }
        };
        let name = format!("{:?}-{}", info.interface(), info.interface_id());
        match info.state() {
            ConnectorState::Connected => {
                connected += 1;
                match info.modes().first() {
                    Some(mode) => {
                        let (w, h) = mode.size();
                        println!(
                            "      · «{name}» CONECTADA — modo preferido {w}×{h} \
                             @ {} Hz  ({} modos)",
                            mode.vrefresh(),
                            info.modes().len(),
                        );
                    }
                    None => println!("      · «{name}» CONECTADA — sin modos anunciados"),
                }
            }
            ConnectorState::Disconnected => println!("      · «{name}» desconectada"),
            other => println!("      · «{name}» — estado {other:?}"),
        }
    }
    println!("      {connected} salida(s) conectada(s).");

    println!("──────────────────────────────────────────────────");
    if connected == 0 {
        eprintln!("mirada-compositor · bring-up OK, pero no hay ninguna salida");
        eprintln!("   conectada — sin pantalla no hay nada que componer.");
    } else {
        println!("mirada-compositor · bring-up DRM completado correctamente.");
    }
    println!(
        "   La composición sobre DRM es la fase 2. Copia estos logs y\n   \
         seguimos. Mientras, el backend winit ya funciona dentro de un\n   \
         escritorio:  mirada-compositor --winit"
    );
    Ok(())
}
