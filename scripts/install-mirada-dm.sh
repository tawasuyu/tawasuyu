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
#   /usr/local/bin/{shuma-shell-llimphi,mirada-launcher,mirada-launcher-llimphi}
#   /usr/local/bin/{mirada-session,mirada-session-pata,mirada-dm}
#   /etc/pam.d/carmen                              (login del greeter)
#   /usr/share/wayland-sessions/{carmen,mirada-pata}.desktop
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

echo "==> construyendo (release): compositor, greeter, pata, shuma y launchers"
cargo build --release \
    -p mirada-compositor -p mirada-greeter -p pata-llimphi \
    -p shuma-shell-llimphi -p mirada-launcher -p mirada-launcher-llimphi

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
sudo install -Dm755 "$BIN/mirada-launcher-llimphi" /usr/local/bin/mirada-launcher-llimphi

# Scripts de sesión + lanzador del DM.
sudo install -Dm755 "$MC/session/mirada-session"      /usr/local/bin/mirada-session
sudo install -Dm755 "$MC/session/mirada-session-pata" /usr/local/bin/mirada-session-pata
sudo install -Dm755 "$REPO/scripts/mirada-dm"         /usr/local/bin/mirada-dm

# PAM del greeter (Artix/Arch: la pila system-login sirve tal cual).
sudo install -Dm644 "$REPO/shared/auth/auth-core/data/carmen" /etc/pam.d/carmen

# Sesiones para gestores de login EXTERNOS (el propio mirada las ofrece
# por sus built-ins y las filtra de esta lista, así que no duplican).
sudo install -Dm644 "$MC/session/carmen.desktop"      /usr/share/wayland-sessions/carmen.desktop
sudo install -Dm644 "$MC/session/mirada-pata.desktop" /usr/share/wayland-sessions/mirada-pata.desktop

# Las sesiones AJENAS (KDE, sway…) corren como tu usuario y necesitan tomar
# el asiento por su cuenta: te metemos en seat/video/input. Las nativas
# (mirada, mirada·pata) no lo necesitan. Requiere re-login para tomar.
for g in seat video input; do
    if getent group "$g" >/dev/null 2>&1; then
        sudo usermod -aG "$g" "$USER" 2>/dev/null || true
    fi
done

cat <<'FIN'

==> listo.

  Como DM (igual que lo arrancaría un init), desde una TTY física:
      sudo mirada-dm
    · login por PAM (usuario del sistema)
    · elegí escritorio con  ‹  ›  (mirada · pata, mirada, o cualquiera instalado)
    · salir:  Ctrl+Alt+Backspace      cambiar de consola:  Ctrl+Alt+F1…F12

  Para probar sin configurar PAM:
      sudo MIRADA_GREETER_MOCK=demo:demo mirada-dm

  Si te agregué a grupos nuevos (seat/video/input), cerrá sesión y volvé a
  entrar antes de elegir una sesión AJENA (KDE/sway); las nativas no lo piden.

  Un segundo DM en otra consola: cambiá de VT (Ctrl+Alt+F4) y corré `sudo mirada-dm` de nuevo.
FIN
