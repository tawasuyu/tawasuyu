//! Renderer WebGL2 que compone geometría + física + paleta + shaders en pantalla.
//!
//! Es agnóstico del DOM: el caller monta el `<canvas>`, le pasa eventos
//! de mouse y llama `render(time_ms)` desde un `requestAnimationFrame`.
//!
//! ```ignore
//! let mut r = Renderer::new(&canvas)?;
//! r.resize(w, h);
//! r.set_mouse_px(dx, dy);
//! r.render(time_ms);
//! ```

use gioser_geom::ChacanaSpec;
use gioser_palette::{cosmos, Rgb};
use gioser_physics::SpringDamper2;
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
/// Inclinación máxima en cada eje. 28° = movimiento bien legible pero
/// no caricaturesco; la chacana se siente "pesada y noble".
const MAX_TILT_DEG: f32 = 28.0;
/// Escala mundo→viewport: con arm_extent=0.65 + aro a 1.45×, la chacana
/// + aro entran cómodos con margen para botones DOM más allá del aro.
const WORLD_SCALE: f32 = 1.05;

/// Identidad de cada cardinal (id, color de acento, label visible).
/// Orden `[N, E, S, W]` coincide con `ChacanaSpec::tips()`.
pub mod tips {
    use gioser_palette::{elements, Rgb};
    pub const ORDER: [(&str, Rgb, &str); 4] = [
        ("aire", elements::AIRE, "AIRE"),       // N
        ("fuego", elements::FUEGO, "FUEGO"),    // E
        ("tierra", elements::TIERRA, "TIERRA"), // S
        ("agua", elements::AGUA, "AGUA"),       // W
    ];
}

pub struct Renderer {
    gl: GL,
    cosmos_prog: Program,
    chacana_prog: Program,
    cosmos_vao: WebGlVertexArrayObject,
    chacana_vao: WebGlVertexArrayObject,
    chacana_quad_count: i32,
    chacana: ChacanaSpec,
    tilt: SpringDamper2,
    sun_pulse: f32,
    last_time_ms: f64,
    viewport: (u32, u32),
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
                "u_sun_pulse",
            ],
        )
        .map_err(JsValue::from)?;

        let (cosmos_vao, _) = upload_quad(&gl, &FULLSCREEN_QUAD, 0).map_err(JsValue::from)?;
        let chacana_quad_verts = chacana_quad(chacana.arm_extent());
        let (chacana_vao, chacana_quad_count) =
            upload_quad(&gl, &chacana_quad_verts, 0).map_err(JsValue::from)?;

        let tilt = SpringDamper2::new(1.7, 0.65);

        Ok(Self {
            gl,
            cosmos_prog,
            chacana_prog,
            cosmos_vao,
            chacana_vao,
            chacana_quad_count,
            chacana,
            tilt,
            sun_pulse: 0.0,
            last_time_ms: 0.0,
            viewport: (canvas.width().max(1), canvas.height().max(1)),
            mouse: (0.0, 0.0),
        })
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        self.viewport = (w.max(1), h.max(1));
        self.gl
            .viewport(0, 0, self.viewport.0 as i32, self.viewport.1 as i32);
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

    /// Posición proyectada NDC de cada tip cardinal `[N, E, S, W]`.
    pub fn tips_ndc(&self) -> [(f32, f32); 4] {
        self.points_ndc(&self.chacana.tips())
    }

    /// Posición NDC de un punto en cualquier radio cardinal (factor sobre
    /// `arm_extent`). Útil para anclar los botones DOM más allá de la chacana
    /// pero dentro del aro.
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

    /// Devuelve `(pitch_deg, yaw_deg)` actuales del spring de tilt.
    /// El caller los inyecta como CSS vars en el contenedor del título para
    /// que el HTML se tumbe junto con la chacana renderizada en GL.
    pub fn tilt_degrees(&self) -> (f32, f32) {
        (self.tilt.position[0] * DEG, self.tilt.position[1] * DEG)
    }

    fn build_mvp(&self) -> Mat4 {
        let (w, h) = self.viewport;
        let aspect = w as f32 / h as f32;
        let proj = Mat4::perspective_rh(45.0_f32.to_radians(), aspect, 0.1, 20.0);
        let view = Mat4::look_at_rh(Vec3::new(0.0, 0.0, 2.6), Vec3::ZERO, Vec3::Y);
        let pitch = Mat4::from_rotation_x(self.tilt.position[0]);
        let yaw = Mat4::from_rotation_y(self.tilt.position[1]);
        let scale = Mat4::from_scale(Vec3::splat(WORLD_SCALE));
        proj * view * yaw * pitch * scale
    }

    pub fn render(&mut self, time_ms: f64) {
        let dt = if self.last_time_ms == 0.0 {
            1.0 / 60.0
        } else {
            ((time_ms - self.last_time_ms) as f32 / 1000.0).clamp(0.0, 1.0 / 15.0)
        };
        self.last_time_ms = time_ms;
        let sub = 4;
        let sub_dt = dt / sub as f32;
        for _ in 0..sub {
            self.tilt.step(sub_dt);
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

        // Chacana (blend aditivo para que dorado y sol sumen luz al cosmos)
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
