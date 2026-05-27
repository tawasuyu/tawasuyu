# matilda-plan

> Planificador de diff (actual → deseado) de [matilda](../../README.md).

Toma actual + deseado, produce `Vec<Action>` ordenada por dependencia. Cada `Action` es atómica y reversible.

## Deps

- [`matilda-core`](../matilda-core/README.md), [`matilda-discover`](../matilda-discover/README.md)
