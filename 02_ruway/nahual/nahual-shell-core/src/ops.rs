//! `ops` — operaciones de archivo del shell nahual y su **cola** (Fase 4.3).
//!
//! Una operación (crear carpeta/archivo, renombrar, borrar, copiar, mover) es
//! un *job* que corre en un hilo aparte (`Handle::spawn`): el disco puede
//! tardar (copiar un árbol grande, borrar recursivo), y bloquear el bucle Elm
//! congelaría la UI. Cada job arranca en `Running`, y al terminar dispatcha un
//! [`OpKind`]-`OpFinished` que actualiza su estado y recarga el panel.
//!
//! El worker reconstruye una [`PosixSource`] anclada en `/` y ejecuta la
//! operación por su cara [`nahual_source_core::SourceMut`]. Es POSIX-only a
//! propósito: POSIX es hoy la única fuente con `SourceMut` (wawa/minga/nouser
//! son content-addressed / derivadas, read-only). Como los `NodeId` de POSIX
//! son rutas absolutas, el worker no necesita el `Navigator` del panel — sólo
//! los ids — así que no hace falta compartir el `Box<dyn Source>` entre hilos.

use std::path::{Path, PathBuf};

use nahual_source_core::{DispositivosSource, NodeId, PosixSource, Source};

/// Qué hace una operación de archivo. Los `NodeId` son rutas absolutas POSIX.
#[derive(Clone, Debug)]
pub enum OpKind {
    /// Crear un directorio `name` dentro de `parent`.
    NewDir { parent: NodeId, name: String },
    /// Crear un archivo vacío `name` dentro de `parent`.
    NewFile { parent: NodeId, name: String },
    /// Renombrar `id` a `new_name` (en su mismo contenedor).
    Rename { id: NodeId, new_name: String },
    /// Borrar `id` (recursivo si es contenedor).
    Delete { id: NodeId, name: String },
    /// Copiar `id` dentro de `dest_parent` (recursivo).
    Copy { id: NodeId, name: String, dest_parent: NodeId },
    /// Mover `id` dentro de `dest_parent`.
    Move { id: NodeId, name: String, dest_parent: NodeId },
    /// **Extraer** `src_id` (de un dispositivo de bloques read-only) a un
    /// directorio POSIX `dest_parent`, recursivo. Es la copia CROSS-SOURCE:
    /// lee por la `Source` del device (bytes, sin montar) y escribe en disco.
    /// `dest_parent` es una ruta POSIX absoluta; `name` el nombre destino.
    Extraer { src_id: NodeId, name: String, es_dir: bool, dest_parent: NodeId },
    /// **Absorber** un dispositivo/partición (`src_id`) al grafo wawa: produce un
    /// bundle `<name>-wawa/` (objetos `<hash>.obj` + `raiz.txt`) dentro de
    /// `dest_parent` (POSIX). Lee el device por bytes, sin montar.
    AbsorberDispositivo { src_id: NodeId, name: String, dest_parent: NodeId },
}

impl OpKind {
    /// Etiqueta humana para la fila de la cola (verbo + nombre).
    pub fn label(&self) -> String {
        match self {
            OpKind::NewDir { name, .. } => format!("Nueva carpeta · {name}"),
            OpKind::NewFile { name, .. } => format!("Nuevo archivo · {name}"),
            OpKind::Rename { new_name, .. } => format!("Renombrar → {new_name}"),
            OpKind::Delete { name, .. } => format!("Borrar · {name}"),
            OpKind::Copy { name, .. } => format!("Copiar · {name}"),
            OpKind::Move { name, .. } => format!("Mover · {name}"),
            OpKind::Extraer { name, .. } => format!("Extraer · {name}"),
            OpKind::AbsorberDispositivo { name, .. } => format!("Absorber · {name}"),
        }
    }

    /// Ejecuta la operación sobre el filesystem real. Devuelve el `NodeId`
    /// resultante (la ruta del nuevo nodo, o `None` para borrar). Bloqueante —
    /// se llama desde el worker, nunca en el hilo de UI.
    pub fn run(&self) -> std::io::Result<Option<NodeId>> {
        let src = PosixSource::new("/");
        let mutable = src.writable().ok_or_else(|| {
            std::io::Error::new(std::io::ErrorKind::Unsupported, "fuente sin escritura")
        })?;
        match self {
            OpKind::NewDir { parent, name } => mutable.create_dir(parent, name).map(Some),
            OpKind::NewFile { parent, name } => mutable.create_file(parent, name).map(Some),
            OpKind::Rename { id, new_name } => mutable.rename(id, new_name).map(Some),
            OpKind::Delete { id, .. } => mutable.delete(id).map(|()| None),
            OpKind::Copy { id, dest_parent, .. } => mutable.copy_into(id, dest_parent).map(Some),
            OpKind::Move { id, dest_parent, .. } => mutable.move_into(id, dest_parent).map(Some),
            OpKind::Extraer { src_id, name, es_dir, dest_parent } => {
                // El device se reconstruye desde el id (no se comparte la fuente
                // viva entre hilos) y se vuelca a POSIX recursivamente.
                let origen = DispositivosSource::reconstruir_para(src_id)?;
                let destino = Path::new(dest_parent).join(sanear(name));
                extraer_nodo(&origen, src_id, *es_dir, &destino)?;
                Ok(Some(destino.to_string_lossy().into_owned()))
            }
            OpKind::AbsorberDispositivo { src_id, name, dest_parent } => {
                absorber_a_bundle(src_id, name, dest_parent).map(Some)
            }
        }
    }
}

/// Absorbe el device/partición `src_id` a un bundle wawa `<name>-wawa/` dentro
/// de `dest_parent`. Reusa el motor PEREZOSO de `foreign-fs` (lee por bytes, sin
/// montar) y el sink `EmisorBundle` (`<hash>.obj`). Devuelve la ruta del bundle.
fn absorber_a_bundle(
    src_id: &NodeId,
    name: &str,
    dest_parent: &NodeId,
) -> std::io::Result<NodeId> {
    use foreign_fs::particion::{
        absorber_dispositivo_fuente, absorber_particion_fuente, tabla_particiones_fuente,
    };
    use foreign_fs::{EmisorBundle, FuenteArchivo};
    use nahual_source_core::ObjetivoAbsorcion;

    let objetivo = nahual_source_core::objetivo_absorcion(src_id).ok_or_else(|| {
        std::io::Error::new(
            std::io::ErrorKind::InvalidInput,
            "sólo un dispositivo o una partición se absorben al grafo",
        )
    })?;

    let destino = Path::new(dest_parent).join(format!("{}-wawa", sanear(name).to_string_lossy()));
    std::fs::create_dir_all(&destino)?;
    let mut emisor = EmisorBundle::nuevo(&destino);

    let raiz = match &objetivo {
        ObjetivoAbsorcion::Dispositivo(ruta) => {
            let fa = FuenteArchivo::abrir(ruta)?;
            absorber_dispositivo_fuente(&fa, &mut emisor)
        }
        ObjetivoAbsorcion::Particion(ruta, idx) => {
            let fa = FuenteArchivo::abrir(ruta)?;
            let parts = tabla_particiones_fuente(&fa).map_err(fserr)?;
            let p = parts.into_iter().find(|p| p.indice == *idx).ok_or_else(|| {
                std::io::Error::new(std::io::ErrorKind::NotFound, format!("partición {idx} inexistente"))
            })?;
            absorber_particion_fuente(&fa, &p, &mut emisor)
        }
    };

    // Propaga el error de I/O real del emisor si lo hubo.
    let raiz = match raiz {
        Ok(h) => h,
        Err(foreign_fs::FsError::EmisionFallida) => {
            return Err(emisor
                .tomar_error_io()
                .unwrap_or_else(|| std::io::Error::other("emisión del bundle fallida")))
        }
        Err(e) => return Err(fserr(e)),
    };

    std::fs::write(destino.join("raiz.txt"), format!("{}\n", foreign_fs::hex32(&raiz)))?;
    Ok(destino.to_string_lossy().into_owned())
}

/// Mapea un `FsError` de foreign-fs a un `io::Error` con detalle.
fn fserr(e: foreign_fs::FsError) -> std::io::Error {
    std::io::Error::new(std::io::ErrorKind::InvalidData, format!("foreign-fs: {e:?}"))
}

/// Vuelca el nodo `id` de `origen` a la ruta POSIX `destino`, recursivo. Un
/// archivo se STREAMEA por ventanas (sin materializarlo entero en RAM — clave
/// para un video/ISO grande); un directorio se crea y se baja a sus hijos. Los
/// nombres se sanean (un FS ajeno podría traer `..` o `/`).
fn extraer_nodo(
    origen: &DispositivosSource,
    id: &NodeId,
    es_dir: bool,
    destino: &Path,
) -> std::io::Result<()> {
    if es_dir {
        std::fs::create_dir_all(destino)?;
        for hijo in origen.children(id)? {
            let sub = destino.join(sanear(&hijo.name));
            extraer_nodo(origen, &hijo.id, hijo.is_container, &sub)?;
        }
    } else {
        if let Some(p) = destino.parent() {
            std::fs::create_dir_all(p)?;
        }
        let mut f = std::fs::File::create(destino)?;
        origen.leer_a_escritor(id, &mut f)?;
    }
    Ok(())
}

/// Sanea un nombre de archivo ajeno para que no escape del destino: sin barras,
/// y `.`/`..` neutralizados. Un nombre vacío cae a `_`.
fn sanear(nombre: &str) -> PathBuf {
    let limpio = nombre.replace(['/', '\\'], "_");
    let limpio = match limpio.as_str() {
        "" | "." | ".." => "_".to_string(),
        _ => limpio,
    };
    PathBuf::from(limpio)
}

/// Estado de un job en la cola.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum OpStatus {
    /// En vuelo (corriendo en el worker).
    Running,
    /// Terminó OK. Lleva el id resultante (para reubicar el cursor).
    Done(Option<NodeId>),
    /// Falló — guarda el mensaje de error para mostrarlo en la fila.
    Failed(String),
}

/// Un job de la cola: su id incremental, qué hace y en qué estado está.
#[derive(Clone, Debug)]
pub struct Op {
    pub id: u64,
    pub kind: OpKind,
    pub label: String,
    pub status: OpStatus,
}

/// La cola de operaciones. Append-only (los jobs viejos quedan como historial
/// hasta que se limpian); `open` controla si el panel inferior se ve.
#[derive(Default)]
pub struct OpQueue {
    pub ops: Vec<Op>,
    next_id: u64,
    /// `true` = el panel inferior colapsable está desplegado.
    pub open: bool,
}

impl OpQueue {
    /// Encola un job nuevo en estado `Running` y devuelve su id (para casar el
    /// `OpFinished` del worker). Despliega el panel.
    pub fn push(&mut self, kind: OpKind) -> u64 {
        let id = self.next_id;
        self.next_id += 1;
        self.ops.push(Op {
            id,
            label: kind.label(),
            kind,
            status: OpStatus::Running,
        });
        self.open = true;
        id
    }

    /// Marca el job `id` como terminado (OK o error).
    pub fn finish(&mut self, id: u64, status: OpStatus) {
        if let Some(op) = self.ops.iter_mut().find(|o| o.id == id) {
            op.status = status;
        }
    }

    /// `true` si hay algún job todavía corriendo.
    pub fn any_running(&self) -> bool {
        self.ops.iter().any(|o| o.status == OpStatus::Running)
    }

    /// Cuántos jobs corriendo / total — para el rótulo del panel.
    pub fn running_count(&self) -> usize {
        self.ops.iter().filter(|o| o.status == OpStatus::Running).count()
    }

    /// Olvida los jobs ya terminados (deja sólo los `Running`).
    pub fn clear_finished(&mut self) {
        self.ops.retain(|o| o.status == OpStatus::Running);
        if self.ops.is_empty() {
            self.open = false;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs;

    fn arbol() -> tempfile::TempDir {
        let dir = tempfile::tempdir().unwrap();
        fs::write(dir.path().join("hola.txt"), b"hola").unwrap();
        dir
    }

    #[test]
    fn new_dir_y_rename_y_delete() {
        let dir = arbol();
        let root = dir.path().to_string_lossy().into_owned();

        // Crear carpeta.
        let nd = OpKind::NewDir { parent: root.clone(), name: "sub".into() };
        let id = nd.run().unwrap().unwrap();
        assert!(dir.path().join("sub").is_dir());

        // Renombrar carpeta.
        let rn = OpKind::Rename { id: id.clone(), new_name: "sub2".into() };
        let id2 = rn.run().unwrap().unwrap();
        assert!(dir.path().join("sub2").is_dir());
        assert!(!dir.path().join("sub").exists());

        // Borrar carpeta.
        let del = OpKind::Delete { id: id2, name: "sub2".into() };
        assert!(del.run().unwrap().is_none());
        assert!(!dir.path().join("sub2").exists());
    }

    #[test]
    fn copy_y_move_entre_dirs() {
        let dir = arbol();
        let root = dir.path().to_string_lossy().into_owned();
        fs::create_dir(dir.path().join("dst")).unwrap();
        let dst = dir.path().join("dst").to_string_lossy().into_owned();
        let hola = dir.path().join("hola.txt").to_string_lossy().into_owned();

        // Copiar deja el original.
        let cp = OpKind::Copy { id: hola.clone(), name: "hola.txt".into(), dest_parent: dst.clone() };
        cp.run().unwrap().unwrap();
        assert!(dir.path().join("hola.txt").exists());
        assert!(dir.path().join("dst/hola.txt").exists());

        // Mover saca el original.
        let mv = OpKind::Move { id: hola, name: "hola.txt".into(), dest_parent: dst };
        mv.run().unwrap().unwrap();
        assert!(!dir.path().join("hola.txt").exists());
    }

    fn which(bin: &str) -> bool {
        std::process::Command::new("sh")
            .arg("-c")
            .arg(format!("command -v {bin}"))
            .output()
            .map(|o| o.status.success())
            .unwrap_or(false)
    }

    #[test]
    fn extraer_de_un_dispositivo_a_posix() {
        use nahual_source_core::{DispositivoInfo, DispositivosSource};
        use std::process::Command;
        if !which("mkfs.fat") || !which("mcopy") {
            eprintln!("SKIP: faltan mkfs.fat/mcopy");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        // Forjar una imagen FAT con un archivo.
        let img = dir.path().join("disco.img");
        fs::File::create(&img).unwrap().set_len(4 * 1024 * 1024).unwrap();
        assert!(Command::new("mkfs.fat").arg(&img).output().unwrap().status.success());
        let fuente = dir.path().join("hola.txt");
        fs::write(&fuente, b"datos del usb\n").unwrap();
        assert!(Command::new("mcopy")
            .arg("-i").arg(&img).arg(&fuente).arg("::hola.txt")
            .output().unwrap().status.success());

        // Navegar el device para obtener el id del archivo.
        let info = DispositivoInfo {
            ruta: img.clone(),
            nombre: "usb".into(),
            tam: Some(fs::metadata(&img).unwrap().len()),
            removible: true,
            modelo: None,
        };
        let src = DispositivosSource::con_dispositivos(vec![info]);
        let devs = src.children(&src.root().id).unwrap();
        let parts = src.children(&devs[0].id).unwrap();
        let files = src.children(&parts[0].id).unwrap();
        let archivo = files.iter().find(|n| n.name == "hola.txt").expect("hola.txt");

        // Extraer a un dir POSIX y verificar bytes idénticos.
        let destino = dir.path().join("salida");
        fs::create_dir_all(&destino).unwrap();
        let op = OpKind::Extraer {
            src_id: archivo.id.clone(),
            name: "hola.txt".into(),
            es_dir: false,
            dest_parent: destino.to_string_lossy().into_owned(),
        };
        let resultado = op.run().unwrap().unwrap();
        assert_eq!(fs::read(&resultado).unwrap(), b"datos del usb\n");
        assert_eq!(fs::read(destino.join("hola.txt")).unwrap(), b"datos del usb\n");
    }

    #[test]
    fn extraer_archivo_grande_streamea_identico() {
        use nahual_source_core::{DispositivoInfo, DispositivosSource};
        use std::process::Command;
        if !which("mkfs.fat") || !which("mcopy") {
            eprintln!("SKIP: faltan mkfs.fat/mcopy");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("disco.img");
        fs::File::create(&img).unwrap().set_len(8 * 1024 * 1024).unwrap();
        assert!(Command::new("mkfs.fat").arg(&img).output().unwrap().status.success());

        // 700 KiB: cruza la ventana de 256 KiB tres veces — ejercita el lazo de
        // streaming (varias `leer_archivo_en`), no una sola lectura.
        let grande: Vec<u8> = (0..700_000u32).map(|i| (i % 251) as u8).collect();
        let fuente = dir.path().join("grande.bin");
        fs::write(&fuente, &grande).unwrap();
        assert!(Command::new("mcopy")
            .arg("-i").arg(&img).arg(&fuente).arg("::grande.bin")
            .output().unwrap().status.success());

        let info = DispositivoInfo {
            ruta: img.clone(),
            nombre: "usb".into(),
            tam: Some(fs::metadata(&img).unwrap().len()),
            removible: true,
            modelo: None,
        };
        let src = DispositivosSource::con_dispositivos(vec![info]);
        let parts = src.children(&src.children(&src.root().id).unwrap()[0].id).unwrap();
        let files = src.children(&parts[0].id).unwrap();
        let archivo = files.iter().find(|n| n.name == "grande.bin").expect("grande.bin");

        let destino = dir.path().join("salida");
        fs::create_dir_all(&destino).unwrap();
        let op = OpKind::Extraer {
            src_id: archivo.id.clone(),
            name: "grande.bin".into(),
            es_dir: false,
            dest_parent: destino.to_string_lossy().into_owned(),
        };
        op.run().unwrap().unwrap();
        // Byte-idéntico tras streamear por ventanas.
        assert_eq!(fs::read(destino.join("grande.bin")).unwrap(), grande);
    }

    #[test]
    fn absorber_un_dispositivo_a_bundle_wawa() {
        use nahual_source_core::{DispositivoInfo, DispositivosSource};
        use std::process::Command;
        if !which("mkfs.fat") || !which("mcopy") {
            eprintln!("SKIP: faltan mkfs.fat/mcopy");
            return;
        }
        let dir = tempfile::tempdir().unwrap();
        let img = dir.path().join("disco.img");
        fs::File::create(&img).unwrap().set_len(4 * 1024 * 1024).unwrap();
        assert!(Command::new("mkfs.fat").arg(&img).output().unwrap().status.success());
        let f = dir.path().join("a.txt");
        fs::write(&f, b"al grafo\n").unwrap();
        assert!(Command::new("mcopy")
            .arg("-i").arg(&img).arg(&f).arg("::a.txt")
            .output().unwrap().status.success());

        // Id del dispositivo entero.
        let info = DispositivoInfo {
            ruta: img.clone(),
            nombre: "usb".into(),
            tam: Some(fs::metadata(&img).unwrap().len()),
            removible: true,
            modelo: None,
        };
        let src = DispositivosSource::con_dispositivos(vec![info]);
        let dev = src.children(&src.root().id).unwrap()[0].id.clone();

        let bundle_parent = dir.path().join("salida");
        fs::create_dir_all(&bundle_parent).unwrap();
        let op = OpKind::AbsorberDispositivo {
            src_id: dev,
            name: "usb".into(),
            dest_parent: bundle_parent.to_string_lossy().into_owned(),
        };
        let bundle = op.run().unwrap().unwrap();
        let bundle = std::path::PathBuf::from(bundle);
        assert!(bundle.ends_with("usb-wawa"));

        // Bundle válido: ≥1 objeto y raiz.txt con un hash de 64 hex.
        let objs = fs::read_dir(&bundle)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter(|e| e.file_name().to_string_lossy().ends_with(".obj"))
            .count();
        assert!(objs >= 1, "el bundle debe tener objetos");
        let raiz = fs::read_to_string(bundle.join("raiz.txt")).unwrap();
        let raiz = raiz.trim();
        assert_eq!(raiz.len(), 64);
        assert!(raiz.chars().all(|c| c.is_ascii_hexdigit()));
    }

    #[test]
    fn sanear_neutraliza_escapes() {
        assert_eq!(sanear("a/b"), PathBuf::from("a_b"));
        assert_eq!(sanear(".."), PathBuf::from("_"));
        assert_eq!(sanear(""), PathBuf::from("_"));
        assert_eq!(sanear("normal.txt"), PathBuf::from("normal.txt"));
    }

    #[test]
    fn cola_empuja_y_limpia() {
        let mut q = OpQueue::default();
        let id = q.push(OpKind::NewDir { parent: "/tmp".into(), name: "x".into() });
        assert!(q.open);
        assert!(q.any_running());
        q.finish(id, OpStatus::Done(None));
        assert!(!q.any_running());
        q.clear_finished();
        assert!(q.ops.is_empty());
        assert!(!q.open);
    }
}
