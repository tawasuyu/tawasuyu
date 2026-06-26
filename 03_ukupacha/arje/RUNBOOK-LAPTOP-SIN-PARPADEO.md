# RUNBOOK — Arranque sin parpadeo de arje en una laptop real

Plan de ejecución para llevar la cadena **GOP → efifb/simpledrm → DRM/KMS →
mirada** (arranque gráfico desde el frame cero, sin caídas a texto ni modeset
visible) a una laptop concreta. La doctrina, el diseño y la verificación previa
están en `SDD-ARRANQUE-SIN-PARPADEO.md` — **leer eso primero**; esto es solo el
runbook operativo.

## Qué ya está hecho (no rehacer)

- **Fase 0** — cmdline flicker-free (`arje-installer::canonical_cmdline`) + logo
  GOP en `arje-loader` (placeholder: cuadrado sólido, sin verificar en QEMU+OVMF).
- **Fase 1** — `arje-splash`: toma DRM reusando el modo vigente (sin re-modeset),
  double-buffer + page-flip, animación + barra + panel de logs automático.
- **Fase 2** — handoff splash↔mirada por socket Unix (`/run/arje-splash.sock`,
  protocolo `READY`/`RELEASED`) + Incrementos 1–3 (present del fondo común,
  `disable_connectors=false`, splash keep-alive). **GAP medido ~389 ms en Iris Xe.**
- **Fase 3** (split render-node/card-node) y **Fase 2-bis** — **ARCHIVADAS**. No
  tocar salvo que en uso real moleste el ratito de fondo común antes de la tarjeta.

> Conclusión: en la laptop **no se programa**, se **instala + certifica**. El
> único riesgo real es que el cmdline/driver de la GPU de la laptop difiera del
> Iris Xe ya probado (Intel) — Fase 0 es GPU-específica.

---

## Paso 0 — Inventario de la laptop (copiar/pegar y guardar la salida)

```bash
echo "== GPU =="; lspci -nn | grep -iE 'vga|3d|display'
echo "== driver KMS cargado =="; lsmod | grep -iE 'i915|amdgpu|nouveau|nvidia|xe '
echo "== firmware =="; [ -d /sys/firmware/efi ] && echo UEFI || echo "BIOS/legacy (esta cadena asume UEFI)"
echo "== EFISTUB en el kernel =="; zgrep -E 'CONFIG_EFI_STUB|CONFIG_DRM_SIMPLEDRM|CONFIG_SYSFB_SIMPLEFB' /proc/config.gz 2>/dev/null || echo "(sin /proc/config.gz; revisá /boot/config-\$(uname -r))"
echo "== nodos DRM =="; ls -l /dev/dri/
echo "== init =="; cat /proc/1/comm
echo "== seat manager =="; pgrep -a seatd; pgrep -a elogind; pgrep -a systemd-logind
echo "== Mesa (greeter real) =="; command -v glxinfo >/dev/null && glxinfo -B 2>/dev/null | grep -iE 'opengl renderer|device' || echo "(instalá mesa-utils para confirmar)"
```

**De acá salen dos decisiones:**

1. **Flags de cmdline por GPU** (Paso 3):
   - Intel (`i915`): `i915.fastboot=1`
   - Intel nuevo (`xe`): probar sin flag de fastboot (el driver `xe` ya hereda
     mejor; medir y ajustar)
   - AMD (`amdgpu`): `amdgpu.dc=1` (default moderno) — no hay equivalente directo
     de fastboot; el seamless depende de simpledrm→amdgpu handover
   - NVIDIA propietario: **no soportado en seamless** (su KMS no hereda simpledrm
     limpio). Usar `nouveau` o quedarse en Camino A (splash post-kernel).
2. **`/dev/dri/cardN`** correcto para los scripts (laptops con iGPU+dGPU tienen
   `card0` y `card1` — el del panel eDP suele ser el de la iGPU).

---

## Paso 1 — Smoke test en QEMU (sin tocar la laptop)

Valida que los binarios compilan estáticos y que la cadena
splash→handoff→greeter-simulado corre. **No** prueba tu GPU real, pero descarta
regresiones de build antes de ir a metal.

```bash
# deps: qemu, edk2-ovmf, rustup target add x86_64-unknown-linux-musl, musl-gcc
cd /ruta/al/repo
./scripts/test-arje-splash-qemu.sh            # ventana: splash → fade → tarjeta
./scripts/test-arje-splash-qemu.sh --headless # veredicto por texto (CI-style)
```

Esperado (texto): `reusando modo vigente` → `READY` → `RELEASED` → tarjeta del
greeter-sim sobre el mismo `BG`, sin warning de fallback de page-flip.

---

## Paso 2 — Crossfade en METAL desde un VT (SEGURO, sin reboot, REVERSIBLE)

Este es el test que de verdad certifica tu GPU. Corre el splash + el greeter
**real** de mirada desde una consola libre, tomando DRM master sin tocar tu
sesión gráfica ni el bootloader. Es la forma de bajo riesgo de validar la GPU de
la laptop antes de instalar nada.

```bash
cargo build --release -p arje-splash -p mirada-compositor -p mirada-greeter
# Ctrl+Alt+F3 → login en un VT libre. NO desde tu sesión gráfica.
./scripts/test-drm-greeter-metal.sh /dev/dri/card0   # ajustá cardN al panel eDP
# Volvés a tu escritorio con Ctrl+Alt+F7 (o F1/F2 según la distro).
```

El script imprime **evidencia de texto**: `GAP de BG estático` (RELEASED →
primer `queue_frame`) y el desglose `LLIMPHI_TIMING` de en qué se va ese gap.

**Criterio de éxito:**
- GAP comparable al baseline Iris Xe (~389 ms) — si es mucho mayor, mirá el
  desglose: si `RELEASED → device-listo` domina, probablemente hay re-modeset
  (panel se apaga) → revisar Fase 0 / `disable_connectors`.
- Visualmente: del splash a la tarjeta **sin negro, sin líneas random, sin
  cambio de resolución**. Líneas horizontales random = bo GBM sin inicializar
  (debería estar tapado por el Incremento 1; si aparece, anotarlo).

> Si `mirada-greeter` falla por falta de Mesa/EGL: instalá Mesa en la laptop.
> El greeter real necesita GPU+Mesa; sin eso solo podés ver el greeter-SIMULADO
> (Paso 1 en QEMU).

---

## Paso 3 — Calibrar el cmdline flicker-free de la laptop

El cmdline canónico lo emite `arje-installer::canonical_cmdline`. Antes de
instalar nativo, verificá que los flags de **tu** GPU dan un kernel silencioso
que hereda el modo del GOP sin re-modeset. Base (ajustar el último token por GPU,
ver Paso 0):

```
quiet loglevel=0 vt.global_cursor_default=0 rd.systemd.show_status=false <flag-gpu>
```

Forma barata de probar **sin instalar arje**: agregar esos flags a una entrada
de tu GRUB/loader actual de la laptop (línea aparte, no pisar la de siempre) y
bootear esa entrada una vez. Si la pantalla va de logo de firmware → splash sin
flash de texto, los flags sirven. (En `i915` el `i915.fastboot=1` es la clave.)

---

## Paso 4 — Instalación nativa en la ESP (REVERSIBLE, no pisa tu GRUB)

### Camino fácil (recomendado): instalador automático

```bash
./scripts/install-arje.sh            # autodetecta ESP/kernel/GPU, pide sudo, registra la entrada
./scripts/install-arje.sh --uninstall   # revierte todo
```

Un solo comando, sin parámetros. Detecta la ESP (por GUID EFI System, montándola
si hace falta), el kernel y la GPU; compila los estáticos musl; copia arje bajo
`/EFI/arje/` y registra la entrada NVRAM con `efibootmgr`. Sólo pregunta lo
ambiguo (varias ESP/kernels) y cómo activar (sólo crear / probar una vez con
BootNext / hacerla default). Usa el seed `arje-laptop` (splash + consola de
prueba en tty1).

> **Honestidad sobre qué bootea hoy:** el arranque nativo es un **demo del
> boot-chain sin parpadeo** — muestra el splash gráfico en la GPU real y baja a
> una consola de prueba de vida en tty1 (`arje-getty-stub` no es un login). El
> greeter gráfico y un login real necesitan rootfs con Mesa / getty estático,
> aún no listos para el arranque nativo. El demo igual sirve para **ver el
> arranque sin parpadeo en tu laptop**, que es el payoff de toda esta línea.

### Camino manual (control fino)

Por si querés elegir seed/cmdline a mano. Ruta EFISTUB directo:
`firmware UEFI → /EFI/arje/vmlinuz → arje-zero`. Convive con el GRUB de la
laptop; agrega **una** entrada NVRAM que podés priorizar o no.

```bash
# 0) efibootmgr presente:  pacman -S efibootmgr  (o el equivalente de tu distro)
# 1) identificar y montar la ESP de la laptop:
sudo blkid | grep -i vfat            # la línea vfat es tu ESP
sudo mkdir -p /mnt/esp && sudo mount /dev/XXXp1 /mnt/esp   # ← tu device
ls /mnt/esp                          # debe verse EFI/

# 2) compilar e instalar (no destructivo; copia bajo /EFI/arje/ al lado del GRUB):
cargo build --release -p arje-zero -p arje-installer
sudo ./target/release/arje-installer to-partition \
    --esp /mnt/esp \
    --kernel /boot/vmlinuz-linux \
    --seed 03_ukupacha/arje/seeds/arje-host.card.json \
    --bin arje-zero=./target/release/arje-zero \
    --bin agetty-ttyS0=/sbin/agetty \
    --cmdline "<el cmdline calibrado en el Paso 3>" \
    --label "arje"
# Imprime el `efibootmgr` exacto. Corrélo a mano, o repetí con:
#   --register --disk /dev/XXX --part 1
```

> Para incluir el splash en el arranque nativo, agregá los `--bin arje-splash=…`
> /`--asset etc/arje/splash.conf=…` como hace `scripts/install-arje-splash.sh
> --esp` y `scripts/test-arje-splash-qemu.sh --stage-esp`. El `seed` host ya
> declara el Ente `arje-splash` antes del display-manager.

**Reversión total (deja la laptop como estaba):**
```bash
sudo efibootmgr            # ver el número de la entrada "arje"
sudo efibootmgr -b <NUM> -B
sudo rm -rf /mnt/esp/EFI/arje /mnt/esp/loader/entries/arje.conf
```

---

## Paso 5 — Veredicto y registro

Capturar **evidencia de texto** (Regla 8 del repo — no PNG de rutina):

- GAP del Paso 2 (RELEASED → primer frame) y el desglose `LLIMPHI_TIMING`.
- Confirmación visual una sola vez (es lo único no certificable en texto): boot
  nativo del Paso 4 sin flash de texto ni cambio de resolución, splash → tarjeta
  continuo.
- Anotar en el SDD (sección «Verificación sobre hardware real») los hallazgos de
  **esta** GPU: flags de cmdline usados, GAP medido, artefactos vistos. Igual que
  está la entrada de Iris Xe (2026-06-24).

---

## Riesgos conocidos / puntos abiertos

1. **GPU ≠ Intel Iris Xe.** Todo lo verificado es Intel `i915`. AMD `amdgpu` y el
   nuevo `xe` no están certificados acá — el handover simpledrm→KMS puede pedir
   otros flags o exhibir re-modeset. NVIDIA propietario: fuera de alcance seamless.
2. **`arje-loader` GOP logo** sigue siendo placeholder (cuadrado sólido) y sin
   verificar en QEMU+OVMF. Con EFISTUB directo (Paso 4) el loader **no** está en
   el camino: el primer pixel gráfico lo da `arje-splash` tras simpledrm, no el
   loader. Si se quiere el logo desde el frame −1 (antes del kernel), hay que
   bootear `arje-loader.efi` en vez de vmlinuz directo — camino aparte, no cubierto
   por este runbook.
3. **Greeter real necesita Mesa** en la laptop. Sin Mesa solo corre el
   greeter-simulado (QEMU).
4. **Camino A (fallback) si el seamless nativo no cuaja en tu GPU:**
   `scripts/install-arje-splash.sh --system` instala el splash como servicio del
   init (en Artix/OpenRC ya quedó soportado). Da splash post-kernel; el tramo
   firmware→kernel previo depende del initramfs/bootsplash de la laptop, pero es
   cero riesgo y no toca el arranque.

## Orden recomendado de ejecución

Paso 0 (inventario) → Paso 1 (QEMU) → **Paso 2 (metal VT, el que más certifica y
es reversible)** → Paso 3 (calibrar cmdline) → Paso 4 (instalar nativo) → Paso 5
(registrar). Si el Paso 2 ya muestra problemas en tu GPU, resolver eso antes de
instalar nativo.
```
