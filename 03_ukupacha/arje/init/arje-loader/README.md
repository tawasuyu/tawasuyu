# `arje-loader` — bootloader EFI propio del fractal

Reemplazo soberano de `systemd-boot` / `rEFInd`. Lee `/loader/entries/arje.conf`
de la ESP donde corre, registra el initrd vía LoadFile2 protocol, carga el
kernel (EFISTUB PE), le pasa el cmdline y le entrega el control. Sin
proyectos externos en la cadena de build — `cargo build --release
--target=x86_64-unknown-uefi` y queda un `.efi` de ~45 KB.

## Build

```sh
cargo build --release \
    --manifest-path 03_ukupacha/arje/init/arje-loader/Cargo.toml \
    --target x86_64-unknown-uefi
```

Output: `03_ukupacha/arje/init/arje-loader/target/x86_64-unknown-uefi/release/arje-loader.efi`.

Notas de profile:
- **Sin `lto`, `codegen-units = 1` ni `strip`** — los tres rompen la PE EFI
  bajo OVMF con `BdsDxe: Invalid Parameter`. Probado experimentalmente. El
  binario gana unos KB pero arranca.

## Smoke end-to-end en QEMU (UEFI/OVMF)

Boot completo verificado (2026-05-26):

```
SeaBIOS/OVMF
  → arje-loader.efi (BOOTX64.EFI)
    → install LoadFile2 protocol + media-vendor device path
      con LINUX_EFI_INITRD_MEDIA_GUID
    → load_image(kernel, FromBuffer)
    → start_image
  → kernel EFISTUB
    → busca handles con LINUX_EFI_INITRD_MEDIA_GUID + LoadFile2
    → llama LoadFile dos veces (size + bytes) → recibe initramfs
    → boot
  → arje-zero como PID 1
    → lee /ente/seed.card.json (arje-qemu)
    → primordial loop, sockets bus/brahman levantados
    → instancia genesis: agetty-ttyS0
  → arje-getty-stub
    → abre /dev/ttyS0, banner visible
```

Para reproducir:

```sh
# Build binarios estáticos del fractal (musl):
cargo build --release --target=x86_64-unknown-linux-musl --jobs 2 \
    -p arje-zero -p arje-net-bring-up -p arje-getty-stub

# Build loader EFI:
cargo build --release --target=x86_64-unknown-uefi \
    --manifest-path 03_ukupacha/arje/init/arje-loader/Cargo.toml

# Stage los archivos a un dir (el installer también arma loader.conf y arje.conf):
mkdir -p /tmp/esp
cargo run --release -p arje-installer -- to-partition \
    --esp /tmp/esp \
    --kernel /boot/vmlinuz-linux \
    --seed 03_ukupacha/arje/seeds/arje-qemu.card.json \
    --bin arje-zero=target/x86_64-unknown-linux-musl/release/arje-zero \
    --bin agetty-ttyS0=target/x86_64-unknown-linux-musl/release/arje-getty-stub \
    --cmdline "console=ttyS0 panic=5 loglevel=4"

# Armar disco GPT con partición ESP y poblarla con mtools:
truncate -s 96M /tmp/arje-disk.img
echo 'label: gpt
,,U,*' | sfdisk /tmp/arje-disk.img
mformat -i /tmp/arje-disk.img@@1048576 -F -v ARJE -h 8 -s 32
mmd -i /tmp/arje-disk.img@@1048576 ::/EFI ::/EFI/BOOT ::/EFI/arje \
    ::/loader ::/loader/entries
mcopy -i /tmp/arje-disk.img@@1048576 \
    03_ukupacha/arje/init/arje-loader/target/x86_64-unknown-uefi/release/arje-loader.efi \
    ::/EFI/BOOT/BOOTX64.EFI
mcopy -i /tmp/arje-disk.img@@1048576 /tmp/esp/EFI/arje/vmlinuz ::/EFI/arje/vmlinuz
mcopy -i /tmp/arje-disk.img@@1048576 /tmp/esp/EFI/arje/initramfs.cpio.gz ::/EFI/arje/initramfs.cpio.gz
mcopy -i /tmp/arje-disk.img@@1048576 /tmp/esp/loader/loader.conf ::/loader/loader.conf
mcopy -i /tmp/arje-disk.img@@1048576 /tmp/esp/loader/entries/arje.conf ::/loader/entries/arje.conf

# Boot UEFI con OVMF:
cp /usr/share/edk2/x64/OVMF_VARS.4m.fd /tmp/ovmf_vars.fd
qemu-system-x86_64 -m 512M \
    -drive if=pflash,format=raw,readonly=on,file=/usr/share/edk2/x64/OVMF_CODE.4m.fd \
    -drive if=pflash,format=raw,file=/tmp/ovmf_vars.fd \
    -drive format=raw,file=/tmp/arje-disk.img \
    -nographic -no-reboot
```

Vas a ver el banner del getty-stub por ttyS0 después de unos segundos.

## LoadFile2 protocol

Linux ≥ 5.10 (Artix 7.0.8 incluido) eliminó el fallback `initrd=` por
cmdline para EFISTUB cuando el kernel se carga via bootloader externo.
Ahora el kernel exige que el bootloader instale el protocol
`EFI_LOAD_FILE2_PROTOCOL` (GUID `4006c0c1-fcb3-403e-996d-4a6c8724e06d`)
sobre un handle con device path media-vendor cuya VendorGuid es
`LINUX_EFI_INITRD_MEDIA_GUID = 5568e427-68fc-4f3d-ac74-ca555231cc68`.

Nuestra implementación (en `src/main.rs::install_initrd_loadfile2`):

1. Leemos el initramfs entero a un `Vec<u8>` y lo `Box::leak`eamos —
   tiene que sobrevivir más allá de `start_image`.
2. Guardamos `(ptr, len)` en `static mut`s — el callback de EFI es
   `extern "efiapi"` (C ABI) y no puede capturar.
3. Construimos un device path de 24 bytes: vendor-media node (4 bytes
   header + 16 bytes GUID) + end node (4 bytes).
4. `boot::install_protocol_interface(None, &DevicePath::GUID, ...)` →
   nos da un handle nuevo.
5. `boot::install_protocol_interface(Some(handle), &LOAD_FILE2_GUID, ...)`
   le adjunta la vtable.
6. Cuando el kernel arranca, busca handles que matchean ambos
   protocolos, llama `load_file(buf=NULL)` para pedir el tamaño,
   alocan el buffer, vuelven a llamar — copiamos los bytes.

## Por qué NO UKI (Unified Kernel Image)

Probé el atajo de `objcopy --add-section .initrd=... vmlinuz uki.efi`.
El kernel de Arch/Artix está zstd-comprimido; insertar secciones
desplaza los offsets internos del bzImage y rompe la descompresión
("ZSTD-compressed data is corrupt"). UKI correcto requiere `ukify`
(systemd) o tooling específico — no es un atajo legítimo, es otro
proyecto.

## Caveats descubiertos

- **Paths en `arje.conf` van con forward slashes** (convención
  systemd-boot) pero **UEFI File::open exige backslashes**. El loader
  normaliza al abrir — sin la normalización, falla con Invalid Parameter
  silencioso.
- **`Box::leak` para datos que sobreviven a `start_image`**: el kernel
  llama nuestro LoadFile2 callback DESPUÉS de que start_image transfiere
  control. Si el initrd vive en un `Vec` con destructor, se libera y el
  kernel lee basura.
- **Sólo se llama `boot::stall` en errores** — si stalleamos en el
  camino feliz, retrasamos el boot 5s+ por nada.
