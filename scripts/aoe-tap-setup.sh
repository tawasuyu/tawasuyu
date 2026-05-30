#!/usr/bin/env bash
#
# aoe-tap-setup.sh — forja (o derriba) el dispositivo TAP que transporta Akasha
# Over Ether entre el host y un wawa en QEMU.
#
# El NAT user-mode de QEMU solo reenvia IP: el EtherType propio de AoE (0x88B5)
# no lo cruza. Un TAP, en cambio, es un cable de capa-2 puro — transporta
# cualquier EtherType. Con el tap arriba:
#
#   * QEMU bridgea la NIC del guest al tap:   RENASER_TAP=tap0 cargo run -p boot ...
#   * el host difunde AoE sobre el mismo tap: sudo -E agora-cli wawa anunciar \
#                                                  --iface tap0 --dir <release>
#
# Crear/derribar un TAP exige CAP_NET_ADMIN (root). Lo hacemos con `sudo ip`.
# El tap queda en propiedad de $USER, asi QEMU lo abre SIN privilegios y el raw
# socket del anunciador tambien (con CAP_NET_RAW / sudo -E).
#
# Uso:
#   scripts/aoe-tap-setup.sh            # crea tap0 (o $RENASER_TAP) y lo deja UP
#   scripts/aoe-tap-setup.sh down       # lo derriba
#
set -euo pipefail

TAP="${RENASER_TAP:-tap0}"
ACCION="${1:-up}"

case "$ACCION" in
    up)
        if ip link show "$TAP" >/dev/null 2>&1; then
            echo "[aoe-tap] «$TAP» ya existe — lo dejo como esta."
        else
            echo "[aoe-tap] creando «$TAP» (mode tap, owner $USER)…"
            sudo ip tuntap add "$TAP" mode tap user "$USER"
        fi
        sudo ip link set "$TAP" up
        echo "[aoe-tap] «$TAP» arriba. Ahora:"
        echo "    RENASER_TAP=$TAP cargo +nightly run -p boot -Z bindeps   # en 03_ukupacha/wawa"
        echo "    sudo -E agora-cli wawa anunciar --iface $TAP --dir <release>"
        ;;
    down)
        if ip link show "$TAP" >/dev/null 2>&1; then
            echo "[aoe-tap] derribando «$TAP»…"
            sudo ip link set "$TAP" down || true
            sudo ip tuntap del "$TAP" mode tap
        else
            echo "[aoe-tap] «$TAP» no existe — nada que derribar."
        fi
        ;;
    *)
        echo "uso: $0 [up|down]   (TAP via \$RENASER_TAP, defecto tap0)" >&2
        exit 2
        ;;
esac
