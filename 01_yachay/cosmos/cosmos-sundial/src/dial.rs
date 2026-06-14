//! Trazado de un cuadrante solar físico — las **líneas horarias** y la
//! geometría del estilo (gnomon).
//!
//! Mientras [`crate::sundial_reading`] resuelve la sombra *instantánea*
//! del Sol, este módulo resuelve lo otro que promete el crate: cómo
//! **diseñar** un reloj de sol. Dada la latitud del lugar y el tipo de
//! cuadrante, devuelve el ángulo de cada línea horaria sobre la cara del
//! cuadrante y la inclinación del estilo (el filo del gnomon que proyecta
//! la sombra y que debe apuntar al polo celeste).
//!
//! Las fórmulas son las clásicas de la gnomónica, exactas (sin
//! aproximación numérica):
//!
//! - **Horizontal:** `tan θ = sin φ · tan H`. El estilo se eleva `|φ|`
//!   sobre la cara. En el ecuador (`φ = 0`) degenera: todas las líneas
//!   colapsan en la de mediodía y el estilo queda plano.
//! - **Vertical** (cara mirando al ecuador — al sur en el hemisferio
//!   norte, al norte en el sur): `tan θ = cos φ · tan H`. El estilo se
//!   eleva la colatitud `90° − |φ|`. En los polos degenera.
//! - **Ecuatorial:** las líneas son **uniformes**, `θ = H` (15° por hora),
//!   porque la cara es paralela al ecuador celeste. El estilo es
//!   perpendicular a la placa (90°), apuntando al polo.
//!
//! `θ` y `H` se miden en grados. `H` (ángulo horario) es `0°` al mediodía
//! solar verdadero, negativo a la mañana, positivo a la tarde
//! (`H = 15° · (hora_solar − 12)`). `θ` se mide desde la línea de mediodía
//! (la substyle), positivo hacia el lado de la tarde, en `(−180, 180]`.

/// Tipo de cuadrante solar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DialKind {
    /// Cara horizontal (la placa yace en el suelo). El más común en
    /// jardines y plazas.
    Horizontal,
    /// Cara vertical mirando al ecuador (sur en el hemisferio norte,
    /// norte en el sur). El típico de fachada.
    Vertical,
    /// Cara paralela al ecuador celeste, inclinada la colatitud sobre el
    /// horizonte. Líneas horarias uniformes.
    Equatorial,
}

/// Una línea horaria trazada sobre la cara del cuadrante.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct HourLine {
    /// Ángulo horario `H` en grados (`0` = mediodía solar).
    pub hour_angle_deg: f64,
    /// Hora solar local correspondiente (`12 + H/15`), p.ej. `9.0`, `13.5`.
    pub local_hour: f64,
    /// Ángulo de la línea sobre la cara, en grados, medido desde la línea
    /// de mediodía. Positivo hacia la tarde.
    pub angle_deg: f64,
}

/// Trazado completo de un cuadrante para una latitud dada.
#[derive(Debug, Clone, PartialEq)]
pub struct DialLayout {
    pub kind: DialKind,
    pub latitude_deg: f64,
    /// Elevación del estilo (filo del gnomon) sobre la cara del cuadrante,
    /// en grados. El estilo debe apuntar al polo celeste. `0` ⇒ cuadrante
    /// degenerado para esa latitud (horizontal en el ecuador, vertical en
    /// el polo).
    pub style_height_deg: f64,
    /// Líneas horarias ordenadas por hora creciente.
    pub hour_lines: Vec<HourLine>,
}

impl DialLayout {
    /// `true` si el cuadrante degenera a esta latitud (estilo plano,
    /// líneas colapsadas) — caso a evitar al construir uno físico.
    pub fn is_degenerate(&self) -> bool {
        self.style_height_deg.abs() < 1e-9
    }
}

/// Ángulo de la línea horaria sobre la cara del cuadrante, en grados,
/// medido desde la línea de mediodía (positivo hacia la tarde). Usa
/// `atan2` para mantener el cuadrante correcto incluso con `|H| > 90°`
/// (sol de verano polar detrás del gnomon).
pub fn hour_line_angle_deg(kind: DialKind, latitude_deg: f64, hour_angle_deg: f64) -> f64 {
    let h = hour_angle_deg.to_radians();
    match kind {
        // Uniforme: la línea ES el ángulo horario.
        DialKind::Equatorial => hour_angle_deg,
        DialKind::Horizontal => {
            let phi = latitude_deg.to_radians();
            (phi.sin() * h.sin()).atan2(h.cos()).to_degrees()
        }
        DialKind::Vertical => {
            let phi = latitude_deg.to_radians();
            (phi.cos() * h.sin()).atan2(h.cos()).to_degrees()
        }
    }
}

/// Elevación del estilo (gnomon) sobre la cara del cuadrante, en grados.
/// El estilo apunta al polo celeste; su ángulo con la cara depende del
/// tipo y la latitud.
pub fn style_height_deg(kind: DialKind, latitude_deg: f64) -> f64 {
    match kind {
        DialKind::Horizontal => latitude_deg.abs(),
        DialKind::Vertical => 90.0 - latitude_deg.abs(),
        DialKind::Equatorial => 90.0,
    }
}

/// Construye el trazado de un cuadrante con las líneas horarias canónicas
/// de la salida a la puesta nominales: de las 6 a las 18 h solares
/// (`H ∈ [−90°, 90°]`, paso 1 h). Para más horas (verano de alta latitud)
/// usá [`hour_line_angle_deg`] directamente con los `H` que quieras.
pub fn dial_layout(kind: DialKind, latitude_deg: f64) -> DialLayout {
    let hour_lines = (6..=18)
        .map(|hour| {
            let local_hour = hour as f64;
            let hour_angle_deg = 15.0 * (local_hour - 12.0);
            HourLine {
                hour_angle_deg,
                local_hour,
                angle_deg: hour_line_angle_deg(kind, latitude_deg, hour_angle_deg),
            }
        })
        .collect();
    DialLayout {
        kind,
        latitude_deg,
        style_height_deg: style_height_deg(kind, latitude_deg),
        hour_lines,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn approx(a: f64, b: f64, tol: f64) -> bool {
        (a - b).abs() < tol
    }

    #[test]
    fn equatorial_lines_are_uniform_15_per_hour() {
        // La placa ecuatorial reparte 15° por hora, sin importar latitud.
        for lat in [0.0, 12.05, 45.0, 66.5] {
            for h in [-90.0, -45.0, -15.0, 0.0, 30.0, 75.0] {
                assert!(
                    approx(hour_line_angle_deg(DialKind::Equatorial, lat, h), h, 1e-12),
                    "ecuatorial uniforme a lat={lat}, H={h}"
                );
            }
        }
        assert_eq!(style_height_deg(DialKind::Equatorial, 30.0), 90.0);
    }

    #[test]
    fn horizontal_matches_textbook_formula() {
        // tan θ = sin φ · tan H. φ=45°, H=45° ⇒ tan θ = 0.7071 ⇒ θ=35.264°.
        let theta = hour_line_angle_deg(DialKind::Horizontal, 45.0, 45.0);
        assert!(approx(theta, 35.2644, 1e-3), "θ={theta}");
        // φ=30°, H=15° (13 h) ⇒ tan θ = 0.5·tan15° = 0.13397 ⇒ θ=7.6307°.
        let t2 = hour_line_angle_deg(DialKind::Horizontal, 30.0, 15.0);
        assert!(approx(t2, 7.6307, 1e-3), "θ={t2}");
    }

    #[test]
    fn vertical_matches_textbook_formula() {
        // tan θ = cos φ · tan H. φ=30°, H=45° ⇒ tan θ=0.8660 ⇒ θ=40.893°.
        let theta = hour_line_angle_deg(DialKind::Vertical, 30.0, 45.0);
        assert!(approx(theta, 40.893, 1e-3), "θ={theta}");
    }

    #[test]
    fn noon_line_is_zero_for_all_kinds() {
        for kind in [DialKind::Horizontal, DialKind::Vertical, DialKind::Equatorial] {
            assert!(approx(hour_line_angle_deg(kind, 40.0, 0.0), 0.0, 1e-12));
        }
    }

    #[test]
    fn hour_lines_are_antisymmetric() {
        // θ(−H) = −θ(H): mañana y tarde son espejo.
        for kind in [DialKind::Horizontal, DialKind::Vertical, DialKind::Equatorial] {
            for h in [15.0, 37.5, 60.0, 82.0] {
                let pos = hour_line_angle_deg(kind, 38.0, h);
                let neg = hour_line_angle_deg(kind, 38.0, -h);
                assert!(approx(pos, -neg, 1e-9), "kind={kind:?} H={h}: {pos} vs {neg}");
            }
        }
    }

    #[test]
    fn hour_lines_increase_monotonically_with_hour_angle() {
        // De la mañana a la tarde el ángulo crece monótono (H ∈ [−90,90]).
        let layout = dial_layout(DialKind::Horizontal, 51.5); // Londres aprox.
        for w in layout.hour_lines.windows(2) {
            assert!(
                w[1].angle_deg > w[0].angle_deg,
                "monótono: {} → {}",
                w[0].angle_deg,
                w[1].angle_deg
            );
        }
    }

    #[test]
    fn style_height_is_latitude_for_horizontal_and_colatitude_for_vertical() {
        assert!(approx(style_height_deg(DialKind::Horizontal, 40.0), 40.0, 1e-12));
        assert!(approx(style_height_deg(DialKind::Vertical, 40.0), 50.0, 1e-12));
        // Hemisferio sur: usa el valor absoluto de la latitud.
        assert!(approx(style_height_deg(DialKind::Horizontal, -12.05), 12.05, 1e-12));
    }

    #[test]
    fn horizontal_dial_degenerates_at_equator() {
        // En el ecuador el cuadrante horizontal no sirve: estilo plano y
        // todas las líneas colapsan en la de mediodía.
        let layout = dial_layout(DialKind::Horizontal, 0.0);
        assert!(layout.is_degenerate());
        for hl in &layout.hour_lines {
            // atan2(0, cos H): 0 mientras cos H ≥ 0 (|H| ≤ 90), ±180 si no.
            assert!(
                approx(hl.angle_deg, 0.0, 1e-9) || approx(hl.angle_deg.abs(), 180.0, 1e-9),
                "colapsada: H={} θ={}",
                hl.hour_angle_deg,
                hl.angle_deg
            );
        }
    }

    #[test]
    fn vertical_dial_degenerates_at_pole() {
        let layout = dial_layout(DialKind::Vertical, 90.0);
        assert!(layout.is_degenerate());
    }

    #[test]
    fn layout_covers_6_to_18_and_labels_hours() {
        let layout = dial_layout(DialKind::Equatorial, 45.0);
        assert_eq!(layout.hour_lines.len(), 13);
        assert_eq!(layout.hour_lines.first().unwrap().local_hour, 6.0);
        assert_eq!(layout.hour_lines.last().unwrap().local_hour, 18.0);
        // 6 h ⇒ H = −90°; 18 h ⇒ +90°.
        assert!(approx(layout.hour_lines.first().unwrap().hour_angle_deg, -90.0, 1e-12));
        assert!(approx(layout.hour_lines.last().unwrap().hour_angle_deg, 90.0, 1e-12));
    }

    #[test]
    fn six_oclock_lines_are_perpendicular_on_full_dials() {
        // A H=±90° la línea de las 6/18 queda perpendicular a la de
        // mediodía (θ=±90°) tanto en horizontal como en vertical.
        for kind in [DialKind::Horizontal, DialKind::Vertical] {
            let t = hour_line_angle_deg(kind, 45.0, 90.0);
            assert!(approx(t, 90.0, 1e-9), "kind={kind:?} θ={t}");
        }
    }
}
