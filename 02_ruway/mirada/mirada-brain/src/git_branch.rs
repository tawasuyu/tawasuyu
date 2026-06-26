//! Workspaces por rama de Git: vigila el `.git/HEAD` de un repo y, al cambiar de
//! rama, avisa para que mirada **intercambie la sesión** — guarda la actual bajo
//! la rama vieja y restaura la de la rama nueva. Sin CRIU ni SIGSTOP: las
//! ventanas se preservan por la persistencia de sesión normal (forma del árbol +
//! `home` por `app_id`); cada rama es un escritorio guardado.
//!
//! El parseo de `.git/HEAD` y la derivación de la ruta por rama son puros y
//! testeados; el vigía reusa el [`FileWatch`](crate::watch::FileWatch) genérico
//! (el mismo de la recarga en caliente de config/keymap).

use std::path::{Path, PathBuf};

use crate::watch::FileWatch;

/// Lee el nombre de rama del contenido de un `.git/HEAD`. Devuelve `Some(rama)`
/// para `ref: refs/heads/<rama>`, o `None` en *detached HEAD* (un SHA crudo): no
/// tiene sentido una sesión por commit suelto. Tolera espacios y el salto final.
pub fn parse_git_head(content: &str) -> Option<String> {
    let rest = content.trim().strip_prefix("ref:")?.trim();
    let branch = rest.strip_prefix("refs/heads/")?;
    (!branch.is_empty()).then(|| branch.to_string())
}

/// Ruta del archivo de sesión de una rama dentro de `dir`. Sanea los separadores
/// (`feature/login` → `feature%login.ron`) para que sea un nombre plano y no se
/// escape del directorio. Las ramas con `..` quedan inocuas porque las `/` se
/// reemplazan (`../x` → `..%x`, un nombre de archivo, no un ascenso).
pub fn branch_session_path(dir: &Path, branch: &str) -> PathBuf {
    let safe: String = branch
        .chars()
        .map(|c| if c == '/' || c == '\\' { '%' } else { c })
        .collect();
    dir.join(format!("{safe}.ron"))
}

/// Un cambio de rama detectado por [`GitBranchWatch::poll`].
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchSwitch {
    /// Rama que se dejó (la sesión actual se guardaría bajo ésta). `None` si se
    /// venía de *detached HEAD* o aún no se había leído ninguna.
    pub from: Option<String>,
    /// Rama nueva (su sesión se restauraría).
    pub to: String,
}

/// Vigía de la rama activa de un repo. Mantiene viva la vigilancia de
/// `<repo>/.git/HEAD` y recuerda la última rama vista; [`poll`](Self::poll)
/// reporta el cambio cuando ocurre.
pub struct GitBranchWatch {
    watch: FileWatch,
    head_path: PathBuf,
    current: Option<String>,
}

impl GitBranchWatch {
    /// Empieza a vigilar `<repo>/.git/HEAD` y lee la rama inicial. `None` si no
    /// hay `.git/HEAD` (el path no es un repo) o si el entorno no tiene backend
    /// de inotify (algunos sandboxes) — la feature simplemente queda inerte.
    pub fn new(repo: &Path) -> Option<GitBranchWatch> {
        let head_path = repo.join(".git").join("HEAD");
        if !head_path.exists() {
            return None;
        }
        let watch = FileWatch::new(&head_path).ok()?;
        let current = read_branch(&head_path);
        Some(GitBranchWatch {
            watch,
            head_path,
            current,
        })
    }

    /// La rama activa según la última lectura (puede ser `None` en detached).
    pub fn current(&self) -> Option<&str> {
        self.current.as_deref()
    }

    /// Consulta si la rama cambió desde la última vez. Devuelve `Some` con la
    /// rama nueva sólo cuando hubo un cambio real a otra rama con nombre; un
    /// salto a *detached HEAD* (checkout de un commit/tag) no dispara swap
    /// (`None`), pero **sí** actualiza el estado interno a "sin rama".
    pub fn poll(&mut self) -> Option<BranchSwitch> {
        if !self.watch.changed() {
            return None;
        }
        let new = read_branch(&self.head_path);
        if new == self.current {
            return None;
        }
        let from = self.current.take();
        self.current = new.clone();
        // Sólo hay swap hacia una rama con nombre; a detached no se restaura nada.
        new.map(|to| BranchSwitch { from, to })
    }
}

/// Lee `.git/HEAD` del disco y extrae la rama (o `None` si no se puede leer /
/// está en detached).
fn read_branch(head_path: &Path) -> Option<String> {
    std::fs::read_to_string(head_path)
        .ok()
        .and_then(|c| parse_git_head(&c))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parsea_rama_normal() {
        assert_eq!(parse_git_head("ref: refs/heads/main\n").as_deref(), Some("main"));
        assert_eq!(
            parse_git_head("ref: refs/heads/feature/login").as_deref(),
            Some("feature/login")
        );
    }

    #[test]
    fn detached_head_es_none() {
        // Un SHA crudo (checkout de un commit) no es una rama.
        assert_eq!(
            parse_git_head("9fceb02d0ae598e95dc970b74767f19372d61af8\n"),
            None
        );
        assert_eq!(parse_git_head("ref: refs/tags/v1.0"), None);
        assert_eq!(parse_git_head(""), None);
    }

    #[test]
    fn ruta_por_rama_sanea_separadores() {
        let dir = Path::new("/x/sessions");
        assert_eq!(
            branch_session_path(dir, "main"),
            PathBuf::from("/x/sessions/main.ron")
        );
        // `feature/login` no debe crear un subdirectorio ni escaparse.
        assert_eq!(
            branch_session_path(dir, "feature/login"),
            PathBuf::from("/x/sessions/feature%login.ron")
        );
        assert_eq!(
            branch_session_path(dir, "../escape"),
            PathBuf::from("/x/sessions/..%escape.ron")
        );
    }
}
