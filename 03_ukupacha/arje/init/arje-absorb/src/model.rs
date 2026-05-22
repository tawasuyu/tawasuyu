//! El modelo intermedio: un servicio descubierto en el init ajeno,
//! independiente de su formato de origen. Cada absorber
//! (`sysvinit`, `runit`, `dinit`, `openrc`) produce [`ForeignService`]s;
//! `card` los traduce a Cards brahman.

/// Un servicio leído de la configuración de otro init.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ForeignService {
    /// Etiqueta del servicio — pasa a ser el `label` de su Card.
    pub name: String,
    /// Ejecutable (path absoluto, tal como existirá en el sistema).
    pub exec: String,
    /// Argumentos del ejecutable.
    pub argv: Vec<String>,
    /// Variables de entorno (clave, valor) a inyectarle.
    pub env: Vec<(String, String)>,
    /// Daemon supervisado, o tarea de una sola ejecución.
    pub kind: ServiceKind,
}

/// Cómo trata arje a un servicio absorbido.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ServiceKind {
    /// Servicio de larga duración: arje lo supervisa y reinicia.
    Daemon,
    /// Tarea puntual: corre una vez y termina.
    OneShot,
}

/// Parte una línea de comando en `(exec, argv)`, respetando comillas
/// simples y dobles. Best-effort: no expande variables ni globs —
/// suficiente para los campos `process`/`command` de los inits
/// clásicos. `None` si la línea no tiene ningún token.
pub fn split_command(line: &str) -> Option<(String, Vec<String>)> {
    let mut tokens: Vec<String> = Vec::new();
    let mut cur = String::new();
    let mut in_token = false;
    let mut chars = line.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '"' | '\'' => {
                in_token = true;
                let quote = c;
                for q in chars.by_ref() {
                    if q == quote {
                        break;
                    }
                    cur.push(q);
                }
            }
            c if c.is_whitespace() => {
                if in_token {
                    tokens.push(std::mem::take(&mut cur));
                    in_token = false;
                }
            }
            c => {
                in_token = true;
                cur.push(c);
            }
        }
    }
    if in_token {
        tokens.push(cur);
    }
    if tokens.is_empty() {
        return None;
    }
    let exec = tokens.remove(0);
    Some((exec, tokens))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splits_plain_command() {
        let (exec, argv) = split_command("/usr/sbin/sshd -D -e").unwrap();
        assert_eq!(exec, "/usr/sbin/sshd");
        assert_eq!(argv, ["-D", "-e"]);
    }

    #[test]
    fn respects_double_quotes() {
        let (exec, argv) = split_command(r#"/bin/foo "arg con espacios" bar"#).unwrap();
        assert_eq!(exec, "/bin/foo");
        assert_eq!(argv, ["arg con espacios", "bar"]);
    }

    #[test]
    fn respects_single_quotes() {
        let (exec, argv) = split_command("/bin/sh -c 'echo hola mundo'").unwrap();
        assert_eq!(exec, "/bin/sh");
        assert_eq!(argv, ["-c", "echo hola mundo"]);
    }

    #[test]
    fn empty_line_yields_none() {
        assert!(split_command("   ").is_none());
        assert!(split_command("").is_none());
    }
}
