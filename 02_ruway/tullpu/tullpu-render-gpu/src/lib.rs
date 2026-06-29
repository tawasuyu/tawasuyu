//! `tullpu-render-gpu` — compositor GPU del editor de capas.
//!
//! Espejo de [`tullpu_render::componer`] sobre un compute shader `wgpu`. La
//! **recursión de grupos** (carpetas/aislamiento) y el orden visual viven en el
//! host, idénticos al compositor CPU; lo que migra a la GPU es el bucle
//! per-píxel caro: una invocación de `blend.wgsl` por píxel funde una capa
//! sobre el acumulador aplicando modo de fusión, opacidad, máscara y clip.
//!
//! ## Forma del acumulador
//!
//! `acc` y los buffers de capa son `array<u32>`: un píxel rgba8 empaquetado
//! little-endian por elemento, byte-a-byte idéntico al `Vec<u8>` del compositor
//! CPU. Como cada capa lee y reescribe `acc` en rgba8 (no en f32 acumulado), el
//! redondeo intermedio coincide con la CPU y la paridad es de ±1 por canal.
//!
//! ## Cobertura
//!
//! Paridad **completa** con el compositor CPU: capas `Pixeles`/`Texto`/`Grupo`
//! con los 28 modos de fusión (incluido **Disolver**, vía splitmix64 emulado en
//! `disolver.wgsl` con `vec2<u32>` por falta de u64 en WGSL), máscaras,
//! opacidad, clipping **y capas de ajuste** ([`ClaseCapa::Ajuste`]) vía
//! `ajuste.wgsl` (las ops independientes por canal se compilan a una LUT con el
//! código exacto de la CPU; Saturacion/Tonalidad portan el HSL). Tres shaders
//! sobre el mismo acumulador `acc`. El único fallback al compositor CPU es la
//! ausencia de adaptador GPU (lo decide el caller).

#![forbid(unsafe_code)]

use image::RgbaImage;
use tullpu_core::{pixel, Capa, ClaseCapa, Hash, Lienzo, ModoFusion, OpLocal, Uuid};
use wgpu::util::DeviceExt;

// Reusamos la fuente de buffers del compositor CPU: así el mismo almacén
// (el de la app) sirve a ambos compositores sin traits paralelos.
pub use tullpu_render::{AlmacenEnMemoria, FuenteBuffers};

// =============================================================================
//  Errores
// =============================================================================

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("no se encontró un adaptador GPU")]
    NoAdapter,
    #[error("request_device falló: {0}")]
    RequestDevice(String),
    #[error("buffer faltante: hash {0:02x?}")]
    BufferFaltante(Hash),
    #[error("tamaño de buffer Rgba8 inválido para {hash:02x?}: esperaba {esperado}, encontré {encontrado}")]
    TamanioRgba {
        hash: Hash,
        esperado: usize,
        encontrado: usize,
    },
    #[error("tamaño de máscara inválido para {hash:02x?}: esperaba {esperado}, encontré {encontrado}")]
    TamanioMascara {
        hash: Hash,
        esperado: usize,
        encontrado: usize,
    },
    #[error("lienzo vacío (0 píxeles)")]
    Vacio,
    #[error("mapeo de readback falló")]
    Readback,
}

// =============================================================================
//  Mapeo ModoFusion → código del shader
// =============================================================================

/// Código numérico que `blend.wgsl` espera para cada modo. Match explícito
/// (no `as u32` sobre el discriminante) para que reordenar el enum no rompa
/// silenciosamente el shader.
fn modo_codigo(m: ModoFusion) -> u32 {
    match m {
        ModoFusion::Normal => 0,
        ModoFusion::Multiplicar => 1,
        ModoFusion::Pantalla => 2,
        ModoFusion::Superponer => 3,
        ModoFusion::Aclarar => 4,
        ModoFusion::Oscurecer => 5,
        ModoFusion::Diferencia => 6,
        ModoFusion::Aditivo => 7,
        ModoFusion::SubExpQuemado => 8,
        ModoFusion::SubLinealQuemado => 9,
        ModoFusion::SobreExpAclarado => 10,
        ModoFusion::LuzFuerte => 11,
        ModoFusion::LuzSuave => 12,
        ModoFusion::LuzViva => 13,
        ModoFusion::LuzLineal => 14,
        ModoFusion::LuzPunto => 15,
        ModoFusion::MezclaDura => 16,
        ModoFusion::Exclusion => 17,
        ModoFusion::Resta => 18,
        ModoFusion::Division => 19,
        ModoFusion::HslTono => 20,
        ModoFusion::HslSaturacion => 21,
        ModoFusion::HslColor => 22,
        ModoFusion::HslLuminosidad => 23,
        ModoFusion::ColorMasOscuro => 24,
        ModoFusion::ColorMasClaro => 25,
        // Disolver no tiene código: el lienzo se rechaza antes de llegar acá.
        ModoFusion::Disolver => u32::MAX,
    }
}

// =============================================================================
//  Parámetros (uniform) — 32 bytes, espejo de `struct Params` del shader
// =============================================================================

struct Params {
    modo: u32,
    has_mask: u32,
    has_clip: u32,
    n: u32,
    opacidad: f32,
    stride: u32,
    /// Semilla del RNG de Disolver (los 8 primeros bytes del Uuid de la capa).
    /// Cero para el resto de modos (`blend.wgsl` la ignora).
    seed: u64,
}

impl Params {
    fn bytes(&self) -> [u8; 32] {
        let mut b = [0u8; 32];
        b[0..4].copy_from_slice(&self.modo.to_le_bytes());
        b[4..8].copy_from_slice(&self.has_mask.to_le_bytes());
        b[8..12].copy_from_slice(&self.has_clip.to_le_bytes());
        b[12..16].copy_from_slice(&self.n.to_le_bytes());
        b[16..20].copy_from_slice(&self.opacidad.to_le_bytes());
        b[20..24].copy_from_slice(&self.stride.to_le_bytes());
        let seed = self.seed.to_le_bytes();
        b[24..28].copy_from_slice(&seed[0..4]); // seed_lo
        b[28..32].copy_from_slice(&seed[4..8]); // seed_hi
        b
    }
}

// =============================================================================
//  Compositor
// =============================================================================

/// Posee el dispositivo GPU y el pipeline de fusión. Construirlo una vez y
/// reusarlo: la creación del adaptador/dispositivo es cara, cada
/// [`Self::componer`] es barata.
pub struct Compositor {
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::ComputePipeline,
    disolver_pipeline: wgpu::ComputePipeline,
    layout: wgpu::BindGroupLayout,
    ajuste_pipeline: wgpu::ComputePipeline,
    ajuste_layout: wgpu::BindGroupLayout,
    /// Límite de hilos por dimensión X de dispatch (de los límites del device).
    max_grupos_dim: u32,
}

impl Compositor {
    /// Inicializa el dispositivo GPU (headless) y compila el shader. Bloquea
    /// con `pollster`. Prefiere backends PRIMARY (Vulkan/Metal/DX12); cae a
    /// todos si no hay (igual criterio que `llimphi-hal`, por el bug de
    /// teardown del backend GL sobre Mesa/Wayland).
    pub fn nuevo() -> Result<Self, Error> {
        pollster::block_on(Self::nuevo_async())
    }

    async fn nuevo_async() -> Result<Self, Error> {
        let opts = wgpu::RequestAdapterOptions {
            power_preference: wgpu::PowerPreference::HighPerformance,
            force_fallback_adapter: false,
            compatible_surface: None,
        };
        let primary = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::PRIMARY,
            ..Default::default()
        });
        let (_instance, adapter) = match primary.request_adapter(&opts).await {
            Ok(a) => (primary, a),
            Err(_) => {
                let all = wgpu::Instance::new(&wgpu::InstanceDescriptor::default());
                let a = all
                    .request_adapter(&opts)
                    .await
                    .map_err(|_| Error::NoAdapter)?;
                (all, a)
            }
        };
        let limits = wgpu::Limits::default().using_resolution(adapter.limits());
        let max_grupos_dim = limits.max_compute_workgroups_per_dimension.max(1);
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("tullpu-render-gpu-device"),
                required_features: wgpu::Features::empty(),
                required_limits: limits,
                memory_hints: wgpu::MemoryHints::Performance,
                experimental_features: wgpu::ExperimentalFeatures::default(),
                trace: wgpu::Trace::Off,
            })
            .await
            .map_err(|e| Error::RequestDevice(e.to_string()))?;

        let shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tullpu-blend"),
            source: wgpu::ShaderSource::Wgsl(include_str!("blend.wgsl").into()),
        });

        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tullpu-blend-bgl"),
            entries: &[
                bgl_storage(0, false), // acc (read_write)
                bgl_storage(1, true),  // src (read)
                bgl_storage(2, true),  // mask (read)
                bgl_storage(3, true),  // clip (read)
                bgl_storage(4, false), // cobertura (read_write)
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tullpu-blend-pl"),
            bind_group_layouts: &[&layout],
            push_constant_ranges: &[],
        });

        let pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("tullpu-blend-pipeline"),
            layout: Some(&pipeline_layout),
            module: &shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // --- pipeline de Disolver (mismo bind group layout que blend) ---
        let disolver_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tullpu-disolver"),
            source: wgpu::ShaderSource::Wgsl(include_str!("disolver.wgsl").into()),
        });
        let disolver_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("tullpu-disolver-pipeline"),
            layout: Some(&pipeline_layout),
            module: &disolver_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        // --- pipeline de capas de ajuste ---
        let ajuste_shader = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tullpu-ajuste"),
            source: wgpu::ShaderSource::Wgsl(include_str!("ajuste.wgsl").into()),
        });
        let ajuste_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("tullpu-ajuste-bgl"),
            entries: &[
                bgl_storage(0, false), // acc (read_write)
                bgl_storage(1, true),  // lut (read)
                bgl_storage(2, true),  // mask (read)
                bgl_storage(3, true),  // clip (read)
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::COMPUTE,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let ajuste_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tullpu-ajuste-pl"),
            bind_group_layouts: &[&ajuste_layout],
            push_constant_ranges: &[],
        });
        let ajuste_pipeline = device.create_compute_pipeline(&wgpu::ComputePipelineDescriptor {
            label: Some("tullpu-ajuste-pipeline"),
            layout: Some(&ajuste_pl),
            module: &ajuste_shader,
            entry_point: Some("main"),
            compilation_options: wgpu::PipelineCompilationOptions::default(),
            cache: None,
        });

        Ok(Self {
            device,
            queue,
            pipeline,
            disolver_pipeline,
            layout,
            ajuste_pipeline,
            ajuste_layout,
            max_grupos_dim,
        })
    }

    /// Compone un [`Lienzo`] en la GPU y devuelve la `RgbaImage` resultante.
    /// Cubre todos los modos de fusión, ajustes, máscaras, clipping y grupos.
    pub fn componer(
        &self,
        l: &Lienzo,
        fuente: &impl FuenteBuffers,
    ) -> Result<RgbaImage, Error> {
        let w = l.width;
        let h = l.height;
        let n = (w as usize) * (h as usize);
        if n == 0 {
            return Err(Error::Vacio);
        }

        // Recursos vivos hasta el submit: la recursión empuja todo acá. (wgpu
        // los Arc-retiene internamente vía el encoder, pero los conservamos
        // explícitamente para no depender de ese detalle.)
        let mut keep = KeepAlive::default();
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor {
                label: Some("tullpu-blend-encoder"),
            });

        let acc = self.componer_lista(l, None, n, fuente, &mut encoder, &mut keep)?;

        // Readback: acc (rgba8 empaquetado) → staging mapeable → RgbaImage.
        let bytes = (n * 4) as u64;
        let staging = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("tullpu-blend-staging"),
            size: bytes,
            usage: wgpu::BufferUsages::MAP_READ | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        encoder.copy_buffer_to_buffer(&acc, 0, &staging, 0, bytes);
        self.queue.submit(std::iter::once(encoder.finish()));

        let slice = staging.slice(..);
        let (tx, rx) = std::sync::mpsc::channel();
        slice.map_async(wgpu::MapMode::Read, move |r| {
            let _ = tx.send(r);
        });
        let _ = self.device.poll(wgpu::PollType::wait_indefinitely());
        rx.recv().map_err(|_| Error::Readback)?.map_err(|_| Error::Readback)?;
        let data = slice.get_mapped_range();
        let pixeles = data.to_vec();
        drop(data);
        staging.unmap();

        RgbaImage::from_raw(w, h, pixeles).ok_or(Error::Vacio)
    }

    /// Compone las capas hijas directas de `grupo` (`None` = raíz) sobre un
    /// acumulador GPU nuevo (transparente) y lo devuelve. Espejo recursivo de
    /// `componer_lista` del compositor CPU.
    fn componer_lista(
        &self,
        l: &Lienzo,
        grupo: Option<Uuid>,
        n: usize,
        fuente: &impl FuenteBuffers,
        encoder: &mut wgpu::CommandEncoder,
        keep: &mut KeepAlive,
    ) -> Result<wgpu::Buffer, Error> {
        let acc = self.buffer_acc_cero(n);

        // Cobertura de la última capa base no-clipping (para clipping masks).
        let mut base_alpha: Option<wgpu::Buffer> = None;

        for i in l.hijos_directos(grupo) {
            let capa = &l.capas[i];
            if !capa.visible {
                continue;
            }
            let mascara = self.cargar_mascara(capa, n, fuente)?;
            let usa_clip = capa.clipping && base_alpha.is_some();

            // Capas de ajuste: aplican una op per-píxel al compuesto in-place.
            // No aportan base de clipping (igual que el compositor CPU).
            if let ClaseCapa::Ajuste(op) = &capa.clase {
                let clip_buf = match (usa_clip, &base_alpha) {
                    (true, Some(b)) => b.clone(),
                    _ => self.buffer_dummy(),
                };
                let mask_buf = mascara.unwrap_or_else(|| self.buffer_dummy());
                self.despachar_ajuste(
                    encoder, &acc, op, capa.opacidad, &mask_buf, &clip_buf, usa_clip, n, keep,
                );
                keep.buffers.push(mask_buf);
                keep.buffers.push(clip_buf);
                continue;
            }

            // Resolver el buffer fuente de la capa.
            let src = match &capa.clase {
                ClaseCapa::Grupo => {
                    self.componer_lista(l, Some(capa.id), n, fuente, encoder, keep)?
                }
                ClaseCapa::Pixeles | ClaseCapa::Texto(_) => {
                    let esperado = n * 4;
                    let bytes = fuente
                        .obtener(capa.contenido)
                        .ok_or(Error::BufferFaltante(capa.contenido))?;
                    if bytes.len() != esperado {
                        return Err(Error::TamanioRgba {
                            hash: capa.contenido,
                            esperado,
                            encontrado: bytes.len(),
                        });
                    }
                    self.buffer_storage(bytes, "tullpu-src")
                }
                // Las capas de ajuste se atienden en el `if let` de arriba.
                ClaseCapa::Ajuste(_) => unreachable!("ajuste atendido antes del match"),
            };

            // Cobertura de salida de esta capa.
            let cobertura = self.buffer_cobertura_cero(n);
            let mask_buf = mascara.unwrap_or_else(|| self.buffer_dummy());
            let clip_buf = match (usa_clip, &base_alpha) {
                (true, Some(b)) => b.clone(),
                _ => self.buffer_dummy(),
            };

            let es_disolver = matches!(capa.blend, ModoFusion::Disolver);
            let params = Params {
                modo: modo_codigo(capa.blend),
                has_mask: if mask_buf.size() > 4 { 1 } else { 0 },
                has_clip: if usa_clip { 1 } else { 0 },
                n: n as u32,
                opacidad: capa.opacidad.clamp(0.0, 1.0),
                stride: 0, // lo completa despachar()
                seed: if es_disolver { semilla_dissolve(capa) } else { 0 },
            };
            let pipeline = if es_disolver {
                &self.disolver_pipeline
            } else {
                &self.pipeline
            };

            self.despachar(
                encoder, pipeline, &acc, &src, &mask_buf, &clip_buf, &cobertura, params, n, keep,
            );

            if !capa.clipping {
                base_alpha = Some(cobertura.clone());
            }

            // Todo esto debe vivir hasta el submit.
            keep.buffers.push(src);
            keep.buffers.push(mask_buf);
            keep.buffers.push(clip_buf);
            keep.buffers.push(cobertura);
        }

        keep.buffers.push(acc.clone());
        Ok(acc)
    }

    /// Resuelve y valida la máscara de una capa. Devuelve un buffer storage con
    /// los bytes de máscara (padded a múltiplo de 4) o `None` si no tiene.
    fn cargar_mascara(
        &self,
        capa: &Capa,
        n: usize,
        fuente: &impl FuenteBuffers,
    ) -> Result<Option<wgpu::Buffer>, Error> {
        let Some(hm) = capa.mascara else {
            return Ok(None);
        };
        let bytes = fuente.obtener(hm).ok_or(Error::BufferFaltante(hm))?;
        if bytes.len() != n {
            return Err(Error::TamanioMascara {
                hash: hm,
                esperado: n,
                encontrado: bytes.len(),
            });
        }
        // Padear a múltiplo de 4 bytes (el shader lee `array<u32>`).
        let mut padded = bytes.to_vec();
        while padded.len() % 4 != 0 {
            padded.push(0);
        }
        Ok(Some(self.buffer_storage(&padded, "tullpu-mask")))
    }

    /// Graba un dispatch del shader de fusión. Calcula la grilla 2D que cubre
    /// `n` hilos sin exceder `max_grupos_dim` por dimensión.
    #[allow(clippy::too_many_arguments)]
    fn despachar(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        pipeline: &wgpu::ComputePipeline,
        acc: &wgpu::Buffer,
        src: &wgpu::Buffer,
        mask: &wgpu::Buffer,
        clip: &wgpu::Buffer,
        cobertura: &wgpu::Buffer,
        mut params: Params,
        n: usize,
        keep: &mut KeepAlive,
    ) {
        const WG: u32 = 64;
        let total_grupos = ((n as u32) + WG - 1) / WG;
        let gx = total_grupos.min(self.max_grupos_dim).max(1);
        let gy = (total_grupos + gx - 1) / gx;
        params.stride = gx * WG;

        let params_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tullpu-params"),
                contents: &params.bytes(),
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tullpu-blend-bg"),
            layout: &self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: acc.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: src.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: mask.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: clip.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: cobertura.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 5, resource: params_buf.as_entire_binding() },
            ],
        });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("tullpu-blend-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(gx, gy, 1);
        }

        // El uniform de params y el bind group deben vivir hasta el submit.
        keep.buffers.push(params_buf);
        keep.bind_groups.push(bind);
    }

    /// Graba un dispatch del shader de ajuste sobre `acc` in-place. Clasifica la
    /// op: las independientes por canal se compilan a una LUT de 256 (con el
    /// código exacto de la CPU); Saturacion/Tonalidad van por la rama HSL del
    /// shader; las espaciales/alfa (`Blur`/`Espejar`/`Opacidad`) no son ajustes
    /// de composición y se omiten (la CPU también las ignora acá).
    #[allow(clippy::too_many_arguments)]
    fn despachar_ajuste(
        &self,
        encoder: &mut wgpu::CommandEncoder,
        acc: &wgpu::Buffer,
        op: &OpLocal,
        opacidad: f32,
        mask: &wgpu::Buffer,
        clip: &wgpu::Buffer,
        usa_clip: bool,
        n: usize,
        keep: &mut KeepAlive,
    ) {
        // op_kind/param/lut según la clase de op.
        let (op_kind, param, lut_buf) = match clasificar_ajuste(op) {
            AjusteClase::Lut(lut) => {
                // Empaquetar 4 entradas/word (256 bytes), como la máscara.
                (0u32, 0.0f32, self.buffer_storage(&lut, "tullpu-lut"))
            }
            AjusteClase::Saturacion(f) => (1u32, f, self.buffer_dummy()),
            AjusteClase::Tonalidad(d) => (2u32, d, self.buffer_dummy()),
            AjusteClase::NoOp => return, // Blur/Espejar/Opacidad: no es ajuste de composición.
        };

        const WG: u32 = 64;
        let total_grupos = ((n as u32) + WG - 1) / WG;
        let gx = total_grupos.min(self.max_grupos_dim).max(1);
        let gy = (total_grupos + gx - 1) / gx;
        let stride = gx * WG;

        // Uniform AjusteParams (32 bytes).
        let mut b = [0u8; 32];
        b[0..4].copy_from_slice(&op_kind.to_le_bytes());
        b[4..8].copy_from_slice(&(if mask.size() > 4 { 1u32 } else { 0 }).to_le_bytes());
        b[8..12].copy_from_slice(&(if usa_clip { 1u32 } else { 0 }).to_le_bytes());
        b[12..16].copy_from_slice(&(n as u32).to_le_bytes());
        b[16..20].copy_from_slice(&opacidad.clamp(0.0, 1.0).to_le_bytes());
        b[20..24].copy_from_slice(&param.to_le_bytes());
        b[24..28].copy_from_slice(&stride.to_le_bytes());
        let params_buf = self
            .device
            .create_buffer_init(&wgpu::util::BufferInitDescriptor {
                label: Some("tullpu-ajuste-params"),
                contents: &b,
                usage: wgpu::BufferUsages::UNIFORM,
            });

        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("tullpu-ajuste-bg"),
            layout: &self.ajuste_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: acc.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 1, resource: lut_buf.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 2, resource: mask.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: clip.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 4, resource: params_buf.as_entire_binding() },
            ],
        });

        {
            let mut pass = encoder.begin_compute_pass(&wgpu::ComputePassDescriptor {
                label: Some("tullpu-ajuste-pass"),
                timestamp_writes: None,
            });
            pass.set_pipeline(&self.ajuste_pipeline);
            pass.set_bind_group(0, &bind, &[]);
            pass.dispatch_workgroups(gx, gy, 1);
        }

        keep.buffers.push(lut_buf);
        keep.buffers.push(params_buf);
        keep.bind_groups.push(bind);
    }

    fn buffer_acc_cero(&self, n: usize) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tullpu-acc"),
            contents: &vec![0u8; n * 4],
            usage: wgpu::BufferUsages::STORAGE | wgpu::BufferUsages::COPY_SRC,
        })
    }

    fn buffer_cobertura_cero(&self, n: usize) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tullpu-cobertura"),
            contents: &vec![0u8; n * 4],
            usage: wgpu::BufferUsages::STORAGE,
        })
    }

    fn buffer_storage(&self, bytes: &[u8], label: &str) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(label),
            contents: bytes,
            usage: wgpu::BufferUsages::STORAGE,
        })
    }

    /// Buffer de 1 elemento para bindings opcionales inactivos (mask/clip).
    fn buffer_dummy(&self) -> wgpu::Buffer {
        self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("tullpu-dummy"),
            contents: &[0u8; 4],
            usage: wgpu::BufferUsages::STORAGE,
        })
    }
}

/// Recursos GPU que deben sobrevivir hasta el `queue.submit`. La recursión
/// los acumula y se sueltan todos juntos al final de `componer`.
#[derive(Default)]
struct KeepAlive {
    buffers: Vec<wgpu::Buffer>,
    bind_groups: Vec<wgpu::BindGroup>,
}

fn bgl_storage(binding: u32, read_only: bool) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::COMPUTE,
        ty: wgpu::BindingType::Buffer {
            ty: wgpu::BufferBindingType::Storage { read_only },
            has_dynamic_offset: false,
            min_binding_size: None,
        },
        count: None,
    }
}

/// Semilla del RNG de Disolver: los primeros 8 bytes del Uuid de la capa, en
/// little-endian. Espejo de `tullpu_render::semilla_dissolve` — estable a través
/// de regeneraciones (lo garantiza el Uuid).
fn semilla_dissolve(capa: &Capa) -> u64 {
    let b = capa.id.as_bytes();
    u64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]])
}

/// Clase de ejecución de una capa de ajuste en la GPU.
enum AjusteClase {
    /// Op independiente por canal compilada a una LUT de 256 (Invertir, Brillo,
    /// Contraste, Niveles, Curvas). La construye el código exacto de la CPU.
    Lut([u8; 256]),
    /// Saturacion HSL: `s' = clamp(s · factor)`.
    Saturacion(f32),
    /// Tonalidad HSL: `h' = rem_euclid(h + grados/360)`.
    Tonalidad(f32),
    /// Op espacial/alfa (Blur/Espejar/Opacidad) — no es ajuste de composición.
    NoOp,
}

/// Clasifica una [`OpLocal`] para el shader de ajuste. Para las ops
/// independientes por canal construye la LUT corriendo `ajustar_rgb_inplace`
/// sobre una rampa de gris 0..255 — reusa la math exacta de la CPU, así esos
/// ajustes son bit-idénticos salvo el redondeo final del blend.
fn clasificar_ajuste(op: &OpLocal) -> AjusteClase {
    match op {
        OpLocal::Saturacion { factor } => AjusteClase::Saturacion(*factor),
        OpLocal::Tonalidad { grados } => AjusteClase::Tonalidad(grados / 360.0),
        OpLocal::Blur { .. }
        | OpLocal::EspejarHorizontal
        | OpLocal::EspejarVertical
        | OpLocal::Opacidad { .. } => AjusteClase::NoOp,
        // Invertir / Brillo / Contraste / Niveles / Curvas: canal-independiente.
        _ => {
            let mut rampa = Vec::with_capacity(256 * 4);
            for i in 0..256u32 {
                rampa.extend_from_slice(&[i as u8, i as u8, i as u8, 255]);
            }
            let ok = pixel::ajustar_rgb_inplace(op, &mut rampa);
            debug_assert!(ok, "op canal-independiente debería devolver true");
            let mut lut = [0u8; 256];
            for (i, slot) in lut.iter_mut().enumerate() {
                *slot = rampa[i * 4];
            }
            AjusteClase::Lut(lut)
        }
    }
}
