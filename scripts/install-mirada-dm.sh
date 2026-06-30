#!/bin/sh
# install-mirada-dm.sh — instala mirada como display manager y como
# compositor/sesión de escritorio en el sistema. Lo construye con tu
# toolchain y copia todo a su lugar (pide sudo sólo para instalar).
#
# Uso:   ./scripts/install-mirada-dm.sh
# Luego: sudo mirada-dm                 # arranca el DM como un init
#
# Deja instalado:
#   /usr/local/bin/{mirada-compositor,mirada-greeter,pata-llimphi}
#   /usr/local/bin/{shuma-shell-llimphi,mirada-launcher}
#   /usr/local/bin/{pata-notify,pata-notify-panel,pata-notify-triage}
#   /usr/local/bin/{mirada-session,mirada-session-pata,mirada-session-plugins,mirada-dm}
#   /usr/local/bin/{mirada-plugin-host,mirada-plugin-sign}
#   /etc/pam.d/mirada                              (login del greeter)
#   /usr/share/wayland-sessions/{mirada,mirada-pata,mirada-plugins}.desktop
#   ~/.config/mirada/autostart                     (siembra el daemon pata-notify)
#   ~/.config/mirada/plugins/*                      (siembra los plugins de ejemplo)
#
# Para desinstalar: borrá esos archivos.
set -eu

if [ "$(id -u)" -eq 0 ]; then
    echo "No me corras con sudo: construyo con tu toolchain y pido sudo solo" >&2
    echo "para los pasos de instalación.  ->  ./scripts/install-mirada-dm.sh" >&2
    exit 1
fi

REPO=$(cd "$(dirname "$0")/.." && pwd)
cd "$REPO"
MC="$REPO/02_ruway/mirada/mirada-compositor"

# Este script NO hace git pull: construye lo que esté CHECKED OUT. Mostramos el
# HEAD para que se vea qué se está por instalar — si no incluye tus cambios,
# corré `git pull` (o `./scripts/actualizar-mirada.sh`) ANTES.
echo "==> repo en: $(git -C "$REPO" log -1 --format='%h %ci %s' 2>/dev/null || echo '¿no es un repo git?')"

echo "==> construyendo (release): compositor, greeter, pata, shuma, launchers, panel, ctl, portal, wallpaper, notificaciones y pacha"
cargo build --release \
    -p mirada-compositor -p mirada-greeter -p pata-llimphi \
    -p shuma-shell-llimphi -p mirada-launcher -p mirada-app-llimphi \
    -p mirada-ctl -p mirada-portal -p mirada-wallpaper -p wawa-panel-llimphi \
    -p pata-notify -p pata-notify-panel -p pata-notify-triage \
    -p mirada-plugin-host -p pacha-cli \
    -p agora-cli -p sandokan-cli -p sandokan-brain-daemon \
    -p rimay-voz-daemon-bin -p pam-tawasuyu

BIN="$REPO/target/release"
echo "==> instalando en el sistema (sudo)"

# Binarios.
sudo install -Dm755 "$BIN/mirada-compositor" /usr/local/bin/mirada-compositor
sudo install -Dm755 "$BIN/mirada-greeter"    /usr/local/bin/mirada-greeter
sudo install -Dm755 "$BIN/pata-llimphi"      /usr/local/bin/pata-llimphi
# El shell (shuma) y los lanzadores: así tus mejoras de shuma y del launcher
# llegan al sistema. El shell se arranca por el autostart (ver autostart.example)
# y el launcher por su atajo (Super+p) o desde la barra superior de shuma.
sudo install -Dm755 "$BIN/shuma-shell-llimphi"     /usr/local/bin/shuma-shell-llimphi
sudo install -Dm755 "$BIN/mirada-launcher"         /usr/local/bin/mirada-launcher
# El panel de control de mirada (binario `mirada-llimphi`): vista espacial
# «Prezi», menú de Atajos para ver/conmutar/duplicar los profiles de keymap, y
# las vistas de escritorio. Antes no se instalaba → los profiles eran
# inalcanzables desde el sistema. Se lanza con `mirada-llimphi`.
sudo install -Dm755 "$BIN/mirada-llimphi"          /usr/local/bin/mirada-llimphi
# CRÍTICO: mirada-ctl es la CLI que pata usa para leer los escritorios
# (`mirada-ctl workspaces`) y enfocar/cerrar ventanas. Sin él, el switcher de
# workspaces de la barra queda vacío («no hay control de workspaces») y el
# task-manager no puede activar por id. Antes no se instalaba.
sudo install -Dm755 "$BIN/mirada-ctl"              /usr/local/bin/mirada-ctl
# Portal XDG (file pickers, screenshots desde apps, tema) y el setter de
# wallpaper — también quedaban sin instalar.
sudo install -Dm755 "$BIN/mirada-portal"           /usr/local/bin/mirada-portal
sudo install -Dm755 "$BIN/mirada-wallpaper"        /usr/local/bin/mirada-wallpaper
# EL panel de control unificado (allichay): combina la config de mirada, pata y
# el sistema —cada app una pestaña—, incluida la «Vista espacial» con el Prezi.
# Es el panel donde las apps integran sus ajustes. No estaba instalado.
sudo install -Dm755 "$BIN/wawa-panel"              /usr/local/bin/wawa-panel
# Contextos de usuario (pacha): la CLI `pacha` — switch de contexto y el
# subcomando `pacha dotfiles …` (versionado + cifrado de dotfiles por contexto).
# El diente «Contextos» del wawa-panel edita `pachas.ron`; esta es la herramienta.
sudo install -Dm755 "$BIN/pacha"                   /usr/local/bin/pacha
# Identidad soberana (agora): la CLI `agora-cli` — crear/listar identidades y
# `agora-cli desbloquear`, que cachea la seed en el session keyring para que
# `pacha dotfiles …` cifre/descifre. El diente «Contextos» del panel la invoca.
sudo install -Dm755 "$BIN/agora-cli"               /usr/local/bin/agora-cli
# Plano de control (sandokan): la CLI `sandokan` — arranca/para/observa unidades
# (Linux y Wawa). `sandokan-monitor` (UI) ya se instala más abajo.
sudo install -Dm755 "$BIN/sandokan"                /usr/local/bin/sandokan
# El cerebro de reglas vivo (sandokan-cerebro): suscribe a los eventos del init
# y, cuando una regla matchea, actúa por el contrato (stop/cpu.weight/freeze).
# Sin reglas en disco es un no-op seguro. Se autostartea más abajo.
sudo install -Dm755 "$BIN/sandokan-cerebro"        /usr/local/bin/sandokan-cerebro
# Daemon de voz (rimay): par STT+TTS por socket Unix. Hoy backends mock; la
# sección «Voz» del panel lo configura. Se autoarranca on-demand por su consumidor.
sudo install -Dm755 "$BIN/voz-daemon"              /usr/local/bin/voz-daemon
# Daemon de embeddings (rimay-verbo): lo consumen pluma-semantic, khipu, chasqui.
# Trae backend fastembed (pesado) → NO se fuerza su build; se instala si ya está
# compilado (`cargo build --release -p rimay-verbo-daemon-bin`).
if [ -x "$BIN/verbo-daemon" ]; then
    sudo install -Dm755 "$BIN/verbo-daemon"        /usr/local/bin/verbo-daemon
    echo "    + verbo-daemon (embeddings)"
fi
# Módulo PAM de desbloqueo de identidad al login (pam_tawasuyu.so): copia la .so a
# /usr/lib/security y el ejemplo de config, pero NO toca /etc/pam.d (una mala
# config puede dejarte sin login). La activación es MANUAL — ver el ejemplo.
PAM_SECDIR="/usr/lib/security"
[ -d "$PAM_SECDIR" ] || PAM_SECDIR="/lib/security"
if [ -f "$BIN/libpam_tawasuyu.so" ]; then
    sudo install -Dm644 "$BIN/libpam_tawasuyu.so"  "$PAM_SECDIR/pam_tawasuyu.so"
    sudo install -Dm644 "$REPO/scripts/pam-tawasuyu.example" \
        /usr/local/share/tawasuyu/pam-tawasuyu.example
    echo "    + pam_tawasuyu.so en $PAM_SECDIR (activación MANUAL:"
    echo "      ver /usr/local/share/tawasuyu/pam-tawasuyu.example)"
fi
# Notificaciones de escritorio (org.freedesktop.Notifications):
#   · pata-notify        — el daemon; pinta los toasts y guarda el historial.
#                          Autoarranca con la sesión (lo sembramos en el
#                          autostart, más abajo) porque necesita el compositor
#                          vivo para su capa wlr-layer-shell.
#   · pata-notify-panel  — sidebar de historial AGRUPADO por el triage (on-demand).
#   · pata-notify-triage — CLI de triage semántico del historial (on-demand).
sudo install -Dm755 "$BIN/pata-notify"             /usr/local/bin/pata-notify
sudo install -Dm755 "$BIN/pata-notify-panel"       /usr/local/bin/pata-notify-panel
sudo install -Dm755 "$BIN/pata-notify-triage"      /usr/local/bin/pata-notify-triage
# El Cerebro de plugins WASM: un compositor con la lógica de escritorio hecha de
# módulos sandboxeados (layout/reactores) en ~/.config/mirada/plugins. Habilita
# la sesión «mirada · plugins». `mirada-plugin-sign` genera claves y firma los
# plugins que piden capacidades peligrosas (ver mirada-plugin-host/README.md).
sudo install -Dm755 "$BIN/mirada-plugin-host"      /usr/local/bin/mirada-plugin-host
sudo install -Dm755 "$BIN/mirada-plugin-sign"      /usr/local/bin/mirada-plugin-sign

# Apps de la suite: los binarios que LANZAN los lanzadores de la barra (botón
# Inicio, dock de mac, front panel de CDE, menú de apps). Sin esto, click en un
# lanzador hace `spawn` de un binario que no está en el PATH y NO PASA NADA.
# Instalamos sólo los que ya existan en target/release (no forzamos un build
# enorme): construilos con `cargo build --release -p <crate>` y recorré esto.
echo "==> instalando apps de la suite presentes en $BIN (para que los lanzadores funcionen)"
for app in \
    nada pluma-editor-llimphi pluma-notebook-llimphi tullpu-app-llimphi \
    takiy-app-llimphi media-app cosmos-app-llimphi dominium-app-llimphi \
    tinkuy-llimphi chaka-app-llimphi nakui-sheet-llimphi puriy raymi-app \
    supay-app-llimphi sandokan-monitor nahual-shell-llimphi
do
    if [ -x "$BIN/$app" ]; then
        sudo install -Dm755 "$BIN/$app" "/usr/local/bin/$app"
        echo "    + $app"
    fi
done

# Scripts de sesión + lanzador del DM.
sudo install -Dm755 "$MC/session/mirada-session"         /usr/local/bin/mirada-session
sudo install -Dm755 "$MC/session/mirada-session-pata"    /usr/local/bin/mirada-session-pata
sudo install -Dm755 "$MC/session/mirada-session-plugins" /usr/local/bin/mirada-session-plugins
sudo install -Dm755 "$REPO/scripts/mirada-dm"            /usr/local/bin/mirada-dm
# Supervisor de fallback: reinicia el compositor con backoff y restaura la
# sesión si cae (los scripts de arriba lo usan si está presente; el camino
# canónico arje-zero da esta supervisión por el fractal). Registra cada caída
# en el directorio de debug del compositor.
sudo install -Dm755 "$MC/session/mirada-supervise"    /usr/local/bin/mirada-supervise

# PAM del greeter (Artix/Arch: la pila system-login sirve tal cual).
sudo install -Dm644 "$REPO/shared/auth/auth-core/data/mirada" /etc/pam.d/mirada

# Sesiones para gestores de login EXTERNOS (el propio mirada las ofrece
# por sus built-ins y las filtra de esta lista, así que no duplican).
sudo install -Dm644 "$MC/session/mirada.desktop"         /usr/share/wayland-sessions/mirada.desktop
sudo install -Dm644 "$MC/session/mirada-pata.desktop"    /usr/share/wayland-sessions/mirada-pata.desktop
sudo install -Dm644 "$MC/session/mirada-plugins.desktop" /usr/share/wayland-sessions/mirada-plugins.desktop

# Las sesiones AJENAS (KDE, sway…) corren como tu usuario y necesitan tomar
# el asiento por su cuenta: te metemos en seat/video/input. Las nativas
# (mirada, mirada·pata) no lo necesitan. Requiere re-login para tomar.
for g in seat video input; do
    if getent group "$g" >/dev/null 2>&1; then
        sudo usermod -aG "$g" "$USER" 2>/dev/null || true
    fi
done

# Sembramos el daemon de notificaciones en TU autostart de mirada (idempotente,
# igual que mirada-session-pata hace con pata-llimphi). Se lanza al arrancar el
# compositor, con WAYLAND_DISPLAY puesto. El panel y el triage son on-demand, no
# se autoarrancan. (Para recibir notificaciones de apps ajenas hace falta un bus
# de sesión D-Bus; si no hay, el daemon igual corre y avisa por log.)
mkdir -p "${HOME}/.config/mirada"
AUTO="${HOME}/.config/mirada/autostart"
grep -qxF 'pata-notify' "$AUTO" 2>/dev/null || echo 'pata-notify' >> "$AUTO"
echo "==> pata-notify sembrado en $AUTO"

# El activador de contextos (pacha): sirve el socket que consumen `pacha switch`
# (lo invocan los chips de «Contexto» de pata y el diente «Contextos» del panel)
# y `pacha list`. Sin él, conmutar de contexto no hace nada. Idempotente.
grep -qxF 'pacha daemon' "$AUTO" 2>/dev/null || echo 'pacha daemon' >> "$AUTO"
echo "==> pacha daemon sembrado en $AUTO"

# El cerebro de reglas vivo (capa 3): reacciona a los eventos del init y actúa
# por el contrato. Arranca con el `cerebro.json` que sembramos abajo (Log-only
# = seguro/observable); editalo para agregar acciones de control. Idempotente.
grep -qxF 'sandokan-cerebro' "$AUTO" 2>/dev/null || echo 'sandokan-cerebro' >> "$AUTO"
echo "==> sandokan-cerebro sembrado en $AUTO"

# --- Configs de ejemplo del plano de control (sólo si faltan; no pisan los tuyos) ---
# Reglas del cerebro (eventos → acciones). El ejemplo sembrado es Log-only:
# el daemon corre y reacciona VISIBLEMENTE (journalctl) sin tocar nada. Para
# acciones de control, mirá ejemplos/cerebro.ejemplo-acciones.json.
CEREBRO_DST="${HOME}/.config/sandokan/cerebro.json"
if [ ! -e "$CEREBRO_DST" ]; then
    mkdir -p "$(dirname "$CEREBRO_DST")"
    cp "$REPO/03_ukupacha/sandokan/ejemplos/cerebro.json" "$CEREBRO_DST" \
        && echo "==> reglas del cerebro sembradas en $CEREBRO_DST (Log-only; editá para actuar)"
fi
# Reglas de métrica por contexto (Vigilante): el contexto debe existir en
# pachas.ron. El ejemplo deprioritiza/congela un slice de fondo ante carga
# sostenida; ajustá los nombres de contexto a los tuyos.
REGLAS_DST="${HOME}/.config/pacha/reglas.ron"
if [ ! -e "$REGLAS_DST" ]; then
    mkdir -p "$(dirname "$REGLAS_DST")"
    cp "$REPO/shared/pacha/ejemplos/reglas.ron" "$REGLAS_DST" \
        && echo "==> reglas de métrica sembradas en $REGLAS_DST (ajustá los contextos)"
fi

# Siembra los plugins de ejemplo en ~/.config/mirada/plugins, para que la sesión
# «mirada · plugins» arranque con algo que mostrar: el layout (right-master) y el
# reactor (terminal Super+a + atenuado por foco + auto-monocle). Idempotente:
# sólo copia los que falten, así NO pisa los que hayas editado. Los .wasm están
# precompilados en el repo (no hace falta el toolchain wasm32).
#
# OJO: `trust.ron` trae la clave DEMO PÚBLICA que firma el reactor (no es un
# secreto). Reemplazala por la tuya — `mirada-plugin-sign keygen` — para tu
# propio anillo de confianza; mientras esté, confiás en plugins firmados con esa
# clave demo. El layout no necesita firma (no importa nada del host).
PLUG_SRC="$REPO/02_ruway/mirada/mirada-plugin-host/assets"
PLUG_DST="${HOME}/.config/mirada/plugins"
mkdir -p "$PLUG_DST"
for f in example-layout.wasm example-layout.ron \
         example-reactor.wasm example-reactor.ron \
         asignador.wasm asignador.ron \
         scratchpads.wasm scratchpads.ron trust.ron; do
    if [ ! -e "$PLUG_DST/$f" ]; then
        cp "$PLUG_SRC/$f" "$PLUG_DST/$f" && echo "    + plugins/$f"
    fi
done
echo "==> plugins de ejemplo sembrados en $PLUG_DST (editá o borrá a gusto)"
echo "    · asignador: enrutador de apps — sin reglas hasta que las pongas (config"
echo "      del manifest, o visualmente en wawa-panel → Inicio → Plugins)."
echo "    · scratchpads: cajones con nombre — sin atajos hasta que los pongas."
echo "    Catálogo extra en $PLUG_SRC (copialos si los querés):"
echo "      layouts: dwindle · three-column · fibonacci · grid  (gana 1 a la vez)"
echo "      reactores: orientacion · nueva-al-maestro · media-keys · efecto-por-app"

cat <<'FIN'

==> listo.

  Como DM (igual que lo arrancaría un init), desde una TTY física:
      sudo mirada-dm
    · login por PAM (usuario del sistema)
    · elegí escritorio con  ‹  ›  (mirada · pata, mirada, mirada · plugins, o cualquiera instalado)
    · «mirada · plugins»: el Cerebro es el host de plugins WASM. Ya sembramos los
      de ejemplo en ~/.config/mirada/plugins (layout right-master + reactor). Se
      recargan en caliente: editá/agregá/quitá un .ron/.wasm y se aplica sin
      reiniciar. Catálogo y firma en 02_ruway/mirada/mirada-plugin-host/README.md.
    · salir:  Ctrl+Alt+Backspace      cambiar de consola:  Ctrl+Alt+F1…F12

  Para probar sin configurar PAM:
      sudo MIRADA_GREETER_MOCK=demo:demo mirada-dm

  Si te agregué a grupos nuevos (seat/video/input), cerrá sesión y volvé a
  entrar antes de elegir una sesión AJENA (KDE/sway); las nativas no lo piden.

  Notificaciones: el daemon `pata-notify` ya quedó en tu autostart. Probalo en
  la sesión con  `notify-send "hola" "mundo"`  (necesita un bus de sesión
  D-Bus). El historial agrupado:  `pata-notify-panel`. El triage por texto:
  `pata-notify-triage`  (o  `pata-notify-triage --aplicar`  para las acciones
  autorizadas de ~/.config/pata-notify/reglas.json).

  Un segundo DM en otra consola: cambiá de VT (Ctrl+Alt+F4) y corré `sudo mirada-dm` de nuevo.
FIN
