# agora-keystore

> Encrypted storage of Ed25519 private seeds for [agora](../README.md).

[`agora-core::Keypair`](../agora-core/README.md) is built from a 32-byte seed. That seed is the only thing that needs to be persisted to perpetuate an identity across runs — but it must *never* travel in plaintext: writing it to disk verbatim would let any process that reads the file impersonate the owner. [`agora-store`](../agora-store/README.md) declares this explicitly and leaves the keystore as a separate, deliberate concern. This crate is that concern.

## Wire format

A keystore file is `magic(8) || version(4 LE) || salt(16) || nonce(12) || ciphertext(48) = 88 bytes`. The ciphertext is `seed(32) || Poly1305 tag(16)`.

- **Magic**: `b"agorakey"`. Files that do not start with this are rejected.
- **Version**: `1` today. Forward-incompatible schema changes bump it.
- **Salt**: 16 random bytes per file, fed to Argon2id alongside the passphrase.
- **Nonce**: 12 random bytes, fresh per encryption — never reused for the same key.
- **KDF**: Argon2id with the default parameters of `argon2 = "0.5"` (`m=19456 KiB, t=2, p=1`). Output: 32 bytes used as the ChaCha20 key.
- **AEAD**: ChaCha20-Poly1305. The Poly1305 tag turns "wrong passphrase" and "tampered file" into the same observable failure.

## Files on disk

A `Keystore` is a directory: one file per identity, named `<hex(IdentityId)>.key`. Default location is `~/.local/share/agora/keys/` (resolved via `directories`). Saves are atomic (tmp + rename).

## API

```rust
use agora_core::Keypair;
use agora_keystore::Keystore;

let ks = Keystore::open_default()?;
let kp = Keypair::from_seed([42; 32]);
ks.save(kp.identity_id(), &[42; 32], "passphrase fuerte")?;

// Later, on another run:
let seed = ks.load(kp.identity_id(), "passphrase fuerte")?;
let same_kp = Keypair::from_seed(seed);
assert_eq!(same_kp.identity_id(), kp.identity_id());
```

## What this crate does *not* do

- It does **not** generate seeds — the caller provides them (typically from a CSPRNG via `agora-core`).
- It does **not** decide unlock policy — the caller asks for the passphrase.
- It does **not** memory-zero the decrypted seed — that is the caller's responsibility while it lives in their process.

## Deps

- [`agora-core`](../agora-core/README.md) (for `IdentityId` only), `argon2`, `chacha20poly1305`, `rand`, `directories`, `thiserror`
