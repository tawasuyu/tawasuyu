# Migración: rename a `tawasuyu` + reorganización de dominios/git

> Pendientes acordados el 2026-06-07. **Nada de esto está ejecutado.** Se hará en la
> otra máquina cuando renueven los tokens. Este doc es la fuente para retomar allá.

## Principio rector

- **`gioser` = marca personal del dev (Sergio) + dominio `gioser.net`.** Se queda.
- **El monorepo englobador `gioser` → `tawasuyu`.** Tawantinsuyu = los cuatro *suyu*,
  que mapean a los cuatro cuadrantes (`00_unanchay`/`01_yachay`/`02_ruway`/`03_ukupacha`
  = PERCIBIR/CONOCER/HACER/RAÍZ). El nombre queda amarrado a la arquitectura.
- **`hammer` es HERMANO de `tawasuyu`, no hijo.** Es el piso (la distro Linux) que cobija
  la suite, no parte de ella. Mismo principio que "no meterlo al cargo workspace".

```
gioser            ← maker / umbrella (marca personal + gioser.net)
├── tawasuyu      ← la suite (este monorepo, ex-gioser)
└── hammer        ← la distro que la cobija (repo aparte ~/hammer)
```

## Pendiente 0 — SEGURIDAD (hacer primero, antes de cualquier push público)

El remote de este repo tiene la **password en claro** en la URL:

```
https://sergio:****@gitea.gioser.net/sergio/gioser.git
```

- [ ] Rotar esa contraseña de la cuenta Gitea.
- [ ] Sacarla del `.git/config` y usar token/credential helper:
  ```bash
  git remote set-url origin https://git.gioser.net/gioser/tawasuyu.git
  git config --global credential.helper store   # o un PAT, NO la password de la cuenta
  ```
- [ ] Verificar que ni reflogs ni `.git/config` viajen con la credencial al espejo público.

## Pendiente 1 — Dominios (bajo `gioser.net`, empezar barato)

No comprar dominios apex todavía (YAGNI; "hammer" es nombre muy pisado). Apex propio sólo
si un producto gana tracción independiente. Por ahora, todo subdominio de `gioser.net`:

| Subdominio | Qué |
|---|---|
| `gioser.net` | hub del maker / landing (`web/gioser-web`) |
| `tawasuyu.gioser.net` | la suite |
| `hammer.gioser.net` | la distro (**NO** `hammer.tawasuyu.net` — eso lo subordina) |
| `git.gioser.net` | Gitea (renombrar de `gitea.` a `git.`, canónico) |
| `docs.gioser.net` | SDDs renderizados (opcional) |

## Pendiente 2 — Git org / repos

- **Gitea (`git.gioser.net`) = canónico.** GitHub = espejo de un solo sentido (coherente
  con la filosofía self-sovereign / content-addressed).
- **Org `gioser`** (la marca), con repos hermanos:
  - `gioser/tawasuyu` (este monorepo, ex `sergio/gioser`)
  - `gioser/hammer` (la distro, ex `sergio/hammer`)
- **NO** org `tawasuyu` (chocaría con el repo) ni `tawasuyu/hammer` (re-acopla la distro
  a la suite).

Pasos aproximados (en la otra máquina, con tokens nuevos):
- [ ] Crear org `gioser` en Gitea.
- [ ] Renombrar/transferir `sergio/gioser` → `gioser/tawasuyu`.
- [ ] Transferir `sergio/hammer` → `gioser/hammer`.
- [ ] Configurar push-mirror de ambos hacia GitHub (org `gioser`, mismos nombres).

## Pendiente 3 — Rename interno del repo (cuando se decida ejecutar)

Esto es lo más invasivo; dejarlo para cuando haya tiempo de revisar:
- [ ] Decidir alcance: ¿sólo nombre del repo/remote, o también renombrar binarios/paths
      que llevan `gioser` (`gioser-web`, `gioser-edit` ya fue a `nada`, scripts
      `build-gioser-web.sh`, etc.)?
- [ ] El nombre `gioser` dentro del código/crates puede quedarse (es marca); el rename de
      arriba es de **repo + org + dominios**, no obliga a tocar crates.
- [ ] `cargo check --workspace` debe seguir pasando después de cualquier cambio.

## Lo que NO cambia

- La estructura de cuadrantes y el workspace de ~210 crates.
- `hammer` sigue siendo repo aparte (toolchain musl/`panic=abort` incompatible). Acople
  futuro = `tawasuyu`-como-recetas `.swm`, dependencia un solo sentido (hammer consume
  tawasuyu, no al revés).
