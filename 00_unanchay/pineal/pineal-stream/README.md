# pineal-stream

> Scrolling time-series canvas for [pineal](../README.md).

Fixed-size ring buffer; each new sample pushes the frame left. Multiple series on the same time axis. Useful for live monitoring (CPU, latency, sensors). No retention (the app decides how much to keep).

## API

```rust
use pineal_stream::{Stream, Window};

let mut s = Stream::new(Window::seconds(60));
s.push(t, sample);
```

## Deps

- [`pineal-core`](../pineal-core/README.md)
