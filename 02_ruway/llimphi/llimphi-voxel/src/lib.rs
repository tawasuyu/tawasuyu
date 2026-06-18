//! # llimphi-voxel — dinámica voxel/juego sobre `llimphi-3d`
//!
//! Capa **opcional** y más comprometida con la "dinámica tipo-Minecraft":
//! genera y maneja el *contenido* y la *interacción* de un mundo voxel sobre el
//! motor 3D **general** [`llimphi_3d`], que es quien renderiza
//! ([`llimphi_3d::Scene3d`] + [`llimphi_3d::VoxelRenderer`]).
//!
//! Rama de dependencias pensada como **librería reusable por cualquier juego
//! voxel** de la suite: `app → llimphi-voxel → llimphi-3d → wgpu`. El motor
//! (cámara, depth compartido, ray-march, mallas) no sabe nada de juegos; acá
//! vive lo que sí: world-gen ([`terrain`]) y picking/edición ([`raycast`]).
//! El resto (chunks, streaming, bloques tipados) crece acá sin tocar el motor.

mod actor;
mod critter;
mod director;
mod lod;
mod player;
mod raycast;
mod terrain;
mod vox;
mod world_stream;

pub use actor::{Actor, Clip, Pose};
pub use director::{ActorKey, ActorSample, ActorScript, Sequence, Shot};
pub use vox::{load_grid, load_scene_grid, model_to_grid, scene_to_grid, stamp, VoxLoadError};
pub use critter::Critter;
pub use lod::{lod_skirt, lod_skirt_pyramid, LodParams, LodRing};
pub use player::{forward_h, look_dir, right_h, Player};
pub use raycast::{raycast, VoxelHit};
pub use terrain::{column_height, fill_terrain_window, terrain};
pub use world_stream::WorldStream;
