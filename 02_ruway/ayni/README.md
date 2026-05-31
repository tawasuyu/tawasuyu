# ayni

> `ayni` (quechua: *reciprocidad*). Chat persona-a-persona soberano, local-first,
> sin servidor. La conversación tratada como un grafo criptográfico reproducible
> (BLAKE3 + DAG + postcard), identidad `agora` Ed25519, transporte por
> `chasqui`/`minga`/`akasha`.

Documento de diseño y tesis completos en [LEEME.md](LEEME.md). Esta página es
el resumen de estado vivo.

## Crates

| Crate | Rol |
|---|---|
| `ayni-core` | DAG de mensajes firmados + membresía/confianza/recibos (`no_std`+alloc). |
| `ayni-crypto` | firma Ed25519 sobre agora + E2EE 1:1 (X25519/HKDF/ChaCha20-Poly1305). |
| `ayni-sync` | trait `Transporte` + `EnlaceTcp` + anti-entropía (diff Merkle). |
| `ayni-minga` | `EnlaceMinga`: transporte P2P sobre libp2p. |
| `ayni-store` | persistencia del DAG + blobs de adjuntos (dedup) sobre sled. |
| `ayni-app` | núcleo de aplicación: transporte + store + cifrado + adjuntos + confianza. |
| `ayni-cli` | chat de terminal (bin `ayni`), frontend delgado sobre `ayni-app`. |
| `ayni-llimphi` | UI Llimphi: charla + gente + adjuntos + recibos. |
| `ayni-index` | búsqueda semántica local (rimay embeddings + coseno). |
| `ayni-ai` | multilienzo: traducir/resumir/tono vía `pluma-llm`. |

## Instalación

```sh
cargo run --release -p ayni-cli       # chat de terminal
cargo run --release -p ayni-llimphi   # UI gráfica
```

## Estado (2026-05-31)

### Hecho

- **P0–P7 cerradas** (ver LEEME.md fase a fase). `ayni-core` con DAG firmado,
  orden topológico determinista y 17 tests verdes (membresía/confianza/recibos
  incluidos).
- **E2EE 1:1** (P2): `CanalSeguro` X25519 + HKDF-SHA256 + ChaCha20-Poly1305, par
  derivado de la misma semilla agora; el cable sólo ve ciphertext.
- **Sin servidor** (P3): anti-entropía por diff de Merkle, persistencia sled, y
  `EnlaceMinga` (transporte P2P real sobre libp2p) tras el mismo trait `Transporte`.
- **Inteligencia local** (P4): `ayni-index` (búsqueda coseno) + `ayni-ai`
  (multilienzo traducir/resumir/tono, Mock determinista sin credenciales).
- **Cross-app** (P5): `Carga::Adjunto` como referencia viva por hash, blobs
  deduplicados y verificados por contenido.
- **Ayni en wawa** (P6/P6+): el mismo `ayni-core` corre como app WASM en el SO
  bare-metal; persiste la conversación en el grafo de objetos akasha y la difunde
  por la red propia del SO (EtherType `0x88B7`, sin TCP/IP) con verificación de
  firma al recibir.
- **`ayni-app` + UI completa**: transporte intercambiable (`--transporte tcp|minga`),
  store local-first, adjuntar con UX, recibos simétricos; GUI de dos columnas
  (gente/charla) con grafo de confianza.
- **Menús** (lote 1): menú principal + menús contextuales en la UI Llimphi.

### Pendiente

- **MLS de grupo** (RFC 9420 / OpenMLS): chat de grupo con forward + post-compromise
  secrecy. `CanalSeguro` es el seam donde entrará; el canal de hoy es 1:1 sin PCS.
- **NAT traversal**: deuda de `minga`, no de ayni (hoy TCP directo + DHT en LAN).
- **En la app de wawa**: anti-entropía completa sobre L2 (hoy se ve lo nuevo en
  vivo, falta reconciliar historial) y cifrado de sesión.
