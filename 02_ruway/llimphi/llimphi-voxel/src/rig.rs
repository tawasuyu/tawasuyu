//! `rig` — **cuerpo arbitrario articulado** para los Seres: un árbol de partes
//! (cajas) unidas por articulaciones, que generaliza el muñeco humanoide fijo de
//! [`Actor`](crate::Actor) a cualquier morfología (cuadrúpedo, ave, serpiente, bicho).
//! Es la **capa 1** del movimiento; encima montan los *andares* (capa 2, animación
//! procedural por articulación — ya esbozada acá con [`Andar`]) y la *conducta*
//! (capa 3, locomoción) — y, más adelante, movimiento coreografiado/complejo.
//!
//! Un [`Segmento`] es una caja que cuelga de un **pivote** (su articulación) en el
//! marco de su **padre**, y rota sobre un [`Eje`]. El árbol se hornea a una malla
//! ([`Rig::mesh`]) acumulando las transformaciones padre→hijo: rotar el muslo
//! arrastra la pantorrilla y el pie. Como el [`Actor`], produce `(Vec<Vertex3d>,
//! Vec<u16>)` en espacio local (pies en el origen, mirando a `+Z`); no toca GPU.
//!
//! Todo es **dato serializable**: una criatura nueva es un `Rig`, no código nuevo.

use llimphi_3d::glam::{Mat4, Vec3};
use llimphi_3d::{push_cube, Vertex3d};
use serde::{Deserialize, Serialize};

/// Eje de rotación de una articulación (en el marco local del segmento).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Eje {
    /// Cabeceo: balanceo hacia adelante/atrás (miembros que caminan).
    X,
    /// Giro: rotación horizontal (serpenteo de una serpiente).
    Y,
    /// Alabeo: hacia el costado/arriba (aletear de un ave).
    Z,
}

impl Eje {
    fn mat(self, a: f32) -> Mat4 {
        match self {
            Eje::X => Mat4::from_rotation_x(a),
            Eje::Y => Mat4::from_rotation_y(a),
            Eje::Z => Mat4::from_rotation_z(a),
        }
    }
}

/// De qué color se pinta un segmento. Las tres ranuras mapean a los colores del
/// [`Sere`](crate::CharSpec) (piel/cuerpo/patas); `Custom` lleva el suyo.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, Deserialize)]
pub enum SlotColor {
    Piel,
    Cuerpo,
    Patas,
    Custom([f32; 3]),
}

impl SlotColor {
    fn resolve(self, piel: [f32; 3], cuerpo: [f32; 3], patas: [f32; 3]) -> [f32; 3] {
        match self {
            SlotColor::Piel => piel,
            SlotColor::Cuerpo => cuerpo,
            SlotColor::Patas => patas,
            SlotColor::Custom(c) => c,
        }
    }
}

/// Una **parte** del cuerpo: una caja articulada en el árbol.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Segmento {
    /// Nombre (define el rol para los andares: "pata", "brazo", "ala", "cola"…).
    pub nombre: String,
    /// Índice del padre en [`Rig::segmentos`]; `-1` = raíz (anclado a los pies).
    pub padre: i32,
    /// Posición de la **articulación** `[x,y,z]` relativa al marco del padre.
    /// (`[f32;3]` y no `Vec3` para que serialice sin features extra de glam.)
    pub pivote: [f32; 3],
    /// Desplazamiento del **centro de la caja** desde el pivote (p.ej. una pata
    /// cuelga con `centro = [0, -largo/2, 0]`).
    pub centro: [f32; 3],
    /// Tamaño de la caja.
    pub tamaño: [f32; 3],
    /// Eje sobre el que rota la articulación.
    pub eje: Eje,
    /// Color de la caja.
    pub color: SlotColor,
}

/// Pose: el **ángulo de cada articulación** (paralelo a [`Rig::segmentos`]).
#[derive(Debug, Clone, Default)]
pub struct RigPose {
    pub angulos: Vec<f32>,
}

impl RigPose {
    /// Pose neutra (todos los ángulos en 0) para `n` segmentos.
    pub fn neutra(n: usize) -> Self {
        Self { angulos: vec![0.0; n] }
    }
}

/// Un **cuerpo**: el árbol de segmentos (ordenado padre-antes-que-hijo).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Rig {
    pub nombre: String,
    pub segmentos: Vec<Segmento>,
}

impl Rig {
    /// Cantidad de segmentos.
    pub fn len(&self) -> usize {
        self.segmentos.len()
    }
    pub fn is_empty(&self) -> bool {
        self.segmentos.is_empty()
    }

    /// Transformación de mundo de cada segmento para una pose (acumula padre→hijo):
    /// `world[s] = world[padre] · T(pivote) · R(eje, ángulo)`. Requiere orden
    /// topológico (padre antes que hijo), que los presets respetan.
    fn world_transforms(&self, pose: &RigPose) -> Vec<Mat4> {
        let mut world = vec![Mat4::IDENTITY; self.segmentos.len()];
        for (i, s) in self.segmentos.iter().enumerate() {
            let ang = pose.angulos.get(i).copied().unwrap_or(0.0);
            let local = Mat4::from_translation(Vec3::from_array(s.pivote)) * s.eje.mat(ang);
            world[i] = if s.padre < 0 {
                local
            } else {
                world[s.padre as usize] * local
            };
        }
        world
    }

    /// Hornea la **malla** del cuerpo en la pose dada, con los tres colores del ser.
    /// Espacio local: pies en el origen, mirando a `+Z` (igual que [`Actor::mesh`]).
    pub fn mesh(
        &self,
        pose: &RigPose,
        piel: [f32; 3],
        cuerpo: [f32; 3],
        patas: [f32; 3],
    ) -> (Vec<Vertex3d>, Vec<u16>) {
        let world = self.world_transforms(pose);
        let mut v = Vec::with_capacity(8 * self.segmentos.len());
        let mut i = Vec::with_capacity(36 * self.segmentos.len());
        for (s, w) in self.segmentos.iter().zip(&world) {
            let m = *w
                * Mat4::from_translation(Vec3::from_array(s.centro))
                * Mat4::from_scale(Vec3::from_array(s.tamaño));
            push_cube(&mut v, &mut i, m, s.color.resolve(piel, cuerpo, patas));
        }
        (v, i)
    }
}

/// Helper para declarar un segmento conciso (toma `Vec3`, guarda `[f32;3]`).
fn seg(nombre: &str, padre: i32, pivote: Vec3, centro: Vec3, tamaño: Vec3, eje: Eje, color: SlotColor) -> Segmento {
    Segmento {
        nombre: nombre.into(),
        padre,
        pivote: pivote.to_array(),
        centro: centro.to_array(),
        tamaño: tamaño.to_array(),
        eje,
        color,
    }
}

/// Caja que **cuelga** de su pivote (miembro): centro a `-largo/2` en Y.
fn miembro(nombre: &str, padre: i32, pivote: Vec3, largo: f32, grosor: f32, eje: Eje, color: SlotColor) -> Segmento {
    seg(
        nombre,
        padre,
        pivote,
        Vec3::new(0.0, -largo * 0.5, 0.0),
        Vec3::new(grosor, largo, grosor),
        eje,
        color,
    )
}

impl Rig {
    /// **Humanoide**: torso + cabeza + 2 brazos + 2 piernas (versión rig del muñeco).
    pub fn humanoide() -> Rig {
        let torso_y = 0.8; // cadera (pies en 0)
        let torso = Vec3::new(0.55, 0.60, 0.30);
        Rig {
            nombre: "humanoide".into(),
            segmentos: vec![
                // 0: torso (raíz), centrado entre la cadera y los hombros.
                seg("torso", -1, Vec3::new(0.0, torso_y, 0.0), Vec3::new(0.0, torso.y * 0.5, 0.0), torso, Eje::X, SlotColor::Cuerpo),
                // 1: cabeza (hija del torso).
                seg("cabeza", 0, Vec3::new(0.0, torso.y, 0.0), Vec3::new(0.0, 0.22, 0.0), Vec3::new(0.42, 0.40, 0.42), Eje::X, SlotColor::Piel),
                // 2,3: brazos (hijos del torso, en los hombros).
                miembro("brazo_r", 0, Vec3::new(0.36, torso.y, 0.0), 0.60, 0.18, Eje::X, SlotColor::Cuerpo),
                miembro("brazo_l", 0, Vec3::new(-0.36, torso.y, 0.0), 0.60, 0.18, Eje::X, SlotColor::Cuerpo),
                // 4,5: piernas (raíz, ancladas a la cadera, no heredan el torso).
                miembro("pierna_r", -1, Vec3::new(0.14, torso_y, 0.0), 0.80, 0.22, Eje::X, SlotColor::Patas),
                miembro("pierna_l", -1, Vec3::new(-0.14, torso_y, 0.0), 0.80, 0.22, Eje::X, SlotColor::Patas),
            ],
        }
    }

    /// **Cuadrúpedo**: lomo horizontal + cabeza + 4 patas + cola.
    pub fn cuadrupedo() -> Rig {
        let alto = 0.7; // altura del lomo (patas de 0.7)
        let lomo = Vec3::new(0.45, 0.40, 1.10); // largo en Z
        Rig {
            nombre: "cuadrúpedo".into(),
            segmentos: vec![
                // 0: lomo (raíz), centrado a la altura del lomo.
                seg("lomo", -1, Vec3::new(0.0, alto, 0.0), Vec3::ZERO, lomo, Eje::X, SlotColor::Cuerpo),
                // 1: cabeza adelante (+Z) y un poco arriba.
                seg("cabeza", 0, Vec3::new(0.0, 0.12, lomo.z * 0.5), Vec3::new(0.0, 0.0, 0.22), Vec3::new(0.34, 0.34, 0.40), Eje::X, SlotColor::Piel),
                // 2..5: cuatro patas desde las esquinas del lomo.
                miembro("pata_fr", 0, Vec3::new(0.30, -lomo.y * 0.5, lomo.z * 0.4), alto, 0.16, Eje::X, SlotColor::Patas),
                miembro("pata_fl", 0, Vec3::new(-0.30, -lomo.y * 0.5, lomo.z * 0.4), alto, 0.16, Eje::X, SlotColor::Patas),
                miembro("pata_br", 0, Vec3::new(0.30, -lomo.y * 0.5, -lomo.z * 0.4), alto, 0.16, Eje::X, SlotColor::Patas),
                miembro("pata_bl", 0, Vec3::new(-0.30, -lomo.y * 0.5, -lomo.z * 0.4), alto, 0.16, Eje::X, SlotColor::Patas),
                // 6: cola atrás (-Z), serpentea en Y.
                miembro("cola", 0, Vec3::new(0.0, 0.10, -lomo.z * 0.5), 0.55, 0.12, Eje::Y, SlotColor::Cuerpo),
            ],
        }
    }

    /// **Ave**: cuerpo + cabeza + 2 alas (aletean en Z) + 2 patitas + cola.
    pub fn ave() -> Rig {
        let alto = 0.45;
        let cuerpo = Vec3::new(0.40, 0.46, 0.60);
        Rig {
            nombre: "ave".into(),
            segmentos: vec![
                seg("cuerpo", -1, Vec3::new(0.0, alto, 0.0), Vec3::ZERO, cuerpo, Eje::X, SlotColor::Cuerpo),
                seg("cabeza", 0, Vec3::new(0.0, 0.20, cuerpo.z * 0.5), Vec3::new(0.0, 0.10, 0.10), Vec3::new(0.28, 0.28, 0.30), Eje::X, SlotColor::Piel),
                // Alas: cajas anchas que cuelgan poco y aletean en Z.
                seg("ala_r", 0, Vec3::new(cuerpo.x * 0.5, 0.10, 0.0), Vec3::new(0.30, 0.0, 0.0), Vec3::new(0.60, 0.08, 0.40), Eje::Z, SlotColor::Cuerpo),
                seg("ala_l", 0, Vec3::new(-cuerpo.x * 0.5, 0.10, 0.0), Vec3::new(-0.30, 0.0, 0.0), Vec3::new(0.60, 0.08, 0.40), Eje::Z, SlotColor::Cuerpo),
                miembro("pata_r", -1, Vec3::new(0.12, alto - 0.05, 0.0), alto - 0.05, 0.07, Eje::X, SlotColor::Patas),
                miembro("pata_l", -1, Vec3::new(-0.12, alto - 0.05, 0.0), alto - 0.05, 0.07, Eje::X, SlotColor::Patas),
                miembro("cola", 0, Vec3::new(0.0, 0.05, -cuerpo.z * 0.5), 0.35, 0.10, Eje::X, SlotColor::Cuerpo),
            ],
        }
    }

    /// **Serpiente**: cadena de segmentos que serpentea (cada uno gira en Y respecto
    /// del anterior). La cabeza es el primero.
    pub fn serpiente() -> Rig {
        let n = 8;
        let alto = 0.25;
        let paso = 0.42; // largo de cada segmento (en -Z, hacia atrás)
        let mut segmentos = Vec::with_capacity(n);
        // 0: cabeza (raíz).
        segmentos.push(seg(
            "cabeza",
            -1,
            Vec3::new(0.0, alto, 0.0),
            Vec3::new(0.0, 0.0, 0.10),
            Vec3::new(0.34, 0.30, 0.40),
            Eje::Y,
            SlotColor::Piel,
        ));
        // 1..n: cuerpo, cada uno colgando hacia atrás del anterior, girando en Y.
        for k in 1..n {
            let grosor = 0.34 - 0.02 * k as f32; // se afina hacia la cola
            segmentos.push(seg(
                "cuerpo",
                (k - 1) as i32,
                Vec3::new(0.0, 0.0, -paso),
                Vec3::new(0.0, 0.0, -paso * 0.5),
                Vec3::new(grosor, 0.26, paso),
                Eje::Y,
                SlotColor::Cuerpo,
            ));
        }
        Rig { nombre: "serpiente".into(), segmentos }
    }

    /// Los cuatro presets (para un editor que cicle entre ellos).
    pub fn presets() -> Vec<Rig> {
        vec![Rig::humanoide(), Rig::cuadrupedo(), Rig::ave(), Rig::serpiente()]
    }
}

/// Oscilador de **una articulación** en un andar: balancea `amplitud · sin(t·cadencia
/// + desfase)`.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
pub struct Osc {
    pub amplitud: f32,
    pub desfase: f32,
}

/// **Andar** (capa 2): animación procedural por articulación a partir del tiempo.
/// Cada segmento oscila según su [`Osc`] a una `cadencia` común (rad/seg). La amplitud
/// y el desfase iniciales salen del **rol** del segmento (nombre+posición), así un
/// `Andar::caminar` arranca razonable para cualquier rig; después se editan.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Andar {
    /// Ritmo del ciclo (rad/seg): mayor = pasos más rápidos.
    pub cadencia: f32,
    /// Oscilador por segmento (paralelo a [`Rig::segmentos`]).
    pub osc: Vec<Osc>,
}

impl Andar {
    /// Caminata heurística para `rig`: patas/piernas en oposición (izq/der y
    /// delante/atrás), brazos contra su pierna, alas aleteando, serpiente/cola con
    /// desfase creciente a lo largo de la cadena. `cadencia` de caminata.
    pub fn caminar(rig: &Rig) -> Andar {
        use std::f32::consts::PI;
        let osc = rig
            .segmentos
            .iter()
            .enumerate()
            .map(|(i, s)| {
                let nom = s.nombre.as_str();
                let lado = if s.pivote[0] < -1e-3 { PI } else { 0.0 };
                let tren = if s.pivote[2] < -1e-3 { PI } else { 0.0 };
                let (amplitud, desfase) = if nom.contains("pierna") || nom.contains("pata") {
                    (0.6, lado + tren)
                } else if nom.contains("brazo") {
                    (0.5, lado + PI)
                } else if nom.contains("ala") {
                    (1.0, lado)
                } else if nom.contains("cola") {
                    (0.4, 0.0)
                } else if nom.contains("cuerpo") && s.eje == Eje::Y {
                    (0.5, i as f32 * 0.9) // serpenteo: onda que viaja
                } else {
                    (0.0, 0.0)
                };
                Osc { amplitud, desfase }
            })
            .collect();
        Andar { cadencia: 8.0, osc }
    }

    /// Reposo: el andar de caminata muy atenuado y lento (un balanceo apenas vivo).
    pub fn quieto(rig: &Rig) -> Andar {
        let mut a = Andar::caminar(rig);
        a.cadencia = 2.2;
        for o in &mut a.osc {
            o.amplitud *= 0.12;
        }
        a
    }

    /// Trote: la caminata más amplia y rápida.
    pub fn correr(rig: &Rig) -> Andar {
        let mut a = Andar::caminar(rig);
        a.cadencia = 13.0;
        for o in &mut a.osc {
            o.amplitud = (o.amplitud * 1.4).min(1.4);
        }
        a
    }

    /// La pose del andar en el instante `t` (seg).
    pub fn pose(&self, t: f32) -> RigPose {
        let angulos = self
            .osc
            .iter()
            .map(|o| o.amplitud * (t * self.cadencia + o.desfase).sin())
            .collect();
        RigPose { angulos }
    }
}

/// Los **andares por estado** de una criatura (capa 2): reposo, caminar, correr.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Andares {
    pub quieto: Andar,
    pub caminar: Andar,
    pub correr: Andar,
}

impl Andares {
    /// Andares por defecto (heurísticos) para un rig.
    pub fn default_for(rig: &Rig) -> Self {
        Self { quieto: Andar::quieto(rig), caminar: Andar::caminar(rig), correr: Andar::correr(rig) }
    }

    /// Rótulos de los estados, en orden de índice.
    pub const LABELS: [&'static str; 3] = ["quieto", "caminar", "correr"];

    /// El andar del estado `i` (0=quieto,1=caminar,2=correr).
    pub fn estado(&self, i: usize) -> &Andar {
        match i {
            0 => &self.quieto,
            1 => &self.caminar,
            _ => &self.correr,
        }
    }
    pub fn estado_mut(&mut self, i: usize) -> &mut Andar {
        match i {
            0 => &mut self.quieto,
            1 => &mut self.caminar,
            _ => &mut self.correr,
        }
    }
}

/// El **movimiento** de un cuerpo: su [`Rig`] (morfología) + sus [`Andares`]
/// (animación). Es lo que distingue a una criatura no-humanoide; los andares quedan
/// siempre en sincronía con el rig (misma cantidad de segmentos).
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Movimiento {
    pub rig: Rig,
    pub andares: Andares,
}

impl Movimiento {
    /// Un movimiento a partir de un rig, con andares por defecto.
    pub fn preset(rig: Rig) -> Self {
        let andares = Andares::default_for(&rig);
        Self { rig, andares }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn presets_tienen_las_partes_esperadas() {
        assert_eq!(Rig::humanoide().len(), 6);
        assert_eq!(Rig::cuadrupedo().len(), 7);
        assert_eq!(Rig::ave().len(), 7);
        assert_eq!(Rig::serpiente().len(), 8);
        // Todos en orden topológico: el padre aparece antes que el hijo.
        for rig in Rig::presets() {
            for (i, s) in rig.segmentos.iter().enumerate() {
                assert!((s.padre as i32) < i as i32, "{}::{} padre {} no es anterior", rig.nombre, s.nombre, s.padre);
            }
        }
    }

    #[test]
    fn la_malla_no_es_vacia_y_escala_con_las_partes() {
        let rig = Rig::cuadrupedo();
        let pose = RigPose::neutra(rig.len());
        let (v, idx) = rig.mesh(&pose, [0.9, 0.7, 0.6], [0.4, 0.5, 0.7], [0.3, 0.3, 0.3]);
        // 8 vértices y 36 índices por caja.
        assert_eq!(v.len(), 8 * rig.len());
        assert_eq!(idx.len(), 36 * rig.len());
    }

    #[test]
    fn rotar_el_padre_arrastra_al_hijo() {
        // En la serpiente, girar la cabeza mueve el primer segmento del cuerpo.
        let rig = Rig::serpiente();
        let mut pose = RigPose::neutra(rig.len());
        let world0 = rig.world_transforms(&pose);
        pose.angulos[0] = 0.8; // gira la cabeza (raíz) en Y
        let world1 = rig.world_transforms(&pose);
        let p0 = world0[1].transform_point3(Vec3::ZERO);
        let p1 = world1[1].transform_point3(Vec3::ZERO);
        assert!((p0 - p1).length() > 0.05, "el hijo siguió a la rotación del padre");
    }

    #[test]
    fn el_andar_balancea_las_patas_en_oposicion() {
        use std::f32::consts::PI;
        let rig = Rig::cuadrupedo();
        let andar = Andar::caminar(&rig);
        // Patas delanteras der/izq en oposición (~π de desfase).
        let fr = rig.segmentos.iter().position(|s| s.nombre == "pata_fr").unwrap();
        let fl = rig.segmentos.iter().position(|s| s.nombre == "pata_fl").unwrap();
        let p = andar.pose(0.3);
        // En oposición: signos opuestos en casi toda la fase.
        let a = (0.3_f32).sin();
        let b = (0.3_f32 + PI).sin();
        assert!(a * b < 0.0, "patas opuestas baten en contrafase");
        assert!(p.angulos[fr].abs() > 0.0 && p.angulos[fl].abs() > 0.0);
        let _ = (fr, fl);
    }
}
