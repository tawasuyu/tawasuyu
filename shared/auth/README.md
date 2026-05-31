# auth — autenticación del escritorio gioser

Dominio de autenticación: ¿esta credencial es válida en esta máquina? Hoy un
solo subcrate, **`auth-core`** (`brahman-auth`).

## Subcrates

- **`auth-core`** — núcleo agnóstico del backend. El consumidor (greeter
  `mirada-greeter` y cualquier otro) llama a `Authenticator::authenticate`
  sin saber si detrás hay PAM real o un mock:
  - `Authenticator` (trait) + `AuthSession` (usuario validado) + `AuthError`.
  - `PamAuthenticator` — PAM real en Linux (feature `pam`, default).
  - `MockAuthenticator` — backend para tests/CI o builds sin la feature.

No guarda usuarios ni tarjetas de identidad: la identidad de red vive en `card`.
Esto es estrictamente validar credenciales contra el host.

## Estado (2026-05-31)

### Hecho
- Trait `Authenticator` agnóstico con `AuthSession`/`AuthError`.
- `PamAuthenticator` (PAM/`pam-client`+`users`, feature `pam`) y
  `MockAuthenticator` para CI.
- Consumido por `mirada-greeter` (login del escritorio).
- Tests del backend mock.

### Pendiente
- Cobertura de PAM real (hoy sólo se ejercita el mock en CI).
- Más métodos (biometría, token) tras el mismo trait.
- Integración con la sesión de identidad de `card`/`agora` (hoy separados).

## Lugar en el repo

`shared/auth` — validación de credenciales del host. La identidad criptográfica
de red es `card` + `agora`.
