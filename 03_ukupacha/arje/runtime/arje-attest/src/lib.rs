//! arje-attest — atestación de integridad al arranque (A1/A2 de
//! `PLAN-ATESTACION-Y-HAMMER.md`).
//!
//! El seed lleva, por binario crítico, una [`ConcesionCapacidad`] firmada
//! sobre `(blake3(binario), permisos)` bajo la **rootkey** del seed — el mismo
//! tipo que agora y el kernel wawa ya verifican (WAWA §14.1.3). No hay
//! criptografía nueva: este crate sólo **cablea** los primitivos de agora a la
//! atestación de los binarios vivos de `/bin` antes de levantar el entorno.
//!
//! - [`firmar_binarios`]: el lado *autor* (lo usa `arje-packager` al empaquetar
//!   el seed con la rootkey).
//! - [`verificar_binario`]: el lado *gate* (lo usa `arje-zero` al boot, antes de
//!   incarnar el target gráfico).
//!
//! La cadena de confianza tiene tres eslabones, en orden de fuerza decreciente:
//! 1. **firma** — la concesión está firmada sobre su `(bytecode, permisos)`;
//! 2. **autor** — esa firma es de la rootkey confiable pinada en el seed
//!    (`Card::attest_rootkey`), no de cualquiera;
//! 3. **hash** — el binario que de verdad corre tiene el BLAKE3 atestado.
//!
//! El eslabón 2 es lo que distingue "alguien firmó esto" de "lo firmó la
//! rootkey de ESTE seed". Sin un ancla soberana fuera de la propia Card (un
//! pubkey compilado o un TPM), un atacante que reescribe el seed entero podría
//! reemplazar también `attest_rootkey` — por eso la política por defecto es
//! observar ([`AttestPolicy::Warn`] en `card-core`) y el endurecimiento a
//! `Halt` + ancla soberana es decisión del operador (igual que el flip a
//! estricto de agora §14.1.3).

use agora_channel::{firmar_capacidad, verificar_capacidad};
use agora_core::Keypair;
use format::{hash as blake3, AgoraId, ConcesionCapacidad, Permisos};

/// BLAKE3 de unos bytes — el mismo hash que `format`/agora/wawa usan como
/// identidad de contenido. Reexportado para que los callers registren el hash
/// vivo (p. ej. en el audit log) sin depender de `format` directamente.
pub fn hash_de(bytes: &[u8]) -> [u8; 32] {
    blake3(bytes)
}

/// Firma una [`ConcesionCapacidad`] por cada binario crítico, sobre
/// `(blake3(bytes), permisos)`, bajo la rootkey derivada de `rootkey_seed`.
///
/// Devuelve `(pubkey_rootkey, concesiones)`: la pubkey va a
/// `Card::attest_rootkey` (para que el gate la exija como `autor`) y las
/// concesiones a `Card::attest`. `items` son `(bytes del binario vivo al
/// empaquetar, permisos que se le autorizan)`, en el orden que el caller quiera.
pub fn firmar_binarios(
    rootkey_seed: [u8; 32],
    items: &[(Vec<u8>, Permisos)],
) -> (AgoraId, Vec<ConcesionCapacidad>) {
    let kp = Keypair::from_seed(rootkey_seed);
    let pubkey = kp.public_key();
    let concesiones = items
        .iter()
        .map(|(bytes, permisos)| firmar_capacidad(&kp, &blake3(bytes), *permisos))
        .collect();
    (pubkey, concesiones)
}

/// Veredicto de atestar un binario vivo contra su concesión firmada.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Veredicto {
    /// Firma válida, autor confiable (si se pidió pin) y el binario vivo casa
    /// con el hash atestado.
    Ok,
    /// La firma de la concesión no verifica bajo su propio `autor`.
    FirmaInvalida,
    /// La firma es válida pero el autor no es la rootkey confiable pinada.
    AutorNoConfiable,
    /// El binario vivo tiene un BLAKE3 distinto del atestado
    /// (tampering o corrupción del binario).
    HashNoCasa,
    /// Ninguna concesión del seed cubre este binario vivo: su hash no aparece
    /// en `attest`. Es el veredicto del *gate* (que busca por hash) cuando el
    /// binario que corre no fue atestado en absoluto (tampering, o binario
    /// ausente del manifiesto).
    NoAtestada,
}

impl Veredicto {
    /// `true` sólo para [`Veredicto::Ok`].
    pub fn es_ok(self) -> bool {
        matches!(self, Veredicto::Ok)
    }

    /// Motivo legible (para el audit log / la shell).
    pub fn motivo(self) -> &'static str {
        match self {
            Veredicto::Ok => "ok",
            Veredicto::FirmaInvalida => "firma inválida",
            Veredicto::AutorNoConfiable => "autor no confiable",
            Veredicto::HashNoCasa => "hash no casa",
            Veredicto::NoAtestada => "binario no atestado",
        }
    }
}

/// Atesta un binario vivo por su contenido contra el manifiesto `attest` del
/// seed: busca la concesión cuyo `bytecode` iguala el BLAKE3 de `bytes_vivos`
/// y, si la encuentra, valida su firma y (si hay pin) su autor.
///
/// Es el punto de entrada del *gate* en `arje-zero`: a diferencia de
/// [`verificar_binario`] no necesita saber qué concesión corresponde a qué
/// path — empareja por hash. Si ningún concesión cubre el hash vivo devuelve
/// [`Veredicto::NoAtestada`] (el binario que corre no fue atestado: tampering
/// o simplemente ausente del manifiesto).
pub fn atestar_bytes(
    attest: &[ConcesionCapacidad],
    bytes_vivos: &[u8],
    confiable: Option<AgoraId>,
) -> Veredicto {
    let h = blake3(bytes_vivos);
    match attest.iter().find(|c| c.bytecode == h) {
        Some(c) => verificar_binario(c, bytes_vivos, confiable),
        None => Veredicto::NoAtestada,
    }
}

/// Verifica un binario vivo (`bytes_vivos`) contra su [`ConcesionCapacidad`]:
///
/// 1. la firma cubre `(bytecode, permisos)` bajo `c.autor` ([`verificar_capacidad`]);
/// 2. si `confiable` es `Some(rootkey)`, `c.autor` debe ser exactamente esa
///    rootkey (si no, [`Veredicto::AutorNoConfiable`]);
/// 3. el BLAKE3 del binario vivo debe igualar `c.bytecode`.
///
/// El orden importa: una firma falsa se descarta antes de mirar autor/hash; un
/// binario re-firmado por un atacante con su propia llave cae en
/// `AutorNoConfiable` (no en `HashNoCasa`), que es el motivo más preciso.
pub fn verificar_binario(
    c: &ConcesionCapacidad,
    bytes_vivos: &[u8],
    confiable: Option<AgoraId>,
) -> Veredicto {
    if verificar_capacidad(c).is_err() {
        return Veredicto::FirmaInvalida;
    }
    if let Some(rootkey) = confiable {
        if c.autor != rootkey {
            return Veredicto::AutorNoConfiable;
        }
    }
    if blake3(bytes_vivos) != c.bytecode {
        return Veredicto::HashNoCasa;
    }
    Veredicto::Ok
}

#[cfg(test)]
mod tests {
    use super::*;

    const SEED: [u8; 32] = [7u8; 32];
    const OTRA_SEED: [u8; 32] = [9u8; 32];
    const PERM: Permisos = 0b101;

    #[test]
    fn firma_y_verifica_roundtrip() {
        let bin = b"binario critico /sbin/arje-zero".to_vec();
        let (rootkey, cs) = firmar_binarios(SEED, &[(bin.clone(), PERM)]);
        assert_eq!(cs.len(), 1);
        // Con el binario intacto y la rootkey correcta pinada → Ok.
        assert_eq!(verificar_binario(&cs[0], &bin, Some(rootkey)), Veredicto::Ok);
        // La concesión guarda el hash y los permisos firmados.
        assert_eq!(cs[0].bytecode, blake3(&bin));
        assert_eq!(cs[0].permisos, PERM);
        assert_eq!(cs[0].autor, rootkey);
    }

    #[test]
    fn binario_alterado_no_casa() {
        let bin = b"original".to_vec();
        let (rootkey, cs) = firmar_binarios(SEED, &[(bin, PERM)]);
        let alterado = b"original + payload malicioso".to_vec();
        assert_eq!(
            verificar_binario(&cs[0], &alterado, Some(rootkey)),
            Veredicto::HashNoCasa
        );
    }

    #[test]
    fn autor_distinto_de_la_rootkey_pinada_se_rechaza() {
        // Un atacante re-firma el binario alterado con SU propia llave: la
        // firma es internamente válida y el hash casa, pero el autor no es la
        // rootkey confiable del seed → AutorNoConfiable.
        let bin = b"binario re-firmado por atacante".to_vec();
        let (autor_atacante, cs) = firmar_binarios(OTRA_SEED, &[(bin.clone(), PERM)]);
        let rootkey_confiable = Keypair::from_seed(SEED).public_key();
        assert_ne!(autor_atacante, rootkey_confiable);
        assert_eq!(
            verificar_binario(&cs[0], &bin, Some(rootkey_confiable)),
            Veredicto::AutorNoConfiable
        );
        // Sin pin (None) esa misma concesión pasa (firma+hash ok) — por eso el
        // pin de la rootkey es lo que cierra el ataque de re-firma.
        assert_eq!(verificar_binario(&cs[0], &bin, None), Veredicto::Ok);
    }

    #[test]
    fn firma_corrupta_se_detecta() {
        let bin = b"x".to_vec();
        let (rootkey, mut cs) = firmar_binarios(SEED, &[(bin.clone(), PERM)]);
        cs[0].firma[0] ^= 0xFF; // corrompé un byte de la firma
        assert_eq!(
            verificar_binario(&cs[0], &bin, Some(rootkey)),
            Veredicto::FirmaInvalida
        );
    }

    #[test]
    fn atestar_bytes_empareja_por_hash() {
        let a = b"/sbin/arje-zero".to_vec();
        let b = b"/usr/bin/mirada".to_vec();
        let (rootkey, cs) = firmar_binarios(SEED, &[(a.clone(), PERM), (b.clone(), PERM)]);
        // Binario atestado e intacto → Ok (lo encuentra por hash).
        assert_eq!(atestar_bytes(&cs, &a, Some(rootkey)), Veredicto::Ok);
        assert_eq!(atestar_bytes(&cs, &b, Some(rootkey)), Veredicto::Ok);
        // Binario alterado → su hash no está en el manifiesto → NoAtestada.
        let a_malo = b"/sbin/arje-zero + backdoor".to_vec();
        assert_eq!(atestar_bytes(&cs, &a_malo, Some(rootkey)), Veredicto::NoAtestada);
        // Manifiesto vacío → nada atesta.
        assert_eq!(atestar_bytes(&[], &a, Some(rootkey)), Veredicto::NoAtestada);
    }

    #[test]
    fn cada_binario_queda_atado_a_su_propio_hash() {
        let a = b"binario A".to_vec();
        let b = b"binario B".to_vec();
        let (rootkey, cs) = firmar_binarios(SEED, &[(a.clone(), PERM), (b.clone(), PERM)]);
        // La concesión de A no valida contra los bytes de B (swap → HashNoCasa).
        assert_eq!(verificar_binario(&cs[0], &a, Some(rootkey)), Veredicto::Ok);
        assert_eq!(
            verificar_binario(&cs[0], &b, Some(rootkey)),
            Veredicto::HashNoCasa
        );
        assert_eq!(verificar_binario(&cs[1], &b, Some(rootkey)), Veredicto::Ok);
    }
}
