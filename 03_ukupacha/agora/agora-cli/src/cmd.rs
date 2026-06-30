//! Definición de los subcomandos del CLI de agora.
//!
//! Todo lo que clap necesita para parsear `agora-cli <subcmd>` vive aquí.
//! Los handlers reales están en los módulos por dominio.

use std::path::PathBuf;

use agora_core::{IdentityKind, RevReason};
use clap::{Subcommand, ValueEnum};

// =============================================================================
//  Árbol de comandos raíz
// =============================================================================

#[derive(Subcommand)]
pub enum Cmd {
    /// Desbloquea una identidad y cachea su seed en el **session keyring** del
    /// kernel, para que `pacha dotfiles …` cifre/descifre sin re-pedir passphrase
    /// (Fase 3). La passphrase del keystore se toma de `AGORA_PASSPHRASE`.
    Desbloquear {
        /// Identidad a desbloquear (hex o prefijo). Si se omite y hay una sola en
        /// el keystore, se usa esa.
        #[arg(long)]
        id: Option<String>,
        /// Lee la passphrase de este archivo (en vez de `AGORA_PASSPHRASE`). Para
        /// el auto-desbloqueo «sellado» del autologin: un archivo 0600 con la frase.
        #[arg(long)]
        passphrase_file: Option<PathBuf>,
    },
    /// Operaciones sobre identidades.
    Identidad {
        #[command(subcommand)]
        op: IdentidadOp,
    },
    /// Operaciones sobre atestaciones.
    Atestacion {
        #[command(subcommand)]
        op: AtestacionOp,
    },
    /// Firma una atestación con una identidad propia y la agrega al grafo.
    Atestar {
        /// Identidad firmante (debe estar en el keystore).
        #[arg(long)]
        como: String,
        /// Sujeto del claim.
        #[arg(long)]
        sobre: String,
        /// Predicado del claim.
        #[arg(long)]
        pred: String,
        /// Valor del claim.
        #[arg(long)]
        valor: String,
    },
    /// Verifica la firma de una atestación leída desde archivo postcard.
    Verificar {
        archivo: PathBuf,
    },
    /// Exporta el grafo entero a un archivo postcard para sneakernet.
    Exportar {
        archivo: PathBuf,
    },
    /// Importa un grafo postcard y mergea sus atestaciones al local.
    Importar {
        archivo: PathBuf,
    },
    /// Resumen del grafo: cuántas identidades, atestaciones, mías.
    Grafo,
    /// Operaciones sobre canales de release (format::Canal).
    Canal {
        #[command(subcommand)]
        op: CanalOp,
    },
    /// Operaciones host-side específicas de wawa: forjar pubkey
    /// para el AGORA_AUTH_RING + forjar propuestas de manifiesto.
    Wawa {
        #[command(subcommand)]
        op: WawaOp,
    },
}

// =============================================================================
//  Subcomandos de identidad
// =============================================================================

#[derive(Subcommand)]
pub enum IdentidadOp {
    /// Genera una identidad nueva, la cifra en el keystore y la registra
    /// en el grafo.
    Nueva {
        /// Nombre legible (no es único ni autoritativo).
        #[arg(long, default_value = "yo")]
        name: String,
        /// Tipo: person, community, alliance, institution.
        #[arg(long, value_enum, default_value_t = KindArg::Person)]
        kind: KindArg,
        /// Lee la seed de stdin en vez de generarla con CSPRNG. Acepta
        /// 64 chars hex (más espacios/saltos de línea) o exactamente
        /// 32 bytes raw. Útil para restaurar identidades desde un
        /// backup offline.
        #[arg(long)]
        seed_stdin: bool,
    },
    /// Lista todas las identidades del grafo (★ = seed propia).
    Listar,
    /// Imprime la cara pública de una identidad (pubkey hex).
    Exportar {
        id: String,
    },
    /// Cambia el `display_name` de una identidad ya registrada (el id
    /// no cambia — sólo la etiqueta presentacional).
    Rename {
        /// Id o prefijo hex de la identidad a renombrar.
        id: String,
        /// Nombre nuevo. No es único; no se valida contra el grafo.
        #[arg(long)]
        nombre: String,
    },
    /// Borra una identidad del grafo local y purga sus atestaciones
    /// asociadas (como attester o como subject). Sólo aplica a seeds
    /// propias del keystore — para borrar identidades ajenas hay que
    /// pasar `--force` (se mantiene la atestación huérfana fuera).
    Remove {
        /// Id o prefijo hex de la identidad a borrar.
        id: String,
        /// Permite borrar identidades sin seed local. Por defecto sólo
        /// se aceptan las propias para evitar errores destructivos.
        #[arg(long)]
        force: bool,
        /// Borra también la seed cifrada del keystore. Sin esto, la
        /// identidad se puede re-registrar con `agora-cli identidad
        /// nueva --seed-stdin` después.
        #[arg(long)]
        purgar_keystore: bool,
    },
    /// Rota una identidad propia: forja una clave NUEVA, la liga a la vieja con
    /// una `KeyRotation` doble-firmada (la vieja autoriza el handoff, la nueva
    /// prueba posesión) y registra la sucesora en grafo + keystore. Handoff
    /// VOLUNTARIO sin compromiso — `current_key_at` seguirá la cadena hasta la
    /// nueva punta. La clave vieja debe vivir en el keystore local.
    Rotar {
        /// Id o prefijo hex de la identidad propia a rotar.
        id: String,
        /// Nombre legible para la sucesora (default: el de la madre).
        #[arg(long)]
        nombre: Option<String>,
        /// Lee la seed de la NUEVA clave de stdin (64 hex o 32 bytes raw) en vez
        /// de generarla con CSPRNG. Útil para sembrar la sucesora desde backup.
        #[arg(long)]
        seed_stdin: bool,
    },
    /// Revoca una identidad (plano SOCIAL): apaga su clave a partir de ahora.
    /// La autoridad son sus GUARDIANES declarados (`predicate="guardian"`) — una
    /// clave no se revoca a sí misma. Firma con cada guardián cuya seed viva en
    /// el keystore local; si no se alcanza el umbral con las locales, falla
    /// (la combinación multi-parte offline de firmas parciales queda pendiente).
    Revocar {
        /// Id o prefijo hex de la identidad a revocar.
        id: String,
        /// Motivo: compromised (permanente), retired o superseded.
        #[arg(long, value_enum, default_value_t = MotivoArg::Compromised)]
        motivo: MotivoArg,
        /// Umbral M-of-N exigido al set de guardianes. Default: la cantidad de
        /// guardianes con seed local (firma con todos los que tengo).
        #[arg(long)]
        umbral: Option<usize>,
        /// Suspensión TEMPORAL: segundos desde ahora hasta que la revocación
        /// vence (la clave vuelve a valer). Sin esto, la revocación es PERMANENTE.
        #[arg(long)]
        vence_en_seg: Option<u64>,
    },
}

// =============================================================================
//  Subcomandos de atestación
// =============================================================================

#[derive(Subcommand)]
pub enum AtestacionOp {
    /// Lista atestaciones del grafo local. Sin filtros, muestra todas
    /// (orden de inserción). Los filtros son AND.
    Listar {
        /// Sólo atestaciones cuyo `claim.subject` matchea ese id/prefijo.
        #[arg(long)]
        subject: Option<String>,
        /// Sólo atestaciones cuyo `attester` matchea ese id/prefijo.
        #[arg(long)]
        attester: Option<String>,
        /// Sólo claims con ese predicado exacto (case-sensitive).
        #[arg(long)]
        predicate: Option<String>,
    },
}

// =============================================================================
//  Subcomandos de canal
// =============================================================================

#[derive(Subcommand)]
pub enum CanalOp {
    /// Crea un canal nuevo (sin raíces) con `autor` como firmante.
    /// La pubkey de `autor` debe estar en el keystore local — sólo
    /// quien tiene la seed puede extender el canal después.
    Nuevo {
        #[arg(long)]
        nombre: String,
        #[arg(long)]
        autor: String,
        /// Archivo postcard a escribir.
        #[arg(long)]
        salida: PathBuf,
    },
    /// Agrega una RaizFirmada al final del historial del canal y
    /// re-escribe el archivo. El timestamp es ahora UNIX.
    Extender {
        #[arg(long)]
        archivo: PathBuf,
        /// Hash hex (64 chars) del manifiesto que la raíz inaugura.
        #[arg(long)]
        raiz: String,
    },
    /// Verifica firma + monotonicidad de timestamps del canal completo.
    Verificar {
        archivo: PathBuf,
    },
    /// Imprime el canal en formato legible.
    Mostrar {
        archivo: PathBuf,
    },
}

// =============================================================================
//  Subcomandos de wawa
// =============================================================================

#[derive(Subcommand)]
pub enum WawaOp {
    /// Forja un par Ed25519 nuevo, guarda la seed cifrada en el
    /// keystore y escribe la pubkey (32 B raw + 64 chars hex) a stdout.
    /// Útil para alimentar el AGORA_AUTH_RING de wawa-kernel en la
    /// ceremonia de la Fase 48: la pubkey va al binario, la seed queda
    /// offline en el HSM/USB del operador.
    ForjarClave {
        #[arg(long, default_value = "wawa-soberano")]
        name: String,
    },
    /// Fase 67 / WAWA §14.1.3 :: forja la CONCESIÓN DE CAPACIDAD de un binario.
    /// Firma con `--como` el par `(hash_objeto_bytecode, permisos)` y emite la
    /// `format::ConcesionCapacidad` envuelta en un `Objeto` del grafo (`<hash>.obj`),
    /// lista para sembrar en el génesis y referenciar desde `EntradaApp.concesion`.
    ///
    /// El hash que firma es el del OBJETO del grafo —`Objeto{datos:wasm,hijos:[]}`
    /// serializado, luego BLAKE3—, IDÉNTICO al que `wawa-boot::sembrar_grafo` y
    /// `construir_release` anclan. No es el hash de los bytes crudos del `.wasm`.
    ///
    /// Es la ceremonia OFFLINE que `boot` no puede hacer (no tiene seed). El
    /// operador la corre con la seed slot-0 del AGORA_AUTH_RING:
    ///
    ///   agora-cli wawa concesion --como wawa-soberano \
    ///     --wasm mudanza.wasm --permisos RAIZ --salida mudanza.cap.obj
    Concesion {
        /// Identidad firmante (seed en el keystore local). Su pubkey DEBE vivir
        /// en `AGORA_AUTH_RING` de claves.rs o el kernel rechaza la concesión.
        #[arg(long)]
        como: String,
        /// Path al `.wasm` compilado de la app (el mismo que sembra el génesis).
        #[arg(long)]
        wasm: PathBuf,
        /// Permisos a conceder: máscara decimal/hex (`0x4`) o nombres separados
        /// por coma (`RED,RAIZ`). Nombres: RED, GRAFO_ESCRITURA, RAIZ, ALTAVOZ,
        /// CONFIG, COMPACTAR, TINKUY.
        #[arg(long)]
        permisos: String,
        /// Archivo de salida del objeto-concesión (`<hash>.obj`, payload postcard).
        #[arg(long)]
        salida: PathBuf,
    },
    /// Toma un hash de manifiesto + una identidad propia y produce un
    /// `format::ManifiestoFirmado` postcard de 128 bytes, listo para
    /// embeber en `apps/mudanza/src/propuesta_demo.bin` o emitir por
    /// `MensajeAkasha::AnunciarCanal`.
    ForjarPropuesta {
        /// Identidad firmante (debe estar en el keystore local).
        #[arg(long)]
        como: String,
        /// Hash hex (64 chars) del manifiesto a anclar.
        #[arg(long)]
        hash: String,
        /// Archivo de salida con los 128 bytes raw.
        #[arg(long)]
        salida: PathBuf,
    },
    /// Fase 64 :: empaqueta un release de wawa COMPLETO a partir de un spec
    /// JSON que lista todas las apps (cada una con su `.wasm` compilado,
    /// región, fuel y permisos). Construye el grafo —objetos de bytecode +
    /// manifiesto + canal—, lo firma con `--como`, y escribe a `--salida/`:
    ///
    ///   - `<hash>.obj`              un archivo por objeto del grafo
    ///   - `anuncio.bin`            168 B: canal|raiz|autor|timestamp_le|firma
    ///   - `manifiesto_firmado.bin` 128 B: el sobre de `sys_manifiesto_proponer`
    ///
    /// El directorio resultante lo difunde y sirve por AoE el example
    /// `servir_release` de `wawa-explorer-aoe`. Es la mitad "fragua" del lazo
    /// Rust→wawa en vivo: compilás, esto empaqueta y firma, wawa absorbe.
    ///
    /// Spec JSON:
    ///   {"canal":"dev","apps":[
    ///     {"nombre":"hola","wasm":"app.wasm","region":[100,120,480,560],
    ///      "fuel":2000000,"permisos":0}]}
    Publicar {
        /// Identidad firmante (debe estar en el keystore local). Para que
        /// wawa la acepte, su pubkey debe vivir en `AGORA_AUTH_RING`.
        #[arg(long)]
        como: String,
        /// Path al spec JSON del release.
        #[arg(long)]
        spec: PathBuf,
        /// Directorio de salida (se crea si no existe).
        #[arg(long)]
        salida: PathBuf,
    },
    /// SDD-rotacion-revocacion §4/§5c :: forja el OVERLAY de revocación del plano
    /// de CONTROL: apaga una clave del `AGORA_AUTH_RING` sin re-forjar el kernel.
    /// Firma la revocación M-of-N con `--como` (varios miembros del anillo,
    /// separados por coma) sobre `mensaje_revocacion_clave(objetivo, motivo, ...)`
    /// y emite el `format::OverlayRevocacion` envuelto en un `Objeto` del grafo
    /// (`<hash>.obj`), listo para sembrar en los assets del génesis. El kernel lo
    /// lee al arrancar y deniega la clave revocada en `autor_en_anillo`.
    ///
    /// Una clave comprometida NO se revoca a sí misma: para `--motivo compromised`
    /// el `objetivo` NO debe figurar entre `--como`. El kernel exige 2-of-3.
    ///
    ///   agora-cli wawa revocar --objetivo <pubkey-hex-filtrada> \
    ///     --como wawa-secundario,wawa-recuperacion --salida overlay-revocacion.obj
    Revocar {
        /// Pubkey hex (64 chars) de la clave del anillo a revocar. Es una PUBKEY
        /// cruda, no un id del grafo: la clave soberana que se apaga.
        #[arg(long)]
        objetivo: String,
        /// Identidades firmantes (seeds en el keystore local), separadas por coma.
        /// Sus pubkeys DEBEN habitar `AGORA_AUTH_RING`; el kernel cuenta firmantes
        /// distintos del anillo y exige el quórum (2-of-3).
        #[arg(long)]
        como: String,
        /// Motivo: compromised (permanente), retired o superseded.
        #[arg(long, value_enum, default_value_t = MotivoArg::Compromised)]
        motivo: MotivoArg,
        /// Suspensión TEMPORAL: segundos desde ahora hasta vencer. Sin esto,
        /// permanente. (El kernel hoy la aplica fail-closed: la auto-caducidad
        /// temporal espera un RTC — ver SDD §4.)
        #[arg(long)]
        vence_en_seg: Option<u64>,
        /// Archivo de salida del objeto-overlay (`<hash>.obj`, payload postcard).
        #[arg(long)]
        salida: PathBuf,
    },
    /// Difunde el release de un directorio (el que produjo `publicar`) por
    /// Akasha-over-Ether y sirve sus objetos a las wawa de la misma red L2.
    /// Es la mitad "transporte" del lazo Rust→wawa: empaqueta el
    /// `MensajeAkasha::AnunciarCanal` firmado desde `anuncio.bin`, lo difunde
    /// en loop y atiende los `SolicitarObjeto` de los peers (fragmentando los
    /// objetos > 1024 B). REQUIERE CAP_NET_RAW o root para el raw socket:
    ///
    ///   sudo -E agora-cli wawa anunciar --iface eth0 --dir ./release
    ///
    /// Cortar con Ctrl-C cuando la wawa haya absorbido el release (su baliza
    /// serial lo confirma) y el operador haya aceptado en `mudanza`.
    Anunciar {
        /// Interfaz Ethernet por la que difundir (ej. `eth0`, `wlp3s0`).
        #[arg(long)]
        iface: String,
        /// Directorio del release (con `anuncio.bin` + los `<hash>.obj`).
        #[arg(long)]
        dir: PathBuf,
        /// Segundos a difundir+servir antes de terminar.
        #[arg(long, default_value_t = 120)]
        segundos: u64,
    },
    /// Fase 66 :: importa un directorio REAL al grafo direccionado por
    /// contenido (el monorepo como grafo). Cada archivo se vuelve un BLOB,
    /// cada subdirectorio un ÁRBOL (git-like). Escribe un `<hash>.obj` por
    /// objeto en `--salida/` + `raiz.txt` con el hash raíz. Archivos idénticos
    /// comparten un solo blob; el árbol entero colapsa a UN hash. El bundle
    /// resultante se sirve a wawa con `servir_release` (objetos grandes se
    /// fragmentan, Fase 65).
    Importar {
        /// Directorio a importar.
        #[arg(long)]
        dir: PathBuf,
        /// Directorio de salida del bundle (se crea si no existe).
        #[arg(long)]
        salida: PathBuf,
    },
    /// importa una IMAGEN DE DISPOSITIVO (USB/partición/imagen de disco) al
    /// grafo, leyendo su sistema de archivos SIN montar vía `foreign-fs`. Lee
    /// la tabla de particiones (GPT/MBR) o un FS suelto, autodetecta FAT vs
    /// ext2/3/4 en cada partición, y absorbe a `<hash>.obj` + `raiz.txt` igual
    /// que `importar` —pero desde bytes crudos, no desde un directorio montado—.
    /// Es la vía host para tragar el USB/partición vieja del usuario hacia un
    /// bundle servible por `servir_release`. (El gemelo in-cage corre dentro de
    /// wawa cuando exista el driver de bloque.)
    ///
    ///   - 1 partición reconocida  → la raíz es el árbol de ESE FS.
    ///   - varias                  → la raíz es un árbol `particionN/` por cada
    ///                               FS reconocido (las swap/desconocidas se omiten).
    ImportarImagen {
        /// Archivo de imagen del dispositivo (un disco entero o una partición).
        #[arg(long)]
        imagen: PathBuf,
        /// Directorio de salida del bundle (se crea si no existe).
        #[arg(long)]
        salida: PathBuf,
        /// Absorber sólo la partición de este slot 1-based de la tabla (en vez
        /// de todo el dispositivo).
        #[arg(long)]
        particion: Option<usize>,
    },
    /// Fase 66 :: exporta un árbol del grafo de vuelta al filesystem —el
    /// inverso de `importar`—. Reconstruye el directorio byte a byte desde los
    /// `<hash>.obj` del bundle, empezando por la raíz dada. Verifica el hash de
    /// cada objeto contra su nombre (integridad de punta a punta).
    Exportar {
        /// Directorio bundle con los `<hash>.obj`.
        #[arg(long)]
        bundle: PathBuf,
        /// Hash raíz del árbol a exportar (64 hex). Si se omite, se lee de
        /// `<bundle>/raiz.txt`.
        #[arg(long)]
        raiz: Option<String>,
        /// Directorio destino (se crea si no existe).
        #[arg(long)]
        destino: PathBuf,
    },
}

// =============================================================================
//  Enums de valor para clap
// =============================================================================

/// Motivo de revocación, espejo CLI de [`RevReason`].
#[derive(Clone, Copy, ValueEnum)]
pub enum MotivoArg {
    /// Clave filtrada / en manos hostiles — revocación permanente.
    Compromised,
    /// Retiro voluntario, sin compromiso.
    Retired,
    /// Reemplazada por una sucesora vía rotación.
    Superseded,
}

impl From<MotivoArg> for RevReason {
    fn from(m: MotivoArg) -> Self {
        match m {
            MotivoArg::Compromised => RevReason::Compromised,
            MotivoArg::Retired => RevReason::Retired,
            MotivoArg::Superseded => RevReason::Superseded,
        }
    }
}

#[derive(Clone, Copy, ValueEnum)]
pub enum KindArg {
    Person,
    Community,
    Alliance,
    Institution,
}

impl From<KindArg> for IdentityKind {
    fn from(k: KindArg) -> Self {
        match k {
            KindArg::Person => IdentityKind::Person,
            KindArg::Community => IdentityKind::Community,
            KindArg::Alliance => IdentityKind::Alliance,
            KindArg::Institution => IdentityKind::Institution,
        }
    }
}
