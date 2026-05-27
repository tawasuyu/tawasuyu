# arje-kernel

> Minimal kernel of [arje](../../README.md) (separate from wawa-kernel).

Functionality subset: scheduler, syscall table, IPC bus, read-only FS from the mounted image. When the system requires wawa-kernel, arje-kernel cleanly hands over control.

## Deps

- `nix`, `linked-list-allocator`
