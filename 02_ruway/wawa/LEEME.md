# wawa (userspace)

> Pareja userspace de `03_ukupacha/wawa`: panel de control + CLI.

Acá vive lo que el operador de Wawa usa **desde un host Linux** (no desde adentro del kernel): el panel Llimphi para ver/mutar config, y `wawactl` para operaciones desde terminal. La parte de kernel/bootloader/filesystem está en `03_ukupacha/wawa/`. Detalle en [SDD.md](SDD.md).

## Instalación

```sh
# panel desktop (Llimphi)
cargo run --release -p wawa-panel-llimphi

# CLI
cargo run --release -p wawactl
```

## Compatibilidad

- **Linux** — primary host. Habla con `wawa-kernel` via virtio-console o socket Unix.
- **macOS / Windows** — sólo si Wawa corre en VM accesible (TCP).

## Crates

| Crate | Rol |
|---|---|
| [`wawa-panel-llimphi`](wawa-panel-llimphi/README.md) | Panel de control Llimphi: estado de apps, config, recursos. |
| [`wawactl`](wawactl/README.md) | CLI: `wawactl show`, `wawactl set`, `wawactl gc`, `wawactl daemon-firma`, etc. |

## Consideraciones

- **Userspace, no kernel.** Si necesitás tocar boot/fs/proc del Wawa, andá a `03_ukupacha/wawa`.
- El panel y `wawactl` comparten el modelo de config con el shell del escritorio (via `shared/wawa-config`).

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
