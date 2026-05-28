# agora-keystore

> Almacén cifrado de seeds privadas Ed25519 para [agora](../LEEME.md).

[`agora-core::Keypair`](../agora-core/LEEME.md) se construye desde una seed de 32 bytes. Esa seed es lo único que hay que persistir para que una identidad sobreviva entre arranques — pero **nunca** debe viajar en claro: escribirla al disco verbatim permitiría que cualquier proceso que lea el archivo suplantara al dueño. [`agora-store`](../agora-store/LEEME.md) lo declara explícito y deja el keystore como una preocupación separada y deliberada. Este crate es esa preocupación.

## Formato en el cable

Un archivo de keystore es `magic(8) || version(4 LE) || salt(16) || nonce(12) || ciphertext(48) = 88 bytes`. El ciphertext es `seed(32) || tag Poly1305(16)`.

- **Magic**: `b"agorakey"`. Archivos que no empiecen con esto se rechazan.
- **Versión**: `1` hoy. Cambios de esquema forward-incompatibles bumpean el número.
- **Salt**: 16 bytes random por archivo, alimentan Argon2id junto con la passphrase.
- **Nonce**: 12 bytes random, fresco por cifrado — nunca reutilizado para la misma clave.
- **KDF**: Argon2id con los parámetros default de `argon2 = "0.5"` (`m=19456 KiB, t=2, p=1`). Salida: 32 bytes que se usan como clave ChaCha20.
- **AEAD**: ChaCha20-Poly1305. El tag Poly1305 hace que "passphrase incorrecta" y "archivo manipulado" sean la misma falla observable.

## Archivos en disco

Un `Keystore` es un directorio: un archivo por identidad, nombrado `<hex(IdentityId)>.key`. La ubicación por defecto es `~/.local/share/agora/keys/` (resuelta vía `directories`). Los saves son atómicos (tmp + rename).

## API

```rust
use agora_core::Keypair;
use agora_keystore::Keystore;

let ks = Keystore::open_default()?;
let kp = Keypair::from_seed([42; 32]);
ks.save(kp.identity_id(), &[42; 32], "passphrase fuerte")?;

// Más tarde, en otro arranque:
let seed = ks.load(kp.identity_id(), "passphrase fuerte")?;
let mismo_kp = Keypair::from_seed(seed);
assert_eq!(mismo_kp.identity_id(), kp.identity_id());
```

## Lo que este crate *no* hace

- **No** genera seeds — quien llama las provee (típicamente desde un CSPRNG vía `agora-core`).
- **No** decide la política de desbloqueo — el caller pide la passphrase.
- **No** zeroea la seed descifrada en memoria — esa es responsabilidad del caller mientras vive en su proceso.

## Deps

- [`agora-core`](../agora-core/LEEME.md) (sólo por `IdentityId`), `argon2`, `chacha20poly1305`, `rand`, `directories`, `thiserror`
