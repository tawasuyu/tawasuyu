use super::*;
use iniy_core::{AsercionId, DocId, Opinion};

use iniy_ingest::{Chunk, Documento};

fn doc_demo() -> Documento {
    let doc_id = DocId::nuevo();
    Documento {
        id: doc_id,
        titulo: "demo".into(),
        chunks: vec![
            Chunk { id: ChunkId::nuevo(), doc_id, orden: 0, texto: "primer párrafo del corpus de prueba.".into() },
            Chunk { id: ChunkId::nuevo(), doc_id, orden: 1, texto: "segundo párrafo del corpus de prueba.".into() },
        ],
    }
}

fn asercion_demo(doc_id: DocId, chunk_id: ChunkId, texto: &str) -> Asercion {
    Asercion {
        id: AsercionId::nuevo(),
        doc_id,
        chunk_id,
        texto: texto.into(),
        opinion_autoral: Opinion::nueva(0.6, 0.1, 0.3, 0.5).unwrap(),
    }
}

#[test]
fn store_en_memoria_migra_ok() {
    let _ = Store::en_memoria().unwrap();
}

#[test]
fn persistir_y_listar_documento() {
    let mut s = Store::en_memoria().unwrap();
    let doc = doc_demo();
    s.persistir_documento(&doc, None).unwrap();
    let lista = s.listar_documentos().unwrap();
    assert_eq!(lista.len(), 1);
    assert_eq!(lista[0].titulo, "demo");
    assert_eq!(lista[0].n_chunks, 2);
    assert_eq!(lista[0].id, doc.id);
}

#[test]
fn round_trip_chunks_preserva_orden_y_texto() {
    let mut s = Store::en_memoria().unwrap();
    let doc = doc_demo();
    s.persistir_documento(&doc, None).unwrap();
    let chunks = s.cargar_chunks(doc.id).unwrap();
    assert_eq!(chunks.len(), 2);
    assert_eq!(chunks[0].orden, 0);
    assert_eq!(chunks[1].orden, 1);
    assert!(chunks[0].texto.starts_with("primer"));
    assert!(chunks[1].texto.starts_with("segundo"));
    assert_eq!(chunks[0].id, doc.chunks[0].id);
}

#[test]
fn persistir_id_duplicado_falla() {
    let mut s = Store::en_memoria().unwrap();
    let doc = doc_demo();
    s.persistir_documento(&doc, None).unwrap();
    assert!(s.persistir_documento(&doc, None).is_err());
}

#[test]
fn round_trip_aserciones_preserva_opinion() {
    let mut s = Store::en_memoria().unwrap();
    let doc = doc_demo();
    s.persistir_documento(&doc, None).unwrap();
    let a = asercion_demo(doc.id, doc.chunks[0].id, "aserción de prueba");
    s.persistir_aserciones(&[a.clone()]).unwrap();
    let cargadas = s.cargar_aserciones(doc.id).unwrap();
    assert_eq!(cargadas.len(), 1);
    assert_eq!(cargadas[0].id, a.id);
    assert_eq!(cargadas[0].texto, a.texto);
    assert!((cargadas[0].opinion_autoral.creencia - 0.6).abs() < 1e-5);
}

#[test]
fn round_trip_implicaciones_filtra_por_doc() {
    let mut s = Store::en_memoria().unwrap();
    let doc = doc_demo();
    s.persistir_documento(&doc, None).unwrap();
    let a1 = asercion_demo(doc.id, doc.chunks[0].id, "primera");
    let a2 = asercion_demo(doc.id, doc.chunks[1].id, "segunda");
    s.persistir_aserciones(&[a1.clone(), a2.clone()]).unwrap();
    let imp = Implicacion {
        premisa: a1.id,
        hipotesis: a2.id,
        relacion: RelacionNli { entailment: 0.0, contradiction: 0.7, neutral: 0.3 },
    };
    s.persistir_implicaciones(&[imp]).unwrap();
    let imps = s.cargar_implicaciones_del_doc(doc.id).unwrap();
    assert_eq!(imps.len(), 1);
    assert!((imps[0].relacion.contradiction - 0.7).abs() < 1e-5);
}

#[test]
fn aserciones_y_implicaciones_son_idempotentes() {
    let mut s = Store::en_memoria().unwrap();
    let doc = doc_demo();
    s.persistir_documento(&doc, None).unwrap();
    let a = asercion_demo(doc.id, doc.chunks[0].id, "x");
    s.persistir_aserciones(&[a.clone()]).unwrap();
    s.persistir_aserciones(&[a.clone()]).unwrap();
    assert_eq!(s.cargar_aserciones(doc.id).unwrap().len(), 1);
}

#[test]
fn obtener_o_crear_fuente_es_idempotente_por_nombre() {
    let mut s = Store::en_memoria().unwrap();
    let f1 = s.obtener_o_crear_fuente("Aristóteles", Some("autor")).unwrap();
    let f2 = s.obtener_o_crear_fuente("Aristóteles", None).unwrap();
    assert_eq!(f1, f2);
    assert_eq!(s.listar_fuentes().unwrap().len(), 1);
}

#[test]
fn obtener_o_crear_fuente_actualiza_kind_si_estaba_vacio() {
    let mut s = Store::en_memoria().unwrap();
    s.obtener_o_crear_fuente("Voltaire", None).unwrap();
    s.obtener_o_crear_fuente("Voltaire", Some("autor")).unwrap();
    let lista = s.listar_fuentes().unwrap();
    assert_eq!(lista[0].fuente.kind.as_deref(), Some("autor"));
}

#[test]
fn documento_atribuido_resuelve_fuente_al_listar() {
    let mut s = Store::en_memoria().unwrap();
    let f = s.obtener_o_crear_fuente("Heráclito", Some("autor")).unwrap();
    let doc = doc_demo();
    s.persistir_documento(&doc, Some(f)).unwrap();
    let docs = s.listar_documentos().unwrap();
    assert_eq!(docs[0].fuente.as_ref().unwrap().nombre, "Heráclito");
}

#[test]
fn listar_fuentes_cuenta_docs_y_aserciones() {
    let mut s = Store::en_memoria().unwrap();
    let f = s.obtener_o_crear_fuente("F1", None).unwrap();
    let doc = doc_demo();
    s.persistir_documento(&doc, Some(f)).unwrap();
    let a1 = asercion_demo(doc.id, doc.chunks[0].id, "uno");
    let a2 = asercion_demo(doc.id, doc.chunks[0].id, "dos");
    s.persistir_aserciones(&[a1, a2]).unwrap();
    let lista = s.listar_fuentes().unwrap();
    assert_eq!(lista[0].n_docs, 1);
    assert_eq!(lista[0].n_aserciones, 2);
}

#[test]
fn aserciones_atribuidas_trae_fuente_y_titulo() {
    let mut s = Store::en_memoria().unwrap();
    let f = s.obtener_o_crear_fuente("Plotino", Some("autor")).unwrap();
    let doc = doc_demo();
    s.persistir_documento(&doc, Some(f)).unwrap();
    let a = asercion_demo(doc.id, doc.chunks[0].id, "el uno trasciende al ser");
    s.persistir_aserciones(&[a]).unwrap();
    let v = s.cargar_aserciones_atribuidas_todas().unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].fuente.as_ref().unwrap().nombre, "Plotino");
    assert_eq!(v[0].doc_titulo, "demo");
}

#[test]
fn fuente_citada_supera_a_fuente_del_doc_en_atribuida() {
    let mut s = Store::en_memoria().unwrap();
    let f_doc = s.obtener_o_crear_fuente("Wikipedia", Some("wiki")).unwrap();
    let f_citada = s.obtener_o_crear_fuente("Aristóteles", Some("autor")).unwrap();
    let doc = doc_demo();
    s.persistir_documento(&doc, Some(f_doc)).unwrap();
    let a = asercion_demo(doc.id, doc.chunks[0].id, "El cosmos es eterno");
    s.persistir_aserciones(&[a.clone()]).unwrap();
    s.asignar_fuente_citada(a.id, Some(f_citada)).unwrap();
    let v = s.cargar_aserciones_atribuidas_todas().unwrap();
    assert_eq!(v.len(), 1);
    assert_eq!(v[0].fuente.as_ref().unwrap().nombre, "Aristóteles");
    assert!(v[0].citada);
}

#[test]
fn aserciones_sin_cita_usan_fuente_del_doc() {
    let mut s = Store::en_memoria().unwrap();
    let f = s.obtener_o_crear_fuente("Anaximandro", Some("autor")).unwrap();
    let doc = doc_demo();
    s.persistir_documento(&doc, Some(f)).unwrap();
    let a = asercion_demo(doc.id, doc.chunks[0].id, "X");
    s.persistir_aserciones(&[a]).unwrap();
    let v = s.cargar_aserciones_atribuidas_todas().unwrap();
    assert_eq!(v[0].fuente.as_ref().unwrap().nombre, "Anaximandro");
    assert!(!v[0].citada);
}

#[test]
fn listar_fuentes_cuenta_aserciones_citadas() {
    let mut s = Store::en_memoria().unwrap();
    let f_doc = s.obtener_o_crear_fuente("Doxógrafo", None).unwrap();
    let f_cita = s.obtener_o_crear_fuente("Tales", Some("autor")).unwrap();
    let doc = doc_demo();
    s.persistir_documento(&doc, Some(f_doc)).unwrap();
    let a1 = asercion_demo(doc.id, doc.chunks[0].id, "agua principio");
    let a2 = asercion_demo(doc.id, doc.chunks[0].id, "otra cosa");
    s.persistir_aserciones(&[a1.clone(), a2]).unwrap();
    s.asignar_fuente_citada(a1.id, Some(f_cita)).unwrap();
    let lista = s.listar_fuentes().unwrap();
    let tales = lista.iter().find(|r| r.fuente.nombre == "Tales").unwrap();
    let doxo = lista.iter().find(|r| r.fuente.nombre == "Doxógrafo").unwrap();
    assert_eq!(tales.n_aserciones, 1); // la citada
    assert_eq!(doxo.n_aserciones, 1);  // la que cae al doc
}

#[test]
fn recalcular_reputaciones_persiste_y_calcula_score() {
    let mut s = Store::en_memoria().unwrap();
    let f1 = s.obtener_o_crear_fuente("F1", None).unwrap();
    let f2 = s.obtener_o_crear_fuente("F2", None).unwrap();
    let doc1 = doc_demo();
    let doc2 = doc_demo();
    s.persistir_documento(&doc1, Some(f1)).unwrap();
    s.persistir_documento(&doc2, Some(f2)).unwrap();
    let a1 = asercion_demo(doc1.id, doc1.chunks[0].id, "F1 dice X");
    let a2 = asercion_demo(doc2.id, doc2.chunks[0].id, "F2 contradice X");
    s.persistir_aserciones(&[a1.clone(), a2.clone()]).unwrap();
    // F1 ←(contradiction)← F2.
    s.persistir_implicaciones(&[Implicacion {
        premisa: a2.id,
        hipotesis: a1.id,
        relacion: RelacionNli { entailment: 0.0, contradiction: 0.8, neutral: 0.2 },
    }]).unwrap();
    let n = s.recalcular_reputaciones().unwrap();
    assert_eq!(n, 2);

    let rep_f1 = s.cargar_reputacion(f1).unwrap().unwrap();
    assert_eq!(rep_f1.contradicha, 1);
    assert_eq!(rep_f1.apoyada, 0);
    assert!((rep_f1.score - (-1.0)).abs() < 1e-5);

    let rep_f2 = s.cargar_reputacion(f2).unwrap().unwrap();
    assert_eq!(rep_f2.contradice, 1);
    assert_eq!(rep_f2.apoya, 0);
    // F2 no recibe nada, score=0.
    assert!((rep_f2.score - 0.0).abs() < 1e-5);
}

#[test]
fn recalcular_reputaciones_es_idempotente() {
    let mut s = Store::en_memoria().unwrap();
    let f = s.obtener_o_crear_fuente("F", None).unwrap();
    let doc = doc_demo();
    s.persistir_documento(&doc, Some(f)).unwrap();
    let a = asercion_demo(doc.id, doc.chunks[0].id, "x");
    s.persistir_aserciones(&[a]).unwrap();
    s.recalcular_reputaciones().unwrap();
    s.recalcular_reputaciones().unwrap();
    assert_eq!(s.cargar_reputaciones_todas().unwrap().len(), 1);
}

#[test]
fn export_import_round_trip_preserva_todo() {
    let mut origen = Store::en_memoria().unwrap();
    let f = origen.obtener_o_crear_fuente("F", Some("autor")).unwrap();
    let doc = doc_demo();
    origen.persistir_documento(&doc, Some(f)).unwrap();
    let a = asercion_demo(doc.id, doc.chunks[0].id, "X");
    origen.persistir_aserciones(&[a.clone()]).unwrap();
    origen.taggear_doc(doc.id, "tema1").unwrap();
    let dump = origen.exportar_todo().unwrap();

    let mut destino = Store::en_memoria().unwrap();
    let stats = destino.importar_dump(&dump).unwrap();
    assert_eq!(stats.fuentes, 1);
    assert_eq!(stats.documentos, 1);
    assert_eq!(stats.aserciones, 1);
    assert_eq!(stats.tags, 1);

    // Verificar que el destino refleja el origen.
    let asercs = destino.cargar_aserciones_atribuidas_todas().unwrap();
    assert_eq!(asercs.len(), 1);
    assert_eq!(asercs[0].asercion.texto, "X");
    assert_eq!(asercs[0].fuente.as_ref().unwrap().nombre, "F");
    assert_eq!(destino.tags_de_doc(doc.id).unwrap(), vec!["tema1".to_string()]);
}

#[test]
fn import_es_idempotente_sobre_re_aplicacion() {
    let mut origen = Store::en_memoria().unwrap();
    let f = origen.obtener_o_crear_fuente("F", None).unwrap();
    let doc = doc_demo();
    origen.persistir_documento(&doc, Some(f)).unwrap();
    let dump = origen.exportar_todo().unwrap();

    let mut destino = Store::en_memoria().unwrap();
    destino.importar_dump(&dump).unwrap();
    let stats2 = destino.importar_dump(&dump).unwrap();
    // Segunda pasada: todos omitidos.
    assert_eq!(stats2.fuentes, 0);
    assert_eq!(stats2.fuentes_omitidas, 1);
    assert_eq!(stats2.documentos, 0);
    assert_eq!(stats2.documentos_omitidos, 1);
}

#[test]
fn migracion_anade_fuente_id_a_documentos_viejos() {
    // DB que existía antes del modelo de fuentes: la primer migración (sin
    // fuentes) corre, y luego una segunda re-migración añade la columna.
    // El esquema final tiene que tener `documentos.fuente_id`.
    let s = Store::en_memoria().unwrap();
    // Forzamos otra migración para verificar idempotencia.
    s.migrar().unwrap();
    let mut stmt = s.conn.prepare("PRAGMA table_info(documentos)").unwrap();
    let cols: Vec<String> = stmt
        .query_map([], |r| r.get::<_, String>(1))
        .unwrap()
        .map(|r| r.unwrap())
        .collect();
    assert!(cols.iter().any(|c| c == "fuente_id"));
}

// ---- calcular_reputaciones (scoring puro, regla #2) ----

fn fuente_demo(nombre: &str) -> Fuente {
    Fuente { id: FuenteId::nuevo(), nombre: nombre.into(), kind: None }
}

fn atribuida(asercion: Asercion, fuente: Fuente) -> AsercionAtribuida {
    AsercionAtribuida { asercion, doc_titulo: "doc".into(), fuente: Some(fuente), citada: false }
}

#[test]
fn reputaciones_apoyo_y_contradiccion_dan_extremos() {
    let did = DocId::nuevo();
    let cid = ChunkId::nuevo();
    let (fa, fb) = (fuente_demo("A"), fuente_demo("B"));
    let (fa_id, fb_id) = (fa.id, fb.id);
    let a = asercion_demo(did, cid, "premisa de A");
    let b = asercion_demo(did, cid, "hipótesis de B");
    let (a_id, b_id) = (a.id, b.id);
    let todas = vec![atribuida(a, fa), atribuida(b, fb)];

    // A ⟹ B (entailment) ⇒ B recibe +1 apoyo ⇒ score 1.0; A sin entrante ⇒ 0.0
    let apoyo = Implicacion {
        premisa: a_id,
        hipotesis: b_id,
        relacion: RelacionNli { entailment: 0.9, contradiction: 0.05, neutral: 0.05 },
    };
    let reps = calcular_reputaciones(&todas, &[apoyo]);
    assert_eq!(reps.get(&fb_id).copied(), Some(1.0));
    assert_eq!(reps.get(&fa_id).copied(), Some(0.0));

    // Misma arista pero contradictoria ⇒ B score -1.0
    let contra = Implicacion {
        premisa: a_id,
        hipotesis: b_id,
        relacion: RelacionNli { entailment: 0.05, contradiction: 0.9, neutral: 0.05 },
    };
    let reps = calcular_reputaciones(&todas, &[contra]);
    assert_eq!(reps.get(&fb_id).copied(), Some(-1.0));
}

#[test]
fn reputaciones_ignora_aristas_intra_fuente() {
    let did = DocId::nuevo();
    let cid = ChunkId::nuevo();
    let f = fuente_demo("única");
    let fid = f.id;
    let a = asercion_demo(did, cid, "a");
    let b = asercion_demo(did, cid, "b");
    let (a_id, b_id) = (a.id, b.id);
    // ambas aserciones de la MISMA fuente
    let f2 = Fuente { id: fid, nombre: "única".into(), kind: None };
    let todas = vec![atribuida(a, f), atribuida(b, f2)];
    let imp = Implicacion {
        premisa: a_id,
        hipotesis: b_id,
        relacion: RelacionNli { entailment: 0.9, contradiction: 0.0, neutral: 0.1 },
    };
    let reps = calcular_reputaciones(&todas, &[imp]);
    // sin evidencia inter-fuente ⇒ score 0
    assert_eq!(reps.get(&fid).copied(), Some(0.0));
}
