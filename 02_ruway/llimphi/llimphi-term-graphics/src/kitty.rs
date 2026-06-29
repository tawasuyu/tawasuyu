//! kitty graphics protocol — parser de control APC + ensamblador de
//! transmisión chunked + decodificación a RGBA.
//!
//! Cuerpo APC esperado (sin el `\e_` ni el `\e\\`): `G<control>;<base64>` donde
//! `<control>` son pares `clave=valor` separados por comas. Claves que nos
//! importan:
//! - `a` acción: `t` transmitir, `T` transmitir+mostrar, `q` query, `d` delete.
//! - `f` formato: `32` RGBA (default), `24` RGB, `100` PNG/imagen embebida.
//! - `s`,`v` ancho,alto en px (para `f=24`/`f=32`).
//! - `c`,`r` columnas,filas de celdas pedidas (0 = derivar de los px).
//! - `i` id de imagen. `m` more-chunks (`1` = sigue, `0`/ausente = último).
//! - `o` compresión: `z` = zlib (RFC1950) sobre los datos pre-base64.

use std::io::Read;

use base64::Engine;

use crate::{DecodedImage, GraphicsCommand, Protocol};

/// Errores de decodificación kitty. Hoy informativos; el scanner los traga
/// (una imagen rota no debe tumbar la terminal) pero los exponemos por si el
/// caller quiere loguearlos.
#[derive(Debug)]
pub enum KittyError {
    Base64,
    Zlib,
    Image(String),
    BadDims,
}

/// Acumula la transmisión chunked (`m=1`) hasta el último chunk.
#[derive(Default)]
pub struct KittyAssembler {
    pending: Option<Pending>,
}

struct Pending {
    fmt: u32,
    s: u32,
    v: u32,
    id: u32,
    cols: u16,
    rows: u16,
    compressed: bool,
    payload_b64: Vec<u8>,
}

impl KittyAssembler {
    /// Procesa un cuerpo APC completo. Devuelve un comando cuando la
    /// transmisión cierra (o es query/delete); `None` mientras espera más
    /// chunks o si el APC no es kitty.
    pub fn feed_apc(&mut self, seq: &[u8]) -> Option<GraphicsCommand> {
        // Debe empezar con 'G' (kitty graphics). Otros APC no son nuestros.
        if seq.first() != Some(&b'G') {
            return None;
        }
        let body = &seq[1..];
        let (control, payload): (&[u8], &[u8]) = match body.iter().position(|&b| b == b';') {
            Some(p) => (&body[..p], &body[p + 1..]),
            None => (body, &[]),
        };

        let is_continuation = self.pending.is_some();

        // Sólo el primer chunk trae el control completo; las continuaciones
        // traen (a lo sumo) `m`. Parseamos siempre `m`; el resto sólo si es
        // el primer chunk.
        let mut more = false;
        let mut action = b't';
        let mut fmt = 32u32;
        let (mut s, mut v) = (0u32, 0u32);
        let mut id = 0u32;
        let (mut cols, mut rows) = (0u16, 0u16);
        let mut compressed = false;

        for tok in control.split(|&b| b == b',') {
            if tok.is_empty() {
                continue;
            }
            let mut kv = tok.splitn(2, |&b| b == b'=');
            let k = kv.next().unwrap_or(&[]);
            let val = kv.next().unwrap_or(&[]);
            match k {
                b"m" => more = val == b"1",
                b"a" => action = val.first().copied().unwrap_or(b't'),
                b"f" => fmt = parse_u32(val).unwrap_or(32),
                b"s" => s = parse_u32(val).unwrap_or(0),
                b"v" => v = parse_u32(val).unwrap_or(0),
                b"i" => id = parse_u32(val).unwrap_or(0),
                b"c" => cols = parse_u32(val).unwrap_or(0) as u16,
                b"r" => rows = parse_u32(val).unwrap_or(0) as u16,
                b"o" => compressed = val == b"z",
                _ => {}
            }
        }

        // Acciones sin payload (sólo válidas en el primer chunk).
        if !is_continuation {
            match action {
                b'q' => {
                    // Responder OK por el PTY para que el emisor sepa que hay
                    // soporte kitty. Formato: `\e_Gi=<id>;OK\e\\`.
                    let response = format!("\x1b_Gi={id};OK\x1b\\").into_bytes();
                    return Some(GraphicsCommand::Query { response });
                }
                b'd' => {
                    let id = if id == 0 { None } else { Some(id) };
                    return Some(GraphicsCommand::Delete { id });
                }
                _ => {}
            }
        }

        // Acumular payload base64.
        if is_continuation {
            let p = self.pending.as_mut().unwrap();
            p.payload_b64.extend_from_slice(payload);
        } else {
            self.pending = Some(Pending {
                fmt,
                s,
                v,
                id,
                cols,
                rows,
                compressed,
                payload_b64: payload.to_vec(),
            });
        }

        if more {
            return None;
        }

        // Último chunk: finalizar.
        let p = self.pending.take()?;
        match decode_pending(&p) {
            Ok(image) => Some(GraphicsCommand::Image {
                image,
                cols: p.cols,
                rows: p.rows,
                id: p.id,
                protocol: Protocol::Kitty,
            }),
            Err(_) => None,
        }
    }
}

fn decode_pending(p: &Pending) -> Result<DecodedImage, KittyError> {
    // 1. base64 → bytes.
    let raw = base64::engine::general_purpose::STANDARD
        .decode(&p.payload_b64)
        .map_err(|_| KittyError::Base64)?;

    // 2. zlib si o=z.
    let data = if p.compressed {
        let mut dec = flate2::read::ZlibDecoder::new(&raw[..]);
        let mut out = Vec::new();
        dec.read_to_end(&mut out).map_err(|_| KittyError::Zlib)?;
        out
    } else {
        raw
    };

    // 3. interpretar según formato.
    match p.fmt {
        32 => {
            let (w, h) = (p.s, p.v);
            let need = (w as usize) * (h as usize) * 4;
            if w == 0 || h == 0 || data.len() < need {
                return Err(KittyError::BadDims);
            }
            Ok(DecodedImage {
                width: w,
                height: h,
                rgba: data[..need].to_vec(),
            })
        }
        24 => {
            let (w, h) = (p.s, p.v);
            let need = (w as usize) * (h as usize) * 3;
            if w == 0 || h == 0 || data.len() < need {
                return Err(KittyError::BadDims);
            }
            let mut rgba = Vec::with_capacity((w as usize) * (h as usize) * 4);
            for px in data[..need].chunks_exact(3) {
                rgba.extend_from_slice(&[px[0], px[1], px[2], 255]);
            }
            Ok(DecodedImage {
                width: w,
                height: h,
                rgba,
            })
        }
        // f=100 (PNG) y cualquier otro: dejamos que el crate `image` adivine
        // el formato por los magic bytes.
        _ => {
            let img = image::load_from_memory(&data).map_err(|e| KittyError::Image(e.to_string()))?;
            let rgba = img.to_rgba8();
            Ok(DecodedImage {
                width: rgba.width(),
                height: rgba.height(),
                rgba: rgba.into_raw(),
            })
        }
    }
}

fn parse_u32(b: &[u8]) -> Option<u32> {
    std::str::from_utf8(b).ok()?.trim().parse().ok()
}
