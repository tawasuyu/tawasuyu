// =============================================================================
//  renaser :: apps/pregon — Fase 19 :: voz del userspace hacia la red
// -----------------------------------------------------------------------------
//  La Fase 18 hizo que el kernel pudiera hablar con la red; la Fase 19 le da la
//  misma capacidad a los apps via `sys_net_*`. `pregon` es el primer app que
//  la usa: al arrancar pide su MAC al host, anuncia su presencia con un frame
//  Ethernet broadcast (EtherType experimental 0x88B5) y, en cada `tick`, drena
//  un paquete de la cola RX y lo muestra en pantalla.
//
//  El kernel envia tambien al arrancar un ARP request al gateway de QEMU
//  (Fase 18); `pregon` recibira su respuesta como uno de los primeros paquetes
//  RX — visible aqui en la linea «ultimo».
// =============================================================================

#![no_std]

use font8x8::legacy::BASIC_LEGACY;

#[link(wasm_import_module = "renaser")]
extern "C" {
    fn sys_render_frame(ptr: u32, len: u32);
    fn sys_net_mac(salida: u32) -> i32;
    fn sys_net_enviar(ptr: u32, len: u32) -> i32;
    fn sys_net_recibir(salida: u32, capacidad: u32) -> i32;
}

#[panic_handler]
fn al_fallar(_: &core::panic::PanicInfo) -> ! {
    loop {}
}

// --- Geometria del lienzo ----------------------------------------------------

const ANCHO: usize = 480;
const ALTO: usize = 160;
const PASO: usize = 16;
const MARGEN: usize = 16;

const FONDO: u32 = 0x0A_18_30;
const TINTA: u32 = 0xE8_EC_F4;
const TINTA_TENUE: u32 = 0x6A_70_80;
const ETIQUETA: u32 = 0x8B_5C_F6;

// --- Constantes del paquete que enviamos ------------------------------------

const ETHER_TYPE_RENASER: u16 = 0x88B5;
const MENSAJE: &[u8; 28] = b"renaser :: hola desde mi red";
const FRAME_LEN: usize = 14 + 28;

// --- Estado del app ---------------------------------------------------------

static mut MAC: [u8; 6] = [0; 6];
static mut MAC_OK: bool = false;
static mut TX: u32 = 0;
static mut RX: u32 = 0;
static mut BUFFER_RX: [u8; 1600] = [0; 1600];
static mut ULTIMA_LEN: usize = 0;
static mut LIENZO: [u32; ANCHO * ALTO] = [0; ANCHO * ALTO];

// --- ABI del userspace ------------------------------------------------------

#[no_mangle]
pub extern "C" fn init() {
    // 1. Pedir la MAC al host. -1 significa «no hay red montada».
    let mac = unsafe { &mut *core::ptr::addr_of_mut!(MAC) };
    // SEGURIDAD: `sys_net_mac` escribe seis bytes en (ptr, 6); el host lo valida.
    let r = unsafe { sys_net_mac(mac.as_mut_ptr() as u32) };
    if r == 0 {
        unsafe {
            MAC_OK = true;
        }
        // 2. Anunciar la presencia con un broadcast Ethernet de EtherType
        //    experimental + un mensaje legible. El payload aparece tal cual en
        //    cualquier pcap, sin descifrar nada.
        let mut frame = [0u8; FRAME_LEN];
        frame[0..6].copy_from_slice(&[0xff; 6]); // destino: broadcast
        frame[6..12].copy_from_slice(mac);
        frame[12..14].copy_from_slice(&ETHER_TYPE_RENASER.to_be_bytes());
        frame[14..].copy_from_slice(MENSAJE);
        // SEGURIDAD: `sys_net_enviar` lee (ptr, len) de nuestra memoria.
        let r = unsafe { sys_net_enviar(frame.as_ptr() as u32, frame.len() as u32) };
        if r == 0 {
            unsafe {
                TX += 1;
            }
        }
    }
    pintar();
}

#[no_mangle]
pub extern "C" fn tick() {
    // Drenar UN paquete por tick — si la cola RX tiene mas, se atendera en
    // las proximas vueltas. Cero coste cuando no hay nada que leer.
    let buf = unsafe { &mut *core::ptr::addr_of_mut!(BUFFER_RX) };
    let n = unsafe { sys_net_recibir(buf.as_mut_ptr() as u32, buf.len() as u32) };
    if n > 0 {
        unsafe {
            RX += 1;
            ULTIMA_LEN = n as usize;
        }
    }
    pintar();
}

// --- Renderizado ------------------------------------------------------------

fn pintar() {
    let lienzo = unsafe { &mut *core::ptr::addr_of_mut!(LIENZO) };
    for p in lienzo.iter_mut() {
        *p = FONDO;
    }

    // Titulo + subrayado.
    pintar_texto(lienzo, b"pregon :: voz hacia la red", MARGEN, 8, ETIQUETA);
    let y_linea = 8 + PASO + 4;
    for x in MARGEN..(ANCHO - MARGEN) {
        lienzo[y_linea * ANCHO + x] = ETIQUETA;
        lienzo[(y_linea + 1) * ANCHO + x] = ETIQUETA;
    }

    let mac_ok = unsafe { MAC_OK };
    if !mac_ok {
        pintar_texto(lienzo, b"sin red -- no hay tarjeta montada", MARGEN, 48, TINTA);
        volcar(lienzo);
        return;
    }

    let mac = unsafe { &*core::ptr::addr_of!(MAC) };
    let tx = unsafe { TX };
    let rx = unsafe { RX };

    // Linea MAC: "mac: xx:xx:xx:xx:xx:xx"
    {
        let mut linea = [b' '; 22];
        linea[0..5].copy_from_slice(b"mac: ");
        let mut p = 5;
        for (i, &b) in mac.iter().enumerate() {
            let par = hex_byte(b);
            linea[p..p + 2].copy_from_slice(&par);
            p += 2;
            if i < 5 {
                linea[p] = b':';
                p += 1;
            }
        }
        pintar_texto(lienzo, &linea, MARGEN, 44, TINTA);
    }

    // Linea TX/RX: "tx: N    rx: M"
    {
        let mut linea = [b' '; 20];
        linea[0..4].copy_from_slice(b"tx: ");
        let mut buf = [0u8; 10];
        let s = dec(tx, &mut buf);
        linea[4..4 + s.len()].copy_from_slice(s);
        let p = 4 + s.len() + 4;
        if p + 4 <= linea.len() {
            linea[p..p + 4].copy_from_slice(b"rx: ");
            let mut buf2 = [0u8; 10];
            let s2 = dec(rx, &mut buf2);
            let q = p + 4;
            if q + s2.len() <= linea.len() {
                linea[q..q + s2.len()].copy_from_slice(s2);
            }
        }
        pintar_texto(lienzo, &linea, MARGEN, 72, TINTA);
    }

    // Linea «ultimo»: cuantos bytes, tipo y src.
    let len = unsafe { ULTIMA_LEN };
    if len >= 14 {
        let buf = unsafe { &*core::ptr::addr_of!(BUFFER_RX) };
        let etype = u16::from_be_bytes([buf[12], buf[13]]);
        // Linea 1: "ultimo: N bytes  type=0x????"
        let mut linea = [b' '; 30];
        linea[0..8].copy_from_slice(b"ultimo: ");
        let mut dbuf = [0u8; 10];
        let s = dec(len as u32, &mut dbuf);
        linea[8..8 + s.len()].copy_from_slice(s);
        let mut p = 8 + s.len();
        linea[p..p + 8].copy_from_slice(b" bytes  ");
        p += 8;
        if p + 7 <= linea.len() {
            linea[p..p + 7].copy_from_slice(b"type=0x");
            p += 7;
            let hi = hex_byte((etype >> 8) as u8);
            let lo = hex_byte((etype & 0xff) as u8);
            if p + 4 <= linea.len() {
                linea[p..p + 2].copy_from_slice(&hi);
                linea[p + 2..p + 4].copy_from_slice(&lo);
            }
        }
        pintar_texto(lienzo, &linea, MARGEN, 104, TINTA_TENUE);

        // Linea 2: "src: xx:xx:xx:xx:xx:xx"
        let src = &buf[6..12];
        let mut linea2 = [b' '; 22];
        linea2[0..5].copy_from_slice(b"src: ");
        let mut p = 5;
        for (i, &b) in src.iter().enumerate() {
            let par = hex_byte(b);
            linea2[p..p + 2].copy_from_slice(&par);
            p += 2;
            if i < 5 {
                linea2[p] = b':';
                p += 1;
            }
        }
        pintar_texto(lienzo, &linea2, MARGEN, 132, TINTA);
    } else {
        pintar_texto(lienzo, b"esperando primer paquete...", MARGEN, 104, TINTA_TENUE);
    }

    volcar(lienzo);
}

fn volcar(lienzo: &[u32]) {
    // SEGURIDAD: el host valida (ptr, len) contra nuestra memoria lineal.
    unsafe {
        sys_render_frame(lienzo.as_ptr() as u32, (ANCHO * ALTO * 4) as u32);
    }
}

fn pintar_texto(lienzo: &mut [u32], texto: &[u8], x: usize, y: usize, color: u32) {
    let mut cx = x;
    for &c in texto {
        if cx + PASO > ANCHO {
            break;
        }
        pintar_glifo(lienzo, c, cx, y, color);
        cx += PASO;
    }
}

fn pintar_glifo(lienzo: &mut [u32], c: u8, x: usize, y: usize, color: u32) {
    let glifo = if (c as usize) < 128 {
        BASIC_LEGACY[c as usize]
    } else {
        BASIC_LEGACY[b'?' as usize]
    };
    for row in 0..8 {
        for col in 0..8 {
            if glifo[row] & (1 << col) != 0 {
                let px = x + col * 2;
                let py = y + row * 2;
                if px + 1 >= ANCHO || py + 1 >= ALTO {
                    continue;
                }
                lienzo[py * ANCHO + px] = color;
                lienzo[py * ANCHO + px + 1] = color;
                lienzo[(py + 1) * ANCHO + px] = color;
                lienzo[(py + 1) * ANCHO + px + 1] = color;
            }
        }
    }
}

fn hex_byte(b: u8) -> [u8; 2] {
    let n = |x: u8| if x < 10 { b'0' + x } else { b'a' + (x - 10) };
    [n(b >> 4), n(b & 0xf)]
}

fn dec(mut n: u32, buf: &mut [u8; 10]) -> &[u8] {
    if n == 0 {
        buf[0] = b'0';
        return &buf[..1];
    }
    let mut i = 10;
    while n > 0 {
        i -= 1;
        buf[i] = b'0' + (n % 10) as u8;
        n /= 10;
    }
    &buf[i..]
}
