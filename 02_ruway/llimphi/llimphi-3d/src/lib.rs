//! # llimphi-3d — pase 3D base de Llimphi (M0 del motor 3D)
//!
//! Lo mínimo para tener **3D real dentro de un `View` de Llimphi**: una
//! [`Camera3d`] (matrices view/proj con `glam`), un depth buffer propio y un
//! [`Renderer3d`] que dibuja geometría indexada con test de profundidad sobre
//! la textura intermedia del frame.
//!
//! ## Cómo encaja con el bucle Elm + vello + wgpu
//!
//! Llimphi ya rasteriza la UI con vello sobre una textura intermedia y expone
//! [`View::gpu_paint_with`] para inyectar una pasada GPU directa *después* de
//! vello (con `LoadOp::Load`, preservando la UI). [`Renderer3d::render`] tiene
//! **exactamente** la firma que esa closure necesita
//! (`device, queue, encoder, target_view, (w, h), &camera`), así que un nodo 3D
//! es:
//!
//! ```ignore
//! let r3d = Arc::new(Mutex::new(Renderer3d::new(&device, fmt)));
//! View::empty().gpu_paint_with(move |dev, q, enc, view, rect, vp| {
//!     r3d.lock().unwrap().render(dev, q, enc, view, vp, &camera);
//! })
//! ```
//!
//! No es un segundo motor: corre sobre el **mismo wgpu** que ya usa Llimphi,
//! que a su vez traduce a Vulkan/Metal/DX12/GL/WebGPU. Ver
//! `01_yachay/dominium/MOTOR-VOXEL.md` §11 para la ruta completa (M0..M4,
//! ray-march de voxels sparse en los hitos siguientes).
//!
//! [`View::gpu_paint_with`]: https://docs/llimphi-compositor

pub use glam;
pub use wgpu;

mod camera;
mod mesh;
mod renderer;

pub use camera::Camera3d;
pub use mesh::{cube, Vertex3d};
pub use renderer::Renderer3d;
