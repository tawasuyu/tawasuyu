# `seeds/` — Tarjetas Semilla canónicas del fractal arje

Cada archivo en este directorio es una **Tarjeta Semilla** [`EntityCard`]
serializada — el input que [`arje-zero`](../init/arje-zero) lee al
encarnar como PID 1. El fractal arranca exactamente el conjunto de
hijas que la semilla describe, supervisándolas según su `Supervision`.

## Catálogo

| Archivo | Para qué | Servicios genesis |
|---|---|---|
| `arje-host.card.json` | Laptop Artix con GPU física | agetty@tty1, network-up oneshot, display-manager (mirada-greeter-llimphi) con Mesa |
| `arje-qemu.card.json` | Pruebas en QEMU sin GPU | agetty@ttyS0 (consola serie) |

## Cómo se usan

```sh
# En tiempo de instalación: el packager copia la semilla al filesystem.
install -m 0644 03_ukupacha/arje/seeds/arje-host.card.json /ente/seed.card.json

# arje-zero la carga al arrancar.
arje-zero               # busca /ente/seed.card.json (o /ente/seed.card)

# En desarrollo, sin /ente:
arje-zero --dev         # busca ./seed.card.json en el CWD
```

Para armar el initramfs bootable a partir de la seed —
ver [`arje-packager`](../init/arje-packager) — usá:

```sh
target/release/arje-packager \
    --seed 03_ukupacha/arje/seeds/arje-qemu.card.json \
    --out  /tmp/arje-qemu.cpio.gz \
    --bin  arje-zero=target/release/arje-zero \
    --bin  agetty-ttyS0=/sbin/agetty
```

`arje-zero` invoca `EntityCard::validate()` recursivamente sobre `genesis`
antes de encarnar nada — una semilla con un campo roto frena el arranque
con `anyhow::bail!` antes de tocar fork(2).

## Garantía de validez

`init/arje-zero/tests/seeds.rs` parsea cada `.card.json` de este
directorio y lo valida con [`EntityCard::from_path`]. Cualquier cambio
al schema (`shared/card/card-core`) o a las propias seeds se detecta en
`cargo test -p arje-zero` antes de llegar al hardware.

Para correr sólo esos tests:

```sh
cargo test -p arje-zero --test seeds
```

## Cómo agregar una semilla

1. Copiá la seed más cercana al target (`arje-host` para con-GPU, `arje-qemu`
   para sin-GPU).
2. Ajustá `label`, los `id` (ulids únicos — usá `ulid` CLI o
   `Ulid::new()` en un REPL), y la lista `genesis`.
3. Agregá un test `*_seed_es_valida()` en `tests/seeds.rs`.
4. Documentá la nueva semilla en la tabla de arriba.

## Lo que NO va acá

- Estado dinámico — `genesis` se *encarna* en runtime; los `id` y
  vínculos de los hijos vivos viven en `arje-snapshot`, no acá.
- Secretos — la semilla viaja con la imagen rootfs, sin cifrar. Si
  necesitás credenciales, declarálas como `Capability` que un Ente
  cap-providing entrega vía socket, no como campos del JSON.
- Configuración mutable — la seed es immutable por convención; mutaciones
  del fractal vivo van por el bus (`arje-bus`), no por re-escritura del
  archivo.
