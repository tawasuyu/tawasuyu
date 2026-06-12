//! Handlers para el subcomando `agora-cli wawa`.
//!
//! Operaciones host-side del dominio wawa: forjar claves para el AGORA_AUTH_RING,
//! concesiones de capacidad, propuestas de manifiesto, publicar/anunciar releases,
//! importar/exportar árboles direccionados por contenido e imágenes de dispositivo.

use std::fs;
use std::path::Path;

use agora_core::{Keypair, RevReason, Revocation};
use rand::RngCore;

use crate::sesion::{ahora_unix, hex_de, parse_hash, parse_hex_32, CliResult, Error, Sesion};

// =============================================================================
//  Tipos internos del spec de release
// =============================================================================

/// El spec JSON de un release: el canal + el conjunto COMPLETO de apps. Es lo
/// que un humano o Claude escribe a mano — la cara legible del manifiesto.
#[derive(serde::Deserialize)]
struct SpecRelease {
    #[serde(default = "canal_por_defecto")]
    canal: String,
    apps: Vec<SpecApp>,
}

fn canal_por_defecto() -> String {
    "dev".to_string()
}

/// Una app dentro del spec. `wasm` es la ruta al `.wasm` ya compilado
/// (relativa al directorio del spec si no es absoluta).
#[derive(serde::Deserialize)]
struct SpecApp {
    nombre: String,
    wasm: String,
    /// `[x, y, ancho, alto]` del lienzo natural.
    region: [u32; 4],
    #[serde(default = "techo_por_defecto")]
    techo_memoria: u32,
    fuel: u32,
    #[serde(default)]
    permisos: u32,
}

fn techo_por_defecto() -> u32 {
    4 * 1024 * 1024
}

// =============================================================================
//  forjar-clave
// =============================================================================

pub fn wawa_forjar_clave(name: &str) -> CliResult<()> {
    let mut s = Sesion::abrir()?;
    let mut seed = [0u8; 32];
    rand::thread_rng().fill_bytes(&mut seed);
    let kp = Keypair::from_seed(seed);
    let id = kp.identity_id();
    s.keystore.save(id, &seed, &s.passphrase).map_err(Error::Keystore)?;
    s.graph.register(kp.identity(agora_core::IdentityKind::Person, name));
    s.guardar()?;

    println!("clave forjada para AGORA_AUTH_RING:");
    println!("  id     {}", hex_de(id.as_bytes()));
    println!("  pubkey {}", hex_de(&kp.public_key()));
    println!();
    println!("Para empotrar en wawa-kernel/src/claves.rs:");
    println!("  pub const AGORA_AUTH_RING: [[u8; 32]; N] = [");
    println!("      // slot X :: {name}");
    print!("      [");
    for (i, b) in kp.public_key().iter().enumerate() {
        if i % 8 == 0 {
            println!();
            print!("          ");
        }
        print!("0x{b:02x}, ");
    }
    println!();
    println!("      ],");
    println!("      // ... otros slots");
    println!("  ];");
    println!();
    println!("La seed correspondiente vive cifrada en el keystore local.");
    println!("Hacer backup con: agora-cli identidad exportar {id}");
    Ok(())
}

// =============================================================================
//  forjar-propuesta
// =============================================================================

pub fn wawa_forjar_propuesta(como: &str, hash_hex: &str, salida: &Path) -> CliResult<()> {
    let s = Sesion::abrir()?;
    let como_id = s.resolver_id(como)?;
    let kp = s.cargar_keypair(como_id)?;
    let manifiesto_hash = parse_hash(hash_hex)?;
    let mf = agora_channel::firmar_manifiesto(&kp, &manifiesto_hash);
    let bytes = mf.serializar().map_err(Error::Canal)?;
    if bytes.len() != 128 {
        return Err(Error::Canal("ManifiestoFirmado postcard ≠ 128 bytes (contrato roto)"));
    }
    fs::write(salida, &bytes)?;
    println!("propuesta forjada: {} bytes → {}", bytes.len(), salida.display());
    println!("  manifiesto_hash : {}", hex_de(&manifiesto_hash));
    println!("  autor (pubkey)  : {}", hex_de(&mf.autor));
    println!("  firma           : {}...{} (64 B)", hex_de(&mf.firma[..4]), hex_de(&mf.firma[60..]));
    println!();
    println!("Para que wawa-kernel lo acepte, la pubkey del autor debe");
    println!("estar en AGORA_AUTH_RING de claves.rs. Si no está, mudanza");
    println!("la verifica en userspace OK y el kernel responde con");
    println!("CapacidadInsuficiente.");
    Ok(())
}

// =============================================================================
//  concesion (§14.1.3)
// =============================================================================

/// Parsea la especificación de permisos del CLI: o una máscara numérica
/// (`6`, `0x6`, `0b110`) o una lista de nombres separados por coma
/// (`RED,RAIZ`). Tolerante a mayúsculas/minúsculas y al prefijo `PERMISO_`.
/// Los nombres reflejan las constantes `format::PERMISO_*` — única fuente de
/// verdad del bitfield, así un permiso nuevo en `format` se nombra aquí sin
/// re-derivar números a mano.
pub fn parse_permisos(s: &str) -> CliResult<format::Permisos> {
    let t = s.trim();
    // Camino numérico: una sola palabra que parsea como entero.
    if !t.contains(',') {
        if let Some(hex) = t.strip_prefix("0x").or_else(|| t.strip_prefix("0X")) {
            return u32::from_str_radix(hex, 16).map_err(|_| Error::Permiso(s.to_string()));
        }
        if let Some(bin) = t.strip_prefix("0b").or_else(|| t.strip_prefix("0B")) {
            return u32::from_str_radix(bin, 2).map_err(|_| Error::Permiso(s.to_string()));
        }
        if t.chars().all(|c| c.is_ascii_digit()) && !t.is_empty() {
            return t.parse::<u32>().map_err(|_| Error::Permiso(s.to_string()));
        }
    }
    // Camino por nombres.
    let mut mask: format::Permisos = 0;
    for parte in t.split(',') {
        let nombre = parte.trim().trim_start_matches("PERMISO_").trim_start_matches("permiso_");
        let bit = match nombre.to_ascii_uppercase().as_str() {
            "RED" => format::PERMISO_RED,
            "GRAFO_ESCRITURA" | "GRAFO" => format::PERMISO_GRAFO_ESCRITURA,
            "RAIZ" => format::PERMISO_RAIZ,
            "ALTAVOZ" => format::PERMISO_ALTAVOZ,
            "CONFIG" => format::PERMISO_CONFIG,
            "COMPACTAR" => format::PERMISO_COMPACTAR,
            "TINKUY" => format::PERMISO_TINKUY,
            "" => continue,
            otro => {
                return Err(Error::Permiso(format!(
                    "nombre de permiso desconocido «{otro}» (válidos: RED, GRAFO_ESCRITURA, \
                     RAIZ, ALTAVOZ, CONFIG, COMPACTAR, TINKUY)"
                )))
            }
        };
        mask |= bit;
    }
    Ok(mask)
}

/// `agora-cli wawa concesion` — la ceremonia OFFLINE de §14.1.3. Firma el par
/// `(hash_objeto_bytecode, permisos)` con la seed del operador y emite la
/// `ConcesionCapacidad` envuelta en un `Objeto` del grafo. El hash que firma es
/// el del OBJETO (`Objeto{datos:wasm,hijos:[]}` → BLAKE3), idéntico al que el
/// génesis y `construir_release` anclan — atar la firma a los bytes crudos NO
/// serviría: el kernel verifica contra `EntradaApp.bytecode`, que es el del objeto.
pub fn wawa_concesion(como: &str, wasm: &Path, permisos_spec: &str, salida: &Path) -> CliResult<()> {
    let s = Sesion::abrir()?;
    let como_id = s.resolver_id(como)?;
    let kp = s.cargar_keypair(como_id)?;

    let permisos = parse_permisos(permisos_spec)?;
    if permisos == 0 {
        return Err(Error::Permiso(
            "una concesión de 0 permisos no tiene sentido: la app correría sin \
             capacidades gateadas igual con concesion: None"
                .to_string(),
        ));
    }

    // El hash del OBJETO-bytecode, calculado EXACTAMENTE como el génesis: los
    // bytes del `.wasm` envueltos en un `Objeto` sin hijos, serializados, BLAKE3.
    let wasm_bytes = fs::read(wasm)
        .map_err(|e| Error::Release(format!("no pude leer {}: {e}", wasm.display())))?;
    let bytecode_obj = format::Objeto { datos: wasm_bytes, hijos: Vec::new() };
    let bytecode_payload = bytecode_obj.serializar().map_err(Error::Canal)?;
    let bytecode_hash = format::hash(&bytecode_payload);

    // La concesión firmada, y su forma como objeto del grafo.
    let concesion = agora_channel::firmar_capacidad(&kp, &bytecode_hash, permisos);
    let datos = concesion.serializar().map_err(Error::Canal)?;
    let concesion_obj = format::Objeto { datos, hijos: Vec::new() };
    let payload = concesion_obj.serializar().map_err(Error::Canal)?;
    let concesion_hash = format::hash(&payload);

    fs::write(salida, &payload)?;

    println!("concesión forjada → {}", salida.display());
    println!("  bytecode (objeto) : {}", hex_de(&bytecode_hash));
    println!("  permisos          : {permisos:#09b} ({permisos})");
    println!("  autor (pubkey)    : {}", hex_de(&concesion.autor));
    println!("  concesion_hash    : {}", hex_de(&concesion_hash));
    println!();
    println!("Es un objeto del grafo: siémbralo en el génesis y referencia su hash");
    println!("desde `EntradaApp.concesion` de la app cuyo bytecode coincide. Cuando");
    println!("`wawa-boot` lea las concesiones de sus assets y las ancle, este `None`");
    println!("pasa a `Some(concesion_hash)` (ver SDD-capacidades §3.3).");
    println!();
    println!("La pubkey del autor DEBE habitar AGORA_AUTH_RING de claves.rs, o el");
    println!("kernel rechaza la concesión (CapacidadInsuficiente) y la app corre");
    println!("con 0 capacidades gateadas.");
    Ok(())
}

// =============================================================================
//  revocar (plano de control — overlay)
// =============================================================================

/// `agora-cli wawa revocar` — forja el overlay de revocación del plano de control.
pub fn wawa_revocar(
    objetivo_hex: &str,
    como: &str,
    motivo: RevReason,
    vence_en_seg: Option<u64>,
    salida: &Path,
) -> CliResult<()> {
    let s = Sesion::abrir()?;

    // El objetivo es una PUBKEY cruda (la clave del anillo a apagar), no un id
    // del grafo: 64 chars hex → 32 bytes.
    let objetivo = parse_hash(objetivo_hex)?;

    // Los firmantes: cada identidad de `--como` (coma-separada) con seed local.
    let mut firmantes_kp: Vec<Keypair> = Vec::new();
    for tok in como.split(',').map(str::trim).filter(|t| !t.is_empty()) {
        let id = s.resolver_id(tok)?;
        firmantes_kp.push(s.cargar_keypair(id)?);
    }
    if firmantes_kp.is_empty() {
        return Err(Error::Canal("--como vacío: pasá al menos un firmante del anillo"));
    }
    // Una clave comprometida no respalda su propia revocación: el kernel
    // descartaría esa firma, así que la rechazamos temprano con un mensaje claro.
    if motivo == RevReason::Compromised
        && firmantes_kp.iter().any(|kp| kp.public_key() == objetivo)
    {
        return Err(Error::Canal(
            "el objetivo no puede firmar su propia revocación por compromiso \
             (M-of-N de OTROS) — quitalo de --como",
        ));
    }

    let now = ahora_unix();
    let expires_at = vence_en_seg.map(|seg| now + seg);

    // Firmar M-of-N sobre el canónico compartido (agora-core delega en
    // format::mensaje_revocacion_clave, el MISMO que el kernel verifica).
    let refs: Vec<&Keypair> = firmantes_kp.iter().collect();
    let rev = Revocation::create(objetivo, motivo, now, expires_at, &refs);

    // Aplanar la multifirma al wire que el kernel deserializa.
    let firmantes: Vec<format::FirmaRevocacion> = rev
        .authorizers
        .signers
        .iter()
        .map(|sig| format::FirmaRevocacion { autor: sig.public_key, firma: sig.signature })
        .collect();
    let motivo_byte = match motivo {
        RevReason::Compromised => 0u8,
        RevReason::Retired => 1,
        RevReason::Superseded => 2,
    };
    let overlay = format::OverlayRevocacion {
        version: format::VERSION_OVERLAY,
        revocaciones: vec![format::RevocacionFirmada {
            objetivo,
            motivo: motivo_byte,
            emitida_en: now,
            vence_en: expires_at,
            firmantes,
        }],
    };

    // El overlay como objeto del grafo (igual trazado que wawa_concesion).
    let datos = overlay.serializar().map_err(Error::Canal)?;
    let overlay_obj = format::Objeto { datos, hijos: Vec::new() };
    let payload = overlay_obj.serializar().map_err(Error::Canal)?;
    let overlay_hash = format::hash(&payload);
    fs::write(salida, &payload)?;

    let n = firmantes_kp.len();
    println!("overlay de revocación forjado → {}", salida.display());
    println!("  objetivo (pubkey) : {}", hex_de(&objetivo));
    println!("  motivo            : {motivo_byte} ({motivo:?})");
    println!("  firmantes         : {n} (el kernel exige quórum 2-of-3 del anillo)");
    match expires_at {
        Some(t) => println!("  vence             : t={t} (fail-closed hasta RTC)"),
        None => println!("  vence             : nunca (permanente)"),
    }
    println!("  overlay_hash      : {}", hex_de(&overlay_hash));
    println!();
    println!("Es un objeto del grafo. Sembralo en los assets del génesis para que");
    println!("`wawa-boot` lo ancle en `Manifiesto.overlay_revocacion`; el kernel lo");
    println!("lee al arrancar y deniega la clave en `autor_en_anillo`. Las pubkeys");
    println!("firmantes DEBEN habitar AGORA_AUTH_RING o no suman al quórum.");
    Ok(())
}

// =============================================================================
//  publicar (Fase 64)
// =============================================================================

/// `agora-cli wawa publicar` — la mitad "fragua" del lazo Rust→wawa en vivo.
pub fn wawa_publicar(como: &str, spec_path: &Path, salida: &Path) -> CliResult<()> {
    let s = Sesion::abrir()?;
    let como_id = s.resolver_id(como)?;
    let kp = s.cargar_keypair(como_id)?;

    let texto = fs::read_to_string(spec_path)?;
    let spec: SpecRelease =
        serde_json::from_str(&texto).map_err(|e| Error::Spec(e.to_string()))?;
    if spec.apps.is_empty() {
        return Err(Error::Spec("el spec no lista ninguna app".to_string()));
    }

    // Los `.wasm` se resuelven relativos al directorio del spec si no son
    // rutas absolutas — así el spec es portable junto a sus binarios.
    let base_dir = spec_path.parent().unwrap_or_else(|| Path::new("."));
    let mut apps = Vec::with_capacity(spec.apps.len());
    for a in &spec.apps {
        let p = Path::new(&a.wasm);
        let wasm_path = if p.is_absolute() {
            p.to_path_buf()
        } else {
            base_dir.join(p)
        };
        let bytecode = fs::read(&wasm_path).map_err(|e| {
            Error::Spec(format!("no pude leer {}: {e}", wasm_path.display()))
        })?;
        apps.push(agora_channel::AppSpec {
            nombre: a.nombre.clone(),
            bytecode,
            region: (a.region[0], a.region[1], a.region[2], a.region[3]),
            techo_memoria: a.techo_memoria,
            fuel_fotograma: a.fuel,
            permisos: a.permisos,
        });
    }

    let timestamp = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map(|d| d.as_secs())
        .unwrap_or(0);

    let release = agora_channel::construir_release(&apps, &kp, &spec.canal, timestamp)
        .map_err(|e| Error::Release(e.to_string()))?;

    fs::create_dir_all(salida)?;

    // 1. Un archivo por objeto del grafo: `<hash>.obj`.
    let mut grandes = 0usize;
    for obj in &release.objetos {
        fs::write(salida.join(format!("{}.obj", hex_de(&obj.hash))), &obj.payload)?;
        if obj.payload.len() > umbral_fragmento() {
            grandes += 1;
        }
    }

    // 2. anuncio.bin — 168 B raw: canal|raiz|autor|timestamp_le(8)|firma(64).
    //    Layout fijo para que `servir_release` lo lea sin postcard.
    let mut anuncio = Vec::with_capacity(168);
    anuncio.extend_from_slice(&release.canal);
    anuncio.extend_from_slice(&release.manifiesto);
    anuncio.extend_from_slice(&release.autor);
    anuncio.extend_from_slice(&release.timestamp.to_le_bytes());
    anuncio.extend_from_slice(&release.firma_anuncio);
    fs::write(salida.join("anuncio.bin"), &anuncio)?;

    // 3. manifiesto_firmado.bin — 128 B, el sobre de sys_manifiesto_proponer
    //    (compatible con el camino `mudanza` que hornea propuesta_demo.bin).
    let mf = release.manifiesto_firmado.serializar().map_err(Error::Canal)?;
    fs::write(salida.join("manifiesto_firmado.bin"), &mf)?;

    println!("release «{}» empaquetado → {}", spec.canal, salida.display());
    println!("  apps           : {}", apps.len());
    println!("  objetos        : {}", release.objetos.len());
    println!("  manifiesto     : {}", hex_de(&release.manifiesto));
    println!("  canal          : {}", hex_de(&release.canal));
    println!("  autor (pubkey) : {}", hex_de(&release.autor));
    if grandes > 0 {
        println!();
        println!("  NOTA: {grandes} objeto(s) superan 1024 B; `servir_release` los");
        println!("  enviará PARTIDOS en ProveedorFragmento y el kernel los reensambla");
        println!("  (Fase 65). El .wasm grande viaja completo.");
    }
    println!();
    println!("Difundir + servir en vivo a una wawa en la misma red L2:");
    println!(
        "  sudo -E agora-cli wawa anunciar --iface <iface> --dir {}",
        salida.display()
    );
    Ok(())
}

/// Umbral a partir del cual `servir_release` parte un objeto en fragmentos
/// (`akasha::MAX_FRAGMENTO_DATOS`), replicado como constante local para no
/// acoplar `agora-cli` al crate `akasha` del kernel sólo por un número. Si
/// aquél cambia, este aviso queda desfasado — es sólo un AVISO informativo.
fn umbral_fragmento() -> usize {
    1024
}

// =============================================================================
//  anunciar (AoE raw socket)
// =============================================================================

/// `agora-cli wawa anunciar` — la mitad "transporte" del lazo Rust→wawa: lee el
/// bundle que `publicar` dejó en disco y lo difunde por AoE, sirviendo sus
/// objetos a las wawa que los pidan. Reusa `wawa_explorer_aoe::ClienteAoE` —el
/// mismo cliente raw-socket ya probado—, así no duplicamos el `unsafe` de libc.
pub fn wawa_anunciar(iface: &str, dir: &Path, segundos: u64) -> CliResult<()> {
    use std::time::{Duration, Instant};
    use wawa_explorer_aoe::ClienteAoE;

    let objetos = cargar_objetos_bundle(dir)?;
    let (canal, raiz, autor, timestamp, firma) = cargar_anuncio_bundle(dir)?;

    let cliente = ClienteAoE::nuevo(iface).map_err(|e| Error::Aoe(e.to_string()))?;
    println!(
        "anunciando release de {} sobre {iface} ({} objetos, MAC {})",
        dir.display(),
        objetos.len(),
        hex_de(&cliente.mac_local())
    );
    println!("  canal {} · raiz {}", hex_de(&canal), hex_de(&raiz));
    println!("  difundiendo + sirviendo {segundos}s (Ctrl-C para cortar)");

    // Lazo: anunciar, servir una ventana corta, repetir. El anuncio se re-emite
    // mientras atendemos pulls —robusto ante pérdida L2 y ante una wawa que
    // arranca después de nosotros—.
    let inicio = Instant::now();
    let total = Duration::from_secs(segundos);
    let mut servidos = 0u64;
    while inicio.elapsed() < total {
        if let Err(e) = cliente.anunciar_canal(canal, raiz, autor, timestamp, firma) {
            eprintln!("agora-cli: fallo al anunciar: {e}");
        }
        let restante = total.saturating_sub(inicio.elapsed());
        match cliente.servir(&objetos, restante.min(Duration::from_secs(5))) {
            Ok(stats) => {
                let n = stats.servidos + stats.fragmentados;
                servidos += n;
                if n > 0 {
                    println!(
                        "  +{} servidos, +{} fragmentados, {} ignorados (acum={})",
                        stats.servidos, stats.fragmentados, stats.ignorados, servidos
                    );
                }
            }
            Err(e) => return Err(Error::Aoe(e.to_string())),
        }
    }
    println!("fin. objetos servidos en total: {servidos}");
    Ok(())
}

/// Carga los `<hash>.obj` del bundle en un mapa `id → payload`, verificando que
/// el nombre (64 hex) sea el hash BLAKE3 del contenido —integridad de punta a
/// punta—. Mismo layout que escriben `publicar`/`importar`.
fn cargar_objetos_bundle(dir: &Path) -> CliResult<std::collections::HashMap<[u8; 32], Vec<u8>>> {
    let mut mapa = std::collections::HashMap::new();
    for ent in fs::read_dir(dir)? {
        let ent = ent?;
        let ruta = ent.path();
        let Some(nombre) = ruta.file_name().and_then(|n| n.to_str()).map(str::to_owned) else {
            continue;
        };
        let Some(hex_hash) = nombre.strip_suffix(".obj") else {
            continue;
        };
        let id = parse_hex_32(hex_hash).map_err(|_| Error::HashInvalido(hex_hash.to_string()))?;
        let datos = fs::read(&ruta)?;
        if format::hash(&datos) != id {
            return Err(Error::Spec(format!(
                "objeto {nombre}: el contenido no rehashea a su nombre (¿corrupto?)"
            )));
        }
        mapa.insert(id, datos);
    }
    if mapa.is_empty() {
        return Err(Error::Spec(format!(
            "{}: no hallé ningún <hash>.obj (¿es un bundle de `publicar`?)",
            dir.display()
        )));
    }
    Ok(mapa)
}

/// Lee `anuncio.bin` (168 B fijos): `canal|raiz|autor|timestamp_le|firma`. Es el
/// layout crudo que escribe `wawa_publicar`, leído sin postcard.
#[allow(clippy::type_complexity)]
fn cargar_anuncio_bundle(dir: &Path) -> CliResult<([u8; 32], [u8; 32], [u8; 32], u64, [u8; 64])> {
    let bytes = fs::read(dir.join("anuncio.bin"))?;
    if bytes.len() != 168 {
        return Err(Error::Spec(format!(
            "anuncio.bin: esperaba 168 B, hallé {}",
            bytes.len()
        )));
    }
    let mut canal = [0u8; 32];
    let mut raiz = [0u8; 32];
    let mut autor = [0u8; 32];
    let mut ts = [0u8; 8];
    let mut firma = [0u8; 64];
    canal.copy_from_slice(&bytes[0..32]);
    raiz.copy_from_slice(&bytes[32..64]);
    autor.copy_from_slice(&bytes[64..96]);
    ts.copy_from_slice(&bytes[96..104]);
    firma.copy_from_slice(&bytes[104..168]);
    Ok((canal, raiz, autor, u64::from_le_bytes(ts), firma))
}

// =============================================================================
//  Fase 66: importar directorio / exportar árbol
// =============================================================================

/// Tamaño de trozo para archivos grandes. 256 KiB << MAX_OBJETO (1 MiB), así
/// cada trozo es un objeto del grafo holgado y el índice (N·32 B) cabe de sobra.
const TAMANO_TROZO: usize = 256 * 1024;

/// `agora-cli wawa importar` — directorio real -> grafo de objetos.
pub fn wawa_importar(dir: &Path, salida: &Path) -> CliResult<()> {
    if !dir.is_dir() {
        return Err(Error::Spec(format!("«{}» no es un directorio", dir.display())));
    }
    fs::create_dir_all(salida)?;
    let raiz = importar_dir(dir, salida)?;
    fs::write(salida.join("raiz.txt"), format!("{}\n", hex_de(&raiz)))?;

    // Contar objetos únicos = archivos `.obj` en el bundle.
    let n_obj = fs::read_dir(salida)?
        .filter_map(|e| e.ok())
        .filter(|e| {
            e.file_name()
                .to_str()
                .map(|n| n.ends_with(".obj"))
                .unwrap_or(false)
        })
        .count();

    println!("importado: {} -> {}", dir.display(), salida.display());
    println!("  objetos : {n_obj}");
    println!("  raiz    : {}", hex_de(&raiz));
    println!();
    println!("Exportar de vuelta (round-trip):");
    println!("  agora-cli wawa exportar --bundle {} --destino <DIR>", salida.display());
    Ok(())
}

/// Importa un directorio recursivamente, de abajo hacia arriba: cada archivo
/// se emite como blob, cada subdirectorio como árbol. Devuelve el hash del
/// árbol de ESTE directorio.
fn importar_dir(dir: &Path, salida: &Path) -> CliResult<format::Hash> {
    use std::os::unix::fs::PermissionsExt;

    let mut entradas: Vec<format::EntradaArbol> = Vec::new();
    for ent in fs::read_dir(dir)? {
        let ent = ent?;
        let ruta = ent.path();
        let nombre = ent.file_name().to_string_lossy().into_owned();
        let ft = ent.file_type()?; // no sigue symlinks: los detecta como tales
        if ft.is_symlink() {
            // El destino del enlace se guarda como blob de texto.
            let destino = fs::read_link(&ruta)?;
            let bytes = destino.to_string_lossy().into_owned().into_bytes();
            let hash = emitir_objeto(&format::objeto_blob(bytes), salida)?;
            entradas.push(format::EntradaArbol {
                nombre,
                modo: format::ModoEntrada::Symlink,
                hash,
            });
        } else if ft.is_dir() {
            let hash = importar_dir(&ruta, salida)?;
            entradas.push(format::EntradaArbol {
                nombre,
                modo: format::ModoEntrada::Directorio,
                hash,
            });
        } else if ft.is_file() {
            let bytes = fs::read(&ruta)?;
            let hash = importar_archivo(bytes, salida)?;
            // Bit de ejecución (cualquiera de los tres x de Unix).
            let ejecutable = fs::metadata(&ruta)?.permissions().mode() & 0o111 != 0;
            let modo = if ejecutable {
                format::ModoEntrada::Ejecutable
            } else {
                format::ModoEntrada::Archivo
            };
            entradas.push(format::EntradaArbol { nombre, modo, hash });
        }
        // Otros tipos (FIFOs, sockets, devices) se ignoran — no son código.
    }
    let objeto = format::objeto_arbol(entradas).map_err(Error::Canal)?;
    emitir_objeto(&objeto, salida)
}

/// Importa el contenido de un archivo: blob plano si cabe en un trozo, o índice
/// de trozos si es grande (blob-chunking en grafo). Devuelve el hash con que el
/// árbol lo referencia.
fn importar_archivo(bytes: Vec<u8>, salida: &Path) -> CliResult<format::Hash> {
    if bytes.len() <= TAMANO_TROZO {
        return emitir_objeto(&format::objeto_blob(bytes), salida);
    }
    // Grande: partir en trozos, emitir cada uno como blob, y un índice que los
    // encadena. El lector concatena los `datos` de los hijos del índice.
    let mut trozos: Vec<format::Hash> = Vec::new();
    for trozo in bytes.chunks(TAMANO_TROZO) {
        trozos.push(emitir_objeto(&format::objeto_blob(trozo.to_vec()), salida)?);
    }
    emitir_objeto(&format::objeto_blob_indice(trozos), salida)
}

/// Serializa un objeto, lo escribe como `<hash>.obj` en el bundle y devuelve
/// su hash. Idempotente: dos objetos idénticos sobreescriben el mismo archivo.
fn emitir_objeto(objeto: &format::Objeto, salida: &Path) -> CliResult<format::Hash> {
    let payload = objeto.serializar().map_err(Error::Canal)?;
    let hash = format::hash(&payload);
    fs::write(salida.join(format!("{}.obj", hex_de(&hash))), &payload)?;
    Ok(hash)
}

// =============================================================================
//  importar-imagen (foreign-fs)
// =============================================================================

/// `foreign_fs::Emisor` que escribe cada objeto como `<hash>.obj` en el bundle
/// —el mismo formato que produce `emitir_objeto`/`importar`, así que la salida
/// de `importar-imagen` es servible por `servir_release` igual que la de
/// `importar`. Captura el primer error de I/O para reportarlo con detalle.
struct EmisorBundle<'a> {
    salida: &'a Path,
    error_io: Option<std::io::Error>,
}

impl<'a> EmisorBundle<'a> {
    fn nuevo(salida: &'a Path) -> Self {
        Self { salida, error_io: None }
    }
}

impl foreign_fs::Emisor for EmisorBundle<'_> {
    fn emitir(&mut self, objeto: &format::Objeto) -> Result<format::Hash, foreign_fs::FsError> {
        let payload = objeto.serializar().map_err(foreign_fs::FsError::Format)?;
        let hash = format::hash(&payload);
        if let Err(e) = fs::write(self.salida.join(format!("{}.obj", hex_de(&hash))), &payload) {
            self.error_io.get_or_insert(e);
            return Err(foreign_fs::FsError::EmisionFallida);
        }
        Ok(hash)
    }
}

/// `agora-cli wawa importar-imagen` — absorbe una imagen de dispositivo (sin
/// montar) al grafo, vía `foreign-fs`.
pub fn wawa_importar_imagen(
    imagen: &Path,
    salida: &Path,
    particion: Option<usize>,
) -> CliResult<()> {
    use foreign_fs::particion::{
        absorber_dispositivo, absorber_particion, detectar_fs, tabla_particiones,
        SistemaArchivos,
    };

    let datos = fs::read(imagen)?;
    fs::create_dir_all(salida)?;

    // Enumera y reporta la tabla — orientación para el operador.
    let particiones = tabla_particiones(&datos).map_err(Error::ForeignFs)?;
    println!("imagen: {} ({} bytes)", imagen.display(), datos.len());
    println!("particiones:");
    for p in &particiones {
        let fin = ((p.inicio + p.tam) as usize).min(datos.len());
        let fs_str = match datos.get(p.inicio as usize..fin) {
            Some(s) => match detectar_fs(s) {
                SistemaArchivos::Fat => "FAT",
                SistemaArchivos::Ext => "ext2/3/4",
                SistemaArchivos::Desconocido => "desconocido (se omite)",
            },
            None => "fuera del medio",
        };
        println!(
            "  [{}] {:?}  inicio={} tam={}  fs={}",
            p.indice, p.esquema, p.inicio, p.tam, fs_str
        );
    }

    let mut emisor = EmisorBundle::nuevo(salida);
    let raiz = if let Some(slot) = particion {
        let p = particiones
            .iter()
            .find(|p| p.indice == slot)
            .ok_or_else(|| Error::Spec(format!("no hay partición en el slot {slot}")))?;
        absorber_particion(&datos, p, &mut emisor)
    } else {
        // Por defecto: una sola partición reconocida → su FS directo (sin
        // envoltorio); varias → árbol de dispositivo `particionN/`.
        let reconocidas: Vec<_> = particiones
            .iter()
            .filter(|p| {
                let fin = ((p.inicio + p.tam) as usize).min(datos.len());
                datos
                    .get(p.inicio as usize..fin)
                    .map(|s| detectar_fs(s) != SistemaArchivos::Desconocido)
                    .unwrap_or(false)
            })
            .collect();
        match reconocidas.len() {
            0 => {
                return Err(Error::Spec(
                    "ninguna partición con un FS reconocido (FAT/ext)".into(),
                ))
            }
            1 => absorber_particion(&datos, reconocidas[0], &mut emisor),
            _ => absorber_dispositivo(&datos, &mut emisor),
        }
    };

    // Propaga un error de I/O del emisor con su detalle real.
    let raiz = match raiz {
        Ok(h) => h,
        Err(foreign_fs::FsError::EmisionFallida) => {
            return Err(emisor
                .error_io
                .map(Error::Io)
                .unwrap_or(Error::ForeignFs(foreign_fs::FsError::EmisionFallida)))
        }
        Err(e) => return Err(Error::ForeignFs(e)),
    };

    fs::write(salida.join("raiz.txt"), format!("{}\n", hex_de(&raiz)))?;
    let n_obj = fs::read_dir(salida)?
        .filter_map(|e| e.ok())
        .filter(|e| e.file_name().to_str().map(|n| n.ends_with(".obj")).unwrap_or(false))
        .count();

    println!();
    println!("absorbido: {} -> {}", imagen.display(), salida.display());
    println!("  objetos : {n_obj}");
    println!("  raiz    : {}", hex_de(&raiz));
    println!();
    println!("Servir a una wawa en la misma red L2:");
    println!(
        "  sudo -E cargo run -p wawa-explorer-aoe --example servir_release -- <iface> {}",
        salida.display()
    );
    Ok(())
}

// =============================================================================
//  exportar árbol (Fase 66, inverso de importar)
// =============================================================================

/// `agora-cli wawa exportar` — grafo de objetos -> directorio real.
pub fn wawa_exportar(bundle: &Path, raiz_hex: Option<&str>, destino: &Path) -> CliResult<()> {
    // La raíz viene del flag o de `raiz.txt` del bundle.
    let raiz_hex = match raiz_hex {
        Some(h) => h.to_string(),
        None => fs::read_to_string(bundle.join("raiz.txt"))
            .map_err(|e| Error::Spec(format!("sin --raiz y no pude leer raiz.txt: {e}")))?
            .trim()
            .to_string(),
    };
    let raiz = parse_hash(&raiz_hex)?;
    fs::create_dir_all(destino)?;
    let n = exportar_arbol(bundle, &raiz, destino)?;
    println!("exportado: raiz {}… -> {}", &hex_de(&raiz)[..16], destino.display());
    println!("  archivos: {n}");
    Ok(())
}

/// Reconstruye el directorio cuyo árbol es `hash` dentro de `destino`.
/// Devuelve cuántos ARCHIVOS escribió (recursivo).
fn exportar_arbol(bundle: &Path, hash: &format::Hash, destino: &Path) -> CliResult<usize> {
    use std::os::unix::fs::PermissionsExt;

    let objeto = leer_objeto(bundle, hash)?;
    let arbol = format::Arbol::deserializar(&objeto.datos).map_err(Error::Canal)?;
    let mut archivos = 0;
    for entrada in &arbol.entradas {
        let dest = destino.join(&entrada.nombre);
        match entrada.modo {
            format::ModoEntrada::Archivo | format::ModoEntrada::Ejecutable => {
                let contenido = reconstruir_archivo(bundle, &entrada.hash)?;
                fs::write(&dest, &contenido)?;
                if entrada.modo == format::ModoEntrada::Ejecutable {
                    fs::set_permissions(&dest, std::fs::Permissions::from_mode(0o755))?;
                }
                archivos += 1;
            }
            format::ModoEntrada::Symlink => {
                let blob = leer_objeto(bundle, &entrada.hash)?;
                let objetivo = String::from_utf8_lossy(&blob.datos).into_owned();
                // Recrear el enlace simbólico; si ya existe, reemplazarlo.
                let _ = fs::remove_file(&dest);
                std::os::unix::fs::symlink(&objetivo, &dest)?;
                archivos += 1;
            }
            format::ModoEntrada::Directorio => {
                fs::create_dir_all(&dest)?;
                archivos += exportar_arbol(bundle, &entrada.hash, &dest)?;
            }
        }
    }
    Ok(archivos)
}

/// Reconstruye el CONTENIDO de un archivo: si su objeto es un blob plano
/// (`hijos` vacío) son sus `datos`; si es un índice (`hijos` no vacío) es la
/// concatenación de los `datos` de cada trozo, en orden. Verifica el hash de
/// cada objeto leído.
fn reconstruir_archivo(bundle: &Path, hash: &format::Hash) -> CliResult<Vec<u8>> {
    let objeto = leer_objeto(bundle, hash)?;
    if objeto.hijos.is_empty() {
        return Ok(objeto.datos);
    }
    let mut contenido = Vec::new();
    for trozo_hash in &objeto.hijos {
        let trozo = leer_objeto(bundle, trozo_hash)?;
        contenido.extend_from_slice(&trozo.datos);
    }
    Ok(contenido)
}

/// Lee un objeto del bundle por su hash y VERIFICA que su contenido rehashea
/// a ese hash — integridad de punta a punta del grafo direccionado por
/// contenido.
fn leer_objeto(bundle: &Path, hash: &format::Hash) -> CliResult<format::Objeto> {
    let ruta = bundle.join(format!("{}.obj", hex_de(hash)));
    let bytes = fs::read(&ruta)
        .map_err(|e| Error::Spec(format!("no pude leer {}: {e}", ruta.display())))?;
    if format::hash(&bytes) != *hash {
        return Err(Error::Spec(format!(
            "objeto {} corrupto: su contenido no rehashea a su nombre",
            &hex_de(hash)[..16]
        )));
    }
    format::Objeto::deserializar(&bytes).map_err(Error::Canal)
}
