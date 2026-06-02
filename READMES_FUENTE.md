# READMES_FUENTE — entrada humana para generar todos los READMEs

> **Qué es este archivo.** La fuente única y autoritativa desde la cual se generan
> los `README.md` / `LEEME.md` / `README.qu.md` de todo el monorepo, en cada nivel
> de la jerarquía: **raíz → cuadrante → dominio/app**. No es un README; es el
> *brief* humano. Acá vos escribís lo que querés que diga cada pieza, y la
> generación se ata a esto. Si algo no está acá, no debería aparecer en un README.
>
> **Cómo se usa.** Editás las ranuras `✍️` y los campos de cada nodo. Cuando el
> proyecto esté cerca de cerrar, una pasada de generación produce los READMEs
> acotados a estas instrucciones. Las descripciones ya pobladas son **semilla**
> (sacadas de CLAUDE.md, los READMEs de cuadrante existentes y el estado real del
> árbol al 2026-06-02) — corregilas, recortalas o borralas a gusto.
>
> **Notación.**
> `✍️` = ranura para que la edites vos. `⚠️` = dato que tengo dudoso, confirmá.
> `# una línea:` = el one-liner canónico que encabeza el README de ese nodo.

---

## 0 · Directivas globales de estilo (aplican a TODOS los READMEs salvo override local)

Estas reglas las hereda cada README. Sobreescribí por nodo cuando haga falta.

- **Idiomas / archivos por nodo:**
  - `README.md` → inglés (técnico, sobrio).
  - `LEEME.md` → español (registro del repo; comentarios y commits ya son en español).
  - `README.qu.md` → quechua.
  - ✍️ ¿los tres en cada nodo, o sólo en raíz+cuadrante y app sólo en uno? → _decidir_
- **Largo máximo:**
  - Raíz: ✍️ _(p.ej. ≤ 120 líneas)_
  - Cuadrante: ✍️ _(p.ej. ≤ 60 líneas)_
  - App/dominio: ✍️ _(p.ej. ≤ 40 líneas)_
- **Audiencia:** ✍️ _(¿dev externo que llega de cero? ¿colaborador del repo? ¿usuario final?)_
- **Tono:** ✍️ _(sobrio/técnico · sin marketing · sin emoji · sin "blazingly fast", etc.)_
- **Qué SIEMPRE incluir por app:** una línea de qué es · cómo correrla (comando real) · subcrates y su rol · estado de madurez.
- **Qué NUNCA incluir:** ✍️ _(p.ej. roadmaps especulativos, benchmarks sin reproducir, capturas, badges, "TODO", nombres de agentes/Claude)_
- **Nombres intocables (rule #6):** no retraducir `khipu, rimay, pluma, wawa, mirada, nahual, chasqui, takiy, agora, arje, minga, shuma, nakui, iniy, tinkuy, chaka, pineal, puriy, supay, sandokan, dominium, cosmos, tullpu, yupay, llimphi, akasha, unanchay, yachay, ruway, ukupacha`. (Agregar a la lista: ✍️ `ayni, paloma, pata, raymi, cards, media, nada`).
- **Comandos:** mostrar siempre el comando real de `CLAUDE.md` (`cargo run -p ... --example ...`), no inventados.
- **Enlaces:** cada README de cuadrante linkea a los READMEs de sus apps; cada README de app linkea a su SDD si existe.

---

## 1 · RAÍZ (monorepo) — `/README.md`

```
# una línea: ✍️ (semilla) Suite vertical en Rust — kernel propio, identidad, motor
#            gráfico, navegador, ERP, shell, broker y simulador — en un solo
#            workspace de ~210 crates, con la arquitectura embebida en el filesystem.
```

**Qué tiene que explicar el README raíz (semilla, ordená/recortá):**
- Qué es gioser y la tesis: **una sola suite vertical, no apps sueltas.**
- El **ciclo de la información** como esqueleto físico: cuatro cuadrantes =
  cuatro fases (PERCIBIR → CONOCER → HACER → RAÍZ). Mover un dominio de cuadrante
  cambia su naturaleza; no son carpetas administrativas.
- El layout (tabla de los 4 cuadrantes + `shared/` + `web/`).
- Las **reglas duras** (un dominio = un crate raíz + subcrates plugin; UIs son
  frontends sobre `*-core`; GPUI extinto → todo gráfico sobre Llimphi; formatos
  ajenos por `shared/foreign-*`; `cargo check --workspace` verde siempre).
- Cómo arrancar (smoke test + dónde están las demos).
- Punteros a la documentación de fondo: `PLAN.md`, `WAWA.md`, los SDD.

**✍️ Demarcación humana (raíz):**
> _Acá escribí el "qué es" definitivo, la frase que querés que abra el repo público.
> Qué resaltar, qué silenciar, a quién le hablás en la primera pantalla de GitHub._

**Estado / alcance de publicación:** v1 = workspace entero público, sin recortes (ver `CIERRE.md`).

---

## 2 · CUADRANTES

> Cada cuadrante ya tiene `README.md/LEEME.md/README.qu.md`. Acá fijás la versión
> canónica de su **frase, regla y manifiesto**, y la lista de apps que le cuelgan.

### 00_unanchay · PERCIBIR
```
# una línea: unanchay (quechua: marcar, señalar, hacer notar). Cuadrante de la
#            PERCEPCIÓN: cómo entra la información. Regla: fidelidad antes que opinión.
```
- **Manifiesto (semilla):** *Percibir antes de pensar.* Lo que llega entra con
  dignidad: no se interpreta, resume ni descarta; se preserva y se ofrece.
- **Apps:** chaka · khipu · pineal · pluma · puriy · rimay
- **✍️ Demarcación:** _ajustá la regla/manifiesto; ¿qué NO va en este cuadrante?_

### 01_yachay · CONOCER
```
# una línea: yachay (quechua: saber, conocimiento). Cuadrante del MODELO: lo
#            percibido se organiza como teoría. Regla: el modelo valida contra la
#            realidad, no contra sí mismo.
```
- **Manifiesto (semilla):** *Conocer es atreverse a equivocarse con precisión.*
  Determinismo cuando se pueda · exactitud sobre estética · unidades explícitas ·
  validar contra efemérides/datos/sims independientes.
- **Apps:** cosmos · dominium · iniy · nakui · tinkuy
- **✍️ Demarcación:**

### 02_ruway · HACER
```
# una línea: ruway (quechua: hacer, fabricar). Cuadrante de la ACCIÓN: interfaces,
#            compositores, brokers, shells. Regla: la materia manda.
```
- **Manifiesto (semilla):** *Hacer es comprometerse con la materia.* Cero deps
  gráficas en `core` · mismo árbol de escena en Wayland y Wawa · el usuario marca
  el ritmo · herramientas que respetan al artesano.
- **Apps:** ayni · cards · chasqui · llimphi · media · mirada · nada · nahual ·
  paloma · pata · raymi · shuma · supay · takiy · tullpu · wawa _(host-side)_
- **✍️ Demarcación:** _este cuadrante creció mucho desde el README viejo; decidí qué apps son "producto" y cuáles son banco de pruebas._

### 03_ukupacha · RAÍZ
```
# una línea: ukupacha (quechua: mundo interior, raíz, lo que está bajo tierra).
#            Cuadrante de la INFRAESTRUCTURA invisible: kernel, boot, FS, red
#            profunda, comunidad. Regla: invariantes antes que features.
```
- **Manifiesto (semilla):** *La raíz sostiene callada.* Sin deps frívolas ·
  content-addressed por default (BLAKE3 = identidad) · el cliente del kernel es el
  operador, no el usuario · documentar como para un arqueólogo a 20 años.
- **Apps:** agora · arje · minga · sandokan · wawa _(kernel + apps WASM)_ · wawa-explorer
- **✍️ Demarcación:**

---

## 3 · APPS / DOMINIOS

> Un bloque por dominio. `# una línea:` es el one-liner del README de la app.
> Bajo "subcrates" listo lo que hay en disco (rol entre paréntesis donde lo sé).
> En `✍️` escribís qué resaltar, qué omitir y el estado de madurez.

### 00_unanchay — PERCIBIR

#### chaka
```
# una línea: puente con lo heredado. Lee fuentes externas (BCD, formatos viejos,
#            lenguajes muertos) y las normaliza al idioma del monorepo.
```
- subcrates: chaka-ir · chaka-bcd · chaka-lexer · chaka-parser · chaka-codegen · chaka-runtime · chaka-shadow · chaka-app · chaka-app-llimphi
- SDD: —
- ✍️ resaltar / omitir / estado:

#### khipu
```
# una línea: captura de notas con gravedad temporal. Olvidar es parte del modelo:
#            lo viejo se desvanece, lo recurrente queda. P2P soberano completo
#            (LAN+WAN+relay/NAT+descubrimiento UDP&DHT+AutoNAT+identidad cifrada).
```
- subcrates: khipu-core · khipu-gravity · khipu-share · khipu-brahman · khipu-app
- ✍️ resaltar / omitir / estado:

#### pineal
```
# una línea: visualización agnóstica de backend (cartesiano · polar · mesh ·
#            treemap · phosphor · flow · heatmap · stream · financiero · umbrella).
#            El órgano visual.
```
- subcrates: pineal-core · pineal-render · pineal-export · pineal-{cartesian,polar,mesh,treemap,heatmap,hexbin,phosphor,umbrella,contour,financial,flow,stream,bars}
- ✍️ resaltar / omitir / estado:

#### pluma
```
# una línea: documentos vivos. Markdown como grafo de átomos editables (haz de
#            cuerpos/lienzos alineados párrafo-a-párrafo) + notebook DAG reactivo,
#            con el LLM como transformador, no como autor.
```
- subcrates (familia grande): núcleo `pluma-core/cuerpo/md/store/semantic/align` · multilienzo `pluma-editor-{llimphi,cuerpo}/transform*/graph*` · notebook `pluma-notebook-{core,exec,store,llimphi,graph-llimphi,app}` + kernels `-kernel-{llm,dominium→cosmos,python,wasm,tinkuy,media}` · deck `pluma-deck-*` · LLM `pluma-llm` + backends `-{anthropic,gemini,cohere,openai-compatible,mock,core}` · web `pluma-md-reader-web/deck-web` · puente `foreign-docx`
- SDD: —  · Ref: `PLAN.md` §11 (modelo multilienzo)
- ✍️ resaltar / omitir / estado: _(familia enorme — decidí qué subcrates merecen README propio)_

#### puriy
```
# una línea: navegador web soberano sobre Servo + Llimphi (motor QuickJS-ng,
#            ES2024 completo). Mismo engine en Linux y en Wawa bare-metal.
```
- subcrates: puriy-core · puriy-engine · puriy-js · puriy-llimphi · puriy-app
- SDD: `00_unanchay/puriy/SDD.md`
- ✍️ resaltar / omitir / estado:

#### rimay
```
# una línea: lenguaje. Embeddings y verbos: cuando algo "quiere decir" algo, pasa
#            por acá. Daemon de embeddings por socket Unix.
```
- subcrates: rimay-verbo(-core) · rimay-verbo-{mock,fastembed} · rimay-verbo-daemon(-bin)
- ✍️ resaltar / omitir / estado:

---

### 01_yachay — CONOCER

#### cosmos
```
# una línea: astronomía con precisión astronómica. Tiempo, efemérides, coordenadas,
#            WCS, astrología, validación contra efemérides oficiales.
```
- subcrates: motor `cosmos-engine/core/model` · astrométrico puro `cosmos-{ephemeris,skywatch,sundial,tides,transits,rise-set,eclipses,coords,time,wcs,sky,pointing}` · astrología `cosmos-astrology` · datos `cosmos-{catalog,corpus,images,store,leo}` · UI/IO `cosmos-{app-llimphi,canvas-llimphi,render,cli,web,card,notebook-kernel,modules,validation}`
- SDD: —
- ✍️ resaltar / omitir / estado:

#### dominium
```
# una línea: simulador mean-field determinista: cinco capas físicas (materia ·
#            psique · poder · oro · degradación) + agentes vectoriales + acople
#            endógeno ψ↔acción.
```
- subcrates: dominium-core · dominium-physics · dominium-iso · dominium-render-plan · dominium-{app-llimphi,canvas-llimphi,cli,notebook-kernel}
- SDD: `01_yachay/dominium/SDD.md`
- ✍️ resaltar / omitir / estado:

#### iniy
```
# una línea: laboratorio semántico. Subjective Logic + eje de dirección-de-
#            subjetividad para auditar afirmaciones. Piloto: auditorías de libros y wikis.
```
- subcrates: iniy-core · iniy-{ingest,extract,nli,nli-llm,graph,store,wiki,server,cli,explorer-llimphi}
- ✍️ resaltar / omitir / estado:

#### nakui
```
# una línea: motor reactivo estilo Excel sobre principios sólidos: Decimal exacto,
#            cascada topológica, WAL, time-travel, invariantes atómicas. Tres vistas
#            (matriz · grafo · forma) sobre el mismo grafo de tokens.
```
- subcrates: nakui-core · nakui-backend (WAL/snapshot) · nakui-sheet(-nakuicore,-llimphi) · nakui-ui-llimphi · nakui-explorer-llimphi
- subcrates PLANEADOS (aún no en disco): `yupay-core` + `yupay-fns` — motor de fórmulas DSL Excel-like (`=SUMA(A1:A10)`, bilingüe es/qu) compilado a Rhai (PLAN §6.ter).
- ✍️ resaltar / omitir / estado:

#### tinkuy
```
# una línea: motor de partículas DOD (ECS-SoA + Grid3D + Velocity-Verlet paralelo)
#            con snapshots BLAKE3 compatibles con Wawa.
```
- subcrates: tinkuy-core · tinkuy-{sim,forces,dsl,abi} · tinkuy-llimphi
- ✍️ resaltar / omitir / estado: _(roadmap B-F cerrado; DslForce queda single-thread)_

---

### 02_ruway — HACER

#### ayni  ✅ = el dominio Chat
```
# una línea: chat persona-a-persona soberano, local-first, sin servidor. La
#            conversación como grafo criptográfico reproducible (BLAKE3 + DAG +
#            postcard), identidad agora Ed25519, E2EE MLS (RFC 9420), transporte
#            chasqui/minga/akasha. No es "otro wasap": es ayni (reciprocidad).
```
- subcrates: ayni-core (DAG firmado, no_std+alloc) · ayni-crypto (Ed25519 + E2EE X25519/HKDF/ChaCha) · ayni-sync (Transporte+TCP+anti-entropía Merkle) · ayni-minga (P2P libp2p) · ayni-store (sled) · ayni-app (núcleo de app) · ayni-cli (`ayni`) · ayni-llimphi (UI) · ayni-index (búsqueda rimay) · ayni-ai (multilienzo vía pluma-llm)
- **YA tiene README.md + LEEME.md propios** (tesis completa fase a fase) — la generación debería partir de ahí, no reescribir desde cero. Estado: P0–P7 cerradas (2026-05-31).
- ✍️ resaltar / omitir / estado:

#### cards  ⚠️ confirmar
```
# una línea: ✍️ (carpeta `cards` sin Cargo.toml propio visible — ¿agrupador de
#            crates relacionados con Cards/Brahman? confirmá rol y contenido)
```
- ✍️ resaltar / omitir / estado:

#### chasqui
```
# una línea: broker de mensajes + bus tipado. El sistema nervioso del monorepo.
#            Descubrimiento DHT y NAT traversal (relay/dcutr/autonat vía card-net).
```
- subcrates: chasqui-core · chasqui-{broker,card,nous,nous-real,nous-mock} · chasqui-{explorer-llimphi,broker-explorer-llimphi} · card-{admin,sidecar,handshake}
- ✍️ resaltar / omitir / estado:

#### llimphi
```
# una línea: framework de UI nativo (hal · raster · layout · text · theme · ui) +
#            widgets + módulos. El núcleo gráfico que comparten todas las apps.
#            Stack wgpu+vello+taffy+parley, bucle Elm input→update→view→layout→raster→present.
```
- subcrates: llimphi-{hal,raster,surface,compositor,gpu-bench} · llimphi-{layout,text,theme,icons,motion} · llimphi-ui · llimphi-{gallery,workspace}
- SDD/MANUAL: `02_ruway/llimphi/SDD.md`, `02_ruway/llimphi/MANUAL.md` (catálogo ~44 widgets, 10 módulos)
- ✍️ resaltar / omitir / estado: _(pieza central — el README probablemente deba apuntar al MANUAL en vez de duplicarlo)_

#### media
```
# una línea: suite de medios: captura, fuentes (wav/flac/mp3/vorbis/opus/gif/
#            webm/av1/imagen), mux (webm) y grabadores/encoders (opus/av1).
```
- subcrates: media-core · media-source-* · media-encode-{opus,av1} · media-mux-webm · media-audio-cpal · media-recorder-{wav,webm,av1,app} · media-app
- ✍️ resaltar / omitir / estado:

#### mirada
```
# una línea: compositor Wayland (mirada-compositor) + portal XDG (mirada-portal) +
#            greeter de login (mirada-greeter). El stack de display. DM completo
#            con WM estilo Hyprland (dropterm, teselado, foco-sigue-ratón).
```
- subcrates: mirada-{compositor,protocol,layout,body,brain,link,portal,greeter,ctl} · mirada-{app-llimphi,launcher,launcher-llimphi,asistente-llimphi,bar-core,bar-web} · asistente-puente
- SDD: —  · Ref: `BRAHMAN.md`, plan WM por rondas (R1 hecha)
- ✍️ resaltar / omitir / estado: _(sólo validado en Intel; NVIDIA/Optimus pendiente)_

#### nada
```
# una línea: editor de archivos sobre Llimphi: file tree + editor con LSP + clipboard
#            real + sesiones. Banco de pruebas del framework. (Antes `gioser-edit`.)
```
- subcrates: crate único (raíz) — _plan de split A/B documentado (3.5k LOC)_
- ✍️ resaltar / omitir / estado:

#### nahual
```
# una línea: visores cotidianos + shell de archivos. Meta-app "open-with" universal:
#            despacha el visor por contenido. Familia de visores Llimphi.
```
- subcrates: nahual-{viewer-core,source-core,thumb-core,geo-core} · nahual-shell-llimphi · nahual-file-explorer-llimphi · nahual-{text,image,video,audio,markdown,svg,hex,font,tree,table,map,card,archive}-viewer-llimphi · nahual-gallery-llimphi
- ✍️ resaltar / omitir / estado:

#### paloma  ✅ = el dominio Correo
```
# una línea: cliente de correo nativo (IMAP/SMTP/JMAP). El reemplazo de Gmail en
#            gioser: núcleo agnóstico + frontend Llimphi, identidad/firma por agora.
```
- subcrates: paloma-core · paloma-net · paloma-store · paloma-app · paloma-llimphi
- ✍️ resaltar / omitir / estado: _(confirmar madurez real de la implementación)_

#### pata
```
# una línea: cliente que pinta, clickea y escribe dentro del compositor mirada
#            (sampler dmabuf no-bloqueante, data-control, input a layers). ⚠️ confirmar encuadre
```
- subcrates: pata-core · pata-config · pata-llimphi
- ✍️ resaltar / omitir / estado: _(elige adapter por render-node del compositor; sólo Intel)_

#### raymi  ⚠️ confirmar
```
# una línea: ✍️ (raymi = "fiesta/celebración"; tiene net/store/app/core/llimphi —
#            confirmá qué hace; aparece como hotspot multi-agente)
```
- subcrates: raymi-core · raymi-{net,store} · raymi-{app,llimphi}
- ✍️ resaltar / omitir / estado:

#### shuma
```
# una línea: shell interactiva (paridad zsh/fish) con vistas en chasis Llimphi
#            (TopBar/Main/BottomBar/Drawer). Cards y pipes restaurados del port GPUI→Llimphi.
```
- subcrates: shuma-shell-llimphi · shuma-{daemon,gateway,cli}
- ✍️ resaltar / omitir / estado:

#### supay
```
# una línea: renderer estilo DOOM sobre Llimphi (FFI a doomgeneric, atlas de
#            sprites, paletas WAD). Audio supay↔takiy integrado.
```
- subcrates: supay-core · supay-{wad,scene,audio,render-llimphi,mini-core} · supay-{app-llimphi,doom-llimphi}
- SDD: `02_ruway/supay/SDD.md`
- ✍️ resaltar / omitir / estado:

#### takiy
```
# una línea: música. Captura, secuenciación, render de audio. MIDI + síntesis + playback.
```
- subcrates: takiy-core · takiy-{synth,midi,playback} · takiy-app-llimphi
- ✍️ resaltar / omitir / estado:

#### tullpu
```
# una línea: pintura/imagen. Kernel de pintura buffer-puro (tullpu-paint) + ops +
#            render; daemon de embeddings de pixeles (pixel-verbo, análogo a rimay-verbo). ⚠️ confirmar
```
- subcrates: tullpu-core · tullpu-{paint,ops,render} · tullpu-app-llimphi · pixel-verbo(-core,-mock,-daemon,-daemon-bin)
- ✍️ resaltar / omitir / estado:

#### wawa _(host-side)_
```
# una línea: panel de control + wawactl para el stack Wawa (la contraparte de
#            userspace del kernel en 03_ukupacha/wawa).
```
- subcrates: wawactl · wawa-panel-llimphi
- ✍️ resaltar / omitir / estado:

---

### 03_ukupacha — RAÍZ

#### agora
```
# una línea: plaza pública. Foro, conversación, deliberación con identidad mínima.
#            Firma y verifica Ed25519 end-to-end; raíz-de-confianza ejecutable de wawa.
```
- subcrates: agora-core · agora-{channel,gossip,graph,store,keystore,net-brahman} · agora-{app,cli}
- Ref: `WAWA.md` §14.1.3 (tabla de capacidades por hash de bytecode — pendiente)
- ✍️ resaltar / omitir / estado:

#### arje
```
# una línea: bootloader y vida temprana del sistema. Semillas, empaquetado,
#            instalación, absorción de un sistema existente. ⚠️ ajustar (subcrates en disco difieren del README viejo)
```
- subcrates: arje-card · arje-card-llimphi · arje-compat  _(README viejo mencionaba arje-{seeds,packager,installer,absorb} — confirmar)_
- Ref: toolchain UEFI (musl/mtools/OVMF)
- ✍️ resaltar / omitir / estado:

#### minga
```
# una línea: colaboración entre nodos (tradición andina de trabajo comunal aplicada
#            a la red). VFS + DHT + P2P + descubrimiento de cards. Backlog cerrado.
```
- subcrates: minga-core · minga-{vfs,store,dht,p2p} · card-discovery · minga-{cli,explorer-llimphi}
- ✍️ resaltar / omitir / estado:

#### sandokan
```
# una línea: plano de control: quién arranca/para/supervisa/observa unidades en
#            Linux y Wawa, sin duplicados. Process manager con UI Llimphi (sparkline CPU).
```
- subcrates: sandokan(-core) · sandokan-{local,remote,daemon,arje-engine} · sandokan-{app,monitor-llimphi}
- SDD: `shared/sandokan/SDD.md` (plano de control)  ⚠️ _(hay sandokan en 03_ukupacha y en shared — aclarar la repartición en el README)_
- ✍️ resaltar / omitir / estado:

#### wawa _(kernel + apps WASM)_
```
# una línea: sistema operativo desde cero. POSIX → ingest BLAKE3; filesystem como
#            DAG direccionado por contenido; gaming-grade (AOT WASM + GPU passthrough
#            + frame pacing cooperativo). Excluido del workspace raíz (x86_64-unknown-none).
```
- subcrates: wawa-kernel · wawa-boot · wawa-fs · gop-probe · `apps/` (WASM cdylib)
- SDD/doc: `02_ruway/wawa/SDD.md`, `WAWA.md` §0–§14
- ✍️ resaltar / omitir / estado: _(bootea end-to-end en QEMU; pendiente lag cursor virtio-tablet)_

#### wawa-explorer
```
# una línea: visor host-side del DAG de Wawa: lee el `.img`, habla el protocolo
#            Akasha sobre raw sockets, muestra el árbol con detalle en Llimphi.
```
- subcrates: wawa-explorer-core · wawa-explorer-aoe · wawa-explorer-llimphi
- ✍️ resaltar / omitir / estado:

---

## 4 · shared/ — crates transversales

> ✍️ Decidí si `shared/` lleva su propio README índice, o sólo READMEs por crate,
> o nada (sólo documentado desde la raíz).

- **sandokan** — plano de control (SDD autoritativo). _(ver nota de repartición arriba)_
- **auth / card / ssh** — identidad, tarjetas/cards, transporte SSH. ✍️
- **format** — formato nativo (BLAKE3 + DAG + postcard); núcleo `no_std`.
- **akasha** ⚠️ _(referenciado por path desde wawa; confirmar ubicación real del crate)_
- **foreign-docx / foreign-av / foreign-fs / foreign-psd / foreign-ytdlp** — puentes
  a formatos/fuentes ajenos. Las apps trabajan en formato nativo; lo ajeno entra
  sólo por acá (rule #4).
- **foreign-xlsx / foreign-pptx** — PLANEADOS (aún no en disco): xlsx ↔ nakui+yupay,
  pptx ↔ pluma-deck (PLAN §6.ter).
- **forth-emisor** — emisor Forth; núcleo `no_std` (cruza a wawa).
- **rimay-localize** — i18n sobre fluent (es/en/qu), bidi isolates.
- **wawa-config / wawa-config-llimphi** — bus de config de wawa (JSON canónico + notify).
- **app-bus** — registro único de apps (dock/spotlight/menubar global).
- **launcher-core / launcher-llimphi** — lanzador de apps.
- ✍️ resaltar / omitir / estado por crate:

---

## 5 · web/ — landing

```
# una línea: landing sobria sobre WASM (wasm-bindgen). No es producto, es cartel.
#            Única pieza del workspace que cruza el puente JS.
```
- crate: `web/gioser-web`  · build: `./scripts/build-gioser-web.sh {dev|release}`
- ✍️ resaltar / omitir / estado:

---

## 5.bis · APPS PLANEADAS — todavía sin dominio en disco

> No saltear estas: son parte del alcance v1 aunque aún no estén arrancadas.
> Fuente: `APPS-NATIVAS.md` (huecos por construir) + `PLAN.md`. Cada una nacerá
> como dominio nuevo con `*-core` agnóstico + frontend(s) Llimphi (regla #1/#2),
> identidad por `agora`, transporte por BrahmanNet/`card-net`, y registro en el
> "open-with universal" de `nahual`. Pre-escribí su README acá si querés que salga
> apenas existan; mientras no haya crate, NO se genera README (sólo queda anotada).

> Ya mapeadas a dominios existentes (NO van acá, tienen su bloque en §3):
> **Correo → `paloma`** · **Chat → `ayni`**. Mantenidas fuera de esta lista.

**Tanda 1 — "Google Workspace" diario:**
- **Calendario + Contactos** — CalDAV/CardDAV; comparte capa de cuentas con `paloma` (correo). ✍️

**Tanda 2 — tiempo real (P2P + WebRTC ya existen):**
- **Videollamadas** — UI de conferencia sobre P2P (akasha/minga) + `media` (video) + `takiy` (audio); stack WebRTC vive en `puriy-js`. _⚠️ ¿`raymi`?_ ✍️

**Tanda 3 — descubrimiento e información:**
- **Mapas / navegación** — tiles OSM + routing como canvas Llimphi (GPU directo). ✍️
- **Lector RSS / noticias** — agregador de feeds + `khipu` (gravedad temporal) + `rimay` (clustering). ✍️
- **Buscador / meta-search nativo** — front sobre varios motores o índice local; pega con puriy + `rimay`. ✍️

**Tanda 4 — segunda línea:**
- **Lector PDF/ePub dedicado** — hoy parcial vía nahual/pluma-deck. ✍️
- **Cliente Git/forge** — GitHub/GitLab vía API + `shuma` + `nada` + pluma notebook. ✍️
- **Dashboard de finanzas** — vertical Fintech sobre el meta-modelo de `nakui`. ✍️

**Otros planeados (puentes/motores, no apps):** `yupay` (motor de fórmulas, bajo nakui) ·
`foreign-xlsx` · `foreign-pptx` — ver §3 (nakui) y §4 (shared).

---

## 6 · Notas de generación (para quien corra la pasada final)

- Respetar el largo y tono fijados en §0 y los overrides por nodo.
- No inventar comandos: copiar de `CLAUDE.md` y de los `scripts/`.
- Cada `# una línea:` ya editada se vuelve el primer párrafo del README de ese nodo.
- Donde haya `⚠️`, NO generar hasta que el humano lo resuelva.
- Si una app no tiene texto humano en su ranura `✍️`, generar sólo lo mínimo
  (one-liner + comando + subcrates) y marcarla como "borrador".
- Mantener la coherencia trilingüe: el `LEEME.md` es traducción fiel del `README.md`,
  no un texto distinto (salvo que §0 diga lo contrario).
