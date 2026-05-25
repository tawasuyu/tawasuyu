//! Imprime el keymap por defecto de mirada en formato RON — exactamente
//! lo que la app escribe la primera vez en `~/.config/mirada/keymap.ron`.
//!
//! ```sh
//! cargo run -p mirada-brain --example keymap-default
//! cargo run -p mirada-brain --example keymap-default > ~/.config/mirada/keymap.ron
//! ```

fn main() {
    print!("{}", mirada_brain::Keymap::default().documented_ron());
}
