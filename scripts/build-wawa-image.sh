#!/usr/bin/env bash
# =============================================================================
#  scripts/build-wawa-image.sh — el creador de la imagen publicable de wawa
# -----------------------------------------------------------------------------
#  Forja, de punta a punta, la imagen QEMU/USB distribuible de Wawa OS con el
#  userspace de genesis completo (las apps del monorepo que ya tienen reflejo
#  bare-metal: pluma, ayni, rimay, testigo/tinkuy, asistente, bitacora, ...).
#
#  Tres etapas encadenables:
#
#    apps    Recompila TODAS las apps WASM de `03_ukupacha/wawa/apps/` y las
#            consolida (con wasm-opt si esta disponible) en
#            `wawa-kernel/assets/` — el directorio que `boot` lee al sembrar
#            el grafo de genesis. Generaliza lo que `build-pluma.sh` hacia
#            para una sola app.
#
#    imagen  Invoca `cargo +nightly run -p boot -Z bindeps -- --forjar`:
#            compila el kernel (x86_64-unknown-none), siembra el grafo con
#            los .wasm de assets/ y fusiona todo en una imagen UEFI con el
#            grafo embebido como ramdisk — autocontenida, sin depender de
#            virtio-blk ni de archivos al costado.
#
#    dist    Empaqueta lo publicable en `dist/wawa-<fecha>-<sha>/`:
#            wawa.img + correr.sh (lanzador QEMU portable que localiza OVMF
#            en las rutas de Arch/Debian/Fedora) + LEEME.md + SHA256SUMS,
#            y un tarball .tar.zst listo para adjuntar a un release.
#
#  Uso:
#     ./scripts/build-wawa-image.sh            # todo: apps + imagen + dist
#     ./scripts/build-wawa-image.sh apps       # solo reconstruir userspace
#     ./scripts/build-wawa-image.sh imagen     # solo forjar la imagen UEFI
#     ./scripts/build-wawa-image.sh dist       # solo empaquetar (imagen ya forjada)
#
#  El ciclo de actualizacion es trivial: tocas una app del monorepo, corres
#  el script sin argumentos, y `dist/` tiene la imagen nueva. Nada que
#  recordar, nada que sembrar a mano.
#
#  Toolchain requerido: nightly con rust-src, targets wasm32-unknown-unknown
#  y x86_64-unknown-none. wasm-opt (binaryen) es OPCIONAL: sin el, los .wasm
#  van crudos (mas gordos, identica semantica) y el script avisa.
# =============================================================================

set -euo pipefail

RAIZ="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
WAWA="$RAIZ/03_ukupacha/wawa"
APPS="$WAWA/apps"
ASSETS="$WAWA/wawa-kernel/assets"
DIST_BASE="$RAIZ/dist"

if [ -t 1 ]; then
    ROJO='\033[31m'; VERDE='\033[32m'; AMARILLO='\033[33m'; AZUL='\033[36m'; RESET='\033[0m'
else
    ROJO=''; VERDE=''; AMARILLO=''; AZUL=''; RESET=''
fi

decir() { echo -e "${AZUL}[wawa/imagen]${RESET} $*"; }
avisar() { echo -e "${AMARILLO}AVISO${RESET} $*"; }
fallar() { echo -e "${ROJO}FALLO${RESET} $*"; exit 1; }

# --- Localizar wasm-opt (opcional, mismas rutas que build-pluma.sh) ----------
WASM_OPT=""
for cand in \
        "$(command -v wasm-opt 2>/dev/null || true)" \
        "$HOME/.cargo/bin/wasm-opt" \
        "/opt/flutter/bin/cache/dart-sdk/bin/utils/wasm-opt" \
        "$HOME/.cache/flutter_sdk/bin/cache/dart-sdk/bin/utils/wasm-opt"; do
    if [ -n "$cand" ] && [ -x "$cand" ]; then
        WASM_OPT="$cand"
        break
    fi
done

# Consolida un .wasm crudo en assets/, optimizado si hay wasm-opt.
consolidar() {
    local crudo="$1" destino="$2"
    if [ -n "$WASM_OPT" ]; then
        # rustc emite por defecto bulk-memory, sign-ext y trunc_sat (nontrapping
        # float-to-int); wasm-opt rechaza validarlos sin habilitar cada feature.
        "$WASM_OPT" -Os --strip-debug --strip-producers --strip-target-features \
            --enable-bulk-memory --enable-sign-ext --enable-nontrapping-float-to-int \
            "$crudo" -o "$destino"
    else
        cp "$crudo" "$destino"
    fi
}

# =============================================================================
#  Etapa 1 :: apps — recompilar el userspace completo hacia assets/
# =============================================================================
etapa_apps() {
    [ -n "$WASM_OPT" ] && decir "wasm-opt :: $WASM_OPT" \
        || avisar "wasm-opt no encontrado — los .wasm van crudos (instala binaryen para sellarlos)"

    local total=0
    for dir in "$APPS"/*/; do
        local nombre; nombre="$(basename "$dir")"
        [ -f "$dir/Cargo.toml" ] || continue

        # El crate `hello_wasm` se consolida como `app.wasm`: es el nombre que
        # la tabla GENESIS de boot referencia para la app `hola` desde Fase 7b.
        local destino="$nombre.wasm"
        [ "$nombre" = "hello_wasm" ] && destino="app.wasm"

        decir "cargo build --release :: apps/$nombre"
        (cd "$dir" && cargo build --release --target wasm32-unknown-unknown --quiet)

        local crudo="$dir/target/wasm32-unknown-unknown/release/$nombre.wasm"
        [ -f "$crudo" ] || fallar "apps/$nombre no produjo $crudo"

        consolidar "$crudo" "$ASSETS/$destino"
        local tam; tam=$(stat -c '%s' "$ASSETS/$destino")
        decir "  consolidado :: assets/$destino ($tam bytes)"
        total=$((total + 1))
    done
    echo -e "${VERDE}OK${RESET}    $total apps consolidadas en wawa-kernel/assets/"
}

# =============================================================================
#  Etapa 2 :: imagen — forjar la UEFI autocontenida (kernel + ramdisk)
# =============================================================================
IMAGEN=""

localizar_imagen() {
    # `boot --forjar` deja `renaser-uefi.img` junto al ELF del kernel, dentro
    # de target/. La mas reciente es la que acabamos de forjar.
    IMAGEN="$(find "$WAWA/target" -name 'renaser-uefi.img' -printf '%T@ %p\n' 2>/dev/null \
        | sort -rn | head -1 | cut -d' ' -f2-)"
}

etapa_imagen() {
    # --release: la dependencia de artefacto hereda el perfil, asi que el
    # kernel sale con opt-level=s + lto (la imagen publicable, no la de debug).
    decir "cargo +nightly run -p boot --release -Z bindeps -- --forjar"
    (cd "$WAWA" && cargo +nightly run -p boot --release -Z bindeps -- --forjar) \
        || fallar "la forja de la imagen UEFI fallo (¿nightly con rust-src y target x86_64-unknown-none?)"

    localizar_imagen
    [ -n "$IMAGEN" ] && [ -f "$IMAGEN" ] || fallar "boot no dejo renaser-uefi.img bajo $WAWA/target"
    local tam; tam=$(du -h "$IMAGEN" | cut -f1)
    echo -e "${VERDE}OK${RESET}    imagen forjada :: $IMAGEN ($tam)"
}

# =============================================================================
#  Etapa 3 :: dist — empaquetar lo publicable
# =============================================================================
etapa_dist() {
    if [ -z "$IMAGEN" ]; then
        localizar_imagen
        [ -n "$IMAGEN" ] && [ -f "$IMAGEN" ] \
            || fallar "no hay renaser-uefi.img forjada — corre antes la etapa 'imagen'"
    fi

    local fecha sha version
    fecha="$(date +%Y%m%d)"
    sha="$(git -C "$RAIZ" rev-parse --short HEAD 2>/dev/null || echo sinrepo)"
    version="wawa-$fecha-$sha"
    local out="$DIST_BASE/$version"

    rm -rf "$out"
    mkdir -p "$out"
    cp "$IMAGEN" "$out/wawa.img"

    # --- El lanzador portable: espejo de `lanzar_qemu` de boot, sin el drive
    #     virtio-blk (el grafo viaja embebido como ramdisk). ---
    cat > "$out/correr.sh" <<'LANZADOR'
#!/usr/bin/env bash
# correr.sh — arranca Wawa OS en QEMU. Requiere qemu-system-x86_64 y firmware
# UEFI OVMF (paquete `edk2-ovmf` en Arch/Artix, `ovmf` en Debian/Ubuntu/Fedora).
# Sobreescribe la ruta del firmware con WAWA_OVMF=<ruta> si vive en otro lado.
set -euo pipefail
AQUI="$(cd "$(dirname "${BASH_SOURCE[0]}")" && pwd)"

OVMF="${WAWA_OVMF:-}"
if [ -z "$OVMF" ]; then
    for cand in \
            /usr/share/edk2/x64/OVMF.4m.fd \
            /usr/share/edk2/x64/OVMF.fd \
            /usr/share/edk2-ovmf/x64/OVMF.fd \
            /usr/share/OVMF/OVMF.fd \
            /usr/share/ovmf/OVMF.fd \
            /usr/share/edk2/ovmf/OVMF.fd \
            /usr/share/qemu/OVMF.fd; do
        [ -f "$cand" ] && OVMF="$cand" && break
    done
fi
if [ -z "$OVMF" ]; then
    echo "FALLO: firmware UEFI OVMF no encontrado." >&2
    echo "  Instala edk2-ovmf (Arch) / ovmf (Debian, Fedora), o exporta WAWA_OVMF=<ruta a OVMF.fd>." >&2
    exit 1
fi

# Audio: elegir el primer backend disponible para que la voz del kernel
# (el acorde de bienvenida y los repiques) se oiga de verdad. Sin backend
# utilizable cae a `none` (silencio, pero el dispositivo existe igual).
# Forzable con WAWA_AUDIO=<backend> (pipewire|pa|alsa|sdl|none).
AUDIO="${WAWA_AUDIO:-}"
if [ -z "$AUDIO" ]; then
    AUDIO=none
    for backend in pipewire pa alsa sdl; do
        if qemu-system-x86_64 -audiodev help 2>/dev/null | grep -qx "$backend"; then
            AUDIO="$backend"
            break
        fi
    done
fi

# accel=kvm:tcg intenta KVM y recae en emulacion pura si no hay /dev/kvm.
# El grafo de objetos viaja DENTRO de la imagen (ramdisk): sin discos extra.
# Nota Arch/Artix: si QEMU segfaultea al arrancar, falta el paquete
# qemu-hw-display-virtio-vga (el device virtio-vga vive en modulo aparte).
exec qemu-system-x86_64 \
    -machine q35,accel=kvm:tcg \
    -m 256M \
    -bios "$OVMF" \
    -drive "format=raw,file=$AQUI/wawa.img" \
    -vga none \
    -device virtio-vga \
    -device virtio-tablet-pci \
    -audiodev "$AUDIO,id=snd0" \
    -device virtio-sound-pci,audiodev=snd0 \
    -serial stdio \
    --no-reboot \
    "$@"
LANZADOR
    chmod +x "$out/correr.sh"

    # --- El LEEME del paquete. ---
    cat > "$out/LEEME.md" <<LEEME
# Wawa OS — imagen de demostracion ($version)

Wawa es un sistema operativo bare-metal soberano: kernel propio en Rust,
userspace de apps WASM aisladas por capacidades, almacenamiento direccionado
por contenido (BLAKE3) y protocolo de red propio (akasha) sin TCP/IP.
Fuente: https://git.tawasuyu.net/tawasuyu/tawasuyu (\`03_ukupacha/wawa\`).

## Arrancar en QEMU

\`\`\`sh
./correr.sh
\`\`\`

Requisitos: \`qemu-system-x86_64\` + firmware OVMF (\`edk2-ovmf\` en
Arch/Artix, \`ovmf\` en Debian/Ubuntu/Fedora). El lanzador localiza el
firmware solo; si vive en una ruta rara: \`WAWA_OVMF=/ruta/OVMF.fd ./correr.sh\`.
Los argumentos extra se reenvian a QEMU (\`./correr.sh -display sdl\`).

## Arrancar en metal real

La imagen es UEFI booteable tal cual:

\`\`\`sh
sudo dd if=wawa.img of=/dev/sdX bs=4M conv=fsync status=progress
\`\`\`

## Que trae

El userspace de genesis completo: \`pluma\` (notebook con celdas Forth
ejecutables), \`ayni\` (chat P2P firmado Ed25519), \`bitacora\` (editor que
persiste), \`asistente\` (puente conversacional a LLMs), \`rimay\`
(embeddings deterministas), \`testigo\` (simulacion fisica tinkuy en el
kernel), \`mudanza\` (reancla soberana del manifiesto), \`pregon\`, \`tonada\`,
\`pulso\`, \`memoriosa\`, \`tonalero\`, \`cronista\`, \`discola\`, \`glotona\`, \`hola\`.

El grafo de objetos viaja embebido como ramdisk: la sesion es efimera
(nada persiste entre arranques). Para persistencia real sobre virtio-blk,
arranca desde el repo con \`cargo +nightly run -p boot -Z bindeps\`.

Esta imagen se forjo con \`scripts/build-wawa-image.sh\` en el commit \`$sha\`.
LEEME

    (cd "$out" && sha256sum wawa.img correr.sh > SHA256SUMS)

    # --- El tarball para el release. ---
    local tarball="$DIST_BASE/$version.tar.zst"
    if command -v zstd >/dev/null 2>&1; then
        tar -C "$DIST_BASE" --zstd -cf "$tarball" "$version"
        local tam; tam=$(du -h "$tarball" | cut -f1)
        echo -e "${VERDE}OK${RESET}    publicable :: $tarball ($tam)"
    else
        avisar "zstd no encontrado — queda el directorio sin comprimir"
    fi
    echo -e "${VERDE}OK${RESET}    publicable :: $out/ (wawa.img + correr.sh + LEEME.md + SHA256SUMS)"
}

# =============================================================================
#  Despacho
# =============================================================================
ETAPA="${1:-todo}"
case "$ETAPA" in
    apps)   etapa_apps ;;
    imagen) etapa_imagen ;;
    dist)   etapa_dist ;;
    todo)   etapa_apps; etapa_imagen; etapa_dist ;;
    *) fallar "etapa desconocida «$ETAPA» — usa: apps | imagen | dist | todo" ;;
esac
