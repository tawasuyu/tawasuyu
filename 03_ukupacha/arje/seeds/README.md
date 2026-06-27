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
| `arje-laptop.card.json` | **DEMO** del boot-chain (lo que instala `install-arje.sh`) | splash + arje-getty-stub@tty1 (prueba de vida, **no** login) |
| `arje-tawasuyu.card.json` | **PRODUCCIÓN** sobre rootfs hammer | hammerd (Restart), network-up, splash (high), display-manager (mirada-greeter-llimphi) con Mesa, console-getty@tty2 (rescate busybox) |

> **`arje-laptop` (demo) vs `arje-tawasuyu` (producción).** El demo termina en
> `arje-getty-stub` (banner en tty1, bloquea, **no es login**) y mete sólo
> binarios estáticos musl — sirve para certificar el boot-chain sin rootfs con
> Mesa. La de producción cambia ese último ente por el **greeter real**
> (`mirada-greeter-llimphi`, que toma el DRM tras el handoff del splash) y suma
> `hammerd`; exige un rootfs con Mesa. Ver «Contrato rootfs» abajo.

## Contrato rootfs para `arje-tawasuyu` (producción)

El sustrato es el rootfs reproducible de [hammer](https://gitea.gioser.net/sergio/hammer)
(ver su `docs/12-init-real.md`), que **ya** bootea con `arje-zero` como PID 1.
Para que `arje-tawasuyu.card.json` arranque al escritorio, ese rootfs debe proveer,
además de lo del SDD 12 (mount points vacíos, `/sbin/init`→arje-zero, `/ente/seed.card.json`):

| Necesidad | Path | Origen |
|---|---|---|
| Greeter real | `/usr/lib/arje/mirada-greeter-llimphi` (bin `mirada-greeter`) | receta/cargo tawasuyu |
| Splash | `/usr/lib/arje/arje-splash` | receta/cargo tawasuyu |
| Red mínima | `/usr/lib/arje/net-bring-up` | receta/cargo tawasuyu |
| Lab daemon | `/usr/bin/hammerd` | hammer (ya en Stage 1) |
| Shell rescate | `/bin/busybox` (getty + `/bin/sh`) | hammer (busybox) |
| **Mesa** | `/usr/lib/dri/*` (`LIBGL_DRIVERS_PATH`), `libEGL`/`libgbm`/`libseat`/`wayland` | Alpine (validación) → receta propia |
| Runtime dir | `/run/arje` (XDG_RUNTIME_DIR del DM) | tmpfs (arje monta `/run`) |
| Nodo DRM | `/dev/dri/card*` + `renderD*` | kernel/devtmpfs |

El handoff splash→greeter es por `/run/arje-splash.sock` (ya implementado). La
dependencia dura nueva del salto demo→prod es **Mesa**: en la fase de validación
sobre Alpine se reusa el Mesa de Alpine; recetizar Mesa propio es trabajo posterior.

## Session profiles (`fragments/`)

A **boot profile** is the base Seed Card **plus**, optionally, the entes
of a *session*. The base already is the **mirada** profile: basic inits
(agetty, network-up, splash) and the **greeter** (`mirada-greeter-llimphi`),
which is the DM. Mirada is native — it needs no extra system services.

Pass `arje.session=<name>` on the kernel cmdline (or `ARJE_SESSION` in
dev) to overlay a session fragment from [`fragments/`](fragments/):

- `arje.session=mirada` (or nothing): the base alone.
- `arje.session=gnome`: base + `session-gnome`, which adds the
  `arje-compat` D-Bus shims (logind, hostnamed, timedated, …) so a GNOME
  session launched from the greeter finds the `org.freedesktop.*` names
  it queries at startup.

`arje-zero` composes them with `profile::overlay_session` (dedup by
`label`). A missing/invalid fragment falls back to the base — a
mistyped profile never leaves the host without a boot. See
[`fragments/README.md`](fragments/README.md).

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
