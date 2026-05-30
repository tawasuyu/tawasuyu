# ARQUITECTURA.md — agora

> Descripción técnico-arquitectónica densa, optimizada para consumo por IA.
> Snapshot: 2026-05-30. Fuente autoritativa cuando difiera con prosa de READMEs.

```yaml
DOMINIO: agora
CUADRANTE: 03_ukupacha (RAÍZ)
TESIS: raíz de confianza EJECUTABLE de un SO soberano. Responde dos preguntas del kernel wawa:
       (1) ¿quién puede empujar una imagen del sistema?  (2) ¿qué bytecode puede tocar el hardware?
       NO es una red social ni una "plaza pública" a escala humanidad — eso es el sustrato, no el producto.
PARADIGMA: firma Ed25519 que viaja CON el dato (release / concesión), verificada idéntica en host (dalek)
           y en bare-metal (compact). El veredicto se computa por-LECTOR vía TrustPolicy; en el plano de
           control el "lector" es el kernel y el set de firmantes es CURADO (anillo), no abierto.
TAMAÑO: 9 crates, ~8.6 KLoC puro Rust
```

## Encuadre honesto (límites duros — leer antes de soñar escala)

```
LÍMITE-1  IdentityId = BLAKE3(pubkey). Crear identidad = generar un keypair. COSTO CERO.
          No hay PoW, stake, ni puzzle (verificado: grep vacío en agora-core/agora-graph).
LÍMITE-2  third_party() = atestadores_distintos − self. Luego `min_third_party = N` se DERROTA
          generando N claves. En un grafo ABIERTO no es defensa anti-Sybil; es decoración —
          a menos que los atestadores estén ellos mismos vetados (el problema que la WoT nunca resolvió).
LÍMITE-3  La WoT a escala masiva tiene 30 años de cementerio (PGP). La causa no fue la cripto:
          fue el bootstrap de confianza + la carga cognitiva del veredicto-por-lector. agora hereda
          ESE fracaso si se vende como red social. No se "arregla" — se vuelve IRRELEVANTE en el
          encuadre de abajo.
CONSECUENCIA  El uso PORTANTE no es humano y no es abierto: es el plano de control de wawa, donde el
              set de firmantes lo controlás vos (AGORA_AUTH_RING) y el "consenso" es la intersección
              que toma el kernel. Ahí Sybil no existe porque no hay registro abierto que sybilear.
              La maquinaria por-lector (TrustPolicy, identidad fractal, gossip) queda intacta en el
              código y sirve igual; lo que cambia es a QUÉ apuntamos con ella.
```

## Modelo conceptual (la idea-fuerza)

```
PRINCIPIO_CENTRAL:
  El grafo almacena EVIDENCIA (atestaciones firmadas), no VERDAD.
  La verdad se computa por-lector vía TrustPolicy negociada.
  Dos lectores honestos pueden discrepar legítimamente sobre la misma evidencia.
  => No hay "fuente autoritativa" de quién-es-quién. Hay corroboración medible.

IDENTIDAD_FRACTAL:
  Person | Community | Alliance | Institution comparten ESTRUCTURA IDÉNTICA (1:1).
  Una comunidad atestigua igual que una persona. Recursión sin casos especiales.
  IdentityId = BLAKE3(pubkey_ed25519)  // la identidad ES su clave, no un registro
```

## Cadena de tipos (núcleo)

```rust
// agora-core (810 LOC) — cripto pura, sin red, sin estado global
IdentityId = BLAKE3(pubkey: [u8;32])
Claim      = { subject: IdentityId, predicate: String, value: String, issued_at: u64 }
             canonical_bytes = "agora_app-claim\x01" || subject || issued_at_LE
                               || len(pred)_LE || pred || len(val)_LE || val   // prefix-len, sin ambigüedad
Attestation= { claim, attester: IdentityId, attester_key: [u8;32], signature: [u8;64] }
             verify() := attester == BLAKE3(attester_key) ∧ ed25519_verify(key, canonical, sig)
             stable_hash() := BLAKE3(canonical || attester_key || signature)  // DETERMINISTA → gossip converge
MultiSignature = { M: usize, firmas: Vec<SingleSig> }  // umbral M-of-N
```

## Invariantes duras (un verificador puede asumirlas)

```
INV-1  add_attestation() RE-VERIFICA la firma; firma rota ⇒ Err. El grafo nunca contiene basura no-comprobable.
INV-2  load() reconstruye llamando add_attestation() 1×1 ⇒ re-verifica TODO al cargar de disco.
INV-3  ed25519-dalek determinista (sin RNG feature) ⇒ misma (seed,claim) ⇒ misma firma ⇒ mismo stable_hash en toda máquina.
INV-4  nombre_de_canal ENTRA en mensaje_a_firmar() ⇒ firma válida en canal "dev" NO valida en "estable" (anti-replay cross-canal).
INV-5  seed Ed25519 jamás se serializa en claro: Argon2id(m=19456,t=2,p=1) → ChaCha20-Poly1305, blob de 88B exactos.
INV-6  veredicto = TrustPolicy.evaluate(corroboration) — función del LECTOR, no del dato. is_accepted ≠ propiedad global.
```

## Capas (orden de dependencia, no de runtime)

```
L0 cripto pura ......... agora-core      identidad fractal · claim · attestation · multisig
L1 grafo + política .... agora-graph     TrustGraph (solo verificado) · Corroboration · TrustPolicy(min_third_party,
                                          accept_self, min_attesters_of_kind, max_age_secs)
L2 transporte .......... agora-gossip    anti-entropy SIN IO: Announce(Digest)→Request(Δhash)→Bundle(att)→[verify]; +Pull
L3 persistencia ........ agora-keystore  seeds cifradas (Argon2id+ChaCha20)  | agora-store  JSON|postcard + append-log + replay
L4 red ................. agora-net-brahman  /agora/gossip/1.0.0 sobre BrahmanNet libp2p; comparte PeerId+Kademlia con minga
L5 interop wawa ........ agora-channel   firma/verifica RaizFirmada · ManifiestoFirmado · ConcesionCapacidad(bytecode,permisos)
L6 interfaz ............ agora-cli (1637) · agora-app (1923, 4 tiles Llimphi draggables + watcher disco)
```

## Protocolo de propagación (gossip = anti-entropía, no flood)

```
Digest = BTreeSet<stable_hash>          // ordenado, sin duplicados, idempotente
PUSH:  A→B Announce(haves_A) ; B→A Request(haves_A∖haves_B) ; A→B Bundle(those) ; B integra+verifica
PULL:  B→A Pull ; A→B Announce(...) ; sigue flujo PUSH
PROPIEDAD: convergente, sin estado de sesión, sin firma nueva en tránsito (la firma viaja DENTRO de la atestación).
WIRE (brahman): u32_LE len || postcard(Message), cap 16MB; rate-limit token-bucket opt-in por peer.
```

## Puente hacia wawa (donde agora deja de ser "social" y se vuelve "sistema")

```
agora-channel produce el sobre que el KERNEL wawa valida:
  firmar_manifiesto(kp, hash) → ManifiestoFirmado
  → AoE broadcast: MensajeAkasha::AnunciarCanal{canal,raiz,autor,timestamp,firma} (anuncio.bin = 168B)
  → app WASM `mudanza` en wawa recibe, verifica, llama sys_manifiesto_proponer (gateado por AGORA_AUTH_RING en .rodata)
  → kernel re-ancla SuperBloque.manifiesto

ESPEJO KERNEL (zero-alloc, ed25519-compact en wawa-kernel/src/claves.rs):
  userspace agora-channel::verificar_{raiz,canal,manifiesto}
    ⟷ kernel verificar_{manifiesto_firmado,anuncio_canal,cuaderno_firmado}
  La MISMA firma se valida en host (dalek) y en bare-metal (compact). format/* cruza la frontera no_std.

=> agora es el plano de control de IDENTIDAD y RELEASE del SO wawa: quién puede empujar una nueva imagen del sistema.
```

## CLI host-side (`agora-cli wawa`)

```
wawa forjar-clave   → genera/guarda seed Ed25519 en keystore
wawa publicar       → firma manifiesto + emite anuncio.bin (168B) + <hash>.obj del DAG
wawa anunciar       → emisión AoE L2: broadcast periódico de AnunciarCanal por --iface,
                       sirve SolicitarObjeto(hash) desde <hash>.obj (fragmenta >1024B), durante --segundos.
                       Reusa wawa-explorer-aoe::ClienteAoE (raw socket, requiere CAP_NET_RAW).
```

## Estado vs aspiración (vector de evolución)

```
HECHO (A→T cerrado 2026-05-28..29):
  ✓ identidad+keystore+grafo+store(append-log+postcard) · TrustPolicy multi-eje · gossip bidireccional+rate-limit
  ✓ bridge canal wawa + mudanza con verify real · UI Llimphi tiles+watcher · CLI prefix-match · multisig M-of-N
  ✓ wawa host-side: agora-cli wawa {forjar-clave, publicar, anunciar(AoE L2)}

YA HECHO que el snapshot viejo daba por pendiente (verificado en disco 2026-05-30):
  ✓ PRIMITIVOS de capacidad por hash-de-bytecode: agora-channel::{firmar_capacidad, verificar_capacidad}
    firman/verifican la firma Ed25519 sobre format::mensaje_capacidad(bytecode, permisos)=[u8;36].
  ✓ ESPEJO KERNEL de la concesión: wawa-kernel/src/claves.rs::verificar_concesion_capacidad (zero-alloc).
  ✓ format::ConcesionCapacidad + permisos_efectivos (intersección manifiesto ∩ concesión).
  => §14.1.3 NO es "tabla por construir": el sobre y los dos verificadores existen. Lo que falta es el CABLEADO.

ASPIRA_A (pendientes, REORDENADO por valor desbloqueado — no por riesgo teórico):
  #1  CABLEAR §14.1.3 — HECHO (2026-05-30, commits 98bc98bc + e27fade9). El kernel deriva permisos
      efectivos por intersección manifiesto ∩ ConcesionCapacidad verificada en el punto de carga; boot
      ancla las concesiones offline (*.cap.obj) del génesis. Rollout escalonado: None⇒declarados hasta
      que el operador provisione. Convierte agora de "mensajería de confianza" en "cargador de capacidades".
  #2  DAEMON mudanza completo — HECHO. El snapshot era stale: recepción AoE + pull DAG + aceptación
      soberana ya existían (Fase 64/65: sys_net_recibir, sys_red_solicitar, sys_canal_{anuncio,aceptar}).
      Cerrado el bucle aceptar/rechazar (commit 330f5a05): sys_canal_descartar (ESC en mudanza).
  #3  BRIDGE Akasha-over-Ether ↔ host — EN GRAN PARTE HECHO. El "crate nuevo" era redundante: emit+serve
      sobre raw socket YA viven en wawa-explorer-aoe::ClienteAoE (anunciar_canal/servir/solicitar,
      AF_PACKET, fragmenta >1024B) y `agora-cli wawa anunciar` los orquesta en loop. El hueco real era el
      TRANSPORTE host↔guest: el NAT user-mode de QEMU solo reenvía IP, NO el EtherType 0x88B5. CERRADO
      (2026-05-30): boot::lanzar_qemu honra RENASER_TAP=<iface> y bridgea la NIC del guest a un TAP de
      capa-2. scripts/aoe-tap-setup.sh forja el tap; scripts/test-mudanza-aoe-qemu.sh corre el E2E vivo.
      Dos máquinas con un cable se instalan SO mutuamente sin IP/DHCP/DNS — el demo que se ve y se entiende.
  #4  ROTACIÓN / REVOCACIÓN de clave (juntas): atestación "cambio-de-clave: vieja→nueva" firmada por
      AMBAS pubkeys + vector de revocación con caducidad estricta. Necesarias ANTES de que exista un set
      de releasers > 1, no antes. Hoy sos vos con una seed; si se compromete, regenerás.

NORTE_ARQUITECTÓNICO:
  agora NO es "red de confianza entre personas". Es la raíz de confianza ejecutable de un SO soberano:
  identidad → release firmado → capacidad derivada de firma sobre (hash_bytecode, permisos) → ejecución
  gateada en kernel por intersección. El mismo Ed25519 que podría decir "esta persona es quién dice"
  acá dice algo más duro y verificable: "este bytecode exacto puede tocar la red". El grafo social y la
  TrustPolicy por-lector son el sustrato de bootstrap; el producto es el plano de control de wawa.
```

## Relaciones (mapa de dependencias inter-dominio)

```
chasqui  : transporte P2P de mensajes (agora puede viajar encima)
minga    : VFS P2P content-addressed; COMPARTE el nodo libp2p (BrahmanNet) y la Kademlia DHT con agora
akasha   : protocolo L2 EtherType-propio sin TCP/IP; transporta AnunciarCanal hacia wawa
format   : tipos no_std compartidos host↔kernel (Canal, *Firmado, AgoraId, Firma, Permisos, MensajeAkasha)
wawa     : consumidor final — agora es su plano de control de identidad/release/capacidad
```

---

**Síntesis de una línea para otra IA:** agora es la **raíz de confianza ejecutable** del SO bare-metal
wawa — su plano de control de identidad/release/capacidad. La maquinaria es una WoT Ed25519 fractal,
transport-agnóstica y por-lector (`TrustPolicy`, no consenso global), construida en capas puras
(cripto → grafo → gossip anti-entropía → persistencia → libp2p); pero su uso PORTANTE no es social a
escala humanidad (eso es WoT abierta y Sybil-vulnerable: identidad gratis ⇒ `min_third_party` no defiende):
es firmar releases (propagados por AoE) y conceder capacidades por `(hash_bytecode, permisos)` que el
kernel honra por intersección contra un anillo CURADO de firmantes. Los primitivos host+kernel ya existen
(`firmar/verificar_capacidad` ⟷ `verificar_concesion_capacidad`); lo pendiente es cablear la derivación
de permisos efectivos en el enlace de capacidades del kernel.
