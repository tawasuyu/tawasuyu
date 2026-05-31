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

## Estado (2026-05-31)

> Fuente autoritativa del estado del kernel: [WAWA.md](../../WAWA.md) (§0–§14) y [SDD-capacidades.md](SDD-capacidades.md). Excluido del workspace raíz: compila `x86_64-unknown-none` con `panic = "abort"`.

### Hecho (subsistemas en disco)
- **Boot + gráficos**: UEFI vía `wawa-boot` (consume el kernel como `artifact = "bin"`), framebuffer GOP, compositor (refactorizado de 1980 LOC a directorio `compositor/`), consola/texto, multi-monitor (`gop-probe`).
- **Reactor cooperativo** (`async_system/`): PIT 100Hz + IRQs, executor de tareas; frame pacing cooperativo.
- **Almacenamiento direccionado por contenido** (`almacen.rs`): BLAKE3 + log + GC mark/sweep/swap semántico; configuración (idioma+tema) como nodo del grafo inyectada en `ContextoCapacidades`.
- **Apps WASM** (`wasm/`): aisladas por `wasmi`, fuel/tick + techo de memoria, capacidades gateadas en el linker (frontera física, no tabla). 17 apps en `apps/` (asistente, ayni, bitácora, cronista, discola, glotona, hello_wasm, memoriosa, mudanza, pluma, pregón, pulso, rimay, testigo, tinkuy, tonada, tonalero).
- **Capacidades firmadas §14.1.3**: concesión firmada por hash de bytecode, enforcement por intersección viva, seam de génesis al boot que ancla concesiones offline, y revocación cableada al overlay de agora.
- **Red `akasha`** (EtherType propio, sin TCP/IP) + bridge Akasha-over-Ether vía transporte TAP host↔guest; descarga recursiva del DAG delta (anuncio arrastra el cono canal→manifiesto→bytecodes); bucle Aceptar/Rechazar de `mudanza`.
- **USB/HID**: inicialización de todos los controladores XHCI, Port Power, ratón USB HID nativo (boot protocol).
- **Firmas Ed25519** en el kernel (`claves.rs`, zero-alloc `ed25519-compact`): `verificar_manifiesto_firmado` / `verificar_anuncio_canal` / `verificar_cuaderno_firmado` / `verificar_revocacion`.

### Pendiente
- Tabla de capacidades por bytecode hash como reemplazo total de las declaradas en `EntradaApp` (WAWA.md §14.1.3) — el enforcement existe, falta derivar permisos de la firma como única fuente.
- Optimizaciones gaming pendientes: AOT WASM (cranelift) sobre el path de ejecución, GPU passthrough, asset streaming BLAKE3 (hoy el render es el camino base).
- Sin restart automático de apps (oneshot por diseño); "process monitor" de Wawa (censo del executor + balizas del compositor) aún por construir.
- aarch64 limitado; el camino sobre hardware metal real (más allá de QEMU) sigue endureciéndose (XHCI/GOP).
