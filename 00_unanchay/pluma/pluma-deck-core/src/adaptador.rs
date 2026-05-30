//! Adaptador opcional **pluma → Recorrido** (feature `pluma`).
//!
//! Puente entre el modelo de documento de pluma ([`NarrativeAtom`] /
//! [`Cuerpo`]) y los slides agnósticos de [`crate::ContenidoMarco`]. Promovido
//! desde el glue que vivía inline en el example `recorrido_md_demo` — ahora es
//! reusable y testeado. El core sigue agnóstico: este módulo sólo compila con
//! la feature `pluma` activa; sin ella, `pluma-deck-core` no conoce pluma.
//!
//! Regla de agrupación: un átomo cuyo texto arranca con `#`+espacio (encabezado
//! markdown) abre un slide nuevo cuyo título es ese encabezado; los demás átomos
//! son párrafos del slide actual. Es el mismo criterio que `pluma-md` usa al
//! emitir un átomo por bloque.

use pluma_core::NarrativeAtom;
use pluma_cuerpo::Cuerpo;
use uuid::Uuid;

use crate::{ContenidoMarco, Recorrido, RejillaOpts};

/// Cierra el slide en construcción (si tiene algo) y lo empuja a `slides`.
fn empujar(slides: &mut Vec<ContenidoMarco>, titulo: &mut Option<String>, parrafos: &mut Vec<String>) {
    if titulo.is_some() || !parrafos.is_empty() {
        slides.push(ContenidoMarco::Texto {
            titulo: titulo.take(),
            parrafos: std::mem::take(parrafos),
        });
    }
}

/// Agrupa una secuencia de textos (en orden de documento) en slides. Un texto
/// que arranca con `#`+espacio abre un slide nuevo (su título = el encabezado
/// sin los `#`); los textos vacíos se ignoran; el resto son párrafos del slide
/// actual. Es la pieza pura — `recorrido_desde_*` la alimentan.
pub fn slides_desde_textos<'a>(textos: impl IntoIterator<Item = &'a str>) -> Vec<ContenidoMarco> {
    let mut slides = Vec::new();
    let mut titulo: Option<String> = None;
    let mut parrafos: Vec<String> = Vec::new();
    for c in textos {
        let hashes = c.chars().take_while(|&ch| ch == '#').count();
        let es_encabezado = hashes > 0 && c[hashes..].starts_with(' ');
        if es_encabezado {
            empujar(&mut slides, &mut titulo, &mut parrafos);
            titulo = Some(c[hashes..].trim().to_string());
        } else if !c.trim().is_empty() {
            parrafos.push(c.to_string());
        }
    }
    empujar(&mut slides, &mut titulo, &mut parrafos);
    slides
}

/// Recorrido desde una secuencia de átomos en **orden de documento** (un
/// encabezado abre slide). `opts` controla el auto-layout en rejilla.
pub fn recorrido_desde_atomos(atomos: &[NarrativeAtom], opts: RejillaOpts) -> Recorrido {
    Recorrido::en_rejilla(slides_desde_textos(atomos.iter().map(|a| a.content.as_str())), opts)
}

/// Recorrido desde un [`Cuerpo`]: recorre `cuerpo.orden` resolviendo cada `Uuid`
/// a su átomo con `resolver` (el cuerpo no conoce el grafo, lo resuelve el
/// caller); los ids que no resuelven se omiten. Así un lienzo del haz multilienzo
/// alimenta una presentación sin que el core conozca el `NarrativeGraph`.
pub fn recorrido_desde_cuerpo<'a>(
    cuerpo: &Cuerpo,
    resolver: impl Fn(&Uuid) -> Option<&'a NarrativeAtom>,
    opts: RejillaOpts,
) -> Recorrido {
    let textos = cuerpo.orden.iter().filter_map(|id| resolver(id)).map(|a| a.content.as_str());
    Recorrido::en_rejilla(slides_desde_textos(textos), opts)
}

#[cfg(test)]
mod tests {
    use super::*;
    use pluma_cuerpo::Intencion;

    fn atom(c: &str) -> NarrativeAtom {
        NarrativeAtom::new(c, "es")
    }

    fn como_texto(c: &ContenidoMarco) -> (Option<&str>, &[String]) {
        match c {
            ContenidoMarco::Texto { titulo, parrafos } => (titulo.as_deref(), parrafos.as_slice()),
            _ => panic!("se esperaba ContenidoMarco::Texto"),
        }
    }

    #[test]
    fn slides_agrupan_por_encabezado() {
        let textos = ["# Título A", "p1", "", "p2", "## Título B", "p3"];
        let slides = slides_desde_textos(textos.iter().copied());
        assert_eq!(slides.len(), 2);
        let (t0, p0) = como_texto(&slides[0]);
        assert_eq!(t0, Some("Título A"));
        assert_eq!(p0, &["p1".to_string(), "p2".to_string()]); // el vacío se ignora
        let (t1, p1) = como_texto(&slides[1]);
        assert_eq!(t1, Some("Título B"));
        assert_eq!(p1, &["p3".to_string()]);
    }

    #[test]
    fn parrafos_sueltos_sin_encabezado_forman_un_slide_sin_titulo() {
        let slides = slides_desde_textos(["solo párrafo", "y otro"].iter().copied());
        assert_eq!(slides.len(), 1);
        let (t, p) = como_texto(&slides[0]);
        assert_eq!(t, None);
        assert_eq!(p.len(), 2);
    }

    #[test]
    fn recorrido_desde_atomos_rutea_en_orden_de_lectura() {
        let atomos = vec![atom("# Uno"), atom("cuerpo de uno"), atom("# Dos")];
        let rec = recorrido_desde_atomos(&atomos, RejillaOpts::default());
        assert_eq!(rec.marcos.len(), 2);
        assert_eq!(rec.pasos, vec![1, 2]);
    }

    #[test]
    fn recorrido_desde_cuerpo_resuelve_el_orden_y_omite_faltantes() {
        let a = atom("# Hola");
        let b = atom("mundo");
        let mut cuerpo = Cuerpo::nuevo("es", "doc", Intencion::Original, 0);
        cuerpo.agregar(a.id, 0);
        cuerpo.agregar(b.id, 0);
        cuerpo.agregar(Uuid::new_v4(), 0); // id huérfano: no resuelve
        let resolver = |id: &Uuid| {
            if *id == a.id {
                Some(&a)
            } else if *id == b.id {
                Some(&b)
            } else {
                None
            }
        };
        let rec = recorrido_desde_cuerpo(&cuerpo, resolver, RejillaOpts::default());
        // Un solo slide: "Hola" con el párrafo "mundo"; el huérfano se omitió.
        assert_eq!(rec.marcos.len(), 1);
        let (t, p) = como_texto(&rec.marcos[0].contenido);
        assert_eq!(t, Some("Hola"));
        assert_eq!(p, &["mundo".to_string()]);
    }
}
