use super::*;

fn snap_of(cells: &[&[(char, vt100::Color, vt100::Color)]]) -> TuiSnapshot {
    let rows = cells.len() as u16;
    let cols = cells.first().map(|r| r.len()).unwrap_or(0) as u16;
    let mut grid: Vec<Vec<TuiCell>> = Vec::with_capacity(rows as usize);
    for row in cells {
        grid.push(
            row.iter()
                .map(|(ch, fg, bg)| TuiCell {
                    ch: ch.to_string(),
                    fg: *fg,
                    bg: *bg,
                })
                .collect(),
        );
    }
    TuiSnapshot {
        cells: grid,
        rows,
        cols,
        cursor_r: 0,
        cursor_c: 0,
        hide_cursor: true,
    }
}

fn atlas() -> llimphi_widget_terminal::GlyphAtlas {
    llimphi_widget_terminal::GlyphAtlas::new(
        llimphi_ui::llimphi_text::MONO_FONT_BYTES,
        14.0,
        16,
        4,
    )
    .expect("atlas")
}

fn rect_400_200() -> llimphi_ui::PaintRect {
    llimphi_ui::PaintRect {
        x: 0.0,
        y: 0.0,
        w: 400.0,
        h: 200.0,
    }
}

#[test]
fn build_skip_blanks_con_bg_default() {
    let snap = snap_of(&[&[
        (' ', vt100::Color::Default, vt100::Color::Default),
        (' ', vt100::Color::Default, vt100::Color::Default),
    ]]);
    let mut a = atlas();
    let theme = llimphi_theme::Theme::dark();
    let cells = build_cell_instances(&snap, &mut a, theme, rect_400_200());
    assert!(cells.is_empty(), "celdas vacías con bg default no van");
}

#[test]
fn build_emite_un_instance_por_celda_con_contenido() {
    let snap = snap_of(&[
        &[
            ('h', vt100::Color::Default, vt100::Color::Default),
            ('i', vt100::Color::Default, vt100::Color::Default),
        ],
        &[
            (' ', vt100::Color::Default, vt100::Color::Default),
            ('!', vt100::Color::Default, vt100::Color::Default),
        ],
    ]);
    let mut a = atlas();
    let theme = llimphi_theme::Theme::dark();
    let cells = build_cell_instances(&snap, &mut a, theme, rect_400_200());
    // Tres chars no-blank (h, i, !), el ' ' con bg default se salta.
    assert_eq!(cells.len(), 3);
    // El primer instance debe arrancar en (pad, pad).
    assert_eq!(cells[0].cell_x, 6.0);
    assert_eq!(cells[0].cell_y, 6.0);
}

#[test]
fn build_no_skip_si_bg_explicito() {
    // Una celda con ' ' pero bg explícito (Idx) SÍ se emite (el bg
    // tiene que pintarse aunque el char sea blank).
    let snap = snap_of(&[&[
        (' ', vt100::Color::Default, vt100::Color::Idx(1)),
        (' ', vt100::Color::Default, vt100::Color::Default),
    ]]);
    let mut a = atlas();
    let theme = llimphi_theme::Theme::dark();
    let cells = build_cell_instances(&snap, &mut a, theme, rect_400_200());
    // Sólo el primero (bg explícito); el segundo (bg default) se salta.
    assert_eq!(cells.len(), 1);
}

#[test]
fn build_uv_y_color_son_consistentes() {
    let snap = snap_of(&[&[('A', vt100::Color::Default, vt100::Color::Default)]]);
    let mut a = atlas();
    let theme = llimphi_theme::Theme::dark();
    let cells = build_cell_instances(&snap, &mut a, theme, rect_400_200());
    assert_eq!(cells.len(), 1);
    let (acw, ach) = a.cell_size();
    // UV apunta al slot 0 (primer glifo rasterizado).
    assert_eq!(cells[0].uv_x, 0.0);
    assert_eq!(cells[0].uv_y, 0.0);
    assert_eq!(cells[0].uv_w, acw as f32);
    assert_eq!(cells[0].uv_h, ach as f32);
    // fg y bg no son 0 (fg = theme.fg_text, bg = default → alpha 0
    // pero los componentes no se chequean — basta con que el instance
    // se haya armado sin pánico).
    assert_ne!(cells[0].fg_rgba, 0);
}
