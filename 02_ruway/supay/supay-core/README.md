# supay-core

> FFI to `doomgeneric` + safe `DoomEngine` of [supay](../README.md).

Wraps the `extern "C"` callbacks the C engine calls (`DG_Init`, `DG_DrawFrame`, `DG_GetKey`, ...). State in `OnceLock<Mutex<HostState>>`: copied framebuffer + input FIFO + ticks + title. Safe API: `DoomEngine::{new, tick, push_key, framebuffer, title, sprite_name, flat_name}`.

## Deps

- `cc` (build), `libc`
