# shuma-exec

> Ejecutor de comandos de [shuma](../../README.md).

Envuelve `std::process::Command` con job-control, signal handling, env del session. Pipes, redirects, &&/||.

## Deps

- [`shuma-core`](../shuma-core/README.md), `nix`
