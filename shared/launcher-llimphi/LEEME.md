# launcher-llimphi

Frontend Llimphi del **motor de launcher único** de tawasuyu. Renderiza un
`launcher-core::Surface` a `View<Msg>` y trae el binario `tawasuyu-launcher`,
el launcher real del escritorio.

No es un launcher más al lado de `mirada-launcher` / `shuma-module-launcher`:
es la pieza que ésos están llamados a montar en vez de reimplementar su
propio despacho. Los datos (`Surface`, `Bar`, `Dock`, `Module`,
`AppMenuBar`) viven en `launcher-core` (`no_std`); la ejecución la resuelve
un `app_bus::Launcher` inyectado; acá vive sólo el render.

## Correr

```bash
cargo run -p launcher-llimphi --bin tawasuyu-launcher --release
```

La primera vez siembra `~/.config/tawasuyu/apps/*.toml` con el set base de
apps del repo (cosmos, nada, pluma, nahual, dominium, tinkuy, takiy, media,
tullpu, supay) y las descubre vía `app_bus::AppRegistry`. El dock se llena
con lo descubierto; click en un ítem lanza el binario (`ProcessLauncher`).

El demo sin descubrimiento ni config (apps de juguete, módulos estáticos):

```bash
cargo run -p launcher-llimphi --example launcher_demo
```

## Configurar

La superficie se describe en `~/.config/tawasuyu/launcher.toml` (respeta
`XDG_CONFIG_HOME`). Si no existe, cae a `Surface::desktop_default()` y
auto-llena el dock. Ver `launcher.example.toml` en este crate para el
schema completo. Mismo TOML/JSON sirve idéntico en host, shuma y wawa.

Kinds de módulo builtin que el render conoce: `app_menu` (slot del menú
global), `launch` (botón que lanza `app_id`), `dock` (inserta el dock por
`id`), `spacer`. Los módulos vivos del host — `clock`, `cpu`, `ram`,
`volume` — los pinta `host.rs` (reloj del sistema, CPU% de `/proc/stat`,
RAM% de `/proc/meminfo`), refrescados por un tick de 2 s. Cualquier otro
`kind` es un widget propio que el host inyecta vía `render_module`.

## API (para montar el launcher en otra app)

- `launcher_view(&LauncherSpec)` → árbol raíz, para `App::view`.
- `launcher_overlay(&LauncherSpec)` → dropdown del menú abierto, para
  `App::view_overlay` (`None` si no hay nada abierto).
- `host::module_view(m, &SysStats, &theme)` → el hook de módulos vivos de
  referencia para `LauncherSpec::render_module`.

El widget es **sin estado** (estilo Llimphi): el `Model` del host lleva qué
menú está abierto y la lista de tarjetas flotantes; el widget aplana la
`Surface` y emite `Msg`. El dock soporta tear-off (grip ⤢ desprende un ítem
como tarjeta flotante; la × la cierra — `on_close` en el spec).

## Estado (2026-05-31)

### Hecho
- Render de `Surface` (barras/dock/menú global/flotantes) a `View<Msg>`.
- Binario `tawasuyu-launcher` real: siembra + discovery de apps (`AppRegistry`),
  carga de `launcher.toml`, lanzamiento por `ProcessLauncher`.
- Módulos vivos del host (`clock`/`cpu`/`ram` desde `/proc`, tick 2 s) +
  dropdown del menú global vía `llimphi-widget-context-menu`.
- Dock con tear-off cerrable; API `launcher_view`/`launcher_overlay` para
  montar el launcher en otra app. (Su consumidor histórico, `mirada-launcher-
  llimphi`, se retiró en 2026-06-03: el marco del escritorio es ahora `pata`,
  con su propio modelo `pata-core`. Este crate queda disponible para reuso.)

### Pendiente
- Persistencia de tear-offs / layout editado por el usuario entre sesiones.
- `volume`/`brightness` reales (hoy parciales/stub).
- Empaquetado como sesión de escritorio (greeter → launcher).
- Render del mismo `Surface` en el compositor de wawa.
