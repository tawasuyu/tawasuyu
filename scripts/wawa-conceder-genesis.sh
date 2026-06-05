#!/usr/bin/env bash
#
# wawa-conceder-genesis.sh — la ceremonia de capacidades del génesis (WAWA §14.1.3).
#
# Forja una `ConcesionCapacidad` firmada Ed25519 por CADA app de génesis que
# declara permisos, sembrándolas en `wawa-kernel/assets/concesiones/<nombre>.cap.obj`.
# Es el paso de operador que falta para poder flipear el kernel a modo ESTRICTO
# (`MODO_CAPACIDAD_ESTRICTO_GLOBAL = true` en wawa-kernel/src/main.rs): sin estas
# concesiones, en estricto esas apps arrancarían con CERO capacidades gateadas.
#
# La tabla de apps es la fuente de verdad VIVA: se parsea de la constante
# `GENESIS: [AppGenesis; N]` de `wawa-boot/src/main.rs`, así el script nunca
# diverge de lo que el génesis realmente siembra (incluye un guard contra drift:
# cuenta declarada vs entradas parseadas). Para cada app con `permisos != 0`
# invoca `agora-cli wawa concesion`, que firma el hash del OBJETO-bytecode
# (idéntico al que el génesis ancla) — no los bytes crudos del `.wasm`.
#
# Es OFFLINE y requiere tu seed: la pubkey de `--como` DEBE habitar
# `AGORA_AUTH_RING` de claves.rs, o el kernel rechaza la concesión.
#
# Uso:
#   scripts/wawa-conceder-genesis.sh [--como <id>] [--dry-run]
#                                    [--salida-dir <dir>] [--assets-dir <dir>]
#
#   --como <hex>       Identidad firmante: el ID HEX (64 chars) o un PREFIJO hex
#                      que matchee una sola identidad del grafo. NO es el --name de
#                      forjar-clave (concesion resuelve por hex, no por nombre).
#                      Obligatorio salvo en --dry-run.
#   --dry-run          Imprime el plan (qué firmaría) y NO firma nada.
#   --salida-dir <dir> Dónde escribir las *.cap.obj
#                      (default: 03_ukupacha/wawa/wawa-kernel/assets/concesiones).
#   --assets-dir <dir> Dónde viven los *.wasm de las apps
#                      (default: 03_ukupacha/wawa/wawa-kernel/assets).
#   AGORA_CLI=<cmd>    Override del binario agora-cli (default: lo compila en release).
#
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
BOOT_SRC="$REPO_ROOT/03_ukupacha/wawa/wawa-boot/src/main.rs"
DEFAULT_ASSETS="$REPO_ROOT/03_ukupacha/wawa/wawa-kernel/assets"

COMO=""
DRY_RUN=0
ASSETS_DIR="$DEFAULT_ASSETS"
SALIDA_DIR=""

while [[ $# -gt 0 ]]; do
    case "$1" in
        --como)       COMO="$2"; shift 2 ;;
        --dry-run)    DRY_RUN=1; shift ;;
        --salida-dir) SALIDA_DIR="$2"; shift 2 ;;
        --assets-dir) ASSETS_DIR="$2"; shift 2 ;;
        -h|--help)    awk 'NR>1 && /^#/{sub(/^# ?/,""); print; next} NR>1{exit}' "${BASH_SOURCE[0]}"; exit 0 ;;
        *) echo "argumento desconocido: $1" >&2; exit 2 ;;
    esac
done
[[ -z "$SALIDA_DIR" ]] && SALIDA_DIR="$ASSETS_DIR/concesiones"

if [[ "$DRY_RUN" -eq 0 && -z "$COMO" ]]; then
    echo "falta --como <hex>: el ID hex (o prefijo) de la identidad firmante." >&2
    echo "Su pubkey-autor debe vivir en AGORA_AUTH_RING. Usá --dry-run para ver el plan." >&2
    exit 2
fi

[[ -f "$BOOT_SRC" ]] || { echo "no encuentro la tabla génesis: $BOOT_SRC" >&2; exit 1; }

# ── Parseo de la tabla GENESIS (fuente de verdad viva) ──────────────────────
# Cuenta declarada en `GENESIS: [AppGenesis; N]` para el guard anti-drift.
DECLARADAS="$(grep -oE 'GENESIS:[[:space:]]*\[AppGenesis;[[:space:]]*[0-9]+\]' "$BOOT_SRC" \
    | grep -oE '[0-9]+' | head -1 || true)"
[[ -n "$DECLARADAS" ]] || { echo "no pude leer la cuenta declarada de GENESIS" >&2; exit 1; }

mapfile -t LINEAS < <(grep -E 'AppGenesis \{[[:space:]]*nombre:' "$BOOT_SRC")
if [[ "${#LINEAS[@]}" -ne "$DECLARADAS" ]]; then
    echo "DRIFT: la tabla declara $DECLARADAS apps pero parseé ${#LINEAS[@]}." >&2
    echo "El formato de GENESIS cambió; revisá el parseo antes de firmar nada." >&2
    exit 1
fi

# Acumula las apps con permisos: nombre|archivo|nombres_de_permiso
declare -a PLAN=()
for linea in "${LINEAS[@]}"; do
    nombre="$(sed -E 's/.*nombre:[[:space:]]*"([^"]+)".*/\1/' <<<"$linea")"
    archivo="$(sed -E 's/.*archivo:[[:space:]]*"([^"]+)".*/\1/' <<<"$linea")"
    permraw="$(sed -E 's/.*permisos:[[:space:]]*(.*)[[:space:]]*\},.*/\1/' <<<"$linea")"
    permraw="$(echo "$permraw" | sed -E 's/[[:space:]]+$//')"
    # permisos: 0 → no necesita concesión (no toca capacidades gateadas).
    [[ "$permraw" == "0" ]] && continue
    # format::PERMISO_RED | format::PERMISO_RAIZ → RED,RAIZ
    nombres="$(echo "$permraw" | sed -E 's/format::PERMISO_//g; s/[[:space:]]//g; s/\|/,/g')"
    PLAN+=("$nombre|$archivo|$nombres")
done

echo "Ceremonia de capacidades del génesis (WAWA §14.1.3)"
echo "  tabla     : $BOOT_SRC ($DECLARADAS apps, ${#PLAN[@]} con permisos)"
echo "  firmante  : --como ${COMO:-«(falta — sólo dry-run)»}  (su pubkey-autor DEBE estar en AGORA_AUTH_RING)"
echo "  assets    : $ASSETS_DIR"
echo "  salida    : $SALIDA_DIR"
echo
printf '  %-12s %-16s %s\n' "APP" "WASM" "PERMISOS"
for entrada in "${PLAN[@]}"; do
    IFS='|' read -r nombre archivo nombres <<<"$entrada"
    printf '  %-12s %-16s %s\n' "$nombre" "$archivo" "$nombres"
done
echo

if [[ "$DRY_RUN" -eq 1 ]]; then
    echo "[dry-run] no se firmó nada. Quitá --dry-run para ejecutar la ceremonia."
    exit 0
fi

# ── Binario agora-cli ───────────────────────────────────────────────────────
if [[ -n "${AGORA_CLI:-}" ]]; then
    read -r -a CLI <<<"$AGORA_CLI"
else
    echo "compilando agora-cli (release)…"
    cargo build -q --release -p agora-cli --manifest-path "$REPO_ROOT/Cargo.toml"
    CLI=("$REPO_ROOT/target/release/agora-cli")
fi

mkdir -p "$SALIDA_DIR"

# ── Firma por app ───────────────────────────────────────────────────────────
firmadas=0
for entrada in "${PLAN[@]}"; do
    IFS='|' read -r nombre archivo nombres <<<"$entrada"
    wasm="$ASSETS_DIR/$archivo"
    salida="$SALIDA_DIR/$nombre.cap.obj"
    if [[ ! -f "$wasm" ]]; then
        echo "  ⚠ $nombre: falta el wasm ($wasm) — la salté." >&2
        continue
    fi
    echo "── $nombre ($nombres) ──"
    "${CLI[@]}" wawa concesion \
        --como "$COMO" \
        --wasm "$wasm" \
        --permisos "$nombres" \
        --salida "$salida"
    firmadas=$((firmadas + 1))
    echo
done

echo "Listo: $firmadas/${#PLAN[@]} concesiones sembradas en $SALIDA_DIR"
echo
echo "Próximos pasos:"
echo "  1. Confirmá que la pubkey-AUTOR de cada concesión (línea «autor (pubkey)» de arriba)"
echo "     está en AGORA_AUTH_RING (wawa-kernel/src/claves.rs). Es la pubkey de FIRMA, que"
echo "     puede diferir del ID hex de --como (la IdentityId no es la pubkey Ed25519)."
echo "  2. Re-forjá la imagen: wawa-boot lee assets/concesiones/ y ancla cada concesión"
echo "     (leer_concesion + sembrar_concesion), poblando EntradaApp.concesion."
echo "  3. Verificá en QEMU (trace serial) que cada app resuelve sus permisos vía concesión."
echo "  4. Recién entonces: flipeá MODO_CAPACIDAD_ESTRICTO_GLOBAL = true en wawa-kernel/src/main.rs."
