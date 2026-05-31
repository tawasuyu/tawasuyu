# wawa (userspace)

> Userspace counterpart of `03_ukupacha/wawa`: control panel + CLI.

This is what the Wawa operator uses **from a Linux host** (not from inside the kernel): the Llimphi panel for state/config, and `wawactl` for terminal ops. The kernel/bootloader/filesystem side is in `03_ukupacha/wawa/`. Detail in [SDD.md](SDD.md).

## Install

```sh
cargo run --release -p wawa-panel-llimphi
cargo run --release -p wawactl
```

## Compatibility

- **Linux** — primary host. Talks to `wawa-kernel` via virtio-console or Unix socket.
- **macOS / Windows** — only if Wawa runs in an accessible VM (TCP).

## Crates

| Crate | Role |
|---|---|
| [`wawa-panel-llimphi`](wawa-panel-llimphi/README.md) | Llimphi control panel: app state, config, resources. |
| [`wawactl`](wawactl/README.md) | CLI: `wawactl status`, `wawactl deploy`, etc. |

## Considerations

- **Userspace, not kernel.** Boot/fs/proc tweaks → `03_ukupacha/wawa`.
- Panel and `wawactl` share the config model with the desktop shell (via `shared/wawa-config`).

## Estado (2026-05-31)

### Hecho

- **`wawa-panel-llimphi`**: app Llimphi gráfica con seis categorías (apariencia, idioma,
  aplicaciones, monitor, módulos, acerca de); productor y consumidor del bus
  `shared/wawa-config` (reacciona a `ConfigChanged` si otro proceso edita el archivo).
- **`wawactl`**: CLI sobre el mismo bus — `path`, `show`, `get`, `set`, `module`,
  `reset`, `watch`; con `--system` para la capa `/etc/wawa/config.json` y
  `--layer system|user|effective` en `show`.
- **Bus de configuración en dos capas** (`shared/wawa-config`): system (`/etc/wawa`) +
  user (`$XDG_CONFIG_HOME/wawa`), deep-merge en `modules`, save atómico (tmp+rename),
  watcher `notify` con debounce 200 ms sobre ambas capas. Adaptador
  `shared/wawa-config-llimphi` (`theme_from_wawa`, 4 tests).
- **Consumidores reales** ya cableados: `nada`, `nahual-shell-llimphi`,
  `dominium/cosmos/nakui-explorer` app-llimphi (theme/accent/lang vivos).
- **`wawactl` ↔ kernel** (Fases 38–63): canal del firmador externo, aduana cripto
  multi-autor + CRL + ventana de pre-autorización, demonio bidireccional + anillo de
  auditoría, daemon pubkey reveal + envelope compositor multi-slot, Boot Trust Ceremony
  con claves soberanas reales, virtio-console + crypto HAL high-speed,
  `wawactl gc` (control remoto del GC sobre virtio-console), `daemon-firma` distingue
  cuaderno y configuración.
- **Docs i18n**: README EN (default) + LEEME ES + README.qu QU, con live reload.
- **Menús** (lote 6): menú principal + menús contextuales en el panel.

### Pendiente

- **`accent` aplicado al theme global**: hoy sólo tinta los chips del panel; falta
  propagarlo como override de `theme.accent` cuando no es `"default"`.
- **Toggles de módulos con efecto real**: hoy persisten estado, no arrancan/paran
  daemons (espera el contrato con el supervisor del SO: arje/mirada-compositor/shuma).
- **Permisos**: cualquier proceso del usuario puede tocar el archivo; falta
  `getpeercred`/`SO_PEERCRED` para multiusuario/sandboxes.
- **Migración a wawa-OS**: `system_config_path()` devolverá el mecanismo nativo de arje
  en lugar de `/etc/wawa` (API pública estable).
- Detalle del bus y los flags en [SDD.md](SDD.md).
