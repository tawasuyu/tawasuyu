//! `arje-packager` — arma un initramfs cpio (newc, formato 070701) + gzip a
//! partir de una Tarjeta Semilla y un mapa de binarios del host.
//!
//! ## Por qué cpio newc en vez de tar
//!
//! El kernel Linux lee el initramfs *exclusivamente* como cpio newc gzipeado
//! (o uncompressed). El soporte está en `init/initramfs.c` y no admite tar,
//! squashfs ni nada más. El "newc" (070701) es el dialecto portable; el
//! viejo "070702" agregaba checksums CRC que el kernel ignora.
//!
//! Spec del header (110 bytes, todos los campos numéricos son hex ASCII):
//!
//! | Offset | Tamaño | Campo        |
//! |-------:|-------:|--------------|
//! |     0  |     6  | c_magic = "070701" |
//! |     6  |     8  | c_ino        |
//! |    14  |     8  | c_mode       |
//! |    22  |     8  | c_uid        |
//! |    30  |     8  | c_gid        |
//! |    38  |     8  | c_nlink      |
//! |    46  |     8  | c_mtime      |
//! |    54  |     8  | c_filesize   |
//! |    62  |     8  | c_devmajor   |
//! |    70  |     8  | c_devminor   |
//! |    78  |     8  | c_rdevmajor  |
//! |    86  |     8  | c_rdevminor  |
//! |    94  |     8  | c_namesize   |
//! |   102  |     8  | c_check = 0  |
//!
//! Después del header viene el nombre (NUL terminado, longitud incluye el
//! NUL en c_namesize) padeado a 4 bytes desde el inicio del header. Después
//! los datos del archivo, padeados a 4 bytes desde el inicio de los datos.
//! El archive termina con una entrada de nombre "TRAILER!!!" con todos los
//! campos en cero salvo `c_nlink=1` y `c_namesize=11`.

use std::io::{self, Write};

/// Tipo de entrada en el archive. El modo (octal) y el `c_filesize` se
/// derivan de aquí — el caller no los toca.
pub enum EntryKind<'a> {
    /// Directorio. `c_filesize` = 0, modo = 0o040755.
    Directory,
    /// Archivo regular. `c_filesize` = `data.len()`, modo = `0o100000 | perm`.
    Regular { data: &'a [u8], perm: u32 },
    /// Symlink. Los datos son la ruta destino (sin NUL). Modo = 0o120777.
    Symlink { target: &'a str },
    /// Nodo de dispositivo de carácter (p. ej. `/dev/console`). Modo =
    /// `0o020000 | perm`. `(major, minor)` van a `c_rdevmajor`/`c_rdevminor`.
    /// El initramfs casi siempre necesita `/dev/console` para que arje-zero
    /// pueda abrir la consola de rescate.
    CharDev { major: u32, minor: u32, perm: u32 },
}

impl EntryKind<'_> {
    fn mode(&self) -> u32 {
        match self {
            EntryKind::Directory => 0o040755,
            EntryKind::Regular { perm, .. } => 0o100000 | (perm & 0o7777),
            EntryKind::Symlink { .. } => 0o120777,
            EntryKind::CharDev { perm, .. } => 0o020000 | (perm & 0o7777),
        }
    }

    fn filesize(&self) -> u32 {
        match self {
            EntryKind::Directory => 0,
            EntryKind::Regular { data, .. } => data.len() as u32,
            EntryKind::Symlink { target } => target.len() as u32,
            EntryKind::CharDev { .. } => 0,
        }
    }

    fn nlink(&self) -> u32 {
        // Convención cpio: dirs → 2 (`.` y `..` mínimos), todo lo demás → 1.
        match self {
            EntryKind::Directory => 2,
            _ => 1,
        }
    }

    fn rdev(&self) -> (u32, u32) {
        match self {
            EntryKind::CharDev { major, minor, .. } => (*major, *minor),
            _ => (0, 0),
        }
    }

    fn data(&self) -> &[u8] {
        match self {
            EntryKind::Regular { data, .. } => data,
            EntryKind::Symlink { target } => target.as_bytes(),
            _ => &[],
        }
    }
}

/// Escritor incremental de un archive cpio newc.
///
/// Cada `append` reserva un inode autoincremental — el kernel no exige
/// inodes únicos para regulares, pero `cpio -tv` se confunde con duplicados.
pub struct CpioWriter<W: Write> {
    out: W,
    next_ino: u32,
    /// Total de bytes ya escritos al stream — necesario para calcular el
    /// padding a 4 bytes en cualquier punto del archive.
    written: u64,
    finalized: bool,
}

impl<W: Write> CpioWriter<W> {
    pub fn new(out: W) -> Self {
        Self {
            out,
            next_ino: 1,
            written: 0,
            finalized: false,
        }
    }

    /// Agrega una entrada al archive. `name` debe ser una ruta absoluta sin
    /// el `/` inicial (convención cpio para initramfs — `/init` se escribe
    /// como `"init"`).
    pub fn append(&mut self, name: &str, kind: EntryKind<'_>) -> io::Result<()> {
        assert!(!self.finalized, "cpio archive ya finalizado");
        // El kernel acepta los nombres con o sin `/` inicial, pero el resto
        // del tooling (cpio, busybox) se confunde si está. Forzamos la
        // convención.
        assert!(
            !name.starts_with('/'),
            "cpio name no debe arrancar con '/' (fue {name:?})"
        );
        // Lo único que el kernel rechaza explícitamente es "..": rompería
        // el árbol al desempaquetar.
        assert!(name != ".." && !name.starts_with("../"), "ruta inválida {name:?}");

        let ino = self.next_ino;
        self.next_ino += 1;
        let (rmaj, rmin) = kind.rdev();
        self.write_header(
            ino,
            kind.mode(),
            kind.nlink(),
            kind.filesize(),
            name.len() as u32 + 1, // +1 por el NUL terminador
            rmaj,
            rmin,
        )?;
        self.write_name(name)?;
        self.write_data(kind.data())?;
        Ok(())
    }

    /// Cierra el archive escribiendo el trailer y devuelve el writer
    /// interno. Si no se llama, el archive queda incompleto — el kernel
    /// truncará al primer error o, peor, hará oops.
    pub fn finish(mut self) -> io::Result<W> {
        self.write_header(0, 0, 1, 0, "TRAILER!!!".len() as u32 + 1, 0, 0)?;
        self.write_name("TRAILER!!!")?;
        // Sin datos detrás del trailer. Algunos cpio padean el archive
        // completo a un múltiplo del blocksize, pero el kernel no lo
        // exige y nuestro consumidor (gzip→initramfs) tampoco.
        self.finalized = true;
        Ok(self.out)
    }

    fn write_header(
        &mut self,
        ino: u32,
        mode: u32,
        nlink: u32,
        filesize: u32,
        namesize: u32,
        rdevmajor: u32,
        rdevminor: u32,
    ) -> io::Result<()> {
        let mut buf = [0u8; 110];
        buf[..6].copy_from_slice(b"070701");
        write_hex8(&mut buf[6..14], ino);
        write_hex8(&mut buf[14..22], mode);
        write_hex8(&mut buf[22..30], 0); // uid = root
        write_hex8(&mut buf[30..38], 0); // gid = root
        write_hex8(&mut buf[38..46], nlink);
        write_hex8(&mut buf[46..54], 0); // mtime = epoch (reproducible)
        write_hex8(&mut buf[54..62], filesize);
        write_hex8(&mut buf[62..70], 0); // devmajor
        write_hex8(&mut buf[70..78], 0); // devminor
        write_hex8(&mut buf[78..86], rdevmajor);
        write_hex8(&mut buf[86..94], rdevminor);
        write_hex8(&mut buf[94..102], namesize);
        write_hex8(&mut buf[102..110], 0); // check (siempre 0 para newc)
        self.out.write_all(&buf)?;
        self.written += 110;
        Ok(())
    }

    fn write_name(&mut self, name: &str) -> io::Result<()> {
        self.out.write_all(name.as_bytes())?;
        self.out.write_all(&[0])?; // NUL terminator
        self.written += name.len() as u64 + 1;
        self.pad_to_4()?;
        Ok(())
    }

    fn write_data(&mut self, data: &[u8]) -> io::Result<()> {
        if data.is_empty() {
            return Ok(());
        }
        self.out.write_all(data)?;
        self.written += data.len() as u64;
        self.pad_to_4()?;
        Ok(())
    }

    fn pad_to_4(&mut self) -> io::Result<()> {
        let pad = (4 - (self.written % 4)) % 4;
        if pad > 0 {
            self.out.write_all(&[0; 4][..pad as usize])?;
            self.written += pad;
        }
        Ok(())
    }
}

/// Escribe `value` como 8 dígitos hexadecimales ASCII en mayúscula (newc
/// admite mayúscula o minúscula; usamos mayúscula porque es lo que GNU cpio
/// emite por default — facilita diff binarios contra archives de referencia).
fn write_hex8(out: &mut [u8], value: u32) {
    debug_assert_eq!(out.len(), 8);
    static HEX: &[u8; 16] = b"0123456789ABCDEF";
    let mut v = value;
    for i in (0..8).rev() {
        out[i] = HEX[(v & 0xf) as usize];
        v >>= 4;
    }
}

/// Comprime un buffer cpio con gzip (nivel default 6 — buen balance
/// tiempo/ratio para initramfs; el kernel descomprime una sola vez al boot).
pub fn gzip(data: &[u8]) -> io::Result<Vec<u8>> {
    use flate2::{write::GzEncoder, Compression};
    let mut enc = GzEncoder::new(Vec::with_capacity(data.len() / 2), Compression::default());
    enc.write_all(data)?;
    enc.finish()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn write_hex8_pads_y_es_uppercase() {
        let mut buf = [b'?'; 8];
        write_hex8(&mut buf, 0x1a);
        assert_eq!(&buf, b"0000001A");
        write_hex8(&mut buf, 0);
        assert_eq!(&buf, b"00000000");
        write_hex8(&mut buf, 0xdeadbeef);
        assert_eq!(&buf, b"DEADBEEF");
    }

    #[test]
    fn archive_minimo_tiene_magic_y_trailer() {
        let mut w = CpioWriter::new(Vec::new());
        w.append(
            "hola",
            EntryKind::Regular {
                data: b"mundo",
                perm: 0o644,
            },
        )
        .unwrap();
        let out = w.finish().unwrap();
        // Magic al inicio.
        assert_eq!(&out[..6], b"070701");
        // El nombre TRAILER!!! aparece literal en el archive.
        let needle = b"TRAILER!!!";
        assert!(
            out.windows(needle.len()).any(|w| w == needle),
            "archive sin trailer"
        );
        // Y el contenido del archivo también.
        assert!(
            out.windows(5).any(|w| w == b"mundo"),
            "archive sin contenido del archivo"
        );
    }

    #[test]
    fn padding_alinea_a_4_bytes() {
        // Un archivo con nombre de longitud impar genera padding tanto
        // después del header+nombre como después del data.
        let mut w = CpioWriter::new(Vec::new());
        w.append(
            "a", // 1 byte + NUL = 2 → +2 padding
            EntryKind::Regular {
                data: b"x", // 1 byte → +3 padding
                perm: 0o644,
            },
        )
        .unwrap();
        let out = w.finish().unwrap();
        // El tamaño total debe ser múltiplo de 4 (porque el último write
        // también padea).
        assert_eq!(out.len() % 4, 0, "archive sin alineación final: {} bytes", out.len());
    }

    #[test]
    fn symlink_guarda_target_como_data() {
        let mut w = CpioWriter::new(Vec::new());
        w.append("init", EntryKind::Symlink { target: "sbin/arje-zero" }).unwrap();
        let out = w.finish().unwrap();
        assert!(
            out.windows(b"sbin/arje-zero".len()).any(|w| w == b"sbin/arje-zero"),
            "symlink target no aparece en el archive"
        );
    }

    #[test]
    fn ino_se_autoincrementa() {
        let mut w = CpioWriter::new(Vec::new());
        w.append("a", EntryKind::Regular { data: b"", perm: 0o644 }).unwrap();
        w.append("b", EntryKind::Regular { data: b"", perm: 0o644 }).unwrap();
        let out = w.finish().unwrap();
        // Los inos en hex aparecen en el byte 6..14 del header.
        // Primer header en 0, segundo donde sea pero contiene "00000002".
        let ino2 = b"00000002";
        assert!(
            out.windows(8).any(|w| w == ino2),
            "no encontré el ino 2 (incremento roto)"
        );
    }

    #[test]
    fn gzip_roundtrip_con_flate2() {
        let original = b"hello cpio";
        let compressed = gzip(original).unwrap();
        // Magic gzip al inicio.
        assert_eq!(&compressed[..2], &[0x1f, 0x8b]);
        // Descomprimimos con flate2 para validar que es bien-formado.
        use flate2::read::GzDecoder;
        use std::io::Read;
        let mut dec = GzDecoder::new(&compressed[..]);
        let mut got = Vec::new();
        dec.read_to_end(&mut got).unwrap();
        assert_eq!(got, original);
    }
}
