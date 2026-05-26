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
  `path`, `show`, `get`, `set`, `module`, `reset`, `watch`.

## El bus

### Medio físico

Un único archivo JSON canónico:

```
$XDG_CONFIG_HOME/wawa/config.json
```

Resuelto con `directories::ProjectDirs::from("", "", "wawa")`. En
Linux es `~/.config/wawa/config.json`. Las constantes
`wawa_config::CONFIG_DIR` y `CONFIG_FILE` están expuestas para que
las herramientas externas (tests, packagers) las puedan referenciar
sin importar `directories` ellas mismas.

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
use wawa_config::WawaConfig;

let mut cfg = WawaConfig::load();      // nunca falla; defaults si no existe
cfg.theme_variant = "aurora".into();
cfg.save()?;                            // atomic: tmp + rename
```

`save()` escribe a `config.json.tmp` y renombra sobre `config.json`.
Los watchers ven un único evento de creación que contiene la versión
completa — no hay riesgo de leer un archivo a medias.

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

| Crate | Rol | Qué consume |
|---|---|---|
| `wawa-panel-llimphi` | productor + consumidor | todo |
| `gioser-edit` | consumidor | `theme_variant`, `lang` |
| `wawactl` | productor + consumidor | todo (CLI) |

Cualquier app Llimphi puede sumarse en ~30 líneas siguiendo la
sección **Consumidor**.

## Cómo probar el bus

```sh
# Build
cargo build -p wawa-config -p wawa-panel-llimphi -p wawactl -p gioser-edit

# Terminal 1: observar el bus
./target/debug/wawactl watch

# Terminal 2: emitir cambios
./target/debug/wawactl set theme_variant aurora
./target/debug/wawactl set lang qu-PE
./target/debug/wawactl module shuma off
./target/debug/wawactl reset

# Terminal 3 (opcional): consumidores reales
./target/debug/wawa-panel       # GUI; cambios se reflejan al instante
./target/debug/gioser-edit      # cambia theme cuando el bus emite
```

## Roadmap

* **`accent` aplicado al theme global** — hoy el acento sólo
  tinta los segmented chips del panel; falta propagarlo como
  override del `theme.accent` cuando no es `"default"`.
* **Toggles de módulos con efecto real** — actualmente persisten
  estado, no arrancan/paran daemons. El binding al supervisor del SO
  (arje, mirada-compositor, shuma) llega cuando exista el contrato.
* **Sección `/etc/wawa/config.json`** — `wawa-config` ya expone
  `watch_path()` para observar un directorio externo; falta una capa
  que mezcle config de sistema y usuario con precedencia clara.
* **Permisos** — hoy cualquier proceso del usuario puede tocar el
  archivo. Para multiusuario o sandboxes futuros, agregar
  `getpeercred`/`SO_PEERCRED` si pasamos a daemon.
