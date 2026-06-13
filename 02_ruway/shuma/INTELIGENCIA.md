# INTELIGENCIA.md — estrategias de control power en shuma

> Propuesta 2026-06-12. Estado: **borrador para discusión** — nada de esto
> está comprometido; cada ítem cita el artefacto real sobre el que se monta.

## Tesis

La inteligencia de shuma no es un chatbot pegado a una terminal: es el shell
**observando el trabajo real** y devolviendo control en dos dosis distintas
según el usuario. El *nerdo habitual* quiere que el shell le ahorre teclas y
le avise cosas sin pedirle nada — inteligencia **que se ofrece sola y se
acepta con una tecla**. El *nerdo extremo* quiere lo contrario: superficies
**programables y direccionables** donde la inteligencia es un instrumento
más bajo su mando, nunca un piloto.

Regla transversal: **determinista primero, LLM opcional después**. Todo lo
de la lista A funciona sin red ni modelo; el LLM (vía `pluma-llm`, fachada
con fallback a Mock) sólo entra explícitamente invocado y rotulado.

## Inventario — lo que ya existe y dónde

| Pieza | Crate | Estado |
|---|---|---|
| Patrones emergentes (coreografías repetidas → abstracción con `Varies`) | `sandbox/shuma-infer` | vivo; alimenta el ghost **y** el chip de coreografía (A1, 2026-06-13) |
| Ghost predictivo (prefijo → sufijo del corpus) | `sandbox/shuma-line::ghost` | vivo en el input |
| Grafo de intenciones (`%cN`/`%pN`, nodos por comando) | `sandbox/shuma-intent::SessionGraph` | vivo; lo pinta `shuma-module-canvas` |
| Macros parametrizables | `sandbox/shuma-intent::MacroBook` | **núcleo listo, sin UI ni builtin** |
| Grupos ejecutables (`:save` → F1..F8) | `shuma-module-shell` | vivo |
| Reprocess (stdout de un bloque → stdin del próximo) | `shuma-module-shell` (chip `» stdin`) | vivo |
| Completions por comando (TOML en `~/.config/shuma/completions/`) | `sandbox/shuma-config` | vivo |
| Coloreo semántico (Severity err/warn/ok, números, fechas…) | `sandbox/shuma-line::decorate` | vivo (2026-06-12) |
| Env aprendible + persistencia aprendible (`:env`, `:persist`) | `shuma-module-shell` + `shuma-config::upsert_key` | vivo (2026-06-12) |
| Daemon + workspaces + quotas + stats | `shuma-daemon` / `sandbox/shuma-protocol` | vivo |
| Gateway JSON/WS (clientes móviles) | `shuma-gateway` | vivo; PTY **efímero** (gap conocido) |
| Historial durable con cwd + éxito | `sandbox/shuma-history` | vivo |
| LLM multi-backend con Mock fallback | `00_unanchay/pluma/pluma-llm` | vivo (en pluma) |

La estrategia entera es **cablear lo que ya está parido**, no inventar
maquinaria nueva. Sólo E3 y E4 requieren código sustancial.

## A — El nerdo habitual: inteligencia que se ofrece sola

Principio: cero configuración, cero prompt engineering. El shell propone,
el usuario acepta con una tecla o ignora. Toda propuesta es descartable y
**aprendible al shumarc** (la infraestructura `upsert_key` ya existe).

### A1. Coreografías que se ofrecen como grupo (cablear `shuma-infer` a UI) ✅ (2026-06-13)
`detect_patterns` corre tras cada comando y alimentaba sólo el ghost. Ahora,
cuando un `EmergingPattern` supera el umbral (`CHOREO_OFFER_THRESHOLD = 3`
ocurrencias), un chip discreto sobre el input ofrece guardarlo:
*«↻ lo corriste 3 veces · guardar «git+cargo+cargo» como grupo? (git pull →
cargo build → cargo test) [guardar] [descartar]»*. **Hecho:**
`choreography_suggestion` / `accept_choreography` en `update/patterns.rs`
(promueve el patrón a `CommandGroup` con `suggested_name()` + las líneas reales
de la última ocurrencia, ejecutable por F-key); `choreography_chip` en
`view/mod.rs` sobre el input; Msgs `AcceptChoreography`/`DismissChoreography`;
descartes en memoria (`State.dismissed_choreo`). Verificado headless
(`examples/choreo_chip.rs` → PNG) + 3 tests unitarios. **Pendiente menor:** el
chip vive en `view()` (shell standalone); falta llevarlo a la barra de pata
(`body_view` no incluye el input).

### A2. Alias sugerido por longitud × frecuencia
Línea > 40 chars repetida ≥ 3 veces sin variación → ofrecer alias corto
(`[aliases]` del rc vía `upsert_key`). Mismo chip que A1, otra fuente.

### A3. Ghost contextual por cwd ✅ (2026-06-13)
El historial guarda `cwd` por entrada. **Hecho:** `current_ghost`
(`update/patterns.rs`) rankea el corpus en dos tramos — primero las entradas
del cwd actual y sus hijos (`cwd_within`), después lo global; dentro de cada
tramo, lo más reciente primero. En un monorepo `cargo b…` en `cosmos/`
completa al build de cosmos, no al de wawa. Test: el del cwd manda aunque sea
más viejo que uno global.

### A4. "¿Quisiste decir…?" determinista ✅ (2026-06-13)
**Hecho:** al cerrar un comando con `command not found`, `detect_did_you_mean`
(`update/patterns.rs`) busca el binario más cercano por **Damerau-Levenshtein**
(transposición = 1, atrapa `cagro`→`cargo`), **priorizando el historial** sobre
el PATH (`ShellSource::commands`). Notice clickeable bajo el bloque
(`did_you_mean_notice` en `surface_view`): *«¿quisiste decir «cargo build
--release»? · click lo lleva al input»* (`Msg::AcceptDidYouMean` rellena el
input para revisar y Enter — nunca auto-ejecuta). `State.did_you_mean` por
bloque. Sin modelo, sin red. Verificado headless (`examples/did_you_mean.rs`)
+ 6 tests (Damerau, corrección desde historial, gates de no-oferta).

### A5. Titular de bloque al colapsar ✅ (2026-06-13)
Al plegarse un bloque, el header gana un resumen determinista contado desde
las decoraciones `Severity`: *«3 errores · 3 avisos · 7 líneas · 4 s»*,
coloreado como semáforo (rojo si hubo errores, ámbar si sólo avisos, tenue si
limpio). El nerdo habitual escanea la columna de headers como un log
semáforo. **Hecho:** helper `semaforo_titular` en `view/output_line.rs`
(cuenta líneas con severidad Error/Warn + duración `block_ended − block_started`,
campo nuevo en `State`); cableado en ambos renderers — en la superficie
(`surface_header`, default) va right-aligned en el header y reemplaza los
chips de acción al colapsar (modo escaneo), en el legacy (`command_card`) va
como segunda fila *«… · clic para ver»*. Verificado headless
(`examples/titular_a5.rs` → PNG) + 3 tests unitarios.

### A6. Aviso de comando largo terminado
Comando > 30 s que cierra mientras el usuario está en otra sesión/diente →
badge en el diente del rail (el LED de actividad ya existe en
`session_tooth_icon`) + notice. Nada de notificaciones del sistema: el
chasis es la superficie.

## B — El nerdo extremo: superficies direccionables y programables

Principio: el shell expone sus entrañas como **datos direccionables** y
**puntos de enganche declarativos**. Nada se ofrece solo: todo se invoca.

### E1. Macros con parámetros (`:macro`) — darle UI al MacroBook
`shuma-intent::MacroBook` ya modela macros parametrizables; falta el puente:
`:macro save deploy %1 %2` captura la intención vigente con huecos,
`:macro run deploy prod v2` la instancia, `:macros` lista. Persistencia en
`~/.config/shuma/macros.toml`. Es el ascensor natural de A1: el patrón
emergente se promociona a macro con nombre y parámetros explícitos.

### E2. El scrollback como base de datos (`%cN` en la línea)
`shuma-intent::parse` ya entiende refs `%cN`/`%pN`; el shell ya tiene
`gather_block_stdout`. Cablear: `grep error %c12 | sort` materializa el
stdout del bloque 12 como stdin. Combinado con las secciones-tabla, un
`ls -l` viejo es una **tabla consultable**, no píxeles muertos. El chip
`» stdin` actual es el caso degenerado (sólo "el próximo comando").

### E3. Reglas declarativas en el rc (`[rules]`) — el plano de control
El shumarc deja de ser sólo preferencias y gana gatillos deterministas:

```toml
[rules]
on_exit_nonzero = ":jobs"                 # qué correr cuando algo falla
on_enter_cwd."~/proyectos/wawa" = ":env RUST_BACKTRACE=1"
on_pattern_score = 3                      # umbral de A1 (0 = nunca ofrecer)
on_long_command_secs = 30                 # umbral de A6
```

Las propuestas de la lista A se vuelven **políticas editables**: lo que el
habitual acepta con un click, el extremo lo gobierna por archivo. Motor:
un match determinista en `update` (sin DSL turing-completo; eso ya lo cubre
Rhai en pluma si algún día hace falta).

### E4. Flota persistente (daemon attach/detach) — el gap real
El caveat del `shuma-gateway` README es exacto: `ExecPty` muere con el WS.
La pieza que falta para "control power" en serio es el **PTY persistente en
el daemon**: `workspace` retiene el par PTY+buffer, el cliente se re-adjunta
(`Request::PtyAttach { workspace, desde_byte }`) y recibe el backlog. Con
eso: N claudes corriendo, attach desde el shell o desde el móvil vía
gateway, quotas por workspace (`WorkspaceQuota` ya está en el protocolo).
`:persist` de hoy documenta el gap; esto lo cierra. Esfuerzo: el mayor de
la lista — es donde conviene gastar el próximo sprint de shuma.

### E5. LLM como instrumento invocado (`:?`)
Con `pluma-llm` enchufado (backend por env, Mock sin credenciales):
- `:? <pregunta>` — lenguaje natural → línea de comando propuesta (NUNCA
  auto-ejecutada; aparece en el input para editar/Enter).
- `:explica %cN` — explicar el output de un bloque.
- `:resume %cN` — titular A5 pero narrativo, para logs gigantes.
Siempre rotulado (`🜲 llm`), siempre opt-in por invocación, local-first si
hay Ollama. El extremo elige modelo; el habitual ni se entera de que existe.

### E6. `:stats` — telemetría propia, local, consultable
El historial + `block_started` + exit codes ya contienen todo: `:stats`
responde frecuencias, tasas de fallo por binario, duraciones p50/p95, horas
pico. Render como sección-tabla ordenable (la misma de `ls -l`). Alimenta
los rankings de A3/A4 y le da al extremo el espejo de su propio uso. Cero
red: los datos no salen de la máquina.

## Orden propuesto

1. **A5 + A1** (titular semáforo + chip de coreografía): máximo efecto/LOC,
   todo el material ya está en memoria. ✅ **hecho 2026-06-13.**
2. **A3 + A4** (ghost por cwd + quisiste-decir): afinan el día a día. ✅ **hecho 2026-06-13.**
3. **E1 + E2** (`:macro` + `%cN`): desbloquean el techo del extremo con
   núcleos ya escritos.
4. **E3 + E6** (`[rules]` + `:stats`): convierten el rc en plano de control.
5. **E4** (PTY persistente): sprint propio, coordinar con `shuma-gateway`
   (el cliente Android lo está esperando).
6. **E5** (LLM): al final, cuando las superficies deterministas ya estén —
   el modelo se monta sobre refs y tablas, no sobre texto plano.
