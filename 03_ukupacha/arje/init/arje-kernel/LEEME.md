# arje-kernel

> Kernel mínimo de [arje](../../README.md) (separado del wawa-kernel).

Subset de funcionalidad: scheduler, syscall table, IPC bus, FS read-only desde la imagen montada. Cuando el sistema requiere wawa-kernel, arje-kernel le pasa el control limpio.

## Deps

- `nix`, `linked-list-allocator`
