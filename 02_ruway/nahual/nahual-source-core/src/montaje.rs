//! Montaje CONVENCIONAL de una partición (lectura-escritura) vía `udisksctl`.
//!
//! El resto de [`dispositivo`](crate::dispositivo) es SOBERANO: lee el device
//! por bytes sin montar, read-only, sin privilegio. Pero ESCRIBIR un FAT/ext
//! correctamente desde userspace —sin el driver del kernel— es impracticable y
//! peligroso (journal de ext4, bitmaps, consistencia de metadata). Así que la
//! lectura-escritura delega en el kernel: montamos la partición y la navegamos
//! como POSIX (ya escribible por las ops del shell).
//!
//! Se usa `udisksctl` (de udisks2) en vez de `mount(2)` directo: monta en
//! `/run/media/$USER/...` SIN root (polkit autoriza al usuario de sesión sobre
//! medios removibles), y no mete una dependencia D-Bus al binario —sólo
//! ejecutamos el binario del sistema si está—. Si falta, la acción no se ofrece
//! y queda la navegación soberana read-only.

use std::io;
use std::path::{Path, PathBuf};
use std::process::Command;

/// `true` si `udisksctl` está en el `PATH` —gate de la acción de montaje rw—.
pub fn hay_udisksctl() -> bool {
    Command::new("sh")
        .arg("-c")
        .arg("command -v udisksctl")
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Deriva el device de partición del kernel a partir del disco y el índice
/// 1-based: `/dev/sdb` + 2 → `/dev/sdb2`; `/dev/nvme0n1` + 2 → `/dev/nvme0n1p2`;
/// `/dev/mmcblk0` + 1 → `/dev/mmcblk0p1`. Regla udev: se interpone `p` cuando el
/// nombre del disco termina en dígito.
pub fn device_de_particion(disco: &Path, indice: usize) -> PathBuf {
    let nombre = disco
        .file_name()
        .map(|n| n.to_string_lossy().into_owned())
        .unwrap_or_default();
    let sep = if nombre.chars().last().map(|c| c.is_ascii_digit()).unwrap_or(false) {
        "p"
    } else {
        ""
    };
    disco.with_file_name(format!("{nombre}{sep}{indice}"))
}

/// Extrae el punto de montaje de la salida de `udisksctl mount`:
/// `Mounted /dev/sdb2 at /run/media/user/MI ETIQUETA` → `/run/media/user/MI
/// ETIQUETA` (la etiqueta puede llevar espacios; se toma todo tras ` at `).
pub fn parsear_punto_montaje(salida: &str) -> Option<PathBuf> {
    let pos = salida.find(" at ")?;
    let resto = salida[pos + 4..].trim().trim_end_matches('.').trim();
    if resto.is_empty() {
        None
    } else {
        Some(PathBuf::from(resto))
    }
}

/// Busca en el contenido de `/proc/mounts` el device montado EXACTAMENTE en
/// `punto`. `/proc/mounts` escapa el espacio como `\040`.
pub fn dispositivo_montado_en(proc_mounts: &str, punto: &Path) -> Option<PathBuf> {
    for linea in proc_mounts.lines() {
        let mut campos = linea.split_whitespace();
        let (Some(dev), Some(mp)) = (campos.next(), campos.next()) else {
            continue;
        };
        if Path::new(&desescapar_mount(mp)) == punto {
            return Some(PathBuf::from(desescapar_mount(dev)));
        }
    }
    None
}

/// El punto donde `dev` está montado, según `/proc/mounts` (inverso de
/// [`dispositivo_montado_en`]). Para recuperar un montaje preexistente.
pub fn punto_de_dispositivo(proc_mounts: &str, dev: &Path) -> Option<PathBuf> {
    for linea in proc_mounts.lines() {
        let mut campos = linea.split_whitespace();
        let (Some(d), Some(mp)) = (campos.next(), campos.next()) else {
            continue;
        };
        if Path::new(&desescapar_mount(d)) == dev {
            return Some(PathBuf::from(desescapar_mount(mp)));
        }
    }
    None
}

/// Desescapa los octales de `/proc/mounts` (`\040`=espacio, `\011`=tab, etc.).
fn desescapar_mount(s: &str) -> String {
    s.replace("\\040", " ").replace("\\011", "\t").replace("\\012", "\n")
}

/// Monta la partición `dev` (lectura-escritura) vía `udisksctl` y devuelve el
/// punto de montaje. Si ya estaba montada, recupera el punto de `/proc/mounts`.
pub fn montar_rw(dev: &Path) -> io::Result<PathBuf> {
    let salida = Command::new("udisksctl")
        .args(["mount", "--no-user-interaction", "-b"])
        .arg(dev)
        .output()?;
    if salida.status.success() {
        let txt = String::from_utf8_lossy(&salida.stdout);
        if let Some(p) = parsear_punto_montaje(&txt) {
            return Ok(p);
        }
    }
    // ¿Ya estaba montada? udisksctl falla con "already mounted"; lo recuperamos.
    if let Ok(mounts) = std::fs::read_to_string("/proc/mounts") {
        if let Some(p) = punto_de_dispositivo(&mounts, dev) {
            return Ok(p);
        }
    }
    Err(io::Error::other(format!(
        "udisksctl mount falló: {}",
        String::from_utf8_lossy(&salida.stderr).trim()
    )))
}

/// Desmonta `dev` vía `udisksctl`.
pub fn desmontar(dev: &Path) -> io::Result<()> {
    let salida = Command::new("udisksctl")
        .args(["unmount", "--no-user-interaction", "-b"])
        .arg(dev)
        .output()?;
    if salida.status.success() {
        Ok(())
    } else {
        Err(io::Error::other(format!(
            "udisksctl unmount falló: {}",
            String::from_utf8_lossy(&salida.stderr).trim()
        )))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn device_de_particion_sata_y_nvme_y_mmc() {
        assert_eq!(device_de_particion(Path::new("/dev/sdb"), 2), PathBuf::from("/dev/sdb2"));
        assert_eq!(device_de_particion(Path::new("/dev/sda"), 1), PathBuf::from("/dev/sda1"));
        assert_eq!(
            device_de_particion(Path::new("/dev/nvme0n1"), 2),
            PathBuf::from("/dev/nvme0n1p2")
        );
        assert_eq!(
            device_de_particion(Path::new("/dev/mmcblk0"), 1),
            PathBuf::from("/dev/mmcblk0p1")
        );
    }

    #[test]
    fn parsea_punto_con_y_sin_punto_final_y_con_espacios() {
        assert_eq!(
            parsear_punto_montaje("Mounted /dev/sdb2 at /run/media/u/DATA"),
            Some(PathBuf::from("/run/media/u/DATA"))
        );
        assert_eq!(
            parsear_punto_montaje("Mounted /dev/sdb2 at /run/media/u/DATA."),
            Some(PathBuf::from("/run/media/u/DATA"))
        );
        // Etiqueta con espacios.
        assert_eq!(
            parsear_punto_montaje("Mounted /dev/sdb2 at /run/media/u/MI USB"),
            Some(PathBuf::from("/run/media/u/MI USB"))
        );
        assert_eq!(parsear_punto_montaje("sin la palabra clave"), None);
    }

    #[test]
    fn lee_proc_mounts_ida_y_vuelta_con_escape() {
        let mounts = "\
/dev/sda1 / ext4 rw,relatime 0 0
/dev/sdb2 /run/media/u/MI\\040USB vfat rw,nosuid 0 0
tmpfs /tmp tmpfs rw 0 0
";
        // device → punto
        assert_eq!(
            punto_de_dispositivo(mounts, Path::new("/dev/sdb2")),
            Some(PathBuf::from("/run/media/u/MI USB"))
        );
        // punto → device (desescapando el espacio)
        assert_eq!(
            dispositivo_montado_en(mounts, Path::new("/run/media/u/MI USB")),
            Some(PathBuf::from("/dev/sdb2"))
        );
        // un punto no montado
        assert_eq!(dispositivo_montado_en(mounts, Path::new("/run/media/u/OTRO")), None);
    }
}
