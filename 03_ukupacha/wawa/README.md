# wawa (root / kernel)

> Sistema operativo desde cero. Kernel + boot + filesystem + apps.

`wawa` (quechua: *bebé, criatura nueva*) es el lado kernel del par; el userspace vive en `02_ruway/wawa/`. Filesystem es un **DAG content-addressed** sobre BLAKE3 (ingesta POSIX → BLAKE3 → grafo); las apps son WASM (cranelift AOT); pacing cooperativo del frame; GPU passthrough cuando hay; el destilador (host) + AoE (red) + atlas (Fontdue) materializan el DAG. Wawa **nunca habla NTFS/Ext4 directo** — todo entra por el destilador.

Gaming-grade: AOT WASM + GPU passthrough + frame pacing cooperativo + asset streaming BLAKE3.

## Instalación

```sh
# build del kernel
cargo build --release -p wawa-kernel

# build del bootloader
cargo build --release -p wawa-boot

# levantar el filesystem
cargo run --release -p wawa-fs

# correr en QEMU (script provisto)
./scripts/wawa-qemu.sh
```

## Compatibilidad

- **x86_64** — primary.
- **aarch64** — limitado.
- Pareja userspace: ver `02_ruway/wawa/` (panel + wawactl).
- Boot via `arje` o vía bootloader externo (GRUB, systemd-boot).

## Crates: kernel

| Crate | Rol |
|---|---|
| [`wawa-kernel`](wawa-kernel/README.md) | Kernel (scheduler, syscalls, capabilities). |
| [`wawa-boot`](wawa-boot/README.md) | Bootloader del kernel. |
| [`wawa-fs`](wawa-fs/README.md) | Filesystem (DAG BLAKE3). |

## Crates: apps (WASM en el kernel)

| App | Función |
|---|---|
| [`pluma`](apps/pluma/README.md) | Visor de markdown adentro del kernel. |
| [`bitacora`](apps/bitacora/README.md) | Bitácora del sistema. |
| [`cronista`](apps/cronista/README.md) | Logger histórico. |
| [`discola`](apps/discola/README.md) | Disco/media. |
| [`glotona`](apps/glotona/README.md) | Comer tareas pesadas (batch). |
| [`hello_wasm`](apps/hello_wasm/README.md) | Hello-world WASM. |
| [`memoriosa`](apps/memoriosa/README.md) | Memoria persistente del usuario. |
| [`mudanza`](apps/mudanza/README.md) | Migrar entre snapshots. |
| [`pregon`](apps/pregon/README.md) | Anuncios del sistema. |
| [`pulso`](apps/pulso/README.md) | Heartbeat/health checks. |
| [`tonada`](apps/tonada/README.md) | Tonada → reproductor de audio. |
| [`tonalero`](apps/tonalero/README.md) | Productor de tonadas. |

## Consideraciones

- **WASM-first**: las apps no son procesos nativos; son módulos WASM con capabilities explícitas.
- **Inmutabilidad de bytes**: una vez calculado el hash, esos bytes no cambian; las "ediciones" son nuevos hashes.
- **Cero NTFS/Ext4 directo**: el destilador en el host genera el DAG; Wawa lee el DAG.
- **Pacing cooperativo**: ninguna app puede monopolizar el frame; el scheduler le pide ceder.
