// =============================================================================
//  ayni :: ayni-core — el grafo de la conversación soberana
// -----------------------------------------------------------------------------
//  Ayni (reciprocidad andina) es chat persona-a-persona local-first: dueño de
//  tus bytes, sin servidor, sobrevive a que muera cualquier empresa. Su tesis
//  no son features añadidas sino el SUSTRATO de gioser llevado a la
//  conversación: BLAKE3 + DAG direccionado por contenido (de `format`),
//  identidad agora Ed25519, transporte por chasqui/minga/akasha.
//
//  Este crate es la PRIMERA capa (P0): el modelo de datos puro. Una
//  conversación es un DAG de mensajes firmados —no un log lineal—, de modo que
//  los hilos son ramas reales, el estado es reproducible por hash, y reordenar
//  el hilo de un autor invalida su firma. Nada de red, nada de cifrado de
//  sesión, nada de UI: eso son P1+ (chasqui), P2 (MLS, en `ayni-crypto`),
//  P3 (sync minga), P-UI (`ayni-llimphi`).
//
//  `#![no_std] + alloc` desde el día cero —para que el MISMO núcleo corra como
//  app WASM dentro de wawa (P6)— y cripto-agnóstico —la firma entra y se
//  verifica por closure; las primitivas viven en `ayni-crypto`/`agora`—.
// =============================================================================

#![cfg_attr(not(test), no_std)]

extern crate alloc;

mod conversacion;
mod error;
mod nodo;

pub use conversacion::Conversacion;
pub use error::ErrorAyni;
pub use nodo::{Adjunto, Carga, Contenido, MensajeNodo, VERSION_NODO};

// Re-export de los tipos del grafo soberano que un consumidor de Ayni maneja
// constantemente, para que no tenga que depender de `format` por separado sólo
// para nombrar un id o una identidad.
pub use format::{hash, AgoraId, Firma, Hash};

#[cfg(test)]
mod tests {
    use super::*;
    use ed25519_dalek::{Signer, SigningKey, Verifier, VerifyingKey};

    // --- arnés de firma Ed25519 REAL para los tests -------------------------
    // Prueba que los closures de `ayni-core` componen con la cripto que
    // `ayni-crypto`/`agora` proveerán en runtime (agora usa el mismo dalek).

    fn clave(semilla: u8) -> SigningKey {
        SigningKey::from_bytes(&[semilla; 32])
    }

    fn autor_de(sk: &SigningKey) -> AgoraId {
        sk.verifying_key().to_bytes()
    }

    /// Firmante: cierra sobre una `SigningKey` y firma los 32 bytes del id.
    fn firmar_con(sk: &SigningKey) -> impl FnOnce(&Hash) -> Firma + '_ {
        move |id: &Hash| sk.sign(id).to_bytes()
    }

    /// Verificador genérico Ed25519: el que pasaría la capa de aplicación.
    fn verificador(autor: &AgoraId, id: &Hash, firma: &Firma) -> bool {
        let Ok(vk) = VerifyingKey::from_bytes(autor) else {
            return false;
        };
        let sig = ed25519_dalek::Signature::from_bytes(firma);
        vk.verify(id, &sig).is_ok()
    }

    #[test]
    fn id_depende_del_contenido_no_de_la_firma() {
        let a = Contenido::nuevo([1u8; 32], alloc::vec![], Carga::Texto("hola".into()), 10);
        let b = Contenido::nuevo([1u8; 32], alloc::vec![], Carga::Texto("hola".into()), 10);
        assert_eq!(a.id(), b.id(), "mismo contenido ⇒ mismo id (determinista)");

        let c = Contenido::nuevo([1u8; 32], alloc::vec![], Carga::Texto("chau".into()), 10);
        assert_ne!(a.id(), c.id(), "distinta carga ⇒ distinto id");
    }

    #[test]
    fn padres_se_normalizan_a_forma_canonica() {
        let p1 = [9u8; 32];
        let p2 = [3u8; 32];
        // mismo conjunto de padres en distinto orden + un duplicado:
        let a = Contenido::nuevo([1u8; 32], alloc::vec![p1, p2], Carga::Texto("x".into()), 1);
        let b = Contenido::nuevo([1u8; 32], alloc::vec![p2, p1, p1], Carga::Texto("x".into()), 1);
        assert_eq!(a.padres, alloc::vec![p2, p1], "ordenados y deduplicados");
        assert_eq!(a.id(), b.id(), "el id no depende del orden de los padres");
    }

    #[test]
    fn sellar_y_verificar_firma_real() {
        let sk = clave(7);
        let autor = autor_de(&sk);
        let contenido = Contenido::nuevo(autor, alloc::vec![], Carga::Texto("firmado".into()), 42);
        let nodo = MensajeNodo::sellar(contenido, firmar_con(&sk));

        assert!(nodo.verificar(verificador), "la firma del autor valida");
    }

    #[test]
    fn contenido_manipulado_invalida_la_firma() {
        let sk = clave(7);
        let autor = autor_de(&sk);
        let contenido = Contenido::nuevo(autor, alloc::vec![], Carga::Texto("original".into()), 1);
        let mut nodo = MensajeNodo::sellar(contenido, firmar_con(&sk));

        // un atacante reescribe el texto tras la firma:
        nodo.contenido.carga = Carga::Texto("manipulado".into());
        assert!(
            !nodo.verificar(verificador),
            "el id cambia ⇒ la firma vieja no valida el contenido nuevo"
        );
    }

    #[test]
    fn firma_de_otra_clave_no_valida() {
        let sk = clave(1);
        let impostor = clave(2);
        let autor = autor_de(&sk); // el nodo dice ser de `sk`...
        let contenido = Contenido::nuevo(autor, alloc::vec![], Carga::Texto("hola".into()), 1);
        // ...pero lo firma el impostor:
        let nodo = MensajeNodo::sellar(contenido, firmar_con(&impostor));
        assert!(!nodo.verificar(verificador), "firma ajena al autor declarado");
    }

    #[test]
    fn agregar_rechaza_padre_ausente() {
        let sk = clave(1);
        let autor = autor_de(&sk);
        let huerfano = Contenido::nuevo(autor, alloc::vec![[42u8; 32]], Carga::Texto("?".into()), 1);
        let nodo = MensajeNodo::sellar(huerfano, firmar_con(&sk));
        let mut conv = Conversacion::nueva();
        assert_eq!(conv.agregar(nodo), Err(ErrorAyni::PadreAusente));
    }

    #[test]
    fn agregar_es_idempotente() {
        let sk = clave(1);
        let autor = autor_de(&sk);
        let mut conv = Conversacion::nueva();
        let id = conv
            .publicar_texto(autor, "uno", 1, firmar_con(&sk))
            .unwrap();
        // reinsertar el mismo nodo no duplica ni falla:
        let nodo = conv.obtener(&id).unwrap().clone();
        assert_eq!(conv.agregar(nodo), Ok(id));
        assert_eq!(conv.len(), 1);
    }

    #[test]
    fn cabezas_y_raices_de_un_hilo_lineal() {
        let sk = clave(1);
        let autor = autor_de(&sk);
        let mut conv = Conversacion::nueva();
        let r = conv.publicar_texto(autor, "raíz", 1, firmar_con(&sk)).unwrap();
        let _m = conv.publicar_texto(autor, "medio", 2, firmar_con(&sk)).unwrap();
        let c = conv.publicar_texto(autor, "cabeza", 3, firmar_con(&sk)).unwrap();

        assert_eq!(conv.raices(), alloc::vec![r], "una sola raíz en hilo lineal");
        assert_eq!(conv.cabezas(), alloc::vec![c], "una sola cabeza en hilo lineal");
    }

    #[test]
    fn bifurcacion_y_reconciliacion() {
        // dos autores responden A LA VEZ a la misma raíz (bifurcan), y un
        // tercer mensaje los une (reconcilia): el DAG modela hilos reales.
        let alicia = clave(1);
        let beto = clave(2);
        let a_id = autor_de(&alicia);
        let b_id = autor_de(&beto);
        let mut conv = Conversacion::nueva();

        let raiz = conv.publicar_texto(a_id, "¿café?", 1, firmar_con(&alicia)).unwrap();

        // ambos ven sólo la raíz y responden: dos nodos con el MISMO padre.
        let rama_a = {
            let c = Contenido::nuevo(a_id, alloc::vec![raiz], Carga::Texto("sí".into()), 2);
            let n = MensajeNodo::sellar(c, firmar_con(&alicia));
            conv.agregar(n).unwrap()
        };
        let rama_b = {
            let c = Contenido::nuevo(b_id, alloc::vec![raiz], Carga::Texto("té mejor".into()), 2);
            let n = MensajeNodo::sellar(c, firmar_con(&beto));
            conv.agregar(n).unwrap()
        };

        // hay DOS cabezas tras la bifurcación:
        let mut cabezas = conv.cabezas();
        cabezas.sort();
        let mut esperadas = alloc::vec![rama_a, rama_b];
        esperadas.sort();
        assert_eq!(cabezas, esperadas, "la conversación está bifurcada");

        // un mensaje nuevo redactado toma AMBAS cabezas como padres:
        let union = conv.redactar(a_id, Carga::Texto("ok los dos".into()), 3, firmar_con(&alicia));
        assert_eq!(union.padres().len(), 2, "el nodo une las dos ramas");
        let union_id = conv.agregar(union).unwrap();

        assert_eq!(conv.cabezas(), alloc::vec![union_id], "reconciliado: una cabeza");
        // sigue habiendo una sola raíz:
        assert_eq!(conv.raices(), alloc::vec![raiz]);
    }

    #[test]
    fn orden_topologico_respeta_padres_y_es_determinista() {
        let sk = clave(1);
        let autor = autor_de(&sk);
        let mut conv = Conversacion::nueva();
        let raiz = conv.publicar_texto(autor, "0", 1, firmar_con(&sk)).unwrap();
        let a = {
            let c = Contenido::nuevo(autor, alloc::vec![raiz], Carga::Texto("a".into()), 2);
            conv.agregar(MensajeNodo::sellar(c, firmar_con(&sk))).unwrap()
        };
        let b = {
            let c = Contenido::nuevo(autor, alloc::vec![raiz], Carga::Texto("b".into()), 3);
            conv.agregar(MensajeNodo::sellar(c, firmar_con(&sk))).unwrap()
        };

        let orden = conv.orden_topologico().expect("DAG sin ciclos");
        assert_eq!(orden.len(), 3);
        // la raíz va antes que sus dos hijos:
        let pos = |x: &Hash| orden.iter().position(|y| y == x).unwrap();
        assert!(pos(&raiz) < pos(&a));
        assert!(pos(&raiz) < pos(&b));
        // empate (a y b comparten padre) roto por ts: a(ts=2) antes que b(ts=3):
        assert!(pos(&a) < pos(&b), "empate roto por (ts, id)");

        // determinismo: dos llamadas dan el mismo orden.
        assert_eq!(conv.orden_topologico(), Some(orden));
    }

    #[test]
    fn verificar_firmas_de_todo_el_grafo() {
        let sk = clave(1);
        let autor = autor_de(&sk);
        let mut conv = Conversacion::nueva();
        conv.publicar_texto(autor, "uno", 1, firmar_con(&sk)).unwrap();
        conv.publicar_texto(autor, "dos", 2, firmar_con(&sk)).unwrap();
        assert!(conv.verificar_firmas(verificador).is_ok());
    }

    #[test]
    fn adjunto_referencia_viva_por_hash() {
        let bytes = b"# Documento\nun cuerpo de pluma cualquiera";
        let adj = nodo::Adjunto::de_bytes("pluma", "text/markdown", "doc.md", bytes);
        assert_eq!(adj.tamano, bytes.len() as u64);
        assert_eq!(adj.hash, format::hash(bytes), "el hash es el del contenido");
        assert!(adj.verifica(bytes), "los bytes correctos verifican");
        assert!(!adj.verifica(b"otros bytes"), "bytes ajenos no verifican");

        // un nodo con adjunto: la referencia viaja firmada; texto() es None.
        let sk = clave(5);
        let autor = autor_de(&sk);
        let conv = Conversacion::nueva();
        let n = conv.redactar(autor, Carga::Adjunto(adj.clone()), 1, firmar_con(&sk));
        assert!(n.verificar(verificador), "la referencia va firmada");
        assert_eq!(n.contenido.carga.texto(), None);
        assert_eq!(n.contenido.carga.adjunto(), Some(&adj));
    }

    #[test]
    fn roundtrip_serializacion_de_un_nodo_suelto() {
        // El grano fino que `ayni-store`, `ayni-sync` y la app de wawa (P6)
        // persisten/transmiten: un nodo solo, no la conversación entera.
        let sk = clave(3);
        let autor = autor_de(&sk);
        let c = Contenido::nuevo(autor, alloc::vec![], Carga::Texto("solo".into()), 9);
        let nodo = MensajeNodo::sellar(c, firmar_con(&sk));

        let bytes = nodo.serializar();
        let recuperado = MensajeNodo::deserializar(&bytes).expect("roundtrip de nodo");
        assert_eq!(recuperado, nodo, "el nodo sobrevive bit-a-bit al postcard");
        assert_eq!(recuperado.id(), nodo.id(), "mismo id ⇒ misma dirección");
        assert!(recuperado.verificar(verificador), "la firma sigue válida");

        assert!(
            MensajeNodo::deserializar(b"\xff\xff basura").is_err(),
            "bytes corruptos ⇒ error, no pánico"
        );
    }

    #[test]
    fn roundtrip_serializacion_de_la_conversacion() {
        let alicia = clave(1);
        let beto = clave(2);
        let a_id = autor_de(&alicia);
        let b_id = autor_de(&beto);
        let mut conv = Conversacion::nueva();
        let raiz = conv.publicar_texto(a_id, "hola", 1, firmar_con(&alicia)).unwrap();
        let _ = {
            let c = Contenido::nuevo(b_id, alloc::vec![raiz], Carga::Texto("buenas".into()), 2);
            conv.agregar(MensajeNodo::sellar(c, firmar_con(&beto))).unwrap()
        };

        let bytes = conv.serializar();
        let recuperada = Conversacion::deserializar(&bytes).expect("roundtrip");
        assert_eq!(recuperada.len(), conv.len());
        assert_eq!(recuperada.orden_topologico(), conv.orden_topologico());
        // las firmas sobreviven al viaje:
        assert!(recuperada.verificar_firmas(verificador).is_ok());
    }
}
