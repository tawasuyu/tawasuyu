# Init + compositor + launcher — reporte técnico (Linux y wawa)

> Consumidor: IA. Densidad antes que floritura. Estado verificado contra código en `2026-05-27`. Cuando este reporte y `PLAN.md` / `WAWA.md` discrepan, **gana este reporte** (los otros documentos quedaron atrás del código en varios puntos — marcado explícitamente abajo).

---

## 0. Mapa de los dos stacks

gioser tiene **dos pilas paralelas de "lo más bajo"**, una por target:

| Capa | Linux (`*-unknown-linux-gnu`) | wawa (`x86_64-unknown-none`) |
|---|---|---|
| Bootloader | `03_ukupacha/arje/init/arje-loader` (EFI propio) | `bootloader_api` + OVMF (vía crate `bootloader`) |
| Init / PID 1 | `arje-zero` | `wawa-kernel::kernel_main` (no hay PID 1, todo userspace son módulos WASM) |
| Compositor | `mirada-compositor` (smithay/Wayland) | `wawa-kernel::compositor` (framebuffer + `mirada-layout`) |
| Login / greeter | `mirada-greeter` (Llimphi + PAM) | (no aplica — sin multiusuario hoy) |
| Launcher gráfico | `mirada-launcher` (TUI XDG) + `mirada-app-llimphi` shell | `compositor::nacer_ventana` + taskbar con botón `+` (Alt+N) |
| Mecanismo de IPC | `mirada-link` (Unix socket + postcard) | sin IPC: `wasmi::Linker` + `ContextoCapacidades` por bit de permiso |
| Almacenamiento | FS Linux + `arje-cas` (SHA‑256) | `almacen.rs` (log direccionado por BLAKE3 sobre virtio‑blk) |

**Pieza compartida real**: `mirada-layout` (29 LOC, `no_std`, función pura `tile()`) — el mismo motor de teselado corre en `mirada-brain` (Linux) y en `wawa-kernel::compositor` (bare-metal). Es el único crate que cruza la frontera con identidad bit‑a‑bit.

---

## 1. Linux side — `arje` + `mirada`

### 1.1 `arje` (init, 21 crates en `03_ukupacha/arje/`)

Sin SDD; arquitectura embebida en descripciones de Cargo.toml y comentarios. **No hay** `arje/SDD.md`. Tres familias de crates:

**Boot + instalación + empaquetado** (ejecutables reales):
- `arje-loader` — bootloader UEFI propio en `x86_64-unknown-uefi` (369 LOC `main.rs`, `no_std`). Lee `/loader/entries/arje.conf`, carga kernel EFISTUB PE + initramfs + cmdline. Reemplaza systemd-boot.
- `arje-installer` — instalador UEFI-only (317 lib + 531 main LOC). Copia kernel + initramfs + tarjeta semilla a ESP, o formatea USB GPT/ESP booteable. Ejecutable real.
- `arje-packager` — arma initramfs (cpio newc + gzip) booteable a partir de Tarjeta Semilla (347 lib + 287 main). Inyecta binarios declarados en `genesis`, deja `/init` listo para PID 1.

**PID 1 + aislamiento** (ejecutables reales):
- `arje-zero` — PID 1 del "fractal" (905 LOC). Lee un `EnteGraph` desde una `Card` semilla, supervisa hijos con `tokio`, maneja `SIGCHLD`/`SIGUSR2`, expone un socket de introspección y un loop de "autopromote". Compila y arranca.
- `arje-incarnate` — wrapper puro sobre `nix::sched` para `clone()` + namespaces + cgroups + rlimits (419 LOC lib). Reutilizable por cualquier supervisor.
- `arje-soma` — fachada histórica sobre `arje-incarnate` con la API `set_bus_sock + incarnate` que consume `arje-zero` (44 LOC). Indirección de compatibilidad.

**Compatibilidad systemd + utilidades** (MVP funcional):
- `arje-compat` — 15 binarios que implementan shims D-Bus de `systemd1`, `hostnamed`, `logind`, `polkit`, `resolved`, `journald`, `binfmt`, `timedated`, `notify`, `tmpfiles`, `timedated` (171 LOC lib). Acepta llamadas y mapea a efectos internos. Suficiente para que apps que esperan systemd no fallen.
- `arje-getty-stub` — agetty mínimo sin libC, musl estático (100 LOC). Smoke de QEMU.
- `arje-net-bring-up` — sube el primer interfaz no-loopback vía ioctl (188 LOC). Oneshot.
- `arje-absorb` — parsea config de otro init (sysvinit/runit/dinit/openrc) y traduce a Tarjeta Semilla (174 LOC). Ejecutable, una pasada.
- `arje-snapshot` — serializa estado del fractal (ULID + JSON, 61 LOC). MVP mínimo.
- `arje-kernel` — primitivas Linux puras (`become_child_subreaper`, `bootstrap_kernel_surface`, `spawn_sigchld_stream`, `spawn_uevent_stream`) en 11 LOC visibles. Determinista y testeable aislada.
- `arje-card` — re-export histórico de `card-core::EntityCard` (30 LOC). Stub de compatibilidad.

**Runtime del "brain"** (stubs estructurales, lógica mínima):
- `arje-bus` — bus IPC tokio sobre Unix socket con `SO_PEERCRED` (228 LOC). Marco postcard con prefijo de longitud. Testeable contra `arje-echo` (cliente demo, 18 lib + 46 main).
- `arje-brain` — wiring del brain (37 LOC). Expone `IntrospectServer`, autopromote loop, métricas HTTP. La lógica vive en subcrates.
- `arje-brain-rules` — motor determinista *Subject + Event + Action*. **Re-audit 2026-05-27**: la primera auditoría leyó solo `lib.rs` (15 LOC de re-exports) y reportó "stub estructural". La realidad: 825 LOC en 5 archivos — `rules.rs` (216), `engine.rs` (399), `dispatch.rs` (73), `loader.rs` (122). `RuleEngine` con dispatch O(1) por discriminante, `ActionSink` async, loader JSON con tres formas (array, object-con-array, JSONL). Lo que faltaba era *config* (`rules.example.json`, ahora canónico), no código del motor.
- `arje-brain-audit` — audit log con hashes encadenados (13 LOC). Tipos sin tests.
- `arje-brain-cognitive` — observador estadístico (sliding window, entropía de Shannon, info mutua) + "crystallize" para detectar patrones (11 LOC). Tipos puros.
- `arje-cas` — almacén direccionado por contenido (SHA‑256, GC, resolve, store) (120 LOC). Funcional.
- `arje-wasm` — intérprete WASM sobre `wasmi`, carga + valida + ejecuta (153 LOC). Funcional en escala mini.

**Estado:** boot + supervisión + compat con systemd es **real y ejecutable**. El "brain" (reglas + cognición + audit) es **andamiaje** — APIs estables pero apenas cuerpo.

### 1.2 `mirada` (compositor + shell, 13 crates en `02_ruway/mirada/`)

**Compositor real:**
- `mirada-compositor` (**1 398 LOC** en `main.rs`, ejecutable) — compositor Wayland sobre **smithay v0.7**. Dos backends: `winit` (anidado dentro de una sesión gráfica host, útil para dev/tests) y `drm` (nativo sobre TTY sin host). Habla `wl_compositor`, `xdg_shell` (toplevels + popups), `wl_shm`, `wl_seat` (keyboard + pointer en drm), `wl_output`, `wl_data_device`, `xdg-decoration`, `zwp_linux_dmabuf` (clients GPU). Composición con `GlesRenderer`. **Backend winit funciona end-to-end**, **drm parcial** (sin conmutación VT, sin hotplug).

**Lógica del escritorio (agnóstica del compositor):**
- `mirada-brain` (34 LOC visibles + módulos) — el "cerebro": consume `BodyEvent`, produce `BrainCommand`. Mantiene salidas, escritorios virtuales, ventanas, foco. Módulos: `action` (atajos), `ctl` (API socket para `mirada-ctl`), `desktop` (loop determinista), `keymap` (RON, recargable en caliente), `rules` (reglas por `app_id`).
- `mirada-body` (445 LOC) — contabilidad del "cuerpo" (compositor). `BodyState` con salidas + superficies; `apply(BrainCommand) → BodyOp` traduce intenciones a operaciones smithay. Agnóstico de smithay en su API pública — testeable.
- `mirada-protocol` (332 LOC) — enums `BrainCommand` (Place, Close, Kill, GrabKeys, SetCursor, Spawn, Shutdown) + `BodyEvent` (OutputAdded/Removed/Resized, WindowOpened/Closed/Retitled, Keybind, PointerEntered, FullscreenRequested). Marco postcard con prefijo `u32` LE.
- `mirada-link` (252 LOC) — transporte Unix socket con hilo lector en background. `BrainLink` y `BodyLink` simétricos; helpers `connected_pair`, `connect`, `listen`.
- `mirada-layout` (29 LOC, **`no_std`**) — `Workspace { ventanas, foco, modo }` + `tile()` puro. **El único crate de este stack que también vive dentro del kernel wawa.**

**Shell user-facing:**
- `mirada-app-llimphi` (795 LOC ejecutable) — "Cerebro" en Llimphi. Dos modos: **autónomo** (un `Desktop` embebido sin compositor, dev) y **enlazado** (se conecta a `MIRADA_SOCKET`). Pinta HUD y ventanas sintéticas para validar el loop de input/foco sin necesitar GPU.
- `mirada-greeter` (316 LOC ejecutable, **portado a Llimphi 2026-05-25**) — pantalla de login. Usa `llimphi-ui` + `llimphi-widget-text-input` + `auth-core` (PAM real o mock con `MIRADA_GREETER_MOCK=user:pass`). Flujo: usuario + pass → hilo `auth_core::Authenticator` → en éxito imprime `SessionTicket` a stdout → el compositor parsea y muta a sesión sin reiniciar.
- `mirada-launcher` (274 LOC ejecutable) — **TUI, no Llimphi**. Escanea `.desktop` XDG, filtra por escritura interactiva, lanza el elegido. Cero deps. Pensado para vivir adentro de `foot -e mirada-launcher` desde un keybind del compositor.

**Auxiliares:**
- `mirada-ctl` (143 LOC ejecutable) — CLI estilo `swaymsg` / `hyprctl`. `mirada-ctl focus-next`, `workspace 3`, `layout grid`, etc. → socket del brain.
- `mirada-portal` (430 LOC ejecutable) — backend `xdg-desktop-portal` para `org.freedesktop.impl.portal.Settings`. Publica tema activo de `nahual` (claro/oscuro + acento + contraste) a GTK/Qt/Firefox/Chromium. Vigila `~/.config/nahual/theme` con `notify` y emite `SettingChanged` por D-Bus.
- `mirada-bar-core` (108 LOC) — modelo agnóstico de taskbar. Sin deps web.
- `mirada-bar-web` (72 LOC, WASM) — bindings web del taskbar. Aplicación-específica, poco código.

**Sesión típica imaginada (script `session/mirada-session` referenciado en código):**
`login TTY` → `mirada-session` setea `XDG_SESSION_TYPE=wayland` + `XDG_CURRENT_DESKTOP=carmen` → `mirada-compositor --greeter --drm` arranca compositor + greeter Llimphi → user auth → `SessionTicket` por stdout → compositor muta a sesión del user (setuid) → autostart desde `~/.config/mirada/autostart` → keybinds disparan `mirada-ctl` o lanzan apps (incluyendo `mirada-app-llimphi` como shell principal, o `mirada-launcher` para apps XDG).

**Pendiente** Linux side: backend DRM con conmutación VT + hotplug; el "brain" de arje cognitivo está casi vacío. La integración arje‑zero ↔ mirada-session ya pasó del script bash a un Ente del fractal (commit `e4b70cc`).

---

## 2. wawa side — boot + kernel + compositor + apps

### 2.1 `wawa-boot` (host, forja la imagen UEFI)

`03_ukupacha/wawa/wawa-boot/src/main.rs`:

1. Localiza el ELF del kernel compilado para `x86_64-unknown-none` vía `CARGO_BIN_FILE_KERNEL_kernel` (artifact-dependency en `[dependencies.kernel]`).
2. Fusiona kernel + bootloader con `bootloader::UefiBoot::new()` → imagen GPT UEFI + OVMF firmware.
3. **Siembra el disco de objetos** (`target/disk.img`, 32 MiB): lee `kernel/assets/*.wasm`, deduplica por contenido (`BTreeMap<&str, Hash>`), graba un log direccionado por hash BLAKE3 (sector 0 = `SuperBloque`, sector 1+ = registros), y graba el `Manifiesto` de Génesis con el bytecode como hijos. Constante `TECHO_GENESIS = 4 MiB` por app.
4. Lanza QEMU q35 con KVM/TCG, 256 MiB, `-drive` raw (UEFI + disco virtio-blk-pci), `-netdev user` con virtio-net-pci.

**BIOS legacy**: no explícito en el código, pero la crate `bootloader` cubre ambos modos transparentemente si se le pide.

### 2.2 GENESIS — apps sembradas hoy (**12, no 10** como dice `WAWA.md §9`)

Verificado contra `wawa-boot/src/main.rs:137` (`const GENESIS: [AppGenesis; 12]`):

| Nombre | .wasm | Región (x,y,w,h) | Fuel | Permisos | Estado |
|---|---|---|---|---|---|
| bitacora | bitacora.wasm | (100,120,480,280) | **FUEL_EDITOR (6M)** | 0 | ✓ Editor persistente |
| pregon | pregon.wasm | (100,120,480,160) | FUEL_COMUN | RED | ✓ Primera voz a la red |
| tonada | tonada.wasm | (100,120,360,120) | FUEL_COMUN | ALTAVOZ | ✓ Demo PC speaker |
| pulso | pulso.wasm | (100,120,360,120) | FUEL_COMUN | 0 | ✓ Compás visual |
| hola | app.wasm | (100,120,480,560) | FUEL_COMUN | 0 | ✓ Cuadrado por teclado |
| memoriosa | memoriosa.wasm | (700,120,360,80) | FUEL_COMUN | 0 | ✓ Persistencia inter-arranque |
| discola | discola.wasm | (60,700,360,80) | FUEL_COMUN | 0 | ✓ Demo SinCombustible |
| glotona | glotona.wasm | (460,700,360,80) | FUEL_COMUN | 0 | ✓ Demo SinMemoria |
| cronista | cronista.wasm | (860,700,360,80) | FUEL_COMUN | GRAFO_ESCRITURA \| RAIZ | ✓ Cuenta arranques |
| tonalero | tonalero.wasm | (700,220,480,300) | FUEL_COMUN | CONFIG | ✓ Testigo Configuración |
| **mudanza** | mudanza.wasm | (60,220,480,240) | FUEL_COMUN | RAIZ | ✓ Re-anca manifiesto firmado (no en WAWA.md §9) |
| **pluma** | pluma.wasm | (160,60,480,400) | **FUEL_EDITOR (6M)** | GRAFO_ESCRITURA | ✓ Notebook 11 KiB tras `wasm-opt -Os` (no en WAWA.md §9) |

`03_ukupacha/wawa/apps/` también contiene `ide/` (no en GENESIS hoy — disponible para Alt+N).

### 2.3 `wawa-kernel` — boot del kernel paso a paso

`kernel_main` (entry via `bootloader_api::entry_point!`) ejecuta en orden:

1. **GDT / TSS** (con stacks separados para double-fault y NMI).
2. **IDT** con excepciones CPU + handler de doble fallo.
3. **PIC 8259** remapeado + **PIT 100 Hz** como timer del reactor.
4. **Heap dinámico** (`linked_list_allocator`).
5. **Framebuffer dual-buffer** + raster con `fontdue`.
6. **Drivers PCI** → `virtio-blk-pci` (`drivers/disco.rs`) → `virtio-net-pci` (`drivers/red.rs`) → PS/2 mouse (`drivers/raton.rs`, IRQ12) → PC speaker.
7. **`almacen::init()`** monta el disco y reconstruye el índice del log entre `[log_inicio, cursor)`.
8. **`manifiesto::cargar()`** lee el `Manifiesto` vivo desde el grafo.
9. **Reactor cooperativo** (`async_system/executor.rs`) — tareas `Future<Output=()>` despertadas por wakers desde IRQ + reloj PIT.
10. `compositor::tarea_compositor` y `lanzar_app` se programan al reactor; el kernel cede.

**Compila hoy** con `cargo +nightly check --target x86_64-unknown-none -Z build-std=core,alloc` (verificado por la auditoría).

### 2.4 Compositor del kernel (`compositor.rs`)

- **Teselado**: `mirada_layout::tile()` (modo `MasterStack` por default, conmutable con **Alt+M**). El lazo crítico `recomponer` es **zero-alloc**: `Escritorio` retiene `capas_buf` y `celdas_buf` (`Vec::with_capacity(MAX_VENTANAS=32)`) reutilizados con `clear() + push()`. Reloj formateado en pila (`[u8; 8]`).
- **Capa flotante** separada de la teselada, con orden-Z. Ventanas cerrables en vivo (Alt+Q marca slot como inerte).
- **Taskbar** (Fase 14–16, 40 px de alto): pestaña por app + **botón lanzador `+`** a la izquierda (36 px ancho) + reloj monótono a la derecha. Click en pestaña enfoca; click en `+` lanza la siguiente plantilla.
- **Ratón PS/2** (`drivers/raton.rs`, IRQ12): paquetes de 3 bytes → `compositor::atender_raton` → foco/arrastre → `puntero::enrutar` con descuento del origen del marco. Eventos fuera del lienzo natural de la app enfocada se descartan en silencio (4.3 del WAWA.md, geometría como contexto inyectado).
- **Cursor visible**: `consola::estampar_puntero` (Fase 13) ya está cosida al camino zero-alloc — `Consola::recomponer` → `presentar()` → `pantalla.estampar_puntero(x, y)`, y `presentar_region` re-estampa si el sprite intersecta. El doc previo decía "pendiente" pero el código lo desmiente (mismo patrón que la firma del manifiesto).

### 2.5 Launcher de wawa

Hoy hay **dos rutas** al userspace:

1. **GENESIS-only en boot**: las 12 apps nacen al inicializar el manifiesto.
2. **Alt+N → rotación ciega**: instancia la siguiente plantilla de `PLANTILLAS` (mismo bytecode, índice de app nuevo, fotograma propio). El botón `+` de la taskbar dispara lo mismo. Útil para devs.
3. **Alt+P → launcher gráfico (Fase 58)**: overlay modal centrado con la lista de apps del manifiesto + **búsqueda por texto en vivo con match jerárquico**. **Teclado**: escribir letras/cifras/espacio filtra el catálogo; Backspace borra el último carácter, Escape cierra; `Alt+J`/`Alt+K` mueven la selección entre las apps filtradas (ciclando); Enter (con o sin Alt) lanza la app resaltada; `Alt+Q` cierra sin lanzar. **Ratón**: hover resalta la fila, clic-izquierdo lanza, clic fuera cancela. Mientras está abierto el launcher se queda con el foco del teclado Y del ratón (ningún evento llega a las ventanas). Pipeline IRQ→compositor: el manejador de IRQ1 lee un mirror atómico `LAUNCHER_ABIERTO: AtomicBool` y, si está activo, traduce el scancode a ASCII minúscula vía `traducir_scancode_a_ascii` y empuja `Mando::TextoLauncher(byte)`; el compositor consume el mando dentro del lock del escritorio y refiltra. Las altas dirigidas viajan por `PARTOS_POR_INDICE: Once<Mutex<Vec<usize>>>` y el orquestador las drena tras los partos por rotación. **Match (v5)**: tres niveles, mejor primero — 3) prefijo, 2) substring contiguo, 1) subsecuencia (chars en orden, no necesariamente pegados — estilo Spotlight: `plm` matchea `pluma`). Dentro de cada nivel, gana el que tiene el primer match más cerca del inicio del nombre; en empate, el orden original del manifiesto. La selección es *sticky*: tras un refiltrado, si la app previa sigue en el listado, el cursor se queda sobre ella (backspace no reinicia al principio). **Highlight (v6)**: cada char que formó parte del match se pinta en `Color::RESALTE_BUSQUEDA` (ámbar dorado) y el resto en `Color::TEXTO` — `evaluar_match` devuelve `(tier, mask: u64)` con un bit a 1 por byte matcheado en el nombre; `pintar_etiqueta_resaltada` consume la máscara y elige tinta por glifo. Para nivel 3/2 los bits son contiguos; para nivel 1 (fuzzy), dispersos. **Scroll (v7)**: `Escritorio::launcher_scroll` es el primer índice de `filtrado` visible; `ajustar_scroll_launcher` empuja el viewport para mantener la selección dentro de `[scroll, scroll + PICKER_MAX_FILAS=16)`. Hover/clic con ratón compensan el scroll. **Atajos numéricos (v8)**: `Alt+1..9` lanza directamente la N-ésima fila VISIBLE — `Mando::LanzarFila(visible)` resuelve `scroll + visible` y dispara la fila correspondiente, o silencio si la fila visible está vacía. **Instalación viva (v9)**: `kernel::instalar_app(EntradaApp)` recupera bytecode del grafo, append a `PLANTILLAS` (ahora `Once<Mutex<Vec<Plantilla>>>` para permitir mutación post-boot) y refresca el catálogo del launcher vía `compositor::fijar_catalogo`. La API existe; falta cablearle un consumidor automático (mudanza tras re-ancla, cronista). MVP feo: query acotada a 32 chars, match acotado a 64 chars del nombre (los del manifiesto andan por 9), `Alt+0` reservado para una eventual fila 10.

Acceso a apps fuera de GENESIS hoy requiere primero introducir su bytecode al grafo (vía Akasha o `cronista`-style) y añadir su `EntradaApp` al manifiesto vivo — no hay UI de instalación en kernel.

### 2.6 Sistema de apps WASM

- Cada app es un `cdylib` WASM con `init()` + `tick()` exportados, `#![no_std]`, panic handler propio.
- En el arranque, `enlazar_capacidades(linker, permisos)` (`wasm/env.rs`) registra cada syscall **gateada dentro de `if permisos & PERMISO_X != 0`**. **Las capacidades NO registradas son símbolos INEXISTENTES**: wasmi rechaza el módulo en `instantiate_and_start` antes de ejecutar nada. Permisos son **frontera física**, no tabla de despacho.
- **Drop limpio** (`AplicacionWasm::drop`): `teclado::cerrar_canal(indice)` + `puntero::cerrar_canal(indice)` + zero-fill de los 4 MiB de memoria lineal. El siguiente owner no encuentra residuos.
- ABI de 22 syscalls documentado en `WAWA.md §6` (verificado contra `wasm/env.rs`).

### 2.7 Almacenamiento + red

- **`almacen.rs`** — log direccionado por contenido. `almacenar(datos, hijos) → Result<Hash>` (append + dedup + persiste superbloque); `recuperar(hash) → Option<Objeto>` (reverifica). **GC semántico (Fase 24)** vivo: `compactar()` corre MARK → SWEEP → SWAP en una sola escritura atómica del superbloque. Trigger: `ESCRITURAS_DESDE_GC > UMBRAL_GC=32` en el tic ocioso del compositor. Emite traza serial `gc :: compactado :: vivos=N muertos=M sectores=A->B`.
- **`akasha.rs`** — demux capa-2 sobre EtherType propio. Procesa `SolicitarObjeto` / `ProveedorObjeto` (con re-hash) / `AnunciarRaiz` (faro) / `AnunciarCanal` (firmado). Dedup `(MAC, hash)` con ventana TOCTOU-safe. **El kernel no verifica firmas** — ingresa el DAG y traza; toda política vive en userspace (app `mudanza`).

### 2.8 Firma criptográfica (estado real)

- `wawa-kernel/src/claves.rs` con `AGORA_AUTH_RING: [[u8; 32]; 3]` (3 pubkeys Ed25519 forjadas en ceremonia Fase 48).
- `verificar_manifiesto_firmado` + `verificar_cuaderno_firmado` zero-alloc sobre `ed25519-compact` con `default-features = false`.
- Syscall `sys_manifiesto_proponer` gatea cada propuesta de re-ancla contra el anillo; pubkey desconocida → rechazada antes de tocar disco.
- `apps/mudanza` ya invoca la syscall (rechaza zero-key correctamente).
- `WAWA.md §14.0` reconcilia esto desde el commit `d2d84c1` (Fase 51) — el doc ya no engaña.

### 2.9 Estado de tests

```
cargo test -p format                       ✓ 20/20 (incl. test vanguard de estabilidad de CodigoError)
./scripts/check-shared-cores.sh            ✓ 5/5 cores no_std (format, akasha, mirada-layout, forth-emisor, pluma-notebook-core)
cargo +nightly check --target x86_64-...   ✓ kernel compila
cargo +nightly run -p boot -Z bindeps      △ Boot al QEMU funciona; el audit reportó una falla intermitente en el lado de la crate `bootloader` que conviene re-verificar antes de cada release
```

`grep -rE "\.unwrap\(\)|\.expect\(|panic!|unreachable!"` en `wasm/`, `almacen.rs`, `manifiesto.rs`, `akasha.rs`: **0 ocurrencias**. Los panics existentes en otros módulos están confinados (init de fuente, DMA arena con limitación del trait `virtio-drivers::Hal`, executor con `TaskId` duplicado lógicamente imposible).

---

## 3. Paralelos arquitectónicos (Linux ↔ wawa)

| Concepto | Linux (mirada/arje) | wawa | Comentario |
|---|---|---|---|
| Aislamiento | namespaces + cgroups + rlimits (`arje-incarnate`) | jaula wasmi + fuel + techo memoria (4 MiB) | wawa es más estricto: permisos son símbolos físicamente ausentes. |
| IPC | Unix socket + postcard (`mirada-link`, `arje-bus`) | `wasmi::Linker` + `ContextoCapacidades` | wawa no tiene IPC inter-app; toda comunicación es vía grafo BLAKE3 + Akasha. |
| Layout de ventanas | `mirada-layout::tile()` (29 LOC, `no_std`) | `mirada-layout::tile()` mismo crate | **Bit-exacto cross-target.** |
| Identidad | PAM (`mirada-greeter` + `auth-core`) | sin multiusuario hoy; sólo `AGORA_AUTH_RING` de pubkeys autorizadas para re-anclar manifiesto | Conceptos distintos: Linux autentica humanos, wawa autoriza re-anclas de raíz. |
| Almacenamiento | FS Linux + `arje-cas` (SHA-256, GC) | `almacen.rs` log BLAKE3 + GC mark/sweep/swap | wawa es estructuralmente más simple: un único log, un único superbloque, dedup gratis. |
| Launcher | `mirada-launcher` (TUI XDG) + `mirada-app-llimphi` | Alt+N + botón `+` taskbar (ciclo sobre PLANTILLAS) | wawa no tiene buscador todavía. |

---

## 4. Plan propuesto — orden de ejecución

### 4.1 Inmediato (alta señal, bajo riesgo)

1. ~~**Actualizar `WAWA.md` §9 + §14**~~ — ✓ cerrado. `WAWA.md §9` ya lista las 12 apps reales (commit `d2d84c1`, Fase 51), `§14.0` reconcilia firma + cursor + GC + Akasha zero-alloc + DMA exhaustion + Fase 58 launcher contra el código. La afirmación "el doc engaña al lector" del reporte original era ella misma stale; este commit la cierra.
2. ~~**Cursor visible en wawa**~~ — **revisado 2026-05-27**: ya estaba integrado al camino zero-alloc desde la Fase 13 (`Consola::recomponer → presentar → pantalla.estampar_puntero`). Misma drift que con la firma del manifiesto: el doc decía "pendiente" sin verificar contra el código.
3. **`wawactl gc` subcomando**: ✓ cerrado en Fase 53 (`sys_grafo_compactar` + `PERMISO_COMPACTAR = 32`) y Fase 57 (`Alt+G` operacional).

### 4.2 Cierre del shell de Linux

4. **`mirada-compositor --drm` con conmutación VT + hotplug.** Hoy el backend DRM es parcial; sin VT-switch no es usable como compositor primario en hardware real. Smithay tiene los hooks (`udev`, `libinput`); falta el cableado. Estimado: 3–5 sesiones.
5. ~~**`arje-zero` ↔ `mirada-session` como Ente del fractal**~~ — ✓ cerrado (commit `e4b70cc`, "feat(arje-zero): mirada-session como Ente del fractal — adiós al bash de sesión"). El script bash dejó de ser pieza obligada; el loop init → compositor pasa por un Ente declarado en la Tarjeta Semilla.
6. ~~**Llenar `arje-brain-rules` y `arje-brain-cognitive`**~~ — **revisado**: el motor `arje-brain-rules` está completo (825 LOC); lo que faltaba era config. `rules.example.json` canónico shipped 2026-05-27. `arje-brain-cognitive` (sliding window, entropía de Shannon, "crystallize") sigue siendo andamiaje a llenar — requeriría ejemplos de patrones cognitivos del dominio, open-ended.

### 4.3 Cierre del shell de wawa

7. **Launcher gráfico en wawa** (Spotlight-like): vueltas 1–9 shipped en Fase 58 — overlay modal `Alt+P`, navegación Alt+J/K + Enter, ratón completo (hover + clic + clic-fuera-cancela), búsqueda por texto en vivo, contador "N/M" en el título, **match jerárquico v5** (prefijo > substring > subsecuencia, selección sticky), **highlight v6** de los chars matcheados, **scroll vertical v7** con auto-track de la selección, **atajos numéricos v8** (`Alt+1..9` lanza la fila visible N), e **instalación viva v9** (API `kernel::instalar_app` que recibe `EntradaApp`, append a PLANTILLAS y refresca el catálogo). Falta: consumidor automático de `instalar_app` (mudanza tras re-ancla del manifiesto, cronista). Paridad funcional con Spotlight conseguida en los inputs; lo que queda es el cableado de origen.
8. **Multi-monitor / resolución dinámica**: `bootloader_api::FrameBufferInfo` ya entrega geometría real; consola y compositor asumen framebuffer único. Capa `Pantalla` extendida. Estimado: 2 sesiones.
9. ~~**Zero-alloc del demuxer Akasha**~~ — ✓ cerrado en Fase 55 (commit `930ff22`). `COLA_USUARIO` pasó de `Mutex<VecDeque<Vec<u8>>>` a un anillo `AnilloCola` de 64 slots MTU (`SLOT_CAPACIDAD = 2048`) pre-alocados en `.bss` + dos pistas `[u8; 64]` (FIFO ocupados / LIFO libres). `encolar_para_usuario` hace `copy_from_slice` directo; cero `to_vec()` en RX.

### 4.4 Convergencia (largo plazo)

10. **Compositor wawa expuesto por protocolo `mirada`**: si wawa habla `BrainCommand`/`BodyEvent` (postcard sobre Akasha o sobre un socket virtual), entonces `mirada-ctl`, `mirada-app-llimphi`, etc. funcionan idénticos en ambos lados. **Esta es la conclusión natural** de tener `mirada-layout` ya compartido. Riesgo: requiere que el `Linker` exponga un "socket" virtual a las apps que quieran hablar el protocolo, sin romper el modelo de capacidades. Estimado: 5–10 sesiones.
11. **Tabla de capacidades por hash de bytecode**: hoy permisos vienen de la `EntradaApp` (texto del manifiesto). Si los permisos se derivan de la firma sobre `(hash_bytecode, permisos)` en lugar de declararse en el manifiesto, se vuelve inmutable: un binario dado SIEMPRE tiene los mismos permisos, sin importar quién lo instala. Estimado: 3–5 sesiones (requiere extender format + claves).
12. **DM real de mirada en hardware**: PLAN.md §5 anota "shell completo + DM en hardware real (Artix laptop con GPU física)". Hoy el ciclo se valida en winit anidado. Hito final del Linux side.

### 4.5 Dependencias (DAG)

```
  4.1.1 ✓ actualizar doc       ──┐ (cerrado)
  4.1.2 ✓ cursor wawa  ──────────┤ (era stale del doc, no del codigo)
  4.1.3 ✓ wawactl gc syscall   ──┤ (Fase 53 + 57; host-side CLI pendiente)
                                 │
  4.2.4 drm hotplug ─┐           │
  4.2.5 ✓ arje session ┴──> 4.2.6 brain reglas
                                 │
  4.3.7 ✓ launcher grafico ──> 4.3.8 multi-monitor (bloqueado por bootloader)
  4.3.9 ✓ akasha zero-alloc (Fase 55)
                                 │
                     4.4.10 protocolo mirada compartido
                     4.4.11 caps por hash bytecode
                     4.4.12 DM hardware real
```

Vivos: 4.1.3 host-side CLI, 4.2.4 DRM hotplug, 4.2.6 brain cognitivo, 4.3.8 multi-monitor (estructuralmente bloqueado, ver `WAWA.md §14.1.2`), §4.4 entero.

---

## 5. Métricas resumidas

| Pieza | LOC | Estado | Bloquea a |
|---|---|---|---|
| arje (init + supervisión) | ~3 500 funcional + ~1 000 stub | MVP funcional | DM hardware real (4.4.12) |
| arje (brain cognitivo) | ~50 LOC visibles | Andamiaje | Auto-recovery del fractal (4.2.6) |
| mirada-compositor | 1 398 LOC | MVP winit + DRM parcial | Sesión productiva Linux (4.2.4) |
| mirada-app-llimphi | 795 LOC | Funcional autónomo + enlazado | Shell user-facing por defecto |
| mirada-greeter | 316 LOC | Funcional (Llimphi + PAM) | — |
| mirada-launcher | 274 LOC | TUI XDG funcional | Launcher gráfico (4.3.7) |
| wawa-kernel | ~5 000+ LOC | V1.0.0-GOLD operacional | Multi-monitor (4.3.8), launcher gráfico (4.3.7) |
| wawa-boot | ~300 LOC main | Funcional | — |
| apps WASM GENESIS | 12 apps, ~30 KiB sumadas | Todas funcionan | — |
| `mirada-layout` | 29 LOC `no_std` | Compartido bit-exacto Linux/wawa | Protocolo mirada cross-target (4.4.10) |

---

## 6. Hitos cerrados que conviene saber

- **2026-05-25**: `mirada-greeter` portado de GPUI a Llimphi.
- **2026-05-26**: GPUI declarado extinto en todo el repo. Toda la migración pluma/dominium/cosmos/nahual/nakui empieza/termina ese día.
- **2026-05-26 (wawa)**: Canal de release firmado (Ed25519 + `AGORA_AUTH_RING`), `mudanza` consumiendo `AnunciarCanal`, pluma embebida como app de GENESIS (11 KiB post `wasm-opt -Os`).
- **2026-05-27**: smoke `cargo check --workspace` fix-up tras detectar que tres crates Android dependían de `log` solo bajo `cfg(target_os = "android")` — ahora `log` está en deps incondicionales en `02_ruway/llimphi/android/*`.
- **2026-05-27 (wawa Fase 58)**: launcher gráfico vueltas 1–9 — `Alt+P` abre overlay modal con teclado + ratón modales, búsqueda por texto en vivo, Escape/Backspace soportados sin Alt. Cola `PARTOS_POR_INDICE` paralela a la legacy de rotación. Mirror atómico `LAUNCHER_ABIERTO` para que el IRQ del teclado decida sin tomar locks. Contador "N/M" en la barra de título (v4). **Match jerárquico v5**: prefijo > substring > subsecuencia (estilo Spotlight, `plm` → `pluma`), con selección *sticky* a través de refiltrados — `evaluar_match` puro reemplazó al `contiene_ascii_ci` lineal de v3. **Highlight v6**: `evaluar_match` ahora devuelve también una `mask: u64` con un bit por carácter matcheado del nombre; el overlay pinta esas letras en `Color::RESALTE_BUSQUEDA` (ámbar dorado) vía `pintar_etiqueta_resaltada`. **Scroll v7**: `launcher_scroll: usize` define el primer índice visible; `ajustar_scroll_launcher` auto-tracks la selección dentro del viewport `[scroll, scroll + PICKER_MAX_FILAS)`. Hover/clic con ratón compensan el scroll para que un clic en la fila visible 3 con scroll=10 resuelva a la app 13 del filtrado. **Atajos numéricos v8**: `Alt+1..9` emite `Mando::LanzarFila(visible)`; el compositor resuelve `scroll + visible`, dispara `PARTOS_POR_INDICE` y cierra el launcher. **Instalación viva v9**: `PLANTILLAS` pasa de `Once<Vec<Plantilla>>` a `Once<Mutex<Vec<Plantilla>>>` para permitir append post-boot. Nueva `pub fn instalar_app(entrada)` recupera bytecode del grafo, construye Plantilla, push al Mutex, y llama a `compositor::fijar_catalogo` con los nombres actualizados — el launcher refleja la nueva app de inmediato sin reboot. La API es pública y verde; falta cablear un consumidor automático (mudanza al re-anclar, cronista al añadir EntradaApp).
