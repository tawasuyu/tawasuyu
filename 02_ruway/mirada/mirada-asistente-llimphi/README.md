# mirada-asistente — conversational assistant for the mirada desktop

A Llimphi app that translates **natural language** into **`mirada-ctl`
commands** by querying an LLM. The AI proposes; the human confirms before
executing.

## What it does, in one sentence

You type "send this window to desktop 3", the LLM returns
`mirada-ctl send-to-workspace 3` with an explanation. You press **Execute** and
the assistant spawns the command. It signs nothing, it does not touch the
`mirada-brain` socket directly — it goes through the public CLI so that an
auditor sees the same events a human would by typing.

## How to start it

```bash
cargo run -p mirada-asistente-llimphi --release
```

With no environment variables it falls back to the **Mock** backend (pluma-llm)
and returns fixed responses — useful for testing the UI without spending tokens.

To query a real LLM, `pluma-llm::from_env()` autodetects the first one that has
a credential:

| Variable                                | Backend     |
|-----------------------------------------|-------------|
| `ANTHROPIC_API_KEY`                     | Anthropic   |
| `GEMINI_API_KEY` / `GOOGLE_API_KEY`     | Gemini      |
| `DEEPSEEK_API_KEY`                      | DeepSeek    |
| `COHERE_API_KEY`                        | Cohere      |
| `PLUMA_LLM_BACKEND=ollama`              | Ollama local|

To force a specific one: `PLUMA_LLM_BACKEND=anthropic` (or whichever)
overrides auto-detection.

The assistant **needs** `mirada-ctl` to be on `PATH` to execute.
If it is not, the spawn fails with a legible message and the operator can
install it (`cargo install --path 02_ruway/mirada/mirada-ctl` or equivalent).

## Keyboard shortcuts

| Key      | Action                                |
|----------|---------------------------------------|
| Enter    | Sends the question to the LLM         |
| Esc      | Clears the question and discards state |
| Mouse    | Type normally; click on buttons       |

## Flow

```
[1] you type a question     "send this window to workspace 3"
              ↓ Enter
[2] query the LLM           pluma-llm → backend → JSON response
              ↓
[3] visible proposal        "mirada-ctl send-to-workspace 3"
                            + explanation
              ↓ Execute
[4] spawn mirada-ctl        captures stdout+stderr
              ↓
[5] visible result          ✓ send-to-workspace executed
```

At any step, **Discard** (or Esc) returns to the initial state without
executing anything.

## Security model

The AI **exercises no capabilities**. It only produces a proposal visible to
the operator. The "execute" step is always a human act: until you
press the button, the compositor stays intact. This is deliberate:
destructive actions (`quit`, `close-focused`) are shown all the same with their
explanation, and we let you decide.

The assistant **goes through the CLI** `mirada-ctl` so that any subsequent
audit — process logs, shell history, daemon monitoring — sees
exactly the same events it would if you had typed them by hand.
There is no side channel to the brain's socket.

For actions that `mirada-ctl` does not expose (re-anchoring manifests, managing
secrets), the assistant **does not propose them**: the list of actions is
in the system prompt and limited to the CLI's subcommands.

## Tests

```bash
cargo test -p mirada-asistente-llimphi
```

They cover the JSON parser logic (15 tests): markdown fences around it,
prose before and after, nested JSON, explicit LLM refusal, unknown
JSON, empty action, etc. Pure logic — they run without a graphical
environment or network.

## Compositor context

Before each query, the assistant tries to spawn `mirada-ctl
windows` and embeds its output in the system prompt as "Current state of the
compositor". That lets the LLM respond with concrete values
(`focus-window 5` with the real id, not made up). If the spawn fails
(compositor down, `mirada-ctl` not on PATH), we continue with the base prompt
and the LLM responds "blind" — the flow does not break, it only loses
precision.

## Known limitations

- **No multi-turn.** Each query is independent; no context is kept
  between requests. If you want to refine ("no, I'd rather grid"), you have
  to reformulate the whole question. Expandable, not urgent.
- **The `mirada-ctl` binary must be on PATH** both to execute
  actions and to obtain context. If not, they fail legibly but
  the assistant does not try other routes.
- **The context is re-read on every query** — one extra spawn per
  question. Trivial against the LLM's RTT, but measurable if the user
  asks a hundred things in a row.

## wawa version

There is a technical design in `docs/ASISTENTE_WAWA.md` for porting this
pattern to the bare-metal kernel. The pieces (`asistente.wasm` app,
Akasha↔HTTP bridge, human signing via `daemon-firma`) are described; the code
is pending.

## Style

Comments and commit messages in Spanish (the repo's convention).
UI strings via `rimay-localize` (ES/EN/QU). To add a
locale: edit the `.ftl` files in `shared/rimay-localize/locales/`.
