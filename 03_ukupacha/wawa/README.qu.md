<!-- Quechua (Cusco/Collao). Revisión bienvenida. -->

# wawa (saphi / kernel)

> Pacha-musuq sistema operativo. Kernel + boot + filesystem + apps.

`wawa` (runa-simi: *musuq kawsay, criatura mosoq*) parejaq kernel ñiqip; userspace `02_ruway/wawa/`-pi. Filesystem **content-addressed DAG** BLAKE3 patanpi (POSIX → BLAKE3 ingest → grafu); apps WASM (cranelift AOT); cooperativo frame pacing; GPU passthrough kaqtin; destilador (host) + AoE (red) + atlas (Fontdue) DAGta materializan. Wawa **manaña NTFS/Ext4 sutilla rimakhuq** — tukuy destilador-rayku haykun.

## Churay

```sh
cargo build --release -p wawa-kernel
cargo build --release -p wawa-boot
cargo run --release -p wawa-fs
./scripts/wawa-qemu.sh
```

## Tinkuy

- **x86_64** — ñawpaq.
- **aarch64** — chinka.
- Userspace pareja: `02_ruway/wawa/`.

Crateskuna [README.md](README.md)-pi.

## Yuyaykunaq

- **WASM-ñawpaq**: apps manan natural procesos; WASM módulos sutilla capabilitieswan.
- **Bytes mana tikraq**: hash yupayqasqa qhipa, byteskuna manaña tikran; "tikrana" musuq hash.
- **Sutilla NTFS/Ext4 mana**: host destilador DAGta paqarichin; Wawa DAG ñawichaq.
- **Cooperativo pacing**: mana huk app frame hatuyqa; scheduler chimpata mañan.
