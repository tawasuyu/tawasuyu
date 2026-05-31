# arje

> `arje` (griego *ἀρχή*: origen, principio). Bootloader y vida temprana del sistema.

`arje` cubre desde el "presionaste ENCENDER" hasta "el kernel está corriendo": semillas (`seeds`), empaquetado (`packager`), instalación (`installer`), absorción de un sistema existente (`absorb`), kernel propio (`kernel`), red mínima (`net-bring-up`), reglas + auditoría (`brain-*`), CAS (`cas`), snapshots, soma, WASM init.

## Instalación

```sh
# crear un installer ISO
cargo run --release -p arje-packager -- build --target iso

# correr el installer
cargo run --release -p arje-installer

# absorber un sistema existente (Linux) hacia un objeto arje
cargo run --release -p arje-absorb -- /path/to/system
```

## Compatibilidad

- **Linux x86_64** — primary target.
- **aarch64** — soportado en `arje-kernel` (limitado).
- **Wawa** — `arje` es el bootloader natural de `wawa-kernel`.

## Crates

| Crate | Rol |
|---|---|
| [`arje-zero`](arje-zero/README.md) | Punto cero: lo primero que corre. |
| [`arje-loader`](arje-loader/README.md) | Loader del kernel. |
| [`arje-kernel`](arje-kernel/README.md) | Kernel mínimo de arje (separado del wawa-kernel). |
| [`arje-incarnate`](arje-incarnate/README.md) | Materializa procesos. |
| [`arje-bus`](arje-bus/README.md) | Bus interno (IPC arje). |
| [`arje-soma`](arje-soma/README.md) | "Cuerpo" del sistema en runtime. |
| [`arje-cas`](arje-cas/README.md) | Content-addressed store. |
| [`arje-snapshot`](arje-snapshot/README.md) | Snapshot del sistema. |
| [`arje-echo`](arje-echo/README.md) | Logging temprano. |
| [`arje-net-bring-up`](arje-net-bring-up/README.md) | Stack de red mínimo. |
| [`arje-wasm`](arje-wasm/README.md) | Runtime WASM de init. |
| [`arje-compat`](arje-compat/README.md) | Compat con userspace POSIX (shims). |
| [`arje-getty-stub`](arje-getty-stub/README.md) | Login mínimo. |
| [`arje-card`](arje-card/README.md) | Alias histórico de `card-core` (re-exporta `EntityCard ≡ Card`); no es UI. |
| [`arje-card-llimphi`](arje-card-llimphi/) | Card escritorio (estado de arje): capacidades de aislamiento del init, sobre Llimphi. |
| [`arje-packager`](arje-packager/README.md) | Empaquetador (ISO, .img). |
| [`arje-installer`](arje-installer/README.md) | Installer interactivo. |
| [`arje-absorb`](arje-absorb/README.md) | Absorbe un sistema existente → objeto arje. |
| [`arje-brain`](arje-brain/README.md) | Reglas + auditoría del init. |
| [`arje-brain-rules`](arje-brain-rules/README.md) | Reglas declarativas. |
| [`arje-brain-cognitive`](arje-brain-cognitive/README.md) | Razonador. |
| [`arje-brain-audit`](arje-brain-audit/README.md) | Auditoría del razonador. |

## Consideraciones

- **`arje` es para arrancar, no para gobernar el sistema en uso**. Esa parte es del `wawa-kernel`.
- `absorb` no destruye nada: opera read-only sobre el host y produce un objeto independiente.
- Cada crate justifica su Cargo.toml línea por línea — la raíz no acepta deps frívolas.
