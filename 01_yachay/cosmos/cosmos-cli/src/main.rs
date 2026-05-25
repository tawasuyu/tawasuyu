//! `cosmos_app-cli` — cliente del service socket de Tahuantinsuyu.
//!
//! Pide cómputos de cartas sin abrir la GUI. Útil para integraciones,
//! scripts y para verificar end-to-end que el data plane brahman está
//! sirviendo. Conecta al socket que la app GUI expone (default
//! `$XDG_CACHE_HOME/cosmos_app/service.sock`).
//!
//! ## Comandos
//!
//! - `ping` — verifica que el server responde.
//! - `natal --year N --month M --day D --hour H --minute MIN
//!   --tz-min TZ --lat LAT --lon LON [--alt ALT] [--label TEXT]`
//!   — pide una carta natal y la imprime como JSON.
//!
//! ## Ejemplo
//!
//! ```bash
//! cargo run -p cosmos_app-cli -- natal \
//!     --year 1987 --month 3 --day 14 \
//!     --hour 5 --minute 22 --tz-min -240 \
//!     --lat 10.4806 --lon -66.9036 \
//!     --label "Sergio"
//! ```

use std::path::PathBuf;

use anyhow::{anyhow, Context, Result};
use clap::{Parser, Subcommand};
use cosmos_card::service::{self, ComputeRequest, ComputeResponse};
use cosmos_model::{StoredBirthData, StoredChartConfig};

#[derive(Parser)]
#[command(
    name = "cosmos_app-cli",
    version,
    about = "Cliente del service socket de Tahuantinsuyu."
)]
struct Cli {
    /// Path al service socket. Default: el resuelto por
    /// `service::default_service_socket()`.
    #[arg(long, global = true)]
    socket: Option<PathBuf>,

    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Health check — verifica que el server responde con Pong.
    Ping,
    /// Pide el cómputo de una carta natal e imprime el RenderModel
    /// como JSON.
    Natal {
        #[arg(long)]
        year: i32,
        #[arg(long)]
        month: u32,
        #[arg(long)]
        day: u32,
        #[arg(long)]
        hour: u32,
        #[arg(long)]
        minute: u32,
        #[arg(long, default_value_t = 0.0)]
        second: f64,
        /// Offset de zona horaria del lugar de nacimiento, en minutos.
        /// Ej: Argentina = -180, UTC = 0, Madrid = 60.
        #[arg(long = "tz-min")]
        tz_offset_minutes: i32,
        #[arg(long)]
        lat: f64,
        #[arg(long)]
        lon: f64,
        #[arg(long, default_value_t = 0.0)]
        alt: f64,
        /// Etiqueta del chart para el title del RenderModel.
        #[arg(long)]
        label: Option<String>,
        /// Offset adicional en minutos sobre el instante natal (útil
        /// para rectificación rápida sin guardar variantes).
        #[arg(long, default_value_t = 0)]
        offset_minutes: i64,
    },
}

fn main() -> Result<()> {
    let cli = Cli::parse();
    let socket = cli
        .socket
        .unwrap_or_else(service::default_service_socket);

    let rt = tokio::runtime::Builder::new_current_thread()
        .enable_io()
        .build()
        .context("crear tokio runtime")?;

    rt.block_on(async {
        match cli.command {
            Command::Ping => {
                let response = service::request(&socket, &ComputeRequest::Ping)
                    .await
                    .with_context(|| format!("ping a {}", socket.display()))?;
                match response {
                    ComputeResponse::Pong => {
                        println!("pong");
                        Ok(())
                    }
                    other => Err(anyhow!("respuesta inesperada al ping: {:?}", other)),
                }
            }
            Command::Natal {
                year,
                month,
                day,
                hour,
                minute,
                second,
                tz_offset_minutes,
                lat,
                lon,
                alt,
                label,
                offset_minutes,
            } => {
                let request = ComputeRequest::Natal {
                    birth: StoredBirthData {
                        year,
                        month,
                        day,
                        hour,
                        minute,
                        second,
                        tz_offset_minutes,
                        latitude_deg: lat,
                        longitude_deg: lon,
                        altitude_m: alt,
                        time_certainty: Default::default(),
                        subject_name: label.clone(),
                        birthplace_label: None,
                    },
                    config: StoredChartConfig::default(),
                    offset_minutes,
                    label,
                };
                let response = service::request(&socket, &request)
                    .await
                    .with_context(|| format!("natal request a {}", socket.display()))?;
                match response {
                    ComputeResponse::Render { render } => {
                        let json = serde_json::to_string_pretty(&render)
                            .context("serializar RenderModel a JSON")?;
                        println!("{}", json);
                        Ok(())
                    }
                    ComputeResponse::Error { message } => {
                        Err(anyhow!("server reportó error: {}", message))
                    }
                    other => Err(anyhow!("respuesta inesperada al natal: {:?}", other)),
                }
            }
        }
    })
}
