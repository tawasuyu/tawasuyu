# wawa-config-llimphi — adaptador Llimphi de wawa-config

Ensambla el `Theme` efectivo de Llimphi a partir del `WawaConfig` (variant +
accent override). Existe para **no obligar a `wawa-config` a depender de
`llimphi-theme`** — así el bus de configuración sigue siendo UI-agnóstico y este
crate hace el puente hacia el render.

## Qué expone

- Conversión `WawaConfig` → `Theme` de Llimphi (variant + acento).
- Helper para que una app Llimphi obtenga su theme efectivo y reaccione al
  watcher de `wawa-config`.

## Estado (2026-05-31)

### Hecho
- Adaptador `WawaConfig` → `Theme` (variant + accent override).
- Cableado a varios consumidores (cosmos, nakui, nahual-shell, nada, dominium,
  shuma).
- ≈4 tests.

### Pendiente
- Mapear más campos de config al theme conforme `wawa-config` crezca.
- Transiciones/animación al cambiar theme en vivo (hoy aplica directo).

## Lugar en el repo

`shared/wawa-config-llimphi` — frontend de theme sobre `shared/wawa-config`.
