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

echo "==> construyendo (release): compositor, greeter, pata, shuma, launchers, panel, ctl, portal, wallpaper y notificaciones"
cargo build --release \
    -p mirada-compositor -p mirada-greeter -p pata-llimphi \
    -p shuma-shell-llimphi -p mirada-launcher -p mirada-app-llimphi \
    -p mirada-ctl -p mirada-portal -p mirada-wallpaper -p wawa-panel-llimphi \
    -p pata-notify -p pata-notify-panel -p pata-notify-triage \
    -p mirada-plugin-host

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

cat <<'FIN'

==> listo.

  Como DM (igual que lo arrancaría un init), desde una TTY física:
      sudo mirada-dm
    · login por PAM (usuario del sistema)
    · elegí escritorio con  ‹  ›  (mirada · pata, mirada, mirada · plugins, o cualquiera instalado)
    · «mirada · plugins»: el Cerebro es el host de plugins WASM. Sin plugins en
      ~/.config/mirada/plugins se comporta como mirada normal; para sembrar los
      de ejemplo corré  ./scripts/build-mirada-plugins.sh  y copiá los assets
      (ver 02_ruway/mirada/mirada-plugin-host/README.md).
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
