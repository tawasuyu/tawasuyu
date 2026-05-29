# tinkuy-abi

> ABI plana, C-friendly, para [tinkuy](../README.md).

Expone el `World + Grid3D` de `tinkuy-core` a través de un handle opaco y una API `extern "C"` pensada para cruzar fronteras WASM y FFI sin exponer genéricos de Rust. Todo va envuelto en `TkSim` y devuelve códigos de estado `i32`.

## Superficie de la API

```c
int32_t tk_sim_new(uint32_t cap, const float origin[3], float cell_size,
                   const uint32_t dims[3], TkSim** out);
void    tk_sim_free(TkSim* sim);
int32_t tk_sim_spawn(TkSim* sim, float x, float y, float z,
                     float vx, float vy, float vz, float m, float q,
                     uint32_t* out_idx);
uint32_t tk_sim_len(const TkSim* sim);
int32_t tk_sim_rebuild_grid(TkSim* sim);
int32_t tk_sim_step_lj(TkSim* sim, float dt, float eps, float sigma, float cutoff,
                       const float bmin[3], const float bmax[3]);
double  tk_sim_kinetic_energy(const TkSim* sim);
double  tk_sim_temperature(const TkSim* sim, double kb);
int32_t tk_sim_total_momentum(const TkSim* sim, float out_xyz[3]);
int32_t tk_sim_snapshot_cid(const TkSim* sim, uint8_t out[32]);
int32_t tk_sim_snapshot_export(const TkSim* sim, uint8_t** out_ptr, uintptr_t* out_len);
int32_t tk_sim_positions(const TkSim* sim, float* out, uint32_t cap_count);
void    tk_buf_free(uint8_t* ptr, uintptr_t len);
```

Códigos: `TK_OK = 0`, `TK_ERR_NULL = -1`, `TK_ERR_INVALID = -2`, `TK_ERR_OOM = -3`.

## Quién la consume

- `03_ukupacha/wawa/apps/tinkuy` — re-exporta este crate como cdylib (`pub use tinkuy_abi::*;`) → el blob WASM que carga `wawa-kernel`.
- `wawa-kernel/src/tinkuy.rs` — resuelve estos símbolos como `wasmi::TypedFunc`.

## Deps

- [`tinkuy-core`](../tinkuy-core/LEEME.md)
