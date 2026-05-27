# supay

> `supay` (Quechua: *spirit, demon*). DOOM-style renderer over Llimphi.

Bridge between the `doomgeneric` engine (C) and Llimphi: FFI + WAD sprite atlas + palettes + reconstructed level scene + vello rendering. Serves two purposes: (1) prove the Llimphi/Wawa stack can run gaming-grade workloads; (2) immediate compatibility with original and community WADs. Phase-by-phase detail in [SDD.md](SDD.md).

## Install

```sh
# precondition: place doom1.wad (shareware or registered) in cwd
cargo run --release -p supay-app-llimphi
cargo run --release -p supay-doom-llimphi
```

## Compatibility

- **Linux / macOS / Windows** — native Llimphi + `cc` to build `doomgeneric`.
- **Wawa** — `supay-core/scene/wad` compile to WASM; renderer uses the Wawa HAL.

Crates listed in [README.md](README.md).

## Considerations

- **Legal WAD:** only shareware `doom1.wad` is referenced; others come from you.
- `vendor/doomgeneric/`: clone it from upstream before build (`build.rs` detects).
- **`FEATURE_SOUND=0`** for now; audio bus goes through `takiy` when ready.
- Simplified 3D rendering (no real BSP-walking); directional sprites only at angle 1 until Phase 3.5.
