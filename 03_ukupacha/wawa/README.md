# wawa (root / kernel)

> Operating system from scratch. Kernel + boot + filesystem + apps.

`wawa` (Quechua: *new creature, baby*) is the kernel side of the pair; userspace lives in `02_ruway/wawa/`. Filesystem is a **content-addressed DAG** over BLAKE3; apps are WASM modules isolated by `wasmi`, with capabilities gated at the linker; cooperative frame pacing; the host-side forge (`boot`) + AoE (network) + atlas (Fontdue) materialize the DAG. Wawa **never speaks NTFS/Ext4 directly** — everything enters the graph as BLAKE3-addressed objects (forge or AoE).

Gaming-grade north star: AOT WASM + GPU passthrough + BLAKE3 asset streaming (today: `wasmi` + cooperative frame pacing — see Estado).

## Install

Toolchain: nightly with `rust-src`, targets `wasm32-unknown-unknown` and `x86_64-unknown-none`.

```sh
cd 03_ukupacha/wawa
cargo +nightly run -p boot -Z bindeps        # forges the UEFI image and boots it in QEMU

cd 03_ukupacha/wawa/wawa-kernel
cargo +nightly check --target x86_64-unknown-none -Z build-std=core,alloc   # kernel alone

./scripts/build-wawa-image.sh                # publishable QEMU/USB image: apps → image → dist/ (tar.zst)
```

## Compatibility

- **x86_64** — only target (`x86_64-unknown-none`).
- **UEFI-only** boot (`bootloader::UefiBoot`); no BIOS path.
- Userspace counterpart: `02_ruway/wawa/`.

## Crates: kernel

| Crate | Role |
|---|---|
| [`wawa-kernel`](wawa-kernel/README.md) | Kernel (scheduler, syscalls, capabilities). |
| [`wawa-boot`](wawa-boot/README.md) | Host-side forge: builds the kernel as artifact, seeds the genesis graph, emits the UEFI image (package `boot`). |
| [`wawa-fs`](wawa-fs/README.md) | Akasha-over-Ether network protocol (the crate is named `akasha`; the directory keeps its historical name). |

## Crates: apps (WASM in the kernel)

| App | Function |
|---|---|
| `asistente` | Conversational assistant; human-signature cycle over proposals (the machine proposes, the human signs). |
| `ayni` | P2P chat signed with Ed25519 over akasha, serverless. |
| [`pluma`](apps/pluma/README.md) | Markdown viewer inside the kernel. |
| [`bitacora`](apps/bitacora/README.md) | System logbook. |
| [`cronista`](apps/cronista/README.md) | Historical logger. |
| [`discola`](apps/discola/README.md) | Disk/media. |
| [`glotona`](apps/glotona/README.md) | Eats heavy tasks (batch). |
| [`hello_wasm`](apps/hello_wasm/README.md) | WASM hello-world. |
| [`memoriosa`](apps/memoriosa/README.md) | Persistent user memory. |
| [`mudanza`](apps/mudanza/README.md) | Migrate between snapshots. |
| [`pregon`](apps/pregon/README.md) | System announcements. |
| [`pulso`](apps/pulso/README.md) | Heartbeat/health checks. |
| [`rimay`](apps/rimay/README.md) | Bare-metal embeddings verb. |
| `testigo` | Witness of the `tinkuy` engine (exercises the `sys_tinkuy_*` capabilities). |
| `tinkuy` | DOD particle engine as a WASM module. |
| [`tonada`](apps/tonada/README.md) | Tonada → audio player. |
| [`tonalero`](apps/tonalero/README.md) | Tonada producer. |

## Considerations

- **WASM-first**: apps aren't native processes; they're WASM modules with explicit capabilities.
- **Byte immutability**: once the hash is computed, those bytes don't change; "edits" are new hashes.
- **Zero direct NTFS/Ext4**: the host-side forge seeds the graph; Wawa reads the DAG.
- **Cooperative pacing**: no app can hog the frame; the scheduler asks it to yield.

## Estado (2026-06-09)

> Fuente autoritativa del estado del kernel: [WAWA.md](../../WAWA.md) (§0–§14) y [SDD-capacidades.md](SDD-capacidades.md). Excluido del workspace raíz: compila `x86_64-unknown-none` con `panic = "abort"`.

### Hecho (subsistemas en disco)
- **Boot + gráficos**: **solo-UEFI** vía `wawa-boot` (`bootloader::UefiBoot`; consume el kernel como `artifact = "bin"`), framebuffer GOP, compositor (refactorizado de 1980 LOC a directorio `compositor/`), consola/texto. **Bootea end-to-end en QEMU** (pila del kernel a 1 MiB: montar virtio-sound desbordaba los 80 KiB por defecto del bootloader).
- **Multi-monitor** (Fase 64): driver virtio-gpu nativo multi-scanout + render por output, gateado por `RENASER_MONITORES`; Alt+O mueve la ventana enfocada al siguiente monitor; `gop-probe` para sondear GOP en metal.
- **Marco de escritorio `pata`** (Fase 9): el kernel pinta el marco (`compositor/pata_marco.rs`) — franja reservada dinámica, start button que abre el launcher, workspace switcher, config bidireccional por akasha, cobertura de `WidgetView::TextRich` y `Moon`.
- **Imagen publicable**: `scripts/build-wawa-image.sh` encadena apps → imagen UEFI autocontenida (grafo de génesis como ramdisk) → `dist/` con `wawa.img` + lanzador QEMU portable + SHA256SUMS + tarball `.tar.zst`.
- **Reactor cooperativo** (`async_system/`): PIT 100Hz + IRQs, executor de tareas; frame pacing cooperativo.
- **Almacenamiento direccionado por contenido** (`almacen.rs`): BLAKE3 + log + GC mark/sweep/swap semántico; configuración (idioma+tema) como nodo del grafo inyectada en `ContextoCapacidades`.
- **Apps WASM** (`wasm/`): aisladas por `wasmi`, fuel/tick + techo de memoria, capacidades gateadas en el linker (frontera física, no tabla). 17 apps en `apps/` (asistente, ayni, bitácora, cronista, discola, glotona, hello_wasm, memoriosa, mudanza, pluma, pregón, pulso, rimay, testigo, tinkuy, tonada, tonalero).
- **Capacidades firmadas §14.1.3** (cableado code-complete): concesión firmada por hash de bytecode (`claves::verificar_concesion_capacidad`), enforcement por intersección viva (`permisos_efectivos_de`), seam de génesis al boot que ancla concesiones offline, revocación cableada al overlay de agora, ceremonia automatizada (`scripts/wawa-conceder-genesis.sh`) y flip estricto como toggle nombrado `MODO_CAPACIDAD_ESTRICTO_GLOBAL`.
- **Red `akasha`** (EtherType propio, sin TCP/IP) + bridge Akasha-over-Ether vía transporte TAP host↔guest; descarga recursiva del DAG delta (anuncio arrastra el cono canal→manifiesto→bytecodes); bucle Aceptar/Rechazar de `mudanza`.
- **USB/HID**: inicialización de todos los controladores XHCI, Port Power, ratón USB HID nativo (boot protocol).
- **Firmas Ed25519** en el kernel (`claves.rs`, zero-alloc `ed25519-compact`): `verificar_manifiesto_firmado` / `verificar_anuncio_canal` / `verificar_cuaderno_firmado` / `verificar_revocacion`.

### Pendiente
- Flipear `MODO_CAPACIDAD_ESTRICTO_GLOBAL` a `true` tras sembrar las concesiones de génesis (hoy `false`: sin concesión rigen los permisos declarados en `EntradaApp`) — el enforcement y la ceremonia ya existen (WAWA.md §14.1.3).
- Optimizaciones gaming pendientes: AOT WASM (cranelift) sobre el path de ejecución, GPU passthrough, asset streaming BLAKE3 (hoy el render es el camino base).
- Sin restart automático de apps (oneshot por diseño); "process monitor" de Wawa (censo del executor + balizas del compositor) aún por construir.
- El camino sobre hardware metal real (más allá de QEMU) sigue endureciéndose (XHCI/GOP). No hay port aarch64: el target único es `x86_64-unknown-none`.
