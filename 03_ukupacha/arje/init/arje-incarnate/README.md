# arje-incarnate

> Process materialization for [arje](../../README.md).

Takes an arje object (chunks + manifest), "incarnates" it as an executable process: extracts needed binaries, mounts capabilities, launches.

## Deps

- [`arje-cas`](../../runtime/arje-cas/README.md), `nix`
