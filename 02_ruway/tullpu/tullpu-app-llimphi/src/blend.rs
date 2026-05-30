//! Ciclo de modos de fusión de la app `tullpu`: catálogo canónico
//! (orden Photoshop) y los helpers para ciclar adelante/atrás y obtener
//! la etiqueta legible de cada modo.
//!
//! Behavior-preserving split de `main.rs` — sin cambios funcionales.

use tullpu_core::ModoFusion;

/// Ciclo canónico de blend modes (orden Photoshop: Normal → catálogo
/// completo → Disolver → Normal). Lo declaramos una vez y derivamos
/// `siguiente_blend` y `blend_anterior` indexando, así Shift+B y B son
/// trivialmente inversos sin dos `match` paralelos que se desincronicen.
pub(crate) const CICLO_BLEND: &[ModoFusion] = &[
    ModoFusion::Normal,
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

pub(crate) fn indice_blend(b: ModoFusion) -> usize {
    CICLO_BLEND.iter().position(|m| *m == b).unwrap_or(0)
}

pub(crate) fn siguiente_blend(b: ModoFusion) -> ModoFusion {
    let i = indice_blend(b);
    CICLO_BLEND[(i + 1) % CICLO_BLEND.len()]
}

pub(crate) fn blend_anterior(b: ModoFusion) -> ModoFusion {
    let i = indice_blend(b);
    CICLO_BLEND[(i + CICLO_BLEND.len() - 1) % CICLO_BLEND.len()]
}

pub(crate) fn etiqueta_blend(b: ModoFusion) -> &'static str {
    match b {
        ModoFusion::Normal => "normal",
        ModoFusion::Multiplicar => "multiplicar",
        ModoFusion::Pantalla => "pantalla",
        ModoFusion::Superponer => "superponer",
        ModoFusion::Aclarar => "aclarar",
        ModoFusion::Oscurecer => "oscurecer",
        ModoFusion::Diferencia => "diferencia",
        ModoFusion::Aditivo => "aditivo",
        ModoFusion::SubExpQuemado => "subexp-quemado",
        ModoFusion::SubLinealQuemado => "sublineal-quemado",
        ModoFusion::SobreExpAclarado => "sobreexp-aclarado",
        ModoFusion::LuzFuerte => "luz-fuerte",
        ModoFusion::LuzSuave => "luz-suave",
        ModoFusion::LuzViva => "luz-viva",
        ModoFusion::LuzLineal => "luz-lineal",
        ModoFusion::LuzPunto => "luz-punto",
        ModoFusion::MezclaDura => "mezcla-dura",
        ModoFusion::Exclusion => "exclusión",
        ModoFusion::Resta => "resta",
        ModoFusion::Division => "división",
        ModoFusion::HslTono => "hsl-tono",
        ModoFusion::HslSaturacion => "hsl-saturación",
        ModoFusion::HslColor => "hsl-color",
        ModoFusion::HslLuminosidad => "hsl-luminosidad",
        ModoFusion::ColorMasOscuro => "color-más-oscuro",
        ModoFusion::ColorMasClaro => "color-más-claro",
        ModoFusion::Disolver => "disolver",
    }
}
