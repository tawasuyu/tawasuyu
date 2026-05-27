# arje-compat

> POSIX userspace compat (shims) of [arje](../README.md).

Reimplements the minimum needed for static POSIX binaries to run inside an arje object without the host being present: `libc` shims, `/dev/null`, basic ttys.

## Deps

- Zero external deps (it's the base)
