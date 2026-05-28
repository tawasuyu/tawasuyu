# agora

> Plaza pública. Identidad federada y un grafo de confianza que no administra nadie.

`agora` es la capa de identidad del monorepo. Cada identidad — persona, comunidad, alianza, institución — es una clave pública ed25519. Cada afirmación sobre ella es un `Claim`. Cada respaldo es una `Attestation`: un claim firmado por un atestador. La verdad del ágora no la dicta un servidor: emerge de quién atestigua qué, pesado por una `TrustPolicy` que cada lector negocia para sí.

No hay registro maestro, no hay algoritmo de moderación, no hay feed. La misma forma — pubkey + atestaciones firmadas — cubre desde una persona hasta una institución, pasando por comunidades y alianzas.

## Instalación

```sh
cargo run --release -p agora-app
```

## Compatibilidad

- **Linux / macOS / Windows / Wawa** — todos los crates son puro Rust.
- Persistencia local; convergencia opcional con peers de la malla `minga`.

## Crates

| Crate | Rol |
|---|---|
| [`agora-core`](agora-core/LEEME.md) | Identidades, claims, atestaciones firmadas ed25519. |
| [`agora-graph`](agora-graph/LEEME.md) | TrustGraph: atestaciones verificadas + corroboración + política negociada. |
| [`agora-store`](agora-store/LEEME.md) | Persistencia JSON atómica con re-verificación al cargar. |
| [`agora-keystore`](agora-keystore/LEEME.md) | Almacén cifrado de seeds privadas (Argon2 + ChaCha20-Poly1305). |
| [`agora-gossip`](agora-gossip/LEEME.md) | Protocolo anti-entropy transport-agnóstico sobre atestaciones firmadas. |
| [`agora-net-brahman`](agora-net-brahman/LEEME.md) | Puente libp2p: registra `/agora/gossip/1.0.0` sobre `BrahmanNet` (compartido con minga). |
| [`agora-app`](agora-app/LEEME.md) | UI Llimphi: identidades, atestaciones, compositor, política. |

## Consideraciones

- **Identidad por clave pública**, nunca email ni teléfono.
- El grafo guarda sólo **evidencia comprobable**: una atestación con firma rota se rechaza al ingresar.
- El veredicto sobre un claim no es una propiedad del grafo — la `TrustPolicy` se **negocia** por lector, y dos lectores pueden discrepar legítimamente sobre la misma evidencia.
- La auto-atestación se preserva pero queda marcada aparte del respaldo de terceros.
- Compatible con `minga`: cuando ambos están activos, ágora viaja sobre el mismo nodo `BrahmanNet` (un PeerId, una tabla Kademlia, dos sub-protocolos de stream).
