# `arje-loader` — bootloader EFI propio del fractal

Reemplazo soberano de `systemd-boot` / `rEFInd`. Lee `/loader/entries/arje.conf`
de la ESP donde corre, carga el kernel (EFISTUB PE), le pasa el cmdline y le
entrega el control. Sin proyectos externos en la cadena de build — `cargo
build --release --target=x86_64-unknown-uefi` y queda un `.efi` de ~40 KB.

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

## Estado actual (2026-05-26)

**Funciona**:
- Firmware UEFI invoca `arje-loader.efi`.
- Loader lee y parsea `/loader/entries/arje.conf`.
- Carga el kernel a un buffer y lo entrega a `BootServices::load_image`.
- Setea el cmdline en `LoadOptions`.
- Llama `start_image` — control transfiere al kernel.

**No funciona todavía**:

Linux ≥ 5.10 (Artix 7.0.8) eliminó el fallback `initrd=` por cmdline para
EFISTUB. Ahora el kernel **exige** que el bootloader instale un
`LINUX_EFI_INITRD_MEDIA_GUID` LoadFile2 protocol. Sin eso, arranca pero
muere así:

```
EFI stub: ERROR: Failed to handle fs_proto
EFI stub: ERROR: Failed to load initrd: 0x8000000000000002
EFI stub: ERROR: efi_stub_entry() failed!
```

## Próxima iteración — LoadFile2

Para cerrar el ciclo, hay que implementar el protocolo `EFI_LOAD_FILE2_PROTOCOL`
con la GUID `5568e427-68fc-4f3d-ac74-ca555231cc68` (la que el kernel busca
para encontrar el initrd):

1. Definir la struct con vtable manual de `EFI_LOAD_FILE2_PROTOCOL`.
2. Construir un device path MEDIA_VENDOR con la GUID arriba.
3. `boot::install_protocol_interface` con esa GUID + un handle nuevo.
4. El callback `LoadFile` se invoca dos veces por el kernel:
   - Primera con `BufferSize = 0, Buffer = NULL` para pedir el tamaño.
   - Segunda con el buffer ya alocado, donde copiamos los bytes del initrd
     que tenemos en memoria.

uefi-rs 0.35 expone `boot::install_protocol_interface`. Lo que falta es
escribir la struct + vtable. Es ~50 líneas de unsafe pero straightforward.

Mientras tanto: el flujo `arje-installer to-partition --register` arranca
bien porque crea una NVRAM entry directa al kernel (EFISTUB), con cmdline en
los `Load Option Args` — Linux acepta el cmdline por esa vía sin LoadFile2,
y el initrd lo lee directo del FS por la ruta del kernel. Sólo el flujo
`to-usb` (que necesita un loader-en-disco porque no puede tocar NVRAM de la
máquina destino) queda con la limitación temporal.

## Por qué no UKI (Unified Kernel Image)

Probé el atajo de `objcopy --add-section .initrd=... vmlinuz uki.efi`. El
kernel de Arch/Artix está comprimido con ZSTD y la inserción de secciones
desplaza los offsets que el bzImage usa para encontrar su propia data
comprimida. Resultado: `EFI stub: ZSTD-compressed data is corrupt`. Para
hacer UKI correctamente hay que usar `ukify` (de systemd) o reconstruir el
PE con tooling específico — no es un atajo legítimo, es otro proyecto.
