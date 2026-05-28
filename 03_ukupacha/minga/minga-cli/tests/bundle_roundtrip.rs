//! Round-trip de bundle: export en repo A → import en repo B vacío,
//! verificando que la raíz, sus atestaciones y su contenido sobreviven
//! intactos y que el import es idempotente.
//!
//! Ítem #O del REPORTE: la red de seguridad mínima para detectar
//! regresiones en el formato `BundleV1` o en el path de
//! verificación criptográfica al importar.

use std::fs;

use minga_cli::{
    cmd_bundle_export, cmd_bundle_export_all, cmd_bundle_import, cmd_bundle_import_all, cmd_init,
    cmd_ingest, cmd_retire, cmd_show, cmd_sign, cmd_status, CliError,
};
use tempfile::TempDir;

#[test]
fn bundle_roundtrip_preserves_root_and_attestation() {
    let dir = TempDir::new().unwrap();

    // Repo A: emisor.
    let repo_a = dir.path().join("a");
    let did_a = cmd_init(&repo_a, "pa").unwrap();
    let src = dir.path().join("f.rs");
    fs::write(&src, "fn f() -> i32 { 42 }").unwrap();
    let ing = cmd_ingest(&repo_a, "pa", &src).unwrap();
    assert_eq!(ing.did, did_a);

    // Export.
    let bundle_path = dir.path().join("f.bundle");
    let exp = cmd_bundle_export(&repo_a, "pa", &ing.alpha.to_string(), &bundle_path).unwrap();
    assert_eq!(exp.alpha, ing.alpha);
    assert_eq!(exp.attestations, 1, "ingest deja una autoatestación");
    assert!(exp.nodes >= 1);
    assert!(bundle_path.exists());

    // Repo B: receptor vacío.
    let repo_b = dir.path().join("b");
    let did_b = cmd_init(&repo_b, "pb").unwrap();
    assert_ne!(did_a, did_b, "los DIDs deben diferir entre repos");

    // Import.
    let imp = cmd_bundle_import(&repo_b, "pb", &bundle_path).unwrap();
    assert_eq!(imp.alpha, ing.alpha, "α preservado tras round-trip");
    assert!(imp.root_was_new);
    assert!(imp.nodes_inserted >= 1);
    assert_eq!(imp.attestations_added, 1);
    assert_eq!(imp.attestations_rejected, 0);
    assert_eq!(imp.retractions_added, 0);

    // El status de B refleja exactamente lo que A tenía sobre esa raíz.
    let s = cmd_status(&repo_b, "pb").unwrap();
    assert_eq!(s.roots_len, 1);
    assert_eq!(s.mst_len, 1);
    assert_eq!(s.attestations_len, 1);

    // El contenido reconstruido en B coincide con el de A.
    let show_a = cmd_show(&repo_a, "pa", &ing.alpha.to_string(), false).unwrap();
    let show_b = cmd_show(&repo_b, "pb", &ing.alpha.to_string(), false).unwrap();
    assert_eq!(show_a.rendered, show_b.rendered);
}

#[test]
fn bundle_import_is_idempotent() {
    let dir = TempDir::new().unwrap();

    let repo_a = dir.path().join("a");
    cmd_init(&repo_a, "p").unwrap();
    let src = dir.path().join("g.rs");
    fs::write(&src, "fn g() -> i32 { 1 }").unwrap();
    let ing = cmd_ingest(&repo_a, "p", &src).unwrap();
    let bundle_path = dir.path().join("g.bundle");
    cmd_bundle_export(&repo_a, "p", &ing.alpha.to_string(), &bundle_path).unwrap();

    let repo_b = dir.path().join("b");
    cmd_init(&repo_b, "p").unwrap();

    let imp1 = cmd_bundle_import(&repo_b, "p", &bundle_path).unwrap();
    assert!(imp1.root_was_new);

    let imp2 = cmd_bundle_import(&repo_b, "p", &bundle_path).unwrap();
    assert!(!imp2.root_was_new, "segunda importación: raíz ya conocida");
    assert_eq!(imp2.nodes_inserted, 0, "nodos deduplicados");
    assert_eq!(imp2.attestations_added, 0, "atestación deduplicada");
    assert_eq!(imp2.retractions_added, 0);

    let s = cmd_status(&repo_b, "p").unwrap();
    assert_eq!(s.roots_len, 1);
    assert_eq!(s.attestations_len, 1);
}

#[test]
fn bundle_roundtrip_propagates_vouching_attestations() {
    // Dos peers atestan la misma raíz; el bundle transporta ambas
    // firmas a un tercero limpio.
    let dir = TempDir::new().unwrap();

    let repo_a = dir.path().join("a");
    cmd_init(&repo_a, "pa").unwrap();
    let src = dir.path().join("h.rs");
    fs::write(&src, "fn h() -> i32 { 7 }").unwrap();
    let ing = cmd_ingest(&repo_a, "pa", &src).unwrap();

    // Ronda intermedia: bundle a→b, b firma como segundo vouching, bundle b→c.
    let pkg_ab = dir.path().join("h.ab.bundle");
    cmd_bundle_export(&repo_a, "pa", &ing.alpha.to_string(), &pkg_ab).unwrap();

    let repo_b = dir.path().join("b");
    cmd_init(&repo_b, "pb").unwrap();
    cmd_bundle_import(&repo_b, "pb", &pkg_ab).unwrap();
    let sign_b = cmd_sign(&repo_b, "pb", &ing.alpha.to_string()).unwrap();
    assert!(sign_b.is_new_attestation);
    assert!(sign_b.is_known_root);

    let pkg_bc = dir.path().join("h.bc.bundle");
    let exp_bc = cmd_bundle_export(&repo_b, "pb", &ing.alpha.to_string(), &pkg_bc).unwrap();
    assert_eq!(exp_bc.attestations, 2, "ambas firmas viajan en el bundle");

    let repo_c = dir.path().join("c");
    cmd_init(&repo_c, "pc").unwrap();
    let imp_c = cmd_bundle_import(&repo_c, "pc", &pkg_bc).unwrap();
    assert_eq!(imp_c.attestations_added, 2);
    assert_eq!(imp_c.attestations_rejected, 0);

    let s = cmd_status(&repo_c, "pc").unwrap();
    assert_eq!(s.attestations_len, 2);
    assert_eq!(s.roots_len, 1);
}

#[test]
fn bundle_roundtrip_carries_retractions() {
    let dir = TempDir::new().unwrap();

    let repo_a = dir.path().join("a");
    cmd_init(&repo_a, "pa").unwrap();
    let src = dir.path().join("r.rs");
    fs::write(&src, "fn r() -> i32 { 9 }").unwrap();
    let ing = cmd_ingest(&repo_a, "pa", &src).unwrap();

    // Bundle base hacia B (antes de retractar — necesitamos α en `roots`
    // para que el export funcione).
    let pkg = dir.path().join("r.bundle");
    cmd_bundle_export(&repo_a, "pa", &ing.alpha.to_string(), &pkg).unwrap();
    let repo_b = dir.path().join("b");
    cmd_init(&repo_b, "pb").unwrap();
    cmd_bundle_import(&repo_b, "pb", &pkg).unwrap();

    // A retracta su propia raíz y exporta de nuevo para arrastrar la
    // retracción. Nota: retire la saca de `roots`, así que el export se
    // hace ANTES sobre la versión retractada — la retracción persiste en
    // su tree pese al cleanup del MST… revisemos.
    cmd_retire(&repo_a, "pa", &ing.alpha.to_string()).unwrap();
    // Tras retire, α salió de roots; export fallaría con HashNotFound.
    // Reingerimos para volver a registrar la raíz; la retracción sigue
    // en su tree por diseño (es prueba histórica) y debería viajar.
    cmd_ingest(&repo_a, "pa", &src).unwrap();

    let pkg2 = dir.path().join("r.2.bundle");
    let exp2 = cmd_bundle_export(&repo_a, "pa", &ing.alpha.to_string(), &pkg2).unwrap();
    assert_eq!(exp2.retractions, 1, "la retracción sigue en el tree y viaja");

    let repo_c = dir.path().join("c");
    cmd_init(&repo_c, "pc").unwrap();
    let imp_c = cmd_bundle_import(&repo_c, "pc", &pkg2).unwrap();
    assert_eq!(imp_c.retractions_added, 1);
    assert_eq!(imp_c.retractions_rejected, 0);
}

#[test]
fn multi_bundle_round_trip_carries_all_roots() {
    // A tiene tres archivos ingeridos; un multi-bundle los lleva a B
    // de un solo viaje, B termina con las mismas tres raíces.
    let dir = TempDir::new().unwrap();

    let repo_a = dir.path().join("a");
    cmd_init(&repo_a, "p").unwrap();
    let files = [
        ("uno.rs", "fn one() -> i32 { 1 }"),
        ("dos.rs", "fn two() -> i32 { 2 }"),
        ("tres.rs", "fn three() -> i32 { 3 }"),
    ];
    let mut alphas = Vec::new();
    for (n, src) in &files {
        let p = dir.path().join(n);
        std::fs::write(&p, src).unwrap();
        alphas.push(cmd_ingest(&repo_a, "p", &p).unwrap().alpha);
    }

    let pkg = dir.path().join("triple.bundle");
    let exp = cmd_bundle_export_all(&repo_a, "p", &pkg).unwrap();
    assert_eq!(exp.roots, 3);
    assert!(exp.skipped_missing_dialect.is_empty());
    assert_eq!(exp.total_attestations, 3);

    let repo_b = dir.path().join("b");
    cmd_init(&repo_b, "p").unwrap();
    let imp = cmd_bundle_import_all(&repo_b, "p", &pkg).unwrap();
    assert_eq!(imp.items.len(), 3);
    assert_eq!(imp.roots_new(), 3);
    assert_eq!(imp.total_attestations_added(), 3);

    let s = cmd_status(&repo_b, "p").unwrap();
    assert_eq!(s.roots_len, 3);
    assert_eq!(s.attestations_len, 3);
    for a in &alphas {
        let shown = cmd_show(&repo_b, "p", &a.to_string(), false).unwrap();
        assert_eq!(shown.alpha, Some(*a));
    }
}

#[test]
fn multi_bundle_import_is_idempotent() {
    let dir = TempDir::new().unwrap();
    let repo_a = dir.path().join("a");
    cmd_init(&repo_a, "p").unwrap();
    let p1 = dir.path().join("a1.rs");
    std::fs::write(&p1, "fn a1() -> i32 { 1 }").unwrap();
    cmd_ingest(&repo_a, "p", &p1).unwrap();
    let p2 = dir.path().join("a2.rs");
    std::fs::write(&p2, "fn a2() -> i32 { 2 }").unwrap();
    cmd_ingest(&repo_a, "p", &p2).unwrap();

    let pkg = dir.path().join("multi.bundle");
    cmd_bundle_export_all(&repo_a, "p", &pkg).unwrap();
    let repo_b = dir.path().join("b");
    cmd_init(&repo_b, "p").unwrap();

    let r1 = cmd_bundle_import_all(&repo_b, "p", &pkg).unwrap();
    assert_eq!(r1.roots_new(), 2);
    let r2 = cmd_bundle_import_all(&repo_b, "p", &pkg).unwrap();
    assert_eq!(r2.roots_new(), 0, "segunda corrida: nada nuevo");
    assert_eq!(r2.total_nodes_inserted(), 0);
    assert_eq!(r2.total_attestations_added(), 0);
}

#[test]
fn multi_bundle_rejects_single_bundle_and_viceversa() {
    // Cruzar single ↔ multi import debe devolver el error específico,
    // no `InvalidBundle`. Eso le ahorra al usuario el "qué archivo es
    // esto" cuando ya tiene la respuesta a mano.
    let dir = TempDir::new().unwrap();
    let repo_a = dir.path().join("a");
    cmd_init(&repo_a, "p").unwrap();
    let p = dir.path().join("x.rs");
    std::fs::write(&p, "fn x() -> i32 { 0 }").unwrap();
    let ing = cmd_ingest(&repo_a, "p", &p).unwrap();

    let single = dir.path().join("s.bundle");
    cmd_bundle_export(&repo_a, "p", &ing.alpha.to_string(), &single).unwrap();
    let multi = dir.path().join("m.bundle");
    cmd_bundle_export_all(&repo_a, "p", &multi).unwrap();

    let repo_b = dir.path().join("b");
    cmd_init(&repo_b, "p").unwrap();
    let err1 = cmd_bundle_import(&repo_b, "p", &multi).unwrap_err();
    assert!(matches!(err1, CliError::ExpectedSingleBundle));
    let err2 = cmd_bundle_import_all(&repo_b, "p", &single).unwrap_err();
    assert!(matches!(err2, CliError::ExpectedMultiBundle));
}
