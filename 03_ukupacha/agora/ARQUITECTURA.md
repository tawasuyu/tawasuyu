# ARQUITECTURA.md — agora

> Descripción técnico-arquitectónica densa, optimizada para consumo por IA.
> Snapshot: 2026-05-30. Fuente autoritativa cuando difiera con prosa de READMEs.

```yaml
DOMINIO: agora
CUADRANTE: 03_ukupacha (RAÍZ)
TESIS: plaza pública federada — confianza sin autoridad central, sin registro, sin moderación
PARADIGMA: web-of-trust criptográfica donde el VEREDICTO no es propiedad del grafo sino del LECTOR
TAMAÑO: 9 crates, ~8.3 KLoC puro Rust
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

ASPIRA_A (pendientes, orden valor/riesgo):
  #9  ROTACIÓN de clave: atestación "cambio-de-clave: vieja→nueva" firmada por AMBAS pubkeys
      (hoy: seed comprometida ⇒ abandonar identidad — sin continuidad)
  #10 REVOCACIÓN: distinguir "no atestiguó hoy" de "fue comprometida" — el TrustGraph aún NO lo modela
  #1  DAEMON mudanza completo: faltan sys_red_recibir(filtro) + sys_grafo_pedir(hash) en kernel + UI aceptar/rechazar
  #2  BRIDGE Akasha-over-Ether ↔ host: crate nuevo agora-akasha-bridge (raw socket + EtherType propio)
  WAWA.md §14.1.3: TABLA DE CAPACIDADES POR HASH-DE-BYTECODE
      — derivar permisos de firma(hash_bytecode, permisos) en vez de declararlos en EntradaApp
      — frontera física de capacidades, no tabla de permisos

NORTE_ARQUITECTÓNICO:
  agora pasa de "red de confianza entre personas" a "raíz de confianza ejecutable de un SO soberano":
  identidad → atestación → política-por-lector → release firmado → capacidad derivada de firma → ejecución gateada en kernel.
  El mismo Ed25519 que dice "esta persona es quién dice" termina diciendo "este bytecode puede tocar la red".
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

**Síntesis de una línea para otra IA:** agora es una web-of-trust Ed25519 fractal y
transport-agnóstica donde el veredicto se computa por-lector vía `TrustPolicy` negociada
(no por consenso global), construida en capas puras (cripto → grafo → gossip anti-entropía →
persistencia → libp2p), que aspira a convertirse en la **raíz de confianza ejecutable** del SO
bare-metal wawa — propagando releases firmados por AoE y, en su norte, derivando capacidades de
ejecución directamente de firmas sobre `(hash_bytecode, permisos)`.
