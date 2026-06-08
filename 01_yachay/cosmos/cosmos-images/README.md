# cosmos-images

Pure Rust astronomical image format library (FITS and XISF support).

[![Crates.io](https://img.shields.io/crates/v/cosmos-images)](https://crates.io/crates/cosmos-images)
[![Documentation](https://docs.rs/cosmos-images/badge.svg)](https://docs.rs/cosmos-images)
[![License: Apache 2.0](https://img.shields.io/crates/l/cosmos-images)](https://gitea.tawasuyu.net/sergio/eternal)

Read, write, and process FITS, XISF, and SER scientific image formats with compression support (Gzip, Rice), binary/ASCII tables, and Bayer demosaicing. No runtime FFI.

## Installation

```toml
[dependencies]
cosmos-images = "0.1"
```

## Modules

| Module     | Purpose                                                      |
|------------|--------------------------------------------------------------|
| `core`     | BitPix, ByteOrder, error types, Result alias                 |
| `fits`     | FITS reader/writer (Primary, Image, ASCII/Binary Table HDUs) |
| `xisf`     | XISF (Extensible Image Serialization Format) reader          |
| `ser`      | SER video format reader/writer with frame timestamps         |
| `formats`  | Unified AstroImage abstraction across formats                |
| `debayer`  | Bayer pattern demosaicing (bilinear interpolation)           |
| `ricecomp` | Rice compression/decompression codec                         |

## Example

```rust
use eternal_images::{FitsFile, BitPix};

// Open a FITS file and read the primary HDU
let mut fits = FitsFile::open("m31.fits")?;
let primary = fits.primary_hdu()?;

// Access header keywords
let object = primary.header().get_string("OBJECT")?;
let exposure = primary.header().get_f64("EXPTIME")?;
println!("{}: {:.1}s exposure", object, exposure);

// Read image data as f32
let (header, data) = fits.primary_hdu_with_data::<f32>()?;
let width = header.get_i64("NAXIS1")? as usize;
let height = header.get_i64("NAXIS2")? as usize;
println!("Image: {}x{} pixels", width, height);
```

## Features

- **`parallel`** (default) — Enables parallel processing via rayon
- **`simd`** — SIMD acceleration for image operations via wide
- **`standard-formats`** — PNG/TIFF export support

## Design Notes

- **Memory-mapped I/O**: Large files use memory mapping for efficient random access without loading entire files into RAM.
- **Strict FITS compliance**: 2880-byte block alignment is validated. Non-compliant files produce errors, not silent corruption.
- **Type-safe data access**: BitPix enum and DataArray trait prevent accidental type mismatches when reading image data.
- **Streaming headers**: HDU headers are parsed on demand and cached, avoiding upfront parsing of multi-extension files.

## License

Licensed under the Apache License, Version 2.0
([LICENSE-APACHE](../LICENSE-APACHE) or
<https://www.apache.org/licenses/LICENSE-2.0>).
See [NOTICE](../NOTICE) for upstream attribution.

## Acknowledgements

Forked from [celestial](https://github.com/gaker/celestial) by **Greg Aker**
(originally dual-licensed under MIT OR Apache-2.0). This crate is derived
directly from that work and is maintained in this fork by Sergio Velásquez
Zeballos with Claude (Anthropic).

## Contributing

See the [repository](https://gitea.tawasuyu.net/sergio/eternal) for contribution guidelines.
