//! Un archivo por tile. Cada `*_view(model, theme) -> View<Msg>` es puro:
//! lee el `Model` y arma la vista; no muta nada (eso es del `update`).

use agora_core::IdentityKind;

pub(crate) mod atestaciones;
pub(crate) mod capacidad;
pub(crate) mod compositor;
pub(crate) mod identidades;
pub(crate) mod multifirma;
pub(crate) mod politica;
pub(crate) mod release;

pub(crate) use atestaciones::atestaciones_view;
pub(crate) use capacidad::capacidad_view;
pub(crate) use compositor::compositor_view;
pub(crate) use identidades::identidades_view;
pub(crate) use multifirma::multifirma_view;
pub(crate) use politica::politica_view;
pub(crate) use release::release_view;

/// Etiqueta corta en español del tipo de identidad.
pub(crate) fn kind_str(k: IdentityKind) -> &'static str {
    match k {
        IdentityKind::Person => "persona",
        IdentityKind::Community => "comunidad",
        IdentityKind::Alliance => "alianza",
        IdentityKind::Institution => "institución",
    }
}

/// Duración legible compacta (`45s` / `5m` / `2h` / `7d`).
pub(crate) fn format_duration(secs: u64) -> String {
    if secs < 60 {
        format!("{secs}s")
    } else if secs < 3_600 {
        format!("{}m", secs / 60)
    } else if secs < 86_400 {
        format!("{}h", secs / 3_600)
    } else {
        format!("{}d", secs / 86_400)
    }
}

/// `off` o la duración legible del preset de `max_age`.
pub(crate) fn format_max_age(v: Option<u64>) -> String {
    match v {
        None => "off".into(),
        Some(s) => format_duration(s),
    }
}
