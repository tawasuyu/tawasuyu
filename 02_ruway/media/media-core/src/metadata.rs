//! metadata — lectura de tags y carátula incrustada (U5 de `PARIDAD.md`).
//!
//! Parser puro (sin deps, sin I/O) de los dos contenedores de metadata
//! más comunes en una biblioteca de audio:
//!
//! - **ID3v2** (2.2 / 2.3 / 2.4), el tag de los `.mp3`.
//! - **Bloques nativos FLAC** (`VORBIS_COMMENT` + `PICTURE`).
//!
//! Ambos son formatos binarios bien definidos, así que se testean con
//! buffers sintéticos en CI sin tocar un archivo. El crate trabaja en
//! formato nativo (regla #4): esto **lee** tags ajenos para mostrarlos /
//! cachearlos, no para meterlos al núcleo.
//!
//! No cubre (a propósito, por ahora): unsynchronisation del tag,
//! `METADATA_BLOCK_PICTURE` en base64 dentro de un comentario Vorbis, ni
//! contenedores MP4/Matroska (esos vienen del demuxer / `foreign-av`).
//! El parser es best-effort: ante bytes corruptos corta y devuelve lo que
//! haya leído en vez de panickear.

use serde::{Deserialize, Serialize};

/// Carátula incrustada: los bytes de la imagen + su MIME type. El
/// frontend la decodifica (PNG/JPEG) y la pinta — igual que cualquier
/// `peniko::Image`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CoverArt {
    pub mime: String,
    pub data: Vec<u8>,
}

/// Tags normalizados de un medio. Todos opcionales: un archivo puede
/// traer sólo algunos. Agnóstico del contenedor de origen.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Metadata {
    pub title: Option<String>,
    pub artist: Option<String>,
    pub album: Option<String>,
    pub year: Option<String>,
    pub track: Option<String>,
    pub genre: Option<String>,
    pub cover: Option<CoverArt>,
}

impl Metadata {
    /// `true` si no se extrajo ningún tag ni carátula.
    pub fn is_empty(&self) -> bool {
        self.title.is_none()
            && self.artist.is_none()
            && self.album.is_none()
            && self.year.is_none()
            && self.track.is_none()
            && self.genre.is_none()
            && self.cover.is_none()
    }
}

/// Autodetecta el contenedor por su firma y extrae la metadata. Devuelve
/// un [`Metadata`] vacío (no `None`) si no reconoce el formato, así el
/// caller siempre tiene una estructura con la que trabajar.
pub fn parse(bytes: &[u8]) -> Metadata {
    if bytes.starts_with(b"ID3") {
        parse_id3v2(bytes).unwrap_or_default()
    } else if bytes.starts_with(b"fLaC") {
        parse_flac(bytes).unwrap_or_default()
    } else {
        Metadata::default()
    }
}

// ============================================================
// ID3v2
// ============================================================

/// Parsea un tag ID3v2 (2.2/2.3/2.4) desde el inicio de `bytes`.
/// Devuelve `None` sólo si el header no es un ID3v2 válido; un tag con
/// frames corruptos a mitad devuelve lo leído hasta ahí.
pub fn parse_id3v2(bytes: &[u8]) -> Option<Metadata> {
    if bytes.len() < 10 || &bytes[0..3] != b"ID3" {
        return None;
    }
    let major = bytes[3];
    let flags = bytes[5];
    // El tamaño del tag es synchsafe (4×7 bits).
    let tag_size = synchsafe(&bytes[6..10])? as usize;
    let mut pos = 10usize;
    let end = (10 + tag_size).min(bytes.len());

    // Extended header (flag bit 6): lo saltamos best-effort.
    if flags & 0x40 != 0 {
        if major >= 4 {
            // v2.4: tamaño synchsafe, incluye los 4 bytes del tamaño.
            let sz = synchsafe(bytes.get(pos..pos + 4)?)? as usize;
            pos += sz.max(4);
        } else {
            // v2.3: u32 plano = tamaño del ext header SIN contar esos 4.
            let sz = be_u32(bytes.get(pos..pos + 4)?)? as usize;
            pos += 4 + sz;
        }
    }

    let v22 = major == 2;
    let id_len = if v22 { 3 } else { 4 };
    let header_len = if v22 { 6 } else { 10 };

    let mut md = Metadata::default();
    while pos + header_len <= end {
        let id = &bytes[pos..pos + id_len];
        // Padding: un frame que arranca en 0x00 marca el relleno final.
        if id[0] == 0 {
            break;
        }
        let size = if v22 {
            be_u24(&bytes[pos + 3..pos + 6])? as usize
        } else if major >= 4 {
            // v2.4: tamaño del frame es synchsafe.
            synchsafe(&bytes[pos + 4..pos + 8])? as usize
        } else {
            // v2.3: tamaño plano.
            be_u32(&bytes[pos + 4..pos + 8])? as usize
        };
        // El contenido va tras el header completo (los flags, si los hay,
        // quedan entre el tamaño y el contenido y no nos interesan).
        let content_start = pos + header_len;
        let content_end = content_start + size;
        if size == 0 || content_end > end {
            break;
        }
        let content = &bytes[content_start..content_end];
        dispatch_id3(&mut md, id, content, v22);
        pos = content_end;
    }
    Some(md)
}

/// Asigna el contenido de un frame al campo que corresponde.
fn dispatch_id3(md: &mut Metadata, id: &[u8], content: &[u8], v22: bool) {
    // IDs de v2.3/2.4 (4 chars) y sus equivalentes v2.2 (3 chars).
    let is = |a: &[u8], b: &[u8]| id == a || (v22 && id == b);
    if is(b"TIT2", b"TT2") {
        md.title = text_value(content);
    } else if is(b"TPE1", b"TP1") {
        md.artist = text_value(content);
    } else if is(b"TALB", b"TAL") {
        md.album = text_value(content);
    } else if is(b"TRCK", b"TRK") {
        md.track = text_value(content);
    } else if is(b"TCON", b"TCO") {
        md.genre = text_value(content);
    } else if id == b"TDRC" || is(b"TYER", b"TYE") {
        // TDRC (v2.4) puede ser una fecha completa; nos quedamos con el año.
        md.year = text_value(content).map(|s| year_of(&s));
    } else if is(b"APIC", b"PIC") {
        if let Some(cover) = parse_picture_frame(content, v22) {
            md.cover = Some(cover);
        }
    }
}

/// Decodifica un frame de texto (primer valor) según su byte de encoding.
fn text_value(content: &[u8]) -> Option<String> {
    let enc = *content.first()?;
    let raw = decode_text(enc, &content[1..]);
    // v2.4 permite múltiples valores separados por NUL; tomamos el primero.
    let v = raw.split('\u{0}').next().unwrap_or("").trim().to_string();
    if v.is_empty() {
        None
    } else {
        Some(v)
    }
}

/// Extrae los 4 primeros dígitos de una fecha ("2021-05-01" → "2021").
fn year_of(s: &str) -> String {
    let digits: String = s.chars().take_while(|c| c.is_ascii_digit()).collect();
    if digits.len() >= 4 {
        digits[..4].to_string()
    } else {
        s.to_string()
    }
}

/// Parsea un frame APIC (v2.3/2.4) o PIC (v2.2) a [`CoverArt`].
fn parse_picture_frame(content: &[u8], v22: bool) -> Option<CoverArt> {
    let enc = *content.first()?;
    let mut i = 1usize;
    let mime = if v22 {
        // PIC: formato de imagen como 3 chars ("JPG"/"PNG").
        let fmt = content.get(1..4)?;
        i = 4;
        format_to_mime(fmt)
    } else {
        // APIC: MIME type latin1 NUL-terminado.
        let start = i;
        while i < content.len() && content[i] != 0 {
            i += 1;
        }
        let mime_str = decode_latin1(&content[start..i]);
        i += 1; // salta el NUL
        normalize_mime(&mime_str)
    };
    // Picture type (1 byte).
    let _pic_type = *content.get(i)?;
    i += 1;
    // Descripción: NUL-terminada en el encoding.
    i = skip_terminated(content, i, enc);
    let data = content.get(i..)?.to_vec();
    if data.is_empty() {
        return None;
    }
    Some(CoverArt { mime, data })
}

// ============================================================
// FLAC — bloques de metadata nativos
// ============================================================

/// Parsea los bloques de metadata FLAC (`VORBIS_COMMENT` + `PICTURE`).
pub fn parse_flac(bytes: &[u8]) -> Option<Metadata> {
    if !bytes.starts_with(b"fLaC") {
        return None;
    }
    let mut md = Metadata::default();
    let mut pos = 4usize;
    loop {
        if pos + 4 > bytes.len() {
            break;
        }
        let header = bytes[pos];
        let is_last = header & 0x80 != 0;
        let block_type = header & 0x7f;
        let len = be_u24(&bytes[pos + 1..pos + 4])? as usize;
        let body_start = pos + 4;
        let body_end = body_start + len;
        if body_end > bytes.len() {
            break;
        }
        let body = &bytes[body_start..body_end];
        match block_type {
            4 => parse_vorbis_comment(&mut md, body),
            6 => {
                if let Some(c) = parse_flac_picture(body) {
                    md.cover = Some(c);
                }
            }
            _ => {}
        }
        if is_last {
            break;
        }
        pos = body_end;
    }
    Some(md)
}

/// Bloque VORBIS_COMMENT: vendor + lista de "KEY=value". Enteros en
/// **little-endian** (a diferencia del resto de FLAC, que es big-endian).
fn parse_vorbis_comment(md: &mut Metadata, body: &[u8]) {
    let mut i = 0usize;
    let vendor_len = match le_u32(body.get(i..i + 4).unwrap_or(&[])) {
        Some(v) => v as usize,
        None => return,
    };
    i += 4 + vendor_len;
    let count = match le_u32(body.get(i..i + 4).unwrap_or(&[])) {
        Some(v) => v as usize,
        None => return,
    };
    i += 4;
    for _ in 0..count {
        let clen = match le_u32(body.get(i..i + 4).unwrap_or(&[])) {
            Some(v) => v as usize,
            None => return,
        };
        i += 4;
        let comment = match body.get(i..i + clen) {
            Some(c) => c,
            None => return,
        };
        i += clen;
        if let Some(eq) = comment.iter().position(|&b| b == b'=') {
            let key = String::from_utf8_lossy(&comment[..eq]).to_ascii_uppercase();
            let val = String::from_utf8_lossy(&comment[eq + 1..]).trim().to_string();
            if val.is_empty() {
                continue;
            }
            match key.as_str() {
                "TITLE" => md.title.get_or_insert(val),
                "ARTIST" => md.artist.get_or_insert(val),
                "ALBUM" => md.album.get_or_insert(val),
                "DATE" | "YEAR" => md.year.get_or_insert(year_of(&val)),
                "TRACKNUMBER" => md.track.get_or_insert(val),
                "GENRE" => md.genre.get_or_insert(val),
                _ => continue,
            };
        }
    }
}

/// Bloque PICTURE de FLAC: enteros big-endian. Estructura idéntica al
/// payload de `METADATA_BLOCK_PICTURE`.
fn parse_flac_picture(body: &[u8]) -> Option<CoverArt> {
    let mut i = 0usize;
    let _pic_type = be_u32(body.get(i..i + 4)?)?;
    i += 4;
    let mime_len = be_u32(body.get(i..i + 4)?)? as usize;
    i += 4;
    let mime = String::from_utf8_lossy(body.get(i..i + mime_len)?).to_string();
    i += mime_len;
    let desc_len = be_u32(body.get(i..i + 4)?)? as usize;
    i += 4 + desc_len;
    // width, height, depth, colors → 4×u32 que saltamos.
    i += 16;
    let data_len = be_u32(body.get(i..i + 4)?)? as usize;
    i += 4;
    let data = body.get(i..i + data_len)?.to_vec();
    if data.is_empty() {
        return None;
    }
    Some(CoverArt {
        mime: normalize_mime(&mime),
        data,
    })
}

// ============================================================
// Helpers de bajo nivel
// ============================================================

/// Entero synchsafe de 4 bytes (7 bits útiles por byte, MSB en 0).
fn synchsafe(b: &[u8]) -> Option<u32> {
    if b.len() < 4 {
        return None;
    }
    Some(
        ((b[0] as u32 & 0x7f) << 21)
            | ((b[1] as u32 & 0x7f) << 14)
            | ((b[2] as u32 & 0x7f) << 7)
            | (b[3] as u32 & 0x7f),
    )
}

fn be_u32(b: &[u8]) -> Option<u32> {
    if b.len() < 4 {
        return None;
    }
    Some(u32::from_be_bytes([b[0], b[1], b[2], b[3]]))
}

fn le_u32(b: &[u8]) -> Option<u32> {
    if b.len() < 4 {
        return None;
    }
    Some(u32::from_le_bytes([b[0], b[1], b[2], b[3]]))
}

fn be_u24(b: &[u8]) -> Option<u32> {
    if b.len() < 3 {
        return None;
    }
    Some(((b[0] as u32) << 16) | ((b[1] as u32) << 8) | b[2] as u32)
}

/// Decodifica texto ID3 según el byte de encoding, sin recortar NULs
/// (eso lo hace el caller). 0=Latin1, 1=UTF-16+BOM, 2=UTF-16BE, 3=UTF-8.
fn decode_text(encoding: u8, data: &[u8]) -> String {
    match encoding {
        1 => decode_utf16_bom(data),
        2 => decode_utf16be(data),
        3 => String::from_utf8_lossy(data).into_owned(),
        _ => decode_latin1(data),
    }
}

/// Latin1 (ISO-8859-1): cada byte mapea directo a su code point U+00xx.
fn decode_latin1(data: &[u8]) -> String {
    data.iter().map(|&b| b as char).collect()
}

fn decode_utf16_bom(data: &[u8]) -> String {
    if data.len() >= 2 && data[0] == 0xFF && data[1] == 0xFE {
        decode_utf16(&data[2..], false)
    } else if data.len() >= 2 && data[0] == 0xFE && data[1] == 0xFF {
        decode_utf16(&data[2..], true)
    } else {
        // Sin BOM: asumimos LE (lo más común en la práctica).
        decode_utf16(data, false)
    }
}

fn decode_utf16be(data: &[u8]) -> String {
    decode_utf16(data, true)
}

fn decode_utf16(data: &[u8], big_endian: bool) -> String {
    let units: Vec<u16> = data
        .chunks_exact(2)
        .map(|c| {
            if big_endian {
                u16::from_be_bytes([c[0], c[1]])
            } else {
                u16::from_le_bytes([c[0], c[1]])
            }
        })
        .collect();
    String::from_utf16_lossy(&units)
}

/// Avanza `i` hasta pasar el terminador NUL de una cadena en `encoding`.
/// Para UTF-16 (enc 1/2) el terminador son dos NUL alineados.
fn skip_terminated(data: &[u8], mut i: usize, encoding: u8) -> usize {
    if encoding == 1 || encoding == 2 {
        while i + 1 < data.len() {
            if data[i] == 0 && data[i + 1] == 0 {
                return i + 2;
            }
            i += 2;
        }
        data.len()
    } else {
        while i < data.len() {
            if data[i] == 0 {
                return i + 1;
            }
            i += 1;
        }
        data.len()
    }
}

/// Normaliza un MIME de imagen ("image/jpg" → "image/jpeg"; sin barra cae
/// a [`format_to_mime`]).
fn normalize_mime(mime: &str) -> String {
    let m = mime.trim().to_ascii_lowercase();
    if m == "image/jpg" {
        "image/jpeg".to_string()
    } else if m.contains('/') {
        m
    } else {
        format_to_mime(mime.as_bytes())
    }
}

/// Mapea un formato corto ("JPG"/"PNG"/"GIF") a su MIME.
fn format_to_mime(fmt: &[u8]) -> String {
    let f = String::from_utf8_lossy(fmt).trim().to_ascii_uppercase();
    match f.as_str() {
        "JPG" | "JPEG" => "image/jpeg".to_string(),
        "PNG" => "image/png".to_string(),
        "GIF" => "image/gif".to_string(),
        "BMP" => "image/bmp".to_string(),
        "WEBP" => "image/webp".to_string(),
        _ => "application/octet-stream".to_string(),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---------- builders sintéticos ----------

    /// Frame de texto ID3v2.3 (encoding 3 = UTF-8).
    fn id3v23_text_frame(id: &[u8; 4], text: &str) -> Vec<u8> {
        let mut content = vec![3u8]; // encoding UTF-8
        content.extend_from_slice(text.as_bytes());
        let mut f = Vec::new();
        f.extend_from_slice(id);
        f.extend_from_slice(&(content.len() as u32).to_be_bytes()); // tamaño plano
        f.extend_from_slice(&[0, 0]); // flags
        f.extend_from_slice(&content);
        f
    }

    /// Envuelve frames en un tag ID3v2.3 con su header (size synchsafe).
    fn id3v23_tag(frames: &[u8]) -> Vec<u8> {
        let mut tag = Vec::new();
        tag.extend_from_slice(b"ID3");
        tag.extend_from_slice(&[3, 0, 0]); // v2.3.0, sin flags
        // size synchsafe del cuerpo.
        let size = frames.len() as u32;
        tag.extend_from_slice(&to_synchsafe(size));
        tag.extend_from_slice(frames);
        tag
    }

    fn to_synchsafe(n: u32) -> [u8; 4] {
        [
            ((n >> 21) & 0x7f) as u8,
            ((n >> 14) & 0x7f) as u8,
            ((n >> 7) & 0x7f) as u8,
            (n & 0x7f) as u8,
        ]
    }

    #[test]
    fn id3v23_lee_tags_de_texto() {
        let mut frames = Vec::new();
        frames.extend(id3v23_text_frame(b"TIT2", "Canción"));
        frames.extend(id3v23_text_frame(b"TPE1", "Artista"));
        frames.extend(id3v23_text_frame(b"TALB", "Álbum"));
        frames.extend(id3v23_text_frame(b"TYER", "2021"));
        frames.extend(id3v23_text_frame(b"TRCK", "3/12"));
        frames.extend(id3v23_text_frame(b"TCON", "Rock"));
        let tag = id3v23_tag(&frames);

        let md = parse(&tag);
        assert_eq!(md.title.as_deref(), Some("Canción"));
        assert_eq!(md.artist.as_deref(), Some("Artista"));
        assert_eq!(md.album.as_deref(), Some("Álbum"));
        assert_eq!(md.year.as_deref(), Some("2021"));
        assert_eq!(md.track.as_deref(), Some("3/12"));
        assert_eq!(md.genre.as_deref(), Some("Rock"));
        assert!(md.cover.is_none());
        assert!(!md.is_empty());
    }

    #[test]
    fn id3v2_latin1_decodifica_acentos() {
        // Encoding 0 = Latin1: "Café" en bytes ISO-8859-1 (é = 0xE9).
        let bytes_latin1 = [b'C', b'a', b'f', 0xE9];
        let mut content = vec![0u8]; // latin1
        content.extend_from_slice(&bytes_latin1);
        let mut frame = Vec::new();
        frame.extend_from_slice(b"TIT2");
        frame.extend_from_slice(&(content.len() as u32).to_be_bytes());
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(&content);
        let tag = id3v23_tag(&frame);

        let md = parse(&tag);
        assert_eq!(md.title.as_deref(), Some("Café"));
    }

    #[test]
    fn id3v24_tdrc_extrae_anio() {
        // v2.4: tamaño de frame synchsafe; TDRC con fecha completa.
        let text = "2019-08-15";
        let mut content = vec![3u8]; // encoding UTF-8
        content.extend_from_slice(text.as_bytes());
        let mut frame = Vec::new();
        frame.extend_from_slice(b"TDRC");
        frame.extend_from_slice(&to_synchsafe(content.len() as u32)); // synchsafe!
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(&content);

        let mut tag = Vec::new();
        tag.extend_from_slice(b"ID3");
        tag.extend_from_slice(&[4, 0, 0]); // v2.4
        tag.extend_from_slice(&to_synchsafe(frame.len() as u32));
        tag.extend_from_slice(&frame);

        let md = parse(&tag);
        assert_eq!(md.year.as_deref(), Some("2019"));
    }

    #[test]
    fn id3v2_utf16_con_bom() {
        // TIT2 en UTF-16 LE con BOM.
        let s = "Ω♪";
        let mut content = vec![1u8]; // encoding UTF-16+BOM
        content.extend_from_slice(&[0xFF, 0xFE]); // BOM LE
        for u in s.encode_utf16() {
            content.extend_from_slice(&u.to_le_bytes());
        }
        let mut frame = Vec::new();
        frame.extend_from_slice(b"TIT2");
        frame.extend_from_slice(&(content.len() as u32).to_be_bytes());
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(&content);
        let tag = id3v23_tag(&frame);

        let md = parse(&tag);
        assert_eq!(md.title.as_deref(), Some("Ω♪"));
    }

    #[test]
    fn id3v2_apic_extrae_caratula() {
        // APIC: enc(0) + mime "image/png\0" + tipo(3) + desc "\0" + datos.
        let img = vec![0x89, b'P', b'N', b'G', 1, 2, 3, 4];
        let mut content = vec![0u8]; // encoding latin1
        content.extend_from_slice(b"image/png");
        content.push(0); // fin del MIME
        content.push(3); // picture type = front cover
        content.push(0); // descripción vacía (terminador)
        content.extend_from_slice(&img);

        let mut frame = Vec::new();
        frame.extend_from_slice(b"APIC");
        frame.extend_from_slice(&(content.len() as u32).to_be_bytes());
        frame.extend_from_slice(&[0, 0]);
        frame.extend_from_slice(&content);
        let tag = id3v23_tag(&frame);

        let md = parse(&tag);
        let cover = md.cover.expect("hay carátula");
        assert_eq!(cover.mime, "image/png");
        assert_eq!(cover.data, img);
    }

    #[test]
    fn id3v22_frames_de_3_chars() {
        // v2.2: ID 3 chars, tamaño 3 bytes, sin flags.
        fn v22_frame(id: &[u8; 3], text: &str) -> Vec<u8> {
            let mut content = vec![0u8];
            content.extend_from_slice(text.as_bytes());
            let mut f = Vec::new();
            f.extend_from_slice(id);
            let sz = content.len() as u32;
            f.extend_from_slice(&[(sz >> 16) as u8, (sz >> 8) as u8, sz as u8]);
            f.extend_from_slice(&content);
            f
        }
        let mut frames = Vec::new();
        frames.extend(v22_frame(b"TT2", "Titulo"));
        frames.extend(v22_frame(b"TP1", "Autor"));
        let mut tag = Vec::new();
        tag.extend_from_slice(b"ID3");
        tag.extend_from_slice(&[2, 0, 0]); // v2.2
        tag.extend_from_slice(&to_synchsafe(frames.len() as u32));
        tag.extend_from_slice(&frames);

        let md = parse(&tag);
        assert_eq!(md.title.as_deref(), Some("Titulo"));
        assert_eq!(md.artist.as_deref(), Some("Autor"));
    }

    #[test]
    fn no_id3_devuelve_vacio() {
        assert!(parse(b"not a tag at all").is_empty());
        assert!(parse_id3v2(b"ID3").is_none()); // muy corto
    }

    // ---------- FLAC ----------

    fn flac_block(block_type: u8, body: &[u8], last: bool) -> Vec<u8> {
        let mut b = Vec::new();
        let header = if last { 0x80 | block_type } else { block_type };
        b.push(header);
        let len = body.len() as u32;
        b.extend_from_slice(&[(len >> 16) as u8, (len >> 8) as u8, len as u8]);
        b.extend_from_slice(body);
        b
    }

    fn vorbis_comment_body(comments: &[&str]) -> Vec<u8> {
        let mut body = Vec::new();
        let vendor = b"test";
        body.extend_from_slice(&(vendor.len() as u32).to_le_bytes());
        body.extend_from_slice(vendor);
        body.extend_from_slice(&(comments.len() as u32).to_le_bytes());
        for c in comments {
            body.extend_from_slice(&(c.len() as u32).to_le_bytes());
            body.extend_from_slice(c.as_bytes());
        }
        body
    }

    #[test]
    fn flac_vorbis_comment() {
        let body = vorbis_comment_body(&[
            "TITLE=Mi Tema",
            "ARTIST=Banda",
            "ALBUM=Disco",
            "DATE=2020-01-01",
            "TRACKNUMBER=5",
            "GENRE=Jazz",
        ]);
        let mut flac = Vec::new();
        flac.extend_from_slice(b"fLaC");
        flac.extend(flac_block(4, &body, true)); // VORBIS_COMMENT, último

        let md = parse(&flac);
        assert_eq!(md.title.as_deref(), Some("Mi Tema"));
        assert_eq!(md.artist.as_deref(), Some("Banda"));
        assert_eq!(md.album.as_deref(), Some("Disco"));
        assert_eq!(md.year.as_deref(), Some("2020"));
        assert_eq!(md.track.as_deref(), Some("5"));
        assert_eq!(md.genre.as_deref(), Some("Jazz"));
    }

    #[test]
    fn flac_picture_block() {
        let img = vec![0xFF, 0xD8, 0xFF, 0xE0, 9, 9]; // JPEG-ish
        let mut body = Vec::new();
        body.extend_from_slice(&3u32.to_be_bytes()); // pic type front
        let mime = b"image/jpeg";
        body.extend_from_slice(&(mime.len() as u32).to_be_bytes());
        body.extend_from_slice(mime);
        body.extend_from_slice(&0u32.to_be_bytes()); // desc len 0
        body.extend_from_slice(&[0u8; 16]); // w,h,depth,colors
        body.extend_from_slice(&(img.len() as u32).to_be_bytes());
        body.extend_from_slice(&img);

        let mut flac = Vec::new();
        flac.extend_from_slice(b"fLaC");
        // Un bloque cualquiera primero (no-último), luego el PICTURE último.
        flac.extend(flac_block(0, &[0u8; 4], false)); // "STREAMINFO" dummy
        flac.extend(flac_block(6, &body, true));

        let md = parse(&flac);
        let cover = md.cover.expect("carátula FLAC");
        assert_eq!(cover.mime, "image/jpeg");
        assert_eq!(cover.data, img);
    }

    #[test]
    fn flac_corrupto_no_panickea() {
        // "fLaC" + header de bloque que promete más bytes de los que hay.
        let mut flac = Vec::new();
        flac.extend_from_slice(b"fLaC");
        flac.push(4); // VORBIS_COMMENT, no último
        flac.extend_from_slice(&[0xFF, 0xFF, 0xFF]); // len enorme
        let md = parse(&flac);
        assert!(md.is_empty());
    }

    #[test]
    fn metadata_round_trip_ron() {
        let md = Metadata {
            title: Some("T".into()),
            artist: Some("A".into()),
            cover: Some(CoverArt {
                mime: "image/png".into(),
                data: vec![1, 2, 3],
            }),
            ..Default::default()
        };
        let txt = ron::ser::to_string(&md).expect("serializa");
        let back: Metadata = ron::from_str(&txt).expect("deserializa");
        assert_eq!(md, back);
    }
}
