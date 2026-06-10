# `seeds/` — canonical Seed Cards of the arje fractal

Every file in this directory is a serialized **Seed Card** [`EntityCard`]
— the input that [`arje-zero`](../init/arje-zero) reads when it
incarnates as PID 1. The fractal boots exactly the set of children the
seed describes, supervising them according to their `Supervision`.

## Catalog

| File | What for | Genesis services |
|---|---|---|
| `arje-host.card.json` | Artix laptop with a physical GPU | agetty@tty1, network-up oneshot, display-manager (mirada-greeter-llimphi) with Mesa |
| `arje-qemu.card.json` | QEMU testing without a GPU | agetty@ttyS0 (serial console) |

## How they are used

```sh
# At install time: the packager copies the seed into the filesystem.
install -m 0644 03_ukupacha/arje/seeds/arje-host.card.json /ente/seed.card.json

# arje-zero loads it on boot.
arje-zero               # looks for /ente/seed.card.json (or /ente/seed.card)

# In development, without /ente:
arje-zero --dev         # looks for ./seed.card.json in the CWD
```

To build the bootable initramfs from a seed —
see [`arje-packager`](../init/arje-packager) — use:

```sh
target/release/arje-packager \
    --seed 03_ukupacha/arje/seeds/arje-qemu.card.json \
    --out  /tmp/arje-qemu.cpio.gz \
    --bin  arje-zero=target/release/arje-zero \
    --bin  agetty-ttyS0=/sbin/agetty
```

`arje-zero` calls `EntityCard::validate()` recursively over `genesis`
before incarnating anything — a seed with a broken field stops the boot
with `anyhow::bail!` before touching fork(2).

## Validity guarantee

`init/arje-zero/tests/seeds.rs` parses every `.card.json` in this
directory and validates it with [`EntityCard::from_path`]. Any change
to the schema (`shared/card/card-core`) or to the seeds themselves is
caught by `cargo test -p arje-zero` before reaching hardware.

To run just those tests:

```sh
cargo test -p arje-zero --test seeds
```

## How to add a seed

1. Copy the seed closest to your target (`arje-host` for with-GPU,
   `arje-qemu` for without-GPU).
2. Adjust `label`, the `id`s (unique ulids — use the `ulid` CLI or
   `Ulid::new()` in a REPL), and the `genesis` list.
3. Add a `*_seed_es_valida()` test in `tests/seeds.rs`.
4. Document the new seed in the table above.

## What does NOT belong here

- Dynamic state — `genesis` is *incarnated* at runtime; the `id`s and
  links of live children live in `arje-snapshot`, not here.
- Secrets — the seed ships with the rootfs image, unencrypted. If you
  need credentials, declare them as a `Capability` that a cap-providing
  Ente hands over via socket, not as JSON fields.
- Mutable configuration — the seed is immutable by convention; mutations
  of the live fractal go through the bus (`arje-bus`), not by rewriting
  the file.
