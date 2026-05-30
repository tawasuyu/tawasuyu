#!/usr/bin/env bash
#
# test-mudanza-aoe-qemu.sh — E2E del CAMINO VIVO de mudanza: un release firmado
# viaja del HOST a un wawa en QEMU por Akasha-Over-Ether (sin IP), el guest lo
# absorbe, lo anuncia en `mudanza`, y el operador lo ACEPTA o lo RECHAZA.
#
# Es el gemelo "en red" de `test-mudanza-qemu.sh` (que prueba el sobre demo
# EMBEBIDO, camino offline). Aqui el sobre NO esta horneado en el binario:
# llega por el cable, como en dos maquinas reales que se actualizan entre si.
#
# El autor lo corre a mano: (1) QEMU necesita display vivo, (2) `boot` panica en
# sandbox por el build.rs de bootloader-x86_64-uefi, (3) el TAP y el raw socket
# exigen root. Este script automatiza TODO lo host-side (build + keystore +
# publicar + tap) y deja impresos los dos comandos finales que corren en
# paralelo: arrancar QEMU bridgeado al tap, y anunciar el release sobre el tap.
#
# La pieza que este flujo ejercita y el offline no:  boot::lanzar_qemu con
# RENASER_TAP bridgea la NIC del guest a un TAP de capa-2 que SI transporta el
# EtherType 0x88B5 de AoE (el NAT user-mode de QEMU solo reenvia IP y lo tira).
#
# Comportamiento esperado en pantalla:
#   - mudanza pinta "PROPUESTA EN RED (AKASHA)" + la raiz  => el frame CRUZO.
#   - SPACE: si la identidad firmante esta en AGORA_AUTH_RING, "OK :: REANCLADO";
#            si no (p.ej. la demo [42u8;32]), "AUTOR AJENO :: RECHAZADO".
#   - ESC:   "PROPUESTA RECHAZADA" y la propuesta desaparece (hasta el proximo
#            anuncio del host, que la repinta — el host re-emite en loop).
#
set -euo pipefail

cd "$(dirname "$0")/.."
ROOT="$(pwd)"
TAP="${RENASER_TAP:-tap0}"

# --- 1. agora-cli --------------------------------------------------------------
if ! [ -x "$ROOT/target/release/agora-cli" ]; then
    echo "[aoe-test] compilando agora-cli…"
    cargo build -p agora-cli --release
fi
CLI="$ROOT/target/release/agora-cli"

# --- 2. keystore aislado + identidad firmante ----------------------------------
# Por defecto la seed demo [42u8;32]: el guest VE la propuesta (prueba el bridge)
# pero la RECHAZA al aceptar porque su pubkey no esta en el anillo. Para un accept
# real, exporta AOE_SEED_HEX con una seed cuya pubkey hayas empotrado en
# wawa-kernel/src/claves.rs:AGORA_AUTH_RING (la imprime `wawa forjar-clave`).
DEMO_HOME=$(mktemp -d)
export HOME="$DEMO_HOME"
export XDG_DATA_HOME="$DEMO_HOME/.local/share"
export AGORA_PASSPHRASE="${AGORA_PASSPHRASE:-demo-mudanza-aoe}"
SEED_HEX="${AOE_SEED_HEX:-$(printf '2a%.0s' {1..32})}"   # 0x2a=42 ×32 por defecto

echo "$SEED_HEX" | "$CLI" identidad nueva --name releaser --seed-stdin >/dev/null
RELEASER=$("$CLI" identidad listar | awk '/releaser$/ {print $2}')
echo "[aoe-test] identidad firmante: $RELEASER"

# --- 3. spec del release -------------------------------------------------------
# Empaquetamos una app real ya compilada del genesis (hola/app.wasm), asi no
# dependemos de un toolchain wasm en esta corrida. El manifiesto resultante es
# la nueva raiz que el guest re-ancla al aceptar.
WASM_SRC="$ROOT/03_ukupacha/wawa/wawa-kernel/assets/app.wasm"
[ -f "$WASM_SRC" ] || { echo "FALTA $WASM_SRC — corre el build del genesis primero" >&2; exit 1; }

RELEASE_DIR="$ROOT/target/aoe-release"
rm -rf "$RELEASE_DIR"
SPEC="$DEMO_HOME/spec.json"
cat > "$SPEC" <<JSON
{"canal":"dev-aoe","apps":[
  {"nombre":"hola","wasm":"$WASM_SRC","region":[100,120,480,560],"fuel":2000000,"permisos":0}
]}
JSON

echo "[aoe-test] publicando release firmado → $RELEASE_DIR"
"$CLI" wawa publicar --como "$RELEASER" --spec "$SPEC" --salida "$RELEASE_DIR"

# --- 4. TAP de capa-2 ----------------------------------------------------------
echo "[aoe-test] preparando TAP «$TAP» (pide sudo)…"
RENASER_TAP="$TAP" "$ROOT/scripts/aoe-tap-setup.sh" up

# --- 5. los dos comandos finales -----------------------------------------------
cat <<FIN

[aoe-test] host listo. Ahora, EN DOS TERMINALES:

  (A) arranca wawa bridgeado al tap (este abre la ventana de QEMU):
      cd $ROOT/03_ukupacha/wawa
      RENASER_TAP=$TAP RENASER_OVMF=\${RENASER_OVMF:-/usr/share/edk2/x64/OVMF.4m.fd} \\
          cargo +nightly run -p boot -Z bindeps

  (B) difunde el release sobre el mismo tap (raw socket → root):
      sudo -E $CLI wawa anunciar --iface $TAP --dir $RELEASE_DIR

  En la app «mudanza» del guest: debe aparecer "PROPUESTA EN RED (AKASHA)".
  SPACE acepta · ESC rechaza.  Corta (B) con Ctrl-C al terminar.

  Al cerrar, derriba el tap con:
      RENASER_TAP=$TAP $ROOT/scripts/aoe-tap-setup.sh down
FIN
