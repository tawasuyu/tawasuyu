# `viewers.d/` — visores como Cards (Brahman, Fase 2a Paso 2)

El shell ensambla su tabla de ruteo de visores como **built-ins + Cards
descubiertas**. Las built-ins son los visores que el binario linkea en
proceso; las descubiertas son `card_core::Card`s (JSON o TOML) que el shell lee
de un directorio al arrancar.

## Dónde se leen

```
$NAHUAL_VIEWERS_DIR                         (si está seteada)
$XDG_CONFIG_HOME/nahual/viewers.d           (si no)
~/.config/nahual/viewers.d                  (fallback)
```

Probar el ejemplo de esta carpeta:

```bash
NAHUAL_VIEWERS_DIR=02_ruway/nahual/nahual-shell-llimphi/viewers.d.example \
  cargo run -p nahual-shell-llimphi --release
```

## Qué hace una Card de visor

Es una `Card` de `kind: "data"` con tres extensiones propias del shell
(serializadas al top-level del JSON):

| clave                    | tipo        | significado                                   |
|--------------------------|-------------|-----------------------------------------------|
| `nahual.viewer_kind`     | string      | a qué visor montado rutea (`image`, `video`, `audio`, `card`, `tree`, `hex`, `table`, `markdown`, `archive`, `font`, `text`) |
| `nahual.mime_exact`      | `[string]`  | mimes exactos que cubre                       |
| `nahual.mime_prefixes`   | `[string]`  | prefijos de mime que cubre (p. ej. `"image/"`) |

Los `lens` salen de `data.presentation_hint` (+ un opcional
`nahual.lenses: [string]`). La `priority` de la Card es el desempate, con el
mismo orden que usa `chasqui-broker` (`low < normal < high < critical`).

## Qué funciona hoy y qué no

- **Extender el ruteo de un visor ya montado** (este ejemplo: enseñar a abrir
  PSD con el visor de imágenes) funciona end-to-end: reusa el constructor
  in-process, no necesita IPC.
- Una Card con un `nahual.viewer_kind` que el shell **no** sabe montar se
  ignora en silencio — sería un visor fuera de proceso, pendiente del
  render-IPC del AppBus.

## La costura hacia el broker

Hoy el origen de las Cards es un directorio en disco. Es deliberadamente el
**mismo formato** que `card-discovery` escanea y que el broker (`chasqui`)
anuncia: cuando el AppBus esté vivo, `discover_viewer_cards()` cambia su fuente
de "directorio" a "broker" sin tocar el algoritmo de ranking. El contrato —una
`Card` con `lens`/`mime`/`priority`— ya es el de Brahman.
