#!/usr/bin/env bash
# Demo / smoke test del arranque sin parpadeo de arje.
# (ver 03_ukupacha/arje/SDD-ARRANQUE-SIN-PARPADEO.md)
#
# Bootea el kernel host por UEFI (OVMF) en QEMU: el kernel recibe el framebuffer
# GOP de OVMF → `simpledrm` crea /dev/dri/card0 → `arje-splash` lo toma reusando
# el modo vigente (sin re-modeset) y anima el splash; un greeter SIMULADO sobre
# DRM hace el handoff (Fase 2) y muestra la tarjeta apareciendo. Todo sin GPU
# en el guest (el greeter real de mirada usa EGL/GLES y necesita una GPU).
#
# Portable a cualquier Linux: encuentra OVMF, el kernel y el backend de display
# solos. Requisitos: qemu-system-x86_64, OVMF/edk2, rustup target
# x86_64-unknown-linux-musl + musl-gcc (el initramfs no trae libc → binarios
# estáticos).
#
# Uso:
#   scripts/test-arje-splash-qemu.sh                 # demo visual (ventana QEMU)
#   scripts/test-arje-splash-qemu.sh --headless      # sin ventana, veredicto por texto
#   scripts/test-arje-splash-qemu.sh --splash-only    # sólo el splash (Fase 1)
#   scripts/test-arje-splash-qemu.sh --stage-esp DIR  # copia a una ESP booteable (instalable)
# Overrides por env: KERNEL=, OVMF_CODE=, OVMF_VARS=, SEED=, TIMEOUT=.
set -euo pipefail

cd "$(git rev-parse --show-toplevel)"

MODE="visual"          # visual | headless | stage
STAGE_DIR=""
SEED="${SEED:-03_ukupacha/arje/seeds/arje-demo.card.json}"
TIMEOUT="${TIMEOUT:-25}"
TARGET="x86_64-unknown-linux-musl"

while [ $# -gt 0 ]; do
    case "$1" in
        --headless)    MODE="headless" ;;
        --splash-only) SEED="03_ukupacha/arje/seeds/arje-qemu.card.json" ;;
        --stage-esp)   MODE="stage"; STAGE_DIR="${2:?--stage-esp requiere un directorio}"; shift ;;
        -h|--help)     sed -n '2,24p' "$0"; exit 0 ;;
        *) echo "opción desconocida: $1" >&2; exit 2 ;;
    esac
    shift
done

die() { echo "✗ $*" >&2; exit 1; }
have() { command -v "$1" >/dev/null 2>&1; }

# ── Dependencias ────────────────────────────────────────────────────────────
have qemu-system-x86_64 || die "falta qemu-system-x86_64 (instalá 'qemu' / 'qemu-system-x86' / 'qemu-full')"
rustup target list --installed 2>/dev/null | grep -qx "$TARGET" \
    || die "falta el target Rust: rustup target add $TARGET"
have musl-gcc || have x86_64-linux-musl-gcc \
    || die "falta musl-gcc (instalá 'musl' / 'musl-tools')"

# ── Kernel ──────────────────────────────────────────────────────────────────
find_kernel() {
    [ -n "${KERNEL:-}" ] && { echo "$KERNEL"; return; }
    for k in /boot/vmlinuz-linux /boot/vmlinuz-linux-lts /boot/vmlinuz; do
        [ -f "$k" ] && { echo "$k"; return; }
    done
    ls -1t /boot/vmlinuz* 2>/dev/null | head -1
}
KERNEL="$(find_kernel)"
[ -n "$KERNEL" ] && [ -f "$KERNEL" ] || die "no encontré un kernel; pasá KERNEL=/ruta/vmlinuz"

# ── OVMF (firmware UEFI) — busca en las rutas de las distros más comunes ─────
find_first() { for p in "$@"; do [ -f "$p" ] && { echo "$p"; return; }; done; }
OVMF_CODE="${OVMF_CODE:-$(find_first \
    /usr/share/edk2/x64/OVMF_CODE.4m.fd \
    /usr/share/edk2/ovmf/OVMF_CODE.fd \
    /usr/share/OVMF/OVMF_CODE_4M.fd \
    /usr/share/OVMF/OVMF_CODE.fd \
    /usr/share/edk2-ovmf/x64/OVMF_CODE.fd \
    /usr/share/qemu/ovmf-x86_64-code.bin )}"
OVMF_VARS="${OVMF_VARS:-$(find_first \
    /usr/share/edk2/x64/OVMF_VARS.4m.fd \
    /usr/share/edk2/ovmf/OVMF_VARS.fd \
    /usr/share/OVMF/OVMF_VARS_4M.fd \
    /usr/share/OVMF/OVMF_VARS.fd \
    /usr/share/edk2-ovmf/x64/OVMF_VARS.fd \
    /usr/share/qemu/ovmf-x86_64-vars.bin )}"
[ -n "$OVMF_CODE" ] && [ -n "$OVMF_VARS" ] \
    || die "no encontré OVMF; instalá 'edk2-ovmf' / 'ovmf' o pasá OVMF_CODE= y OVMF_VARS="

# ── Compilar estáticos + empaquetar el initramfs ────────────────────────────
echo "==> compilando binarios estáticos (musl)"
cargo build --release --target "$TARGET" -p arje-zero -p arje-splash -p arje-getty-stub
cargo build --release -p arje-packager
M="target/$TARGET/release"

OUT="$(mktemp -d)"
echo "==> artefactos en $OUT (initramfs + log; podés borrarlo cuando quieras)"
echo "==> empaquetando initramfs ($SEED)"
# Pasamos todos los bins posibles; el packager toma sólo los que el seed declara.
./target/release/arje-packager \
    --seed "$SEED" \
    --bin arje-zero="$M/arje-zero" \
    --bin arje-splash="$M/arje-splash" \
    --bin greeter-sim="$M/arje-splash" \
    --bin agetty-ttyS0="$M/arje-getty-stub" \
    --out "$OUT/initramfs.cpio.gz"

# ── Modo stage: copiar a una ESP booteable y salir ──────────────────────────
if [ "$MODE" = "stage" ]; then
    echo "==> copiando kernel + initramfs + seed a la ESP en $STAGE_DIR"
    ./target/release/arje-installer to-partition \
        --esp "$STAGE_DIR" --kernel "$KERNEL" --seed "$SEED" \
        --bin arje-zero="$M/arje-zero" \
        --bin arje-splash="$M/arje-splash" \
        --bin greeter-sim="$M/arje-splash" \
        --bin agetty-ttyS0="$M/arje-getty-stub"
    echo "✓ ESP lista en $STAGE_DIR — booteá esa partición por UEFI."
    exit 0
fi

# ── Firmware writable (copia de VARS) ───────────────────────────────────────
cp -f "$OVMF_VARS" "$OUT/vars.fd"

# Flicker-free: el kernel no escribe sobre el framebuffer; printk va al serial.
APPEND="console=ttyS0,115200 panic=-1 loglevel=4 vt.global_cursor_default=0 i915.fastboot=1"

ACCEL=(); [ -w /dev/kvm ] && ACCEL=(-enable-kvm)
QEMU=(qemu-system-x86_64 -machine q35 -m 512M "${ACCEL[@]}"
      -drive if=pflash,format=raw,readonly=on,file="$OVMF_CODE"
      -drive if=pflash,format=raw,file="$OUT/vars.fd"
      -kernel "$KERNEL" -initrd "$OUT/initramfs.cpio.gz" -append "$APPEND"
      -vga std -no-reboot)

# ── Elegir backend de display ───────────────────────────────────────────────
pick_display() {
    [ -n "${DISPLAY:-}${WAYLAND_DISPLAY:-}" ] || return 1
    local help; help="$(qemu-system-x86_64 -display help 2>&1 || true)"
    for d in gtk sdl; do echo "$help" | grep -qw "$d" && { echo "$d"; return 0; }; done
    return 1
}

if [ "$MODE" = "headless" ] || ! DISP="$(pick_display)"; then
    [ "$MODE" = "headless" ] || echo "⚠ sin display gráfico disponible — caigo a headless"
    echo "==> QEMU headless (timeout ${TIMEOUT}s) — veredicto por texto"
    set +e
    timeout "$TIMEOUT" "${QEMU[@]}" -display none -serial mon:stdio </dev/null \
        | tee "$OUT/serial.log"
    set -e
    echo; echo "==> veredicto (logs de arje-splash):"
    grep -aE 'arje-splash|simpledrm|reusando modo|READY|RELEASED|fade|greeter|Ente (encarnado|disuelto)' \
        "$OUT/serial.log" | sed 's/\x1b\[[0-9;]*m//g' || echo "  (revisá $OUT/serial.log)"
else
    echo "==> QEMU con ventana ($DISP). Vas a ver: splash respirando → fade → tarjeta del greeter."
    echo "    (los logs van a $OUT/serial.log; cerrá la ventana o esperá el timeout de ${TIMEOUT}s)"
    timeout "$TIMEOUT" "${QEMU[@]}" -display "$DISP" -serial "file:$OUT/serial.log" </dev/null || true
    echo "✓ fin. Log del arranque en $OUT/serial.log"
fi
