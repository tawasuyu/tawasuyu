//! Genera un wheel sample como SVG/PNG para verificación visual del
//! render (sin la app Llimphi, sin display server). Tirá esto cuando
//! tu sandbox no tiene Wayland/X11.

use cosmos_engine::{compose_with_options, NatalOptions};
use cosmos_model::{
    Chart, ChartId, ChartKind, ContactId, StoredBirthData, StoredChartConfig, TimeCertainty,
};
use cosmos_render::{compose_wheel, draw_commands_to_svg, CompositionOpts, Palette};

fn sample_chart() -> Chart {
    Chart {
        id: ChartId::new(),
        contact_id: ContactId::new(),
        kind: ChartKind::Natal,
        label: "demo".into(),
        birth_data: StoredBirthData {
            year: 1990,
            month: 6,
            day: 21,
            hour: 12,
            minute: 0,
            second: 0.0,
            tz_offset_minutes: -300,
            latitude_deg: -12.0464,
            longitude_deg: -77.0428,
            altitude_m: 154.0,
            time_certainty: TimeCertainty::Estimated,
            subject_name: None,
            birthplace_label: Some("Lima".into()),
        },
        config: StoredChartConfig::default(),
        related_chart_id: None,
        created_at_ms: 0,
    }
}

fn main() {
    let opts = NatalOptions {
        show_majors: true,
        show_minors: false,
        orb_multiplier: 1.0,
        show_dignities: true,
        harmonic: 1,
    };
    let model = compose_with_options(&sample_chart(), 0, &[], &opts).expect("compose");
    let mut copts = CompositionOpts {
        size: 900.0,
        rot_offset_deg: 0.0,
        include_bodies: true,
        palette: Palette::dark(),
        draw_ascensional_cross: true,
        show_coord_labels: true,
        show_minor_aspects: false,
        dial_3d: true,
        selected_body: None,
        detail: 1.0,
    };
    // Render base (sin selección).
    let cmds = compose_wheel(&model, &copts);
    let mut svg = draw_commands_to_svg(&cmds, 900.0);
    svg = svg.replace("<svg ", "<svg style=\"background:rgb(8,10,16)\" ");
    std::fs::write("/tmp/cosmos_wheel.svg", svg).unwrap();
    println!("→ /tmp/cosmos_wheel.svg  ({} cmds)", cmds.len());
    // Render con selección (planeta `sun`) — verifica que los aspectos y
    // cuerpos no relacionados se atenúan.
    copts.selected_body = Some("sun".into());
    let cmds = compose_wheel(&model, &copts);
    let mut svg = draw_commands_to_svg(&cmds, 900.0);
    svg = svg.replace("<svg ", "<svg style=\"background:rgb(8,10,16)\" ");
    std::fs::write("/tmp/cosmos_wheel_sun.svg", svg).unwrap();
    println!("→ /tmp/cosmos_wheel_sun.svg  ({} cmds, sun selected)", cmds.len());
}
