//! Renderer WebGL2 que compone geometría + física + paleta + shaders en pantalla.
//!
//! El loop externo (típicamente `requestAnimationFrame`) llama `render(time_ms)`.
//! Los eventos input se propagan vía métodos: `set_mouse_px`, `release_tilt`,
//! `impulse_click`. El cliente puede consultar dimensiones derivadas
//! (`click_radius_css_px`, `tilt_degrees`, `cardinal_positions_ndc`) para
//! sincronizar DOM (botones, título, taskbar).

use gioser_geom::ChacanaSpec;
use gioser_palette::{cosmos, Rgb};
use gioser_physics::{SpringDamper1, SpringDamper2};
use gioser_shaders::{
    chacana_quad, FS_CHACANA, FS_COSMOS, FULLSCREEN_QUAD, VS_CHACANA, VS_FULLSCREEN,
};
use glam::{Mat4, Vec3, Vec4};
use std::collections::HashMap;
use wasm_bindgen::{JsCast, JsValue};
use web_sys::{
    HtmlCanvasElement, WebGl2RenderingContext as GL, WebGlProgram, WebGlShader,
    WebGlUniformLocation, WebGlVertexArrayObject,
};

const RAD: f32 = core::f32::consts::PI / 180.0;
const DEG: f32 = 180.0 / core::f32::consts::PI;
/// Inclinación máxima en cada eje.
const MAX_TILT_DEG: f32 = 28.0;
/// `cot(45°/2)` — factor de proyección. Lo necesitamos también para calcular
/// el radio del círculo en pixels (hit-test del click).
const COT_HALF_FOV: f32 = 2.414_213_5;
/// Distancia del aro principal respecto al centro de la chacana — sincronizar
/// con `FS_CHACANA::ringR_main` del shader.
const RING_FACTOR: f32 = 1.45;

/// Identidad de cada cardinal (id, color de acento, label). Orden `[N, E, S, W]`.
pub mod tips {
    use gioser_palette::{elements, Rgb};
    pub const ORDER: [(&str, Rgb, &str); 4] = [
        ("aire", elements::AIRE, "AIRE"),
        ("fuego", elements::FUEGO, "FUEGO"),
        ("tierra", elements::TIERRA, "TIERRA"),
        ("agua", elements::AGUA, "AGUA"),
    ];
}

/// Colores zodiacales en orden Aries→Piscis. Sigue la asignación tradicional
/// por triplicidad elemental:
///   fuego: aries, leo, sagitario     (rojo, dorado, púrpura)
///   tierra: tauro, virgo, capricornio (verde, marrón, verde oscuro)
///   aire: géminis, libra, acuario     (amarillo, rosa, celeste)
///   agua: cáncer, escorpio, piscis    (plata, rojo profundo, verde mar)
///
/// El shader los recibe como `uniform vec3 u_zodiac[12]` y los dibuja como
/// trazos radiales muy sutiles entre la chacana y el aro exterior.
pub const ZODIAC_COLORS: [[f32; 3]; 12] = [
    [0.95, 0.30, 0.20], // 0 Aries — fuego rojo
    [0.35, 0.65, 0.30], // 1 Tauro — tierra verde
    [0.95, 0.85, 0.30], // 2 Géminis — aire amarillo
    [0.80, 0.88, 0.95], // 3 Cáncer — agua plata
    [0.98, 0.65, 0.20], // 4 Leo — fuego dorado
    [0.62, 0.50, 0.32], // 5 Virgo — tierra marrón
    [0.95, 0.65, 0.82], // 6 Libra — aire rosa
    [0.55, 0.15, 0.22], // 7 Escorpio — agua rojo profundo
    [0.60, 0.30, 0.85], // 8 Sagitario — fuego púrpura
    [0.22, 0.45, 0.28], // 9 Capricornio — tierra verde oscuro
    [0.48, 0.78, 0.95], // 10 Acuario — aire celeste
    [0.22, 0.72, 0.62], // 11 Piscis — agua verde mar
];

pub struct Renderer {
    gl: GL,
    cosmos_prog: Program,
    chacana_prog: Program,
    cosmos_vao: WebGlVertexArrayObject,
    chacana_vao: WebGlVertexArrayObject,
    chacana_quad_count: i32,
    chacana: ChacanaSpec,
    /// Spring del tilt 3D que sigue al mouse. Sub-crítico orgánico.
    tilt: SpringDamper2,
    /// Spring de "vibración" tras click: rotación Z bien underdamped que
    /// decae naturalmente. Independiente del tilt.
    shake: SpringDamper1,
    /// Contador para alternar sentido del shake en clicks sucesivos.
    click_count: u32,
    sun_pulse: f32,
    last_time_ms: f64,
    /// Dimensiones device-pixel del canvas (lo que GL viewport usa).
    viewport: (u32, u32),
    /// Dimensiones CSS-pixel del canvas (lo que ven los eventos DOM).
    client_size: (f32, f32),
    /// Mouse en clip-space, x ∈ [-aspect, aspect], y ∈ [-1, 1].
    mouse: (f32, f32),
}

struct Program {
    program: WebGlProgram,
    uniforms: HashMap<&'static str, WebGlUniformLocation>,
}

impl Program {
    fn new(gl: &GL, vs: &str, fs: &str, names: &[&'static str]) -> Result<Self, String> {
        let vs = compile_shader(gl, GL::VERTEX_SHADER, vs)?;
        let fs = compile_shader(gl, GL::FRAGMENT_SHADER, fs)?;
        let program = gl.create_program().ok_or("create_program failed")?;
        gl.attach_shader(&program, &vs);
        gl.attach_shader(&program, &fs);
        gl.bind_attrib_location(&program, 0, "a_pos");
        gl.link_program(&program);
        let linked = gl
            .get_program_parameter(&program, GL::LINK_STATUS)
            .as_bool()
            .unwrap_or(false);
        if !linked {
            return Err(gl
                .get_program_info_log(&program)
                .unwrap_or_else(|| "link failed".into()));
        }
        let mut uniforms = HashMap::new();
        for n in names {
            if let Some(loc) = gl.get_uniform_location(&program, n) {
                uniforms.insert(*n, loc);
            }
        }
        Ok(Self { program, uniforms })
    }

    fn u(&self, name: &'static str) -> Option<&WebGlUniformLocation> {
        self.uniforms.get(name)
    }
}

fn compile_shader(gl: &GL, ty: u32, src: &str) -> Result<WebGlShader, String> {
    let s = gl.create_shader(ty).ok_or("create_shader failed")?;
    gl.shader_source(&s, src);
    gl.compile_shader(&s);
    let ok = gl
        .get_shader_parameter(&s, GL::COMPILE_STATUS)
        .as_bool()
        .unwrap_or(false);
    if !ok {
        return Err(gl
            .get_shader_info_log(&s)
            .unwrap_or_else(|| "compile failed".into()));
    }
    Ok(s)
}

fn upload_quad(
    gl: &GL,
    verts: &[f32],
    attr_loc: u32,
) -> Result<(WebGlVertexArrayObject, i32), String> {
    let vao = gl
        .create_vertex_array()
        .ok_or("create_vertex_array failed")?;
    gl.bind_vertex_array(Some(&vao));
    let buf = gl.create_buffer().ok_or("create_buffer failed")?;
    gl.bind_buffer(GL::ARRAY_BUFFER, Some(&buf));
    // SAFETY: `Float32Array::view` apunta a memoria WASM lineal; no la
    // movemos durante este scope (no hay allocs intermedias).
    unsafe {
        let view = js_sys::Float32Array::view(verts);
        gl.buffer_data_with_array_buffer_view(GL::ARRAY_BUFFER, &view, GL::STATIC_DRAW);
    }
    gl.vertex_attrib_pointer_with_i32(attr_loc, 2, GL::FLOAT, false, 0, 0);
    gl.enable_vertex_attrib_array(attr_loc);
    gl.bind_vertex_array(None);
    Ok((vao, (verts.len() / 2) as i32))
}

/// Devuelve el factor de escala mundo→viewport en función del aspect.
/// Para portrait (aspect < 1), achicamos proporcionalmente para que la
/// circunferencia exterior no se corte por los lados.
fn world_scale_for_aspect(aspect: f32) -> f32 {
    let base = 1.05;
    if aspect >= 1.0 {
        base
    } else {
        // En portrait, el extent visible horizontal se reduce con `aspect`.
        // Bajamos la escala para mantener el aro entero dentro del viewport,
        // con piso 0.45 para que no quede ridículamente pequeña.
        (base * aspect.max(0.45)).min(base)
    }
}

impl Renderer {
    pub fn new(canvas: &HtmlCanvasElement) -> Result<Self, JsValue> {
        let gl = canvas
            .get_context("webgl2")?
            .ok_or_else(|| JsValue::from_str("WebGL2 no soportado"))?
            .dyn_into::<GL>()?;

        let chacana = ChacanaSpec::CLASSIC;

        let cosmos_prog = Program::new(
            &gl,
            VS_FULLSCREEN,
            FS_COSMOS,
            &[
                "u_resolution",
                "u_time",
                "u_parallax",
                "u_void",
                "u_nebula_a",
                "u_nebula_b",
                "u_stardust",
            ],
        )
        .map_err(JsValue::from)?;

        let chacana_prog = Program::new(
            &gl,
            VS_CHACANA,
            FS_CHACANA,
            &[
                "u_mvp",
                "u_time",
                "u_thickness",
                "u_center_half",
                "u_arm_extent",
                "u_line_color",
                "u_rim_color",
                "u_sun_color",
                "u_dark_color",
                "u_aire_color",
                "u_fuego_color",
                "u_tierra_color",
                "u_agua_color",
                "u_zodiac[0]",
                "u_sun_pulse",
            ],
        )
        .map_err(JsValue::from)?;

        let (cosmos_vao, _) = upload_quad(&gl, &FULLSCREEN_QUAD, 0).map_err(JsValue::from)?;
        let chacana_quad_verts = chacana_quad(chacana.arm_extent());
        let (chacana_vao, chacana_quad_count) =
            upload_quad(&gl, &chacana_quad_verts, 0).map_err(JsValue::from)?;

        let tilt = SpringDamper2::new(1.7, 0.65);
        // Shake: alta frecuencia, muy underdamped → vibración fuerte que
        // muere en ~0.8 s con varios ciclos visibles.
        let shake = SpringDamper1::new(7.5, 0.13);

        Ok(Self {
            gl,
            cosmos_prog,
            chacana_prog,
            cosmos_vao,
            chacana_vao,
            chacana_quad_count,
            chacana,
            tilt,
            shake,
            click_count: 0,
            sun_pulse: 0.0,
            last_time_ms: 0.0,
            viewport: (canvas.width().max(1), canvas.height().max(1)),
            client_size: (
                canvas.client_width().max(1) as f32,
                canvas.client_height().max(1) as f32,
            ),
            mouse: (0.0, 0.0),
        })
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.viewport = (w.max(1), h.max(1));
        self.gl
            .viewport(0, 0, self.viewport.0 as i32, self.viewport.1 as i32);
    }

    /// Tamaño en CSS pixels (independiente del DPR). Lo usa el hit-test del
    /// click para que coincida con coordenadas DOM.
    pub fn set_client_size(&mut self, w: f32, h: f32) {
        self.client_size = (w.max(1.0), h.max(1.0));
    }

    pub fn set_mouse_px(&mut self, x: f32, y: f32) {
        let (w, h) = self.viewport;
        if h == 0 {
            return;
        }
        let aspect = w as f32 / h as f32;
        let half_h = h as f32 * 0.5;
        let mx = (x / half_h).clamp(-aspect, aspect);
        let my = (y / half_h).clamp(-1.0, 1.0);
        self.mouse = (mx, my);
        let max_tilt = MAX_TILT_DEG * RAD;
        let target = [my * max_tilt, -mx * max_tilt / aspect];
        self.tilt.set_target(target);
    }

    /// Mouse fuera del canvas — la chacana vuelve al frente con rebote
    /// natural del spring sub-crítico.
    pub fn release_tilt(&mut self) {
        self.tilt.set_target([0.0, 0.0]);
        // mouse parallax (fondo) también vuelve al centro
        self.mouse = (0.0, 0.0);
    }

    /// Inyecta un impulso al spring shake — la chacana vibra fuerte y decae.
    /// Llamar en respuesta a un click/tap dentro del aro.
    pub fn impulse_click(&mut self) {
        self.click_count = self.click_count.wrapping_add(1);
        let dir = if self.click_count % 2 == 0 { 1.0 } else { -1.0 };
        // Magnitud del impulso en rad/s. Con ω≈47, esto produce un pico
        // de ~5-7° en la rotación Z, decayendo en ~0.8 s.
        self.shake.velocity[0] += 6.5 * dir;
    }

    /// Radio del aro exterior, en CSS pixels desde el centro del canvas.
    /// El cliente lo usa para decidir si un click cae dentro del círculo.
    pub fn click_radius_css_px(&self) -> f32 {
        let (w, _h) = self.viewport;
        let aspect = w as f32 / self.viewport.1.max(1) as f32;
        let scale = world_scale_for_aspect(aspect);
        let ring_ndc = self.chacana.arm_extent() * RING_FACTOR * scale * COT_HALF_FOV / 2.6;
        ring_ndc * self.client_size.1 / 2.0
    }

    /// Posición proyectada NDC de cada tip cardinal `[N, E, S, W]`.
    pub fn tips_ndc(&self) -> [(f32, f32); 4] {
        self.points_ndc(&self.chacana.tips())
    }

    /// Posiciones NDC para anclar botones en los 4 cardinales a un radio
    /// específico (factor sobre `arm_extent`).
    pub fn cardinal_positions_ndc(&self, radius_factor: f32) -> [(f32, f32); 4] {
        let r = self.chacana.arm_extent() * radius_factor;
        self.points_ndc(&[(0.0, r), (r, 0.0), (0.0, -r), (-r, 0.0)])
    }

    fn points_ndc(&self, pts: &[(f32, f32); 4]) -> [(f32, f32); 4] {
        let mvp = self.build_mvp();
        let mut out = [(0.0_f32, 0.0_f32); 4];
        for (i, t) in pts.iter().enumerate() {
            let p = mvp * Vec4::new(t.0, t.1, 0.0, 1.0);
            let w = if p.w == 0.0 { 1.0 } else { p.w };
            out[i] = (p.x / w, p.y / w);
        }
        out
    }

    pub fn chacana(&self) -> &ChacanaSpec {
        &self.chacana
    }

    pub fn mouse_clip(&self) -> (f32, f32) {
        self.mouse
    }

    /// `(pitch_deg, yaw_deg, roll_deg)` actuales. Roll viene del shake spring.
    pub fn tilt_degrees(&self) -> (f32, f32, f32) {
        (
            self.tilt.position[0] * DEG,
            self.tilt.position[1] * DEG,
            self.shake.position[0] * DEG,
        )
    }

    fn build_mvp(&self) -> Mat4 {
        let (w, h) = self.viewport;
        let aspect = w as f32 / h as f32;
        let scale_val = world_scale_for_aspect(aspect);
        let proj = Mat4::perspective_rh(45.0_f32.to_radians(), aspect, 0.1, 20.0);
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 2.6), Vec3::ZERO, Vec3::Y);
        let pitch = Mat4::from_rotation_x(self.tilt.position[0]);
        let yaw = Mat4::from_rotation_y(self.tilt.position[1]);
        let roll = Mat4::from_rotation_z(self.shake.position[0]);
        let scale = Mat4::from_scale(Vec3::splat(scale_val));
        proj * view * yaw * pitch * roll * scale
    }

    pub fn render(&mut self, time_ms: f64) {
        let dt = if self.last_time_ms == 0.0 {
            1.0 / 60.0
        } else {
            ((time_ms - self.last_time_ms) as f32 / 1000.0).clamp(0.0, 1.0 / 15.0)
        };
        self.last_time_ms = time_ms;

        // Subdividir físico — el shake corre a alta frecuencia y necesita
        // dt < 1/freq para mantenerse estable (1/7.5 ≈ 133 ms; 8 sub-pasos a
        // 60fps dejan 2 ms por sub-paso).
        let sub = 8;
        let sub_dt = dt / sub as f32;
        for _ in 0..sub {
            self.tilt.step(sub_dt);
            self.shake.step(sub_dt);
        }

        let t = time_ms as f32 * 0.001;
        self.sun_pulse = 0.5 + 0.5 * (t * 1.4).sin();

        let gl = &self.gl;
        gl.viewport(0, 0, self.viewport.0 as i32, self.viewport.1 as i32);
        gl.disable(GL::DEPTH_TEST);
        gl.enable(GL::BLEND);
        gl.blend_func(GL::SRC_ALPHA, GL::ONE_MINUS_SRC_ALPHA);
        gl.clear_color(0.02, 0.015, 0.04, 1.0);
        gl.clear(GL::COLOR_BUFFER_BIT);

        // Cosmos
        gl.use_program(Some(&self.cosmos_prog.program));
        if let Some(u) = self.cosmos_prog.u("u_resolution") {
            gl.uniform2f(Some(u), self.viewport.0 as f32, self.viewport.1 as f32);
        }
        if let Some(u) = self.cosmos_prog.u("u_time") {
            gl.uniform1f(Some(u), t);
        }
        if let Some(u) = self.cosmos_prog.u("u_parallax") {
            gl.uniform2f(Some(u), self.mouse.0, self.mouse.1);
        }
        upload_rgb(gl, self.cosmos_prog.u("u_void"), cosmos::VOID);
        upload_rgb(gl, self.cosmos_prog.u("u_nebula_a"), cosmos::NEBULA_A);
        upload_rgb(gl, self.cosmos_prog.u("u_nebula_b"), cosmos::NEBULA_B);
        upload_rgb(gl, self.cosmos_prog.u("u_stardust"), cosmos::STARDUST);
        gl.bind_vertex_array(Some(&self.cosmos_vao));
        gl.draw_arrays(GL::TRIANGLES, 0, 6);

        // Chacana (blend aditivo)
        gl.blend_func(GL::SRC_ALPHA, GL::ONE);
        gl.use_program(Some(&self.chacana_prog.program));
        let mvp = self.build_mvp();
        if let Some(u) = self.chacana_prog.u("u_mvp") {
            gl.uniform_matrix4fv_with_f32_array(Some(u), false, &mvp.to_cols_array());
        }
        if let Some(u) = self.chacana_prog.u("u_time") {
            gl.uniform1f(Some(u), t);
        }
        if let Some(u) = self.chacana_prog.u("u_thickness") {
            gl.uniform1f(Some(u), self.chacana.thickness);
        }
        if let Some(u) = self.chacana_prog.u("u_center_half") {
            gl.uniform1f(Some(u), self.chacana.center_half());
        }
        if let Some(u) = self.chacana_prog.u("u_arm_extent") {
            gl.uniform1f(Some(u), self.chacana.arm_extent());
        }
        upload_rgb(gl, self.chacana_prog.u("u_line_color"), cosmos::CHACANA_LINE);
        upload_rgb(gl, self.chacana_prog.u("u_rim_color"), cosmos::CHACANA_RIM);
        upload_rgb(gl, self.chacana_prog.u("u_sun_color"), cosmos::SUN_CORE);
        upload_rgb(gl, self.chacana_prog.u("u_dark_color"), cosmos::CHACANA_DARK);
        upload_rgb(
            gl,
            self.chacana_prog.u("u_aire_color"),
            gioser_palette::elements::AIRE,
        );
        upload_rgb(
            gl,
            self.chacana_prog.u("u_fuego_color"),
            gioser_palette::elements::FUEGO,
        );
        upload_rgb(
            gl,
            self.chacana_prog.u("u_tierra_color"),
            gioser_palette::elements::TIERRA,
        );
        upload_rgb(
            gl,
            self.chacana_prog.u("u_agua_color"),
            gioser_palette::elements::AGUA,
        );
        // Subir las 12 colores zodiacales como vec3[12]. Aplanamos a un único
        // slice de 36 floats; uniform3fv interpreta cada terna como vec3.
        if let Some(u) = self.chacana_prog.u("u_zodiac[0]") {
            let mut flat = [0.0f32; 36];
            for (i, c) in ZODIAC_COLORS.iter().enumerate() {
                flat[i * 3] = c[0];
                flat[i * 3 + 1] = c[1];
                flat[i * 3 + 2] = c[2];
            }
            gl.uniform3fv_with_f32_array(Some(u), &flat);
        }
        if let Some(u) = self.chacana_prog.u("u_sun_pulse") {
            gl.uniform1f(Some(u), self.sun_pulse);
        }
        gl.bind_vertex_array(Some(&self.chacana_vao));
        gl.draw_arrays(GL::TRIANGLES, 0, self.chacana_quad_count);
        gl.bind_vertex_array(None);
    }
}

fn upload_rgb(gl: &GL, loc: Option<&WebGlUniformLocation>, c: Rgb) {
    if let Some(u) = loc {
        gl.uniform3f(Some(u), c.0, c.1, c.2);
    }
}
