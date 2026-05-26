//! `showcase` — imprime todos los IDs del catálogo en los 3 idiomas
//! soportados. Smoke test visual + referencia para revisores.
//!
//! Ejecutar con: `cargo run -p rimay-localize --example showcase`

use std::borrow::Cow;

use rimay_localize as l10n;

/// IDs en el orden de aparición en los `.ftl`. Mantener sincronizado a
/// mano — el ejemplo es referencia, no test exhaustivo.
const IDS: &[&str] = &[
    "save", "load", "open", "close", "cancel", "confirm", "yes", "no", "delete", "edit", "new",
    "play", "pause", "resume", "stop", "file", "view", "help", "settings", "exit", "info",
    "warning", "error", "success",
];

fn main() {
    let locales = l10n::available_locales();
    for locale in &locales {
        l10n::set_locale(locale).unwrap();
        println!("\n========= {locale} =========");
        for id in IDS {
            println!("  {:<10} {}", id, l10n::t(id));
        }
        println!(
            "  {:<10} {}",
            "welcome-user",
            l10n::t_args("welcome-user", &[("name", Cow::Borrowed("Sergio"))])
        );
        println!(
            "  {:<10} {}",
            "items-count",
            l10n::t_args("items-count", &[("count", Cow::Borrowed("3"))])
        );
    }
}
