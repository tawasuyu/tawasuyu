// =============================================================================
//  foreign-fs :: prueba de absorción FAT == importación host
// -----------------------------------------------------------------------------
//  La prueba decisiva del puente: un mismo árbol de directorios, absorbido por
//  el lector FAT (sobre la imagen cruda) vs. importado leyéndolo del disco con
//  la MISMA lógica de grafo que `agora-cli wawa importar`, debe colapsar al
//  MISMO hash raíz y al MISMO número de objetos. Si coinciden, el lector FAT
//  recupera nombres, contenido, estructura y troceado idénticos — la absorción
//  desde dentro de wawa produciría un grafo intercambiable con el del host.
//
//  Requiere `mkfs.fat` + `mcopy` (paquete dosfstools + mtools). Si faltan, la
//  prueba se salta limpiamente en vez de fallar — no son dependencias del
//  crate, sólo del banco de pruebas.
//
//  Ejercita FAT12 / FAT16 / FAT32, raíz fija vs raíz en cadena, nombres largos
//  VFAT (LFN) y 8.3, archivo vacío, archivo en el límite exacto de trozo y
//  archivo troceado (> 256 KiB).
// =============================================================================

use std::fs;
use std::path::{Path, PathBuf};
use std::process::Command;

use foreign_fs::{absorber, fat::LectorFat, EmisorMemoria, TAMANO_TROZO};

/// ¿Están las herramientas FAT disponibles?
fn hay_herramientas() -> bool {
    which("mkfs.fat") && which("mcopy")
}

fn which(bin: &str) -> bool {
    Command::new("sh")
        .arg("-c")
        .arg(format!("command -v {bin}"))
        .output()
        .map(|o| o.status.success())
        .unwrap_or(false)
}

/// Contenido determinista de `n` bytes (patrón estable, no aleatorio).
fn patron(n: usize, semilla: u8) -> Vec<u8> {
    (0..n).map(|i| ((i + semilla as usize) % 251) as u8).collect()
}

/// Construye un árbol de prueba bajo `raiz`. SIN bits de ejecución ni symlinks
/// (FAT no los representa), para que el oráculo de disco coincida con FAT.
fn sembrar_arbol(raiz: &Path) {
    fs::create_dir_all(raiz).unwrap();
    // Nombres largos en minúscula → fuerzan LFN.
    fs::write(raiz.join("hola_mundo.txt"), b"hola, grafo\n").unwrap();
    fs::write(raiz.join("exacto.bin"), patron(TAMANO_TROZO, 7)).unwrap(); // límite: 1 blob
    fs::write(raiz.join("datos_grandes.bin"), patron(600_000, 3)).unwrap(); // troceado
    fs::write(raiz.join("vacio.dat"), b"").unwrap(); // archivo vacío (cluster 0)
    // Nombre 8.3 válido en mayúsculas → camino de nombre corto (sin LFN).
    fs::write(raiz.join("READ.ME"), b"corto\n").unwrap();

    let sub = raiz.join("subcarpeta");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("anidado.txt"), b"profundo\n").unwrap();
    fs::write(sub.join("otro_grande.bin"), patron(300_000, 11)).unwrap();
}

/// Variante con sólo archivos pequeños (para FAT12, que vive en imágenes
/// chicas donde un archivo > 256 KiB no entra cómodo).
fn sembrar_arbol_chico(raiz: &Path) {
    fs::create_dir_all(raiz).unwrap();
    fs::write(raiz.join("hola_mundo.txt"), b"hola, grafo\n").unwrap();
    fs::write(raiz.join("vacio.dat"), b"").unwrap();
    fs::write(raiz.join("READ.ME"), b"corto\n").unwrap();
    let sub = raiz.join("subcarpeta");
    fs::create_dir_all(&sub).unwrap();
    fs::write(sub.join("anidado.txt"), patron(5000, 1)).unwrap();
}

// --- Oráculo: la lógica de grafo del host (espejo de agora-cli importar) -----

fn oraculo_dir(dir: &Path, emisor: &mut EmisorMemoria) -> format::Hash {
    use foreign_fs::Emisor;
    let mut entradas: Vec<format::EntradaArbol> = Vec::new();
    for ent in fs::read_dir(dir).unwrap() {
        let ent = ent.unwrap();
        let ruta = ent.path();
        let nombre = ent.file_name().to_string_lossy().into_owned();
        let ft = ent.file_type().unwrap();
        let (modo, hash) = if ft.is_dir() {
            (format::ModoEntrada::Directorio, oraculo_dir(&ruta, emisor))
        } else {
            let bytes = fs::read(&ruta).unwrap();
            (format::ModoEntrada::Archivo, oraculo_archivo(bytes, emisor))
        };
        entradas.push(format::EntradaArbol { nombre, modo, hash });
    }
    let objeto = format::objeto_arbol(entradas).unwrap();
    emisor.emitir(&objeto).unwrap()
}

fn oraculo_archivo(bytes: Vec<u8>, emisor: &mut EmisorMemoria) -> format::Hash {
    use foreign_fs::Emisor;
    if bytes.len() <= TAMANO_TROZO {
        return emisor.emitir(&format::objeto_blob(bytes)).unwrap();
    }
    let mut trozos = Vec::new();
    for trozo in bytes.chunks(TAMANO_TROZO) {
        trozos.push(emisor.emitir(&format::objeto_blob(trozo.to_vec())).unwrap());
    }
    emisor.emitir(&format::objeto_blob_indice(trozos)).unwrap()
}

// --- Forja de la imagen FAT --------------------------------------------------

/// Crea una imagen FAT de `mib` MiB del sabor `-F flavor` y copia los hijos
/// directos de `src` a la raíz del volumen. Devuelve los bytes de la imagen.
fn forjar_imagen_fat(src: &Path, mib: u64, flavor: u8, tmp: &Path) -> Vec<u8> {
    let img = tmp.join(format!("fat{flavor}.img"));
    let f = fs::File::create(&img).unwrap();
    f.set_len(mib * 1024 * 1024).unwrap();
    drop(f);

    let salida = Command::new("mkfs.fat")
        .arg("-F")
        .arg(flavor.to_string())
        .arg(&img)
        .output()
        .unwrap();
    assert!(
        salida.status.success(),
        "mkfs.fat -F {flavor} falló: {}",
        String::from_utf8_lossy(&salida.stderr)
    );

    // mcopy de cada hijo directo del árbol a la raíz del volumen (recursivo).
    for ent in fs::read_dir(src).unwrap() {
        let ruta = ent.unwrap().path();
        let salida = Command::new("mcopy")
            .env("MTOOLS_SKIP_CHECK", "1")
            .arg("-s")
            .arg("-i")
            .arg(&img)
            .arg(&ruta)
            .arg("::")
            .output()
            .unwrap();
        assert!(
            salida.status.success(),
            "mcopy {} falló: {}",
            ruta.display(),
            String::from_utf8_lossy(&salida.stderr)
        );
    }

    fs::read(&img).unwrap()
}

/// Comprueba que absorber la imagen FAT == importar el árbol del disco.
fn comparar(src: &Path, mib: u64, flavor: u8, tmp: &Path) {
    let bytes = forjar_imagen_fat(src, mib, flavor, tmp);

    let lector = LectorFat::nuevo(&bytes)
        .unwrap_or_else(|e| panic!("FAT{flavor}: BPB no parseó: {e:?}"));
    let mut emisor_fat = EmisorMemoria::nuevo();
    let raiz_fat = absorber(&lector, &mut emisor_fat)
        .unwrap_or_else(|e| panic!("FAT{flavor}: absorción falló: {e:?}"));

    let mut emisor_oraculo = EmisorMemoria::nuevo();
    let raiz_oraculo = oraculo_dir(src, &mut emisor_oraculo);

    assert_eq!(
        raiz_fat, raiz_oraculo,
        "FAT{flavor}: el hash raíz de la absorción FAT difiere del importado del disco"
    );
    assert_eq!(
        emisor_fat.len(),
        emisor_oraculo.len(),
        "FAT{flavor}: distinto número de objetos únicos en el grafo"
    );
}

#[test]
fn fat32_grande_coincide_con_importar() {
    if !hay_herramientas() {
        eprintln!("SKIP: faltan mkfs.fat/mcopy");
        return;
    }
    let tmp = tmpdir("ff-fat32");
    let src = tmp.join("src");
    sembrar_arbol(&src);
    comparar(&src, 64, 32, &tmp);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn fat16_grande_coincide_con_importar() {
    if !hay_herramientas() {
        eprintln!("SKIP: faltan mkfs.fat/mcopy");
        return;
    }
    let tmp = tmpdir("ff-fat16");
    let src = tmp.join("src");
    sembrar_arbol(&src);
    comparar(&src, 48, 16, &tmp);
    let _ = fs::remove_dir_all(&tmp);
}

#[test]
fn fat12_chico_coincide_con_importar() {
    if !hay_herramientas() {
        eprintln!("SKIP: faltan mkfs.fat/mcopy");
        return;
    }
    let tmp = tmpdir("ff-fat12");
    let src = tmp.join("src");
    sembrar_arbol_chico(&src);
    comparar(&src, 2, 12, &tmp);
    let _ = fs::remove_dir_all(&tmp);
}

/// Directorio temporal único bajo el target dir del crate (evita ensuciar /tmp
/// con permisos raros y respeta `CARGO_TARGET_TMPDIR` si está).
fn tmpdir(prefijo: &str) -> PathBuf {
    let base = std::env::var("CARGO_TARGET_TMPDIR")
        .map(PathBuf::from)
        .unwrap_or_else(|_| std::env::temp_dir());
    let dir = base.join(format!("{prefijo}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).unwrap();
    dir
}
