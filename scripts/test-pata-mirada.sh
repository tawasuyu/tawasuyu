#!/bin/sh
# test-pata-mirada.sh — verificación de humo de pata sobre mirada (single-GPU).
#
# No automatiza el login (el DM es interactivo): vos arrancás la sesión, y el
# script chequea (a) que el binario instalado sea el recién compilado y (b) que
# el log de la sesión tenga los marcadores de que pata pintó bien, sin la
# regresión del adaptador / WSI.
#
# Uso típico en la máquina de prueba:
#   git pull && ./scripts/install-mirada-dm.sh        # traer + instalar los fixes
#   ./scripts/test-pata-mirada.sh setup               # deja PATA_DIAG en el autostart
#   sudo mirada-dm                                     # login, mirá las barras ~5s,
#                                                      #   salí con Ctrl+Alt+Backspace
#   ./scripts/test-pata-mirada.sh check                # PASS/FAIL contra /tmp/mirada.log
#
# `check` (o sin argumento) es lo normal. Acepta un log alternativo:
#   ./scripts/test-pata-mirada.sh check /ruta/al/mirada.log

set -u

REPO="$(CDPATH= cd -- "$(dirname -- "$0")/.." && pwd)"
cmd="${1:-check}"
LOG="${2:-/tmp/mirada.log}"
BIN_MC=/usr/local/bin/mirada-compositor
BIN_PATA=/usr/local/bin/pata-llimphi

ok()   { printf '  \033[32m✓\033[0m %s\n' "$1"; }
bad()  { printf '  \033[31m✗\033[0m %s\n' "$1"; FAILED=1; }
warn() { printf '  \033[33m·\033[0m %s\n' "$1"; }

# --- setup: deja PATA_DIAG en el autostart del usuario ---------------------
if [ "$cmd" = "setup" ]; then
    mkdir -p "$HOME/.config/mirada"
    AUTO="$HOME/.config/mirada/autostart"
    if [ -f "$AUTO" ] && grep -q 'pata-llimphi' "$AUTO"; then
        warn "ya hay un pata-llimphi en $AUTO — revisalo a mano si querés PATA_DIAG"
    else
        printf 'PATA_DIAG=1 pata-llimphi\n' > "$AUTO"
        ok "escrito $AUTO  ->  'PATA_DIAG=1 pata-llimphi'"
    fi
    echo
    echo "Ahora:  sudo mirada-dm   (login, mirá las barras ~5s, Ctrl+Alt+Backspace)"
    echo "Después: ./scripts/test-pata-mirada.sh check"
    exit 0
fi

if [ "$cmd" != "check" ]; then
    echo "uso: $0 [setup|check] [log]" >&2
    exit 2
fi

FAILED=0
echo "== Binario instalado == (¿es el recién compilado?)"
# Si hay build en target/release, comparamos byte a byte: la forma más fiable
# de saber que el install reemplazó el binario que corre el DM.
for pair in "mirada-compositor:$BIN_MC" "pata-llimphi:$BIN_PATA"; do
    name="${pair%%:*}"; inst="${pair#*:}"
    built="$REPO/target/release/$name"
    if [ ! -x "$inst" ]; then
        bad "$name no está instalado en $inst"
    elif [ -x "$built" ] && cmp -s "$built" "$inst"; then
        ok "$name instalado == target/release (al día)"
    elif [ -x "$built" ]; then
        bad "$name instalado DIFIERE de target/release — corré ./scripts/install-mirada-dm.sh"
    else
        warn "$name instalado, sin target/release para comparar (¿build release?)"
    fi
done

echo
echo "== Log de la sesión == ($LOG)"
if [ ! -r "$LOG" ]; then
    bad "no puedo leer $LOG — ¿corriste 'sudo mirada-dm' (con setup) y entraste a la sesión?"
    echo; echo "RESULTADO: \033[31mFALLA\033[0m (sin log)"; exit 1
fi

has()  { grep -qiF "$1" "$LOG"; }

# (a) dmabuf v4 con feedback y formatos > 0
line="$(grep -iF 'dmabuf v4 (feedback)' "$LOG" | tail -1)"
if [ -n "$line" ]; then
    n="$(printf '%s' "$line" | grep -oE '[0-9]+ format' | grep -oE '[0-9]+' | head -1)"
    if [ "${n:-0}" -gt 0 ] 2>/dev/null; then
        ok "dmabuf v4 con feedback ($n formatos)"
    else
        bad "dmabuf v4 anunciado pero con 0 formatos"
    fi
else
    bad "no se anunció dmabuf v4 (feedback) — ¿mirada viejo?"
fi

# (b) pata arrancó el backend layer-shell
has 'backend LAYER-SHELL arranca' && ok "pata arrancó en layer-shell" \
    || bad "pata no arrancó en layer-shell (¿cayó a winit?)"

# (c) creó las surfaces de GPU (la regresión del adaptador rompería esto)
has 'surface creada' && ok "pata creó la(s) surface(s) de GPU" \
    || bad "pata NO creó surface de GPU"

# (d) presentó ambos paneles (las dos barras pintaron)
has 'present panel 0' && ok "panel 0 presentado (barra superior)" \
    || bad "panel 0 no presentó"
has 'present panel 1' && ok "panel 1 presentado (barra inferior)" \
    || bad "panel 1 no presentó"

# (e) marcadores de FRACASO que NO deben aparecer
echo
echo "== Sin regresiones =="
has 'sin gpu'            && bad "aparece 'sin gpu' (panel sin adaptador)"      || ok "ningún panel quedó sin gpu"
has 'no expone formatos' && bad "aparece 'no expone formatos' (WSI/adaptador)" || ok "ningún '0 formatos'"
has 'layer-shell falló'  && bad "pata cayó del layer-shell a winit"           || ok "no cayó a winit"
has 'panicked'           && bad "hubo un panic"                               || ok "sin panics"

echo
if [ "$FAILED" -eq 0 ]; then
    printf 'RESULTADO: \033[32mPASA\033[0m — pata pinta en mirada sin regresiones.\n'
    exit 0
else
    printf 'RESULTADO: \033[31mFALLA\033[0m — revisá los ✗ de arriba y pegá el log.\n'
    exit 1
fi
