# shuma-exec

> Command executor of [shuma](../../README.md).

Wraps `std::process::Command` with job-control, signal handling, session env. Pipes, redirects, &&/||.

## Deps

- [`shuma-core`](../shuma-core/README.md), `nix`
