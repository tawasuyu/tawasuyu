//! Handlers para el subcomando `agora-cli identidad`.
//!
//! Operaciones: nueva, listar, exportar, rename, remove, rotar, revocar.

use agora_core::{IdentityKind, KeyRotation, Keypair, RevReason, Revocation};
use rand::RngCore;

use crate::sesion::{ahora_unix, hex_de, kind_label, leer_seed_de_stdin, CliResult, Error, Sesion};

// =============================================================================
//  desbloquear — cachea la seed en el session keyring (para pacha)
// =============================================================================

/// Desbloquea la identidad (la indicada por `id`, o la única del keystore) y
/// guarda su seed en el session keyring bajo `pacha:id:default`, que es donde
/// `pacha`/`pacha-manager` la buscan para derivar la clave del store de dotfiles.
pub fn desbloquear(id_input: Option<String>) -> CliResult<()> {
    use pacha_llavero::Llavero;

    let s = Sesion::abrir()?;
    let id = match id_input {
        Some(x) => s.resolver_id(&x)?,
        None => {
            let mias = s.keystore.list().map_err(Error::Keystore)?;
            match mias.len() {
                1 => mias[0],
                0 => {
                    return Err(Error::Llavero(
                        "no hay ninguna identidad en el keystore; creá una con \
                         `agora-cli identidad nueva <nombre>`"
                            .into(),
                    ))
                }
                _ => {
                    return Err(Error::Llavero(
                        "hay varias identidades en el keystore; especificá --id <prefijo>".into(),
                    ))
                }
            }
        }
    };
    let seed = s.keystore.load(id, &s.passphrase).map_err(Error::Keystore)?;
    pacha_llavero::LlaveroKernel::new()
        .guardar("id:default", &seed)
        .map_err(|e| Error::Llavero(e.to_string()))?;
    println!(
        "✓ identidad {} desbloqueada y cacheada en la sesión \
         (pacha la usa para cifrar/descifrar dotfiles)",
        hex_de(id.as_bytes())
    );
    Ok(())
}

// =============================================================================
//  nueva
// =============================================================================

pub fn identidad_nueva(name: String, kind: IdentityKind, seed_stdin: bool) -> CliResult<()> {
    let mut s = Sesion::abrir()?;
    let seed = if seed_stdin {
        leer_seed_de_stdin()?
    } else {
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        seed
    };
    let kp = Keypair::from_seed(seed);
    let id = kp.identity_id();
    s.keystore.save(id, &seed, &s.passphrase).map_err(Error::Keystore)?;
    s.graph.register(kp.identity(kind, &name));
    s.guardar()?;
    println!("nueva identidad creada");
    println!("  id     {id_full}", id_full = hex_de(id.as_bytes()));
    println!("  kind   {}", kind_label(kind));
    println!("  name   {name}");
    println!("  pubkey {}", hex_de(&kp.public_key()));
    Ok(())
}

// =============================================================================
//  listar
// =============================================================================

pub fn identidad_listar() -> CliResult<()> {
    let s = Sesion::abrir()?;
    let mut idents: Vec<&agora_core::Identity> = s.graph.identities().collect();
    idents.sort_by(|a, b| a.id().as_bytes().cmp(b.id().as_bytes()));
    if idents.is_empty() {
        println!("(grafo vacío — corré `agora-cli identidad nueva`)");
        return Ok(());
    }
    println!("{:>2}  {:<64}  {:<11}  {}", "", "id (hex)", "kind", "name");
    for ident in idents {
        let mark = if s.es_mia(ident.id()) { "★" } else { " " };
        println!(
            "{mark:>2}  {id}  {kind:<11}  {name}",
            id = hex_de(ident.id().as_bytes()),
            kind = kind_label(ident.kind),
            name = ident.display_name,
        );
    }
    Ok(())
}

// =============================================================================
//  exportar
// =============================================================================

pub fn identidad_exportar(id: &str) -> CliResult<()> {
    let s = Sesion::abrir()?;
    let id = s.resolver_id(id)?;
    let ident = s.graph.identity(id).ok_or(Error::IdentidadDesconocida(id))?;
    println!("id     {}", hex_de(id.as_bytes()));
    println!("kind   {}", kind_label(ident.kind));
    println!("name   {}", ident.display_name);
    println!("pubkey {}", hex_de(&ident.public_key));
    Ok(())
}

// =============================================================================
//  rename
// =============================================================================

pub fn identidad_rename(id: &str, nombre: &str) -> CliResult<()> {
    if nombre.is_empty() {
        return Err(Error::Canal("nombre vacío — pasá --nombre con un valor"));
    }
    let mut s = Sesion::abrir()?;
    let id = s.resolver_id(id)?;
    let prev = s
        .graph
        .identity(id)
        .ok_or(Error::IdentidadDesconocida(id))?
        .display_name
        .clone();
    if !s.graph.set_display_name(id, nombre.to_string()) {
        // No debería pasar — `identity()` ya devolvió Some — pero
        // dejamos el error explícito por si el contrato del graph cambia.
        return Err(Error::IdentidadDesconocida(id));
    }
    s.guardar()?;
    println!(
        "identidad {} renombrada: \"{}\" → \"{}\"",
        hex_de(id.as_bytes()),
        prev,
        nombre
    );
    Ok(())
}

// =============================================================================
//  remove
// =============================================================================

pub fn identidad_remove(id: &str, force: bool, purgar_keystore: bool) -> CliResult<()> {
    let mut s = Sesion::abrir()?;
    let id = s.resolver_id(id)?;
    if s.graph.identity(id).is_none() {
        return Err(Error::IdentidadDesconocida(id));
    }
    if !s.es_mia(id) && !force {
        return Err(Error::Canal(
            "identidad ajena (sin seed local) — pasá --force si querés \
             borrarla igual del grafo local",
        ));
    }
    let stats = s.graph.remove_identity(id);
    if purgar_keystore && s.keystore.exists(id) {
        s.keystore.remove(id).map_err(Error::Keystore)?;
    }
    s.guardar()?;
    println!(
        "identidad {} borrada del grafo · {} atestación{} relacionada{} purgada{}{}",
        hex_de(id.as_bytes()),
        stats.attestations,
        if stats.attestations == 1 { "" } else { "es" },
        if stats.attestations == 1 { "" } else { "s" },
        if stats.attestations == 1 { "" } else { "s" },
        if purgar_keystore {
            " · seed borrada del keystore"
        } else if s.es_mia(id) {
            " · seed PRESERVADA en el keystore (re-registrable con --seed-stdin)"
        } else {
            ""
        }
    );
    Ok(())
}

// =============================================================================
//  rotar
// =============================================================================

pub fn identidad_rotar(id: &str, nombre: Option<String>, seed_stdin: bool) -> CliResult<()> {
    let mut s = Sesion::abrir()?;
    let vieja_id = s.resolver_id(id)?;
    // La rotación se auto-autoriza con la clave vieja viva: tiene que ser nuestra.
    let vieja_kp = s.cargar_keypair(vieja_id)?;
    let madre = s
        .graph
        .identity(vieja_id)
        .ok_or(Error::IdentidadDesconocida(vieja_id))?;
    let kind = madre.kind;
    let nombre = nombre.unwrap_or_else(|| madre.display_name.clone());

    // Forjar (o sembrar) la clave sucesora.
    let seed = if seed_stdin {
        leer_seed_de_stdin()?
    } else {
        let mut seed = [0u8; 32];
        rand::thread_rng().fill_bytes(&mut seed);
        seed
    };
    let nueva_kp = Keypair::from_seed(seed);
    let nueva_id = nueva_kp.identity_id();

    // El record doble-firmado y su ingreso al grafo (re-verifica ambas firmas).
    let rot = KeyRotation::create(&vieja_kp, &nueva_kp, ahora_unix());
    s.graph.add_rotation(rot)?;
    // Registrar la sucesora como identidad de primera clase + su seed.
    s.keystore.save(nueva_id, &seed, &s.passphrase).map_err(Error::Keystore)?;
    s.graph.register(nueva_kp.identity(kind, &nombre));
    s.guardar()?;

    println!("identidad rotada (handoff voluntario, doble-firmado)");
    println!("  vieja  {}", hex_de(vieja_id.as_bytes()));
    println!("  nueva  {}  ★ (seed en el keystore)", hex_de(nueva_id.as_bytes()));
    println!("  name   {nombre}");
    println!(
        "la cadena de sucesión queda viva: `current_key_at` desde la vieja apunta a la nueva."
    );
    Ok(())
}

// =============================================================================
//  revocar (plano social)
// =============================================================================

pub fn identidad_revocar(
    id: &str,
    motivo: RevReason,
    umbral: Option<usize>,
    vence_en_seg: Option<u64>,
) -> CliResult<()> {
    let mut s = Sesion::abrir()?;
    let target_id = s.resolver_id(id)?;
    let target = s
        .graph
        .identity(target_id)
        .ok_or(Error::IdentidadDesconocida(target_id))?;
    let target_key = target.public_key;

    // Autoridad SOCIAL: el set de guardianes que la propia identidad declaró.
    let guardianes = s.graph.guardians_of(target_id);
    if guardianes.is_empty() {
        return Err(Error::Canal(
            "la identidad no declaró guardianes — sin autoridad de revocación \
             social (atestá `predicate=\"guardian\"` con `agora-cli atestar`)",
        ));
    }
    // Sólo podemos firmar con los guardianes cuya seed vive en el keystore local.
    let mut firmantes: Vec<Keypair> = Vec::new();
    for g in &guardianes {
        if s.keystore.exists(*g) {
            firmantes.push(s.cargar_keypair(*g)?);
        }
    }
    if firmantes.is_empty() {
        return Err(Error::Canal(
            "ningún guardián declarado tiene seed en el keystore local — \
             no puedo aportar firmas (la combinación multi-parte queda pendiente)",
        ));
    }
    let min = umbral.unwrap_or(firmantes.len());

    let now = ahora_unix();
    let expires_at = vence_en_seg.map(|seg| now + seg);
    let refs: Vec<&Keypair> = firmantes.iter().collect();
    let rev = Revocation::create(target_key, motivo, now, expires_at, &refs);

    // El grafo es el juez: NO guarda una revocación que sus guardianes no
    // respalden al umbral pedido (anti-DoS de tombstones).
    s.graph
        .add_revocation(rev, min, &guardianes)
        .map_err(Error::MultiSig)?;
    s.guardar()?;

    let motivo_str = match motivo {
        RevReason::Compromised => "compromised (PERMANENTE)",
        RevReason::Retired => "retired",
        RevReason::Superseded => "superseded",
    };
    println!("identidad revocada (plano social, M-of-N de guardianes)");
    println!("  target  {}", hex_de(target_id.as_bytes()));
    println!("  motivo  {motivo_str}");
    println!("  quórum  {min}-of-{} guardianes (firmé con {})", guardianes.len(), firmantes.len());
    match expires_at {
        Some(t) => println!("  vence   en t={t} (suspensión temporal)"),
        None => println!("  vence   nunca (permanente)"),
    }
    println!("la evidencia de esta clave deja de contar en `corroboration_at` desde ahora.");
    Ok(())
}
