# arje

> `arje` (griego *ἀρχή*: origen, principio). Bootloader y vida temprana del sistema.

`arje` cubre desde el "presionaste ENCENDER" hasta "el kernel está corriendo": semillas (`seeds`), empaquetado (`packager`), instalación (`installer`), absorción de un sistema existente (`absorb`), kernel propio (`kernel`), red mínima (`net-bring-up`), reglas + auditoría (`brain-*`), CAS (`cas`), snapshots, soma, WASM init.

## Instalación

```sh
# initramfs (cpio.gz) desde una seed canónica + binarios estáticos
cargo run --release -p arje-packager -- --seed 03_ukupacha/arje/seeds/arje-qemu.card.json --bin arje-zero=target/release/arje-zero --out /tmp/arje-qemu.cpio.gz

# instalar a una partición ESP (`to-partition`) o armar un USB booteable (`to-usb`)
cargo run --release -p arje-installer -- to-partition --esp /boot --kernel bzImage --seed 03_ukupacha/arje/seeds/arje-host.card.json

# absorber la config de un init existente (sysvinit/runit/dinit/OpenRC) hacia una Semilla
cargo run --release -p arje-absorb -- --from auto --root / --output seed.card.json
```

## Compatibilidad

- **Linux x86_64** — primary target.
- **aarch64** — soportado en `arje-kernel` (limitado).
- **Wawa** — `arje` es el bootloader natural de `wawa-kernel`.

## Crates

| Crate | Rol |
|---|---|
| [`arje-zero`](init/arje-zero/README.md) | Punto cero: lo primero que corre (PID 1). |
| [`arje-loader`](init/arje-loader/README.md) | Bootloader EFI propio (uefi-rs): carga kernel EFISTUB + initramfs + cmdline. |
| [`arje-kernel`](init/arje-kernel/README.md) | Kernel mínimo de arje (separado del wawa-kernel). |
| [`arje-incarnate`](init/arje-incarnate/README.md) | Materializa procesos. |
| [`arje-bus`](runtime/arje-bus/README.md) | Bus interno (IPC arje, Unix socket + postcard): el wire de control del init. |
| [`arje-soma`](init/arje-soma/README.md) | "Cuerpo" del sistema en runtime. |
| [`arje-cas`](runtime/arje-cas/README.md) | Content-addressed store. |
| [`arje-snapshot`](init/arje-snapshot/README.md) | Snapshot del sistema. |
| [`arje-echo`](runtime/arje-echo/README.md) | Logging temprano. |
| [`arje-net-bring-up`](init/arje-net-bring-up/README.md) | Stack de red mínimo. |
| [`arje-wasm`](runtime/arje-wasm/README.md) | Runtime WASM de init. |
| [`arje-compat`](arje-compat/README.md) | Compat con userspace POSIX (shims). |
| [`arje-getty-stub`](init/arje-getty-stub/README.md) | Login mínimo. |
| [`arje-card`](arje-card/README.md) | Alias histórico de `card-core` (re-exporta `EntityCard ≡ Card`); no es UI. |
| [`arje-card-llimphi`](arje-card-llimphi/) | Card escritorio (estado de arje): capacidades de aislamiento del init, sobre Llimphi. |
| [`arje-packager`](init/arje-packager/README.md) | Empaquetador (initramfs cpio.gz desde una Tarjeta Semilla). |
| [`arje-installer`](init/arje-installer/README.md) | Installer (partición ESP / USB booteable). |
| [`arje-absorb`](init/arje-absorb/README.md) | Absorbe la config de un init existente → Semilla arje. |
| [`arje-brain`](runtime/arje-brain/README.md) | Reglas + auditoría del init. |
| [`arje-brain-rules`](runtime/arje-brain-rules/README.md) | Reglas declarativas. |
| [`arje-brain-cognitive`](runtime/arje-brain-cognitive/README.md) | Razonador. |
| [`arje-brain-audit`](runtime/arje-brain-audit/README.md) | Auditoría del razonador. |

Las semillas canónicas (`arje-host`, `arje-qemu`) viven en [`seeds/`](seeds/).

## Consideraciones

- **`arje` es para arrancar, no para gobernar el sistema en uso**. Esa parte es del `wawa-kernel`.
- `absorb` no destruye nada: opera read-only sobre el host y produce un objeto independiente.
- Cada crate justifica su Cargo.toml línea por línea — la raíz no acepta deps frívolas.
