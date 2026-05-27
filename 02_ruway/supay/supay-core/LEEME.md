# supay-core

> FFI a `doomgeneric` + `DoomEngine` safe de [supay](../README.md).

Envuelve los callbacks `extern "C"` que el motor C llama (`DG_Init`, `DG_DrawFrame`, `DG_GetKey`, ...). Estado en `OnceLock<Mutex<HostState>>`: framebuffer copiado + FIFO input + ticks + título. API safe: `DoomEngine::{new, tick, push_key, framebuffer, title, sprite_name, flat_name}`.

## Deps

- `cc` (build), `libc`
