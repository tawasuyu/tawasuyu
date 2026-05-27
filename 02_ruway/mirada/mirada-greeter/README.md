# mirada-greeter

> Greeter (login screen) of [mirada](../README.md)'s desktop.

Runs from TTY, asks for credentials, validates against the system user database, then launches the user's session. Uses Llimphi for the visual; no PAM dependency — uses the same auth model as the rest of the monorepo.

## Usage

```sh
cargo run --release -p mirada-greeter
```

## Deps

- [`llimphi-ui`](../../llimphi/) + widgets `text-input`, `button`
- [`shared/auth/auth-core`](../../../shared/auth/auth-core/LEEME.md)
