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
| [`wawactl`](wawactl/README.md) | CLI: `wawactl show`, `wawactl set`, `wawactl gc`, `wawactl daemon-firma`, etc. |

## Considerations

- **Userspace, not kernel.** Boot/fs/proc tweaks → `03_ukupacha/wawa`.
- Panel and `wawactl` share the config model with the desktop shell (via `shared/wawa-config`).

## Estado (2026-06-09)

### Hecho

- **`wawa-panel-llimphi`**: panel de configuración navegado por un rail de **dientes**
  (`llimphi-widget-dock-rail`) con jerarquía de 3 niveles (pestaña → items en sidebar →
  canvas): pestañas **Sistema** (Apariencia · Idioma · Interfaz · Arranque · Módulos) e
  **Información**, más dientes-de-app suscritas (**mirada** — incl. keymap editable como
  tabla — y **pata**). Renderiza con `llimphi-module-allichay` (edición in-situ de celdas
  de tabla/lista, campo hex `#RRGGBB` en el color-picker, sidebar resizable/ocultable);
  productor y consumidor del bus `shared/wawa-config`, con debounce del guardado.
- **`wawactl`**: CLI sobre el mismo bus — `path`, `show`, `get`, `set`, `module`,
  `reset`, `watch`, `firmar-cuaderno`, `claves`, `gc`, `daemon-firma`; con `--system`
  para la capa `/etc/wawa/config.json` y `--layer system|user|effective` en `show`.
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

- **Toggles de módulos con efecto real**: hoy persisten estado y ocultan el diente de la
  app en el panel, pero no arrancan/paran daemons (espera el contrato con el supervisor
  del SO: arje/mirada-compositor/shuma).
- **Permisos**: cualquier proceso del usuario puede tocar el archivo; falta
  `getpeercred`/`SO_PEERCRED` para multiusuario/sandboxes.
- **Migración a wawa-OS**: `system_config_path()` devolverá el mecanismo nativo de arje
  en lugar de `/etc/wawa` (API pública estable).
- Detalle del bus y los flags en [SDD.md](SDD.md).
