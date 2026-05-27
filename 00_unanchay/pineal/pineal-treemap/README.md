# pineal-treemap

> Hierarchical treemap canvas for [pineal](../README.md).

Implements squarified treemap layout (Bruls et al.): divides a rectangle into sub-rectangles proportional to a weight, recursively across the hierarchy. Useful for filesystems, budgets, CPU profiles.

## API

```rust
use pineal_treemap::{Treemap, Node};

let tm = Treemap::new(root_node)
    .color_by(|n| color_for(n.tag));
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
