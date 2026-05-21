//! Vuelca a stdout el `gtk.css` que `nahual-theme` generaría para cada
//! preset. Útil para inspeccionar o depurar la exportación a toolkits
//! sin tener que cambiar de tema en una app real.
//!
//! `cargo run -p nahual-theme --example dump-toolkit-css`

use nahual_theme::{toolkit, Theme};

fn main() {
    for theme in Theme::all() {
        println!("\n================= {} =================", theme.name);
        println!("--- gtk-4.0/gtk.css ---");
        print!("{}", toolkit::gtk4_css(&theme));
        println!("--- gtk-3.0/gtk.css ---");
        print!("{}", toolkit::gtk3_css(&theme));
    }
}
