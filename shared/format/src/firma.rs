use super::*;

// =============================================================================
//  Fase 67 :: la CONCESION DE CAPACIDAD — "que binario puede hacer que", firmado
// -----------------------------------------------------------------------------
//  Hoy los permisos de una app viven en su `EntradaApp` del manifiesto: el
//  manifiesto firmado dice "el bytecode X corre con permisos P". El binding es
//  tan fuerte como el manifiesto — re-firmar un manifiesto nuevo basta para
//  darle al MISMO binario permisos distintos. La concesion eleva ese binding a
//  un hecho INDEPENDIENTE del manifiesto: una firma Ed25519 de una llave del
//  `AGORA_AUTH_RING` sobre el par `(hash_bytecode, permisos)`. La firma viaja
//  con el binario y NINGUN manifiesto puede escalar un binario mas alla de lo
//  que su concesion autoriza —el kernel toma la INTERSECCION, ver
//  [`permisos_efectivos`]—. Gemelo estructural de [`ManifiestoFirmado`]: la
//  verificacion comparte el camino Ring 0 zero-alloc de `ed25519-compact`, pero
//  el mensaje firmado es [`mensaje_capacidad`], no el hash pelado.
// =============================================================================

/// Una concesion de capacidad firmada: liga inmutablemente un bytecode (por su
/// hash BLAKE3) a un bitfield de permisos, respaldada por la firma de una
/// identidad soberana. Es un objeto del grafo (direccionado por contenido) que
/// un `EntradaApp` referencia; el kernel la verifica contra el `AGORA_AUTH_RING`
/// antes de enlazar capacidad alguna.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct ConcesionCapacidad {
    /// Hash BLAKE3 del objeto-bytecode WASM al que esta concesion aplica. La
    /// firma lo cubre: una concesion para el bytecode X jamas vale para Y.
    pub bytecode: Hash,
    /// Bitfield de permisos que esta concesion AUTORIZA para ese bytecode (ver
    /// [`Permisos`] y las constantes `PERMISO_*`). Subir un bit invalida la firma.
    pub permisos: Permisos,
    /// Llave publica Ed25519 de quien concede. El kernel exige que habite el
    /// `AGORA_AUTH_RING` antes de gastar un ciclo en criptografia.
    pub autor: AgoraId,
    /// Firma Ed25519 sobre [`mensaje_capacidad`]`(bytecode, permisos)`.
    #[serde(with = "BigArray")]
    pub firma: Firma,
}

impl ConcesionCapacidad {
    /// Serializa la concesion a `postcard` — la carga util del objeto del grafo
    /// que la aloja.
    pub fn serializar(&self) -> Result<Vec<u8>, &'static str> {
        postcard::to_allocvec(self)
            .map_err(|_| "concesion_capacidad :: serializacion fallida")
    }

    /// Reconstruye una concesion desde su forma binaria. Tolera bytes
    /// sobrantes tras la estructura — el relleno del registro.
    pub fn deserializar(bytes: &[u8]) -> Result<ConcesionCapacidad, &'static str> {
        postcard::take_from_bytes::<ConcesionCapacidad>(bytes)
            .map(|(c, _)| c)
            .map_err(|_| "concesion_capacidad :: deserializacion fallida")
    }
}

// =============================================================================
//  Overlay de revocacion del plano de CONTROL (SDD-rotacion-revocacion §4)
// -----------------------------------------------------------------------------
//  El AGORA_AUTH_RING del kernel es `const` en `.rodata`: rotar el ancla = reflash
//  deliberado. Pero entre reflasheos una clave soberana puede filtrarse, y esperar
//  al re-forjado deja una ventana abierta. El overlay la cierra: un objeto del
//  grafo, anclado por el manifiesto (`Manifiesto::overlay_revocacion`), que lista
//  revocaciones firmadas M-of-N por el RESTO del anillo. El kernel lo lee FRESH en
//  el arranque y deniega en `autor_en_anillo` toda clave del anillo revocada.
//
//  Tipos `no_std + alloc`: el kernel los deserializa (postcard) y los verifica con
//  `claves::verificar_revocacion` sobre el canonico de `mensaje_revocacion_clave`.
//  El productor host-side (`agora-cli wawa revocar`) emite el mismo wire.
//
//  TIEMPO: el kernel hoy lleva ticks PIT, no wall-clock. Aplica la revocacion
//  mientras este ANCLADA (fail-closed, deny-wins); `vence_en` entra en el canonico
//  firmado pero la auto-caducidad temporal espera un RTC. Des-revocar = anclar un
//  overlay nuevo sin esa entrada (gemelo de mover el puntero de `configuracion`).
// =============================================================================

/// Version del format del [`OverlayRevocacion`] serializado.
pub const VERSION_OVERLAY: u32 = 1;

/// Una firma individual dentro de una [`RevocacionFirmada`]: la pubkey del
/// firmante y su firma Ed25519 sobre el canonico de la revocacion. Espejo
/// minimo de `agora_core::SingleSig` (sin el `IdentityId` redundante: el kernel
/// re-deriva la autoridad comparando la pubkey contra el `AGORA_AUTH_RING`).
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct FirmaRevocacion {
    /// Clave publica Ed25519 del firmante (un miembro del anillo, en control).
    pub autor: AgoraId,
    /// Firma Ed25519 sobre [`mensaje_revocacion_clave`]`(objetivo, motivo,
    /// emitida_en, vence_en)`.
    #[serde(with = "BigArray")]
    pub firma: Firma,
}

/// Una revocacion de clave firmada por un quorum, en forma de wire para el
/// overlay. Espejo de `agora_core::Revocation` aplanado para el kernel: el
/// `motivo` es el discriminante estable de `RevReason` (0=Compromised,
/// 1=Retired, 2=Superseded) y `firmantes` es la multifirma desnuda.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct RevocacionFirmada {
    /// La clave que se revoca (en el plano de control, una del anillo).
    pub objetivo: AgoraId,
    /// Motivo (entra en el canonico firmado): 0=Compromised, 1=Retired, 2=Superseded.
    pub motivo: u8,
    /// Segundos UNIX desde cuando rige.
    pub emitida_en: u64,
    /// `None` ⇒ permanente; `Some(t)` ⇒ suspension hasta `t` (auto-caducidad
    /// pendiente de RTC en el kernel — ver nota de tiempo arriba).
    pub vence_en: Option<u64>,
    /// Las firmas del quorum autorizador.
    pub firmantes: Vec<FirmaRevocacion>,
}

/// El overlay de revocacion: la lista de revocaciones que el kernel consulta al
/// arrancar. Objeto del grafo direccionado por contenido; el manifiesto guarda
/// su hash en `overlay_revocacion`.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug, Default)]
pub struct OverlayRevocacion {
    /// Version del format — debe ser [`VERSION_OVERLAY`].
    pub version: u32,
    /// Las revocaciones vigentes. El kernel aplica las que apunten a un slot del
    /// anillo y reunan el quorum; ignora el resto (no son su jurisdiccion).
    pub revocaciones: Vec<RevocacionFirmada>,
}

impl OverlayRevocacion {
    /// Serializa el overlay a `postcard` — la carga util del objeto del grafo
    /// que lo aloja.
    pub fn serializar(&self) -> Result<Vec<u8>, &'static str> {
        postcard::to_allocvec(self).map_err(|_| "overlay_revocacion :: serializacion fallida")
    }

    /// Reconstruye un overlay desde su forma binaria. Rechaza una version de
    /// format desconocida en lugar de malinterpretarla. Tolera bytes sobrantes
    /// tras la estructura — el relleno del registro.
    pub fn deserializar(bytes: &[u8]) -> Result<OverlayRevocacion, &'static str> {
        let (overlay, _) = postcard::take_from_bytes::<OverlayRevocacion>(bytes)
            .map_err(|_| "overlay_revocacion :: deserializacion fallida")?;
        if overlay.version != VERSION_OVERLAY {
            return Err("overlay_revocacion :: version de format desconocida");
        }
        Ok(overlay)
    }
}

// =============================================================================
//  Fase 37 :: el sello criptografico del CUADERNO SOBERANO
// -----------------------------------------------------------------------------
//  La integridad de un cuaderno —un nodo del grafo cuyo payload es
//  `Vec<CeldaWawa>` (Fase 43, modelo unificado)— se proteje en dos planos:
//
//    * Localmente, el direccionamiento por contenido garantiza que un
//      bit alterado en cualquier celda cambia el hash del cuaderno
//      —y ese hash es la identidad del nodo en el almacen—.
//    * En la red capa-2 (Akasha), eso no basta: un peer hostil puede
//      reescribir el cuaderno entero y reanunciarlo con su propio hash.
//      Para que el sistema reconozca un cuaderno como SOBERANO del
//      operador local, el peer ha de adjuntar una firma Ed25519 del
//      cuaderno_raiz_hash producida con la clave privada que pertenece
//      a la `AGORA_PUBLIC_KEY_LOCAL` empotrada en el binario del kernel.
//
//  Gemelo estructural de `ManifiestoFirmado`: la verificacion comparte
//  el camino Ring 0 zero-alloc de `ed25519-compact`.
// =============================================================================

/// Sobre criptografico de un cuaderno: vincula su `hash` con un autor y
/// una firma Ed25519. Sin este sobre, un cuaderno es solo un nodo mas
/// del grafo — con el sobre, queda anclado como SOBERANO al usuario
/// que firmo, y el kernel lo distingue de cualquier otro nodo cuaderno
/// que viaje por la red.
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct CuadernoFirmado {
    /// Hash BLAKE3 del cuaderno propuesto. El payload del cuaderno es
    /// `Vec<CeldaWawa>` serializado con postcard; este hash es el
    /// resumen criptografico que va a engrapar la firma.
    pub cuaderno_raiz_hash: Hash,
    /// Llave publica Ed25519 del autor. El kernel la compara contra
    /// `AGORA_PUBLIC_KEY_LOCAL` antes de gastar un ciclo en criptografia
    /// — un autor ajeno cae con `CapacidadInsuficiente`.
    pub autor: AgoraId,
    /// Firma Ed25519 sobre los 32 bytes de `cuaderno_raiz_hash`.
    #[serde(with = "BigArray")]
    pub firma: Firma,
}

impl CuadernoFirmado {
    /// Serializa el sobre a su forma binaria `postcard`. La forma cruda
    /// ocupa 32 + 32 + 64 = 128 bytes; postcard agrega un preludio
    /// minusculo (longitudes varint) que mantiene el sobre bajo 140 B.
    pub fn serializar(&self) -> Result<Vec<u8>, &'static str> {
        postcard::to_allocvec(self)
            .map_err(|_| "cuaderno_firmado :: serializacion fallida")
    }

    /// Reconstruye un sobre desde su forma binaria. Tolera bytes
    /// sobrantes tras la estructura — el relleno del registro o el
    /// padding del payload del syscall.
    pub fn deserializar(bytes: &[u8]) -> Result<CuadernoFirmado, &'static str> {
        postcard::take_from_bytes::<CuadernoFirmado>(bytes)
            .map(|(cf, _)| cf)
            .map_err(|_| "cuaderno_firmado :: deserializacion fallida")
    }
}

/// Una entrada del historial de un canal: una raiz de manifiesto, el instante
/// en que el autor la propuso, y la firma Ed25519 con la que el autor la
/// respalda. La firma se calcula sobre [`mensaje_a_firmar`].
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq, Debug)]
pub struct RaizFirmada {
    /// Instante en que el autor propuso esta raiz, segundos desde UNIX epoch.
    /// Un receptor desconfia de raices con timestamp futuro mas alla de un
    /// margen razonable —proteccion barata contra anuncios envenenados—.
    pub timestamp: u64,
    /// El hash del [`Manifiesto`] que esta raiz inaugura. Re-anclar el
    /// superbloque a este hash es, literalmente, "actualizar a esta version".
    pub raiz_manifiesto: Hash,
    /// La firma Ed25519 del autor del canal sobre [`mensaje_a_firmar`].
    /// `serde` no derivara `Deserialize` para `[u8; 64]` sin ayuda —su soporte
    /// directo se detiene en 32 bytes—; `serde-big-array` cierra ese hueco.
    #[serde(with = "BigArray")]
    pub firma: Firma,
}

