# wawa-config — bus de configuración del SO

El **bus de configuración** del escritorio/SO: un archivo TOML canónico
(`~/.config/wawa/config.toml`) + un watcher (`notify`) que reemite cambios en
vivo, sobre una capa de sistema (`/etc/wawa/config.toml`) que el usuario puede
override-ar. Los consumidores (apps Llimphi del escritorio) se suscriben y
reaccionan al vuelo: cambiar theme/acento se propaga **sin reiniciar**.

UI-agnóstico: **no depende de `llimphi`**. El adaptador que ensambla un `Theme`
efectivo a partir del `WawaConfig` vive en `wawa-config-llimphi`.

## Qué expone

- `WawaConfig` — la configuración (variant de theme, accent override, …).
- Carga con merge `/etc/wawa` (sistema) bajo override de usuario.
- Watcher (`notify`) que reemite el config al cambiar el archivo.

## Estado (2026-05-31)

### Hecho
- Archivo canónico TOML + watcher `notify` (live reload).
- Capa de sistema `/etc/wawa/config.toml` mergeada bajo el override de usuario.
- Auto-apply del acento al theme global; ≈10 tests.
- Consumido por nada, cosmos, nakui, dominium, shuma, nahual, minga, arje,
  wawa-panel y `wawactl` (CLI).

### Pendiente
- Esquema de config más amplio (más que theme/acento).
- Validación/migración de versiones del TOML.
- Consumo desde el SO wawa bare-metal (hoy es el escritorio host).

## Lugar en el repo

`shared/wawa-config` — fuente de verdad UI-agnóstica. Adaptador de theme:
`wawa-config-llimphi`. CLI: `wawactl`.
