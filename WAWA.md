# Wawa — descripción técnica del sistema

> **V1.0.0‑GOLD — forja sellada (commit `00feda8`, Fase 50).**
> *"La integridad no es una esperanza estadística; es una certeza geométrica."*
> 5/5 shared cores verified. Loop autónomo finalizado. Estado estable.
>
> **Post‑GOLD (Fases 51–62).** El sello no detuvo el desarrollo. Arco
> «legacy → virtio moderno» completado: consola (49), scanout virtio‑gpu (60),
> puntero absoluto virtio‑input/tableta (61), audio PCM real virtio‑sound (62).
> Arco IA «la máquina propone, el humano firma» (app `asistente`, Fase 60).
> App `rimay` (verbo de embeddings bare‑metal). GENESIS pasó de 12 a **14** apps.
> Las secciones §9, §13 y §14 de abajo reflejan este estado; el resto del
> documento describe la arquitectura sellada en GOLD, que sigue vigente.

> SO experimental SASOS x86_64 bare‑metal. Direccionamiento por contenido,
> userspace WebAssembly aislado por software, reactor asíncrono cooperativo,
> red propia capa‑2 (Akasha). Documento dirigido a un agente experto que va
> a operar sobre el repositorio.

---

## 0. Acta de cierre del Manifiesto Técnico (V1.0.0‑GOLD)

Cincuenta fases han descendido desde el sector cero de un disco UEFI virgen
hasta la celda interactiva que teclea el operador soberano. El sistema queda
sellado con los siguientes veredictos en verde inmaculado:

| Capa | Sello |
|---|---|
| **Layer 1 — tipos puros** | `format`, `akasha`, `mirada-layout`, `forth-emisor`, `pluma-notebook-core` pasan `scripts/check-shared-cores.sh` (5/5 `#![no_std]` + `cargo check --target wasm32-unknown-unknown`). |
| **Layer 2 — microkernel** | Linker paravirtualizado moderno sobre `consola_virtio.rs` (Fase 49) + `virtio-blk-pci` (Fase 6) + anillo Ed25519 multi‑autor (Fase 48) con CRL en `.rodata`. ABI Ring 0 ↔ Ring 3 congelado: 8 variantes de `CodigoError` con firma numérica fija (test `test_wawa_ecosystem_immutable_vanguard`). |
| **Layer 3 — userspace unificado** | `apps/pluma` consolidado en 11 159 B (10.90 KiB) tras `wasm-opt -Os --strip-debug --strip-producers --strip-target-features --enable-bulk-memory`. Walker rehidrata el cuaderno persistido (Fase 44/45) y la cascada (`RETORNO_HEREDADO`) sobrevive a cualquier corte de energía. |

Wawa supera por construcción los pecados de los monolitos hipertróficos de los
años 70:
* No hay tabla de privilegios que escalar — las capacidades no se registran en
  el `Linker` de wasmi si el bit del bitfield `Permisos` no está puesto.
* No hay puntero salvaje que desreferenciar — cada módulo vive en su jaula WASM
  cooperativa con cuota de combustible per‑app.
* No hay raíz mutable que pisar — cada cambio de estado es un nodo nuevo en el
  grafo direccionado por contenido, anclado por hash BLAKE3.

La ortogonalidad SASOS deja de ser teoría de paper y se vuelve arquitectura
corriendo en silicio.

### Reproducir el veredicto

```bash
./scripts/check-shared-cores.sh    # 5/5 núcleos no_std en verde
cargo test -p format                # 20/20 tests incluyendo vanguard
./scripts/build-pluma.sh            # pipeline cargo + wasm-opt + consolidación
```

---

## 1. Layout del monorepo

```
/mnt/vvv/gioser/
├── shared/format/                       # nucleo no_std del format en disco
├── 02_ruway/
│   ├── mirada/mirada-layout/            # teselado puro, no_std, no smithay
│   └── wawa/                            # userspace de control (host-side)
│       ├── wawa-config/                 # bus de config /etc/wawa (host)
│       ├── wawa-config-llimphi/         # adaptador para apps Llimphi
│       ├── wawa-panel-llimphi/          # panel de control
│       └── wawactl/                     # CLI
└── 03_ukupacha/
    ├── wawa/                            # SO bare-metal (kernel + apps WASM)
    │   ├── wawa-boot/                   # host: forja la imagen UEFI/BIOS
    │   ├── wawa-kernel/                 # x86_64-unknown-none
    │   ├── wawa-fs/                     # crate `akasha` (protocolo capa-2)
    │   └── apps/                        # apps WASM cdylib
    │       ├── hello_wasm/              # cuadrado movido por teclado
    │       ├── bitacora/                # editor persistente
    │       ├── pregon/                  # primera voz hacia la red
    │       ├── tonada/ pulso/           # demos audio + reloj
    │       ├── memoriosa/               # demo sys_estado_*
    │       ├── discola/ glotona/        # demos de guardarrailes (fuel/mem)
    │       ├── cronista/                # cuenta arranques (escritura grafo)
    │       └── tonalero/                # testigo de Configuracion (Fase 22+)
    └── wawa-explorer/                   # explorer host-side de imágenes
```

El **kernel está EXCLUIDO del workspace raíz** (target distinto: `x86_64-unknown-none`).
`wawa-boot` lo compila como `[dependencies.kernel]` con `artifact = "bin"` y lo
inyecta vía `CARGO_BIN_FILE_KERNEL_kernel`.

---

## 2. Stack del kernel bare‑metal

`03_ukupacha/wawa/wawa-kernel/src/`:

| Módulo | Rol |
|---|---|
| `main.rs` | Entrada `bootloader_api::entry_point!`, orquesta arranque y reactor |
| `gdt.rs` + `interrupts.rs` + `pic.rs` | GDT/TSS, IDT con excepciones CPU, PIC 8259 remapeado, PIT |
| `memory/` | Heap dinámico (`linked_list_allocator`), MMIO mapping |
| `grafico.rs` + `consola.rs` + `texto.rs` | Framebuffer doble buffer, raster `fontdue` |
| `async_system/` | Reactor cooperativo: `executor`, `task`, `waker`, `reloj` (PIT 100Hz), `teclado` (IRQ1), `puntero` (IRQ12) |
| `drivers/` | `pci` (descubrimiento), `disco` (virtio-blk + DMA HAL), `red` (virtio-net), `gpu` (virtio-gpu, scanout propio — Fase 60), `tableta` (virtio-input, puntero absoluto — Fase 61), `sonido` (virtio-sound, PCM real — Fase 62), `raton` (PS/2 IRQ12), `altavoz` (PC speaker, fallback) |
| `almacen.rs` | Log direccionado por contenido + índice + compactador (Fase 24) |
| `manifiesto.rs` | Manifiesto de Génesis vivo (apps + estado + Configuracion) |
| `compositor.rs` | Teselado vía `mirada-layout` + taskbar + ventanas flotantes |
| `akasha.rs` | Demultiplexor de protocolo Akasha en RX |
| `control.rs` | Canal de control host→kernel sobre virtio-console (`wawactl gc` → `almacen::compactar`, Fase 63) |
| `wasm/` | Runtime `wasmi` + matriz de capacidades + ContextoCapacidades |
| `baliza.rs` | Red de seguridad visual: panic handler, OOM handler, traza serial |

Disco: 32 MiB en `virtio-blk-pci`. Sector 0 = `SuperBloque`. Sector 1+ = log de
registros `[longitud u32 LE][payload postcard][relleno a cero]` alineados a 512 B.
Reactor cooperativo: tareas tipo `Future<Output=()>` despertadas por wakers
desde IRQ y por el reloj PIT.

---

## 3. Crates compartidos no_std (validados por `scripts/check-shared-cores.sh`)

### `shared/format` — la verdad del format en disco

- `VERSION_SUPERBLOQUE = 3` (actual). `SuperBloque { magia, version, log_inicio: u64, cursor: u64, raiz: Option<Hash>, manifiesto: Option<Hash> }`.
- `VERSION_MANIFIESTO = 4`. `Manifiesto { version, apps: Vec<EntradaApp>, configuracion: Option<Hash> }`.
- `EntradaApp { nombre, bytecode: Hash, region_*: u32, techo_memoria: u32, fuel_fotograma: u32, estado: Option<Hash>, permisos: Permisos }`.
- `VERSION_CONFIGURACION = 1`. `Configuracion { version, idioma: IdiomaCodigo, paleta: [u8;20] }`.
- `Hash = [u8; 32]` (BLAKE3 sobre el payload postcard).
- `Permisos: u32` bitfield. Constantes: `PERMISO_RED=1`, `PERMISO_GRAFO_ESCRITURA=2`, `PERMISO_RAIZ=4`, `PERMISO_ALTAVOZ=8`, `PERMISO_CONFIG=16`, `PERMISO_COMPACTAR=32`.
- `CodigoError: #[repr(i32)]` — `Ok=0, Ausente=-1, CapacidadInsuficiente=-2, AlmacenamientoFallo=-3, SinFoco=-4, EnvioFallo=-5`.
- Estabilidad: una variante NUEVA no renumera las existentes (test `codigo_error_tiene_valores_estables`).

### `03_ukupacha/wawa/wawa-fs` — crate `akasha`

Protocolo capa‑2 sobre Ethernet crudo, EtherType propio. `MensajeAkasha`:
- `SolicitarObjeto(Hash)` — broadcast / unicast.
- `ProveedorObjeto(Hash, Vec<u8>)` — respuesta unicast con payload re‑hashable.
- `AnunciarRaiz(Hash)` — faro periódico de la raíz local.
- `AnunciarCanal { canal: Hash, raiz: Hash, autor: AgoraId, timestamp: u64, firma: Firma }` — anuncio firmado de canal de release.

### `02_ruway/mirada/mirada-layout` — teselado

`#![cfg_attr(not(test), no_std)]`. Tipos geométricos puros (`Rect`), modos
de teselado (`LayoutMode::MasterStack` y otros), función `tile()` determinista.

---

## 4. Modelo de aislamiento — los cuatro pilares del hermetismo Ring 0

### 4.1 Time‑Capsule

`wasm::env::ContextoCapacidades.tiempo_ms_fotograma: u64` es un snapshot del
reloj monótono tomado por `AplicacionWasm::tick()` ANTES de ceder la CPU.
`sys_tiempo_mono` lee del contexto, NO del reloj físico. Tres llamadas dentro
del mismo `tick` devuelven el MISMO valor → determinismo intra‑fotograma.

### 4.2 Permisos como fronteras físicas

`enlazar_capacidades(linker, permisos)` en `wasm/env.rs` registra cada
capacidad **gateada** dentro de un `if permisos & PERMISO_X != 0 { ... }`.
Las capacidades NO registradas son símbolos INEXISTENTES — wasmi rechaza
el módulo en `instantiate_and_start` antes de ejecutar nada. **No hay
tabla de despacho que escalar**. La frontera es fisica.

Capacidades de **lectura pasiva siempre disponibles**: `sys_render_frame`,
`sys_get_scancode`, `sys_puntero`, `sys_object_datos`, `sys_object_hijo`,
`sys_object_raiz`, `sys_estado_cargar`, `sys_estado_guardar`,
`sys_tiempo_mono`, `sys_config_idioma`, `sys_config_paleta`.

Gateadas:
- `PERMISO_RED` ⇒ `sys_net_mac`, `sys_net_enviar`, `sys_net_recibir`, `sys_red_solicitar`.
- `PERMISO_GRAFO_ESCRITURA` ⇒ `sys_object_put`.
- `PERMISO_RAIZ` ⇒ `sys_object_fijar_raiz`, `sys_manifiesto_proponer`.
- `PERMISO_ALTAVOZ` ⇒ `sys_tono`.
- `PERMISO_CONFIG` ⇒ `sys_config_proponer`.
- `PERMISO_COMPACTAR` ⇒ `sys_grafo_compactar`.

### 4.3 Geometría del puntero

`async_system::puntero` (espejo de `teclado`): `CanalPuntero = Arc<ArrayQueue<EventoPuntero>>`,
censo indexado por `indice_app`. `compositor::atender_raton` drena la cola
global del PS/2 (driver `drivers/raton.rs`, paquete de 3 bytes en IRQ12), aplica
foco/arrastre, y al final invoca `puntero::enrutar(foco, abs_x, abs_y, …)` con
el marco y el lienzo natural del foco. La traducción descuenta el origen del
marco más el padding de centrado; si el (x,y) cae fuera del **lienzo natural**
de la app enfocada, **se descarta silenciosamente**. La app nunca ve
coordenadas absolutas ni eventos sobre el cromo de su propia ventana ni sobre
otras ventanas.

`sys_puntero(salida) -> i32`: escribe 5 bytes `local_x u16 LE | local_y u16 LE | botones u8`. Retorno `5` si hubo evento, `0` si la cola está vacía.

### 4.4 Swap semántico (no es paginación)

No hay paginación ciega del kernel hacia el disco. La app decide cuándo
serializar sus estructuras intermedias con `postcard`, grabarlas con
`sys_object_put` o `sys_estado_guardar` (retornan un hash), y limpiar su
memoria lineal. Las recupera por hash con `sys_object_datos` /
`sys_estado_cargar`. El coste E/S está a la vista del userspace que lo paga.

---

## 5. Manifiesto de Muerte (Drop limpio de la jaula)

`AplicacionWasm::drop` (`wasm/mod.rs`):

```rust
let indice = self.almacen.data().indice_app;
teclado::cerrar_canal(indice);
puntero::cerrar_canal(indice);
self.memoria.data_mut(&mut self.almacen).fill(0);  // 4 MiB zero-fill
```

El `wasmi::Memory` se retiene como campo de `AplicacionWasm` solo para esto.
El siguiente owner del bloque del heap no encuentra residuos de la app
desalojada.

---

## 6. ABI WASM completo

| Capacidad | Firma | Permiso | Descripción |
|---|---|---|---|
| `sys_render_frame` | `(ptr, len)` | — | Composita un fotograma del lienzo natural (BGRA, `nat_ancho × nat_alto × 4`). |
| `sys_get_scancode` | `() -> u32` | — | Scancode set 1, 0 si la cola está vacía. |
| `sys_puntero` | `(salida) -> i32` | — | 5 bytes evento o `Ok=0` (cola vacía). |
| `sys_object_put` | `(datos_ptr, datos_len, hijos_ptr, hijos_cnt, salida)` | GRAFO_ESCRITURA | Graba objeto, devuelve hash en `salida`. |
| `sys_object_datos` | `(hash, salida, capacidad)` | — | Copia payload del objeto; `n` bytes, `Ausente`, `CapacidadInsuficiente`, `AlmacenamientoFallo`. |
| `sys_object_hijo` | `(hash, indice, salida)` | — | Devuelve nº de hijos; si `indice < n`, escribe el hash en `salida`. |
| `sys_object_raiz` | `(salida) -> i32` | — | `1` si raíz, `0` si no, escribe hash en `salida` si la hay. |
| `sys_object_fijar_raiz` | `(hash_ptr)` | RAIZ | Corona objeto como raíz del DAG. |
| `sys_grafo_compactar` | `() -> i32` | COMPACTAR | Fuerza pasada GC (MARK→SWEEP→SWAP). Retorna `nodos_vivos` o `AlmacenamientoFallo`. |
| `sys_estado_cargar` | `(salida, capacidad)` | — | Lee el estado anclado para esta app (slot del manifiesto). |
| `sys_estado_guardar` | `(datos, datos_len)` | — | Graba nuevo estado y reanca manifiesto vivo. |
| `sys_tiempo_mono` | `() -> u64` | — | Tiempo congelado por fotograma. |
| `sys_tono` | `(frecuencia_hz)` | ALTAVOZ + foco | PC speaker; calla si la app no tiene foco. |
| `sys_net_mac` | `(salida) -> i32` | RED | 6 bytes MAC o `Ausente` si no hay tarjeta. |
| `sys_net_enviar` | `(ptr, len)` | RED | Envía frame Ethernet crudo (sin CRC). |
| `sys_net_recibir` | `(salida, capacidad)` | RED | Recibe frame no‑Akasha; **buffer en pila** de 2048 B máx (MTU clásico). |
| `sys_red_solicitar` | `(hash_ptr)` | RED | Broadcast `SolicitarObjeto(hash)` por Akasha. |
| `sys_config_idioma` | `() -> u32` | — | ISO 639‑1 empaquetado en `u16` (byte bajo = primera letra). |
| `sys_config_paleta` | `(salida) -> i32` | — | 20 bytes (5 colores RGBA8) del tema activo. |
| `sys_config_proponer` | `(idioma, paleta_ptr)` | CONFIG + foco | Engendra `Configuracion` nueva, reanca manifiesto atómicamente. |

Aborto / trampa wasmi cuando un puntero del módulo se sale de su memoria
lineal (todos los `rango()` lo verifican). Aborto cuando agota `fuel_fotograma`.
Aborto cuando intenta crecer memoria más allá de `techo_memoria` (4 MiB).
Cualquier aborto se atrapa, se desaloja la ventana con baliza visual, el
kernel sigue.

---

## 7. Almacenamiento: grafo direccionado por contenido

`almacen.rs`. API:
- `init() -> Resumen` — monta disco, reconstruye índice desde `[log_inicio, cursor)`.
- `almacenar(datos, hijos) -> Result<Hash>` — append al log, dedup, persiste superbloque.
- `recuperar(hash) -> Result<Option<Objeto>>` — lee y reverifica.
- `raiz()`, `fijar_raiz(hash)` — ancla userspace.
- `manifiesto()`, `fijar_manifiesto(hash)` — ancla kernel.
- `compactar() -> Result<EstadisticasCompacta>` — **GC semántico**.

### GC semántico (Fase 24)

`compactar()` ejecuta MARK → SWEEP → SWAP:

1. **MARK**: DFS lineal desde `raiz` y `manifiesto`, siguiendo `objeto.hijos`. Tolera referencias colgantes (no replicadas vía Akasha) sin tumbar el GC.
2. **SWEEP**: copia los registros vivos a partir del cursor actual (sectores limpios al final del disco). Si el set vivo no cabe → `Err` SIN tocar el disco.
3. **SWAP**: una sola escritura del superbloque (`log_inicio = nuevo_inicio`, `cursor = nuevo_cursor`). virtio‑blk entrega el sector entero o nada; ante crash, el superbloque viejo sigue válido y el segmento nuevo es trailing data inerte.

Trigger: `ESCRITURAS_DESDE_GC` (atomic) se incrementa en cada `almacenar` que NO deduplica. Tras `UMBRAL_GC=32` escrituras, la tarea del compositor llama `compactar()` en su tic ocioso y emite traza serial:

```
gc :: compactado :: vivos=N muertos=M sectores=A->B
```

---

## 8. Red — Akasha Over Ether

`drivers::red` (virtio-net) + `akasha.rs` (demultiplexer).

`drenar_y_demultiplexar()` corre cada tic del compositor: para cada frame RX,
intenta parsear como Akasha; si lo es, lo procesa en el kernel (sin entregar al
userspace). Si no, va a `COLA_USUARIO` (`VecDeque<Vec<u8>>` con backpressure
LIFO‑drop).

Hermetismo: `MensajeAkasha::AnunciarCanal` ingresa el DAG en el grafo local
(via `SolicitarObjeto` → `ProveedorObjeto` → `absorber_proveedor` con rehash
verificado), **pero NO reanca nada**. El kernel jamás verifica firmas; la
política de aplicar una raíz de canal es del userspace (futura app `mudanza`).

Dedup de solicitudes recientes por `(MAC origen, hash)` en una ventana
`VENTANA_DEDUP_MS` para tolerar la retransmisión del cliente AoE sin generar
ráfagas de `ProveedorObjeto` redundantes.

---

## 9. Apps de Génesis (boot/main.rs::GENESIS)

Tras la Fase 50 (consolidación de `pluma`) y el ciclo de release firmado de la
Fase 48 (`AGORA_AUTH_RING` + `mudanza`), y sumadas las apps post‑GOLD
`asistente` (Fase 60) y `rimay`, el censo del array
`const GENESIS: [AppGenesis; 14]` (en `wawa-boot/src/main.rs:142`) es **catorce**
módulos:

| Nombre | .wasm | Region (x,y,w,h) | Fuel | Permisos |
|---|---|---|---|---|
| bitacora | bitacora.wasm | (100,120,480,280) | `FUEL_EDITOR` (6M) | 0 (solo estado propio) |
| pregon | pregon.wasm | (100,120,480,160) | `FUEL_COMUN` (2M) | RED |
| tonada | tonada.wasm | (100,120,360,120) | `FUEL_COMUN` | ALTAVOZ |
| pulso | pulso.wasm | (100,120,360,120) | `FUEL_COMUN` | 0 |
| hola | app.wasm | (100,120,480,560) | `FUEL_COMUN` | 0 |
| memoriosa | memoriosa.wasm | (700,120,360,80) | `FUEL_COMUN` | 0 |
| discola | discola.wasm | (60,700,360,80) | `FUEL_COMUN` | 0 (demo `SinCombustible`) |
| glotona | glotona.wasm | (460,700,360,80) | `FUEL_COMUN` | 0 (demo `SinMemoria`) |
| cronista | cronista.wasm | (860,700,360,80) | `FUEL_COMUN` | GRAFO_ESCRITURA \| RAIZ |
| tonalero | tonalero.wasm | (700,220,480,300) | `FUEL_COMUN` | CONFIG |
| **mudanza** | mudanza.wasm | (60,220,480,240) | `FUEL_COMUN` | **RAIZ** — primer cliente de `MensajeAkasha::AnunciarCanal`; invoca `sys_manifiesto_proponer` (Fase 41/48). |
| **pluma** | pluma.wasm | (160,60,480,400) | `FUEL_EDITOR` | **GRAFO_ESCRITURA** — cuaderno reactivo consolidado en 11 KiB (post `wasm-opt -Os`, Fase 50). |
| **asistente** | asistente.wasm | (600,220,480,240) | `FUEL_COMUN` | **RED \| RAIZ** — app conversacional (Fase 60). Pregunta a un LLM externo vía el puente Linux (`asistente-puente --akasha`, EtherType `0x88B6` sobre `CANAL_ASISTENTE 0x4153`); para propuestas hash el operador pulsa SPACE → `RequestFirma` → al llegar la `Firma` host‑side invoca `sys_manifiesto_proponer`. Segunda app con `PERMISO_RAIZ` junto a `mudanza`. La IA propone; el humano firma. |
| **rimay** | rimay.wasm | (100,120,480,560) | `FUEL_COMUN` | 0 — reflejo bare‑metal del verbo de embeddings (FNV‑1a + LCG + coseno, mismo algoritmo que `rimay-verbo-mock`). Sin permisos: sólo framebuffer + teclado. |

`TECHO_GENESIS = 4 MiB`. Cada app: módulo cdylib WASM con `init()` y `tick()`
exportados, `#![no_std]`, panic handler propio (`loop {}` que será atrapado por
el guardarraíl de fuel). `03_ukupacha/wawa/apps/` además contiene `ide/`, fuera
de GENESIS por ahora — disponible para invocación dinámica (Alt+N).

---

## 10. Aislamiento total verificado

Audit (`grep -rE "\.unwrap\(\)|\.expect\(|panic!|unreachable!"`):
- **0 ocurrencias** en `wasm/`, `almacen.rs`, `manifiesto.rs`, `akasha.rs` — los caminos kernel↔userspace propagan errores vía `Result` → `CodigoError` o `FallaApp` → trap wasmi.
- Panics existentes confinados: `interrupts.rs` (#PF/#GP/#DF, no recuperables), `texto.rs` (init de fuente al arranque), `executor.rs` (TaskId duplicado — wrap u64 imposible), `drivers/disco.rs` (DMA HAL — limitación estructural del trait `virtio-drivers::Hal`).

## 11. Zero‑alloc en el lazo crítico

- `compositor::recomponer` **no aloca**. `Escritorio` retiene `capas_buf` y `celdas_buf` (`Vec::with_capacity(MAX_VENTANAS=32)`) reutilizados con `clear() + push()`. El reloj se formatea en pila (`[u8; 8]` + `formatear_reloj`). `consola::{CapaSlot, CeldaTaskbarSlot}` no tienen lifetimes; resolución de bytes/nombres por trait `Resolver`.
- `sys_net_recibir` usa buffer en pila `[u8; 2048]` (MTU clásico Ethernet); cap > 2048 ⇒ `CapacidadInsuficiente`.
- Asignaciones que quedan documentadas como legítimas: `almacenar` en escrituras al grafo (gasto E/S explícito del userspace), `nacer_ventana` (cache de fotograma única al alta). El `to_vec` histórico de `encolar_para_usuario` desapareció en la Fase 55 — `COLA_USUARIO` es ahora un anillo de slots MTU pre-alocados (ver §14.0).

## 12. Simetría no_std

`scripts/check-shared-cores.sh` enumera los núcleos que viajan entre kernel
bare‑metal, módulos WASM y red Akasha (`format`, `akasha`, `mirada-layout`) y
verifica:
1. Declaración `#![no_std]` (acepta `#![cfg_attr(not(test), no_std)]`).
2. `cargo check --target wasm32-unknown-unknown` — std no existe en ese target; un `use std::...` por descuido se delata.

---

## 13. Estado del sistema (commits clave en `main`)

| Commit | Cambio |
|---|---|
| `79a7129` | Configuración como nodo del grafo + inyección en ContextoCapacidades |
| `6a95761`, `d4f13f3` | App `tonalero` (testigo visual de Configuración) |
| `9be011b` | Hermetismo Ring 0 (Time‑Capsule + Permisos + Swap semántico) |
| `6f05ec3` | Pilar 4: geometría del puntero como contexto inyectado |
| `c7644a3` | CodigoError tipado + Drop con zero‑fill + auditoría de permisos |
| `6cd5b95` | GC semántico (compactador del log direccionado por contenido) |
| `5cd1311` | Tres leyes inmutables (no_std symmetry script + zero‑alloc parcial + panic‑free verificado) |
| `90631ac` | Zero‑alloc completo en `compositor::recomponer` |
| `0e8702c` | Fase 53 — `sys_grafo_compactar` + `PERMISO_COMPACTAR` |
| `930ff22` | Fase 55 — demuxer Akasha cero‑alloc (anillo MTU pre‑alocado) |
| `9c00555` | Fase 56 — `asignar_marcos` aborta vía baliza, no panic |
| `21f332a` | Fase 57 — Alt+G dispara GC manual del grafo |
| `5e967e5` | Fase 58 v1 — Alt+P abre launcher gráfico modal |
| `1c03019` | Fase 58 v2 — ratón modal en el launcher |
| `6aa8228` | Fase 58 v3 — búsqueda por texto en vivo (substring CI) |
| `7d35c4a` | Fase 58 v4 — contador "N/M" en el título del launcher |
| (Fase 58 v5) | match jerárquico del launcher (prefijo > substring > subsecuencia) + selección sticky |
| (Fase 58 v6) | highlight de chars matcheados (Spotlight-classic, máscara u64 + tinta `RESALTE_BUSQUEDA`) |
| (Fase 58 v7) | scroll vertical con auto-track de la selección (PICKER_MAX_FILAS=16) |
| (Fase 58 v8) | `Alt+1..9` quick-launch sobre la fila VISIBLE del filtrado |
| (Fase 58 v9) | `instalar_app(EntradaApp)` — PLANTILLAS mutable + refresh del catálogo |
| (Fase 58 v10) | polling automático del manifiesto: `tarea_compositor` invoca `refrescar_apps_desde_manifiesto` cada ~6 s |
| (Fase 59 v1) | módulo `pantallas` — registro `Once<Mutex<Vec<Output>>>` con N=1, fundado desde el framebuffer del bootloader |
| (Fase 59 v2) | `Ventana::output` + `aplicar_teselado` agrupa por output y tesela cada uno en su `Output::region` |
| (Fase 60) | App `asistente` + arco IA «la máquina propone, el humano firma»: `format::MensajeAsistente`/`CANAL_ASISTENTE`, puente Linux `asistente-puente` (Akasha + `pluma-llm`), `daemon-firma` distingue cuaderno/configuración, firma humana de propuestas hash, cierre del ciclo `RequestFirma → sys_manifiesto_proponer` |
| (Fase 60 gpu) | `drivers/gpu.rs` — virtio‑gpu con scanout propio; `consola::presentar` y `baliza` cruzan la frontera vía `gpu::presentar()`/`presentar_baliza()` cuando el dispositivo está montado |
| `89fd6c4` (Fase 61) | `drivers/tableta.rs` — virtio‑input como tableta: coordenadas ABSOLUTAS por sondeo (no IRQ), comprometidas en cada `EV_SYN` por el sumidero común del ratón. Complementa al PS/2, no lo reemplaza |
| `1f406b3` (Fase 62) | `drivers/sonido.rs` — virtio‑sound: PCM real S16/2ch/44.1 kHz por DMA no‑bloqueante (`pcm_xfer_nb`/`pcm_xfer_ok`), `tarea_sonido` mantiene periodos en vuelo anti‑underrun, mezclador square‑wave con prioridad a la voz del sistema. `sys_tono`/`altavoz` enrutan aquí; bocina del PIT como fallback |
| `95c63cf` | App `rimay` — verbo de embeddings bare‑metal (3.3 KiB .wasm), 14ª de GENESIS |
| (Fase 63) | `wawactl gc` — control remoto del GC sobre virtio-console (`kernel/src/control.rs` + subcomando host). `mirada-layout::outputs` — geometría pura de disposición multi-output (down-payment de multi-monitor) |

## 14. Plan — siguientes hitos

### 14.0 Hitos previamente listados que YA están en el código

Esta sección anota lo que el plan histórico daba por pendiente y el árbol ya
materializa. Si una IA o un humano lee este documento como mapa del trabajo
restante, debe descontar primero estos hitos para no duplicar esfuerzo:

- **Firma criptográfica del manifiesto** — **HECHA** (Fase 41/48). `claves.rs`
  con `AGORA_AUTH_RING: [[u8; 32]; 3]` (tres pubkeys Ed25519 forjadas en la
  ceremonia de la Fase 48, sin placeholders). `verificar_manifiesto_firmado`
  (`claves.rs:151`) + `verificar_cuaderno_firmado` operan zero-alloc sobre
  `ed25519-compact` con `default-features = false`. La syscall
  `sys_manifiesto_proponer` (`wasm/env.rs:1670`) gatea cada propuesta de
  re-ancla contra el anillo; clave pública desconocida ⇒ rechazada antes de
  tocar disco.
- **App `mudanza`** — **HECHA** y sembrada en GENESIS (ver §9). Consume
  `MensajeAkasha::AnunciarCanal`, valida la firma del autor contra el anillo
  vía la syscall, presenta UI de aceptar/rechazar. Verificación crypto en
  **dos niveles** (2026-05-28): (1) userspace — la app parsea el sobre de
  128 B raw y verifica con `ed25519-compact` ANTES del syscall (rechazo local
  con código `-100 = VERIFICACION LOCAL FALLO` sin gastar trap a Ring 0);
  (2) kernel — `claves::verificar_manifiesto_firmado` re-verifica + filtra
  por `AGORA_AUTH_RING`. Los sobres los produce host-side el crate
  [`agora-channel`](../03_ukupacha/agora/agora-channel/) (`firmar_manifiesto`
  + `firmar_para_anuncio`) sobre identidades [`agora-core`](../03_ukupacha/agora/agora-core/).
- **IDE nativo / Notebook engine** — **HECHA por el camino corto**: en lugar
  de portar tree-sitter, se embebió `pluma` (cuaderno reactivo sobre
  `pluma-notebook-core`, núcleo `no_std + alloc` compartido bit a bit con el
  host). Consolidada a 11 KiB post `wasm-opt -Os` (Fase 50). Persistencia
  cerrada por las syscalls `sys_cuaderno_anexar_celda` /
  `sys_cuaderno_leer_celda` / `sys_cuaderno_firmar_y_anclar`. Walker rehidrata
  el grafo entre arranques.
- **GC syscall + permiso** — **HECHA** (Fase 53). `PERMISO_COMPACTAR = 1 << 5`
  añadido en `shared/format`. Syscall `sys_grafo_compactar() -> i32` registrada
  en `wasm/env.rs` (CAPACIDAD 7c) gateada por el bit; cuerpo invoca
  `crate::almacen::compactar()` y retorna `stats.nodos_vivos` (cap `i32::MAX`)
  o `CodigoError::AlmacenamientoFallo`. El compactador automatico del tic
  ocioso del compositor sigue intacto — esta syscall es la palanca
  complementaria para `wawactl gc` / `cronista`.
- **Mouse cursor visible** — **HECHA** (auditoría 2026-05-27). El sprite
  `PUNTERO` (`grafico.rs:431`, flecha NW 18×12, borde + relleno) ya se
  estampa al final de cada recomposición. La cadena es
  `compositor::recomponer → consola::recomponer (consola.rs:310) →
  self.presentar() (consola.rs:490)`; `presentar` invoca
  `Pantalla::estampar_puntero(x, y)` con la posición viva de
  `crate::drivers::raton::posicion()`. El camino parcial
  (`presentar_region`, consola.rs:501) re-estampa el cursor cuando la
  región intersecta el sprite (`region_solapa(region, sprite_puntero_rect)`).
  El cursor vive en framebuffer, no en lienzo (el lienzo HACE de save-under),
  así que la siguiente recomposición lo borra y la siguiente presentación
  lo redibuja — cero artefactos.
- **`wawactl daemon-firma`** — **HECHA** (Fase 39/41/49, auditoría
  2026-05-27). `02_ruway/wawa/wawactl/src/main.rs` (1158 LOC) tiene
  `cmd_daemon_firma` cableado con dos transportes: `--pty <PATH>` (legacy,
  fase 39) y `--char-device <PATH>` (virtio-console, fase 49). Parser de
  ventana deslizante reconoce `PREFIJO_SOLICITUD_VIRTIO =
  b"wawactl::sign_pci::"` + 32 B hash; prompt interactivo
  `[y/N] (timeout 30 s)` vía `tokio::time::timeout(TIMEOUT_CONFIRMACION, ...)`
  sobre `spawn_blocking` para stdin; al aceptar firma con
  `ed25519-compact` la seed del slot indicado y emite 65 B (1 slot id +
  64 firma) por el mismo canal. `chrono` deja marcas de tiempo en el log
  de auditoría.
- **Zero-alloc del demuxer Akasha** — **HECHA** (Fase 55). `COLA_USUARIO`
  pasa de `Mutex<VecDeque<Vec<u8>>>` a un anillo `Mutex<AnilloCola>` de 64
  slots MTU (`SLOT_CAPACIDAD = 2048`) pre-alocados en `.bss` (~128 KiB) +
  dos pistas `[u8; 64]`: FIFO de ocupados + LIFO de libres.
  `encolar_para_usuario` ahora hace `copy_from_slice` directo al slot que
  asoma la pila; `pop_usuario` desencola al `buf` del userspace y devuelve
  el slot a libres. Cero `to_vec()` en RX, cero `push_back` que alocan.
  Invariante mantenida: `fifo_n + libres_n == 64` siempre.
- **Defensa en profundidad para `dma_alloc`** — **HECHA** (Fase 56). La
  back-pressure adversarial estaba cubierta estructuralmente (32 apps × 4
  páginas en vuelo = 128 << 4096 arena, ver `drivers/disco.rs:45` y
  `wasm/env.rs:169`); lo único pendiente era reemplazar los dos
  `.expect()` de `asignar_marcos` por algo legible cuando el bug ocurre.
  Hecho: ambos casos (`ASIGNADOR` no fundado, arena exhausta) ahora
  invocan `baliza::aborto_fatal_carmesi(traza_corta, traza_serial)` —
  pantalla carmesí + traza serial sin recorte + IRQs apagadas. El
  operador ve YA en pantalla qué pasó en lugar de tener que rescatar
  el panic handler del COM1.
- **GC manual desde el teclado** — **HECHA** (Fase 57). `Alt+G` engendra
  `Mando::CompactarGrafo` (nuevo, `compositor.rs:127`); el compositor
  lo atiende en su tic invocando `crate::almacen::compactar()` y emite
  el resultado por la baliza serial:
  `gc :: manual (Alt+G) :: vivos=N muertos=M sectores=A->B`. Palanca
  operacional in-VM que demuestra la cadena tecla → compositor → GC
  end-to-end sin esperar al protocolo host-side de `wawactl gc`. El
  scancode `0x22` se registra en `async_system/teclado.rs:60` como
  `TECLA_G`.
- **Launcher gráfico tipo Spotlight** — **HECHA** (Fase 58, vueltas
  1–10 + polish). `Alt+P` engendra `Mando::ToggleLauncher`; el
  compositor pinta un overlay modal centrado con la lista de apps del
  manifiesto y la roba el foco del teclado y del ratón.
  - Teclado: `Alt+J/K` mueven la selección entre las apps filtradas,
    Enter (con o sin Alt) lanza la resaltada, `Alt+Q` o Escape cierran
    sin lanzar. `Alt+1..9` (v8) lanza directamente la fila VISIBLE
    correspondiente (1..9 → índices 0..=8) — `Mando::LanzarFila(visible)`
    resuelve `launcher_scroll + visible` y dispara la app si la fila
    visible no está vacía; `Alt+0` queda reservado para una eventual
    «fila 10».
  - Ratón: hover resalta una fila, clic izquierdo la lanza, clic fuera
    del overlay cierra sin lanzar.
  - Búsqueda por texto en vivo: escribir letras/cifras/espacio filtra
    el catálogo; Backspace borra el último carácter; la lista se
    recalcula por keystroke (`refiltrar_launcher`).
  - Match jerárquico (v5): `evaluar_match` clasifica cada nombre en
    tres niveles — 3) prefijo, 2) substring contiguo, 1) subsecuencia
    (chars en orden, no necesariamente pegados, estilo Spotlight:
    `plm` matchea `pluma`). Dentro de cada nivel, gana el que tiene
    el primer match más cerca del inicio; en empate, el orden original
    del manifiesto. La selección es *sticky*: tras un refiltrado, si
    la app previa sigue lanzable, el cursor se queda sobre ella
    (backspace ya no tira el cursor al primer item).
  - Highlight de chars matcheados (v6): `evaluar_match` devuelve, junto
    al nivel, una máscara `u64` con un bit a 1 por cada byte de pajar
    que formó parte del match (contigua para nivel 3/2, dispersa para
    nivel 1). `LauncherOverlay::mascaras` viaja paralela a `filtrado` y
    `consola::pintar_etiqueta_resaltada` pinta cada glifo en
    `Color::RESALTE_BUSQUEDA` (ámbar dorado) si su bit está a 1, en
    `Color::TEXTO` si está a 0. Visualiza el "por qué" del match —
    indispensable cuando el catálogo crezca y el nivel 1 produzca
    coincidencias inesperadas. Caracteres más allá del bit 63 se pintan
    en tinta normal (degradación silenciosa).
  - Scroll vertical (v7): `Escritorio::launcher_scroll` es el primer
    índice de `filtrado` que entra al viewport;
    `ajustar_scroll_launcher` empuja el viewport para mantener la
    selección dentro de `[scroll, scroll + PICKER_MAX_FILAS=16)` tras
    cada movimiento de cursor o refiltrado. `LauncherOverlay::{scroll,
    filas_visibles}` viajan al renderer, que itera
    `filtrado[scroll..scroll+filas_visibles]` y compara la selección
    contra el índice ABSOLUTO. El hover/clic del ratón compensa el
    offset: una fila visible 3 con `scroll = 10` resuelve a la app 13
    del filtrado. Para 12 apps el scroll queda en 0 toda la vida — el
    código es invisible hasta que el catálogo crece.
  - Instalación viva (v9): `static PLANTILLAS` pasó de
    `Once<Vec<Plantilla>>` a `Once<Mutex<Vec<Plantilla>>>` para
    permitir mutación post-boot, y `Plantilla` ahora deriva `Clone`
    para clonar antes de instanciar sin anidar locks. Nueva API
    pública `pub fn instalar_app(entrada: &manifiesto::EntradaApp) ->
    Option<usize>` que recupera el bytecode del grafo, construye la
    plantilla, push al Mutex y refresca el catálogo del launcher vía
    `compositor::fijar_catalogo`.
  - Polling automático (v10): `tarea_compositor` incrementa
    `CONTADOR_REFRESCO_APPS` en cada tic; al alcanzar
    `INTERVALO_REFRESCO_APPS=600` fotogramas (~6 s a 100 Hz),
    `refrescar_apps_desde_manifiesto` relee `manifiesto::cargar()` y,
    si el manifiesto del disco tiene más entradas que las plantillas
    vigentes, instala las nuevas con `instalar_app`. Tras un
    `sys_manifiesto_proponer` aceptado (la app `mudanza`), las apps
    nuevas aparecen en `Alt+P` en ≤ 6 s sin reboot. La traza emite
    `launcher :: app instalada en vivo :: idx=N nombre=X` por la
    baliza serial — el operador ve el alta en tiempo real. El
    protocolo NO retira plantillas que desaparezcan del manifiesto
    (eso invalidaría ventanas vivas y queda como política futura).
  - Contador "N/M" (v4) a la derecha de la barra de título: hace
    visible cuándo la query deja cero matches o cuántas apps quedan
    tras filtrar; se pinta en `Color::SIN_FOCO` como información
    subordinada al texto principal (`formatear_contador` en
    `consola.rs`, sin alocación).
  - Pipeline IRQ→compositor sin locks: mirror atómico
    `compositor::LAUNCHER_ABIERTO: AtomicBool` que `recibir_scancode`
    consulta sin tomar el cerrojo del escritorio; si está vivo,
    `traducir_scancode_a_ascii` mapea el make code a un byte ASCII y
    el compositor recibe un `Mando::TextoLauncher(byte)` que absorbe
    dentro del lock.
  - Las altas dirigidas viajan por
    `PARTOS_POR_INDICE: Once<Mutex<Vec<usize>>>`, paralelo al contador
    legacy `PARTOS` de la rotación ciega (`Alt+N` y botón `+` de la
    taskbar siguen vivos). El orquestador
    (`main.rs::tarea_compositor`) drena ambos y resuelve cada índice
    contra `PLANTILLAS[idx]` via `lanzar_app_por_indice`.
  - Geometría compartida `compositor::PICKER_*` + `region_launcher` +
    `consola::pintar_launcher` — un solo origen para alto de fila,
    título, padding inferior y techo de filas visibles (`PICKER_MAX_FILAS
    = 16`).

### 14.1 Hitos genuinamente pendientes (orden de mérito)

1. **`wawactl gc`** — **HECHA (Fase 63).** Subcomando host-side
   complementario a `sys_grafo_compactar` (Fase 53) y a `Alt+G` (Fase 57).
   Reusa el virtio-console ya cableado por `daemon-firma` (Fase 49) con un
   prefijo nuevo `wawactl::gc_request::`; el kernel responde
   `wawactl::gc_reply::vivos=N muertos=M sectores=A->B`. Piezas:
   - Host: `wawactl gc --char-device <PATH>` (`02_ruway/wawa/wawactl`),
     emite el request y espera el veredicto con timeout (default 30 s).
     Parser `extraer_gc_reply` tolera basura intercalada (trazas de boot,
     ecos); 3 tests unitarios.
   - Kernel: `kernel/src/control.rs` — `tarea_consola_control` drena el
     virtio-console cada fotograma y, ante un `gc_request` completo, invoca
     `almacen::compactar()` y responde por el mismo canal. Convive con la
     cadena de firma sin pisarla: el reactor es cooperativo de un núcleo, la
     syscall de firma drena el ring de forma síncrona dentro del tic de la
     app, así que la tarea de control nunca corre a la vez.
   Decisión vs el plan original: se atiende en kernel (kernel-direct, como
   `Alt+G`) en vez de delegar a un Ente userspace con `PERMISO_COMPACTAR` —
   el canal de control host↔kernel ya es privilegiado por construcción
   (quien controla el virtio-console controla la VM). El Ente userspace
   queda como endurecimiento futuro si se quiere la frontera de capacidad.
   **Pendiente de validar en QEMU** (el autor corre la imagen).

2. **Multi-monitor — refactor estructural HECHO, bloqueador físico vigente**
   (Fase 59 v1+v2). El **modelo** ya es N-output: el módulo `pantallas`
   (`pantallas.rs`) mantiene un registro `Once<Mutex<Vec<Output>>>`,
   `Ventana::output` asocia cada ventana a un output, y `aplicar_teselado`
   agrupa por output y tesela cada uno en su `Output::region`. Para N=1
   (el caso vivo hoy) el resultado es idéntico al teselado anterior. El
   **bloqueador físico** sigue siendo `bootloader_api 0.11` que entrega
   `BootInfo.framebuffer: Optional<FrameBuffer>` — UN solo framebuffer.
   Para que `pantallas::registrar` reciba un output adicional con
   geometría real hace falta (a) forkear `bootloader_api` para exponer
   todos los handles GOP que el firmware mantiene, o (b) escribir un
   driver GPU propio (virtio-gpu / PCI) que enumere outputs en runtime.
   **Actualización (Fase 60 + 63):** la opción (b) está a medio camino
   pero el bloqueador se desplazó al CRATE. `drivers/gpu.rs` monta un
   virtio‑gpu y presenta a UN scanout vía `virtio-drivers` 0.13. Auditado
   el crate (Fase 63): `VirtIOGpu` LEE `num_scanouts` de la config pero
   **no lo expone**, y `change_resolution`/`setup_framebuffer`/`set_scanout`/
   `flush` están cableados a un único `SCANOUT_ID`/`RESOURCE_ID_FB`;
   `get_display_info` (que trae el rect por‑scanout) es privado. Conclusión:
   enumerar cabezas reales exige **forkear `virtio-drivers`** (exponer
   `num_scanouts` + `get_display_info` + setup multi‑recurso) o **escribir
   un driver virtio‑gpu nativo**. Ninguno es verificable sin QEMU
   multi‑display.
   Down‑payment hecho (Fase 63): la **matemática pura de disposición** de N
   outputs vive en `mirada-layout::outputs` (`disponer(&[(w,h)], Disposicion)
   -> Vec<Rect>` + `envolvente`), host‑testeada (7 tests) y `no_std`. El día
   que la enumeración exista, traducir cada `Rect` a `RegionPantalla` y
   llamar `pantallas::registrar` es lo único que resta — la geometría ya
   está probada. La capa de software (registro N‑output + teselado por
   output, Fase 59) sigue lista; falta sólo el origen del dato. Limitación
   conocida del MVP: `area_apps` resta la consola y la taskbar globales,
   que en N>1 sólo tienen sentido en el output primario — la decisión
   "taskbar replicada en cada monitor vs sólo en uno" queda para cuando
   haya N>1 que validar.

3. **Tabla de capacidades por bytecode hash**: cuando el manifiesto declare
   `bytecode` por hash, los permisos podrían derivarse de la firma sobre
   `(hash_bytecode, permisos)` en lugar de declararse en `EntradaApp`. Daría
   inmutabilidad real al binding "qué binario puede hacer qué".

---

## 15. Cómo construir y ejecutar

```bash
# Pruebas de format (no_std):
cargo test -p format

# Guardia de simetría no_std (kernel/wasm-side cores):
./scripts/check-shared-cores.sh

# Compilar el kernel bare-metal (sin enlazar la imagen):
cd 03_ukupacha/wawa/wawa-kernel
cargo +nightly check --target x86_64-unknown-none -Z build-std=core,alloc

# Forjar imagen UEFI + ejecutar en QEMU (requiere bindeps + rust-src):
cd 03_ukupacha/wawa
cargo +nightly run -p boot -Z bindeps

# Apps WASM:
cd 03_ukupacha/wawa/apps/<nombre>
cargo build --target wasm32-unknown-unknown --release
# Copiar el .wasm a kernel/assets/ y re-forjar la imagen.
```

Toolchain: nightly con `rust-src`, `wasm32-unknown-unknown`, `x86_64-unknown-none`.
