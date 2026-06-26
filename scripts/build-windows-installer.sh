#!/usr/bin/env bash
# build-windows-installer.sh — forja instalables Windows (.exe + .zip + .msi)
# de apps Llimphi de la suite, cross-compilando desde Linux. Sin máquina
# Windows: cargo-xwin aporta la CRT/linker de Microsoft y wixl (msitools)
# forja el MSI nativo.
#
# Uso:
#   scripts/build-windows-installer.sh <app> [<app>...] [--version X.Y.Z]
#   scripts/build-windows-installer.sh all
#   scripts/build-windows-installer.sh cosmos --version 1.2.0
#
# Apps registradas: cosmos takiy pluma supay nakui
#
# Mecanismo de actualización (importante): cada app tiene un UpgradeCode
# FIJO (no cambia entre releases). El MSI declara <MajorUpgrade>, así que
# instalar una versión con número MAYOR reemplaza la anterior en sitio —
# el equivalente nativo Windows al actualizador churay de Linux. NUNCA
# cambiar el UpgradeCode de una app ya publicada: rompería la cadena de
# updates (Windows vería un producto distinto e instalaría en paralelo).
#
# Prerrequisitos (una vez):
#   rustup target add x86_64-pc-windows-msvc
#   cargo install cargo-xwin
#   wixl (paquete msitools). En Arch/Artix no está en repos; vía AUR
#   necesita glib2-devel. Si el PKGBUILD de msitools-git falla en package()
#   por un LICENSE inexistente, el binario igual queda compilado: apuntá
#   WIXL a ~/.cache/yay/msitools-git/pkg/msitools-git/usr/bin/wixl.
#
# Envs útiles (para máquinas con / lleno, como esta):
#   WINPKG_OUT     dir de salida          (default: dist/windows en el repo)
#   XWIN_CACHE_DIR caché SDK de Microsoft (default: el de cargo-xwin)
#   TMPDIR         temporales del build
#   WIXL           ruta a wixl si no está en PATH
#   WIXL_LIBDIR    dir con libmsi-1.0.so si wixl no está instalado a system
set -euo pipefail

REPO_ROOT="$(cd "$(dirname "${BASH_SOURCE[0]}")/.." && pwd)"
TARGET="x86_64-pc-windows-msvc"
VERSION="1.0.0"
OUT="${WINPKG_OUT:-$REPO_ROOT/dist/windows}"

# ── Registro de apps ─────────────────────────────────────────────────────
# clave → "crate|Producto|exe_base|UpgradeCode|CompExeGUID|CompShortcutGUID|descripción"
# Los GUIDs son FIJOS por app (ver nota de actualización arriba).
declare -A APPS=(
  [cosmos]="cosmos-app-llimphi|Cosmos|cosmos|32c3162b-bb0a-428e-95dd-5f685615a423|9f2c185b-7b34-4247-a693-c5b5c4a2e4d7|20fd0dd1-bc6d-4df8-92da-f004183c663e|cartas astrales y astronomia"
  [takiy]="takiy-app-llimphi|Takiy|takiy|baa974ac-70f9-4ecb-af44-bf010094a3d5|2408a127-50ef-45ff-857c-72d6641e9571|5aeb8fab-1b9a-442f-8909-a58837ae7f03|piano roll y sintetizador"
  [pluma]="pluma-app-llimphi|Pluma|pluma|f8217a2a-cad4-4d7f-93c6-7ad0be9f4b57|89ea0314-34c5-4b85-a4cb-68fbbe8fd287|3287f99e-df79-451f-b4ec-17d8f533dec9|editor de escritura multilienzo"
  [supay]="supay-app-llimphi|Supay|supay|e847d052-8ca5-497d-b00a-23873e6c66d2|d65f5b93-8e0d-40c5-9f70-204540ef1f92|723640b7-16ce-41fa-900d-af79e73b4a08|raycaster estilo Doom"
  # nakui = la hoja de cálculo (nakui-sheet): anzuelo limpio y autocontenido.
  # El shell completo (nakui-ui-llimphi) NO está acá: arrastra card-handshake
  # (autenticación SO_PEERCRED del init brahman, sin equivalente Windows) y
  # surrealdb. Portarlo es un proyecto aparte, no un anzuelo.
  [nakui]="nakui-sheet-llimphi|Nakui|nakui-sheet|2d7dcde5-0a4e-4506-b063-5a885cf947d5|76d3fc48-e128-4ccd-afaf-95097db0de0f|01f2ba2c-ebf8-41b6-a339-2ddd15df0372|hoja de cálculo soberana estilo Excel"
)

# ── Parse de argumentos ──────────────────────────────────────────────────
SELECTED=()
while [[ $# -gt 0 ]]; do
  case "$1" in
    --version) VERSION="$2"; shift 2 ;;
    all)       SELECTED=("${!APPS[@]}"); shift ;;
    -h|--help) sed -n '2,40p' "$0"; exit 0 ;;
    *)         SELECTED+=("$1"); shift ;;
  esac
done
[[ ${#SELECTED[@]} -eq 0 ]] && { echo "uso: $0 <app|all> [--version X.Y.Z]"; echo "apps: ${!APPS[*]}"; exit 1; }

# ── Localizar wixl ───────────────────────────────────────────────────────
WIXL="${WIXL:-$(command -v wixl || true)}"
if [[ -z "$WIXL" ]]; then
  cand="$HOME/.cache/yay/msitools-git/pkg/msitools-git/usr/bin/wixl"
  [[ -x "$cand" ]] && { WIXL="$cand"; WIXL_LIBDIR="${WIXL_LIBDIR:-$HOME/.cache/yay/msitools-git/pkg/msitools-git/usr/lib}"; }
fi
[[ -z "$WIXL" ]] && { echo "ERROR: wixl no encontrado. Instalá msitools o exportá WIXL=/ruta/a/wixl"; exit 1; }
[[ -n "${WIXL_LIBDIR:-}" ]] && export LD_LIBRARY_PATH="${WIXL_LIBDIR}:${LD_LIBRARY_PATH:-}"

mkdir -p "$OUT"
echo "▶ versión $VERSION · salida $OUT · wixl $WIXL"

# ── Genera el .wxs de una app ────────────────────────────────────────────
emit_wxs() {
  local product="$1" exe="$2" upgrade="$3" comp="$4" short="$5" desc="$6" wxs="$7"
  local productcode; productcode="$(uuidgen)"
  cat > "$wxs" <<EOF
<?xml version="1.0" encoding="utf-8"?>
<Wix xmlns="http://schemas.microsoft.com/wix/2006/wi">
  <Product Id="$productcode" Name="$product" Language="1033" Version="$VERSION"
           Manufacturer="tawasuyu" UpgradeCode="$upgrade">
    <Package InstallerVersion="200" Compressed="yes" InstallScope="perMachine"
             Description="$product — $desc (tawasuyu)" />
    <!-- Actualización en sitio: un MSI con Version mayor reemplaza al instalado. -->
    <MajorUpgrade DowngradeErrorMessage="Ya hay una versión más nueva de $product instalada."
                  AllowSameVersionUpgrades="yes" />
    <Media Id="1" Cabinet="$exe.cab" EmbedCab="yes" />
    <Directory Id="TARGETDIR" Name="SourceDir">
      <Directory Id="ProgramFiles64Folder">
        <Directory Id="INSTALLDIR" Name="$product">
          <Component Id="MainExe" Guid="$comp" Win64="yes">
            <File Id="MainExeFile" Name="$exe.exe" Source="$exe.exe" KeyPath="yes" />
          </Component>
        </Directory>
      </Directory>
      <Directory Id="ProgramMenuFolder">
        <Component Id="MenuShortcut" Guid="$short" Win64="yes">
          <Shortcut Id="StartMenu" Name="$product" Target="[INSTALLDIR]$exe.exe"
                    WorkingDirectory="INSTALLDIR" />
          <RegistryValue Root="HKCU" Key="Software\\tawasuyu\\$product"
                         Name="installed" Type="integer" Value="1" KeyPath="yes" />
        </Component>
      </Directory>
    </Directory>
    <Feature Id="Main" Title="$product" Level="1">
      <ComponentRef Id="MainExe" />
      <ComponentRef Id="MenuShortcut" />
    </Feature>
  </Product>
</Wix>
EOF
}

# ── Forja una app ────────────────────────────────────────────────────────
build_one() {
  local key="$1"
  local spec="${APPS[$key]:-}"
  [[ -z "$spec" ]] && { echo "✗ app desconocida: $key (apps: ${!APPS[*]})"; return 1; }
  IFS='|' read -r crate product exe upgrade comp short desc <<<"$spec"

  echo "── $key ($crate) ────────────────────────────────────────────"
  echo "  · cargo xwin build --release"
  ( cd "$REPO_ROOT" && cargo xwin build --release --target "$TARGET" -p "$crate" )

  local stage; stage="$(mktemp -d)"
  cp "$REPO_ROOT/target/$TARGET/release/$crate.exe" "$stage/$exe.exe"
  printf '%s — %s (tawasuyu)\r\n\r\nDoble-click en %s.exe para ejecutar.\r\nWindows x64. Sin instalacion necesaria para el .zip.\r\n' \
    "$product" "$desc" "$exe" > "$stage/LEEME.txt"
  emit_wxs "$product" "$exe" "$upgrade" "$comp" "$short" "$desc" "$stage/app.wxs"

  local base="$OUT/${exe}-${VERSION}-windows-x64"
  ( cd "$stage" && zip -j -9 -q "${base}.zip" "$exe.exe" LEEME.txt )
  ( cd "$stage" && "$WIXL" -a x64 -o "$OUT/${exe}-${VERSION}-x64.msi" app.wxs )

  rm -rf "$stage"
  echo "  ✓ $(basename "${base}.zip")  +  $(basename "$OUT/${exe}-${VERSION}-x64.msi")"
}

for app in "${SELECTED[@]}"; do build_one "$app"; done
echo "▶ listo. Artefactos en $OUT"
ls -la "$OUT"
