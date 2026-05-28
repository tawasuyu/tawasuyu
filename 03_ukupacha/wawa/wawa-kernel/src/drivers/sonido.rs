// =============================================================================
//  renaser :: kernel/src/drivers/sonido.rs — Fase 62 :: la voz de verdad
// -----------------------------------------------------------------------------
//  La bocina del PC (Fase 12, `altavoz`) es un solo bit: una onda cuadrada del
//  canal 2 del PIT, sin PCM, sin DMA, sin mezclador. La Fase 62 cierra el arco
//  «legacy -> virtio moderno» (consola 49, scanout 60, puntero 61) con el
//  audio: un dispositivo `virtio-sound` recibe MUESTRAS PCM reales por DMA.
//
//  El reto es de impedancia: el contrato del kernel es por FRECUENCIA
//  (`altavoz::tono(hz)`, secuencias `(freq, dur_ms)`), pero virtio-sound quiere
//  un flujo de PERIODOS de PCM. Este modulo es el puente: SINTETIZA la onda
//  —cuadrada, del timbre de la vieja bocina— y la transmite periodo a periodo.
//
//  La reproduccion NO bloquea el reactor. `pcm_xfer_nb` encola un periodo y
//  devuelve un token sin esperar; `pcm_xfer_ok` lo recupera cuando el
//  dispositivo lo consumio —y devuelve `NotReady` sin colgar si aun no—. Una
//  tarea cooperativa (`bombear`, un fotograma si y otro tambien) mantiene unos
//  pocos periodos EN VUELO: recupera los terminados en orden FIFO y rellena la
//  tuberia, de modo que el dispositivo nunca cae en underrun y el escritorio
//  jamas se congela esperando al audio.
//
//  Mezclador minimo: las notas del kernel (la voz del sistema —acorde de
//  bienvenida, repiques de lanzar/cerrar/desalojar—) tienen PRIORIDAD; cuando
//  no hay nota viva, se rellena con el tono sostenido que una app fijo via
//  `sys_tono`; si tampoco, silencio. `altavoz` enruta hacia aqui cuando el
//  dispositivo existe; si no, recae en la bocina del PIT —nada se rompe—.
// =============================================================================

use core::sync::atomic::{AtomicU32, Ordering};

use alloc::collections::VecDeque;
use spin::{Mutex, Once};
use virtio_drivers::device::sound::{PcmFeatures, PcmFormat, PcmRate, VirtIOSound};
use virtio_drivers::transport::pci::bus::{Command, DeviceFunction, PciRoot};
use virtio_drivers::transport::pci::PciTransport;
use virtio_drivers::Error;

use super::disco::KernelHal;
use super::pci::CamPuertos;

/// Vendor ID de VirtIO; Device ID de un dispositivo de sonido. `virtio-sound`
/// es de la era virtio-1.0: su unico ID es el moderno `0x1040 + 25 = 0x1059`.
const VENDOR_VIRTIO: u16 = 0x1AF4;
const VIRTIO_SOUND_IDS: [u16; 1] = [0x1059];

// --- Parametros del flujo PCM. S16, estereo, 44.1 kHz: lo mas universal. ----
/// Frecuencia de muestreo, en Hz.
const RATE_HZ: u32 = 44_100;
/// Canales (estereo). Cada frame lleva una muestra por canal.
const CANALES: u8 = 2;
/// Bytes por frame: 2 (S16) x 2 canales.
const BYTES_POR_FRAME: usize = 2 * CANALES as usize;
/// Frames por periodo — el grano de transferencia. 1024 @ 44.1 kHz ≈ 23 ms.
const PERIODO_FRAMES: usize = 1024;
/// Bytes por periodo. Debe coincidir EXACTO con lo que pide `pcm_xfer_nb`.
const PERIODO_BYTES: usize = PERIODO_FRAMES * BYTES_POR_FRAME;
/// Tamaño del bufer del dispositivo: 4 periodos. Multiplo de `PERIODO_BYTES`.
const BUFFER_BYTES: u32 = (PERIODO_BYTES * 4) as u32;
/// Periodos que mantenemos EN VUELO. Menos que los 4 del bufer, para dejar
/// holgura: ~3 x 23 ms ≈ 70 ms amortiguados contra el jitter del reactor.
const MAX_EN_VUELO: usize = 3;
/// Amplitud de la onda cuadrada — moderada, lejos del recorte a fondo de escala.
const AMPLITUD: i16 = 5_000;

// =============================================================================
//  EL MEZCLADOR — de frecuencias a muestras PCM
// =============================================================================

/// La cola de notas del kernel pendientes — `(frecuencia_hz, duracion_ms)`.
/// La llena `agendar` (la voz del sistema); la drena la fuente, nota a nota.
/// `const`-inicializable, asi vive sin un `Once`.
static COLA_NOTAS: Mutex<VecDeque<(u32, u32)>> = Mutex::new(VecDeque::new());

/// El tono SOSTENIDO que una app fijo via `sys_tono` (0 = silencio). Se rellena
/// con el cuando no hay nota del kernel viva — las notas del kernel mandan.
static TONO_APP: AtomicU32 = AtomicU32::new(0);

/// El estado de sintesis: la nota viva y la fase de la onda. La onda es
/// continua entre periodos (la fase persiste) para no introducir chasquidos.
#[derive(Default)]
struct Fuente {
    /// Frecuencia de la nota del kernel en curso (0 si ninguna).
    freq_nota: u32,
    /// Frames que le restan a la nota del kernel en curso.
    frames_nota: u32,
    /// Fase de la onda cuadrada — un contador de frames, modulo el periodo.
    fase: u32,
}

impl Fuente {
    /// Rellena `buf` (exactamente `PERIODO_BYTES`) con el proximo periodo de
    /// PCM. Por cada frame: si hay nota del kernel viva, la usa y la descuenta;
    /// si no, recurre al tono sostenido de la app (o silencio). Asi una nota
    /// que acaba a mitad de periodo cede sin costura al tono de fondo.
    fn siguiente_periodo(&mut self, buf: &mut [u8]) {
        let tono_app = TONO_APP.load(Ordering::Relaxed);
        for frame in 0..PERIODO_FRAMES {
            if self.frames_nota == 0 {
                if let Some((freq, dur_ms)) = COLA_NOTAS.lock().pop_front() {
                    self.freq_nota = freq;
                    self.frames_nota = (RATE_HZ as u64 * dur_ms as u64 / 1000) as u32;
                    self.fase = 0;
                }
            }
            let freq = if self.frames_nota > 0 {
                self.freq_nota
            } else {
                tono_app
            };
            let muestra = onda_cuadrada(freq, &mut self.fase);
            let le = muestra.to_le_bytes();
            let off = frame * BYTES_POR_FRAME;
            // Misma muestra a ambos canales — mono difundido a estereo.
            buf[off..off + 2].copy_from_slice(&le);
            buf[off + 2..off + 4].copy_from_slice(&le);
            if self.frames_nota > 0 {
                self.frames_nota -= 1;
            }
        }
    }
}

/// Una muestra de onda cuadrada a `freq` Hz, avanzando la `fase`. `freq == 0`
/// es silencio (y no avanza la fase). El timbre —cuadrado— es deliberado: hereda
/// el caracter de la vieja bocina del PIT, no busca fidelidad de instrumento.
fn onda_cuadrada(freq: u32, fase: &mut u32) -> i16 {
    if freq == 0 {
        return 0;
    }
    // Frames por ciclo completo de la onda. `.max(2)` evita un periodo nulo
    // ante una frecuencia absurda por encima de Nyquist.
    let periodo = (RATE_HZ / freq).max(2);
    let pos = *fase % periodo;
    *fase = fase.wrapping_add(1);
    // Primera mitad del ciclo arriba, segunda abajo.
    if pos * 2 < periodo {
        AMPLITUD
    } else {
        -AMPLITUD
    }
}

// =============================================================================
//  EL DISPOSITIVO
// =============================================================================

/// El dispositivo de sonido, ya montado y con su flujo de salida arrancado.
struct Sonido {
    dev: VirtIOSound<KernelHal, PciTransport>,
    /// Identificador del flujo de salida que reproducimos.
    stream_id: u32,
    /// Tokens de los periodos EN VUELO, en orden de envio (se recuperan FIFO).
    en_vuelo: VecDeque<u16>,
    /// El estado del mezclador.
    fuente: Fuente,
}

// SEGURIDAD: `Sonido` encierra punteros crudos a las colas virtio y al MMIO del
// dispositivo. renaser es de un solo nucleo y todo acceso se serializa tras el
// `Mutex` global. No hay manejador de IRQ que lo dispute: el bombeo es por
// sondeo desde el reactor cooperativo, con las interrupciones habilitadas.
unsafe impl Send for Sonido {}

/// El dispositivo global. Se monta una sola vez, en `montar`.
static SONIDO: Once<Mutex<Sonido>> = Once::new();

/// Enumera el bus PCI, localiza un `virtio-sound`, monta su transporte moderno
/// y arranca su primer flujo de SALIDA con parametros S16/estereo/44.1 kHz.
/// Toda falla —sin dispositivo, sin flujo de salida, parametros rechazados— se
/// devuelve como `Err`: el sonido recae limpiamente en la bocina del PIT.
pub fn montar() -> Result<(), &'static str> {
    let mut raiz = PciRoot::new(CamPuertos);

    // 1. Localizar el primer virtio-sound del bus.
    let mut hallado: Option<DeviceFunction> = None;
    'busqueda: for bus in 0..=255u8 {
        for (device_function, info) in raiz.enumerate_bus(bus) {
            if info.vendor_id == VENDOR_VIRTIO && VIRTIO_SOUND_IDS.contains(&info.device_id) {
                hallado = Some(device_function);
                break 'busqueda;
            }
        }
    }
    let device_function = hallado.ok_or("virtio-sound no hallado en el bus PCI")?;

    // 2. Habilitar E/S, memoria y BUS-MASTER: el dispositivo leera el PCM por DMA.
    raiz.set_command(
        device_function,
        Command::IO_SPACE | Command::MEMORY_SPACE | Command::BUS_MASTER,
    );

    // 3. Montar el transporte y el dispositivo de sonido.
    let transporte = PciTransport::new::<KernelHal, _>(&mut raiz, device_function)
        .map_err(|_| "no se pudo montar el transporte PCI de virtio-sound")?;
    let mut dev = VirtIOSound::<KernelHal, _>::new(transporte)
        .map_err(|_| "no se pudo inicializar el dispositivo virtio-sound")?;

    // 4. Elegir el primer flujo de SALIDA y configurarlo, prepararlo, arrancarlo.
    let salidas = dev
        .output_streams()
        .map_err(|_| "virtio-sound no enumero sus flujos de salida")?;
    let stream_id = *salidas
        .first()
        .ok_or("virtio-sound no expone ningun flujo de salida")?;

    dev.pcm_set_params(
        stream_id,
        BUFFER_BYTES,
        PERIODO_BYTES as u32,
        PcmFeatures::empty(),
        CANALES,
        PcmFormat::S16,
        PcmRate::Rate44100,
    )
    .map_err(|_| "virtio-sound rechazo los parametros PCM (S16/estereo/44.1 kHz)")?;
    dev.pcm_prepare(stream_id)
        .map_err(|_| "virtio-sound no pudo preparar el flujo de salida")?;
    dev.pcm_start(stream_id)
        .map_err(|_| "virtio-sound no pudo arrancar el flujo de salida")?;

    SONIDO.call_once(|| {
        Mutex::new(Sonido {
            dev,
            stream_id,
            en_vuelo: VecDeque::new(),
            fuente: Fuente::default(),
        })
    });
    Ok(())
}

/// ¿Gobierna el kernel un dispositivo virtio-sound? `true` solo tras un `montar`
/// con exito. `altavoz` lo consulta para decidir si enruta el sonido aqui o a
/// la bocina del PIT.
pub fn disponible() -> bool {
    SONIDO.get().is_some()
}

/// Agenda una secuencia de notas del kernel — la voz del sistema. Cada
/// `(frecuencia_hz, duracion_ms)` sonara en orden; `frecuencia_hz = 0` es una
/// pausa. Espeja `altavoz::agendar`, que enruta aqui cuando hay dispositivo.
pub fn agendar(secuencia: &[(u32, u32)]) {
    let mut cola = COLA_NOTAS.lock();
    for &nota in secuencia {
        cola.push_back(nota);
    }
}

/// Fija el tono SOSTENIDO de una app (`sys_tono`). Suena cuando no hay nota del
/// kernel viva; `0` lo calla. No interrumpe la voz del kernel — esta tiene
/// prioridad en el mezclador.
pub fn fijar_tono_app(frecuencia_hz: u32) {
    TONO_APP.store(frecuencia_hz, Ordering::Relaxed);
}

/// Bombea el flujo de salida: recupera los periodos que el dispositivo ya
/// consumio (FIFO) y rellena la tuberia hasta `MAX_EN_VUELO`. La invoca la
/// tarea de sonido en cada fotograma. No-op si no hay dispositivo. Mantiene el
/// flujo SIEMPRE alimentado —con silencio cuando no hay audio— para que el
/// dispositivo no caiga en underrun y detenga la reproduccion.
pub fn bombear() {
    let Some(sonido) = SONIDO.get() else {
        return;
    };
    let mut guardia = sonido.lock();
    // Tomar prestados los campos por separado: `pcm_xfer_ok`/`pcm_xfer_nb`
    // necesitan `&mut dev` mientras leemos/escribimos `en_vuelo` y `fuente`.
    let Sonido {
        dev,
        stream_id,
        en_vuelo,
        fuente,
    } = &mut *guardia;
    let stream_id = *stream_id;

    // 1. Recuperar, en orden, los periodos terminados. `NotReady` corta el
    //    barrido (el frente aun no se consumio); cualquier otro error descarta
    //    ese token para no atascar la cola.
    while let Some(&token) = en_vuelo.front() {
        match dev.pcm_xfer_ok(token) {
            Ok(()) => {
                en_vuelo.pop_front();
            }
            Err(Error::NotReady) => break,
            Err(_) => {
                en_vuelo.pop_front();
            }
        }
    }

    // 2. Rellenar la tuberia. Generamos periodos —audio o silencio— hasta el
    //    techo de vuelo o hasta que el envio falle (cola virtio llena).
    let mut periodo = [0u8; PERIODO_BYTES];
    while en_vuelo.len() < MAX_EN_VUELO {
        fuente.siguiente_periodo(&mut periodo);
        match dev.pcm_xfer_nb(stream_id, &periodo) {
            Ok(token) => en_vuelo.push_back(token),
            Err(_) => break,
        }
    }
}
