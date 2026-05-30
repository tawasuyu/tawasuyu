# Aplicaciones de agora — orden de prioridad

Catálogo de aplicaciones **útiles ahora** sobre las primitivas que `agora` ya
expone (no roadmap especulativo). El criterio de orden combina dos ejes:

1. **Utilidad presente** — resuelve un problema real hoy, no "cuando se publique".
2. **Apalancamiento** — cuánto del código necesario ya existe y compila.

Cada entrada marca su pista: **[T]** técnica/infraestructura, **[S]** social/organizativa.

## Primitivas disponibles hoy

- **Identidad Ed25519 + atestaciones firmadas + grafo de confianza sin servidor**
  (`agora-core`, `agora-graph`, `agora-keystore` cifrado con Argon2id+ChaCha20).
- **Identidad fractal**: `IdentityKind::{Person, Community, Alliance, Institution}`
  — la organización anida por diseño, no por convención.
- **Multifirma M-of-N con whitelist**: `MultiSignature::verify_threshold_in`.
- **Canal append-only firmado** con timestamps estrictamente monótonos:
  `verificar_canal` (libro mayor autoverificable).
- **Manifiestos firmados + concesión de capacidad atada a `BLAKE3(bytecode)`**:
  `firmar_capacidad` / `verificar_capacidad`.
- **Distribución content-addressed por LAN sin IP/TCP** (AoE: anuncio broadcast +
  `SolicitarObjeto`/`ProveedorObjeto` + descarga recursiva del DAG).
- **Gossip anti-entropía transport-agnóstico** + transporte libp2p compartido con
  minga (`agora-net-brahman`, mismo `PeerId`).

## Gaps transversales (afectan a varias entradas)

- **No hay revocación** de identidades/atestaciones todavía. Un claim de
  revocación atestado cubriría el 80%.
- **§14.1.3 (enforcement de capacidad en kernel)** está hecho en host pero
  pendiente de validar en QEMU.

---

## Prioridad 1 — útil ya, casi todo el código existe

### P1. Store/repo de apps wawa firmado, offline, por LAN  [T]
Reusa: `wawa publicar` + `wawa anunciar` (ya difunden el release por Ethernet
crudo) + `verificar_canal`. Falta: índice navegable (qué canales/versiones hay en
la red) + botón "instalar" en `mudanza`. Caso: parque de máquinas sin internet
(fábrica, aula, barco, sitio remoto).

### P2. Distribución de datasets/árboles POSIX firmados por LAN  [T]
Reusa: `wawa importar` / `importar-imagen` / `exportar` (direccionan árboles por
contenido con dedup) envueltos en canal firmado. Es Syncthing/IPFS-lite **pero
autenticado**: N nodos descargan verificando cada objeto contra su hash. Caso:
repartir un corpus o assets a varias máquinas sin servidor central.

### P3. Notarización y firma de documentos  [T]
Reusa: `Attestation::create` firma cualquier claim; `firmar_manifiesto` firma
cualquier hash BLAKE3. Conectar a **pluma/khipu**: sellar un documento y atestar
"identidad X aprobó este blob el día Y". Verificación offline, sin CA externa.

### P4. Credenciales y membresía verificables de una organización  [S]
Reusa: `IdentityKind` + atestaciones. Una institución atesta "persona X es socia /
tiene rol Y / completó Z". El portador guarda la atestación; cualquiera la
verifica offline contra la pubkey de la institución, sin que esta esté online.
Casos: carnet de socio, certificación profesional, diploma, acreditación de
prensa, pase de evento.

---

## Prioridad 2 — alto valor, base sólida, falta UI/flujo

### P5. OTA delta para flotas de wawa  [T]
Reusa: canal append-only + descarga recursiva. Una nueva raíz transmite **sólo
los objetos nuevos** (dedup por contenido). Rollback = apuntar a una raíz anterior
del historial (ya trivial). Caso: upgrade incremental verificado de fábrica.

### P6. Web of trust / reputación sin autoridad central  [S]
Reusa: `agora-graph` (corroboración) + `TrustPolicy` negociada por lector (no hay
veredicto central). "¿A esta persona la avalan otros que yo ya respeto?" Cada quien
fija su umbral. Casos: marketplaces P2P, reputación de proveedores, vetting de
colaboradores, anti-sybil ligero.

### P7. Gobernanza por quórum (decisión colectiva M-of-N)  [S]
Reusa: `MultiSignature::verify_threshold_in` + whitelist. Una decisión sólo es
válida si K de N miembros firmaron. Gobernanza sin smart-contract ni blockchain.
Casos: junta que aprueba un gasto, comité que ratifica una propuesta, multifirma
para una acción crítica.

### P8. Libro de actas / registro de decisiones inmutable  [S]
Reusa: canal append-only firmado con orden temporal garantizado. Cada raíz = un
acta firmada y fechada, encadenada y auditable por cualquier miembro con
`verificar_canal`. Casos: actas de asamblea, historial de acuerdos de una
cooperativa, bitácora de gobernanza.

### P9. Log de auditoría / compliance verificable  [T]
Misma primitiva que P8 (el canal es un libro mayor), enfocada a trazabilidad
regulatoria, cadena de custodia, historial de aprobaciones. Re-verificación total
de la cadena por cualquiera.

---

## Prioridad 3 — ambicioso: requiere componer y/o cerrar gaps

### P10. Cooperativa / colectivo / DAO ligera sin cadena de bloques  [S]
Compone P4 + P7 + P8: identidad fractal anidada (persona ∈ comunidad ∈ alianza) +
quórum + libro de actas. Autogobierno con identidad soberana, decisiones por
quórum y registro inmutable, **offline-first y sin gas/fees**. Es el caso emblema
de la identidad fractal de agora.

### P11. Coordinación entre organizaciones que no se confían entre sí  [S]
Reusa: gossip + política por lector. Dos orgs intercambian atestaciones **sin
ceder su grafo** ni adoptar una autoridad común; cada una mantiene soberanía sobre
a quién cree. Casos: alianzas, consorcios, redes de cooperativas, federaciones
gremiales.

### P12. Mini-PKI con delegación de autoridad encadenada + revocación  [T]
Reusa: multifirma M-of-N + `AGORA_AUTH_RING` + cadena de atestaciones
("institución → delega en comunidad → delega en persona, ámbito X"). **Bloqueante:
falta revocación** (ver gaps). Casos: poderes verificables, apoderados, jerarquías
de aprobación.

### P13. Licenciamiento / entitlements por bytecode hash  [T]
Reusa: `firmar_capacidad(kp, bytecode_hash, permisos)` — "este binario exacto puede
usar estos permisos exactos, no transplantable". **Depende de §14.1.3 enforcement
en kernel** (ver gaps). Casos: gating de features, autorización de apps de terceros.

### P14. Identidad soberana (self-sovereign) anti-plataforma  [S]
Reusa: keystore cifrado local; tú emites tus claims, nadie te borra. Casos:
identidad que sobrevive a la muerte de una plataforma, perfil portable, "tu
reputación es tuya".

### P15. Padrón / censo verificable sin doble conteo  [S]
Reusa: `IdentityId = BLAKE3(pubkey)` estable + atestaciones cruzadas. Registro de
miembros con identidad única y verificable. Casos: padrón de un sindicato, censo de
una comunidad, lista de socios.

### P16. Mensajería / alertas autenticadas broadcast en LAN  [T]
Reusa: `MensajeAkasha` ya viaja firmado por L2. Extender el enum para anuncios
operativos (alertas, comandos a la flota) = bus de eventos autenticado sin broker.

### P17. Federación de confianza con minga (grafo de la verdad)  [T/S]
Reusa: `agora-net-brahman` comparte `PeerId` con minga
(`/agora/gossip/1.0.0` + `/minga/sync/1.0.0`). Las atestaciones agora son fuentes
firmadas con reputación para el "grafo de la verdad" de minga.

---

## Empieza por aquí

Ruta de menor fricción y máximo aprendizaje, encadenando artefactos demostrables:

1. **P1** (store LAN firmado) — monta sobre `wawa publicar`/`anunciar` ya operativos.
2. **P4** (credenciales de organización) — abre toda la pista social con sólo
   atestaciones + una UI mínima en `agora-app`.
3. **P7 + P8** (quórum + libro de actas) — la base de gobernanza, que luego
   compone en P10 (cooperativa/DAO ligera), el destino más distintivo de agora.
