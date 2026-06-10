# auth — tawasuyu desktop authentication

Authentication domain: is this credential valid on this machine? Today a
single subcrate, **`auth-core`** (`brahman-auth`).

## Subcrates

- **`auth-core`** — backend-agnostic core. The consumer (the
  `mirada-greeter` greeter and any other) calls `Authenticator::authenticate`
  without knowing whether real PAM or a mock is behind it:
  - `Authenticator` (trait) + `AuthSession` (validated user) + `AuthError`.
  - `PamAuthenticator` — real PAM on Linux (feature `pam`, default).
  - `MockAuthenticator` — backend for tests/CI or builds without the feature.

It does not store users nor identity cards: the network identity lives in `card`.
This is strictly validating credentials against the host.

## Status (2026-05-31)

### Done
- Agnostic `Authenticator` trait with `AuthSession`/`AuthError`.
- `PamAuthenticator` (PAM/`pam-client`+`users`, feature `pam`) and
  `MockAuthenticator` for CI.
- Consumed by `mirada-greeter` (desktop login).
- Mock backend tests.

### Pending
- Real PAM coverage (today only the mock is exercised in CI).
- More methods (biometrics, token) behind the same trait.
- Integration with the `card`/`agora` identity session (today separate).

## Place in the repo

`shared/auth` — host credential validation. The network cryptographic identity
is `card` + `agora`.
