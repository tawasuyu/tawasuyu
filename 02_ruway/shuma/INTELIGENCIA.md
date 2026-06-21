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

### A2. Alias sugerido por longitud × frecuencia ✅ (2026-06-13)
Línea ≥ 40 chars repetida ≥ 3 veces idéntica → ofrecer alias corto. **Hecho:**
gemelo de A1 sobre **una sola línea** en vez de una secuencia.
`alias_suggestion` (`update/patterns.rs`) cuenta líneas idénticas del historial
(externas — los `:builtins` no se aliasan; el dedup `IgnoreConsecutive` ya
descarta las repes pegadas, así que cuenta las separadas por otro comando, que
es la buena señal), filtra por largo/umbral/descartadas/ya-aliasadas y rankea
por (veces, largo, lex). El nombre lo arma `suggest_alias_name`: iniciales de
los tokens no-flag (`git push origin feature…` → `gpof`), con sufijo numérico
si choca contra un alias o un binario del PATH (no pisa comandos del sistema).
`alias_chip` (`view/mod.rs`) lo ofrece sobre el input — **sólo si no hay
coreografía pendiente** (una oferta a la vez); «aliasar» llama `accept_alias`
(núcleo puro `learn_alias` → config viva + `upsert_key` al `[aliases]` del
shumarc, preservando comentarios), «descartar» lo calla en la sesión. Mismo
molde visual que A1, otra fuente. Verificado headless (`examples/alias_chip.rs`
→ PNG) + 6 tests (oferta, gates de largo/umbral/builtin/descartada/aliasada,
unicidad del nombre, línea de puras flags, aprendizaje a la config viva).

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

### A6. Aviso de comando largo terminado ✅ (2026-06-13)
Comando ≥ `[rules].on_long_command_secs` (default 30 s) que cierra mientras el
usuario está en otra sesión/diente → badge en el diente del rail + rastro en el
bloque. Nada de notificaciones del sistema: el chasis es la superficie. **Hecho
— por fin consume el `on_long_command_secs` que quedaba inerte:** módulo —
`register_long_command` (`update/run_exec.rs`, puro y testeable) corre al cerrar
cada comando externo; si `ended − block_started ≥ umbral` (`0` = apagado) suma a
`State.long_alerts` y deja un notice `⏲ comando largo — terminó tras Ns` en el
bloque. `State::long_alerts()`/`ack_long_alerts()` lo exponen. Chasis
(`shuma-shell-llimphi`) — `Session::long_alerts()`/`ack_long_alerts()` puentean
al módulo; `session_tooth_icon` gana un parámetro `alert` que pinta un **punto
ámbar con halo** en la esquina opuesta al LED verde, **sólo en sesiones no
activas** (`!activa && long_alerts() > 0`); se acusa al `SelectSession` (volvés
a mirarla) y por `ShellTick` sobre la sesión activa (un comando largo en primer
plano no deja badge stale al cambiar de diente). Verificado: 4 tests del módulo
(suma con umbral, corto no alerta, umbral-0 apaga, ack limpia) + build release
del chasis. La badge en sí es espejo del LED de actividad ya existente.

## B — El nerdo extremo: superficies direccionables y programables

Principio: el shell expone sus entrañas como **datos direccionables** y
**puntos de enganche declarativos**. Nada se ofrece solo: todo se invoca.

### E1. Macros con parámetros (`:macro`) — darle UI al MacroBook ✅ (2026-06-13)
**Hecho:** builtin `:macro` en `update/builtins.rs` sobre el `MacroBook` ya
existente — `:macro save deploy cargo build --bin %1 && scp %1 %2:/srv`,
`:macro run deploy app host` instancia (`substitute_macro_params`: `%1..%9` +
`%*`, `instantiate_macro` une los pasos con `&&` y reusa `run_submitted`),
`:macro rm`, `:macros`/`:macro list`. Persistencia en
`~/.config/shuma/macros.toml` (`load/save_macro_book`, atómico tmp+rename;
`shuma_config::macros_path`); `State.macro_book` cargado al arrancar. Es el
ascensor de A1: el patrón emergente se promociona a macro con parámetros
explícitos. 3 tests (sustitución, instanciación multipaso, macro inexistente).

### E2. El scrollback como base de datos (`%cN` en la línea) ✅ (2026-06-13)
**Hecho:** `resolve_injects` (`update/run_exec.rs`) parsea la línea con
`shuma_intent::Intention`; una etapa-ref `%cN`/`%pN` materializa el stdout del
bloque `N` (`gather_block_stdout`) como **stdin** del resto del pipeline —
`%c12 | grep error | sort` corre `grep error | sort` sobre el bloque 12; `%c12`
solo se re-muestra con `cat`. Tiene prioridad sobre el reprocess del chip
`» stdin` (su caso degenerado). Tag `%cN` clickeable en el header
(`surface_header` + `Msg::InsertBlockRef`) hace visible el número y lo inserta
al input. Combinado con las secciones-tabla, un `ls -l` viejo es una tabla
consultable. 4 tests (ref como fuente, ref sola→cat, %pN, línea sin ref).

### E3. Reglas declarativas en el rc (`[rules]`) — el plano de control ✅ (2026-06-13)
El shumarc gana gatillos deterministas (`shuma_config::RulesConfig`):

```toml
[rules]
on_exit_nonzero = ":jobs"                 # qué correr cuando algo falla
on_pattern_score = 3                      # umbral de A1 (0 = nunca ofrecer)
on_long_command_secs = 30                 # umbral de A6 (aún sin consumidor)

[rules.on_enter_cwd]
"~/proyectos/wawa" = ":env RUST_BACKTRACE=1"
```

**Hecho:** `on_exit_nonzero` corre el comando declarado cuando un comando
externo cierra con exit ≠ 0 (guarda `exit_rule_fired` re-armada por submit
del usuario → el propio comando de la regla no la re-dispara). `on_enter_cwd`
(mapa prefijo→comando, `~` expandido, gana el prefijo más largo;
`RulesConfig::command_for_cwd`) corre al `cd` local exitoso (guarda
`in_cwd_rule` contra recursión). `on_pattern_score` gobierna el umbral de A1
(`choreography_suggestion`; `0` lo apaga). Motor: match determinista en
`update`/`apply_cd`, sin DSL turing-completo. `on_long_command_secs` queda
declarable pero inerte hasta A6. Verificado: 1 test en shuma-config
(matching + más-específico-gana) + 2 en shuma-module-shell (on_exit_nonzero
una-sola-vez, on_enter_cwd dispara).

### E4. Flota persistente (daemon attach/detach) ✅ (2026-06-13)
El daemon ya tenía el **registro de sesiones PTY persistentes**
(`pty_sessions::PtyRegistry`: spawn/attach/list/kill, ring de scrollback +
broadcast, desacoplado de la conexión) y el protocolo
(`PtySpawn`/`PtyAttach`/`PtyList`/`PtyKill`); faltaba el **cliente para el
nerdo de terminal**. **Hecho:** `shuma pty {spawn,ls,attach,kill}` en
`shuma-cli`. `attach` es un cliente full-duplex real: terminal en raw
(`RawGuard` con restauración en Drop), teclas → `PtyInput`, SIGWINCH →
`PtyResize`, `ExecBytes` → stdout; **Ctrl-]** desadjunta sin matar la sesión.
Verificado end-to-end contra el daemon: spawn persiste entre invocaciones,
attach hace round-trip (eco de `cat`), detach deja la sesión `viva`. Bonus:
arreglado el detach idle en `handle_pty_attach`/`_enc` (un `select!` sobre la
tarea lectora corta el writer al instante → el `attached` baja a 0 sin
esperar tráfico).

**Pulido — el shell sobre sesiones del daemon** ✅ (2026-06-13):
`shuma-remote-exec` ganó `spawn_session`/`attach_session`/`list_sessions`/
`kill_session` (el `RemoteRunHandle` que devuelven es idéntico al de
`run_pty`, así el shell las rinde igual). El shell tiene builtins
`:spawn <cmd>` (corre en el daemon, **sobrevive a cerrar shuma**, se adjunta
y rinde como TUI), `:sessions` (lista), `:attach <id>` (re-adjunta), y
`:kill-session <id>`. Cerrar shuma = detach (la sesión vive); reconectás con
`:attach` o `shuma pty attach`. Verificado e2e contra daemon vivo
(`examples/session_smoke`: spawn→list→attach lee scrollback→detach-sigue-viva
→kill).

**Cliente móvil vía gateway ✅ (2026-06-21):** `shuma-gateway` sirve `GET /term`
— una página HTML autocontenida (xterm.js de CDN pinneado) pensada para un
teléfono en la misma red. Lista las sesiones por `POST /rpc` (`"PtyList"`),
adjunta a una (o crea) por el WebSocket `/ws/pty` (primer msg JSON
`{"session":id,rows,cols}` o `{"program",args,…}`; binarios = stdin/salida;
`{"t":"resize",…}`), con botones Abrir/Matar/Nueva. El token (si el gateway lo
exige) va en `?token=…`: el JS lo manda como `Authorization: Bearer` a `/rpc` y
como `?token=` al WS. La página en sí no requiere auth (no tiene secretos; el
gateo está en /rpc y /ws/pty). Servido verificado por curl
(`text/html`, 8.9 KB, markers del protocolo); el flujo terminal en vivo pide
daemon + navegador real. Quedaría como follow-up vendorizar xterm.js para uso
100% offline-LAN. **E4 cerrado del todo.**

### E5. LLM como instrumento invocado (`:?`) ✅ (2026-06-13)
**Hecho** con `pluma-llm` (backend por env, Mock sin credenciales):
- `:? <pregunta>` — lenguaje natural → línea de comando propuesta, **al
  input** (NUNCA auto-ejecutada; revisar y Enter).
- `:explica [%cN]` — explica la salida de un bloque (la del más reciente si
  no se da ref).
- `:resume [%cN]` — resumen narrativo, para logs gigantes (el cuerpo se capea
  por cabeza+cola si excede 8k).
Siempre rotulado `🜲`, opt-in por invocación. **Arquitectura (Regla 2):** el
módulo `shuma-module-shell` sólo expresa la intención (`State::llm_request`,
sin dependencias de red); el **chasis** la toma (`take_llm_request`), corre
`pluma-llm` en un thread con su runtime y devuelve `Msg::LlmResult`. Sin
credenciales `from_env` cae a Mock (responde igual, nunca cuelga). El LLM se
monta sobre las refs `%cN` y el scrollback ya deterministas — no sobre texto
plano. Verificado: 3 tests del módulo (petición armada, tomada una sola vez,
resultado al input/output); el chasis compila con el stack LLM.

### E6. `:stats` — telemetría propia, local, consultable ✅ (2026-06-13)
**Hecho:** builtin `:stats [filtro]` (`update/builtins.rs`) sobre el historial
durable (`line`/`exit`/`started`/`duration_ms`). Agrega por binario (primera
palabra; los `:` builtins se omiten) → veces, fallos, %fallo, p50/p95 de
duración, último uso (`hace Nm/Nh/Nd`); resumen con total, distintos, con-exit
y hora pico (UTC). Corazón puro `compute_stats(entries, filtro, now_s)` →
líneas; emite 1 línea de resumen sin tab + tabla tab-separada que
`sections::detect_stats` reconoce y parte en sección «resumen» (Lines) +
«por comando» (Table **ordenable**, el mismo widget que `ls -l`; columna
`comando` ensanchada en `section_table_view`). `:stats foo` filtra a binarios
que contienen `foo`. Cero red: los datos no salen de la máquina; alimenta los
rankings de A3/A4. Verificado headless (`examples/stats_e6.rs` → PNG) + 4 tests
(agregación con fallos/percentiles, filtro + None, round-trip detector,
`humanizar_hace`).

## Orden propuesto

1. **A5 + A1** (titular semáforo + chip de coreografía): máximo efecto/LOC,
   todo el material ya está en memoria. ✅ **hecho 2026-06-13.**
2. **A3 + A4** (ghost por cwd + quisiste-decir): afinan el día a día. ✅ **hecho 2026-06-13.**
3. **E1 + E2** (`:macro` + `%cN`): desbloquean el techo del extremo con
   núcleos ya escritos. ✅ **hecho 2026-06-13.**
4. **E3 + E6** (`[rules]` + `:stats`): convierten el rc en plano de control.
   ✅ **ambos hechos 2026-06-13.**
5. **E4** (PTY persistente): cliente `shuma pty` ✅ **hecho 2026-06-13**
   (daemon + gateway ya estaban). Pulido pendiente: shell Llimphi sobre
   sesiones del daemon; cliente móvil vía gateway.
6. **E5** (LLM): ✅ **hecho 2026-06-13** — montado sobre refs/tablas, no
   sobre texto plano, como decía el plan.

---

**Roadmap COMPLETO (2026-06-13):** A1·A2·A3·A4·A5·A6 + E1·E2·E3·E4·E5·E6 ✅ —
toda la lista de inteligencia, cerrada. El pulido de E4 (cliente móvil vía
gateway) quedó cerrado el 2026-06-21 con `GET /term`. **Sin pendientes.**
