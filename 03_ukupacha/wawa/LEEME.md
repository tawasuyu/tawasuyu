# wawa (root / kernel)

> Sistema operativo desde cero. Kernel + boot + filesystem + apps.

`wawa` (quechua: *bebé, criatura nueva*) es el lado kernel del par; el userspace vive en `02_ruway/wawa/`. Filesystem es un **DAG content-addressed** sobre BLAKE3; las apps son módulos WASM aislados por `wasmi`, con capacidades gateadas en el linker; pacing cooperativo del frame; la forja host-side (`boot`) + AoE (red) + atlas (Fontdue) materializan el DAG. Wawa **nunca habla NTFS/Ext4 directo** — todo entra al grafo como objetos direccionados por BLAKE3 (forja o AoE).

Norte gaming-grade: AOT WASM + GPU passthrough + asset streaming BLAKE3 (hoy: `wasmi` + frame pacing cooperativo — ver Estado).

## Instalación

Toolchain: nightly con `rust-src`, targets `wasm32-unknown-unknown` y `x86_64-unknown-none`.

```sh
cd 03_ukupacha/wawa
cargo +nightly run -p boot -Z bindeps        # forja la imagen UEFI y la arranca en QEMU

cd 03_ukupacha/wawa/wawa-kernel
cargo +nightly check --target x86_64-unknown-none -Z build-std=core,alloc   # solo el kernel

./scripts/build-wawa-image.sh                # imagen QEMU/USB publicable: apps → imagen → dist/ (tar.zst)
```

## Compatibilidad

- **x86_64** — único target (`x86_64-unknown-none`).
- Boot **solo-UEFI** (`bootloader::UefiBoot`); sin camino BIOS.
- Pareja userspace: ver `02_ruway/wawa/` (panel + wawactl).

## Crates: kernel

| Crate | Rol |
|---|---|
| [`wawa-kernel`](wawa-kernel/README.md) | Kernel (scheduler, syscalls, capabilities). |
| [`wawa-boot`](wawa-boot/README.md) | Forja host-side: compila el kernel como artefacto, siembra el grafo de génesis y emite la imagen UEFI (paquete `boot`). |
| [`wawa-fs`](wawa-fs/README.md) | Protocolo de red Akasha-over-Ether (el crate se llama `akasha`; el directorio conserva el nombre histórico). |

## Crates: apps (WASM en el kernel)

| App | Función |
|---|---|
| `asistente` | Asistente conversacional; ciclo de firma humana de propuestas (la máquina propone, el humano firma). |
| `ayni` | Chat P2P firmado con Ed25519 sobre akasha, sin servidor. |
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
| [`rimay`](apps/rimay/README.md) | Verbo de embeddings bare-metal. |
| `testigo` | Testigo del motor `tinkuy` (ejerce las capacidades `sys_tinkuy_*`). |
| `tinkuy` | Motor de partículas DOD como módulo WASM. |
| [`tonada`](apps/tonada/README.md) | Tonada → reproductor de audio. |
| [`tonalero`](apps/tonalero/README.md) | Productor de tonadas. |

## Consideraciones

- **WASM-first**: las apps no son procesos nativos; son módulos WASM con capabilities explícitas.
- **Inmutabilidad de bytes**: una vez calculado el hash, esos bytes no cambian; las "ediciones" son nuevos hashes.
- **Cero NTFS/Ext4 directo**: la forja en el host siembra el grafo; Wawa lee el DAG.
- **Pacing cooperativo**: ninguna app puede monopolizar el frame; el scheduler le pide ceder.

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
