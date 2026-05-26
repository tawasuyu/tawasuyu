# Cómo probar pluma — guía rápida

Pluma es la familia de crates para edición de documentos multilienzo: un
documento es un *haz de cuerpos* (lienzos) — el original, sus
traducciones, resúmenes, anotaciones — alineados párrafo a párrafo por
*hebras*. Esta guía muestra qué correr para ver cada pieza funcionando.

> Última actualización: 2026-05-26. Las versiones de modelo y nombres de
> env vars están vigentes al cierre de la primera iteración del stack
> LLM+multilienzo.

## TL;DR

```bash
# Tests unitarios del haz multilienzo (sin red, sin GUI):
cargo test \
  -p pluma-cuerpo \
  -p pluma-align \
  -p pluma-align-embeddings \
  -p pluma-transform \
  -p pluma-transform-tabla \
  -p pluma-transform-llm \
  -p pluma-graph-transform \
  -p pluma-store \
  -p pluma-llm-core \
  -p pluma-llm-mock \
  -p pluma-llm-anthropic \
  -p pluma-llm-gemini \
  -p pluma-llm-openai-compatible \
  -p pluma-llm \
  -p pluma-notebook-kernel-llm \
  -p pluma-editor-llimphi

# Demo visual sin red ni keys (mock pre-poblado):
cargo run -p pluma-editor-llimphi --example multilienzo_demo --release
```

Lo demás en esta guía es para ir cubriendo el resto del stack en
profundidad y/o usar IAs reales.

## 1. Validar tu API key contra un servicio real (gasto mínimo)

Antes de meterse a generar un multilienzo entero, corré el smoke de
Gemini para confirmar que tu key funciona. Una sola request,
~30 tokens, fracción de centavo:

```bash
GEMINI_API_KEY=tu_key cargo run \
  -p pluma-llm-gemini --example smoke --release
```

Salida esperada:

```
smoke :: usando modelo gemini-2.5-flash
respuesta: Rimay
tokens: input=28 output=2 cache_read=0 cache_creation=0
stop_reason: MAX_TOKENS
```

Existe el mismo patrón para los otros backends en cuanto se sumen
smokes — Anthropic y DeepSeek implementan la misma `ChatClient` y se
prueban igual contra sus respectivas API keys.

## 2. El multilienzo visual

Tres demos visuales del editor, todos ejecutables sin recompilar entre
ellos:

### 2.1 Estático — datos hardcoded

Lo más sencillo. No necesita ni LLM ni embeddings.

```bash
cargo run -p pluma-editor-llimphi --example multilienzo_demo --release
```

Tres cuerpos (`es` / `qu` runa simi / `en` resumen) con los cuatro
estados de hebra: Derivada fresca verde, Embeddings azul atenuado por
fuerza, Manual ámbar, Stale gris punteado.

### 2.2 LLM-driven — un solo flujo, cinco backends

El factory `pluma_llm::from_env` elige el backend según env vars.
Cambiar de IA = una variable.

```bash
# Sin keys → mock predecible con traducciones hardcoded:
cargo run -p pluma-editor-llimphi --example multilienzo_llm_demo --release

# Gemini real:
GEMINI_API_KEY=... PLUMA_LLM_BACKEND=gemini \
  cargo run -p pluma-editor-llimphi --example multilienzo_llm_demo --release

# Anthropic:
ANTHROPIC_API_KEY=sk-ant-... \
  cargo run -p pluma-editor-llimphi --example multilienzo_llm_demo --release

# DeepSeek:
DEEPSEEK_API_KEY=... PLUMA_LLM_BACKEND=deepseek \
  cargo run -p pluma-editor-llimphi --example multilienzo_llm_demo --release

# Ollama 100% local (requiere `ollama serve` y el modelo pulled):
PLUMA_LLM_BACKEND=ollama PLUMA_LLM_MODEL=llama3.1 \
  cargo run -p pluma-editor-llimphi --example multilienzo_llm_demo --release
```

Para que las hebras `qu↔en` sean semánticamente verdaderas (no
random del mock), levanta el embedder global en paralelo:

```bash
verbo-daemon --provider fastembed &
# (descarga ~120 MB de multilingual-e5-small la primera vez)

# después correr cualquiera de los demos
```

El demo detecta el socket en `$XDG_RUNTIME_DIR/verbo.sock` y se conecta
automáticamente.

### 2.3 Dinámico — botones de transformación

Toolbar con cuatro botones (`→ qu`, `→ en`, `tono formal`, `resumir 30p`).
Click → spawn thread con runtime tokio efímero → LLM transparente →
columna nueva al volver. Un trabajo a la vez (los botones quedan
deshabilitados durante la ejecución).

```bash
GEMINI_API_KEY=... PLUMA_LLM_BACKEND=gemini \
  cargo run -p pluma-editor-llimphi \
  --example multilienzo_dinamico_demo --release
```

### 2.3.0 Importar un archivo `.docx` como cuerpo madre

```rust
use foreign_docx::parse_docx;
let bytes = std::fs::read("informe.docx")?;
let imp = parse_docx(&bytes, "es", "informe.docx", ahora_unix())?;
// imp.cuerpo: Original, branch_id "es", lengua None
// imp.atoms: un NarrativeAtom por <w:p> con texto no vacío
```

Mismo shape que `pluma_md::DocumentoImportado` para que el caller los
trate uniforme. Formato Word (negrita, cursiva, estilos, headers,
footers, tablas, comments) se descarta — solo contenido legible.

### 2.3.1 Importar un archivo `.md` como cuerpo madre

```rust
use pluma_md::parse_md;
let texto = std::fs::read_to_string("notas.md")?;
let imp = parse_md(&texto, "es", "notas.md", ahora_unix());
// imp.atoms: un NarrativeAtom por bloque (párrafo, lista, encabezado…)
// imp.cuerpo: Intencion::Original con todos los atoms en orden.
for atom in &imp.atoms {
    graph.insert(atom.clone());
}
// Pasar imp.cuerpo como madre a cualquier ejecutor LLM.
```

Formato inline (negrita, cursiva, code inline) se aplana — el LLM
recibe texto limpio. Encabezados preservan jerarquía vía prefijo `# `,
`## `, etc.

### 2.4 Completo — toolbar dinámica CON persistencia + focus + búsqueda

El más cercano a "app real": botones LLM como en 2.3, pero CADA
transformación se persiste en `~/.cache/gioser/pluma-multilienzo-completo/`
antes de mostrarse. Cierra el demo, volvé a abrirlo: cuerpos y hebras
siguen ahí + podés seguir generando.

```bash
GEMINI_API_KEY=... PLUMA_LLM_BACKEND=gemini \
  cargo run -p pluma-editor-llimphi \
  --example multilienzo_completo_demo --release

# Reset:
MULTILIENZO_COMPLETO_RESET=1 cargo run -p pluma-editor-llimphi \
  --example multilienzo_completo_demo --release
```

Tras unas cuantas corridas tendrás un haz crecido: una madre `es` con
varias derivadas (`qu`, `en`, formal, resumen). El editor las muestra
todas alineadas por hebras Derivadas 1↔1.

**Atajos y botones del demo completo**:

| Acción | Cómo |
|---|---|
| Derivar cuerpo nuevo | Botones `→ qu` · `→ en` · `tono formal` · `resumir 30p` |
| Editar la madre | Botón `editar madre` (anexa marca incremental al 1er párrafo) |
| Marcar todas las hijas stale | Botón `tocar madre` |
| Regenerar hija stale | Botón `regenerar stale (N)` — una a la vez |
| Cambiar IA en runtime | Botón `modelo: X` — cicla por los 6 backends |
| Scroll horizontal | `Shift + rueda del mouse` · o eje X de touchpad |
| Focus mode | Botón `solo madre` / `todos` |
| Búsqueda transversal | Tipeá cualquier texto · `Backspace` borra · `Esc` limpia |
| Persistencia UI | Automática: scroll/focus/búsqueda se restauran al reabrir |
| Reset cache | `MULTILIENZO_COMPLETO_RESET=1` en el env |

### 2.5 Persistente — sobrevive entre corridas

Primera vez: genera y guarda en `~/.cache/gioser/pluma-multilienzo/`.
Siguientes: lee y muestra instantáneo, sin red.

```bash
# Primera corrida — pega al LLM:
GEMINI_API_KEY=... PLUMA_LLM_BACKEND=gemini \
  cargo run -p pluma-editor-llimphi \
  --example multilienzo_store_demo --release

# Segunda corrida — sin red ni tokens:
cargo run -p pluma-editor-llimphi --example multilienzo_store_demo --release

# Resetear el cache:
MULTILIENZO_RESET=1 cargo run -p pluma-editor-llimphi \
  --example multilienzo_store_demo --release
```

## 3. El stack LLM por crate (sin GUI)

Si querés ver cómo encajan las piezas sin abrir una ventana, los tests
unitarios cuentan bien la historia. Cada crate tiene su `pruebas` mod
con happy path + failure modes:

```bash
# El contrato + tipos:
cargo test -p pluma-llm-core

# Determinista para tests:
cargo test -p pluma-llm-mock

# Cada backend:
cargo test -p pluma-llm-anthropic
cargo test -p pluma-llm-gemini
cargo test -p pluma-llm-openai-compatible

# Fachada / factory:
cargo test -p pluma-llm

# Ejecutores de transformación:
cargo test -p pluma-transform-tabla
cargo test -p pluma-transform-llm

# Pegamento con el grafo y la store:
cargo test -p pluma-graph-transform
cargo test -p pluma-store

# Notebook con kernel LLM:
cargo test -p pluma-notebook-kernel-llm
```

## 4. Embeddings: provider real vs mock

`pluma-align-embeddings` calcula hebras `qu↔en` (o cualquier par) con
cualquier `rimay_verbo_core::Provider`. Tres formas de servirlo:

```bash
# Mock determinista (sin descarga, sin red, vectores random estables):
verbo-daemon --provider mock

# BGE local sin API key (descarga ~120 MB la primera vez):
verbo-daemon --provider fastembed

# Otro socket o dimensión:
verbo-daemon --provider mock --socket /tmp/mi.sock --dim 768
```

Y en la app:

```rust
let daemon = DaemonClient::connect("$XDG_RUNTIME_DIR/verbo.sock").await?;
let carta = alinear_por_embeddings(&qu, &en, &idx, &daemon, &params, ahora).await?;
```

Cualquier consumidor de `&dyn Provider` (pluma-semantic, chasqui-core,
khipu) habla igual.

## 5. Notebook + LLM — demo CLI ejecutable

`notebook_llm_demo` arma un notebook con cuatro celdas (markdown fuente
+ traducir-qu + tono-formal + resumir-20), corre `run_all` con el
`LlmKernel`, e imprime los outputs por consola. No usa ventana — útil
para verificar que el flujo entero funciona en un servidor sin GUI.

```bash
# Sin keys (mock que diferencia acciones por system):
cargo run -p pluma-notebook-kernel-llm --example notebook_llm_demo --release

# Con Gemini:
GEMINI_API_KEY=... PLUMA_LLM_BACKEND=gemini \
  cargo run -p pluma-notebook-kernel-llm --example notebook_llm_demo --release

# Con Ollama:
PLUMA_LLM_BACKEND=ollama PLUMA_LLM_MODEL=llama3.1 \
  cargo run -p pluma-notebook-kernel-llm --example notebook_llm_demo --release
```

Salida esperada (modo mock):

```
notebook_llm_demo :: LLM = mock-nb
=== ejecución ===
ejecutadas: 4   fallidas: 0   skipped: 0
=== outputs ===
[1/markdown] sin output
[2/llm-traducir-qu]     Kuntur wayqu hanaqpachatakta…
[3/llm-tono-formal]     El cóndor surcó con majestuosidad…
[4/llm-resumir-20]      Amanecer andino: cóndor, llamas, tejedora.
```

## 5.bis Notebook + LLM — uso programático

```rust
use pluma_llm::{build_client, BackendKind, LlmConfig};
use pluma_notebook_kernel_llm::LlmKernel;
use pluma_notebook_exec::run_all;

let chat = build_client(&LlmConfig {
    kind: BackendKind::Gemini,
    ..Default::default()
})?;
let kernel = LlmKernel::from_arc(chat);

let mut notebook = /* armar celdas con language="llm-traducir-qu", etc. */;
let reporte = run_all(&mut notebook, &kernel).await.unwrap();
println!("ejecutadas: {} | fallidas: {}", reporte.executed.len(), reporte.failed.len());
```

Lenguajes que `LlmKernel` entiende:

| `language`             | Qué hace                                              |
|------------------------|-------------------------------------------------------|
| `llm-prompt`           | El source ES el prompt completo. Sin system.          |
| `llm-traducir-{LANG}`  | Traduce al idioma dado (qu, en, fr…).                 |
| `llm-tono-{ETIQUETA}`  | Reescribe con tono (formal, casual, infantil…).       |
| `llm-resumir`          | Resumen libre.                                        |
| `llm-resumir-{N}`      | Resumen a aproximadamente N palabras.                 |
| `llm-reescribir`       | Primera línea del source = prompt, resto = texto.     |

## 6. Mapa del stack

```
pluma-core           NarrativeAtom + coherencia
pluma-graph          NarrativeGraph (DAG con propagación PendingEvaluation)
pluma-cuerpo         Cuerpo + MetaCuerpo + Intencion
pluma-align          Alineamiento + CartaHebras + alineadores manuales
pluma-align-embeddings    alineador semántico async via Provider
pluma-transform      trait Ejecutor async + EjecutorIdentidad
pluma-transform-tabla     EjecutorTraducirTabla (tabla explícita)
pluma-transform-llm  Ejecutor{Traducir,Tono,Resumir,Reescribir}Llm
pluma-graph-transform  indice_atoms + persistir_producto (pegamento)
pluma-store          PlumaStore (atoms+cuerpos+transformaciones+cartas en sled)
pluma-editor-llimphi::multilienzo  vista columnas+carriles+hebras

pluma-llm-core       trait ChatClient (contrato agnóstico de proveedor)
pluma-llm-mock       determinista para tests
pluma-llm-anthropic  Claude con prompt caching del system
pluma-llm-gemini     Gemini con cachedContentTokenCount
pluma-llm-cohere     Cohere Command (Anthropic-shape de response)
pluma-llm-openai-compatible  DeepSeek + Ollama + Groq/Together/vLLM
pluma-llm            fachada transparente: build_client(&cfg)/from_env

pluma-notebook-{core,exec,store}  notebook reactivo (DAG de celdas)
pluma-notebook-kernel-llm  conecta el notebook al LLM transparente
pluma-notebook-kernel-{python,wasm}  los otros kernels

rimay-verbo-{core,mock,daemon,daemon-bin,fastembed}  embedder global
```

## 7. Variables de entorno reconocidas

| Variable | Quién | Para qué |
|---|---|---|
| `PLUMA_LLM_BACKEND` | `pluma_llm::from_env` | `anthropic`/`gemini`/`deepseek`/`ollama`/`mock` |
| `PLUMA_LLM_MODEL` | `pluma_llm::from_env` | sobrescribe el default del backend |
| `PLUMA_LLM_ENDPOINT` | `pluma_llm::from_env` | proxy interno o endpoint custom |
| `ANTHROPIC_API_KEY` | `pluma-llm-anthropic` | Claude |
| `GEMINI_API_KEY` o `GOOGLE_API_KEY` | `pluma-llm-gemini` | Gemini AI Studio |
| `DEEPSEEK_API_KEY` | `pluma-llm-openai-compatible` | DeepSeek |
| `COHERE_API_KEY` | `pluma-llm-cohere` | Cohere Command |
| `MULTILIENZO_RESET` | `multilienzo_store_demo` | `=1` para limpiar el cache |
| `XDG_RUNTIME_DIR` | `verbo-daemon` y clientes | path del socket del embedder |
| `XDG_CACHE_HOME` | `multilienzo_store_demo` | base del cache de PlumaStore |
