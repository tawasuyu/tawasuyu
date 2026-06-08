# tawasuyu — la suite completa

> Documento dirigido a cualquier humano: desde alguien que nunca abrió una terminal
> hasta alguien que firma kernels. La idea es que se entienda **qué es**, **cómo
> está hecho**, **qué hazaña técnica representa** y **qué cambia en el mundo si
> existe**.

---

## 1. El escenario — por qué existe tawasuyu

Hoy, casi todo lo que usás en una computadora pasa por una capa controlada por
tres o cuatro corporaciones:

- El sistema operativo (Windows, macOS, Android, iOS).
- El motor del navegador (Chromium/Blink, WebKit, Gecko).
- El compositor gráfico, el toolkit, los frameworks de UI (Apple AppKit,
  Microsoft WinUI, Google Material, Meta React).
- La identidad (cuenta Google/Apple/Microsoft), el almacenamiento (Drive,
  iCloud, OneDrive), el ERP (SAP, Oracle), las herramientas de oficina, los
  navegadores de archivos, las llamadas de sistema profundas.

Cada una de esas capas decide, por vos, **qué podés ver, qué podés ejecutar,
qué se almacena, qué se transmite y bajo qué condiciones**. No hay malicia
necesariamente: hay arquitectura. Si la pila te pertenece, mandás vos. Si la
pila pertenece a otro, el otro manda — aunque la máquina sea tuya.

**tawasuyu es un intento de devolver la pila completa al usuario.** Desde el
arranque del hardware hasta el píxel que dibuja una letra, todo el camino se
escribe en una sola casa, en un solo lenguaje (Rust), bajo un solo principio:
*el filesystem es la arquitectura, y la arquitectura sigue el ciclo de la
información*.

No es un sistema operativo más. No es un “Linux con otro escritorio”. Es una
**suite vertical**: kernel propio, identidad propia, motor gráfico propio,
navegador propio, ERP propio, shell propio, compositor propio, broker de
mensajes propio, simulador propio, lenguaje musical propio. Todo encaja
porque todo se diseñó junto.

---

## 2. La cartografía — cómo está organizado

tawasuyu se estructura en **cuatro cuadrantes** que corresponden, en quechua,
a las cuatro fases del ciclo de la información:

```
tawasuyu/
├── 00_unanchay/   PERCIBIR  — pluma · khipu · rimay · chaka · pineal · puriy
├── 01_yachay/     CONOCER   — cosmos · dominium · nakui · iniy
├── 02_ruway/      HACER     — mirada · shuma · nahual · chasqui · takiy · llimphi · supay
├── 03_ukupacha/   RAÍZ      — arje · wawa · agora · minga
├── shared/                  — sandokan · auth · card · ssh · format
└── web/                     — landing sobria (no producto)
```

- **`00_unanchay` (Percibir)** — todo lo que mete información al sistema:
  documentos, notas, lenguaje, datos legacy, visualizaciones, navegador web.
- **`01_yachay` (Conocer)** — los modelos del mundo: astrología, simulación
  determinista, ERP, laboratorio semántico.
- **`02_ruway` (Hacer)** — interfaces y ejecución: shell gráfico, runtime de
  espacios, motor gráfico, broker de mensajes, composición musical, juego.
- **`03_ukupacha` (Raíz)** — la base inamovible: init, kernel, identidad,
  filesystem distribuido.

A esto se suma `shared/` (librerías cruzadas) y `web/` (una landing sobria, no
es producto).

El principio: **cada cuadrante es una fase del ciclo de la información**.
Mover un dominio de cuadrante es renombrar la naturaleza del dominio. No hay
carpetas “utils”, “misc”, “common”. Si algo no encaja en una fase, es porque
está mal pensado, no porque haga falta una nueva carpeta.

Disciplina constante:
1. **Un dominio = un crate raíz con subcrates plugin**. Sin proliferación.
2. **Las UIs son frontends intercambiables** sobre `*-core` agnósticos. La
   lógica de dominio no sabe quién la pinta.
3. **Los nombres con carga semántica fuerte se respetan** sea cual sea su
   idioma. Si el quechua nombra mejor una cosa que el inglés, queda en
   quechua.

Hoy (2026-05-25): **~220 crates compilando** en el workspace, más 13 más en
wawa (kernel, excluido del workspace por target distinto). GPUI está marcado
para extinción y se reemplaza por Llimphi.

---

## 3. Los dominios, uno por uno

### `00_unanchay` — Percibir

**pluma** — Edición de documentos vivos. No “un editor de markdown”: un
sistema donde un documento es un grafo dirigido (DAG) de bloques que se
pueden ejecutar, encadenar, observar. Incluye un *notebook* estilo Jupyter
pero con runtimes vía WASM (RustPython, Boa para JS, webR para R), y un
*deck* (presentaciones). Editor portado a Llimphi (2026-05-25): bloques
posicionados absolutamente, conectores en S, osciloscopio de coherencia
visible.

> *Logro técnico:* un notebook donde el kernel no es un proceso sino un
> módulo WASM addressable por contenido — el output se identifica por su
> hash, no por su orden temporal. Eso permite re-ejecutar partes sin
> reordenar.

**khipu** — Notas con gravedad semántica. Las notas se atraen entre sí
según afinidad de contenido, no por carpetas. El nombre viene del sistema
de cuerdas anudadas inca: información codificada en topología.

**rimay** — Capa de lenguaje natural. Un *daemon* (`verbo-daemon`) sirve
embeddings y modelos pequeños locales a cualquier otro dominio que necesite
entender o generar texto. Sin enviar nada a servidores externos.

**chaka** — Puente con código legacy. *Chaka* es “puente” en quechua: un
compilador-traductor con lexer, parser, IR, codegen y runtime que entiende
subconjuntos de COBOL (con planes de extender a CICS y dialectos SQL). El
objetivo: permitir que sistemas bancarios y gubernamentales escritos en
COBOL hace 40 años corran dentro de tawasuyu sin tener que reescribirlos a
mano. Hay un patrimonio enorme de software crítico en COBOL que la
industria abandonó; chaka lo recupera.

**pineal** — Visualización viva. Charts, heatmaps, treemaps, polares,
cartesianos, financieros, malla 3D, fósforo (oscilloscopio retro),
streaming. Backend basado en *SceneCanvas* sobre Llimphi (ya migrado del
GPUI viejo). Cuatro widgets y cuatro demos funcionando.

**puriy** — *Navegador web soberano*. Quizás la apuesta más ambiciosa del
cuadrante. Embebe **Servo** (motor web en Rust, sin C++) y delega *todo* el
render a Llimphi. Resultado: un navegador que corre **idéntico** en Linux
sobre el compositor mirada, y en wawa bare-metal directamente sobre
framebuffer, sin sistema operativo intermedio. Cuatro crates:

- `puriy-core` — sesiones, tabs, history, perfiles. Puro Rust, sin gráficos.
- `puriy-engine` — bridge a Servo: DOM/CSS/JS/networking → primitivas
  geométricas.
- `puriy-llimphi` — el “chrome” (barra, pestañas, address bar).
- `puriy-app` — el binario lanzable.

> *Logro técnico:* romper el monopolio de tres motores web (Blink, WebKit,
> Gecko) con un cuarto motor que no asume ni X11 ni Win32 ni macOS — su
> superficie es abstraíble al framebuffer.

---

### `01_yachay` — Conocer

**cosmos** — Modelo astronómico-astrológico. Cálculo de efemérides,
sistemas de coordenadas (ecuatoriales, eclípticas, galácticas), tiempo
(juliano, sidéreo), WCS, catálogos estelares, *sky* (cielo simulable),
*pointing* (apunte de telescopios), corpus de interpretación, render
gráfico de rueda zodiacal con glifos unicode (☉♀♈…) traducidos a primitivas
vello. **23 subcrates**. La rama astrológica no es esotérica para el sistema:
es un dominio de cómputo simbólico que se modela tan formalmente como una
órbita planetaria.

> *Logro técnico:* un solo modelo donde la efeméride newtoniana real y la
> interpretación simbólica conviven sin contaminarse. Un astrónomo puede
> ignorar el corpus; un astrólogo puede ignorar la mecánica celeste; los
> datos son los mismos.

**dominium** — Simulador determinista. Física (`physics`), proyección
isométrica (`iso`), `render-plan` (plan de dibujo declarativo) y app
Llimphi que corre un loop a ~11 Hz reinyectando ticks vía `Handle::dispatch`.
Sirve para simular dinámicas sociales, ecológicas, urbanas, económicas.
*Determinista* significa: dada la misma semilla, la misma simulación. Sin
sorpresas, sin ruido oculto.

**nakui** — ERP soberano. Entidades, registros, módulos
(inventory/sales/treasury/crm), WAL + replay + snapshot + auto-compact +
executors Rhai (scripting). Explorer en Llimphi con timeline de cards,
breakdown, banners, polling cada 2 s. Un negocio chico, una cooperativa, una
asociación, puede llevar su contabilidad y operación entera **sin SAP, sin
Oracle, sin nube de un tercero**. Los datos viven en disco propio, en
formato auditable, con historial completo.

**iniy** — Laboratorio semántico. Modela **grados de creencia** (Subjective
Logic) y *dirección de subjetividad*. Subcrates: `ingest`, `extract`, `nli`,
`graph`, `store`, `cli`. Piloto: auditar libros — leer un texto y mapear,
proposición por proposición, *con qué grado* el autor lo afirma y *desde
qué perspectiva*. Es decir, no “qué dice el libro” sino “con cuánta
seguridad lo dice, y desde dónde”.

> *Logro técnico:* incorporar incertidumbre estructural al pipeline de
> conocimiento. La mayoría de los sistemas tratan los enunciados como
> binarios (verdadero/falso) o probabilísticos (0..1). iniy modela la
> creencia como una tripleta (creo / no creo / no sé) con dirección — el
> mismo aparato que usa la inteligencia militar y el derecho probatorio.

---

### `02_ruway` — Hacer

**llimphi** — *Motor gráfico soberano*. El pilar técnico de toda la suite.
Cuatro capas estrictas:

1. **`llimphi-hal`** — abstracción de hardware sobre **wgpu** (Vulkan
   nativo en Linux) y **winit** (mientras desarrollamos en Linux). El día
   que se monta sobre wawa, se desenchufa winit y se pasa el puntero crudo
   del framebuffer del kernel. Pantalla gris plomo a 144 Hz como hito de
   vida.
2. **`llimphi-raster`** — rasterizador vectorial sobre **vello** (compute
   shaders): líneas, círculos, polígonos, gradientes, antialiasing
   matemático.
3. **`llimphi-layout`** — motor de layout sobre **taffy** (flex/grid).
   Paneles redimensionados en menos de 1 ms por frame.
4. **`llimphi-ui`** — bucle Elm completo (input → update → view → layout →
   raster), texto con shaping vía **parley** (fallback CJK/emoji por
   fontique), hit-test, clip (push_layer/pop_layer con Mix::Clip),
   `on_wheel`, `hover_fill`, `draggable`, drop-targets globales
   (`drag_payload` + `on_drop`), imágenes (`View::image` con `peniko::Image`
   en aspect-fit), canvas custom (`paint_with(Fn(&mut Scene, &mut
   Typesetter, PaintRect))`), `Handle::spawn_periodic` para ticks de
   simulación, override de tamaño inicial (`App::initial_size`).

Sobre eso, una **biblioteca de widgets reutilizables** en
`02_ruway/llimphi/widgets/`: `list`, `text-input`, `button` (con hover),
`splitter` (con drag), `tabs`, `tree` (expand/collapse + selección),
`app-header`, `card`, `stat-card`, `banner` (Info/Success/Warning/Error),
`tiled` (grid auto cols×rows con drag-to-swap activo). Cada uno con su
`examples/{widget}_demo.rs` ejecutable, más una `gallery` (binario) que
pinta todos juntos como referencia visual y smoke test. Paleta compartida
en `llimphi-theme` con *slots semánticos* (bg_app, fg_text, accent, etc.) —
cada widget consume `Palette::from_theme(&theme)`.

> *Logro técnico:* reemplazar la pila GPUI (de Zed) por una pila propia con
> el mismo nivel de abstracción y la mitad de la opacidad. Y prepararla
> para correr **idéntica** sobre Wayland y sobre framebuffer crudo en
> wawa, sin condicionales de plataforma en el código de aplicación.

**mirada** — Compositor + shell gráfico Wayland. Subcrates: `compositor`,
`brain`, `body`, `layout`, `protocol`, `link`, `launcher`, `portal`,
`greeter` (ya en Llimphi), `bar-core`/`bar-web`, `ctl`, `app`. Es el
escritorio entero, hecho a mano. El usuario inicia sesión vía `greeter`
(usa `auth-core` intacto), elige espacios desde `launcher`, abre apps
desde `portal`. Sin GNOME, sin KDE, sin XFCE: tawasuyu tiene su propio
escritorio.

**shuma** — Runtime de *espacios*. Sandbox + baremetal (matilda
absorbido). 8 bloques de roadmap apuntan a paridad con zsh/fish y
reemplazo de ssh/mosh/tmux en una sola pieza. Chasis basado en Llimphi
con 4 *slots* (TopBar/Main/BottomBar/DrawerTab) y un drawer estilo Quake
(desplegable con tecla). El shell/launcher/command-bar son **módulos**, no
hardcodeados. Conectividad SSH para *matilda* ya funciona (dry-run remoto
vía SSH sin tocar nada).

**nahual** — Shell de apps. Hoy ofrece `nahual-shell-llimphi`: file
explorer + viewer dual (texto o imagen según extensión PNG/JPG/JPEG) en
split *draggable*. Cada pieza extraída a su crate Llimphi reusable
(`nahual-file-explorer-llimphi`, `nahual-text-viewer-llimphi`,
`nahual-image-viewer-llimphi`, decodificando PNG/JPEG con el crate
`image`). Navegación con teclado, mouse, rueda. Más una biblioteca de
widgets propios (`app-header`, `banner`, `card`, `stat-card`,
`theme-switcher`, `tree`) y libs de bus/meta-runtime/meta-schema.

**chasqui** — Message broker monádico. El nombre viene de los corredores
incas que llevaban mensajes a lo largo del Tahuantinsuyo. Subcrates:
`core`, `broker`, `broker-explorer`, `card`, `explorer`, `nous`,
`nous-mock`, `nous-real`. Es la capa de mensajería interna entre dominios
de tawasuyu — equivalente a Kafka/RabbitMQ pero monádico (los mensajes
componen como funciones, no como colas).

**takiy** — Composición musical. Por ahora `takiy-core`. La meta: una app
de composición musical con generador IA de sonidos. *Takiy* es “cantar /
hacer música” en quechua.

**supay** — Modernizar Doom sin tocar su alma. *Supay* es el espíritu
infernal andino. Fase 0.x: raycaster hardcoded sobre Llimphi con sprites,
sector lights, texturas procedurales, disparo, enemies, pickups, game
over. Fase 1.0: `supay-core` con FFI + `build.rs` a *doomgeneric*, y
`supay-doom-llimphi` que pinta el framebuffer 320×200 como `View::image`.
Es una prueba contundente de que Llimphi puede hostear desde una app de
oficina hasta un shooter clásico.

---

### `03_ukupacha` — Raíz

**arje** — Init system. `init`, `runtime`, `compat`, `card`. El proceso 1
de tawasuyu: levanta el sistema, monta el rootfs, lanza mirada, conecta con
mesa para la GPU. End-to-end en hardware real es uno de los hitos
pendientes.

**wawa** — Kernel SASOS (Single Address Space Operating System) basado en
**WASM**. *Wawa* es “niño / bebé” en quechua: porque el kernel arranca
desde lo más pequeño. La idea radical: en vez de procesos aislados por
MMU (con todo el costo de cambio de contexto), todas las aplicaciones son
módulos **WASM compilados AOT con cranelift**, viven en un único espacio
de direcciones, y la seguridad la garantiza la **verificación de tipos
del bytecode**, no el hardware. Ingesta de datos vía POSIX → BLAKE3
(Destilador en host + AoE en red + atlas con Fontdue, fase 21a): wawa
*nunca* habla NTFS/Ext4 directo, todo entra por contenido hasheado.

Optimizaciones gaming planificadas: AOT WASM cranelift, GPU passthrough,
frame pacing cooperativo, asset streaming BLAKE3.

Apps wawa ya esbozadas: `bitacora` (logs), `cronista` (historia/registro),
`discola` (audio), `glotona` (consumidor de recursos / test), `hello_wasm`,
`memoriosa` (memoria), `pregon` (anuncios), `pulso` (heartbeat),
`tonada` (audio melódico).

> *Logro técnico:* un kernel donde la unidad de aislamiento es el módulo
> WASM, no el proceso UNIX. Eso significa: arranque más rápido,
> comunicación entre apps sin syscalls, portabilidad nativa (la misma app
> corre en x86_64, ARM, RISC-V sin recompilar), y un modelo de seguridad
> que no depende del fabricante de la CPU.

**wawa-explorer** — Visor *host-side* del DAG de wawa.
`wawa-explorer-core` lee imágenes `.img`, `wawa-explorer-aoe` es cliente
*Akasha* con raw sockets, `wawa-explorer-llimphi` es la UI tree + detalle.
Permite inspeccionar el filesystem de contenido de wawa **sin bootear
wawa** — desde una máquina huésped corriente.

**agora** — Identidad federada. `agora-core`, `agora-graph`, `agora-store`,
`agora-app`. Sin Google, sin Apple, sin Microsoft, sin cuenta de empresa.
Identidad criptográfica propia, opcionalmente federada con otros nodos
tawasuyu (o no — uno puede vivir aislado).

**minga** — VFS P2P (peer-to-peer). `minga-core`, `minga-dht`, `minga-p2p`,
`minga-store`, `minga-vfs`, `minga-cli`, `minga-explorer`. *Minga* es el
trabajo comunitario andino. Filesystem distribuido entre pares, sin
servidor central. Tus archivos viven en tu nodo y en los nodos de tu red
de confianza, no en Drive ni en S3.

---

### `shared/`

Librerías que cruzan cuadrantes:

- **sandokan** — orquestador *hot-swap* consumible por shuma y otros. Le
  permite a un proceso reemplazarse en vivo sin perder estado.
- **auth** — primitivas de autenticación reutilizables (lo usa el
  `mirada-greeter`).
- **card** — tarjetas/payloads tipados que viajan entre módulos.
- **ssh** — cliente/servidor SSH propio (lo usa shuma para matilda).
- **format** — formateo común (números, fechas, bytes, etc.).

---

## 4. Las tecnologías involucradas

tawasuyu no inventa todo desde cero: se apoya en lo mejor que existe en el
ecosistema Rust, y rellena los huecos.

| Capa | Tecnología | Rol en tawasuyu |
|---|---|---|
| Lenguaje | **Rust** | Único lenguaje del workspace. Seguridad de memoria sin recolector. |
| GPU | **wgpu** | Vulkan portable, control fino sobre el silicio. |
| Rasterizado vectorial | **vello** | Compute shaders para curvas, AA, gradientes. |
| Layout | **taffy** | Flex/grid con la semántica de CSS pero en Rust nativo. |
| Texto | **parley + fontique + swash + skrifa** | Shaping completo, BiDi, CJK, emoji. |
| UI architecture | Bucle Elm propio (en `llimphi-ui`) | input→update→view, sin VDOM. |
| Window manager (dev) | **winit** | Mientras desarrollamos en Linux. Se desenchufa en wawa. |
| Compositor Wayland | **mirada** (propio) | Compositor + shell + bar. |
| Motor web | **Servo** + **puriy** | Único motor web nativo en Rust. |
| Hashing de contenido | **BLAKE3** | Direccionamiento por contenido en wawa y notebooks. |
| WebAssembly runtime | **wasmtime/cranelift** | AOT para apps de wawa y kernels de notebook. |
| Lenguajes embebidos | **Rhai** (scripting de nakui), **RustPython** / **Boa** / **webR** (kernels de notebook) | Ejecución sandboxed. |
| Decodificación imágenes | crate `image` (PNG, JPEG) | Viewers de nahual. |
| Persistencia de ERP | WAL + snapshot + auto-compact propios | nakui-core. |
| P2P / DHT | minga propio | VFS distribuido. |
| Mensajería | chasqui propio | Broker monádico interno. |
| Init | arje propio | PID 1. |
| Kernel | wawa propio | SASOS sobre WASM. |
| Embeddings / NLI | rimay + iniy | Locales, sin nube. |

**Nada de lo anterior depende de Google, Apple, Microsoft, Meta, Amazon u
Oracle.** Las dependencias externas están auditadas, son OSS, y en su
mayoría son crates atómicos (wgpu, vello, taffy, parley) que hacen una sola
cosa bien.

---

## 5. El logro técnico

Visto en frío, lo que tawasuyu logra es:

1. **Verticalidad real, no marketing.** Una sola pila va desde el
   framebuffer (`llimphi-hal`) hasta la interpretación astrológica
   (`cosmos-corpus`) sin que ninguna capa intermedia sea un binario
   cerrado.

2. **El mismo motor gráfico corre con OS y sin OS.** Llimphi sobre Wayland
   (mirada) y Llimphi sobre framebuffer de wawa comparten el código de
   aplicación al byte. Eso no existe casi en ningún lado fuera de los
   sistemas embebidos militares.

3. **Un navegador web cuarto.** Tres motores monopolizan el 99% del
   tráfico mundial (Blink, WebKit, Gecko). Puriy+Servo+Llimphi es una
   cuarta opción viable, y la primera en Rust puro embebida en una suite
   de escritorio.

4. **Un kernel SASOS sobre WASM.** El aislamiento no viene de la MMU sino
   del verificador de tipos. Eso libera enormes oportunidades: arranque en
   milisegundos, IPC sin syscalls, portabilidad CPU-agnóstica.

5. **Direccionamiento por contenido en todo el sistema.** wawa nunca habla
   NTFS o Ext4. Lo que entra entra por BLAKE3. Eso significa que dos
   archivos idénticos ocupan un espacio. Que cualquier corrupción se
   detecta. Que la sincronización P2P (minga) es trivial — los archivos
   son sus hashes.

6. **Subjetividad como tipo de dato.** iniy modela creencia y dirección
   subjetiva como ciudadanos de primera clase. La mayoría de los sistemas
   no distinguen entre “es verdad” y “alguien lo cree”. tawasuyu sí.

7. **ERP, navegador web, simulador, juego, compositor, kernel — todos en
   el mismo workspace, todos verdes en `cargo check --workspace`.** Más de
   220 crates conviven sin romperse. Eso es disciplina de monorepo a un
   nivel raro en proyectos no corporativos.

8. **Reescritura simultánea de GPUI.** GPUI (el toolkit de Zed) era una
   dependencia clave. Se está desenchufando crate a crate (mirada-greeter,
   pluma-editor, dominium-canvas, cosmos-canvas, nakui-explorer, nahual-shell
   ya migrados). Salir de un toolkit de UI vivo es brutal — y tawasuyu lo
   hace en producción.

---

## 6. Las consecuencias sociales del uso

Una pieza de software puede parecer neutra. No lo es. Cada decisión técnica
materializa una posición sobre cómo viven los humanos. Estas son las
consecuencias previsibles si tawasuyu se usa en serio:

### 6.1. Autonomía digital del individuo y la organización

Una persona con tawasuyu corriendo en su hardware **no le debe nada a una
plataforma** para hacer su trabajo cotidiano. Su identidad (agora), sus
documentos (pluma, khipu), su contabilidad (nakui), sus archivos (minga),
su navegación (puriy), su escritorio (mirada) y su computación (wawa) no
están alojadas en un servidor de un tercero ni dependen de un servicio
remoto para funcionar. Eso es **soberanía digital** en sentido literal:
nadie puede apagarte la computadora desde afuera.

### 6.2. Resistencia frente a la obsolescencia programada

Las empresas tradicionales sacan versiones nuevas que rompen las viejas y
obligan a comprar hardware nuevo o pagar suscripciones. Una suite escrita
para correr desde un framebuffer pelado, sin sistema operativo, en
hardware modesto (porque no necesita la complejidad de Windows o macOS),
**alarga la vida útil de las máquinas**. Es ecológico no por marketing
sino por arquitectura.

### 6.3. Recuperación de patrimonio software (COBOL)

Hay billones de dólares y décadas de lógica de negocio escritas en COBOL
ejecutándose en bancos, gobiernos, aseguradoras. Cuando esos sistemas se
retiran, *la lógica se pierde* o se reescribe a mano con errores. chaka
permite correr ese código adentro de tawasuyu sin reescritura. Es
**arqueología de software activa**, no museo.

### 6.4. Un cuarto motor web

Tres motores controlan la web. Si uno bloquea una API, la API muere. Si
los tres deciden que cierto contenido no se renderiza, no se renderiza.
Puriy+Servo cambia ese cálculo: la web también puede leerse desde un
motor que no responde a Mountain View, Cupertino ni Redmond.

### 6.5. Subjetividad explícita en los modelos de conocimiento

iniy y el corpus de cosmos enseñan algo que la industria mainstream
oculta: **los datos siempre tienen sujeto**. Que un libro afirme algo no
es lo mismo que que ese algo sea verdad. Que un sistema lo registre no es
lo mismo que ese registro sea neutral. Modelar la subjetividad como tipo
de dato es **un acto político**: dignifica al lector y al disidente.

### 6.6. ERP fuera del oligopolio

SAP y Oracle dominan el ERP mundial. Cooperativas, pymes, asociaciones,
escuelas y sindicatos no pueden pagarlos — terminan con planillas Excel
sin auditoría. nakui les ofrece un ERP propio, con WAL auditable,
scripting en Rhai, modular. La consecuencia social es directa:
**organizaciones medianas y comunitarias pueden llevar finanzas e
inventarios al mismo nivel técnico que una multinacional**, sin pagar
licencias.

### 6.7. Una computadora que vuelve a ser una computadora

Quizá lo más profundo. Hoy, una *computadora personal* dejó de ser
personal: es un terminal que se conecta a servicios. tawasuyu propone
revertir eso. Tu máquina vuelve a ser una máquina **que ejecuta tu
software con tus datos bajo tus reglas**. No es nostalgia: es una
infraestructura para que la vida digital deje de ser, por defecto,
alquilada.

### 6.8. La cultura andina como infraestructura

Los nombres no son decoración. *Khipu*, *chasqui*, *minga*, *yachay*,
*ruway*, *unanchay*, *ukupacha*, *pacha*, *wawa*, *supay*, *llimphi*,
*takiy*, *rimay*, *puriy*, *agora*, *nakui*, *cosmos*, *iniy* — algunos
quechua, otros griegos, otros latinos, todos elegidos porque **nombran
mejor** la cosa que un anglicismo técnico. Que un usuario aymara, quechua,
peruano, boliviano, argentino o ecuatoriano encuentre su lengua viva en
el corazón de una pila técnica seria **es una consecuencia social en sí
misma**: la informática no es solo Silicon Valley.

---

## 7. Estado al 2026-05-25

- ~220 crates en el workspace tawasuyu, verde en `cargo check --workspace`.
- 13 crates más en wawa (kernel, target distinto, excluido del workspace).
- Llimphi: 5 crates (`hal/raster/layout/text/ui`) verdes en hardware.
  Texto con shaping completo. Bucle Elm con hit-test funcional. 12
  widgets reusables con demos.
- GPUI: en extinción. Apps ya portadas a Llimphi: mirada-greeter,
  pluma-editor, dominium-canvas + dominium-app, cosmos-canvas + cosmos-app
  (MVP), nakui-explorer + nakui-ui (MVP), nahual-shell + viewers, pineal
  (backend + 4 widgets + 4 demos).
- Shuma: chasis con 4 slots y drawer Quake. Conectividad SSH para matilda
  funcionando (dry-run remoto vía SSH).
- Cosmos: MVP que renderiza rueda zodiacal con glifos unicode astrológicos
  vía vello.
- Supay: Fase 0.x (raycaster con sprites, sector lights, disparo, enemies)
  entregada. Fase 1.0 (doomgeneric via FFI) andamiada.
- Nahual: shell con file explorer + viewer dual draggable, todo modular en
  Llimphi.
- Wawa-explorer: visor host-side del DAG de wawa funcionando.

Hitos próximos: Puriy fase 1 (`puriy-core` puro Rust en paralelo), migración
completa de GPUI a Llimphi, arje end-to-end en hardware real con mesa, wawa
expandiendo hardware soportado, dominium validado como simulador
determinista.

---

## 8. Cómo leer este documento

- Si nunca tocaste código: leíste un sistema operativo y una suite de
  aplicaciones hechos en un solo proyecto, en un solo lenguaje, sin
  depender de Google, Apple ni Microsoft. Cada palabra rara (cosmos,
  nakui, mirada, llimphi) es el nombre de un módulo, como “Word” es el
  nombre de un programa de Microsoft.
- Si sos programador: es un workspace Rust de ~220 crates organizados en
  cuatro cuadrantes semánticos, con un motor gráfico propio (Llimphi)
  sobre wgpu/vello/taffy/parley, un compositor Wayland propio (mirada),
  un kernel WASM SASOS propio (wawa), un navegador propio sobre Servo
  (puriy), un ERP con WAL+snapshot+Rhai (nakui), y disciplina monorepo
  con `cargo check --workspace` verde.
- Si sos investigador o periodista: es un caso de estudio sobre soberanía
  digital, recuperación de COBOL, descentralización P2P, modelado de
  subjetividad, y revaloración de lenguas no-coloniales como
  infraestructura semántica.

El nombre **tawasuyu** (de *geocentric organizer*) refleja el principio
fundacional: la computación parte del lugar concreto donde el usuario
está parado, no del centro de datos de otro.
