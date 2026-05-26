# `arje-packager` — initramfs cpio.gz desde una Tarjeta Semilla

Lib + binario que materializa una semilla canónica (ver
[`03_ukupacha/arje/seeds/`](../../seeds)) en un `initramfs.cpio.gz` bootable.

```sh
cargo build --release -p arje-packager
target/release/arje-packager \
    --seed 03_ukupacha/arje/seeds/arje-qemu.card.json \
    --out  /tmp/arje-qemu.cpio.gz \
    --bin  arje-zero=target/release/arje-zero \
    --bin  agetty-ttyS0=/sbin/agetty
```

El packager:

1. Carga y valida la seed con `EntityCard::from_path` — falla rápido si el
   schema cambió o si la seed está rota.
2. Recorre `genesis` recursivamente y exige un `--bin label=path` por cada
   payload `Native`/`Legacy`. El label viene del campo `label` del Ente; la
   ruta destino dentro del archive sale del `exec` declarado.
3. Crea los directorios mínimos del initramfs (`/dev`, `/proc`, `/sys`,
   `/ente`, `/run`, `/sbin`, `/usr/lib/arje`) más `/dev/console` como char
   `5,1` (necesario para que `arje-zero` abra la shell de rescate sin
   depender de devtmpfs).
4. Plantea `/init` como symlink a `/sbin/arje-zero` — convención del
   kernel Linux para que el primer proceso del initramfs sea PID 1 sin
   bootloader extra.
5. Embebe la seed serializada (JSON canónico) en `/ente/seed.card.json`.
6. Comprime con gzip y escribe a `--out`.

## Booteo con QEMU (smoke test sin GPU)

```sh
qemu-system-x86_64 \
    -m 256M \
    -kernel /boot/vmlinuz-lts \
    -initrd /tmp/arje-qemu.cpio.gz \
    -append "console=ttyS0 panic=10" \
    -nographic
```

Si la seed es `arje-qemu.card.json`, vas a ver el banner de arje-zero por
`ttyS0` seguido del prompt de `agetty`.

## Lo que NO hace

- **Resolver dependencias dinámicas.** Los binarios deben ser estáticos
  (musl). Si no, vas a tener que agregar `/lib*/ld-linux-*.so` y compañía
  por separado — `arje-packager` no corre `ldd`.
- **Generar el kernel.** El `bzImage` viene de otro lado.
- **Firmar el archive.** Integridad cruzada (seed ↔ binarios) sale del
  Capability fractal en tiempo de boot, no del packaging.
