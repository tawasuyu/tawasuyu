//! `agorapura` — demostración narrada del ágora de identidad.
//!
//! Recorre el escenario canónico de extremo a extremo: la institución
//! *Venezuela* atestigua la nacionalidad de la persona *Yumaira*, y
//! otras identidades la corroboran. Imprime la evidencia acumulada y
//! cómo distintas políticas *negociadas* la aceptan o no.
//!
//! No es la app definitiva — es un smoke test legible y la mejor forma
//! de ver el módulo funcionando: `cargo run -p agorapura`.

use agorapura_core::{Attestation, Claim, IdentityKind, Keypair};
use agorapura_graph::{TrustGraph, TrustPolicy};

/// Segundo Unix fijo para que la demo sea reproducible.
const T0: u64 = 1_700_000_000;

fn main() {
    println!("\n  ágora · demostración de identidad federada\n");

    // --- Identidades. Semillas fijas → demo reproducible. ---
    let yumaira = Keypair::from_seed([20; 32]);
    let venezuela = Keypair::from_seed([10; 32]);
    let comunidad = Keypair::from_seed([30; 32]);
    let vecina = Keypair::from_seed([40; 32]);

    let mut agora = TrustGraph::new();
    agora.register(yumaira.identity(IdentityKind::Person, "Yumaira"));
    agora.register(venezuela.identity(IdentityKind::Institution, "Venezuela"));
    agora.register(comunidad.identity(IdentityKind::Community, "Vecinos del Valle"));
    agora.register(vecina.identity(IdentityKind::Person, "Carmen"));

    println!("  identidades registradas:");
    for kp in [&yumaira, &venezuela, &comunidad, &vecina] {
        let id = kp.identity_id();
        let name = agora.identity(id).map(|i| i.display_name.as_str()).unwrap_or("?");
        println!("    {id}  {name}");
    }

    // --- Atestaciones sobre la nacionalidad de Yumaira. ---
    let nacionalidad = |by: &Keypair| {
        Attestation::create(
            by,
            Claim::new(yumaira.identity_id(), "nacionalidad", "venezolana", T0),
        )
    };
    println!("\n  atestaciones de «nacionalidad = venezolana» sobre Yumaira:");
    for (by, label) in [
        (&venezuela, "Venezuela (institución)"),
        (&comunidad, "Vecinos del Valle (comunidad)"),
        (&yumaira, "Yumaira (ella misma)"),
    ] {
        let att = nacionalidad(by);
        match agora.add_attestation(att) {
            Ok(()) => println!("    ✔ firma verificada — {label}"),
            Err(e) => println!("    ✘ rechazada — {label}: {e}"),
        }
    }

    // --- Intento de fraude: una firma manipulada. ---
    let mut falsa = nacionalidad(&vecina);
    falsa.claim.value = "marciana".into(); // rompe la firma
    print!("\n  intento de atestación con firma manipulada: ");
    match agora.add_attestation(falsa) {
        Ok(()) => println!("ACEPTADA (esto sería un bug)"),
        Err(e) => println!("rechazada — {e}"),
    }

    // --- Corroboración. ---
    let c = agora.corroboration(yumaira.identity_id(), "nacionalidad", "venezolana");
    println!("\n  corroboración del claim:");
    println!("    atestadores totales : {}", c.total());
    println!("    terceros (no ella)  : {}", c.third_party());
    println!("    auto-atestado       : {}", c.self_attested);

    // --- Veredicto según la política negociada. ---
    println!("\n  veredicto según la política (la verdad depende de lo pactado):");
    for (policy, label) in [
        (TrustPolicy::strict(1), "laxa  · 1 tercero basta"),
        (TrustPolicy::strict(2), "media · 2 terceros"),
        (TrustPolicy::strict(3), "estricta · 3 terceros"),
    ] {
        let ok = policy.accepts(&c);
        let mark = if ok { "ACEPTA" } else { "rechaza" };
        println!("    [{mark}]  {label}");
    }

    println!(
        "\n  el ágora no dicta la verdad: acumula evidencia firmada y\n  \
         cada quien la pesa con la política que negocie.\n"
    );
}
