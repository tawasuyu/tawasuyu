# launcher-core — layout del motor único de launcher

El **layout configurable** de un solo motor de launcher reusable. No tres
launchers (mirada / shuma / wawa) sino **una** estructura de datos que se monta
donde sea. Lo que varía por entorno NO vive aquí: el render (Llimphi en
host/shuma, compositor en wawa) y la instrucción de ejecución
(`app_bus::Launcher`) son adaptadores inyectados. Aquí vive sólo el *qué se
dibuja y dónde*, como datos puros `no_std`.

Generaliza el `WidgetSpec { kind, props }` de `mirada-launcher`: cada `Module`
es un `kind` + props arbitrarios que el render interpreta.

## Qué expone

- `Surface` — la superficie completa: `bars` + `docks` + `floating` + `app_menu`.
  Se describe en `~/.config/gioser/launcher.toml`, idéntica en host/shuma/wawa.
- `Bar` (anclada a un `Edge`, slots start/center/end), `Dock` (con `tear_off`),
  `FloatingCard` (tear-off materializado), `AppMenuBar` (menú global estilo mac).
- `Module` (`kind` + `props: BTreeMap<String, Prop>`) con accesores tipados.
- `Surface::desktop_default()` — escritorio de arranque sensato.

`#![no_std] + alloc`; sólo depende de `serde`. El render es `launcher-llimphi`.

## Estado (2026-05-31)

### Hecho
- Schema completo de `Surface`/`Bar`/`Dock`/`FloatingCard`/`AppMenuBar`/`Module`.
- `Prop` portátil (bool/int/float/str) y accesores `str_prop`/`f64_prop`/`bool_prop`.
- Default de escritorio + roundtrip TOML/JSON; tests del schema.

### Pendiente
- Render en wawa (compositor) — hoy sólo lo consume `launcher-llimphi` en host.
- Persistencia de tear-offs / posición de flotantes entre sesiones.
- Kinds de módulo extra más allá de los builtin documentados.

## Lugar en el repo

`shared/launcher-core` — schema de datos. Frontend: `launcher-llimphi`. Catálogo
de apps + bus: `app-bus`.
