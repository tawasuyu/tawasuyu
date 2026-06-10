# foreign-fs

Bridge from **foreign file systems** into wawa's native graph (BLAKE3 + DAG
+ postcard). Reads a foreign FS —today FAT12/16/32 and ext2/3/4— directly
over raw bytes, **without mounting or a kernel FS driver**, and absorbs it into
the content-addressed graph, producing objects **byte-identical** to those that
`agora-cli wawa importar` emits from a mounted directory.

It's the piece that fulfills the "absorb the user's old data / swallow a USB"
wish of wawa's self-hosting vision (`WAWA.md`, vision memory).
It follows rule #4 of `CLAUDE.md`: foreign formats enter through
`shared/foreign-*` bridges, never into the apps' core.

## Architecture

```
              ┌─────────────┐   absorber()   ┌──────────┐
  dispositivo │  LectorFs   │ ─────────────► │  Emisor  │ ─► objetos del grafo
   (&[u8])    │ (FAT / ext) │  (chunking,    └──────────┘   (<hash>.obj / sys_object_put)
              └─────────────┘   objeto_arbol)
                    ▲
              particion.rs: GPT/MBR → sub-slice por partición → autodetecta FS
```

- **`LectorFs`** (`lib.rs`) — trait for a read-only FS traversable node by
  node (`raiz`, `listar`, `leer_archivo`, `destino_symlink`). The `Manija` is
  opaque to the absorber.
- **`Emisor`** (`lib.rs`) — sink for graph objects. Mirrors
  `agora-cli::emitir_objeto`: serializes, hashes, persists, returns the hash.
  `EmisorMemoria` accumulates in memory with dedup by hash.
- **`absorber()`** (`lib.rs`) — traverses any `LectorFs` bottom-up
  and emits the graph. It reproduces the host's construction **bit by bit**:
  `TAMANO_TROZO` chunking (256 KiB), name-sorted `objeto_arbol`,
  flat blob vs index. Same content → same root hash, wherever it comes from.
- **`fat::LectorFat`** — FAT12/16/32: BPB, FAT chains by type, fixed root
  (12/16) vs root in a chain (32), 8.3 entries + VFAT long names (LFN), lowercase
  flags, empty file. FAT has no exec or symlinks → everything is `Archivo`.
- **`ext4::LectorExt4`** — ext2/3/4: superblock, 32/64 B group descriptors
  (64BIT feature), inodes, files via extent tree (ext4) and direct/single/
  double/triple indirect blocks (ext2/3), linear directories (skips
  the `metadata_csum`/htree padding via `inode==0`), fast+slow symlinks,
  and the **execution bit** from `i_mode`.
- **`particion`** — GPT table (`EFI PART`) and MBR (offset 446); a bare FS with
  no table = one partition. `detectar_fs` sniffs ext (`0xEF53`) vs FAT (BPB).
  `absorber_dispositivo` builds a top tree `particionN/` for each recognized FS.

`#![no_std] + alloc`: the core travels to the bare-metal kernel and will eventually run
as an in-cage WASM app. Validated on `wasm32-unknown-unknown` by
`scripts/check-shared-cores.sh`.

## Usage (host)

```bash
# Absorber una imagen de dispositivo (disco entero, partición o imagen FS) a un
# bundle <hash>.obj + raiz.txt — servible a wawa por servir_release:
agora-cli wawa importar-imagen --imagen disco.img --salida bundle/
agora-cli wawa importar-imagen --imagen disco.img --salida bundle/ --particion 2

# Reconstruir el árbol de vuelta al filesystem (inverso, verifica hashes):
agora-cli wawa exportar --bundle bundle/ --destino salida/
```

For a mounted ext4 partition you can use the directory path
(`agora-cli wawa importar --dir /mnt/... --salida bundle/`); `importar-imagen`
is the path for raw bytes (no mounting).

## Verified guarantees (host)

12 tests. The central invariant: **the root hash of the absorption == that of
importing the same tree from disk** (if parsing lost/corrupted something, the
hash would diverge — self-validating).

| Suite | Covers |
|---|---|
| `lib` (unit) | determinism, content dedup, chunking at the exact boundary (synthetic FS, no tools) |
| `roundtrip_fat` | FAT12 / FAT16 / FAT32, LFN, 8.3, empty, chunking (`mkfs.fat`+`mcopy`) |
| `roundtrip_ext4` | ext4 (extents) and ext2 (double indirect), exec + symlink + empty + chunking (`mke2fs -d`) |
| `roundtrip_particion` | GPT disk (FAT+ext4) + MBR (ext4) + bare FS (`sfdisk` injecting images) |
| `stress_ext4` | large tree: 23 block groups, multi-block dir, multi-MiB file, unicode, hard links |

Tests with external tools (`mkfs.fat`/`mcopy`/`mke2fs`/`sfdisk`) skip
cleanly if missing; the unit tests depend on nothing.

## Known limitations

- **Read-only.** Doesn't write, doesn't repair, doesn't verify checksums (`metadata_csum`
  is ignored — reading doesn't need it).
- **512 B logical sectors** (GPT/MBR convention). Native 4Kn disks: outside
  the MVP.
- **htree-index not exercised per se.** `mke2fs -d` builds large LINEAR
  directories (which are covered, multi-block); the htree index is built by the
  kernel when inserting into a mounted FS (out of scope without loopback). The reader
  parses both alike (skips the index via `inode==0`), but that path has
  no direct coverage.
- **NTFS / btrfs absent.** The vision is Linux→wawa, so ext4 is the
  priority; NTFS (Windows data) would be another future `LectorFs`.
- **Directories are read whole into RAM** (FILE content is not — see
  below). A gigantic directory (hundreds of thousands of entries) materializes
  its content; real cases (thousands of entries, tens of KiB) fit
  comfortably under the 4 MiB ceiling.

## In-cage ready (library side) + what's missing (QEMU, gated)

The reader+absorber core is ready and tested host-side and **already has the shape
to run in-cage**:

1. ✅ **Block source** — the medium lives behind the trait `Fuente { leer_en,
   tamano }` (not a fixed `&[u8]`). The host satisfies it with `&[u8]` (blanket impl);
   in-cage, with a syscall. Verified: absorbing through an arbitrary `Fuente` gives
   the same graph as through `&[u8]` (`tests/fuente.rs`).
2. ✅ **Absorber with bounded memory** — file content is read in
   `TAMANO_TROZO` windows (256 KiB) via `leer_archivo_en` and emitted chunk by
   chunk; the logical→physical resolution (`bloque_logico` in ext4, chain
   traversal in FAT) navigates on demand with O(1) RAM per block. Verified: absorbing
   a 2.5 MiB file never asks the `Fuente` for more than one block (≤4 KiB) at
   a time (`tests/fuente.rs`).

Missing the bare-metal/QEMU piece (the author runs the image):

3. ⬜ **Syscall + app** — `sys_dispositivo_leer(lba, buf)` (gated by a new
   permission) that exposes a second read-only virtio-blk, and a WASM app
   `absorbedor` that implements `Fuente` over that syscall and `Emisor` over
   `sys_object_put`, then `sys_object_fijar_raiz`. With pieces 1 and 2 done,
   this app is a thin wrapper.

Related: the block driver for real hardware (AHCI/NVMe) and the USB installer
are phase 4 of the vision —the most expensive leap—, and the point where
in-cage absorption becomes useful over a physical USB.

## Status (2026-05-31)

### Done
- `no_std` readers for FAT12/16/32 and ext2/3/4 over raw bytes (no mounting),
  behind the `Fuente` trait; GPT/MBR partition table + FS autodetection.
- Absorber with bounded memory (256 KiB windows) byte-identical to
  `agora-cli wawa importar`; validated on `wasm32-unknown-unknown`.
- 12 tests, including ext4 stress (multi-group, multi-block dir, multi-MiB).

### Pending
- Read-only: doesn't write, doesn't repair, doesn't verify checksums.
- NTFS / btrfs absent (the priority is Linux→wawa).
- Bare-metal piece: `sys_dispositivo_leer` + in-cage WASM app `absorbedor` (⬜).
- 4Kn disks and htree-index with no direct coverage.
