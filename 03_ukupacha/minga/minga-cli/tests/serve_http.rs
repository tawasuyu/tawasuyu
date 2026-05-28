//! Smoke del daemon HTTP: levanta el router en proceso, manda requests
//! sintéticas via `tower::ServiceExt::oneshot`, valida JSON.
//!
//! No abre un socket real — el server arranca en tests E2E sería más
//! ruido que valor.

use std::fs;

use axum::body::Body;
use axum::http::{Request, StatusCode};
use http_body_util::BodyExt;
use minga_cli::serve::build_router_for_test;
use minga_cli::{cmd_init, cmd_ingest};
use serde_json::Value;
use tempfile::TempDir;
use tower::ServiceExt;

fn populate_repo() -> (TempDir, String) {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("r");
    cmd_init(&repo, "p").unwrap();
    let src = dir.path().join("hola.rs");
    fs::write(&src, "fn hola() -> i32 { 1 }").unwrap();
    let alpha = cmd_ingest(&repo, "p", &src).unwrap().alpha.to_string();
    let r = repo.to_string_lossy().to_string();
    std::mem::forget(dir);
    let kept = TempDir::new().unwrap();
    // El dir original ya tiene contenido pero no podemos retornarlo
    // post-forget; re-encapsulamos.
    (kept, format!("{}|{}", r, alpha))
}

#[tokio::test]
async fn http_status_returns_counts() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("r");
    cmd_init(&repo, "p").unwrap();
    let src = dir.path().join("a.rs");
    fs::write(&src, "fn a() -> i32 { 1 }").unwrap();
    cmd_ingest(&repo, "p", &src).unwrap();

    let app = build_router_for_test(repo.clone(), "p".to_string());
    let res = app
        .oneshot(Request::builder().uri("/status").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    assert_eq!(json["roots"], 1);
    assert_eq!(json["attestations"], 1);
}

#[tokio::test]
async fn http_roots_lists_ingested_alpha() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("r");
    cmd_init(&repo, "p").unwrap();
    let src = dir.path().join("b.rs");
    fs::write(&src, "fn b() -> i32 { 9 }").unwrap();
    let ing = cmd_ingest(&repo, "p", &src).unwrap();

    let app = build_router_for_test(repo.clone(), "p".to_string());
    let res = app
        .oneshot(Request::builder().uri("/roots").body(Body::empty()).unwrap())
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    let items = json["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["alpha"], ing.alpha.to_string());
    assert_eq!(items[0]["dialect"], "rust");
    assert_eq!(items[0]["attestations"], 1);
}

#[tokio::test]
async fn http_show_returns_rendered_source() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("r");
    cmd_init(&repo, "p").unwrap();
    let src = dir.path().join("c.rs");
    fs::write(&src, "fn c() -> i32 { 7 }").unwrap();
    let ing = cmd_ingest(&repo, "p", &src).unwrap();

    let app = build_router_for_test(repo.clone(), "p".to_string());
    let res = app
        .oneshot(
            Request::builder()
                .uri(format!("/roots/{}/show", ing.alpha))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let body = String::from_utf8(bytes.to_vec()).unwrap();
    assert!(body.contains("fn"), "render incluye 'fn'; got: {body}");
}

#[tokio::test]
async fn http_show_unknown_alpha_is_404() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("r");
    cmd_init(&repo, "p").unwrap();

    let app = build_router_for_test(repo.clone(), "p".to_string());
    let fake = "00112233445566778899aabbccddeeff00112233445566778899aabbccddeeff";
    let res = app
        .oneshot(
            Request::builder()
                .uri(format!("/roots/{fake}/show"))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::NOT_FOUND);
}

#[tokio::test]
async fn http_show_invalid_hash_is_400() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("r");
    cmd_init(&repo, "p").unwrap();

    let app = build_router_for_test(repo.clone(), "p".to_string());
    let res = app
        .oneshot(
            Request::builder()
                .uri("/roots/no-soy-un-hash/show")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::BAD_REQUEST);
}

#[tokio::test]
async fn http_signers_returns_local_author() {
    let dir = TempDir::new().unwrap();
    let repo = dir.path().join("r");
    let did = cmd_init(&repo, "p").unwrap();
    let src = dir.path().join("s.rs");
    fs::write(&src, "fn s() -> i32 { 4 }").unwrap();
    let ing = cmd_ingest(&repo, "p", &src).unwrap();

    let app = build_router_for_test(repo.clone(), "p".to_string());
    let res = app
        .oneshot(
            Request::builder()
                .uri(format!("/roots/{}/signers", ing.alpha))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();
    assert_eq!(res.status(), StatusCode::OK);
    let bytes = res.into_body().collect().await.unwrap().to_bytes();
    let json: Value = serde_json::from_slice(&bytes).unwrap();
    let items = json["items"].as_array().unwrap();
    assert_eq!(items.len(), 1);
    assert_eq!(items[0]["author"], did.to_string());
}

// Silenciamos el unused warning del helper de exploración.
#[allow(dead_code)]
fn _quiet_populate_unused() {
    let _ = populate_repo;
}
