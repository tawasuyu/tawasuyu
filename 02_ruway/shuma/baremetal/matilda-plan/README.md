# matilda-plan

> Diff planner (actual → desired) of [matilda](../../README.md).

Takes actual + desired, produces a dependency-ordered `Vec<Action>`. Each `Action` is atomic and reversible.

## Deps

- [`matilda-core`](../matilda-core/README.md), [`matilda-discover`](../matilda-discover/README.md)
