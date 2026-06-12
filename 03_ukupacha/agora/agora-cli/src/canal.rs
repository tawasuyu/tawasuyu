//! Handlers para el subcomando `agora-cli canal`.
//!
//! Operaciones: nuevo, extender, verificar, mostrar.

use std::fs;
use std::path::Path;

use crate::sesion::{ahora_unix, hex_de, parse_hash, CliResult, Error, Sesion};

// =============================================================================
//  nuevo
// =============================================================================

pub fn canal_nuevo(nombre: &str, autor: &str, salida: &Path) -> CliResult<()> {
    use format::{Canal, NOMBRE_CANAL_LIMITE, VERSION_CANAL};
    let s = Sesion::abrir()?;
    let autor_id = s.resolver_id(autor)?;
    let ident = s
        .graph
        .identity(autor_id)
        .ok_or(Error::IdentidadDesconocida(autor_id))?;
    if !s.es_mia(autor_id) {
        return Err(Error::IdentidadNoPropia(autor_id));
    }
    if nombre.is_empty() || nombre.len() > NOMBRE_CANAL_LIMITE {
        return Err(Error::Canal("nombre vacío o más largo que NOMBRE_CANAL_LIMITE"));
    }
    let canal = Canal {
        version: VERSION_CANAL,
        nombre: nombre.to_string(),
        autor: ident.public_key,
        raices: Vec::new(),
    };
    let bytes = canal.serializar().map_err(Error::Canal)?;
    fs::write(salida, &bytes)?;
    println!(
        "canal nuevo creado: nombre=\"{}\" autor={} → {} ({} bytes)",
        nombre,
        hex_de(autor_id.as_bytes()),
        salida.display(),
        bytes.len()
    );
    Ok(())
}

// =============================================================================
//  extender
// =============================================================================

pub fn canal_extender(archivo: &Path, raiz_hex: &str) -> CliResult<()> {
    use format::Canal;
    let s = Sesion::abrir()?;
    let raiz_hash = parse_hash(raiz_hex)?;
    let bytes = fs::read(archivo)?;
    let mut canal = Canal::deserializar(&bytes).map_err(Error::Canal)?;

    let autor_id = agora_core::IdentityId::from_public_key(&canal.autor);
    if !s.es_mia(autor_id) {
        return Err(Error::IdentidadNoPropia(autor_id));
    }
    let kp = s.cargar_keypair(autor_id)?;

    let ts = ahora_unix();
    // Forzamos timestamp estrictamente posterior al último — verificar_canal
    // lo exigirá al releer.
    let ts = match canal.raices.last() {
        Some(prev) if ts <= prev.timestamp => prev.timestamp + 1,
        _ => ts,
    };
    let nueva = agora_channel::firmar_raiz(&kp, &canal.nombre, &raiz_hash, ts);
    canal.raices.push(nueva.clone());

    let bytes = canal.serializar().map_err(Error::Canal)?;
    fs::write(archivo, &bytes)?;
    println!(
        "canal \"{}\" extendido: raíz={} ts={} → ahora {} raíces ({} bytes)",
        canal.nombre,
        hex_de(&raiz_hash),
        ts,
        canal.raices.len(),
        bytes.len()
    );
    Ok(())
}

// =============================================================================
//  verificar
// =============================================================================

pub fn canal_verificar(archivo: &Path) -> CliResult<()> {
    use format::Canal;
    let bytes = fs::read(archivo)?;
    let canal = Canal::deserializar(&bytes).map_err(Error::Canal)?;
    agora_channel::verificar_canal(&canal).map_err(Error::AgoraChannel)?;
    println!(
        "canal \"{}\" válido: {} raíces firmadas por {} (timestamps estrictamente monotónicos)",
        canal.nombre,
        canal.raices.len(),
        hex_de(&canal.autor)
    );
    Ok(())
}

// =============================================================================
//  mostrar
// =============================================================================

pub fn canal_mostrar(archivo: &Path) -> CliResult<()> {
    use format::Canal;
    let s = Sesion::abrir()?;
    let bytes = fs::read(archivo)?;
    let canal = Canal::deserializar(&bytes).map_err(Error::Canal)?;
    let autor_id = agora_core::IdentityId::from_public_key(&canal.autor);
    let autor_name = s
        .graph
        .identity(autor_id)
        .map(|i| i.display_name.as_str())
        .unwrap_or("(desconocido en el grafo local)");
    println!("canal: {}", canal.nombre);
    println!("autor: {} ({})", hex_de(&canal.autor), autor_name);
    println!("version: {}", canal.version);
    println!("raíces: {}", canal.raices.len());
    for (i, raiz) in canal.raices.iter().enumerate() {
        let valida = agora_channel::verificar_raiz(&canal.autor, &canal.nombre, raiz).is_ok();
        let mark = if valida { "✔" } else { "✘" };
        println!(
            "  #{i:<3} {mark}  ts={ts}  raíz={raiz}",
            i = i,
            ts = raiz.timestamp,
            raiz = hex_de(&raiz.raiz_manifiesto)
        );
    }
    Ok(())
}
