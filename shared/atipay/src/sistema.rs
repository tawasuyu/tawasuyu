//! Fuente de capacidades del **sistema operativo**: energía, sesión, red,
//! bluetooth, brillo y volumen. A diferencia de mirada/sandokan, no hay un CLI
//! único — cada acción usa su herramienta estándar (`systemctl`, `loginctl`,
//! `nmcli`, `bluetoothctl`, `brightnessctl`, `wpctl`), exactamente las que ya
//! envuelve `pata` (Regla 2: el control del sistema son comandos, no un cliente
//! D-Bus en el árbol). Por eso cada capacidad lleva su propio `programa` +
//! `args_base` (constructor [`Capacidad::cli_args`]).
//!
//! Las acciones privilegiadas (apagar/reiniciar/zona horaria) las gobierna el
//! agente polkit del sistema cuando hace falta; acá sólo se arma el comando.

use crate::{Capacidad, FuenteCapacidades, Param, Peligro, Superficie};

/// La fuente del sistema. Sin estado.
pub struct FuenteSistema;

impl FuenteCapacidades for FuenteSistema {
    fn superficie(&self) -> Superficie {
        Superficie::Sistema
    }

    fn capacidades(&self) -> Vec<Capacidad> {
        use Peligro::*;
        // Helper: capacidad con programa + args base explícitos.
        let c = |sufijo, programa, args: &[&str], resumen, peligro, params| {
            Capacidad::cli_args(Superficie::Sistema, sufijo, programa, args, resumen, peligro, params)
        };
        vec![
            // Energía / sesión.
            c("bloquear", "loginctl", &["lock-session"], "Bloquea la sesión actual.", Reversible, vec![]),
            c("suspender", "systemctl", &["suspend"], "Suspende el equipo (a RAM).", Disruptivo, vec![]),
            c("hibernar", "systemctl", &["hibernate"], "Hiberna el equipo (a disco).", Disruptivo, vec![]),
            c("reiniciar", "systemctl", &["reboot"], "Reinicia el equipo.", Disruptivo, vec![]),
            c("apagar", "systemctl", &["poweroff"], "Apaga el equipo.", Disruptivo, vec![]),
            // Red (NetworkManager).
            c("wifi-on", "nmcli", &["radio", "wifi", "on"], "Enciende la radio Wi-Fi.", Reversible, vec![]),
            c("wifi-off", "nmcli", &["radio", "wifi", "off"], "Apaga la radio Wi-Fi.", Reversible, vec![]),
            c("listar-redes", "nmcli", &["device", "wifi", "list"], "Lista las redes Wi-Fi visibles.", Seguro, vec![]),
            c("conectar-wifi", "nmcli", &["device", "wifi", "connect"],
                "Conecta a una red Wi-Fi por su SSID (usa el perfil/agente de secretos para la clave).", Reversible,
                vec![Param::texto("ssid", "Nombre (SSID) de la red.")]),
            // Bluetooth.
            c("bluetooth-on", "bluetoothctl", &["power", "on"], "Enciende el controlador Bluetooth.", Reversible, vec![]),
            c("bluetooth-off", "bluetoothctl", &["power", "off"], "Apaga el controlador Bluetooth.", Reversible, vec![]),
            // Brillo (panel del portátil).
            c("brillo", "brightnessctl", &["set"],
                "Ajusta el brillo de la pantalla.", Reversible,
                vec![Param::texto("nivel", "Nivel: '50%' absoluto, '+10%'/'10%-' relativo.")]),
            // Volumen (PipeWire/WirePlumber).
            c("volumen", "wpctl", &["set-volume", "@DEFAULT_AUDIO_SINK@"],
                "Ajusta el volumen de salida.", Reversible,
                vec![Param::texto("nivel", "Nivel: '0.5' (50%), '5%+'/'5%-' relativo.")]),
            c("silenciar", "wpctl", &["set-mute", "@DEFAULT_AUDIO_SINK@", "toggle"],
                "Alterna silencio en la salida de audio.", Reversible, vec![]),
        ]
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{Catalogo, Invocacion};

    fn cat() -> Catalogo {
        let mut c = Catalogo::new();
        c.registrar(Box::new(FuenteSistema));
        c
    }

    #[test]
    fn apagar_es_systemctl_poweroff_disruptivo() {
        let p = cat().plan(&Invocacion::nueva("sistema.apagar")).unwrap();
        assert_eq!(p.programa, "systemctl");
        assert_eq!(p.args, vec!["poweroff"]);
        assert_eq!(p.peligro, Peligro::Disruptivo);
    }

    #[test]
    fn conectar_wifi_apila_el_ssid_sobre_los_args_base() {
        let p = cat().plan(&Invocacion::nueva("sistema.conectar-wifi").con("ssid", "MiRed")).unwrap();
        assert_eq!(p.programa, "nmcli");
        assert_eq!(p.args, vec!["device", "wifi", "connect", "MiRed"]);
    }

    #[test]
    fn brillo_arma_brightnessctl_set_nivel() {
        let p = cat().plan(&Invocacion::nueva("sistema.brillo").con("nivel", "50%")).unwrap();
        assert_eq!(p.programa, "brightnessctl");
        assert_eq!(p.args, vec!["set", "50%"]);
    }
}
