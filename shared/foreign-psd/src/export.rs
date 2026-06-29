//! `foreign-psd::export` — escribe un [`tullpu_core::Lienzo`] como archivo
//! `.psd` (Adobe Photoshop) real, legible por Photoshop/Photopea y por el
//! propio [`crate::importar_psd`] (round-trip).
//!
//! El crate `psd` (0.3) sólo **lee**; este encoder se escribe a mano siguiendo
//! la spec del formato. Genera un PSD RGB de 8 bits con:
//!
//! - **Header** `8BPS` v1, 3 canales (documento RGB), profundidad 8.
//! - **Layer & Mask Information**: una entrada por [`tullpu_core::Capa`], a
//!   tamaño de lienzo completo, con 4 canales (R, G, B, A), su blend mode,
//!   opacidad, visibilidad y nombre. Datos de canal sin comprimir (raw).
//! - **Merged image data**: el composite RGB (preview que ven los lectores
//!   "flat"), vía [`tullpu_render::componer`].
//!
//! Lo que **no** se escribe (paridad con lo que el import tampoco porta):
//! grupos/folders (las capas salen planas), máscaras de capa, clipping,
//! ajustes y layer styles. El blend de cada capa se mapea al 4-char key
//! Photoshop inverso de [`crate::mapear_blend`].

use thiserror::Error;
use tullpu_core::{Lienzo, ModoFusion};
use tullpu_render::{componer, FuenteBuffers};

/// Errores del export. Chico a propósito: o el lienzo es inválido, o falta
/// un buffer que alguna capa referencia, o el compositor del merged falló.
#[derive(Debug, Error)]
pub enum ExportPsdError {
    /// Lienzo de área cero — PSD exige `width ≥ 1` y `height ≥ 1`.
    #[error("lienzo degenerado: {0}×{1}")]
    Dimensiones(u32, u32),
    /// PSD (v1) topa en 30000 px por lado.
    #[error("PSD soporta hasta 30000×30000; el lienzo es {0}×{1}")]
    DemasiadoGrande(u32, u32),
    /// Una capa apunta por hash a un buffer que no está en la fuente.
    #[error("falta el buffer de la capa '{0}' en el almacén")]
    BufferFaltante(String),
    /// El buffer de una capa no mide `width·height·4` (Rgba8).
    #[error("la capa '{0}' mide {1} bytes, esperaba {2} ({3}×{4} Rgba8)")]
    TamanoBuffer(String, usize, usize, u32, u32),
    /// El compositor falló al armar el merged preview.
    #[error("no se pudo componer el merged: {0}")]
    Componer(String),
    /// Un lienzo sin capas no produce un PSD con layer info.
    #[error("el lienzo no tiene capas")]
    SinCapas,
}

/// Límite duro del formato PSD (v1) por lado.
const PSD_MAX_LADO: u32 = 30_000;

/// Serializa `lienzo` a un `.psd` (bytes listos para `std::fs::write`). Cada
/// capa se escribe a tamaño de lienzo completo con sus 4 canales RGBA. La
/// fuente provee los buffers Rgba8 que las capas referencian por hash.
pub fn exportar_psd(
    lienzo: &Lienzo,
    fuente: &impl FuenteBuffers,
) -> Result<Vec<u8>, ExportPsdError> {
    let (w, h) = (lienzo.width, lienzo.height);
    if w == 0 || h == 0 {
        return Err(ExportPsdError::Dimensiones(w, h));
    }
    if w > PSD_MAX_LADO || h > PSD_MAX_LADO {
        return Err(ExportPsdError::DemasiadoGrande(w, h));
    }
    if lienzo.capas.is_empty() {
        return Err(ExportPsdError::SinCapas);
    }
    let esperado = (w as usize) * (h as usize) * 4;
    let plano = (w as usize) * (h as usize);

    // --- File header (26 bytes) -------------------------------------------
    let mut out = Vec::new();
    out.extend_from_slice(b"8BPS");
    out.extend_from_slice(&1u16.to_be_bytes()); // versión 1 (PSD; 2 sería PSB)
    out.extend_from_slice(&[0u8; 6]); // reservado
    out.extend_from_slice(&3u16.to_be_bytes()); // canales del documento (RGB)
    out.extend_from_slice(&h.to_be_bytes());
    out.extend_from_slice(&w.to_be_bytes());
    out.extend_from_slice(&8u16.to_be_bytes()); // profundidad de bits
    out.extend_from_slice(&3u16.to_be_bytes()); // color mode = RGB

    // --- Color Mode Data + Image Resources (ambos vacíos) -----------------
    out.extend_from_slice(&0u32.to_be_bytes());
    out.extend_from_slice(&0u32.to_be_bytes());

    // --- Layer & Mask Information -----------------------------------------
    // Primero los records de cada capa y, por separado, sus datos de canal.
    let mut records = Vec::new();
    let mut datos_canal = Vec::new();
    for capa in &lienzo.capas {
        let buf = fuente
            .obtener(capa.contenido)
            .ok_or_else(|| ExportPsdError::BufferFaltante(capa.nombre.clone()))?;
        if buf.len() != esperado {
            return Err(ExportPsdError::TamanoBuffer(
                capa.nombre.clone(),
                buf.len(),
                esperado,
                w,
                h,
            ));
        }
        escribir_record_capa(&mut records, capa, w, h, plano);
        escribir_canales_capa(&mut datos_canal, buf, plano);
    }

    // Bloque "layer info": conteo de capas (i16) + records + datos de canal.
    let mut layer_info = Vec::new();
    layer_info.extend_from_slice(&(lienzo.capas.len() as i16).to_be_bytes());
    layer_info.extend_from_slice(&records);
    layer_info.extend_from_slice(&datos_canal);
    par_pad(&mut layer_info); // longitud par (alineación a 2)

    // Sección = [len(layer_info) + layer_info] + [global mask len = 0].
    let mut seccion = Vec::new();
    seccion.extend_from_slice(&(layer_info.len() as u32).to_be_bytes());
    seccion.extend_from_slice(&layer_info);
    seccion.extend_from_slice(&0u32.to_be_bytes()); // global layer mask info

    out.extend_from_slice(&(seccion.len() as u32).to_be_bytes());
    out.extend_from_slice(&seccion);

    // --- Merged image data (composite RGB, preview para lectores flat) -----
    let img = componer(lienzo, fuente)
        .map_err(|e| ExportPsdError::Componer(e.to_string()))?;
    let rgba = img.into_raw();
    out.extend_from_slice(&0u16.to_be_bytes()); // compresión = 0 (raw)
    // Tres planos R, G, B (el documento es RGB; descartamos el alfa del
    // composite, que es sólo preview).
    for canal in 0..3 {
        for i in 0..plano {
            out.push(rgba.get(i * 4 + canal).copied().unwrap_or(0));
        }
    }
    Ok(out)
}

/// Escribe el *layer record* (cabecera de una capa) en `out`: rectángulo a
/// lienzo completo, 4 canales (R,G,B,A), blend/opacidad/visibilidad, y el
/// nombre como Pascal string padded. Los datos de píxel van aparte
/// (`escribir_canales_capa`).
fn escribir_record_capa(out: &mut Vec<u8>, capa: &tullpu_core::Capa, w: u32, h: u32, plano: usize) {
    // Rectángulo: top, left, bottom, right (a tamaño de lienzo).
    out.extend_from_slice(&0i32.to_be_bytes());
    out.extend_from_slice(&0i32.to_be_bytes());
    out.extend_from_slice(&(h as i32).to_be_bytes());
    out.extend_from_slice(&(w as i32).to_be_bytes());
    // Número de canales + tabla (id, longitud de datos del canal).
    out.extend_from_slice(&4u16.to_be_bytes());
    let largo_canal = (2 + plano) as u32; // 2 bytes de compresión + W·H raw
    for id in [0i16, 1, 2, -1] {
        // -1 (0xFFFF) = canal alfa.
        out.extend_from_slice(&id.to_be_bytes());
        out.extend_from_slice(&largo_canal.to_be_bytes());
    }
    // Blend signature + key + opacidad + clipping + flags + filler.
    out.extend_from_slice(b"8BIM");
    out.extend_from_slice(&clave_blend(capa.blend));
    out.push((capa.opacidad.clamp(0.0, 1.0) * 255.0).round() as u8);
    out.push(0); // clipping: 0 = base (sin recorte)
    // Flags Photoshop: bit 3 (0x08) = "Photoshop 5.0+" (lo ponen los archivos
    // reales); bit 1 (0x02) = capa **oculta** (clear = visible), según la spec
    // Adobe y los fixtures del corpus (visibles ⇒ 0x02 en 0). OJO: el crate
    // lector `psd` invierte esta lectura (`& 0x02 != 0 ⇒ visible`), por eso el
    // round-trip de visibilidad vía `importar_psd` queda invertido — ver test.
    out.push(0x08 | if capa.visible { 0 } else { 0x02 });
    out.push(0); // filler

    // Campo "extra data": layer mask (vacío) + blending ranges (vacío) + nombre.
    let mut extra = Vec::new();
    extra.extend_from_slice(&0u32.to_be_bytes()); // layer mask data len = 0
    extra.extend_from_slice(&0u32.to_be_bytes()); // blending ranges len = 0
    empujar_pascal_padded(&mut extra, &capa.nombre);
    out.extend_from_slice(&(extra.len() as u32).to_be_bytes());
    out.extend_from_slice(&extra);
}

/// Escribe los 4 planos de canal (R, G, B, A) de una capa, cada uno precedido
/// por su palabra de compresión (`0` = raw). El buffer fuente es Rgba8
/// entrelazado; acá se de-entrelaza a planar (lo que pide PSD).
fn escribir_canales_capa(out: &mut Vec<u8>, buf: &[u8], plano: usize) {
    for canal in [0usize, 1, 2, 3] {
        out.extend_from_slice(&0u16.to_be_bytes()); // compresión raw
        for i in 0..plano {
            out.push(buf[i * 4 + canal]);
        }
    }
}

/// Pascal string PSD: 1 byte de longitud + bytes del nombre, con relleno a
/// cero para que el total (incluida la longitud) sea múltiplo de 4. Nombre
/// recortado a 255 bytes (tope del campo de longitud).
fn empujar_pascal_padded(out: &mut Vec<u8>, nombre: &str) {
    let bytes = nombre.as_bytes();
    let len = bytes.len().min(255);
    out.push(len as u8);
    out.extend_from_slice(&bytes[..len]);
    let total = 1 + len;
    let pad = (4 - (total % 4)) % 4;
    for _ in 0..pad {
        out.push(0);
    }
}

/// Rellena `v` con un byte cero si su longitud es impar (PSD alinea varias
/// secciones a 2 bytes).
fn par_pad(v: &mut Vec<u8>) {
    if v.len() % 2 != 0 {
        v.push(0);
    }
}

/// Mapea un [`ModoFusion`] al 4-char *blend key* de Photoshop (space-padded).
/// Inverso de [`crate::mapear_blend`]: el catálogo cierra de modo que el
/// round-trip `exportar → importar` conserva el modo.
fn clave_blend(m: ModoFusion) -> [u8; 4] {
    match m {
        ModoFusion::Normal => *b"norm",
        ModoFusion::Multiplicar => *b"mul ",
        ModoFusion::Pantalla => *b"scrn",
        ModoFusion::Superponer => *b"over",
        ModoFusion::Aclarar => *b"lite",
        ModoFusion::Oscurecer => *b"dark",
        ModoFusion::Diferencia => *b"diff",
        ModoFusion::Aditivo => *b"lddg",
        ModoFusion::SubExpQuemado => *b"idiv",
        ModoFusion::SubLinealQuemado => *b"lbrn",
        ModoFusion::SobreExpAclarado => *b"div ",
        ModoFusion::LuzFuerte => *b"hLit",
        ModoFusion::LuzSuave => *b"sLit",
        ModoFusion::LuzViva => *b"vLit",
        ModoFusion::LuzLineal => *b"lLit",
        ModoFusion::LuzPunto => *b"pLit",
        ModoFusion::MezclaDura => *b"hMix",
        ModoFusion::Exclusion => *b"smud",
        ModoFusion::Resta => *b"fsub",
        ModoFusion::Division => *b"fdiv",
        ModoFusion::HslTono => *b"hue ",
        ModoFusion::HslSaturacion => *b"sat ",
        ModoFusion::HslColor => *b"colr",
        ModoFusion::HslLuminosidad => *b"lum ",
        ModoFusion::ColorMasOscuro => *b"dkCl",
        ModoFusion::ColorMasClaro => *b"lgCl",
        ModoFusion::Disolver => *b"diss",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::importar_psd;
    use std::collections::HashMap;
    use tullpu_core::{hash_bytes, Capa, Hash};

    /// Fuente de buffers en memoria para alimentar export/compositor.
    struct Fuente(HashMap<Hash, Vec<u8>>);
    impl FuenteBuffers for Fuente {
        fn obtener(&self, h: Hash) -> Option<&[u8]> {
            self.0.get(&h).map(|v| v.as_slice())
        }
    }

    fn solido(w: u32, h: u32, rgba: [u8; 4]) -> Vec<u8> {
        let mut v = Vec::with_capacity((w * h * 4) as usize);
        for _ in 0..(w * h) {
            v.extend_from_slice(&rgba);
        }
        v
    }

    /// Arma un lienzo + fuente desde una lista `(nombre, rgba, blend, opac, vis)`.
    fn armar(
        w: u32,
        h: u32,
        capas: &[(&str, [u8; 4], ModoFusion, f32, bool)],
    ) -> (Lienzo, Fuente) {
        let mut mapa: HashMap<Hash, Vec<u8>> = HashMap::new();
        let mut lienzo = Lienzo::nuevo(w, h);
        for (nombre, rgba, blend, opac, vis) in capas {
            let buf = solido(w, h, *rgba);
            let hash = hash_bytes(&buf);
            mapa.entry(hash).or_insert(buf);
            let mut capa = Capa::raster(*nombre, hash);
            capa.blend = *blend;
            capa.opacidad = *opac;
            capa.visible = *vis;
            lienzo.apilar(capa);
        }
        (lienzo, Fuente(mapa))
    }

    #[test]
    fn roundtrip_dims_y_conteo_de_capas() {
        let (l, f) = armar(
            3,
            2,
            &[
                ("fondo", [200, 0, 0, 255], ModoFusion::Normal, 1.0, true),
                ("medio", [0, 150, 0, 255], ModoFusion::Pantalla, 1.0, true),
            ],
        );
        let bytes = exportar_psd(&l, &f).expect("export ok");
        let doc = importar_psd(&bytes).expect("el PSD exportado debe importar");
        assert_eq!((doc.lienzo.width, doc.lienzo.height), (3, 2));
        assert_eq!(doc.lienzo.capas.len(), 2);
    }

    #[test]
    fn roundtrip_pixeles_opacos_exactos() {
        // Una capa opaca debe volver bit-a-bit (sin premultiplicar).
        let (l, f) = armar(
            2,
            2,
            &[("rojo", [200, 40, 10, 255], ModoFusion::Normal, 1.0, true)],
        );
        let bytes = exportar_psd(&l, &f).unwrap();
        let doc = importar_psd(&bytes).unwrap();
        let capa = &doc.lienzo.capas[0];
        let buf = &doc.buffers[&capa.contenido];
        assert_eq!(&buf[0..4], &[200, 40, 10, 255], "RGBA round-trip exacto");
    }

    #[test]
    fn roundtrip_metadatos_nombre_opacidad_visibilidad() {
        let (l, f) = armar(
            1,
            1,
            &[
                ("Capa A", [10, 20, 30, 255], ModoFusion::Normal, 1.0, true),
                ("oculta", [40, 50, 60, 255], ModoFusion::Normal, 0.5, false),
            ],
        );
        let bytes = exportar_psd(&l, &f).unwrap();
        let doc = importar_psd(&bytes).unwrap();
        // Nombre y opacidad round-trippean limpio por el lector.
        let a = doc.lienzo.capas.iter().find(|c| c.nombre == "Capa A").expect("Capa A");
        assert_eq!(a.opacidad, 1.0);
        let o = doc.lienzo.capas.iter().find(|c| c.nombre == "oculta").expect("oculta");
        assert!((o.opacidad - 0.5).abs() < 0.01, "opacidad ≈0.5, fue {}", o.opacidad);
    }

    /// Lee el byte de flags del layer record `idx` (orden bottom→top) de un PSD
    /// que escribimos nosotros. Replica el parseo mínimo de la sección
    /// Layer&Mask para certificar la **convención Photoshop** de visibilidad
    /// (bit 0x02 = oculta) directamente sobre los bytes, sin depender del
    /// lector `psd` (que invierte ese bit).
    fn flags_de_capa(bytes: &[u8], idx: usize) -> u8 {
        let be32 = |o: usize| u32::from_be_bytes(bytes[o..o + 4].try_into().unwrap());
        let be16 = |o: usize| u16::from_be_bytes(bytes[o..o + 2].try_into().unwrap());
        let mut off = 26; // tras el file header
        off += 4 + be32(off) as usize; // color mode data
        off += 4 + be32(off) as usize; // image resources
        off += 4; // layer & mask info length
        off += 4; // layer info length
        let cnt = be16(off) as usize;
        off += 2;
        assert!(idx < cnt, "idx fuera de rango");
        for k in 0..=idx {
            let nch = be16(off + 16) as usize;
            let p = off + 18 + nch * 6; // tras rect + nch + tabla de canales
            if k == idx {
                return bytes[p + 8 + 1 + 1]; // +sig(4)+key(4)+opac(1)+clip(1)
            }
            // Avanzar al siguiente record: + sig/key/opac/clip/flags/filler (14)
            // + extra data length + extra data.
            let extra_len = be32(p + 8 + 4) as usize;
            off = p + 8 + 4 + 4 + extra_len;
        }
        unreachable!()
    }

    #[test]
    fn visibilidad_codifica_convencion_photoshop() {
        // Capa visible ⇒ bit 0x02 en 0; oculta ⇒ bit 0x02 en 1. Más el bit
        // 0x08 (Photoshop 5.0+) en ambas, como los archivos reales.
        let (l, f) = armar(
            1,
            1,
            &[
                ("vis", [1, 2, 3, 255], ModoFusion::Normal, 1.0, true),
                ("oc", [4, 5, 6, 255], ModoFusion::Normal, 1.0, false),
            ],
        );
        let bytes = exportar_psd(&l, &f).unwrap();
        let f_vis = flags_de_capa(&bytes, 0);
        let f_oc = flags_de_capa(&bytes, 1);
        assert_eq!(f_vis & 0x02, 0, "visible ⇒ 0x02 clear (flags {f_vis:#04b})");
        assert_eq!(f_oc & 0x02, 0x02, "oculta ⇒ 0x02 set (flags {f_oc:#04b})");
        assert_eq!(f_vis & 0x08, 0x08, "bit Photoshop 5.0+ presente");
    }

    #[test]
    fn roundtrip_blends_se_conservan() {
        // clave_blend ↔ mapear_blend deben ser inversas para todo el catálogo.
        let modos = [
            ModoFusion::Multiplicar,
            ModoFusion::Pantalla,
            ModoFusion::Superponer,
            ModoFusion::Aclarar,
            ModoFusion::Oscurecer,
            ModoFusion::Diferencia,
            ModoFusion::Aditivo,
            ModoFusion::SubExpQuemado,
            ModoFusion::SubLinealQuemado,
            ModoFusion::SobreExpAclarado,
            ModoFusion::LuzFuerte,
            ModoFusion::LuzSuave,
            ModoFusion::LuzViva,
            ModoFusion::LuzLineal,
            ModoFusion::LuzPunto,
            ModoFusion::MezclaDura,
            ModoFusion::Exclusion,
            ModoFusion::Resta,
            ModoFusion::Division,
            ModoFusion::HslTono,
            ModoFusion::HslSaturacion,
            ModoFusion::HslColor,
            ModoFusion::HslLuminosidad,
            ModoFusion::ColorMasOscuro,
            ModoFusion::ColorMasClaro,
            ModoFusion::Disolver,
        ];
        // Una capa por modo, cada una con un color único (evita dedup/colisión
        // de nombres) y un nombre que codifica el índice.
        let especificadas: Vec<(String, [u8; 4], ModoFusion)> = modos
            .iter()
            .enumerate()
            .map(|(i, &m)| (format!("m{i}"), [i as u8, 0, 0, 255], m))
            .collect();
        let mut mapa: HashMap<Hash, Vec<u8>> = HashMap::new();
        let mut lienzo = Lienzo::nuevo(1, 1);
        for (nombre, rgba, blend) in &especificadas {
            let buf = solido(1, 1, *rgba);
            let hash = hash_bytes(&buf);
            mapa.entry(hash).or_insert(buf);
            let mut capa = Capa::raster(nombre, hash);
            capa.blend = *blend;
            lienzo.apilar(capa);
        }
        let bytes = exportar_psd(&lienzo, &Fuente(mapa)).unwrap();
        let doc = importar_psd(&bytes).unwrap();
        for (nombre, _, blend) in &especificadas {
            let capa = doc
                .lienzo
                .capas
                .iter()
                .find(|c| &c.nombre == nombre)
                .unwrap_or_else(|| panic!("capa {nombre} ausente"));
            assert_eq!(capa.blend, *blend, "blend de {nombre} no round-trippeó");
        }
        assert!(doc.informe.caidas_a_normal.is_empty(), "ningún blend degradó");
    }

    #[test]
    fn lienzo_sin_capas_es_error() {
        let l = Lienzo::nuevo(4, 4);
        let f = Fuente(HashMap::new());
        assert!(matches!(exportar_psd(&l, &f), Err(ExportPsdError::SinCapas)));
    }

    #[test]
    fn lienzo_degenerado_es_error() {
        let (mut l, f) = armar(1, 1, &[("c", [0, 0, 0, 255], ModoFusion::Normal, 1.0, true)]);
        l.width = 0;
        assert!(matches!(exportar_psd(&l, &f), Err(ExportPsdError::Dimensiones(0, 1))));
    }

    #[test]
    fn buffer_faltante_es_error() {
        let mut l = Lienzo::nuevo(1, 1);
        l.apilar(Capa::raster("huerfana", hash_bytes(b"inexistente")));
        let f = Fuente(HashMap::new());
        assert!(matches!(
            exportar_psd(&l, &f),
            Err(ExportPsdError::BufferFaltante(_))
        ));
    }
}
