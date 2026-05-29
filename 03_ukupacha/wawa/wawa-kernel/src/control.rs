// =============================================================================
//  renaser :: kernel/src/control.rs — Fase 63 :: canal de control host->kernel
// -----------------------------------------------------------------------------
//  El virtio-console (Fase 49) ya cargaba un protocolo kernel->host: la cadena
//  de firma (`wawactl::sign_pci::` / `wawactl::sign_cfg::`) donde el KERNEL
//  emite una solicitud y el operador host responde con la firma. Esta fase
//  abre el sentido inverso: comandos de control que el HOST inicia
//  (`wawactl gc`) y el kernel atiende.
//
//  Hoy hay un solo comando: `wawactl::gc_request::` — dispara una pasada del
//  compactador semantico (`almacen::compactar`, el mismo motor de `Alt+G` de
//  la Fase 57 y de la syscall `sys_grafo_compactar` de la Fase 53) y devuelve
//  el veredicto por el mismo canal: `wawactl::gc_reply::vivos=N muertos=M
//  sectores=A->B`. Es la palanca operacional remota que pedia WAWA.md §14.1.1.
//
//  CONVIVENCIA CON LA FIRMA :: ambos protocolos comparten el ring RX del
//  virtio-console. No se pisan porque el reactor es cooperativo de un solo
//  nucleo: la syscall de firma drena el ring de forma SINCRONA dentro del tic
//  de la app (sin ceder la CPU), asi que esta tarea de control jamas corre a
//  la vez. Lo unico que esta tarea puede ver son bytes NO solicitados: o un
//  `gc_request` (que atiende), o restos de una firma que expiro (que descarta
//  silenciosamente, igual que `vaciar_input` haria en la proxima firma).
// =============================================================================

use crate::drivers::consola_virtio;

/// Prefijo que el host (`wawactl gc`) emite por el virtio-console para pedir
/// una compactacion. El host lo termina con '\n'. Distinto en cuerpo y
/// proposito de los prefijos de firma, que viajan en el sentido contrario.
const PREFIJO_GC_REQUEST: &[u8] = b"wawactl::gc_request::";

/// Cota dura de una linea de control. Una solicitud es corta; cualquier cosa
/// mas larga es ruido y se descarta hasta el proximo salto de linea.
const LINEA_CAP: usize = 64;

/// Acumulador de la linea de control en curso. Vive como variable local de la
/// tarea (un solo lector), no como estatico: sin cerrojos, sin contienda.
struct Acumulador {
    buf: [u8; LINEA_CAP],
    n: usize,
    /// La linea actual reboso `LINEA_CAP`: la ignoramos entera hasta el '\n'.
    desbordada: bool,
}

impl Acumulador {
    const fn nuevo() -> Self {
        Acumulador {
            buf: [0; LINEA_CAP],
            n: 0,
            desbordada: false,
        }
    }

    /// Absorbe un byte del RX. En el salto de linea evalua la linea y la
    /// reinicia. '\r' se ignora (tolera CRLF del host). El desborde marca la
    /// linea como basura sin perder el sincronismo con el proximo '\n'.
    fn empujar(&mut self, b: u8) {
        match b {
            b'\n' => {
                if !self.desbordada {
                    self.evaluar();
                }
                self.n = 0;
                self.desbordada = false;
            }
            b'\r' => {}
            _ => {
                if self.n < LINEA_CAP {
                    self.buf[self.n] = b;
                    self.n += 1;
                } else {
                    self.desbordada = true;
                }
            }
        }
    }

    /// Despacha la linea completa contra el catalogo de comandos de control.
    fn evaluar(&self) {
        if &self.buf[..self.n] == PREFIJO_GC_REQUEST {
            atender_gc();
        }
        // Comandos futuros (p.ej. `wawactl::raiz_request::`) se ramifican aqui.
    }
}

/// Ejecuta la compactacion y responde por el mismo canal + deja huella en la
/// baliza serial para el log local del operador.
fn atender_gc() {
    use core::fmt::Write;
    let reply = match crate::almacen::compactar() {
        Ok(stats) => alloc::format!(
            "wawactl::gc_reply::vivos={} muertos={} sectores={}->{}\n",
            stats.nodos_vivos,
            stats.nodos_muertos,
            stats.sectores_antes,
            stats.sectores_despues,
        ),
        Err(motivo) => alloc::format!("wawactl::gc_reply::error::{}\n", motivo),
    };
    consola_virtio::escribir(reply.as_bytes());
    let _ = write!(crate::baliza::Serie, "gc :: remoto (wawactl) :: {}", reply);
}

/// Tarea del reactor: en cada fotograma drena el virtio-console y alimenta el
/// acumulador. Solo se engendra si el dispositivo se monto. No bloquea: el
/// trabajo pesado (la compactacion) solo ocurre cuando llega un `gc_request`
/// completo, un gesto explicito y poco frecuente del operador.
pub async fn tarea_consola_control() {
    let mut acc = Acumulador::nuevo();
    let mut tmp = [0u8; 64];
    loop {
        crate::async_system::reloj::EsperaFrame::nueva().await;
        consola_virtio::drenar_input();
        loop {
            let leidos = consola_virtio::leer_disponible(&mut tmp);
            if leidos == 0 {
                break;
            }
            for &b in &tmp[..leidos] {
                acc.empujar(b);
            }
        }
    }
}
