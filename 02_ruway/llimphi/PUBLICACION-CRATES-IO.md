# llimphi en crates.io — estado y decisión

> Evaluación 2026-06-24: *"¿vale la pena el crates.io de llimphi?"*

## TL;DR

**Ya está publicado y el modelo es el correcto.** `llimphi` vive en crates.io
desde la **v0.1.1 (2026-06-19)** — "Native Rust UI framework — 2D and 3D",
facade sobre `vello` + `wgpu` + `taffy` + `parley` + motor voxel. **No** hay que
publicar el monorepo: se mantiene desde el repo-extracto standalone
[`git.tawasuyu.net/tawasuyu/llimphi`](https://git.tawasuyu.net/tawasuyu/llimphi).
El workspace de tawasuyu sigue `publish = false`, y debe seguir así.

## Por qué el monorepo NO se publica

`02_ruway/llimphi/` son **100 crates** (`llimphi-{hal,raster,layout,text,ui,
compositor,theme,motion,surface,svg,image,icons,3d,voxel,…}` + ~80 widgets + 11
módulos), todos cableados entre sí por `path`. Publicar eso a crates.io exigiría:

- **Publicar los 100 en orden topológico** (un crate no puede subir antes que sus
  dependencias), y republicar la cadena entera en cada release.
- **Versionar cada `path` dep** (crates.io rechaza deps sólo-path) y sostener
  **semver** sobre una API que todavía se mueve rápido ("feo pero sirve").
- **Ocupar ~100 nombres** `llimphi-*` en el registro central — justo lo opuesto a
  la postura self-sovereign / content-addressed de la suite.

Costo alto, beneficio nulo: nadie consume `llimphi-hal` suelto.

## El modelo que sí se usa (y funciona)

Un **único crate consolidado `llimphi`** (la *front-door*), extraído al repo
standalone y publicado desde ahí. Es el mismo patrón que el README raíz ya
declara para los extractos (llimphi, mirada, …). El consumidor externo obtiene
un `cargo add llimphi` que le da el framework completo (2D + 3D) detrás de una
fachada, sin arrastrar la topología interna del monorepo.

## Recomendación

1. **Mantener** el modelo: un solo crate `llimphi` desde el repo-extracto. No
   tocar `publish = false` en el workspace de tawasuyu.
2. **Cortar una 0.1.2** desde el repo standalone cuando valga sincronizarlo con
   lo nuevo del monorepo posterior al 19-jun: selección por doble/triple-click en
   `text-input`, `llimphi-image` (decode → `peniko::Image`), `dock-rail`, y el uso
   de `llimphi-3d` por la esfera celeste de cosmos. Es decisión de cadencia, no de
   arquitectura.
3. **Checklist de release** (en el repo standalone, no acá): bump de versión,
   `description`/`keywords`/`categories`/`readme` al día, `cargo publish --dry-run`,
   y nota de cambios. La metadata compartida del monorepo (`license`, `authors`,
   `repository`) ya está en `[workspace.package]`; lo específico de publicación
   vive en el extracto.
