# `viewers.d/` — viewers as Cards (Brahman, Phase 2a Step 2)

The shell assembles its viewer routing table as **built-ins + discovered
Cards**. The built-ins are the viewers the binary links in
process; the discovered ones are `card_core::Card`s (JSON or TOML) that the shell reads
from a directory on startup.

## Where they are read from

```
$NAHUAL_VIEWERS_DIR                         (if set)
$XDG_CONFIG_HOME/nahual/viewers.d           (if not)
~/.config/nahual/viewers.d                  (fallback)
```

Try the example in this folder:

```bash
NAHUAL_VIEWERS_DIR=02_ruway/nahual/nahual-shell-llimphi/viewers.d.example \
  cargo run -p nahual-shell-llimphi --release
```

## What a viewer Card does

It is a `Card` of `kind: "data"` with three shell-specific extensions
(serialized at the top-level of the JSON):

| key                      | type        | meaning                                       |
|--------------------------|-------------|-----------------------------------------------|
| `nahual.viewer_kind`     | string      | which mounted viewer it routes to (`image`, `video`, `audio`, `card`, `tree`, `hex`, `table`, `markdown`, `archive`, `font`, `text`) |
| `nahual.mime_exact`      | `[string]`  | exact mimes it covers                         |
| `nahual.mime_prefixes`   | `[string]`  | mime prefixes it covers (e.g. `"image/"`)     |

The `lens`es come from `data.presentation_hint` (+ an optional
`nahual.lenses: [string]`). The Card's `priority` is the tie-break, with the
same order `chasqui-broker` uses (`low < normal < high < critical`).

## What works today and what doesn't

- **Extending the routing of an already-mounted viewer** (this example: teaching it to open
  PSD with the image viewer) works end-to-end: it reuses the in-process
  constructor, no IPC needed.
- A Card with a `nahual.viewer_kind` the shell **does not** know how to mount is
  silently ignored — it would be an out-of-process viewer, pending the
  AppBus render-IPC.

## The seam toward the broker

Today the source of Cards is a directory on disk. It is deliberately the
**same format** that `card-discovery` scans and that the broker (`chasqui`)
announces: when the AppBus is alive, `discover_viewer_cards()` changes its source
from "directory" to "broker" without touching the ranking algorithm. The contract — a
`Card` with `lens`/`mime`/`priority` — is already Brahman's.
