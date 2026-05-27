# wawa (root / kernel)

> Operating system from scratch. Kernel + boot + filesystem + apps.

`wawa` (Quechua: *new creature, baby*) is the kernel side of the pair; userspace lives in `02_ruway/wawa/`. Filesystem is a **content-addressed DAG** over BLAKE3 (POSIX → BLAKE3 ingest → graph); apps are WASM (cranelift AOT); cooperative frame pacing; GPU passthrough when present; the distiller (host) + AoE (network) + atlas (Fontdue) materialize the DAG. Wawa **never speaks NTFS/Ext4 directly** — everything enters via the distiller.

Gaming-grade: AOT WASM + GPU passthrough + cooperative frame pacing + BLAKE3 asset streaming.

## Install

```sh
cargo build --release -p wawa-kernel
cargo build --release -p wawa-boot
cargo run --release -p wawa-fs
./scripts/wawa-qemu.sh
```

## Compatibility

- **x86_64** — primary.
- **aarch64** — limited.
- Userspace counterpart: `02_ruway/wawa/`.

Crates listed in [README.md](README.md).

## Considerations

- **WASM-first**: apps aren't native processes; they're WASM modules with explicit capabilities.
- **Byte immutability**: once the hash is computed, those bytes don't change; "edits" are new hashes.
- **Zero direct NTFS/Ext4**: the host distiller produces the DAG; Wawa reads the DAG.
- **Cooperative pacing**: no app can hog the frame; the scheduler asks it to yield.
