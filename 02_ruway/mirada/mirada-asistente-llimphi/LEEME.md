# mirada-asistente — asistente conversacional del escritorio mirada

App Llimphi que traduce **lenguaje natural** a **comandos de `mirada-ctl`**
consultando un LLM. La IA propone; el humano confirma antes de ejecutar.

## Qué hace, en una frase

Escribís «manda esta ventana al escritorio 3», el LLM te devuelve
`mirada-ctl send-to-workspace 3` con explicación. Pulsás **Ejecutar** y
el asistente spawnea el comando. No firma nada, no toca el socket de
`mirada-brain` directamente — pasa por la CLI pública para que un
auditor vea los mismos eventos que verá un humano tipeando.

## Cómo arrancarlo

```bash
cargo run -p mirada-asistente-llimphi --release
```

Sin variables de entorno cae al backend **Mock** (pluma-llm) y devuelve
respuestas fijas — útil para probar la UI sin gastar tokens.

Para consultar un LLM real, `pluma-llm::from_env()` autodetecta el
primero que tenga credencial:

| Variable                                | Backend     |
|-----------------------------------------|-------------|
| `ANTHROPIC_API_KEY`                     | Anthropic   |
| `GEMINI_API_KEY` / `GOOGLE_API_KEY`     | Gemini      |
| `DEEPSEEK_API_KEY`                      | DeepSeek    |
| `COHERE_API_KEY`                        | Cohere      |
| `PLUMA_LLM_BACKEND=ollama`              | Ollama local|

Para forzar uno en concreto: `PLUMA_LLM_BACKEND=anthropic` (o el que sea)
sobreescribe la auto-detección.

El asistente **necesita** que `mirada-ctl` esté en `PATH` para ejecutar.
Si no lo está, el spawn falla con un mensaje legible y el operador puede
instalarlo (`cargo install --path 02_ruway/mirada/mirada-ctl` o equivalente).

## Atajos de teclado

| Tecla    | Acción                                |
|----------|---------------------------------------|
| Enter    | Manda la pregunta al LLM              |
| Esc      | Limpia la pregunta y descarta estado  |
| Mouse    | Tipear normalmente; clic en botones   |

## Flujo

```
[1] tipeás pregunta         "manda esta ventana al workspace 3"
              ↓ Enter
[2] consulta al LLM         pluma-llm → backend → respuesta JSON
              ↓
[3] propuesta visible       "mirada-ctl send-to-workspace 3"
                            + explicación
              ↓ Ejecutar
[4] spawn mirada-ctl        captura stdout+stderr
              ↓
[5] resultado visible       ✓ send-to-workspace ejecutado
```

En cualquier paso, **Descartar** (o Esc) vuelve al estado inicial sin
ejecutar nada.

## Modelo de seguridad

La IA **no ejerce capacidades**. Sólo produce una propuesta visible para
el operador. El paso de "ejecutar" es siempre un acto humano: hasta que
pulses el botón, el compositor sigue intacto. Esto es deliberado:
acciones destructivas (`quit`, `close-focused`) las muestra igual con su
explicación, y dejamos que vos decidas.

El asistente **pasa por la CLI** `mirada-ctl` para que cualquier auditoría
posterior — logs de proceso, history shell, monitoring de daemons — vea
exactamente los mismos eventos que vería si los hubieras tipeado a mano.
No hay un canal lateral al socket del brain.

Para acciones que `mirada-ctl` no expone (re-anclar manifiestos, gestionar
secretos), el asistente **no las propone**: la lista de acciones está
en el system prompt y limitada a los subcomandos del CLI.

## Tests

```bash
cargo test -p mirada-asistente-llimphi
```

Cubren la lógica del parser JSON (15 tests): markdown fences alrededor,
prosa antes y después, JSON anidado, rechazo explícito del LLM, JSON
desconocido, acción vacía, etc. Lógica pura — corren sin entorno gráfico
ni red.

## Contexto del compositor

Antes de cada consulta, el asistente intenta spawnear `mirada-ctl
windows` y embebe su salida en el system prompt como "Estado actual del
compositor". Eso le permite al LLM responder con valores concretos
(`focus-window 5` con el id real, no inventado). Si el spawn falla
(compositor caído, `mirada-ctl` no en PATH), seguimos con el prompt base
y el LLM responde "a ciegas" — el flujo no se rompe, sólo pierde
precisión.

## Limitaciones conocidas

- **Sin multi-turn.** Cada consulta es independiente; no se mantiene
  contexto entre pedidos. Si querés refinar ("no, prefiero grid"), tenés
  que reformular la pregunta entera. Ampliable, no urgente.
- **El binario `mirada-ctl` debe estar en PATH** tanto para ejecutar
  acciones como para obtener contexto. Si no, fallan legiblemente pero
  el asistente no intenta otras rutas.
- **El contexto se relee en cada consulta** — un spawn extra por
  pregunta. Trivial frente al RTT del LLM, pero medible si el usuario
  pregunta cien cosas seguidas.

## Versión wawa

Existe un diseño técnico en `docs/ASISTENTE_WAWA.md` para portar este
patrón al kernel bare-metal. Las piezas (app `asistente.wasm`, puente
Akasha↔HTTP, firma humana vía `daemon-firma`) están descritas; el código
está pendiente.

## Estilo

Comentarios y mensajes de commit en español (convención del repo).
Strings de UI a través de `rimay-localize` (ES/EN/QU). Para agregar una
locale: editar los `.ftl` en `shared/rimay-localize/locales/`.
