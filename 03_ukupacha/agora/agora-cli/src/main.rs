//! `agora-cli` — operación shell del ágora.
//!
//! Comparte keystore y grafo con [`agora-app`]. Es la cara CLI del
//! mismo dominio: lo que se crea acá aparece en la UI y viceversa.
//!
//! Módulos:
//! - `cmd`        — definición clap de todos los subcomandos
//! - `sesion`     — estado de sesión, tipo `Error`, helpers de bajo nivel
//! - `identidad`  — handlers de `agora-cli identidad`
//! - `atestacion` — handlers de `atestar`, `verificar`, `exportar`, `importar`, `grafo`
//! - `canal`      — handlers de `agora-cli canal`
//! - `wawa`       — handlers de `agora-cli wawa`

mod atestacion;
mod canal;
mod cmd;
mod identidad;
mod sesion;
mod wawa;

use std::process::ExitCode;

use clap::Parser;

use cmd::{AtestacionOp, CanalOp, Cmd, IdentidadOp, WawaOp};
use sesion::CliResult;

// =============================================================================
//  CLI shape
// =============================================================================

#[derive(Parser)]
#[command(name = "agora-cli", about = "Shell para el ágora — identidad, atestaciones, grafo.")]
struct Cli {
    #[command(subcommand)]
    cmd: Cmd,
}

// =============================================================================
//  Dispatch
// =============================================================================

fn run(cmd: Cmd) -> CliResult<()> {
    match cmd {
        Cmd::Identidad { op } => match op {
            IdentidadOp::Nueva { name, kind, seed_stdin } => {
                identidad::identidad_nueva(name, kind.into(), seed_stdin)
            }
            IdentidadOp::Listar => identidad::identidad_listar(),
            IdentidadOp::Exportar { id } => identidad::identidad_exportar(&id),
            IdentidadOp::Rename { id, nombre } => identidad::identidad_rename(&id, &nombre),
            IdentidadOp::Remove { id, force, purgar_keystore } => {
                identidad::identidad_remove(&id, force, purgar_keystore)
            }
            IdentidadOp::Rotar { id, nombre, seed_stdin } => {
                identidad::identidad_rotar(&id, nombre, seed_stdin)
            }
            IdentidadOp::Revocar { id, motivo, umbral, vence_en_seg } => {
                identidad::identidad_revocar(&id, motivo.into(), umbral, vence_en_seg)
            }
        },
        Cmd::Atestacion { op } => match op {
            AtestacionOp::Listar { subject, attester, predicate } => {
                atestacion::atestacion_listar(
                    subject.as_deref(),
                    attester.as_deref(),
                    predicate.as_deref(),
                )
            }
        },
        Cmd::Atestar { como, sobre, pred, valor } => {
            atestacion::atestar(&como, &sobre, &pred, &valor)
        }
        Cmd::Verificar { archivo } => atestacion::verificar(&archivo),
        Cmd::Exportar { archivo } => atestacion::exportar(&archivo),
        Cmd::Importar { archivo } => atestacion::importar(&archivo),
        Cmd::Grafo => atestacion::grafo_resumen(),
        Cmd::Canal { op } => match op {
            CanalOp::Nuevo { nombre, autor, salida } => {
                canal::canal_nuevo(&nombre, &autor, &salida)
            }
            CanalOp::Extender { archivo, raiz } => canal::canal_extender(&archivo, &raiz),
            CanalOp::Verificar { archivo } => canal::canal_verificar(&archivo),
            CanalOp::Mostrar { archivo } => canal::canal_mostrar(&archivo),
        },
        Cmd::Wawa { op } => match op {
            WawaOp::Publicar { como, spec, salida } => wawa::wawa_publicar(&como, &spec, &salida),
            WawaOp::Anunciar { iface, dir, segundos } => {
                wawa::wawa_anunciar(&iface, &dir, segundos)
            }
            WawaOp::Importar { dir, salida } => wawa::wawa_importar(&dir, &salida),
            WawaOp::ImportarImagen { imagen, salida, particion } => {
                wawa::wawa_importar_imagen(&imagen, &salida, particion)
            }
            WawaOp::Exportar { bundle, raiz, destino } => {
                wawa::wawa_exportar(&bundle, raiz.as_deref(), &destino)
            }
            WawaOp::ForjarClave { name } => wawa::wawa_forjar_clave(&name),
            WawaOp::ForjarPropuesta { como, hash, salida } => {
                wawa::wawa_forjar_propuesta(&como, &hash, &salida)
            }
            WawaOp::Concesion { como, wasm, permisos, salida } => {
                wawa::wawa_concesion(&como, &wasm, &permisos, &salida)
            }
            WawaOp::Revocar { objetivo, como, motivo, vence_en_seg, salida } => {
                wawa::wawa_revocar(&objetivo, &como, motivo.into(), vence_en_seg, &salida)
            }
        },
    }
}

// =============================================================================
//  Entrypoint
// =============================================================================

fn main() -> ExitCode {
    let cli = Cli::parse();
    match run(cli.cmd) {
        Ok(()) => ExitCode::SUCCESS,
        Err(e) => {
            eprintln!("agora-cli: {e}");
            ExitCode::FAILURE
        }
    }
}

// =============================================================================
//  Tests de contrato cross-crate
// =============================================================================

#[cfg(test)]
mod tests {
    use wawa::parse_permisos;

    use super::*;

    #[test]
    fn parse_permisos_numerico_y_por_nombre_coinciden() {
        // Decimal, hex, binario y nombres deben dar el mismo bitfield.
        let esperado = format::PERMISO_RED | format::PERMISO_RAIZ; // 1 | 4 = 5
        assert_eq!(parse_permisos("5").unwrap(), esperado);
        assert_eq!(parse_permisos("0x5").unwrap(), esperado);
        assert_eq!(parse_permisos("0b101").unwrap(), esperado);
        assert_eq!(parse_permisos("RED,RAIZ").unwrap(), esperado);
        // Tolerancia: espacios, minúsculas, prefijo PERMISO_, orden.
        assert_eq!(parse_permisos(" raiz , permiso_red ").unwrap(), esperado);
    }

    #[test]
    fn parse_permisos_rechaza_nombre_desconocido() {
        assert!(matches!(parse_permisos("RED,VOLAR"), Err(sesion::Error::Permiso(_))));
    }

    #[test]
    fn parse_permisos_cubre_todas_las_constantes() {
        let todas = parse_permisos(
            "RED,GRAFO_ESCRITURA,RAIZ,ALTAVOZ,CONFIG,COMPACTAR,TINKUY",
        )
        .unwrap();
        let esperado = format::PERMISO_RED
            | format::PERMISO_GRAFO_ESCRITURA
            | format::PERMISO_RAIZ
            | format::PERMISO_ALTAVOZ
            | format::PERMISO_CONFIG
            | format::PERMISO_COMPACTAR
            | format::PERMISO_TINKUY;
        assert_eq!(todas, esperado);
    }

    /// EL CONTRATO de la ceremonia: el hash que `wawa concesion` firma debe ser
    /// IDÉNTICO al `EntradaApp.bytecode` que `construir_release` ancla para el
    /// mismo `.wasm` — si no, la concesión cubre un hash que el kernel nunca verá
    /// y la app correría con 0 capacidades. Locked aquí contra drift.
    #[test]
    fn hash_objeto_bytecode_coincide_con_construir_release() {
        let wasm = b"\0asm-binario-de-prueba".to_vec();

        // Vía `wawa concesion` (replicado: la fn lee de disco, el cómputo es éste).
        let obj = format::Objeto { datos: wasm.clone(), hijos: Vec::new() };
        let mio = format::hash(&obj.serializar().unwrap());

        // Vía release: el bytecode anclado en la EntradaApp.
        let kp = agora_core::Keypair::from_seed([3u8; 32]);
        let app = agora_channel::AppSpec {
            nombre: "x".into(),
            bytecode: wasm,
            region: (0, 0, 1, 1),
            techo_memoria: 1024,
            fuel_fotograma: 1,
            permisos: format::PERMISO_RAIZ,
        };
        let r = agora_channel::construir_release(&[app], &kp, "dev", 1).unwrap();
        let mobj = format::Objeto::deserializar(
            &r.objetos.iter().find(|o| o.hash == r.manifiesto).unwrap().payload,
        )
        .unwrap();
        let manifiesto = format::Manifiesto::deserializar(&mobj.datos).unwrap();

        assert_eq!(mio, manifiesto.apps[0].bytecode, "el hash de la concesión debe ser el del objeto-bytecode anclado");
    }

    /// EL CONTRATO cross-frontera de la revocación: el overlay que `wawa revocar`
    /// produce debe (a) round-trippear por `format::OverlayRevocacion` y (b) que
    /// cada firma verifique sobre `format::mensaje_revocacion_clave` — el MISMO
    /// canónico que `claves::verificar_revocacion` reconstruye en el kernel. Si
    /// esto pasa, el kernel acreditaría los firmantes y alcanzaría el quórum.
    #[test]
    fn overlay_revocacion_wire_verifica_bajo_el_canonico_del_kernel() {
        let slot0 = agora_core::Keypair::from_seed([10u8; 32]);
        let slot1 = agora_core::Keypair::from_seed([11u8; 32]);
        let slot2 = agora_core::Keypair::from_seed([12u8; 32]);
        let objetivo = slot0.public_key(); // la clave comprometida

        // Construir el overlay igual que `wawa_revocar` (slot1 + slot2 firman).
        let now = 1_700_000_000;
        let rev = agora_core::Revocation::create(
            objetivo,
            agora_core::RevReason::Compromised,
            now,
            None,
            &[&slot1, &slot2],
        );
        let firmantes: Vec<format::FirmaRevocacion> = rev
            .authorizers
            .signers
            .iter()
            .map(|s| format::FirmaRevocacion { autor: s.public_key, firma: s.signature })
            .collect();
        let overlay = format::OverlayRevocacion {
            version: format::VERSION_OVERLAY,
            revocaciones: vec![format::RevocacionFirmada {
                objetivo,
                motivo: 0,
                emitida_en: now,
                vence_en: None,
                firmantes,
            }],
        };

        // (a) round-trip envuelto en un Objeto del grafo, como en disco.
        let obj = format::Objeto { datos: overlay.serializar().unwrap(), hijos: Vec::new() };
        let leido_obj = format::Objeto::deserializar(&obj.serializar().unwrap()).unwrap();
        let leido = format::OverlayRevocacion::deserializar(&leido_obj.datos).unwrap();
        let rf = &leido.revocaciones[0];

        // (b) reconstruir el canónico EXACTO del kernel y verificar cada firma.
        let mensaje = format::mensaje_revocacion_clave(
            &rf.objetivo,
            rf.motivo,
            rf.emitida_en,
            rf.vence_en,
        );
        let ring = [slot0.public_key(), slot1.public_key(), slot2.public_key()];
        let mut slots_acreditados = 0u32;
        for f in &rf.firmantes {
            // El objetivo comprometido no cuenta (espejo de verificar_revocacion).
            if f.autor == objetivo {
                continue;
            }
            if let Some(slot) = ring.iter().position(|k| *k == f.autor) {
                if agora_core::verify_signature(&f.autor, &mensaje, &f.firma).is_ok() {
                    slots_acreditados |= 1 << slot;
                }
            }
        }
        // 2 slots distintos del anillo respaldan ⇒ alcanza el quórum 2-of-3.
        assert_eq!(slots_acreditados.count_ones(), 2);
    }
}
