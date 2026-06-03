//! `release` — empaquetado de un release de wawa: de `.wasm` a grafo firmado.
//!
//! El lazo "fragua → firma → Akasha → instala en vivo" empieza aquí. Una app
//! Rust cross-compilada a `wasm32-unknown-unknown` es sólo bytes; este módulo
//! la convierte en lo que wawa sabe absorber:
//!
//! 1. Un **objeto de bytecode** por app (`Objeto { datos: wasm, hijos: [] }`),
//!    direccionado por su hash BLAKE3.
//! 2. Un **manifiesto** (`format::Manifiesto`) que lista cada app con su región,
//!    fuel y permisos, envuelto a su vez en un `Objeto` cuyos hijos son los
//!    hashes de los bytecodes — así el GC del kernel los alcanza desde la raíz.
//! 3. Un **canal** (`format::Canal`) con una única `RaizFirmada` que recomienda
//!    el hash del manifiesto, firmada por el autor.
//! 4. El **sobre `ManifiestoFirmado`** (128 B) que `sys_manifiesto_proponer`
//!    exige — firma Ed25519 sobre los 32 bytes del hash del manifiesto.
//! 5. Los **campos del anuncio** (`autor`, `timestamp`, firma sobre el mensaje
//!    canónico) que el caller mete en `MensajeAkasha::AnunciarCanal`.
//!
//! Es lógica pura: construye y firma, sin tocar red ni el camino crítico de
//! re-ancla del kernel. El transporte (servir los objetos, difundir el anuncio)
//! vive en el caller — `wawa-explorer-aoe` para AoE sobre raw sockets.

use std::collections::BTreeSet;

use agora_core::Keypair;
use format::{
    AgoraId, Canal, EntradaApp, Firma, Hash, Manifiesto, ManifiestoFirmado, Objeto, Permisos,
    VERSION_CANAL, VERSION_MANIFIESTO,
};

use crate::{firmar_capacidad, firmar_manifiesto, firmar_raiz};

/// La especificación de una app a incluir en el manifiesto del release.
/// El `bytecode` son los bytes crudos del `.wasm` ya compilado y (idealmente)
/// pasado por `wasm-opt`.
#[derive(Clone, Debug)]
pub struct AppSpec {
    pub nombre: String,
    pub bytecode: Vec<u8>,
    /// `(x, y, ancho, alto)` del lienzo natural de la app, en píxeles.
    pub region: (u32, u32, u32, u32),
    /// Techo de memoria lineal, en bytes (el genesis usa 4 MiB).
    pub techo_memoria: u32,
    /// Presupuesto de fuel por fotograma.
    pub fuel_fotograma: u32,
    /// Bitfield de permisos (`format::PERMISO_*`).
    pub permisos: Permisos,
}

/// Un objeto del grafo listo para el cable: su hash y su payload `postcard`
/// (un `format::Objeto` serializado). El receptor re-hashea el payload y
/// confía sólo si coincide — direccionamiento por contenido de punta a punta.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ObjetoEmitido {
    pub hash: Hash,
    pub payload: Vec<u8>,
}

/// El release completo: todos los objetos que wawa debe absorber + los
/// metadatos firmados del anuncio.
#[derive(Clone, Debug)]
pub struct Release {
    /// Objetos a servir por Akasha, en orden de dependencia: bytecodes,
    /// luego el manifiesto, luego el canal.
    pub objetos: Vec<ObjetoEmitido>,
    /// Hash del objeto-manifiesto: lo que se ancla como raíz del manifiesto y
    /// lo que `AnunciarCanal.raiz` recomienda.
    pub manifiesto: Hash,
    /// Hash del objeto-canal: lo que viaja en `AnunciarCanal.canal`.
    pub canal: Hash,
    /// El sobre de 128 B para `sys_manifiesto_proponer`: firma sobre el hash.
    pub manifiesto_firmado: ManifiestoFirmado,
    /// Autor del anuncio (= pubkey del firmante).
    pub autor: AgoraId,
    /// Timestamp del anuncio (segundos UNIX), idéntico al de la raíz del canal.
    pub timestamp: u64,
    /// Firma del anuncio sobre `mensaje_a_firmar(nombre_canal, timestamp, raiz)`.
    /// Es la MISMA firma que la `RaizFirmada` del canal — un anuncio y un
    /// historial comparten firma.
    pub firma_anuncio: Firma,
}

/// Por qué falló construir un release.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ReleaseError {
    /// `format` no supo serializar un objeto/manifiesto/canal.
    Serializacion(&'static str),
    /// Una app llegó con bytecode vacío — no hay `.wasm` que anclar.
    AppVacia(String),
    /// La lista de apps está vacía: un manifiesto sin apps no tiene sentido.
    SinApps,
}

impl core::fmt::Display for ReleaseError {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            ReleaseError::Serializacion(e) => write!(f, "serialización falló: {e}"),
            ReleaseError::AppVacia(n) => write!(f, "la app «{n}» tiene bytecode vacío"),
            ReleaseError::SinApps => write!(f, "el release no tiene apps"),
        }
    }
}

impl std::error::Error for ReleaseError {}

/// Envuelve `datos` + `hijos` en un `format::Objeto`, lo serializa y devuelve
/// su hash junto al payload — listo para el cable.
fn emitir_objeto(datos: Vec<u8>, hijos: Vec<Hash>) -> Result<ObjetoEmitido, ReleaseError> {
    let obj = Objeto { datos, hijos };
    let payload = obj.serializar().map_err(ReleaseError::Serializacion)?;
    let hash = format::hash(&payload);
    Ok(ObjetoEmitido { hash, payload })
}

/// Construye un release firmado a partir del conjunto COMPLETO de apps que el
/// manifiesto debe contener (no sólo la app nueva — re-anclar a un manifiesto
/// parcial huérfanaría las demás). El `timestamp` es responsabilidad del caller
/// (segundos UNIX) para mantener esta función determinista y testeable.
///
/// Dedup por contenido: dos apps con bytecode idéntico comparten un solo objeto
/// de bytecode en el grafo (sus `EntradaApp` apuntan al mismo hash), igual que
/// hace `wawa-boot::sembrar_grafo`.
pub fn construir_release(
    apps: &[AppSpec],
    kp: &Keypair,
    canal_nombre: &str,
    timestamp: u64,
) -> Result<Release, ReleaseError> {
    if apps.is_empty() {
        return Err(ReleaseError::SinApps);
    }

    let mut objetos: Vec<ObjetoEmitido> = Vec::new();
    let mut entradas: Vec<EntradaApp> = Vec::new();
    let mut hijos_manifiesto: Vec<Hash> = Vec::new();
    let mut vistos: BTreeSet<Hash> = BTreeSet::new();

    for app in apps {
        if app.bytecode.is_empty() {
            return Err(ReleaseError::AppVacia(app.nombre.clone()));
        }
        let obj = emitir_objeto(app.bytecode.clone(), Vec::new())?;
        let bc_hash = obj.hash;
        // El grafo no guarda dos veces el mismo contenido; el manifiesto sí
        // puede referenciar el mismo bytecode desde dos entradas distintas.
        if vistos.insert(bc_hash) {
            hijos_manifiesto.push(bc_hash);
            objetos.push(obj);
        }

        // §14.1.3 — la CONCESIÓN. A diferencia de `wawa-boot` (que no tiene
        // clave privada), quien publica un release TIENE el `kp`: puede firmar,
        // aquí y ahora, una `ConcesionCapacidad` sobre `(bytecode, permisos)`.
        // Así el release viaja con su propio techo per-bytecode — ningún
        // manifiesto re-firmado río abajo escala el binario. Apps sin permisos
        // gateados (`permisos == 0`) no necesitan concesión: `None`.
        let concesion = if app.permisos != 0 {
            let c = firmar_capacidad(kp, &bc_hash, app.permisos);
            let datos = c.serializar().map_err(ReleaseError::Serializacion)?;
            let cobj = emitir_objeto(datos, Vec::new())?;
            let chash = cobj.hash;
            // Dedup: dos apps con el mismo bytecode Y los mismos permisos
            // comparten una sola concesión (mismo mensaje firmado → mismo hash).
            if vistos.insert(chash) {
                hijos_manifiesto.push(chash);
                objetos.push(cobj);
            }
            Some(chash)
        } else {
            None
        };

        let (x, y, ancho, alto) = app.region;
        entradas.push(EntradaApp {
            nombre: app.nombre.clone(),
            bytecode: bc_hash,
            region_x: x,
            region_y: y,
            region_ancho: ancho,
            region_alto: alto,
            techo_memoria: app.techo_memoria,
            fuel_fotograma: app.fuel_fotograma,
            estado: None,
            permisos: app.permisos,
            concesion,
        });
    }

    // El manifiesto, envuelto en un Objeto cuyos hijos son los bytecodes —
    // así el MARK del GC del kernel los alcanza desde la raíz del manifiesto.
    let manifiesto = Manifiesto {
        version: VERSION_MANIFIESTO,
        apps: entradas,
        configuracion: None,
        // Un release nuevo no ancla overlay de revocación: las revocaciones de
        // claves del anillo las ancla el operador aparte (SDD §4).
        overlay_revocacion: None,
        // Ni marco de escritorio: `pata` lo siembra/propone en el dispositivo.
        marco: None,
    };
    let man_datos = manifiesto.serializar().map_err(ReleaseError::Serializacion)?;
    let man_obj = emitir_objeto(man_datos, hijos_manifiesto)?;
    let manifiesto_hash = man_obj.hash;
    objetos.push(man_obj);

    // El canal con una sola raíz firmada que recomienda este manifiesto.
    let raiz_firmada = firmar_raiz(kp, canal_nombre, &manifiesto_hash, timestamp);
    let firma_anuncio = raiz_firmada.firma;
    let canal = Canal {
        version: VERSION_CANAL,
        nombre: canal_nombre.to_string(),
        autor: kp.public_key(),
        raices: vec![raiz_firmada],
    };
    let canal_datos = canal.serializar().map_err(ReleaseError::Serializacion)?;
    let canal_obj = emitir_objeto(canal_datos, vec![manifiesto_hash])?;
    let canal_hash = canal_obj.hash;
    objetos.push(canal_obj);

    // El sobre de 128 B para sys_manifiesto_proponer (firma sobre el hash).
    let manifiesto_firmado = firmar_manifiesto(kp, &manifiesto_hash);

    Ok(Release {
        objetos,
        manifiesto: manifiesto_hash,
        canal: canal_hash,
        manifiesto_firmado,
        autor: kp.public_key(),
        timestamp,
        firma_anuncio,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{verificar_canal, verificar_manifiesto, verificar_raiz};
    use format::RaizFirmada;

    fn app(nombre: &str, bc: &[u8]) -> AppSpec {
        AppSpec {
            nombre: nombre.to_string(),
            bytecode: bc.to_vec(),
            region: (10, 20, 480, 240),
            techo_memoria: 4 * 1024 * 1024,
            fuel_fotograma: 2_000_000,
            permisos: 0,
        }
    }

    #[test]
    fn release_vacio_es_error() {
        let kp = Keypair::from_seed([7u8; 32]);
        let err = construir_release(&[], &kp, "dev", 100).unwrap_err();
        assert_eq!(err, ReleaseError::SinApps);
    }

    #[test]
    fn app_sin_bytecode_es_error() {
        let kp = Keypair::from_seed([7u8; 32]);
        let err = construir_release(&[app("vacia", b"")], &kp, "dev", 100).unwrap_err();
        assert_eq!(err, ReleaseError::AppVacia("vacia".to_string()));
    }

    #[test]
    fn manifiesto_hash_coincide_con_su_objeto_emitido() {
        let kp = Keypair::from_seed([7u8; 32]);
        let r = construir_release(&[app("uno", b"\0asm-uno"), app("dos", b"\0asm-dos")], &kp, "dev", 100)
            .expect("debe construir");
        // El objeto-manifiesto está entre los emitidos y su hash es el anclado.
        let emitido = r
            .objetos
            .iter()
            .find(|o| o.hash == r.manifiesto)
            .expect("el manifiesto debe estar entre los objetos");
        assert_eq!(format::hash(&emitido.payload), r.manifiesto);
    }

    #[test]
    fn manifiesto_deserializa_y_lista_las_apps() {
        let kp = Keypair::from_seed([7u8; 32]);
        let r = construir_release(&[app("uno", b"\0asm-uno"), app("dos", b"\0asm-dos")], &kp, "dev", 100)
            .expect("debe construir");
        let emitido = r.objetos.iter().find(|o| o.hash == r.manifiesto).unwrap();
        // payload = Objeto serializado; Objeto.datos = Manifiesto serializado.
        let objeto = Objeto::deserializar(&emitido.payload).expect("objeto válido");
        let manifiesto = Manifiesto::deserializar(&objeto.datos).expect("manifiesto válido");
        assert_eq!(manifiesto.version, VERSION_MANIFIESTO);
        assert_eq!(manifiesto.apps.len(), 2);
        assert_eq!(manifiesto.apps[0].nombre, "uno");
        assert_eq!(manifiesto.apps[1].nombre, "dos");
        // Los hijos del objeto-manifiesto son los dos bytecodes.
        assert_eq!(objeto.hijos.len(), 2);
        assert!(objeto.hijos.contains(&manifiesto.apps[0].bytecode));
        assert!(objeto.hijos.contains(&manifiesto.apps[1].bytecode));
    }

    #[test]
    fn bytecode_identico_se_deduplica() {
        let kp = Keypair::from_seed([7u8; 32]);
        // Dos apps, MISMO bytecode → un solo objeto de bytecode.
        let r = construir_release(&[app("a", b"\0asm-igual"), app("b", b"\0asm-igual")], &kp, "dev", 100)
            .expect("debe construir");
        // objetos = 1 bytecode (deduped) + manifiesto + canal = 3.
        assert_eq!(r.objetos.len(), 3);
        let emitido = r.objetos.iter().find(|o| o.hash == r.manifiesto).unwrap();
        let objeto = Objeto::deserializar(&emitido.payload).unwrap();
        let manifiesto = Manifiesto::deserializar(&objeto.datos).unwrap();
        // Las dos entradas apuntan al MISMO hash de bytecode.
        assert_eq!(manifiesto.apps[0].bytecode, manifiesto.apps[1].bytecode);
        assert_eq!(objeto.hijos.len(), 1);
    }

    #[test]
    fn app_con_permisos_emite_concesion_firmada_que_verifica() {
        use crate::verificar_capacidad;
        let kp = Keypair::from_seed([42u8; 32]);
        let mut con_red = app("conectada", b"\0asm-red");
        con_red.permisos = format::PERMISO_RED | format::PERMISO_RAIZ;
        let sin_perms = app("muda", b"\0asm-muda"); // permisos: 0

        let r = construir_release(&[con_red, sin_perms], &kp, "estable", 7)
            .expect("debe construir");

        // El manifiesto referencia la concesión de la app con permisos y None
        // para la que no tiene.
        let mobj = Objeto::deserializar(
            &r.objetos.iter().find(|o| o.hash == r.manifiesto).unwrap().payload,
        )
        .unwrap();
        let manifiesto = Manifiesto::deserializar(&mobj.datos).unwrap();
        let con = manifiesto.apps.iter().find(|a| a.nombre == "conectada").unwrap();
        let muda = manifiesto.apps.iter().find(|a| a.nombre == "muda").unwrap();
        assert!(muda.concesion.is_none(), "app sin permisos no lleva concesión");
        let chash = con.concesion.expect("app con permisos lleva concesión");

        // La concesión está entre los objetos emitidos y es hija del manifiesto
        // (alcanzable por el MARK del GC del kernel).
        assert!(mobj.hijos.contains(&chash), "la concesión cuelga del manifiesto");
        let cobj = Objeto::deserializar(
            &r.objetos.iter().find(|o| o.hash == chash).unwrap().payload,
        )
        .unwrap();
        let concesion = format::ConcesionCapacidad::deserializar(&cobj.datos).unwrap();

        // La firma cubre (bytecode, permisos) bajo el autor del release.
        assert_eq!(concesion.bytecode, con.bytecode);
        assert_eq!(concesion.permisos, format::PERMISO_RED | format::PERMISO_RAIZ);
        assert_eq!(concesion.autor, kp.public_key());
        verificar_capacidad(&concesion).expect("la concesión del release verifica");

        // La intersección con lo declarado es idempotente: declarado == concedido.
        assert_eq!(
            format::permisos_efectivos(con.permisos, concesion.permisos),
            con.permisos,
        );
    }

    #[test]
    fn canal_y_sobre_firmados_verifican() {
        let kp = Keypair::from_seed([99u8; 32]);
        let r = construir_release(&[app("uno", b"\0asm-uno")], &kp, "estable", 1234)
            .expect("debe construir");

        // 1. El sobre ManifiestoFirmado verifica (firma sobre el hash).
        verificar_manifiesto(&r.manifiesto_firmado).expect("sobre válido");
        assert_eq!(r.manifiesto_firmado.manifiesto_hash, r.manifiesto);

        // 2. El canal del grafo verifica (firma sobre el mensaje canónico).
        let emitido = r.objetos.iter().find(|o| o.hash == r.canal).unwrap();
        let objeto = Objeto::deserializar(&emitido.payload).unwrap();
        let canal = Canal::deserializar(&objeto.datos).expect("canal válido");
        verificar_canal(&canal).expect("historial del canal válido");
        assert_eq!(canal.raices[0].raiz_manifiesto, r.manifiesto);

        // 3. La firma del anuncio verifica como RaizFirmada bajo el autor.
        let raiz = RaizFirmada {
            timestamp: r.timestamp,
            raiz_manifiesto: r.manifiesto,
            firma: r.firma_anuncio,
        };
        verificar_raiz(&r.autor, "estable", &raiz).expect("firma de anuncio válida");
    }

    #[test]
    fn nombre_de_canal_distinto_invalida_la_firma_del_anuncio() {
        // La firma cubre el nombre del canal: válida en "estable" no replica
        // en "dev". Es la garantía de firmar_raiz, re-verificada aquí end-to-end.
        let kp = Keypair::from_seed([99u8; 32]);
        let r = construir_release(&[app("uno", b"\0asm-uno")], &kp, "estable", 1234).unwrap();
        let raiz = RaizFirmada {
            timestamp: r.timestamp,
            raiz_manifiesto: r.manifiesto,
            firma: r.firma_anuncio,
        };
        assert!(verificar_raiz(&r.autor, "dev", &raiz).is_err());
    }
}
