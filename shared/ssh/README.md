# ssh — minimal SSH client for tawasuyu

A `russh` wrapper reduced to what tawasuyu needs: **encrypted transport +
authentication + a command-execution channel**. It doesn't aim to cover all of
OpenSSH; the real interactive shell will be covered by `shuma`
(see `project_shuma_shell_roadmap`).

## What it exposes

- `Client` — connects, authenticates and runs remote commands (async, over tokio).
- `Config` — connection parameters.
- `Auth` — authentication method (key / password).

## Non-goals (today)

- It is not a replacement for OpenSSH nor mosh/tmux.
- It doesn't do session multiplexing or reconnection.

## Status (2026-05-31)

### Done
- `Client`: connection, authentication (key/password) and command exec.
- Async API over tokio + configuration/error types.

### Pending
- `Server` (accept connections + exec handler) — only mentioned, not implemented.
- Interactive PTY, port-forwarding and SFTP.
- Multiplexing / reconnection (deferred to `shuma`).
- Integration tests (today no automated coverage).

## Place in the repo

`shared/ssh` — minimal SSH client. The remote shell experience is integrated by
`shuma`.
