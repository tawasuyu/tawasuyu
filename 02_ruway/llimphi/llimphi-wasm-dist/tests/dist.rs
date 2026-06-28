//! Certificación headless de la distribución por hash: CAS, integridad,
//! concesión Ed25519 contra anillo, permisos efectivos, y correr la app
//! resuelta de punta a punta. Sin red ni GPU.

use agora_core::Keypair;
use app_bus::Launch;
use format::{ConcesionCapacidad, Hash, Permisos, PERMISO_GRAFO_ESCRITURA, PERMISO_RED};
use llimphi_wasm_dist::{
    bytecode_hash, hash_to_hex, resolve, resolve_launch, verify_grant, verify_integrity, AppRef,
    BlobSource, DiskStore, DistError, TrustRing,
};

/// El mismo wasm que corre el runner Tier 3 — lo distribuimos por hash.
const COUNTER_WASM: &[u8] = include_bytes!("../../llimphi-wasm-runner/assets/counter.wasm");

const SEED: [u8; 32] = [7; 32];

fn tmpdir(nombre: &str) -> std::path::PathBuf {
    let dir = std::env::temp_dir().join(format!("llimphi-wasm-dist-{nombre}"));
    let _ = std::fs::remove_dir_all(&dir);
    dir
}

/// Firma una concesión como lo haría `agora-cli wawa concesion` (offline).
fn firmar(kp: &Keypair, bytecode: Hash, permisos: Permisos) -> ConcesionCapacidad {
    let mensaje = format::mensaje_capacidad(&bytecode, permisos);
    ConcesionCapacidad {
        bytecode,
        permisos,
        autor: kp.public_key(),
        firma: kp.sign(&mensaje),
    }
}

/// Source en memoria que devuelve bytes arbitrarios — para forzar tampering.
struct FixedSource(Vec<u8>);
impl BlobSource for FixedSource {
    fn fetch(&self, _hash: &Hash) -> Option<Vec<u8>> {
        Some(self.0.clone())
    }
}

#[test]
fn disk_store_round_trip() {
    let store = DiskStore::open(tmpdir("rt")).unwrap();
    let h = store.put(COUNTER_WASM).unwrap();
    assert_eq!(h, bytecode_hash(COUNTER_WASM), "put devuelve el hash canónico");
    assert_eq!(store.get(&h).as_deref(), Some(COUNTER_WASM), "get recupera los bytes");
    assert_eq!(store.fetch(&[0u8; 32]), None, "hash desconocido ⇒ None");
}

#[test]
fn integridad_detecta_tampering() {
    let h = bytecode_hash(COUNTER_WASM);
    assert!(verify_integrity(COUNTER_WASM, &h));

    let mut roto = COUNTER_WASM.to_vec();
    roto[100] ^= 0xff;
    assert!(!verify_integrity(&roto, &h), "un byte cambiado ⇒ falla");

    // Un source que entrega bytes corruptos bajo el hash legítimo es rechazado.
    let source = FixedSource(roto);
    let err = resolve(&source, &TrustRing::empty(), &AppRef::pure(h)).unwrap_err();
    assert_eq!(err, DistError::IntegridadFallo);
}

#[test]
fn concesion_valida_da_permisos() {
    let store = DiskStore::open(tmpdir("grant-ok")).unwrap();
    let h = store.put(COUNTER_WASM).unwrap();
    let kp = Keypair::from_seed(SEED);
    let grant = firmar(&kp, h, PERMISO_RED);
    let trust = TrustRing::new(vec![kp.public_key()]);

    // La concesión sola verifica y rinde sus permisos.
    assert_eq!(verify_grant(&grant, &trust), Ok(PERMISO_RED));

    // Resuelta con declarados = RED ⇒ efectivos = RED.
    let app = AppRef {
        bytecode: h,
        declarados: PERMISO_RED,
        concesion: Some(grant),
    };
    let verified = resolve(&store, &trust, &app).unwrap();
    assert_eq!(verified.permisos, PERMISO_RED);
    assert_eq!(verified.wasm, COUNTER_WASM);
}

#[test]
fn concesion_autor_no_confiable() {
    let kp = Keypair::from_seed(SEED);
    let grant = firmar(&kp, bytecode_hash(COUNTER_WASM), PERMISO_RED);
    // Anillo vacío: el autor no está → rechazo antes de gastar cripto.
    assert_eq!(verify_grant(&grant, &TrustRing::empty()), Err(DistError::AutorNoConfiable));
}

#[test]
fn concesion_firma_invalida() {
    let kp = Keypair::from_seed(SEED);
    let mut grant = firmar(&kp, bytecode_hash(COUNTER_WASM), PERMISO_RED);
    grant.firma[0] ^= 0xff; // corrompe la firma
    let trust = TrustRing::new(vec![kp.public_key()]);
    assert_eq!(verify_grant(&grant, &trust), Err(DistError::FirmaInvalida));
}

#[test]
fn concesion_para_otro_bytecode() {
    let store = DiskStore::open(tmpdir("otro-bc")).unwrap();
    let h = store.put(COUNTER_WASM).unwrap();
    let kp = Keypair::from_seed(SEED);
    // Concesión firmada para OTRO bytecode (hash distinto), válida en sí misma.
    let otro = bytecode_hash(b"otra app distinta");
    let grant = firmar(&kp, otro, PERMISO_RED);
    let trust = TrustRing::new(vec![kp.public_key()]);

    let app = AppRef {
        bytecode: h,
        declarados: PERMISO_RED,
        concesion: Some(grant),
    };
    assert_eq!(
        resolve(&store, &trust, &app).unwrap_err(),
        DistError::ConcesionParaOtroBytecode
    );
}

#[test]
fn permisos_efectivos_son_interseccion() {
    let store = DiskStore::open(tmpdir("interseccion")).unwrap();
    let h = store.put(COUNTER_WASM).unwrap();
    let kp = Keypair::from_seed(SEED);
    // La concesión otorga RED|GRAFO, pero el manifiesto sólo declara RED.
    let grant = firmar(&kp, h, PERMISO_RED | PERMISO_GRAFO_ESCRITURA);
    let trust = TrustRing::new(vec![kp.public_key()]);

    let app = AppRef {
        bytecode: h,
        declarados: PERMISO_RED,
        concesion: Some(grant),
    };
    let verified = resolve(&store, &trust, &app).unwrap();
    // efectivos = declarados & concedidos = RED. No escala a GRAFO.
    assert_eq!(verified.permisos, PERMISO_RED);
}

#[test]
fn resolve_launch_y_corre_la_app() {
    // El camino "abrir app distribuida": el Launch lleva sólo el hash hex.
    let store = DiskStore::open(tmpdir("e2e")).unwrap();
    let h = store.put(COUNTER_WASM).unwrap();
    let launch = Launch::Wasm {
        bytecode_hex: hash_to_hex(&h),
    };

    let verified = resolve_launch(&store, &TrustRing::empty(), &launch).unwrap();
    assert_eq!(verified.bytecode, h);
    assert_eq!(verified.permisos, 0, "UI pura sin concesión ⇒ sin permisos");

    // Y la app resuelta CORRE: cargamos el guest y lo manejamos.
    let mut guest = verified.load().unwrap();
    let n0 = guest.view().children[0].text.as_ref().unwrap().content.clone();
    assert_eq!(n0, "0");
    guest.dispatch(&[0]).unwrap(); // Msg::Increment
    let n1 = guest.view().children[0].text.as_ref().unwrap().content.clone();
    assert_eq!(n1, "1", "la app distribuida por hash incrementa de verdad");
}
