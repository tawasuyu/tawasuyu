//! Constraints de rigging sobre la jerarquía de huesos (estilo Rive): IK de 2
//! huesos y aim. Corren **después** de que la animación posó el esqueleto,
//! ajustando rotaciones para satisfacer un objetivo; luego el skinning deforma.
//!
//! Convención: los huesos apuntan a lo largo de su eje local **+x** (su hijo se
//! traslada en +x, el efector está en +x) — la misma que arma `skel`. Todo en
//! 2D, analítico (sin iteración), sobre `kurbo`.

use kurbo::{Affine, Point, Vec2};

use crate::skel::{BoneId, Pose, Skeleton};

/// Ángulo de rotación de un `Affine` (asume escala ~uniforme y sin shear).
fn affine_rotation(aff: Affine) -> f64 {
    let c = aff.as_coeffs(); // [a, b, c, d, e, f]; a=cosθ·sx, b=sinθ·sx
    c[1].atan2(c[0])
}

/// Origen (punto de pivote) de un hueso en mundo. La rotación del propio hueso
/// no mueve su origen, así que sirve aunque vayamos a re-rotarlo.
fn bone_origin(skel: &Skeleton, bone: BoneId) -> Point {
    skel.world(bone) * Point::ZERO
}

fn parent_rotation(skel: &Skeleton, bone: BoneId) -> f64 {
    match skel.parent(bone) {
        Some(p) => affine_rotation(skel.world(p)),
        None => 0.0,
    }
}

/// Resuelve **IK de 2 huesos**: ajusta las rotaciones locales de `upper` (padre)
/// y `lower` (hijo de `upper`) para que el efector — el punto `tip_local` en el
/// frame de `lower` — alcance `target` (en espacio de mundo del esqueleto).
/// `flip` elige la solución del codo (arriba/abajo). Si `target` está fuera de
/// alcance, el brazo se estira hacia él (alcance máximo). Llama `update` al final.
///
/// Requiere que los world transforms estén actualizados antes (la animación ya
/// posó el esqueleto). Las longitudes salen del rig: `L1` = offset `upper→lower`,
/// `L2` = `|tip_local|`.
pub fn solve_two_bone_ik(
    skel: &mut Skeleton,
    upper: BoneId,
    lower: BoneId,
    tip_local: Vec2,
    target: Point,
    flip: bool,
) {
    let base = bone_origin(skel, upper);
    let l1 = skel.pose(lower).translation.hypot();
    let l2 = tip_local.hypot();
    if l1 < 1e-9 || l2 < 1e-9 {
        return;
    }
    let s = if flip { -1.0 } else { 1.0 };

    let d = target - base;
    let dist = d.hypot().clamp((l1 - l2).abs() + 1e-6, l1 + l2 - 1e-6);
    let alpha = d.y.atan2(d.x);
    let cos_beta = ((dist * dist + l1 * l1 - l2 * l2) / (2.0 * dist * l1)).clamp(-1.0, 1.0);
    let beta = cos_beta.acos();
    let cos_elbow = ((l1 * l1 + l2 * l2 - dist * dist) / (2.0 * l1 * l2)).clamp(-1.0, 1.0);
    let elbow = cos_elbow.acos();

    let theta_upper_world = alpha - s * beta;
    let theta_lower_local = s * (std::f64::consts::PI - elbow);

    let up = skel.pose(upper);
    skel.set_pose(
        upper,
        Pose {
            rotation: theta_upper_world - parent_rotation(skel, upper),
            ..up
        },
    );
    let low = skel.pose(lower);
    skel.set_pose(
        lower,
        Pose {
            rotation: theta_lower_local,
            ..low
        },
    );
    skel.update();
}

/// **Aim**: rota `bone` para que su eje local `forward` apunte desde su origen
/// hacia `target` (mundo). Llama `update`. Útil para que una cabeza/ojo/torreta
/// siga a un objetivo.
pub fn aim_at(skel: &mut Skeleton, bone: BoneId, forward: Vec2, target: Point) {
    let origin = bone_origin(skel, bone);
    let d = target - origin;
    if d.hypot() < 1e-9 || forward.hypot() < 1e-9 {
        return;
    }
    let desired_world = d.y.atan2(d.x) - forward.y.atan2(forward.x);
    let pose = skel.pose(bone);
    skel.set_pose(
        bone,
        Pose {
            rotation: desired_world - parent_rotation(skel, bone),
            ..pose
        },
    );
    skel.update();
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::skel::Pose;

    fn two_bone(l1: f64) -> (Skeleton, BoneId, BoneId) {
        let mut s = Skeleton::new();
        let a = s.add_bone(None, Pose::identity());
        let b = s.add_bone(Some(a), Pose::translate(Vec2::new(l1, 0.0)));
        s.bind();
        s.update();
        (s, a, b)
    }

    fn tip(skel: &Skeleton, lower: BoneId, tip_local: Vec2) -> Point {
        skel.world(lower) * Point::new(tip_local.x, tip_local.y)
    }

    #[test]
    fn ik_alcanza_un_objetivo_dentro_de_rango() {
        let (mut s, a, b) = two_bone(50.0);
        let tip_local = Vec2::new(50.0, 0.0); // L2 = 50, alcance 0..100
        let target = Point::new(60.0, 40.0); // dist ≈ 72 < 100
        solve_two_bone_ik(&mut s, a, b, tip_local, target, false);
        let t = tip(&s, b, tip_local);
        assert!(
            (t.x - target.x).abs() < 1e-6 && (t.y - target.y).abs() < 1e-6,
            "tip {t:?} debería alcanzar {target:?}"
        );
    }

    #[test]
    fn ik_fuera_de_rango_se_estira() {
        let (mut s, a, b) = two_bone(50.0);
        let tip_local = Vec2::new(50.0, 0.0);
        let target = Point::new(300.0, 0.0); // muy lejos → estira a ~100 en x
        solve_two_bone_ik(&mut s, a, b, tip_local, target, false);
        let t = tip(&s, b, tip_local);
        assert!((t.x - 100.0).abs() < 1e-3, "x del tip {} ~ 100", t.x);
        assert!(t.y.abs() < 1e-3, "y del tip {} ~ 0 (en línea al objetivo)", t.y);
    }

    #[test]
    fn ik_flip_da_el_otro_codo() {
        let (mut s, a, b) = two_bone(50.0);
        let tip_local = Vec2::new(50.0, 0.0);
        let target = Point::new(60.0, 40.0);

        solve_two_bone_ik(&mut s, a, b, tip_local, target, false);
        let elbow_down = bone_origin(&s, b);
        solve_two_bone_ik(&mut s, a, b, tip_local, target, true);
        let elbow_up = bone_origin(&s, b);

        // Ambas soluciones alcanzan el target, pero el codo cae en lados
        // opuestos de la línea base→target.
        assert!(
            (elbow_down.y - elbow_up.y).abs() > 1.0,
            "los codos deberían diferir: {elbow_down:?} vs {elbow_up:?}"
        );
        let t = tip(&s, b, tip_local);
        assert!((t.x - target.x).abs() < 1e-6 && (t.y - target.y).abs() < 1e-6);
    }

    #[test]
    fn aim_apunta_el_eje_forward_al_objetivo() {
        let mut s = Skeleton::new();
        let bone = s.add_bone(None, Pose::identity());
        s.bind();
        s.update();
        aim_at(&mut s, bone, Vec2::new(1.0, 0.0), Point::new(0.0, 10.0));
        // El eje +x del hueso ahora debe apuntar hacia +y (al objetivo).
        let dir = s.world(bone) * Point::new(1.0, 0.0);
        assert!(dir.x.abs() < 1e-9 && dir.y > 0.0, "dir {dir:?} debería ser +y");
    }
}
