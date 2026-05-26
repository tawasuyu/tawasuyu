//! Demo end-to-end de nakui-sheet. Construye una hoja de
//! contabilidad sencilla y demuestra:
//!   1. Edición con cascada topológica.
//!   2. Detección de ciclos.
//!   3. Invariantes que rechazan ediciones inválidas.
//!   4. Time-travel sobre el WAL.
//!   5. Persistencia + replay del log.
//!
//! No es interactivo: corre de cabo a rabo. Si pasa, los tests del
//! crate ya cubren las garantías; esto es para que un humano lea la
//! salida y vea el sistema en movimiento.

use nakui_sheet::{CellRef, SheetValue, Workbook};
use std::io::{BufReader, Cursor};

fn main() {
    println!("════════════════════════════════════════════════════════════");
    println!("  nakui-sheet — demo");
    println!("════════════════════════════════════════════════════════════\n");

    let mut wb = Workbook::new();

    section("1. Construyo una hoja de gastos");
    seed_expenses(&mut wb);
    render_grid(&wb, "A", "F", 1, 8);

    section("2. Edición con cascada");
    println!("  Tecleo C2 = 25 (era 20). La cascada llega hasta TOTAL.");
    let report = wb.set_cell(cr("C2"), "25").unwrap();
    println!("  Celdas recomputadas en orden topo:");
    for (cell, _, new) in &report.changed {
        println!("    {cell:>4} → {}", show(new));
    }
    println!();
    render_grid(&wb, "A", "F", 1, 8);

    section("3. Detección de ciclo");
    println!("  Intento D2 = F2 + 1. Como F2 = SUM(D2:E5) ya lee D2,");
    println!("  esto cerraría el bucle D2 → F2 → D2.");
    match wb.set_cell(cr("D2"), "=F2+1") {
        Ok(_) => println!("  (no esperado) edición aceptada"),
        Err(e) => println!("  RECHAZADO: {e}\n  La hoja queda intacta."),
    }
    // Confirmo que D2 sigue siendo la fórmula original.
    println!("  D2 sigue siendo {}.", wb.raw(cr("D2")).unwrap_or(""));
    println!();

    let total_actual = wb.value(cr("F2"));
    let tope = "300";
    section("4. Invariante: 'F2 ≤ 300'");
    wb.add_invariant("tope_total", &format!("=F2<={tope}")).unwrap();
    println!("  F2 actual = {}. Tope declarado = {tope}.", show(&total_actual));
    println!();
    println!("  Edición permitida: C3 = 10. Recalcula F2:");
    let _ = wb.set_cell(cr("C3"), "10").unwrap();
    println!("    F2 = {}", show(&wb.value(cr("F2"))));
    println!();
    println!("  Edición prohibida: C3 = 500 (haría F2 muy alto).");
    match wb.set_cell(cr("C3"), "500") {
        Ok(_) => println!("  (no esperado) edición aceptada"),
        Err(e) => println!("  RECHAZADO: {e}"),
    }
    println!("  La hoja queda intacta. F2 sigue siendo {}.\n", show(&wb.value(cr("F2"))));

    section("5. Time-travel");
    let total = wb.events().len();
    println!("  Eventos registrados en el WAL: {total}");
    // Localizo el evento que metió el primer F2.
    let f2_seq = wb
        .events()
        .iter()
        .position(|e| matches!(&e.event, nakui_sheet::SheetEvent::SetCell { cell, .. } if *cell == cr("F2")))
        .map(|i| i as u64);
    if let Some(seq) = f2_seq {
        println!("  F2 entró en escena en el evento #{seq}.");
        let snap_before = wb.snapshot_at(seq as usize).unwrap();
        let snap_after = wb.snapshot_at((seq + 1) as usize).unwrap();
        println!("    F2 ANTES del evento #{seq}: {}", show(&snap_before.value(cr("F2"))));
        println!("    F2 DESPUÉS:                {}", show(&snap_after.value(cr("F2"))));
    }
    println!();

    section("6. Persistencia: serializo el WAL y lo recargo");
    let mut buf = Vec::new();
    wb.write_log(&mut buf).unwrap();
    println!("  Tamaño del WAL: {} bytes", buf.len());
    println!("  Primer evento (JSONL):");
    let first_line = std::str::from_utf8(&buf)
        .unwrap()
        .lines()
        .next()
        .unwrap();
    println!("    {first_line}");
    let wb_replay = Workbook::from_log(BufReader::new(Cursor::new(buf.clone()))).unwrap();
    println!("\n  Hoja reconstruida desde el WAL:");
    render_grid(&wb_replay, "A", "F", 1, 8);

    section("Resumen");
    println!("  ✓ Decimal exacto: 0.1 + 0.2 = {}",
             show(&{
                 let mut w = Workbook::new();
                 w.set_cell(cr("A1"), "=0.1+0.2").unwrap();
                 w.value(cr("A1"))
             }));
    println!("  ✓ Cascada en orden topo (solo el subgrafo afectado).");
    println!("  ✓ Ciclos detectados antes de aplicar el cambio.");
    println!("  ✓ Invariantes atómicos: edición rechazada → hoja intacta.");
    println!("  ✓ Time-travel y replay deterministas sobre el WAL JSONL.");
}

fn seed_expenses(wb: &mut Workbook) {
    let rows = [
        ("A1", "Concepto"), ("B1", "Cant"), ("C1", "Unit"), ("D1", "Subtotal"), ("E1", "IVA"), ("F1", "TOTAL"),
        ("A2", "Café"),     ("B2", "5"),    ("C2", "20"),  ("D2", "=B2*C2"),    ("E2", "=D2*16%"), ("F2", "=SUM(D2:E5)"),
        ("A3", "Té"),       ("B3", "3"),    ("C3", "15"),  ("D3", "=B3*C3"),    ("E3", "=D3*16%"),
        ("A4", "Azúcar"),   ("B4", "2"),    ("C4", "10"),  ("D4", "=B4*C4"),    ("E4", "=D4*16%"),
    ];
    for (cell, raw) in rows {
        wb.set_cell(cr(cell), raw).unwrap();
    }
}

fn cr(s: &str) -> CellRef {
    s.parse().expect("valid cell ref")
}

fn show(v: &SheetValue) -> String {
    match v {
        SheetValue::Empty => "·".to_string(),
        other => other.to_display_string(),
    }
}

fn section(title: &str) {
    println!("─── {title} ");
    println!();
}

/// Renderiza un rango como cuadrícula ASCII. Solo para presentación
/// del demo; nada de lo que dependan los tests.
fn render_grid(wb: &Workbook, col_from: &str, col_to: &str, row_from: u32, row_to: u32) {
    let c0: u32 = cr(&format!("{col_from}1")).col;
    let c1: u32 = cr(&format!("{col_to}1")).col;
    let cell_w = 14usize;

    print!("       ");
    for c in c0..=c1 {
        print!(" {:^width$} ", CellRef::col_label(c), width = cell_w);
    }
    println!();

    for r in row_from..=row_to {
        print!("  {r:>3}  ");
        for c in c0..=c1 {
            let cell = CellRef::new(c, r - 1);
            let v = wb.value(cell);
            let s = match v {
                SheetValue::Empty => String::new(),
                _ => v.to_display_string(),
            };
            print!(" {:>width$} ", truncate(&s, cell_w), width = cell_w);
        }
        println!();
    }
    println!();
}

fn truncate(s: &str, w: usize) -> String {
    if s.chars().count() <= w {
        s.to_string()
    } else {
        let mut out: String = s.chars().take(w - 1).collect();
        out.push('…');
        out
    }
}
