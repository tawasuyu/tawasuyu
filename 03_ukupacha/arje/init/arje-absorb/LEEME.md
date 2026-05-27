# arje-absorb

Absorbe la configuración de **otro init** y la traduce a una **Tarjeta
Semilla** brahman: una `Card` JSON con cada servicio como hija `genesis`
de `arje-zero`.

Es el paso «absorber» de la migración a arje — para no perder los
servicios del sistema al cambiar de init. Lo usa `scripts/migrate-to-arje.sh`.

## Inits soportados

| init       | qué lee                                   | servicio → Card |
| ---------- | ----------------------------------------- | --------------- |
| `sysvinit` | `/etc/inittab`                            | `respawn` → daemon supervisado; `wait`/`once`/`sysinit` → one-shot |
| `runit`    | el `runsvdir` activo (o `/etc/sv`)        | cada script `run` → daemon supervisado (calce 1:1) |
| `dinit`    | `/etc/dinit.d/*`                          | `type=process`/`bgprocess` → daemon; `scripted` → one-shot |
| `openrc`   | `/etc/runlevels/{sysinit,boot,default}`   | `/etc/init.d/<svc> start` → one-shot |

`systemd` no se absorbe — sus units no son un formato de texto trivial.
Para systemd ya existe la capa de shims (`crates/compat/`) y la seed
`seeds/arje-host.card.json`.

## Uso

```sh
# autodetecta el init y emite la Semilla a stdout
arje-absorb

# explícito, a un archivo, agregando el gestor de login gráfico
arje-absorb --from openrc --output /ente/seed.card.json --with-carmen

# absorber otra raíz (un chroot, una imagen montada)
arje-absorb --root /mnt/sistema --from runit
```

Opciones: `--from <init>` (def. `auto`), `--root <dir>` (def. `/`),
`--output <f>` (def. `-` = stdout), `--label <s>`, `--with-carmen`.

## Lo que NO hace

La Semilla absorbida **conserva el comportamiento** del init viejo: los
servicios corren sin aislar (namespaces en `false`, FS read-write, red
`full`). No endurece el sandbox — eso se hace después, Card por Card.

La absorción de OpenRC es **superficial**: envuelve `/etc/init.d/<svc>
start` como one-shot (los scripts de OpenRC son shell completo, no se
parsean — el daemon lo deja en segundo plano OpenRC mismo, no arje).
runit y dinit, en cambio, dan supervisión real porque su `run`/`command`
es un proceso en primer plano.

Revisá la Semilla antes de instalarla como `/ente/seed.card.json`.
