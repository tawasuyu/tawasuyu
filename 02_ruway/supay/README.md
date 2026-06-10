# supay

> `supay` (Quechua: *spirit, demon*). DOOM-style renderer over Llimphi.

Bridge between the `doomgeneric` engine (C) and Llimphi: FFI + WAD sprite atlas + palettes + reconstructed level scene + vello rendering. Serves two purposes: (1) prove the Llimphi/Wawa stack can run gaming-grade workloads; (2) immediate compatibility with original and community WADs. Phase-by-phase detail in [SDD.md](SDD.md).

## Install

```sh
# real Doom (precondition: doom1.wad, shareware or registered, in cwd)
cargo run --release -p supay-doom-llimphi

# Phase 0 mini-raycaster (hardcoded, no WAD needed)
cargo run --release -p supay-app-llimphi

# headless frame dump to PNG (verification without a window)
cargo run --release -p supay-doom-llimphi --example dump_frame
```

## Compatibility

- **Linux / macOS / Windows** — native Llimphi + `cc` to build `doomgeneric`.
- **Wawa** — `supay-core/scene/wad` compile to WASM; the renderer over the Wawa HAL is not closed yet (see "Status").

| Crate | Role |
|---|---|
| [`supay-core`](supay-core/README.md) | FFI to `doomgeneric` + safe `DoomEngine`. |
| [`supay-wad`](supay-wad/README.md) | WAD parser (lumps, patches, flats, sprites). |
| [`supay-scene`](supay-scene/README.md) | Level snapshot: sectors, mobjs, player. |
| [`supay-render-llimphi`](supay-render-llimphi/README.md) | `scene_view` → vello polygons + atlas. |
| [`supay-audio`](supay-audio/) | Doom mixer over cpal: WAD SFX + MUS→FM music + takiy bridge. |
| [`supay-mini-core`](supay-mini-core/) | Mini-raycaster world + sim (Phase 0), GUI-agnostic. |
| [`supay-doom-llimphi`](supay-doom-llimphi/README.md) | Driver: links engine + atlas + UI. |
| [`supay-app-llimphi`](supay-app-llimphi/README.md) | Phase 0 mini-raycaster binary (paints `supay-mini-core`). |

## Considerations

- **Legal WAD:** only shareware `doom1.wad` is referenced; others come from you.
- `vendor/doomgeneric/`: clone it from upstream before build (`build.rs` detects).
- **`FEATURE_SOUND=0`** stays: audio doesn't come from the C engine but from `supay-audio` (Rust), which already synthesizes WAD SFX + MUS music and bridges `takiy` (`AudioEngine::play_takiy_score`).
- Simplified 3D rendering: BSP-correct painter's ordering (Phase 3.13b) but no per-column BSP occlusion clipping yet — see "Status" below.

## Status (2026-05-31)

### Done

- **Mature software 3D renderer over vello**: textured walls (TEXTURE1+PNAMES,
  textureoffset/rowoffset alignment + pegging), per-triangle perspective-correct
  floors/ceilings with back-to-front BSP ordering, sky scrolling, animated
  flats/walls, full-bright.
- **Real WAD sprites** with directional rotation, light attenuation and a sample
  point at the patch's real height.
- **Unified BSP painter's ordering (Phase 3.13b)**: walls, sprites, planes and decals
  share a primary ordering key — the back-to-front rank of the subsector that
  contains them (`compute_bsp_ranks` + `bsp_rank_at`), with euclidean distance only as
  an intra-subsector tiebreaker. Closes the wall↔sprite crossing bug (a sprite near in
  euclidean distance no longer pierces a wall the BSP puts in front). Without BSP (stub)
  it falls back to the historical euclidean order.
- **Advanced lighting (Phases 3.22–3.39)**: world point lights from `FF_FULLBRIGHT`
  mobjs, muzzle world light with sectorial occlusion (multi-hop BFS, Dijkstra-lite
  cumulative radius), directional 3D BRDF (rim) for walls, floors/ceilings,
  sprites and weapon; per-spritenum tint per channel; Doom 2 tint table + pickups + keys.
- **HUD/weapons**: weapon psprite, muzzle flash (`ps_flash`) + berserk tint, player
  palette overlays (damage/pickup/radsuit), **invulnerability with real color
  inversion** (`Difference` blend, photographic negative of Doom's colormap, not the old
  white approximation), weapon shading by sector light.
- **Mouse-look**: cosmetic pitch (y-shear) via PageUp/PageDown/Home **and via mouse
  drag**. Doom has no real vertical aim; this moves the horizon.
- **Audio (supay-audio, Phases 4.0–4.6)**: SFX from the WAD, MUS→synth music with
  GENMIDI (FM per instrument), equal-power spatialization + per-sector reverb,
  ambient crossfade, low-pass occlusion per linedef and per opening (closed doors
  muffle the sound). Closes the audio gap (0% → playing).
- **Menus** (batches 4 and 6): main menu + context menus.
- Rule #1 refactors: split of `supay-render` (8556 LOC) and `supay-app-llimphi` main (1849);
  extraction of the raycaster's world+sim into `supay-mini-core` (rule #2).
- **Headless frame dump to PNG** (`supay-doom-llimphi/examples/dump_frame.rs`) to
  verify the renderer without a window.

### Pending

- **Visibility BSP-walking** (exact occlusion). The *ordering* is already BSP-correct
  (Phase 3.13b, see above), but *visibility* is still resolved by painter's overdraw:
  there is no per-column clipping of solid segments like the original R_RenderBSPNode,
  so geometry that would be hidden is still drawn. It works, but wastes fill.
- **`FEATURE_SOUND` of the C engine at 0**: audio does not come from `doomgeneric` (C) but
  from `supay-audio` (Rust), which already synthesizes SFX + music and **already bridges `takiy`**
  (`AudioEngine::play_takiy_score` renders a `takiy_core::Score` and queues it as one
  more voice of the mixer). There is no pending integration with takiy — its own FM/OPL
  synthesis does not overlap with the basic oscillators of `takiy-synth`. The only open
  item is raising `FEATURE_SOUND` if one day the C engine's audio is wanted instead of the
  native one (not the plan).
- **Wawa**: `supay-core/scene/wad` compile to WASM, but the renderer over the Wawa HAL
  is not closed yet (depends on `llimphi-ui` exposing a `custom_pass` wgpu usable
  outside Linux). It is a `no_std` port to another target, not pending polish.
- **Cycle-accurate OPL2**: the music synth is a 2-operator FM approximation, not an
  exact OPL2 (no KSL/vibrato/tremolo). Future option: Nuked-OPL or a GM soundfont.
- Phase-by-phase detail and fine debt in [SDD.md](SDD.md).
