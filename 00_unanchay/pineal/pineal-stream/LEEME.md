# pineal-stream

> Canvas de series temporales con scroll para [pineal](../README.md).

Buffer circular fixed-size; cada nueva muestra empuja el frame a la izquierda. Múltiples series sobre el mismo eje de tiempo. Útil para monitoreo en vivo (CPU, latencia, sensores). Sin retención (eso es trabajo de la app que decide cuánto guardar).

## API

```rust
use pineal_stream::{Stream, Window};

let mut s = Stream::new(Window::seconds(60));
s.push(t, sample);
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
