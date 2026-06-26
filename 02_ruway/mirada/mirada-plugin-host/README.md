# mirada-plugin-host

Un **Cerebro de mirada hecho de plugins WASM**. Se conecta al Cuerpo
(`mirada-compositor`) por `MIRADA_SOCKET` como un cerebro más —igual que
`mirada-app-llimphi`, pero sin UI— y embebe un `mirada_brain::Desktop`
autoritativo (foco, atajos, reglas, multi-monitor) que los plugins **aumentan**,
nunca reemplazan. Un plugin roto, lento o no confiable jamás tumba al escritorio:
está sandboxeado por `wasmi`, con fuel acotado y capacidades gateadas por
importación.

```
   clientes Wayland          MIRADA_SOCKET
        │                        │
   ┌────▼─────────┐   eventos ┌──▼───────────────────────────┐
   │   Cuerpo     │──────────▶│  mirada-plugin-host (Cerebro) │
   │ (compositor) │           │  ┌─────────┐   ┌───────────┐  │
   │  posee GPU/  │◀──────────│  │ Desktop │ + │  plugins  │  │
   │  DRM/input   │  comandos │  │ (manda) │   │ (aumentan)│  │
   └──────────────┘           │  └─────────┘   └───────────┘  │
                              │        Conductor (arbitra)    │
                              └───────────────────────────────┘
```

El **Conductor** es lo único que decide qué se manda al Cuerpo. El `Desktop`
sigue siendo autoritativo: un plugin nunca puede suprimir el comando de otro ni
corromper el estado de ventanas.

## Dos tipos de plugin

| Tipo        | Export WASM        | Qué hace                                                            | Frontera |
|-------------|--------------------|--------------------------------------------------------------------|----------|
| **Layout**  | `mirada_tile`      | Función pura de teselado: recibe ventanas + área útil, devuelve rects | **No importa nada del host** → cero superficie |
| **Reactor** | `mirada_on_event`  | Reacciona a cada `BodyEvent` y emite comandos por un `Ctx`          | Importa funciones del host **gateadas por capacidad** |

- Entre varios plugins de **layout**, gana el de mayor `priority` (rol
  singleton); el resto queda inactivo. El layout recibe los `LayoutParams` que el
  `Desktop` usaría en cada salida, así los atajos del usuario (crecer maestra, nº
  de maestras, gap) siguen gobernando el teselado aunque lo dibuje el plugin.
- Los **reactores** se acumulan: todos ven cada evento. Sus `GrabKeys` se **unen**
  (no se pisan) con los del `Desktop` y entre sí.

## Capacidades

Cada capacidad gatea una importación del host. **Si el bit no se concede, la
función no se registra en el linker y un módulo que la importe ni instancia** —
es una frontera física, no un chequeo de runtime eludible (espejo del bitfield
`Permisos` del kernel wawa).

| Nombre en el manifest | Importación host       | Permite                                          |
|-----------------------|------------------------|--------------------------------------------------|
| `layout`              | *(ninguna)*            | Ser el plugin de teselado (`mirada_tile`)        |
| `spawn`               | `host_emit_spawn`      | Lanzar programas (`sh -c`)                        |
| `window_control`      | `host_emit_close/kill` | Cerrar o matar ventanas                           |
| `keys`                | `host_emit_keys`       | Registrar atajos globales (se unen a los del Desktop) |
| `decor`               | `host_emit_decor/cursor` | Fijar decoración de ventana y cursor del puntero |
| `effects`             | `host_emit_effects`    | Efectos por ventana: opacidad + sombra (atenuar según foco, …) |
| `actions`             | `host_emit_action`     | Pedir **acciones de escritorio** al `Desktop` (foco, teselado, escritorios…) |

`host_log` (diagnóstico) está siempre disponible, sin capacidad.

`actions` es la que convierte a un reactor de **observador** en **gestor de
ventanas**: pide acciones por su forma textual estable —`"layout:monocle"`,
`"workspace:3"`, `"focus-next"`, `"swap-master"`…— el mismo vocabulario del
keymap y `mirada-ctl`. El `Conductor` la parsea y la aplica al `Desktop` como si
fuera un atajo del usuario, manteniendo el estado consistente.

## Escribir un plugin

Los plugins se escriben contra `mirada-plugin-sdk`, que da la ABI y los traits.
Un crate de plugin compila a `wasm32-unknown-unknown`, es `#![no_std]` y vive
**fuera del workspace raíz** (tiene su propio `[workspace]`).

### Layout — una función pura de teselado

```rust
#![no_std]
extern crate alloc;
use alloc::vec::Vec;
use mirada_plugin_sdk::{export_layout_plugin, LayoutPlugin, Rect, TileInput, WindowId};

#[derive(Default)]
struct MiLayout;

impl LayoutPlugin for MiLayout {
    fn tile(&mut self, input: &TileInput) -> Vec<(WindowId, Rect)> {
        // input.ids   = ventanas teseladas, en orden
        // input.work  = área útil de la salida (px)
        // input.params= modo/ratio maestra/nº maestras/gap del Desktop
        // Devolvé un rect por id; los que no devuelvas conservan su geometría.
        todo!()
    }
}

export_layout_plugin!(MiLayout::default());
```

Un layout **no activa la feature `reactor`** del SDK: así no enlaza ninguna
importación y su frontera es de cero superficie. En el manifest pide sólo
`caps: ["layout"]` — y al no pedir capacidades peligrosas, **no requiere firma**.

Ver el ejemplo completo: `mirada-plugin-example-layout` (right-master, honra los
`LayoutParams`).

### Reactor — reacciona y maneja ventanas

```rust
#![no_std]
extern crate alloc;
use mirada_plugin_sdk::{export_reactor_plugin, BodyEvent, Ctx, ReactorPlugin};

#[derive(Default)]
struct MiReactor { ventanas: usize }

impl ReactorPlugin for MiReactor {
    fn on_event(&mut self, event: BodyEvent, ctx: &mut Ctx) {
        ctx.grab_keys(&["Super+a"]);                 // CAP_KEYS (idempotente)
        match event {
            BodyEvent::Keybind(k) if k == "Super+a" => ctx.spawn("foot"), // CAP_SPAWN
            BodyEvent::WindowOpened { .. } => {
                self.ventanas += 1;
                // CAP_ACTIONS: despejar cuando hay multitud
                ctx.act(if self.ventanas >= 3 { "layout:monocle" } else { "layout:master-stack" });
            }
            BodyEvent::WindowClosed { .. } => { self.ventanas = self.ventanas.saturating_sub(1); }
            _ => {}
        }
    }
}

export_reactor_plugin!(MiReactor::default());
```

El reactor **sí activa la feature `reactor`** (`mirada-plugin-sdk = { …,
features = ["reactor"] }`). Cada método de `Ctx` está respaldado por una
importación gateada: usar uno cuya capacidad no se concedió hace que el módulo ni
instancie.

Ver el ejemplo completo: `mirada-plugin-example-reactor` (terminal + dimming por
foco + auto-teselado).

### La ABI, por si la necesitás cruda

El host y el guest cruzan memoria por dos búferes estáticos reusados (el guest es
mono-hilo). Los macros `export_*_plugin!` cablean todo esto; sólo importa si
escribís el plugin en otro lenguaje:

- `alloc(len: u32) -> u32` — el host reserva y escribe el input (postcard).
- `mirada_tile(ptr, len) -> u64` — devuelve `(out_ptr << 32 | out_len)` con los
  rects (postcard). `0` = sin cambios.
- `mirada_on_event(ptr, len)` — sin retorno; los comandos salen por las
  importaciones `mirada_host::host_emit_*`.
- Cada llamada corre con fuel acotado: un plugin desbocado trampa en vez de
  congelar el escritorio.

## Firmar e instalar

### Manifest `.ron`

Cada plugin se declara con un `.ron` junto a su `.wasm`:

```ron
(
    wasm: "mi-plugin.wasm",
    kind: Reactor,                 // o Layout
    caps: ["keys", "spawn", "actions"],
    priority: 0,                   // mayor gana el rol singleton de layout
    signer: "ed25519:…",           // requerido si pide caps peligrosas
    signature: "…",                // hex de 64 bytes sobre blake3(wasm) ‖ caps
)
```

Las `caps` del manifest son una **declaración**; el host las verifica contra las
importaciones reales del `.wasm` al cargar (fail-closed). Pedir de menos rechaza
el módulo; pedir de más no concede nada que el módulo no importe.

### Confianza

Un plugin que pide **cualquier capacidad más allá de `layout`** requiere una
**firma de una clave de confianza**. El anillo sale de `<dir>/trust.ron`:

```ron
( trusted: [ "ed25519:…tu_pubkey…" ] )
```

Sin firma válida de una clave del anillo, un plugin con capacidades peligrosas
**no carga** (fail-closed). Genera tu clave y firma con `mirada-plugin-sign`:

```bash
# Genera un par Ed25519; guarda la semilla y muestra la pubkey para trust.ron.
cargo run -p mirada-plugin-host --bin mirada-plugin-sign -- keygen --out mi.seed

# Firma blake3(wasm) ‖ caps; pega las líneas signer:/signature: en el .ron.
cargo run -p mirada-plugin-host --bin mirada-plugin-sign -- \
    sign --seed mi.seed --wasm mi-plugin.wasm --caps keys,spawn,actions
```

### Directorio y hot-reload

Por defecto el host lee `$XDG_CONFIG_HOME/mirada/plugins`
(`~/.config/mirada/plugins`), o `$MIRADA_PLUGINS` si está puesto. Deja ahí los
`.wasm` + `.ron` (y un `trust.ron`).

**Se recarga en caliente:** agregar, editar o quitar un plugin se aplica sin
reiniciar el host —re-reparte roles preservando el estado de ventanas y re-tesela
al instante— igual que el hot-reload del keymap/config/permisos.

## Construir los plugins de ejemplo

```bash
./scripts/build-mirada-plugins.sh
```

Compila los crates de ejemplo a `wasm32-unknown-unknown`, los pasa por `wasm-opt`
(si está), firma el reactor con una semilla **demo pública** (NO un secreto) y
deposita los `.wasm` + `.ron` + `trust.ron` en `assets/`. Esos `.wasm` se
commitean: los tests del host los cargan con `include_bytes!`, herméticos, sin
asumir el toolchain wasm32 en cada máquina.

Requiere `rustup target add wasm32-unknown-unknown`.

## Correr el host

```bash
# El Cuerpo arranca con MIRADA_SOCKET puesto; el host se conecta como cerebro.
MIRADA_SOCKET=$XDG_RUNTIME_DIR/mirada.sock cargo run -p mirada-plugin-host
```

## Mapa de archivos

| Archivo            | Qué                                                         |
|--------------------|-------------------------------------------------------------|
| `src/conductor.rs` | Orquesta `Desktop` + plugins; arbitra el flujo de comandos  |
| `src/wasm.rs`      | Integración `wasmi`: carga, gateo de importaciones, despacho |
| `src/caps.rs`      | El bitfield de capacidades y el mapa importación→capacidad  |
| `src/manifest.rs`  | Lee los `.ron`                                              |
| `src/trust.rs`     | Anillo de confianza + verificación de firmas                |
| `src/bin/sign.rs`  | `mirada-plugin-sign` (keygen + sign)                        |
| `src/main.rs`      | El binario: bucle evento→comandos + hot-reload             |

El lado *guest* vive en `../mirada-plugin-sdk`; los ejemplos en
`../mirada-plugin-example-{layout,reactor}`.
