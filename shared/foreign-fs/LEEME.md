# foreign-fs

Puente de **sistemas de archivos ajenos** al grafo nativo de wawa (BLAKE3 + DAG
+ postcard). Lee un FS extranjero —hoy FAT12/16/32 y ext2/3/4— directamente
sobre bytes crudos, **sin montar ni driver de FS del kernel**, y lo absorbe al
grafo direccionado por contenido produciendo objetos **byte-idénticos** a los
que emite `agora-cli wawa importar` desde un directorio montado.

Es la pieza que cumple el deseo "absorber los datos viejos del usuario / tragar
un USB" de la visión de self-hosting de wawa (`WAWA.md`, memoria de visión).
Sigue la regla #4 de `CLAUDE.md`: los formatos ajenos entran por puentes
`shared/foreign-*`, nunca al núcleo de las apps.

## Arquitectura

```
              ┌─────────────┐   absorber()   ┌──────────┐
  dispositivo │  LectorFs   │ ─────────────► │  Emisor  │ ─► objetos del grafo
   (&[u8])    │ (FAT / ext) │  (chunking,    └──────────┘   (<hash>.obj / sys_object_put)
              └─────────────┘   objeto_arbol)
                    ▲
              particion.rs: GPT/MBR → sub-slice por partición → autodetecta FS
```

- **`LectorFs`** (`lib.rs`) — trait de un FS de sólo-lectura recorrible nodo a
  nodo (`raiz`, `listar`, `leer_archivo`, `destino_symlink`). La `Manija` es
  opaca al absorbedor.
- **`Emisor`** (`lib.rs`) — sumidero de objetos del grafo. Espeja
  `agora-cli::emitir_objeto`: serializa, hashea, persiste, devuelve el hash.
  `EmisorMemoria` acumula en memoria con dedup por hash.
- **`absorber()`** (`lib.rs`) — recorre cualquier `LectorFs` de abajo hacia
  arriba y emite el grafo. Reproduce **bit a bit** la construcción del host:
  troceado de `TAMANO_TROZO` (256 KiB), `objeto_arbol` ordenado por nombre,
  blob plano vs índice. Mismo contenido → mismo hash raíz, venga de donde venga.
- **`fat::LectorFat`** — FAT12/16/32: BPB, cadenas FAT por tipo, raíz fija
  (12/16) vs raíz en cadena (32), entradas 8.3 + nombres largos VFAT (LFN), flags
  de minúsculas, archivo vacío. FAT no tiene exec ni symlinks → todo `Archivo`.
- **`ext4::LectorExt4`** — ext2/3/4: superbloque, descriptores de grupo 32/64 B
  (feature 64BIT), inodos, archivos por árbol de extents (ext4) y bloques
  indirectos directo/simple/doble/triple (ext2/3), directorios lineales (salta
  el relleno de `metadata_csum`/htree vía `inode==0`), symlinks rápidos+lentos,
  y **bit de ejecución** desde `i_mode`.
- **`particion`** — tabla GPT (`EFI PART`) y MBR (offset 446); FS suelto sin
  tabla = una partición. `detectar_fs` olfatea ext (`0xEF53`) vs FAT (BPB).
  `absorber_dispositivo` arma un árbol top `particionN/` por cada FS reconocido.

`#![no_std] + alloc`: el núcleo viaja al kernel bare-metal y eventualmente correrá
como app WASM in-cage. Validado en `wasm32-unknown-unknown` por
`scripts/check-shared-cores.sh`.

## Uso (host)

```bash
# Absorber una imagen de dispositivo (disco entero, partición o imagen FS) a un
# bundle <hash>.obj + raiz.txt — servible a wawa por servir_release:
agora-cli wawa importar-imagen --imagen disco.img --salida bundle/
agora-cli wawa importar-imagen --imagen disco.img --salida bundle/ --particion 2

# Reconstruir el árbol de vuelta al filesystem (inverso, verifica hashes):
agora-cli wawa exportar --bundle bundle/ --destino salida/
```

Para una partición ext4 montada se puede usar el camino de directorio
(`agora-cli wawa importar --dir /mnt/... --salida bundle/`); `importar-imagen`
es el camino para bytes crudos (sin montar).

## Garantías verificadas (host)

12 tests. El invariante central: **el hash raíz de la absorción == el de
importar el mismo árbol del disco** (si el parseo perdiera/corrompiera algo, el
hash divergiría — autovalidante).

| Suite | Cubre |
|---|---|
| `lib` (unit) | determinismo, dedup por contenido, troceado en el límite exacto (FS sintético, sin herramientas) |
| `roundtrip_fat` | FAT12 / FAT16 / FAT32, LFN, 8.3, vacío, troceado (`mkfs.fat`+`mcopy`) |
| `roundtrip_ext4` | ext4 (extents) y ext2 (indirecto doble), exec + symlink + vacío + troceado (`mke2fs -d`) |
| `roundtrip_particion` | disco GPT (FAT+ext4) + MBR (ext4) + FS suelto (`sfdisk` inyectando imágenes) |
| `stress_ext4` | árbol grande: 23 block groups, dir multi-bloque, archivo multi-MiB, unicode, hard links |

Tests con herramientas externas (`mkfs.fat`/`mcopy`/`mke2fs`/`sfdisk`) se saltan
limpiamente si faltan; los unit no dependen de nada.

## Limitaciones conocidas

- **Sólo-lectura.** No escribe, no repara, no verifica checksums (`metadata_csum`
  se ignora — leer no lo necesita).
- **Sectores lógicos de 512 B** (convención GPT/MBR). Discos 4Kn nativos: fuera
  del MVP.
- **htree-index no ejercitado en sí.** `mke2fs -d` arma directorios LINEALES
  grandes (que sí se cubren, multi-bloque); el índice htree lo construye el
  kernel al insertar en un FS montado (fuera de alcance sin loopback). El lector
  parsea ambos por igual (salta el índice vía `inode==0`), pero ese camino no
  tiene cobertura directa.
- **NTFS / btrfs ausentes.** La visión es Linux→wawa, así que ext4 es la
  prioridad; NTFS (datos de Windows) sería otro `LectorFs` futuro.
- **Directorios se leen enteros en RAM** (el contenido de ARCHIVO no — ver
  abajo). Un directorio gigantesco (cientos de miles de entradas) materializa
  su contenido; los casos reales (miles de entradas, decenas de KiB) caben de
  sobra bajo el techo de 4 MiB.

## Listo para in-cage (lado librería) + lo que falta (QEMU, gated)

El núcleo lector+absorbedor está listo y probado host-side y **ya tiene la forma
para correr in-cage**:

1. ✅ **Fuente de bloques** — el medio vive detrás del trait `Fuente { leer_en,
   tamano }` (no `&[u8]` fijo). El host lo satisface con `&[u8]` (blanket impl);
   in-cage, con un syscall. Verificado: absorber por una `Fuente` arbitraria da
   el mismo grafo que por `&[u8]` (`tests/fuente.rs`).
2. ✅ **Absorbedor con memoria acotada** — el contenido de archivo se lee en
   ventanas de `TAMANO_TROZO` (256 KiB) vía `leer_archivo_en` y se emite trozo a
   trozo; la resolución lógico→físico (`bloque_logico` en ext4, recorrido de
   cadena en FAT) navega a demanda con RAM O(1) por bloque. Verificado: absorber
   un archivo de 2.5 MiB nunca pide a la `Fuente` más de un bloque (≤4 KiB) de
   una vez (`tests/fuente.rs`).

Falta la pieza bare-metal/QEMU (el autor corre la imagen):

3. ⬜ **Syscall + app** — `sys_dispositivo_leer(lba, buf)` (gateado por un permiso
   nuevo) que exponga un segundo virtio-blk de sólo-lectura, y una app WASM
   `absorbedor` que implemente `Fuente` sobre ese syscall y `Emisor` sobre
   `sys_object_put`, luego `sys_object_fijar_raiz`. Con las piezas 1 y 2 hechas,
   esta app es un envoltorio delgado.

Relacionado: el driver de bloque para hierro real (AHCI/NVMe) y el instalador
USB son la fase 4 de la visión —el salto más caro—, y el punto donde la
absorción in-cage se vuelve útil sobre un USB físico.

## Estado (2026-05-31)

### Hecho
- Lectores `no_std` de FAT12/16/32 y ext2/3/4 sobre bytes crudos (sin montar),
  tras el trait `Fuente`; tabla de particiones GPT/MBR + autodetección de FS.
- Absorbedor con memoria acotada (ventanas de 256 KiB) byte-idéntico a
  `agora-cli wawa importar`; validado en `wasm32-unknown-unknown`.
- 12 tests, incluido stress ext4 (multi-grupo, dir multi-bloque, multi-MiB).

### Pendiente
- Sólo-lectura: no escribe, no repara, no verifica checksums.
- NTFS / btrfs ausentes (la prioridad es Linux→wawa).
- Pieza bare-metal: `sys_dispositivo_leer` + app WASM `absorbedor` in-cage (⬜).
- Discos 4Kn y htree-index sin cobertura directa.
