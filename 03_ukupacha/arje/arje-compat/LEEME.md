# arje-compat

> Compat con userspace POSIX (shims) de [arje](../README.md).

Reimplementa el mínimo necesario para que binarios estáticos POSIX corran adentro de un objeto arje sin que el sistema host esté presente: `libc` shims, `/dev/null`, ttys básicas.

## Deps

- Cero deps externas (es la base)
