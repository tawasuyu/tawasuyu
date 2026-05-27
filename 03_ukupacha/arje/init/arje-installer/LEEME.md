# `arje-installer` — instalación a partición o USB

Pega kernel + initramfs (via [`arje-packager`](../arje-packager)) + seed +
metadatos de boot en una ESP montada (modo no-destructivo, `to-partition`)
o formatea un USB entero como GPT/ESP booteable (`to-usb`). Solo UEFI.

## Modelo de boot

Doctrina: **EFISTUB directo, cero bootloader runtime**.

El kernel Linux moderno se compila con `CONFIG_EFI_STUB=y` (Arch, Artix,
Debian, Ubuntu lo traen así por default). Ese stub hace que el `bzImage`
sea, además de un kernel, un ejecutable PE válido que la firmware UEFI
puede correr directamente — sin GRUB, sin systemd-boot, sin nada en
medio. El cmdline (incluyendo `initrd=`) se pasa como **UEFI Load
Option args**, que se registran una sola vez en la NVRAM de la
placa con `efibootmgr`.

Esto da el camino más corto posible:

```
firmware UEFI → /EFI/arje/vmlinuz → init=/init → arje-zero (PID 1)
```

Sin bootloader que pueda romperse, sin ESP encadenada al GRUB del host,
sin layers de configuración.

## Modo `to-partition` (instalar desde otro Linux)

Caso típico: tenés Artix corriendo en una partición, vas a probar arje
en otra partición de la misma máquina. Montás la ESP existente y dejás
que el installer pegue los archivos al lado de tu loader actual sin
tocarlo.

```sh
sudo mount /dev/sda1 /mnt/esp    # tu ESP existente
cargo build --release -p arje-zero -p arje-installer

sudo ./target/release/arje-installer to-partition \
    --esp /mnt/esp \
    --kernel /boot/vmlinuz-linux \
    --seed 03_ukupacha/arje/seeds/arje-qemu.card.json \
    --bin arje-zero=./target/release/arje-zero \
    --bin agetty-ttyS0=/sbin/agetty \
    --cmdline "console=ttyS0 panic=10" \
    --label "arje-test"
```

Eso copia bajo `/mnt/esp/EFI/arje/`:

```
EFI/arje/
├── vmlinuz                 # tu kernel host
├── initramfs.cpio.gz       # producto del packager
├── seed.card.json          # la semilla del fractal
└── cmdline.txt             # cmdline canónico (referencia humana)
EFI/BOOT/                   # (vacío hasta to-usb o bootloader externo)
loader/
├── loader.conf             # default arje, timeout 3
└── entries/arje.conf       # systemd-boot/rEFInd format
```

Sin `--register` el installer **no toca la NVRAM** — sólo te imprime el
comando `efibootmgr` exacto para que lo ejecutes vos cuando estés
listo. Con `--register --disk /dev/sda --part 1` ejecuta efibootmgr
automáticamente.

Después de rebootear, la firmware UEFI ofrece "arje-test" en el menú de
boot.

## Modo `to-usb` (USB booteable portátil)

Caso: querés un USB que arranque arje en cualquier máquina UEFI,
incluyendo máquinas donde nunca corriste un Linux. Como cada máquina
destino tiene su propia NVRAM, el USB necesita un bootloader EN disco
(no podemos pre-registrar la NVRAM de máquinas que no conocemos).

```sh
# DESTRUCTIVO — borra el contenido entero de /dev/sdb.
sudo ./target/release/arje-installer to-usb \
    --device /dev/sdb \
    --kernel /boot/vmlinuz-linux \
    --seed 03_ukupacha/arje/seeds/arje-host.card.json \
    --bin arje-zero=./target/release/arje-zero \
    --bin agetty-tty1=/sbin/agetty \
    --bin network-up=./target/release/arje-net-bring-up \
    --bin display-manager-mesa=./target/release/mirada-greeter-llimphi \
    --cmdline "console=tty0" \
    --yes-destroy
```

Pasos internos:

1. `sfdisk /dev/sdb` con script GPT + una partición ESP FAT32 con flag
   booteable.
2. `mkfs.fat -F32 -n ARJE /dev/sdb1` (o `sdb p1` para nvme/mmc).
3. `mount /dev/sdb1 /tmp/...` (tempdir).
4. Stage los archivos como en `to-partition`.
5. Bootloader:
   - Si en el host existe `systemd-bootx64.efi` → se copia a
     `/EFI/BOOT/BOOTX64.EFI` (path UEFI fallback). En boot la firmware
     lo ejecuta, systemd-boot lee `/loader/entries/arje.conf` y arranca
     el kernel con el cmdline correcto.
   - Si existe `refind_x64.efi` → idem para rEFInd.
   - Si no encontramos ninguno → copiamos el kernel directo a
     `/EFI/BOOT/BOOTX64.EFI`. La firmware lo ejecuta pero **sin
     cmdline**, así que el initrd no se carga. Es un fallback pobre que
     sirve sólo para validar que el kernel arranca; para uso real
     necesitás un bootloader en disco o registrar NVRAM en cada destino.
6. `sync` + `umount`.

## Smoke test sin USB físico (QEMU)

El `to-partition` produce un layout que QEMU puede arrancar directo,
pasando el kernel y el initramfs sin necesidad de ESP:

```sh
qemu-system-x86_64 \
    -m 256M \
    -kernel /tmp/esp/EFI/arje/vmlinuz \
    -initrd /tmp/esp/EFI/arje/initramfs.cpio.gz \
    -append "console=ttyS0 panic=10" \
    -nographic
```

Si la seed es `arje-qemu.card.json`, deberías ver el banner de arje-zero
por `ttyS0` seguido del prompt de `agetty`.

## Limitaciones explícitas

- **Solo UEFI.** BIOS legacy fuera de alcance — habría que agregar
  GRUB/syslinux. Probablemente no llegue hasta que aparezca una máquina
  pre-2010 que lo necesite.
- **Binarios estáticos.** Los `--bin` deben ser ejecutables sin
  dependencias dinámicas (`musl-static` o `+crt-static`). El installer
  no resuelve `ldd`. Si necesitás un binario glibc-dinámico, vas a
  tener que armar `/lib/` también.
- **Sin firma del archive.** La integridad cruzada seed↔binarios sale
  del Capability fractal en boot, no del packaging.
- **`to-usb` sin bootloader en el host**: termina con un USB que la
  firmware puede ejecutar pero que no carga initrd. Documentado arriba.
