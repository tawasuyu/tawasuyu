# llimphi-voxel-app — máquina de cine voxel (machinima)

App showcase del motor voxel 3D de Llimphi, convertida en una **máquina de cine
voxel**: generar escenas controladas, mover personajes y **filmar una película**
— sin pantalla, de forma determinista, a un video con sonido.

No es un clon de juego: es la prueba de que el motor (`llimphi-3d`) + la capa de
dinámica (`llimphi-voxel`) alcanzan para *dirigir* una escena. El diseño y el
porqué están en `01_yachay/dominium/MOTOR-VOXEL.md` §12; este README es la **guía
práctica**: cómo correrla y cómo escribir tu propia película.

---

## Los tres modos

```bash
cargo run -p llimphi-voxel-app --release -- --film    # filma el guion → /tmp/voxel_film.mkv (AV1+Opus, con audio)
cargo run -p llimphi-voxel-app --release -- --poses   # catálogo de clips de actor → /tmp/actor_clips.png
cargo run -p llimphi-voxel-app --release -- --vox     # importa un .vox y lo renderiza → /tmp/vox_import.png
cargo run -p llimphi-voxel-app --release              # ventana interactiva (orbitar / explorar en 1ª persona)
mpv /tmp/voxel_film.mkv                                # ver la película
```

Todo el render de cine es **headless** (no necesita display): se rinde cada
cuadro a una textura, se baja por supersampling a PNG, y `ffmpeg` los junta.

---

## Arquitectura en capas

```
                 takiy-core/synth ─ banda sonora (Score → WAV)
                          │
 foreign-vox ── .vox ─────┤
 foreign-av  ── ffmpeg ───┤
                          ▼
   app (este crate)  ── screenplay() + soundtrack() + golem_model()
        │  Sequence (dirección) · Actor (reparto) · World (escenario)
        ▼
   llimphi-voxel  ── Actor/Clip · director (ActorScript/Shot/Sequence) · terrain · vox import
        ▼
   llimphi-3d     ── Scene3d · Camera3d · CameraTrack · VoxelRenderer · Renderer3d · Hud
        ▼
        wgpu (Vulkan/Metal/DX12/GL/WebGPU)
```

El motor (`llimphi-3d`) es **general** y no sabe de cine; la dirección vive en
`llimphi-voxel::director` (data) y el **guion concreto** en la app (`film.rs`).

---

## Escribir una película (el guion)

Una película es un [`Sequence`]: un reparto de **guiones de actor**
(`ActorScript`) + una lista de **planos de cámara** (`Shot`) + la duración. Todo
es determinista por tiempo: `t` fija el estado de la escena.

### 1. El guion de cada actor — `ActorScript`

Keyframes `(t, posición de grilla, clip?, rumbo?)`. La posición interpola; el
clip y el rumbo son automáticos si no se fijan (camina si se mueve, mira hacia
donde va).

```rust
use llimphi_voxel::{ActorScript, ActorKey, Clip};
use std::f32::consts::{FRAC_PI_2, PI};

let script = ActorScript::new(vec![
    ActorKey::at(0.0, 34.0, 50.0),                          // arranca acá (en grilla)
    ActorKey::at(2.6, 50.0, 50.0).facing(FRAC_PI_2),        // camina hasta acá mirando +X
    ActorKey::at(3.0, 50.0, 50.0).play(Clip::Wave).facing(PI), // gira a cámara y saluda
    ActorKey::at(5.6, 50.0, 50.0).play(Clip::Wave).facing(PI), // sostiene hasta el final
]);
// script.sample(t) → ActorSample { gx, gz, facing, clip }
```

- **`.facing(yaw)`** en dos keys consecutivas → el actor **gira suave** entre
  ambas (interpolación por el ángulo más corto). En una sola → mantiene ese rumbo.
- **`.play(clip)`** fuerza un clip (saludar, festejar…); sin él, el director
  elige `Walk` si el segmento se mueve o `Idle` si está quieto.
- Posición en **grilla** (`gx, gz`): la app la posa sobre el relieve con
  `World::ground_at` cada cuadro.

### 2. Los planos de cámara — `Shot` + `CameraTrack`

Una `CameraTrack` interpola poses `(t, eye, target, fov)` con suavizado; un
`Shot` la ancla a un instante. Varios planos = **cortes duros** (cambia de plano
sin interpolar entre ellos).

```rust
use llimphi_3d::{CameraTrack, CamKey, glam::Vec3};
use llimphi_voxel::{Sequence, Shot};

let focus = Vec3::new(0.0, 8.0, 0.0); // a dónde mira (mundo, centrado en el origen)

let establishing = CameraTrack::new(vec![
    CamKey::look(0.0, focus + Vec3::new(-6.0, 9.0, -30.0), focus, 52.0), // grúa lejos
    CamKey::look(2.8, focus + Vec3::new(-2.0, 3.5, -17.0), focus, 44.0), // entra
]);
let closeup = CameraTrack::new(vec![
    CamKey::look(0.0, focus + Vec3::new(0.0, 2.6, -11.0), focus, 40.0),  // plano corto
    CamKey::look(2.8, focus + Vec3::new(1.4, 2.1, -9.0), focus, 37.0),   // empuja
]);

let seq = Sequence::new(
    vec![script],                                    // reparto
    vec![Shot::new(0.0, establishing), Shot::new(2.8, closeup)], // corte a los 2.8 s
    5.6,                                             // duración
);
// seq.camera(t) → Camera3d  ·  seq.frames(fps) → nº de cuadros  ·  seq.cast_centroid(t)
```

Las `CamKey` se construyen **después** de conocer el terreno (la `Y` del foco
sale de `World::ground_at`), por eso el guion se arma en `screenplay()` con el
mundo ya construido.

### 3. Reproducir — el bucle de `--film`

```rust
for f in 0..seq.frames(FPS) {
    let t = f as f32 / FPS as f32;
    for (actor, script) in cast.iter_mut().zip(&seq.actors) {
        let s = script.sample(t);
        actor.pos = world.ground_at(s.gx as u32, s.gz as u32); // posar sobre el relieve
        actor.facing = s.facing;
        actor.set_clip(s.clip);   // dispara el cross-fade si cambió de clip
        actor.advance(dt);        // avanza la animación
    }
    // … subir las mallas de los actores, render con seq.camera(t), volcar PNG …
}
```

**El guion canónico vive en `src/film.rs::screenplay()`** — copialo y editalo
para tu propia escena.

---

## El reparto — `Actor` y la librería de clips

Un [`Actor`] es un muñeco de cajas articuladas (cabeza/torso/2 brazos/2 piernas)
que produce una **malla** por cuadro; la app la sube a un `Renderer3d` y la
compone con el terreno en `Scene3d` (oclusión correcta).

```rust
use llimphi_voxel::{Actor, Clip};
use llimphi_3d::glam::Vec3;

let mut a = Actor::new(Vec3::ZERO, 0.0)
    .with_colors([0.88,0.70,0.56], [0.82,0.28,0.26], [0.18,0.20,0.28]); // piel, remera, pantalón
a.set_clip(Clip::Run);   // cambia con cross-fade suave (no corta)
a.advance(dt);           // avanza la fase a la cadencia del clip
let (verts, idx) = a.mesh();   // malla local de la pose actual
let model = a.model();         // matriz de ubicación (pos + rumbo)
```

Clips disponibles: `Idle`, `Walk`, `Run`, `Wave`, `Point`, `Cheer`.

### Agregar un clip nuevo

Un clip es una **función `fase → Pose`** (los ángulos de todas las
articulaciones). Para sumar una animación, editá `llimphi-voxel/src/actor.rs`:

1. Agregá la variante a `enum Clip`.
2. Dale una cadencia en `Clip::cadence`.
3. Devolvé su `Pose` en `Clip::pose` (mezclando `leg_l/r`, `arm_l/r`,
   `arm_l/r_out` —apertura lateral—, `head_pitch`, `bob`, `lean`).

No se toca el render: `Actor::mesh` hornea cualquier `Pose`. Verificalo con
`--poses` (vuelca una fila etiquetada con todos los clips).

---

## Importar assets diseñados — `.vox` (MagicaVoxel)

Diseñás un set o personaje en MagicaVoxel y lo metés a la escena:

```rust
// Un archivo .vox → VoxelGrid del motor (remapea z-arriba → y-arriba):
let grid = llimphi_voxel::load_grid("mi_modelo.vox")?;

// O componer un set metiendo piezas en un grid existente:
let model = foreign_vox::parse(&std::fs::read("pieza.vox")?)?[0].clone();
llimphi_voxel::stamp(&mut grid, &model, [origin_x, origin_y, origin_z]);
```

`foreign_vox::{parse, write}` también permite **exportar** una escena voxel a
`.vox` para editarla afuera. El modo `--vox` genera un golem programático
(`golem_model()`), lo escribe a `.vox` y lo reimporta — el camino real de carga.

> Nota: si el `.vox` no trae chunk `RGBA`, se usa una paleta de fallback (rampa
> HSV). Los exportes de MagicaVoxel siempre incluyen `RGBA`, así que entran bien.

---

## La banda sonora — `takiy`

`src/soundtrack.rs` compone un `Score` con `takiy-core` (progresión, pads, bajo,
melodía) y lo sintetiza a WAV con `takiy-synth`:

```rust
use takiy_core::{Score, Track, ScoreNote, Pitch, Chord, ChordQuality, PitchClass};
use takiy_synth::{OscRenderer, Renderer, Waveform, Adsr, write_wav};

let mut score = Score::new(86.0); // bpm
let mut lead = Track::new("lead");
lead.add(ScoreNote::new(Pitch::from_midi(72).unwrap(), 0.0, 1.0, 78)); // (pitch, start, dur, vel)
score.add_track(lead);

let buf = OscRenderer { sample_rate: 44_100, waveform: Waveform::Triangle, envelope: Adsr::DEFAULT }
    .render(&score);
write_wav(&buf, "/tmp/banda.wav")?;
```

`--film` muxea ese WAV al video con `foreign_av::encode_frames(pattern, fps, crf,
Some(audio_path), out)` → pista **Opus**, recortada a la duración del video
(`-shortest`).

---

## El render (cómo se ve bien sin pantalla)

- **Headless**: cada cuadro se rinde a una textura intermedia y se lee de vuelta
  a PNG (`crate::write_png_downsampled`); `--film` arma la secuencia
  `frame_%04d.png` y la pasa a `foreign_av::encode_frames` (AV1 + Opus).
- **Supersampling (SSAA)**: `--film`/`--vox` renderizan a **2×** y bajan
  promediando bloques 2×2 → antialias de los bordes duros del ray-march.
- **Luz con color**: el sol se tiñe por su elevación (cálido al ras → blanco en
  lo alto) y el ambiente por el color de cielo (rebote frío). Se controla el
  *mood* moviendo `VoxelRenderer::sun_dir` y `Atmosphere::{sky_zenith, sky_horizon}`.
- **HUD/texto**: `llimphi_3d::{Hud, HudQuad}` pinta overlays screen-space (mira,
  texto con fuente bitmap 5×7) — lo usa `--poses` para las etiquetas.

---

## Gotchas (aprendidos filmando)

- **Encuadre del trío**: si los actores están muy separados en Z y la cámara
  cerca, el del carril cercano se come el lente. Apretá la separación (±2.5) o
  alejá la cámara (≥9 unidades).
- **Tierra firme y plana**: el terreno es procedural; `find_flat_strip` (en
  `film.rs`) elige un llano sobre el nivel del mar para que no caminen sobre el
  agua ni los hunda el relieve.
- **El monumento**: el escenario trae un cubo-malla flotante de demo; apagalo con
  `World::show_monument(false)` cuando filmás personajes.
- **`sun_dir` y caras a cámara**: el sol ilumina las caras que lo miran; si tu
  cámara mira la cara opuesta al sol, esa cara queda sólo con ambiente. Orientá
  `sun_dir` hacia la cámara para un *hero shot*.
- **`ffmpeg`**: hace falta en el PATH con `libsvtav1` + `libopus`. Sin él, los
  PNG quedan igual en `/tmp/voxel_film/` y se avisa cómo muxear a mano.
- **Determinismo**: todo es función de `t` (y de semillas LCG). El mismo guion
  produce el mismo video — ideal para iterar.

---

## Mapa de archivos

| Archivo | Qué tiene |
|---|---|
| `src/main.rs` | flags (`--film/--poses/--vox`), bucle interactivo, `write_png[_downsampled]` |
| `src/film.rs` | `screenplay()` (el guion), `--film`/`--poses`/`--vox`, `golem_model()`, SSAA |
| `src/world.rs` | escenario: terreno + atmósfera + monumento + manada; `render_with`, `ground_at` |
| `src/soundtrack.rs` | la banda sonora (takiy) |
| `llimphi-voxel/src/{actor,director,vox}.rs` | reparto, timeline, import `.vox` (reusables) |
| `llimphi-3d/src/{scene,camera,cinema,voxel_renderer,renderer,hud}.rs` | el motor 3D general |

[`Actor`]: ../llimphi-voxel/src/actor.rs
[`Sequence`]: ../llimphi-voxel/src/director.rs
