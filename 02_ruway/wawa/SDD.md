# SDD — wawa (panel y bus de configuración)

`02_ruway/wawa/` aloja las piezas de espacio de usuario del SO wawa
relacionadas con configuración global y su UI. El kernel y el userland
mínimo viven aparte (`03_ukupacha/wawa/`, `03_ukupacha/arje/`).

## Componentes

```
02_ruway/wawa/
├── wawa-panel-llimphi/   GUI nativa para configurar el SO
└── wawactl/              CLI sobre el mismo bus
shared/wawa-config/       el bus en sí (modelo + watcher)
```

* **`shared/wawa-config`** — biblioteca que define el modelo
  (`WawaConfig`) y un suscriptor (`ConfigWatcher`). No tiene binario;
  la usan productores y consumidores.
* **`wawa-panel-llimphi`** — app Llimphi gráfica con seis categorías
  (apariencia, idioma, aplicaciones, monitor, módulos, acerca de).
  Productor (escribe en cada *Save*) y consumidor (recibe
  `ConfigChanged` si otro proceso edita el archivo).
* **`wawactl`** — CLI delgada para scripts y debugging. Subcomandos:
  `path`, `show`, `get`, `set`, `module`, `reset`, `watch`. Acepta
  `--system` en operaciones de escritura/lectura para targetear la
  capa `/etc/wawa/config.json` y `--layer system|user|effective` en
  `show` para inspeccionar capas concretas.

## El bus

### Medio físico

Dos archivos JSON canónicos, en dos capas:

```
/etc/wawa/config.json                  # sistema (Linux), requiere root
$XDG_CONFIG_HOME/wawa/config.json      # usuario
```

`user_config_path()` resuelve la de usuario con
`directories::ProjectDirs::from("", "", "wawa")` — en Linux es
`~/.config/wawa/config.json`. `system_config_path()` devuelve
`/etc/wawa/config.json` en Linux y `None` en otras plataformas.

La capa de **sistema** define machine-wide defaults; la de **usuario**
override campo por campo. `WawaConfig::load()` mergea `defaults →
system → user` con deep merge en `modules` (clave por clave, no
reemplazo total del mapa) y reemplazo total en el resto. Si la capa
de sistema no existe, el comportamiento es idéntico al original.

`/etc/` es una convención Unix/Linux: cuando wawa sea su propio SO,
`system_config_path()` devolverá el mecanismo nativo que defina arje y
la API pública no cambia. Las constantes `CONFIG_DIR`, `CONFIG_FILE`
y `SYSTEM_CONFIG_DIR_LINUX` están expuestas para que herramientas
externas (tests, packagers, instaladores) las puedan referenciar sin
importar `directories`.

### Modelo

```rust
pub struct WawaConfig {
    pub theme_variant: String,         // dark | light | aurora | sunset
    pub accent: String,                // default | unanchay | yachay | …
    pub lang: String,                  // es-PE | en-US | qu-PE
    pub timefmt_24h: bool,
    pub modules: BTreeMap<String, bool>,
}
```

Cada campo tiene `#[serde(default = "…")]` para que JSONs parciales
o de versiones anteriores se hidraten con defaults en lugar de
fallar. Esto permite agregar keys nuevas sin coordinar releases.

`BTreeMap` para `modules` (no `HashMap` ni `Vec`): orden estable al
serializar → diffs limpios en git si el archivo está versionado.

### Productor

```rust
use wawa_config::{Layer, WawaConfig};

let mut cfg = WawaConfig::load();      // efectiva mergeada; nunca falla
cfg.theme_variant = "aurora".into();
cfg.save()?;                            // atomic: tmp + rename, capa usuario

// O explícito a la capa de sistema (requiere root):
cfg.save_to(Layer::System)?;
```

`save()`/`save_to()` escriben a `config.json.tmp` y renombran sobre
`config.json` en la capa indicada. Los watchers ven un único evento
de creación que contiene la versión completa — no hay riesgo de leer
un archivo a medias. `save()` (sin argumento) sigue apuntando a la
capa de usuario para no romper callers existentes.

Para inspeccionar una capa concreta sin mergear: `WawaConfig::
load_layer(Layer::System)` o `Layer::User` devuelven `Option<Self>`
(`None` si el archivo no existe en esa capa — distingue "ausente" de
"presente con defaults").

### Consumidor

```rust
use wawa_config::{ConfigWatcher, WawaConfig};

// En App::init de tu app Llimphi (donde tenés un Handle<Msg>):
let cfg = WawaConfig::load();
// aplicar lo que corresponda (theme, locale, …)

let handle = handle.clone();
let _watcher = ConfigWatcher::spawn(move |new_cfg| {
    handle.dispatch(Msg::WawaConfigChanged(Box::new(new_cfg)));
})?;
// Guardar `_watcher` en el Model para que viva todo lo que vive la app.
```

El callback corre en un thread del watcher (no en el event loop de
Llimphi). El patrón canónico es capturar un `Handle<Msg>` clonado y
hacer `handle.dispatch(...)`: la reentrada al `update` ocurre en el
hilo de UI, donde es seguro tocar el `Model`.

### Debounce

Editores y herramientas suelen escribir con la secuencia
`truncate → write → close` o con `O_TMPFILE + rename`, lo que genera
varios eventos `notify` por una sola operación lógica. El watcher
agrupa señales durante **200 ms** y emite un único callback con la
versión leída tras la pausa. Acepta perder estados intermedios
durante ráfagas — solo importa el estado final.

El `ConfigWatcher` observa **ambas capas**: el callback se dispara
cuando cambia cualquiera de los dos archivos, ya con la config
efectiva mergeada. Si la capa de sistema no aplica en la plataforma
(no Linux) o no se puede observar (p. ej. `/etc/wawa` ausente al
arrancar la app), el watcher sigue activo sólo sobre la capa de
usuario — best-effort, log a `warn` y no rompe.

## Por qué archivo + `notify` y no daemon pub-sub

| Aspecto | Archivo + notify | Daemon pub-sub |
|---|---|---|
| Estado inicial | leer un archivo | conectarse al socket |
| Dep en runtime | ninguna | el daemon arrancado |
| Auditable | `cat`, `jq`, `vim` | `busctl`, herramienta propia |
| Atomic update | `rename(2)` | protocolo |
| Concurrencia | OS file locking trivial | manejo de sesiones |
| Pérdida de estado | nunca (el archivo es la verdad) | si el daemon muere |
| Tests | filesystem temporal | mock socket |

Para configuración del usuario, "fuente de verdad" es naturalmente
un archivo. Un daemon agrega complejidad sin valor: el caso de uso
real es "el panel cambia algo y los consumidores reaccionan en el
próximo turno", para lo cual `notify` con debounce alcanza y sobra.

`arje-bus` (el bus de capabilities del init) **no encaja** acá:
requiere `ENTE_BUS_SOCK` y `ENTE_ID` en el env de cada proceso, lo
que sólo tienen los entes hijos del init. Las apps Llimphi se
lanzan independientemente desde el usuario y no son entes hijos.

## Quién es consumidor hoy

| Crate | Cuadrante | Rol | Qué consume |
|---|---|---|---|
| `wawa-panel-llimphi` | 02_ruway | productor + consumidor | todo |
| `wawactl` | 02_ruway | productor + consumidor | todo (CLI) |
| `nada` | 02_ruway | consumidor | theme, accent, lang |
| `nahual-shell-llimphi` | 02_ruway | consumidor | theme, accent |
| `dominium-app-llimphi` | 01_yachay | consumidor | theme, accent, lang |
| `cosmos-app-llimphi` | 01_yachay | consumidor | theme, accent, lang |
| `nakui-explorer-llimphi` | 01_yachay | consumidor | theme, accent, lang |

Cualquier app Llimphi puede sumarse en ~30 líneas siguiendo la
sección **Consumidor**.

### `shared/wawa-config-llimphi`

Adaptador Llimphi del bus. Expone un único helper:

```rust
pub fn theme_from_wawa(cfg: &WawaConfig, fallback: &Theme) -> Theme;
```

Existe para no obligar a `wawa-config` (UI-agnóstico) a depender de
`llimphi-theme`. Los consumidores Llimphi importan ambos: `wawa-config`
para `WawaConfig`/`ConfigWatcher`, y `wawa-config-llimphi` para el
helper. 4 tests unitarios cubren los 4 caminos (variant base,
override de acento, variant desconocido → fallback, accent default
= no override).

**Por qué crate separado y no feature flag**: features en `wawa-config`
contaminarían sus tests (Llimphi arrastra winit + wgpu en CI). Un
crate dedicado mantiene el grafo limpio para herramientas no-GUI.

### Qué no encaja

- **`mirada-bar-core` / `mirada-bar-web`** no son apps Llimphi sino
  taskbar DOM (HTML+CSS+JS sobre wasm-bindgen). El patrón de
  `Handle<Msg>` + `ConfigWatcher` no aplica directo. Si en el
  futuro se quisiera sincronizar el theme entre el escritorio web y
  el SO, el path es escribir un proxy HTTP/WebSocket que lea el
  archivo y lo emita a la página por SSE — eso es trabajo aparte.

## Cómo probar el bus

```sh
# Build
cargo build -p wawa-config -p wawa-panel-llimphi -p wawactl -p nada

# Terminal 1: observar el bus
./target/debug/wawactl watch

# Terminal 2: emitir cambios
./target/debug/wawactl set theme_variant aurora
./target/debug/wawactl set lang qu-PE
./target/debug/wawactl module shuma off
./target/debug/wawactl reset

# Capa de sistema (requiere root):
sudo ./target/debug/wawactl set theme_variant dark --system
sudo ./target/debug/wawactl module mirada off --system
./target/debug/wawactl show --layer system    # sólo /etc/wawa/...
./target/debug/wawactl show --layer user      # sólo $XDG_CONFIG_HOME/...
./target/debug/wawactl show                   # efectiva (default)

# Terminal 3 (opcional): consumidores reales
./target/debug/wawa-panel       # GUI; cambios se reflejan al instante
./target/debug/nada      # cambia theme cuando el bus emite
```

## Roadmap

* **`accent` aplicado al theme global** — hoy el acento sólo
  tinta los segmented chips del panel; falta propagarlo como
  override del `theme.accent` cuando no es `"default"`.
* **Toggles de módulos con efecto real** — actualmente persisten
  estado, no arrancan/paran daemons. El binding al supervisor del SO
  (arje, mirada-compositor, shuma) llega cuando exista el contrato.
* **Permisos** — hoy cualquier proceso del usuario puede tocar el
  archivo. Para multiusuario o sandboxes futuros, agregar
  `getpeercred`/`SO_PEERCRED` si pasamos a daemon.
* **Migración a wawa-OS** — `system_config_path()` devolverá el
  mecanismo nativo que defina arje en lugar de `/etc/wawa`. Los
  consumidores no deberían enterarse: la API pública (`load`,
  `save`, `Layer::{System,User}`) se mantiene; sólo cambia lo que
  resuelve el path interno.
