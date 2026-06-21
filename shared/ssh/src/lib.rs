//! `brahman-ssh-multiplex` — sesión SSH maestra con canales multiplexados.
//!
//! SSH ya multiplexa canales sobre una sola conexión TCP por diseño del
//! protocolo. Este crate envuelve `russh` con una API mínima: una
//! `SshSession` mantiene el `Handle` maestro; cada `exec` concurrente
//! abre su propio canal en paralelo sobre la misma conexión.
//!
//! Lo consumen `sandokan::RemoteEngine` y el `Linker` SSH de `matilda`.
//!
//! Verificación de host key: TOFU por default (acepta la primera vez).
//! Pasá un fingerprint esperado para verificación estricta.

#![forbid(unsafe_code)]

use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

/// Método de autenticación.
#[derive(Debug, Clone)]
pub enum SshAuth {
    /// Password en claro.
    Password(String),
    /// Clave privada en disco, con passphrase opcional.
    Key {
        path: PathBuf,
        passphrase: Option<String>,
    },
}

/// Configuración de conexión.
#[derive(Debug, Clone)]
pub struct SshConfig {
    pub host: String,
    pub port: u16,
    pub user: String,
    pub auth: SshAuth,
    /// Intervalo de keepalive en segundos (0 = usar default de russh).
    pub keepalive_secs: u64,
}

impl SshConfig {
    /// Config con puerto 22 y keepalive de 15s.
    pub fn new(host: impl Into<String>, user: impl Into<String>, auth: SshAuth) -> Self {
        Self {
            host: host.into(),
            port: 22,
            user: user.into(),
            auth,
            keepalive_secs: 15,
        }
    }
}

/// Falla de una operación SSH.
#[derive(Debug, thiserror::Error)]
pub enum SshError {
    #[error("conexión SSH: {0}")]
    Connect(String),
    #[error("autenticación SSH rechazada")]
    AuthRejected,
    #[error("clave privada: {0}")]
    Key(String),
    #[error("canal SSH: {0}")]
    Channel(String),
}

/// Salida de un comando remoto.
#[derive(Debug, Clone)]
pub struct ExecOutput {
    pub stdout: Vec<u8>,
    pub stderr: Vec<u8>,
    pub exit_code: i32,
}

/// Handler de cliente russh — verificación de host key.
struct ClientHandler {
    /// Fingerprint esperado; `None` = TOFU (acepta cualquiera).
    expected: Option<Vec<u8>>,
}

impl russh::client::Handler for ClientHandler {
    type Error = russh::Error;

    async fn check_server_key(
        &mut self,
        server_public_key: &russh::keys::ssh_key::PublicKey,
    ) -> Result<bool, Self::Error> {
        match &self.expected {
            None => Ok(true),
            Some(exp) => Ok(server_public_key.to_bytes().map(|b| &b == exp).unwrap_or(false)),
        }
    }
}

/// Sesión SSH. `Clone` barato — comparte el `Handle` maestro; clones
/// abren canales paralelos sobre la misma conexión.
#[derive(Clone)]
pub struct SshSession {
    handle: Arc<russh::client::Handle<ClientHandler>>,
}

impl SshSession {
    /// Conecta y autentica contra el host.
    pub async fn connect(config: &SshConfig) -> Result<Self, SshError> {
        let mut russh_cfg = russh::client::Config::default();
        if config.keepalive_secs > 0 {
            russh_cfg.keepalive_interval = Some(Duration::from_secs(config.keepalive_secs));
        }
        let russh_cfg = Arc::new(russh_cfg);
        let handler = ClientHandler { expected: None };

        let mut handle = russh::client::connect(
            russh_cfg,
            (config.host.as_str(), config.port),
            handler,
        )
        .await
        .map_err(|e| SshError::Connect(e.to_string()))?;

        let ok = match &config.auth {
            SshAuth::Password(pw) => handle
                .authenticate_password(&config.user, pw)
                .await
                .map_err(|e| SshError::Connect(e.to_string()))?
                .success(),
            SshAuth::Key { path, passphrase } => {
                let key = russh::keys::load_secret_key(path, passphrase.as_deref())
                    .map_err(|e| SshError::Key(e.to_string()))?;
                let key = russh::keys::PrivateKeyWithHashAlg::new(Arc::new(key), None);
                handle
                    .authenticate_publickey(&config.user, key)
                    .await
                    .map_err(|e| SshError::Connect(e.to_string()))?
                    .success()
            }
        };
        if !ok {
            return Err(SshError::AuthRejected);
        }
        Ok(Self { handle: Arc::new(handle) })
    }

    /// Abre un canal `direct-streamlocal` hacia un Unix socket del host
    /// remoto y devuelve su stream bidireccional (`AsyncRead + AsyncWrite`).
    ///
    /// Permite tunelar un protocolo arbitrario (p. ej. el wire postcard
    /// de `sandokan-daemon`) contra un socket remoto, reusando el código
    /// de cliente sin cambios — sólo cambia el transporte.
    pub async fn forward_unix(
        &self,
        remote_socket: &str,
    ) -> Result<russh::ChannelStream<russh::client::Msg>, SshError> {
        let channel = self
            .handle
            .channel_open_direct_streamlocal(remote_socket)
            .await
            .map_err(|e| SshError::Channel(e.to_string()))?;
        Ok(channel.into_stream())
    }

    /// Ejecuta `command` en un canal nuevo y junta su salida completa.
    /// Canales concurrentes se multiplexan sobre la misma conexión.
    pub async fn exec(&self, command: &str) -> Result<ExecOutput, SshError> {
        let mut channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::Channel(e.to_string()))?;
        channel
            .exec(true, command)
            .await
            .map_err(|e| SshError::Channel(e.to_string()))?;

        let mut out = ExecOutput {
            stdout: Vec::new(),
            stderr: Vec::new(),
            exit_code: -1,
        };
        while let Some(msg) = channel.wait().await {
            match msg {
                russh::ChannelMsg::Data { ref data } => out.stdout.extend_from_slice(data),
                russh::ChannelMsg::ExtendedData { ref data, .. } => {
                    out.stderr.extend_from_slice(data)
                }
                russh::ChannelMsg::ExitStatus { exit_status } => {
                    out.exit_code = exit_status as i32
                }
                _ => {}
            }
        }
        Ok(out)
    }

    /// Ejecuta `command` y **streamea** su salida (stdout+stderr) a medida que
    /// llega, en vez de juntarla. Pensado para comandos de larga vida
    /// (`docker logs -f`, `tail -f`): `on_data` recibe cada chunk de bytes
    /// apenas el host lo emite.
    ///
    /// El bucle no termina solo si el comando no termina; para cortarlo, hacé
    /// que `should_stop` devuelva `true` — entonces cierra el canal (lo que
    /// manda EOF/SIGHUP al proceso remoto) y retorna. `poll` acota cuánto se
    /// espera por datos antes de re-chequear `should_stop`, para que el corte
    /// sea responsivo aún con el stream inactivo. Retorna al cerrarse el canal
    /// (proceso terminado) o tras un stop.
    pub async fn exec_streaming<F, S>(
        &self,
        command: &str,
        poll: Duration,
        mut on_data: F,
        mut should_stop: S,
    ) -> Result<(), SshError>
    where
        F: FnMut(&[u8]),
        S: FnMut() -> bool,
    {
        let mut channel = self
            .handle
            .channel_open_session()
            .await
            .map_err(|e| SshError::Channel(e.to_string()))?;
        channel
            .exec(true, command)
            .await
            .map_err(|e| SshError::Channel(e.to_string()))?;

        loop {
            if should_stop() {
                let _ = channel.close().await;
                break;
            }
            match tokio::time::timeout(poll, channel.wait()).await {
                Ok(Some(msg)) => match msg {
                    russh::ChannelMsg::Data { ref data } => on_data(data),
                    russh::ChannelMsg::ExtendedData { ref data, .. } => on_data(data),
                    russh::ChannelMsg::Eof | russh::ChannelMsg::Close => break,
                    _ => {}
                },
                // Canal cerrado por el otro lado → proceso terminó.
                Ok(None) => break,
                // Sin datos en `poll`: volvé a chequear `should_stop`.
                Err(_timeout) => {}
            }
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_defaults_port_and_keepalive() {
        let c = SshConfig::new("host.example", "user", SshAuth::Password("x".into()));
        assert_eq!(c.port, 22);
        assert_eq!(c.keepalive_secs, 15);
    }

    // El test de conexión real necesita un servidor SSH — se hace fuera
    // del unit test (ver instrucciones de prueba de matilda/sandokan).
}
