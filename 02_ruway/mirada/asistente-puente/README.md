# asistente-puente — Linux bridge between wawa and the external LLMs

Receives queries from the `asistente.wasm` app (wawa kernel, via Akasha),
translates them into an LLM query with `pluma-llm` autodetect, and returns
an interpreted proposal ready to present to the human.

## Status: three transport modes

The binary offers three modes depending on the command-line flag:

- **stdio** (default, no args): a single turn
  `Consulta → Propuesta/Error` over stdin/stdout. Useful for tests or
  exercises with `printf` + `xxd`. Payload: `MensajeAsistente` postcard
  with a `u32 LE` prefix.
- **daemon Unix socket** (`--socket <path>`): listen + accept serially,
  each client can send N turns until EOF. Useful so the
  Linux assistant can query it without launching a process per question. Same
  payload as stdio.
- **Akasha** (`--akasha <iface>`): bind to `AF_PACKET SOCK_DGRAM` over the
  physical interface, filtered by `ETHERTYPE_ASISTENTE = 0x88B6`. Short binary
  payload (`format::TipoCable`: 12 B header + type-specific
  bytes). It is the protocol that `asistente.wasm` speaks from the
  wawa kernel. Requires permissions to open AF_PACKET (cap_net_raw, root, or
  `setcap cap_net_raw=ep target/release/asistente-puente`).

## What already works

- **Pure tested logic** (`src/lib.rs`, 12 tests): translation
  of LLM JSON → `AccionPropuesta`, explicit system prompt, user
  prompt that pastes in the `Contexto` received from the kernel + the human
  question. No network, no graph — the hottest block of the bridge.
- **Stub binary** (`src/main.rs`): a single turn of
  `Consulta → Propuesta/Error`. Initializes pluma-llm from the env;
  with no credentials it falls back to Mock.

## Phase 60 v4 :: human signing of hash proposals

To the `--akasha` mode you can add three optional flags so that
THIS binary signs the proposals the app pressed SPACE to
authorize:

```bash
target/release/asistente-puente --akasha eth0 \
    --firma-clave ~/.config/wawa/operador.sk \
    --firma-slot 0 \
    --firma-log ~/asistente_puente_audit.log
```

- `--firma-clave PATH`: Ed25519 key (32 B seed or 64 B SecretKey).
  Interchangeable with the one `wawactl daemon-firma` already loads.
- `--firma-slot N`: slot of the `AGORA_AUTH_RING` ring (0/1/2). Default 0.
- `--firma-log PATH`: audit append. Default
  `asistente_puente_audit.log` in the CWD.

Flow: when a `TipoCable::RequestFirma` arrives over the wire (33 B
payload = `[tipo_obj: u8, hash: [u8;32]]`), the bridge prints to
stderr the HASH + type (CUADERNO/CONFIGURACION) and waits for `y` on stdin
(30 s timeout, identical to `daemon-firma`'s). If authorized, it signs with
the loaded key and returns a `TipoCable::Firma` with
`[slot, firma 64 B]`. Without `--firma-clave`, every `RequestFirma` bounces
with a `TipoCable::Error("PUENTE SIN CLAVE: ...")` and the app shows it.

## What's missing

- For `InstalarApp` / `CambiarConfiguracion`: emit the
  `Manifiesto` / `Configuracion` object over the graph (another Akasha frame) before
  proposing its hash. Today the LLM can invent hashes — the kernel will
  reject them on verification, but we should catch it here first.
- Multiplexing between nodes: today the `--akasha` mode responds to the
  broadcast; a node receives responses directed at *any* node on
  the same network (it filters them by `id` in the `asistente.wasm` app).
  Improvable with a unicast sendto to the sender that `recvfrom` revealed.
- Node context: the v3 `Consulta` travels without `Contexto` (available
  apps, current manifest). The bridge builds an empty `Contexto::default()`.
  When v4 adds the context to the wire payload, the bridge passes it
  to the LLM and the proposals can refer to real apps.

## Trying it locally

stdio mode, no credentials (falls back to Mock):

```bash
# You need a helper that writes postcard. For a quick test:
cargo run -p asistente-puente -- --help
```

With real credentials, any of the ones `pluma-llm` autodetects:

```bash
ANTHROPIC_API_KEY=sk-... cargo run -p asistente-puente < consulta.bin
```

daemon mode on a Unix socket:

```bash
cargo run -p asistente-puente -- --socket /tmp/asistente.sock
```

Any client that opens that socket and emits postcard frames can
query it.

Akasha mode over a physical interface:

```bash
# Release build and grant the capability without sudo (preferred):
cargo build -p asistente-puente --release
sudo setcap cap_net_raw=ep target/release/asistente-puente
target/release/asistente-puente --akasha eth0

# Or directly with sudo:
sudo cargo run -p asistente-puente --release -- --akasha eth0
```

This mode binds an `AF_PACKET SOCK_DGRAM` socket to the indicated
interface, filtered by `EtherType 0x88B6`. Every `Consulta` that
`asistente.wasm` emits from a wawa node on the same network arrives here,
is translated into a prompt for the LLM, and the response returns by broadcast
on the same EtherType.

## Full design

See `docs/ASISTENTE_WAWA.md`.
