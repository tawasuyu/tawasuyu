# churay — graphical installer/updater for the suite

*Léelo en español: [LEEME.md](LEEME.md).*

"churay" (Quechua: *to place / to install*). An Office-style installer for
tawasuyu on **any Linux**: pick apps from a catalog, click Install, and they
land with their `.desktop` entry in the system menu. With a built-in **updater**.

## Architecture decision (2026-06-24)

Three axes pinned down the design:

1. **Sides A + B.** The installer ships **precompiled** binaries from the bundle
   when they exist (instant, Office-style install) and falls back to
   `cargo build --release --bin <prog>` when they don't (dev, with the repo
   present). Same UI, same flow; each unit decides based on whether a binary
   exists.
2. **System mode (root) + local.** **System** mode → `/usr/local`, asks for
   root, and includes heavy components like `arje` (init). **Local** mode →
   `~/.local`, no root, apps only. Each unit carries a `Scope` (`App`/`System`)
   that decides where it may go.
3. **Package layer: hybrid.** The expensive part is the package architecture,
   and `~/hammer` already has a model (BLAKE3 CAS + ed25519). Rather than couple
   the two repos' builds, we **vendor** hammer's hash type (`ArtifactHash`,
   identical `b3:…` format → future interop under a unified CAS) and use
   tawasuyu's **own** primitives for the rest: ed25519 signing via `agora-core`.
   hammer-build/overlay (bwrap, zig, overlayfs, root) does **not** fit a user
   installer, so it isn't used.

## Crates

- **`churay-core`** — the frontend-agnostic engine. Catalog (from the repo's
  single app table, `app-bus`), signed manifest, atomic install, a registry of
  what's installed, and update checking.
  - `catalog` — `suite_catalog()`: the `Exec` apps from `app-bus` + `arje`.
  - `manifest` — `Unit`/`Manifest`/`SignedManifest` (BLAKE3 CAS + ed25519).
  - `install` — `Source` (trait: bundle / build), `install_unit`, `.desktop`,
    `InstallMode`, atomic install (`.tmp` + rename), `uninstall_unit`.
  - `state` — `installed.json` under `<prefix>/share/tawasuyu/`.
  - `update` — `check_updates` / `pending_updates`.
  - `hash` — `ArtifactHash` vendored from hammer.
  - `repo` — `RemoteRepo` (a `Source`) + `fetch_signed_manifest`: a signed
    remote repo served over HTTP; downloads binaries **by hash**
    (`blobs/<hex>`), verifies BLAKE3, and caches them. Transport via the
    `Fetcher` trait (curl in production, local in tests). This is what closes
    the online updater.
  - bin **`churay-bundle`** — forges the precompiled bundle + signed manifest.
    Also emits `blobs/<hex>`: **a bundle served over HTTP is a remote repo**.
  - bin **`churay-cli`** — headless frontend (servers/scripts/CI):
    `list` · `check` · `install <id…>` · `update [<id…>]` · `uninstall <id…>`.
- **`churay-llimphi`** — the GUI (bin `churay`): a **welcome screen** with the
  suite logo (from `marca`) and a "don't show again" checkbox (persisted in
  `Prefs`); a catalog with checkboxes by quadrant, a mode selector, **per-app
  progress with a stage label** (downloading/compiling/copying — no longer stuck
  at 0%), a **suggestions banner** (pata ↔ shuma), a **source notice** (warns if
  something will be compiled or if there's nothing to install from), a **result
  screen** (what got installed, where, an **Open** button, and suggestions), an
  updates tab that checks against the remote repo, and a "Reopen as root" button.

**System-agnostic**: building from source is only offered if there's `cargo` + a
workspace (dev mode). A user system without cargo installs from a bundle or a
remote repo; if there's neither, it says so plainly instead of trying to compile.

Visual identity is centralized in **`shared/marca`**: logo + name + tagline +
accent for the suite/hammer/wawa, with a disk override (`$TAWASUYU_MARCA` or
`~/.config/tawasuyu/marca/<suite|hammer|wawa>.png`) to rebrand without recompiling.

## Usage

```bash
# installer (dev: builds whatever you pick, from the workspace)
cargo run -p churay-llimphi

# forge the precompiled bundle (side A) and sign it
export CHURAY_SIGN_SEED=$(head -c32 /dev/urandom | xxd -p -c64)
scripts/build-tawasuyu-bundle.sh dist/tawasuyu-bundle

# install against a bundle, no compiling
CHURAY_BUNDLE=$PWD/dist/tawasuyu-bundle cargo run -p churay-llimphi

# serve the bundle as a remote repo and update online (headless)
( cd dist/tawasuyu-bundle && python3 -m http.server 8080 ) &
CHURAY_REPO=http://localhost:8080 churay-cli --local check
CHURAY_REPO=http://localhost:8080 churay-cli --local install cosmos nada
CHURAY_REPO=http://localhost:8080 churay-cli update      # downloads what changed
```

Envs: `CHURAY_BUNDLE` (bundle dir), `CHURAY_WORKSPACE` (root to compile from),
`CHURAY_REPO` (signed remote repo), `CHURAY_MODE=system|local`,
`CHURAY_SIGN_SEED` (bundle signing seed).

Source priority on install: **local bundle → remote repo → compile**.

## Pending

- A fully portable bundle (static musl / AppImage) for GPU apps. Today the
  bundle is dynamic (comparable glibc).
- Trust anchored by default: today the signature self-verifies; seeding the
  publisher's public key to require it is still pending
  (`SignedManifest::verify(Some(&k))` already supports it).
