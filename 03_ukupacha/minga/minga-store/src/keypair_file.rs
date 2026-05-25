//! Persistencia en disco de keypairs cifrados.
//!
//! El cifrado en sí (AES-GCM + Argon2id) vive en `minga-core`, que es
//! pure logic. Aquí solo se monta la parte de IO: leer/escribir
//! bytes a un archivo.
//!
//! Layout del archivo: el blob crudo que produce
//! `Keypair::encrypt(passphrase)`. 85 bytes total.

use std::fs;
use std::io;
use std::path::Path;

use minga_core::{Keypair, KeypairCryptoError};

#[derive(Debug, thiserror::Error)]
pub enum KeypairFileError {
    #[error("io: {0}")]
    Io(#[from] io::Error),

    #[error("crypto: {0}")]
    Crypto(#[from] KeypairCryptoError),
}

/// Guarda un keypair cifrado con la passphrase en `path`. Si el
/// archivo ya existe, lo sobrescribe.
pub fn save<P: AsRef<Path>>(
    keypair: &Keypair,
    path: P,
    passphrase: &str,
) -> Result<(), KeypairFileError> {
    let blob = keypair.encrypt(passphrase)?;
    fs::write(path, blob)?;
    Ok(())
}

/// Carga un keypair desde un archivo cifrado.
pub fn load<P: AsRef<Path>>(path: P, passphrase: &str) -> Result<Keypair, KeypairFileError> {
    let blob = fs::read(path)?;
    Ok(Keypair::decrypt(&blob, passphrase)?)
}
