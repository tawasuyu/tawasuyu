# shuma-sysmon

> Embedded system monitor of [shuma](../../README.md).

Reads /proc periodically and publishes CPU/mem/disk/net to a [`chasqui`](../../../chasqui/README.md) topic. Subscribable from any app.

## Deps

- [`chasqui-core`](../../../chasqui/chasqui-core/README.md), `sysinfo`
