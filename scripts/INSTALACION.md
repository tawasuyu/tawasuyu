# Instalación de la capa de sistema de tawasuyu

Qué instalan los `scripts/install-*.sh`, qué binarios/daemons dejan en el sistema,
qué se autoarranca y cómo desinstalar. Foco en la **capa de sistema** (escritorio +
herramientas), no en las apps de dominio.

> Fuente de verdad = los scripts. Este doc los resume; si difieren, manda el script.

## Punto de entrada

```bash
./scripts/install-tawasuyu.sh            # desktop + splash (binario, sin habilitar)
./scripts/install-tawasuyu.sh --with-compat   # + shims arje-compat (sesión GNOME)
./scripts/install-tawasuyu.sh --with-init     # + arje como init alterno (entrada GRUB)
./scripts/install-tawasuyu.sh --all           # todo lo de arriba
./scripts/install-tawasuyu.sh --yes           # sin preguntas
./scripts/install-tawasuyu.sh --uninstall     # revierte TODO lo que instaló
```

`install-tawasuyu.sh` **orquesta** los `install-*.sh` por etapas:

| Etapa     | Script                          | Qué hace                                                                 |
|-----------|---------------------------------|--------------------------------------------------------------------------|
| desktop   | `install-mirada-dm.sh`          | Compositor + greeter + barra (pata) + shell (shuma) + lanzadores + panel + **pacha** + **agora** + notificaciones + portal + wallpaper. Es donde vivís (`sudo mirada-dm`). |
| splash    | `install-arje-splash.sh --system` | Binario + config del splash sin parpadeo (no habilita el servicio).     |
| compat    | `install-arje-session-gnome.sh` | (opt, `--with-compat`) shims D-Bus de arje-compat (logind/hostnamed/…).  |
| init      | `install-arje-init.sh`          | (opt, `--with-init`) arje-zero como **init alterno** (PID 1) en una entrada de GRUB aparte. |

## Qué se instala en `/usr/local/bin` (etapa desktop)

`install-mirada-dm.sh` compila en release y copia. Agrupado por subsistema:

### Escritorio (mirada) — el SO gráfico
- `mirada-compositor` — compositor Wayland + WM.
- `mirada-greeter` — pantalla de login.
- `mirada-llimphi` — app de control del compositor.
- `mirada-ctl` — CLI del compositor (vista, special-workspaces, place-app).
- `mirada-portal` — xdg-desktop-portal.
- `mirada-wallpaper` — fondo (estático + rotación).
- `mirada-launcher` — spotlight de apps.
- `mirada-session`, `mirada-session-pata`, `mirada-session-plugins`, `mirada-dm`,
  `mirada-supervise` — scripts/lanzadores de sesión.
- `mirada-plugin-host`, `mirada-plugin-sign` — host de plugins WASM del WM + firma
  (ambos binarios salen del crate `mirada-plugin-host`).

### Barra y shell
- `pata-llimphi` — la barra/panel (reloj, tray, red, control, lanzadores, **chips de
  contexto pacha**…).
- `shuma-shell-llimphi` — el shell/terminal-workspace.
- `pata-notify`, `pata-notify-panel`, `pata-notify-triage` — daemon de
  notificaciones + sidebar de historial + triage semántico.

### Configuración
- `wawa-panel` — el panel de control unificado (allichay). Cada subsistema es un
  **diente**: Vista · Themes · Atajos · Animaciones · Pata · Inicio · Sistema ·
  Acerca · Correo (paloma) · **Contextos (pacha + identidad/cifrado)**.

### Sistema soberano (nuevo)
- **`pacha`** — contextos de usuario (modos de uso con nombre). `pacha switch <id>`
  (lo invocan los chips de pata y el diente «Contextos»), `pacha list`, y
  `pacha dotfiles {add,snapshot,restore,list,pubkey,publish,push}` (versionado +
  cifrado de dotfiles por contexto). **Se autoarranca** como daemon (ver abajo).
- **`agora-cli`** — identidad soberana Ed25519. `agora-cli identidad nueva`,
  `agora-cli desbloquear` (cachea la seed en el session keyring para que pacha
  cifre). El diente «Contextos» del panel lo invoca para crear/desbloquear.
- **`sandokan`** — plano de control: arranca/para/observa unidades (Linux y Wawa).
  La UI `sandokan-monitor` se instala con las apps de la suite.
- **`voz-daemon`** — daemon de voz STT+TTS por socket Unix (hoy backends mock). La
  sección «Voz» del panel lo configura.
- **`verbo-daemon`** — daemon de embeddings (lo consumen pluma-semantic, khipu,
  chasqui). Trae backend fastembed (pesado) → **no se fuerza su build**; se instala
  **sólo si ya está compilado** (`cargo build --release -p rimay-verbo-daemon-bin`).

### Apps de la suite (sólo si ya están en `target/release`)
`nada`, `pluma-editor-llimphi`, `pluma-notebook-llimphi`, `tullpu-app-llimphi`,
`takiy-app-llimphi`, `media-app`, `cosmos-app-llimphi`, `dominium-app-llimphi`,
`tinkuy-llimphi`, `chaka-app-llimphi`, `nakui-sheet-llimphi`, `puriy`, `raymi-app`,
`supay-app-llimphi`, `sandokan-monitor`, `nahual-shell-llimphi`. No se fuerza su
build: instalá las que quieras con `cargo build --release -p <crate>` y recorré el
instalador. Existen para que los **lanzadores** de la barra encuentren el binario.

## Autostart de sesión (`~/.config/mirada/autostart`)

`install-mirada-dm.sh` siembra (idempotente) en tu autostart:

- `pata-notify` — el daemon de notificaciones (necesita el compositor vivo).
- `pacha daemon` — el activador de contextos. **Sin él, `pacha switch` no hace
  nada** (los chips de pata y el panel quedan inertes). El panel y el triage de
  notificaciones son on-demand, no se autoarrancan.

`verbo-daemon`/`voz-daemon` **no** se autoarrancan: sus consumidores los levantan
on-demand (o caen a backend Mock si no corren).

## Identidad + cifrado de dotfiles (cómo encaja agora ↔ pacha)

El versionado de dotfiles de pacha cifra el almacén en reposo con tu identidad
soberana. El flujo:

```bash
agora-cli identidad nueva --name yo      # crea la identidad (1 sola vez)
export AGORA_PASSPHRASE="tu-frase"       # passphrase del keystore
agora-cli desbloquear                    # descifra la seed y la cachea en el
                                         # session keyring (pacha:id:default)
pacha dotfiles add shell .zshrc          # a partir de acá el store va CIFRADO
pacha dotfiles snapshot shell
```

Lo mismo se hace desde el **wawa-panel → Contextos → Identidad** (botones «Crear» /
«Desbloquear» + campo de frase, que se pasa a `agora-cli` por `AGORA_PASSPHRASE` y
no se guarda). Mientras la seed esté en el keyring, el cifrado está activo; al
cerrar sesión el kernel la olvida.

### Desbloqueo automático al login (módulo PAM)

`pam_tawasuyu.so` desbloquea la identidad **al loguear**, reusando la contraseña
que ya tipeás. El instalador **copia la `.so`** a `/usr/lib/security/` y el ejemplo
de config a `/usr/local/share/tawasuyu/pam-tawasuyu.example`, pero **NO toca
`/etc/pam.d`** (una mala config puede dejarte sin login): la activación es manual.

Cómo funciona:

- En la fase `auth` (tras `pam_unix`), el módulo captura `PAM_AUTHTOK` (tu
  contraseña ya validada) y la guarda para la fase de sesión.
- En la fase `session` (tras `pam_keyinit`), ejecuta `agora-cli desbloquear`
  **como el usuario**, con la frase en `AGORA_PASSPHRASE`. La seed queda en el
  *session keyring* que tu sesión hereda → `pacha` cifra/descifra sin re-pedir.
  `pam_keyinit ... revoke` la evapora al cerrar sesión.

Activación (en, p.ej., `/etc/pam.d/login` o el servicio de tu greeter):

```text
auth     optional   pam_tawasuyu.so                 # después de pam_unix
session  optional   pam_keyinit.so   force revoke
session  optional   pam_tawasuyu.so                 # después de pam_keyinit
```

Requisito: la passphrase de la identidad agora (la que pusiste al crearla con
`AGORA_PASSPHRASE`) debe ser la **misma** que tu contraseña de login. Si difieren,
el desbloqueo falla en silencio (best-effort — nunca rompe el login). Probá con
`pamtester <servicio> "$USER" open_session` y `keyctl search @s user
pacha:id:default` **antes** de cerrar tu sesión. Detalle completo en el `.example`.

Si la passphrase de agora NO es tu contraseña de login, usá el **desbloqueo
manual**: el panel (Contextos → Identidad → Desbloquear) o `agora-cli desbloquear`.

## Desinstalar

```bash
./scripts/install-tawasuyu.sh --uninstall
```

Revierte por etapas: `install-arje-init.sh --uninstall`, `install-arje.sh
--uninstall`, y borra de `/usr/local/bin` la lista `MIRADA_BINS` (que incluye
`pacha`, `agora-cli`, `sandokan`, `voz-daemon`, `verbo-daemon` además de todo
mirada/pata/shuma). Las entradas de autostart sembradas (`pacha daemon`,
`pata-notify`) quedan en tu `~/.config/mirada/autostart` — borralas a mano si no
las querés.

## Build manual (sin instalar)

```bash
# Todo el desktop + sistema soberano:
cargo build --release -p mirada-compositor -p mirada-greeter -p pata-llimphi \
  -p shuma-shell-llimphi -p wawa-panel-llimphi -p pacha-cli -p agora-cli \
  -p sandokan-app -p rimay-voz-daemon-bin -p pam-tawasuyu
# Embeddings (pesado, opcional):
cargo build --release -p rimay-verbo-daemon-bin
```
