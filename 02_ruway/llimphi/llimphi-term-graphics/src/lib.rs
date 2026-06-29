//! `llimphi-term-graphics` — decodificador agnóstico de protocolos de gráficos
//! de terminal.
//!
//! Dos protocolos cubiertos:
//! - **kitty graphics protocol** — secuencias APC `\e_G<control>;<base64>\e\\`.
//!   Soporta transmisión chunked (`m=1`), formatos `f=100` (PNG/JPEG/… vía el
//!   crate `image`), `f=24` (RGB crudo) y `f=32` (RGBA crudo), y compresión
//!   `o=z` (zlib). Acciones `t`/`T` (transmitir [y mostrar]), `q` (query),
//!   `d` (delete).
//! - **sixel** — secuencias DCS `\eP<params>q<datos>\e\\`. Decodificador a mano
//!   (paleta, selección de color `#`, RLE `!`, CR `$`, LF `-`, atributos
//!   raster `"`).
//!
//! El corazón es [`GraphicsScanner`]: un autómata streaming al que se le
//! alimentan los chunks de bytes del PTY. Deja pasar **intacto** todo el texto
//! y los escapes ANSI normales (para que el emulador vt100 los procese) y sólo
//! **secuestra** las secuencias gráficas completas, que devuelve decodificadas
//! como [`GraphicsCommand`]. No tiene estado de UI ni dependencias de render:
//! produce píxeles RGBA8, el caller los sube a una textura / `peniko::Image`.
//!
//! Robusto a fronteras de chunk: una secuencia partida entre dos `feed()`
//! queda buffereada y se completa en la llamada siguiente.

#![forbid(unsafe_code)]

mod kitty;
mod sixel;

pub use kitty::KittyError;

/// Imagen decodificada: RGBA8 sin premultiplicar, fila-mayor, sin padding.
/// `rgba.len() == width * height * 4`.
#[derive(Clone, PartialEq, Eq)]
pub struct DecodedImage {
    pub width: u32,
    pub height: u32,
    pub rgba: Vec<u8>,
}

impl std::fmt::Debug for DecodedImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DecodedImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("rgba_len", &self.rgba.len())
            .finish()
    }
}

/// Protocolo de origen de un comando gráfico.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Protocol {
    Kitty,
    Sixel,
}

/// Un comando gráfico completo extraído del stream del PTY.
#[derive(Debug, Clone)]
pub enum GraphicsCommand {
    /// Transmitir (y normalmente mostrar) una imagen en la posición actual del
    /// cursor. `cols`/`rows` son las celdas pedidas por el protocolo (kitty
    /// `c=`/`r=`); `0` = el caller deriva el tamaño de los píxeles.
    Image {
        image: DecodedImage,
        cols: u16,
        rows: u16,
        /// id de imagen kitty (para placement/delete posterior); sixel = 0.
        id: u32,
        protocol: Protocol,
    },
    /// kitty `a=d`: borrar placements. `id == None` = borrar todo.
    Delete { id: Option<u32> },
    /// Una query de capacidad (kitty `a=q`) que el emulador debe **responder**
    /// escribiendo `response` de vuelta por el stdin del PTY.
    Query { response: Vec<u8> },
}

const ESC: u8 = 0x1b;
const BEL: u8 = 0x07;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum State {
    /// Texto normal; copiando a passthrough.
    Normal,
    /// Vimos ESC, esperando el byte que decide (no lo copiamos todavía).
    Esc,
    /// Dentro de APC (`\e_`), acumulando en `seq` hasta ST/BEL.
    Apc,
    /// Dentro de APC y vimos ESC (esperando `\` del ST).
    ApcEsc,
    /// Dentro de DCS (`\eP`), acumulando en `seq` hasta ST/BEL.
    Dcs,
    /// Dentro de DCS y vimos ESC.
    DcsEsc,
}

/// Autómata streaming que separa el texto/ANSI normal de las secuencias
/// gráficas. Ver el módulo para el contrato completo.
pub struct GraphicsScanner {
    state: State,
    /// Cuerpo de la secuencia en curso (sin el ESC inicial ni el terminador).
    seq: Vec<u8>,
    /// Acumulador de la transmisión chunked de kitty (`m=1`).
    kitty: kitty::KittyAssembler,
    /// Tope de bytes para una sola secuencia — defensa contra un stream malo
    /// que nunca cierra el APC/DCS (no crecemos sin límite).
    max_seq: usize,
}

impl Default for GraphicsScanner {
    fn default() -> Self {
        Self::new()
    }
}

impl GraphicsScanner {
    pub fn new() -> Self {
        Self {
            state: State::Normal,
            seq: Vec::new(),
            kitty: kitty::KittyAssembler::default(),
            // 64 MiB: una imagen base64 grande cabe; un stream roto no nos cuelga.
            max_seq: 64 * 1024 * 1024,
        }
    }

    /// Consume `input`: escribe los bytes no-gráficos en `passthrough` (para el
    /// vt100) y devuelve los comandos gráficos completados en este chunk.
    pub fn feed(&mut self, input: &[u8], passthrough: &mut Vec<u8>) -> Vec<GraphicsCommand> {
        let mut out = Vec::new();
        for &b in input {
            match self.state {
                State::Normal => {
                    if b == ESC {
                        self.state = State::Esc;
                    } else {
                        passthrough.push(b);
                    }
                }
                State::Esc => match b {
                    b'_' => {
                        self.seq.clear();
                        self.state = State::Apc;
                    }
                    b'P' => {
                        self.seq.clear();
                        self.state = State::Dcs;
                    }
                    ESC => {
                        // ESC ESC: el primero no era nuestro → a passthrough;
                        // seguimos evaluando este nuevo ESC.
                        passthrough.push(ESC);
                    }
                    _ => {
                        // Cualquier otra secuencia (CSI `[`, OSC `]`, charset…)
                        // no nos interesa: devolvemos el ESC + este byte y el
                        // resto fluye como texto normal al vt100.
                        passthrough.push(ESC);
                        passthrough.push(b);
                        self.state = State::Normal;
                    }
                },
                State::Apc => self.accumulate(b, &mut out, false),
                State::ApcEsc => self.accumulate_esc(b, &mut out, false),
                State::Dcs => self.accumulate(b, &mut out, true),
                State::DcsEsc => self.accumulate_esc(b, &mut out, true),
            }
        }
        out
    }

    /// Byte dentro de un cuerpo APC/DCS (no estábamos tras un ESC).
    fn accumulate(&mut self, b: u8, out: &mut Vec<GraphicsCommand>, is_dcs: bool) {
        if b == ESC {
            self.state = if is_dcs { State::DcsEsc } else { State::ApcEsc };
        } else if b == BEL {
            // BEL también cierra (algunas terminales lo aceptan para APC).
            self.finish(out, is_dcs);
        } else {
            if self.seq.len() < self.max_seq {
                self.seq.push(b);
            }
            // Si rebalsa el tope, seguimos consumiendo bytes (descartándolos)
            // hasta el terminador, para no resincronizar mal.
        }
    }

    /// Byte dentro de un cuerpo APC/DCS justo después de un ESC.
    fn accumulate_esc(&mut self, b: u8, out: &mut Vec<GraphicsCommand>, is_dcs: bool) {
        if b == b'\\' {
            // ST: secuencia completa.
            self.finish(out, is_dcs);
        } else if b == ESC {
            // ESC ESC dentro del cuerpo: el primero es literal.
            if self.seq.len() < self.max_seq {
                self.seq.push(ESC);
            }
            // seguimos en *Esc esperando el resolutor.
        } else {
            // El ESC era literal y este byte también.
            if self.seq.len() + 2 <= self.max_seq {
                self.seq.push(ESC);
                self.seq.push(b);
            }
            self.state = if is_dcs { State::Dcs } else { State::Apc };
        }
    }

    /// Cierra la secuencia acumulada: la decodifica y resetea a Normal.
    fn finish(&mut self, out: &mut Vec<GraphicsCommand>, is_dcs: bool) {
        let seq = std::mem::take(&mut self.seq);
        self.state = State::Normal;
        if is_dcs {
            if let Some(img) = sixel::decode(&seq) {
                out.push(GraphicsCommand::Image {
                    image: img,
                    cols: 0,
                    rows: 0,
                    id: 0,
                    protocol: Protocol::Sixel,
                });
            }
        } else if let Some(cmd) = self.kitty.feed_apc(&seq) {
            out.push(cmd);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// El texto plano pasa intacto y no genera comandos.
    #[test]
    fn texto_normal_pasa_intacto() {
        let mut sc = GraphicsScanner::new();
        let mut pt = Vec::new();
        let cmds = sc.feed(b"hola mundo\n", &mut pt);
        assert!(cmds.is_empty());
        assert_eq!(pt, b"hola mundo\n");
    }

    /// Una secuencia CSI (color SGR) NO se secuestra: fluye al passthrough.
    #[test]
    fn csi_sgr_no_se_secuestra() {
        let mut sc = GraphicsScanner::new();
        let mut pt = Vec::new();
        let input = b"\x1b[31mrojo\x1b[0m";
        let cmds = sc.feed(input, &mut pt);
        assert!(cmds.is_empty());
        assert_eq!(pt, input);
    }

    /// kitty RGBA crudo 1x1: APC con f=32, s=1, v=1, payload = 4 bytes base64.
    #[test]
    fn kitty_rgba_1x1() {
        use base64::Engine;
        let px = [10u8, 20, 30, 255];
        let b64 = base64::engine::general_purpose::STANDARD.encode(px);
        let seq = format!("\x1b_Gf=32,s=1,v=1,a=T;{b64}\x1b\\");
        let mut sc = GraphicsScanner::new();
        let mut pt = Vec::new();
        let cmds = sc.feed(seq.as_bytes(), &mut pt);
        assert!(pt.is_empty(), "no debería pasar nada al vt100");
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            GraphicsCommand::Image { image, protocol, .. } => {
                assert_eq!(*protocol, Protocol::Kitty);
                assert_eq!((image.width, image.height), (1, 1));
                assert_eq!(image.rgba, vec![10, 20, 30, 255]);
            }
            other => panic!("se esperaba Image, vino {other:?}"),
        }
    }

    /// La secuencia partida entre dos `feed()` se completa igual.
    #[test]
    fn secuencia_partida_entre_chunks() {
        use base64::Engine;
        let px = [1u8, 2, 3, 4];
        let b64 = base64::engine::general_purpose::STANDARD.encode(px);
        let seq = format!("\x1b_Gf=32,s=1,v=1;{b64}\x1b\\");
        let bytes = seq.as_bytes();
        let mid = bytes.len() / 2;
        let mut sc = GraphicsScanner::new();
        let mut pt = Vec::new();
        let mut cmds = sc.feed(&bytes[..mid], &mut pt);
        cmds.extend(sc.feed(&bytes[mid..], &mut pt));
        assert!(pt.is_empty());
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            GraphicsCommand::Image { image, .. } => {
                assert_eq!(image.rgba, vec![1, 2, 3, 4]);
            }
            other => panic!("se esperaba Image, vino {other:?}"),
        }
    }

    /// Texto antes y después de la imagen sobrevive en el passthrough.
    #[test]
    fn texto_rodeando_imagen() {
        use base64::Engine;
        let b64 = base64::engine::general_purpose::STANDARD.encode([9u8, 9, 9, 9]);
        let seq = format!("antes\x1b_Gf=32,s=1,v=1;{b64}\x1b\\después");
        let mut sc = GraphicsScanner::new();
        let mut pt = Vec::new();
        let cmds = sc.feed(seq.as_bytes(), &mut pt);
        assert_eq!(cmds.len(), 1);
        assert_eq!(pt, "antesdespués".as_bytes());
    }

    /// kitty f=100 (PNG embebido) — el camino real de chafa/icat. Construimos
    /// un PNG 2×2 con el crate `image`, lo metemos en un APC y verificamos que
    /// vuelve a salir como RGBA 2×2.
    #[test]
    fn kitty_png_embebido() {
        use base64::Engine;
        // PNG 2×2: rojo, verde / azul, blanco.
        let mut img = image::RgbaImage::new(2, 2);
        img.put_pixel(0, 0, image::Rgba([255, 0, 0, 255]));
        img.put_pixel(1, 0, image::Rgba([0, 255, 0, 255]));
        img.put_pixel(0, 1, image::Rgba([0, 0, 255, 255]));
        img.put_pixel(1, 1, image::Rgba([255, 255, 255, 255]));
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(png.into_inner());
        let seq = format!("\x1b_Gf=100,a=T,c=4,r=2;{b64}\x1b\\");

        let mut sc = GraphicsScanner::new();
        let mut pt = Vec::new();
        let cmds = sc.feed(seq.as_bytes(), &mut pt);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            GraphicsCommand::Image { image, cols, rows, .. } => {
                assert_eq!((image.width, image.height), (2, 2));
                assert_eq!((*cols, *rows), (4, 2));
                assert_eq!(&image.rgba[0..4], &[255, 0, 0, 255]); // rojo
            }
            other => panic!("se esperaba Image, vino {other:?}"),
        }
    }

    /// Transmisión chunked: control + payload en varios APC con `m=1` y el
    /// último con `m=0`. El payload base64 se concatena y decodifica una vez.
    #[test]
    fn kitty_chunked() {
        use base64::Engine;
        // 2×1 RGBA crudo = 8 bytes.
        let raw = [10u8, 11, 12, 13, 20, 21, 22, 23];
        let b64 = base64::engine::general_purpose::STANDARD.encode(raw);
        let (a, b) = b64.split_at(b64.len() / 2);
        // primer chunk: control completo + m=1; segundo: sólo m=0 + resto.
        let seq = format!("\x1b_Gf=32,s=2,v=1,m=1;{a}\x1b\\\x1b_Gm=0;{b}\x1b\\");

        let mut sc = GraphicsScanner::new();
        let mut pt = Vec::new();
        let cmds = sc.feed(seq.as_bytes(), &mut pt);
        assert_eq!(cmds.len(), 1, "un solo Image al cerrar el último chunk");
        match &cmds[0] {
            GraphicsCommand::Image { image, .. } => {
                assert_eq!((image.width, image.height), (2, 1));
                assert_eq!(image.rgba, raw.to_vec());
            }
            other => panic!("se esperaba Image, vino {other:?}"),
        }
    }

    /// Compresión zlib (o=z) sobre RGBA crudo.
    #[test]
    fn kitty_zlib() {
        use base64::Engine;
        use std::io::Write;
        let raw = [1u8, 2, 3, 4, 5, 6, 7, 8];
        let mut enc = flate2::write::ZlibEncoder::new(Vec::new(), flate2::Compression::default());
        enc.write_all(&raw).unwrap();
        let compressed = enc.finish().unwrap();
        let b64 = base64::engine::general_purpose::STANDARD.encode(compressed);
        let seq = format!("\x1b_Gf=32,s=2,v=1,o=z;{b64}\x1b\\");

        let mut sc = GraphicsScanner::new();
        let mut pt = Vec::new();
        let cmds = sc.feed(seq.as_bytes(), &mut pt);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            GraphicsCommand::Image { image, .. } => {
                assert_eq!(image.rgba, raw.to_vec());
            }
            other => panic!("se esperaba Image, vino {other:?}"),
        }
    }

    /// Round-trip contra el encoder real `chafa`, en sus dos formatos. Genera
    /// un PNG, lo pasa por `chafa -f {sixel,kitty}` y verifica que el scanner
    /// recupera exactamente una imagen coherente — pese a la `CSI` de mostrar/
    /// ocultar cursor que chafa intercala (debe ir a passthrough) y a que el
    /// formato kitty viene en chunks. Se salta si chafa no está instalado.
    #[test]
    fn chafa_round_trip_real() {
        use std::io::Write;
        use std::process::{Command, Stdio};

        // PNG 16×16 con patrón de dos colores.
        let mut img = image::RgbaImage::new(16, 16);
        for y in 0..16u32 {
            for x in 0..16u32 {
                let c = if (x / 4 + y / 4) % 2 == 0 {
                    image::Rgba([200, 30, 30, 255])
                } else {
                    image::Rgba([30, 30, 200, 255])
                };
                img.put_pixel(x, y, c);
            }
        }
        let mut png = std::io::Cursor::new(Vec::new());
        image::DynamicImage::ImageRgba8(img)
            .write_to(&mut png, image::ImageFormat::Png)
            .unwrap();
        let png = png.into_inner();

        for (fmt, want) in [("sixel", Protocol::Sixel), ("kitty", Protocol::Kitty)] {
            let mut child = match Command::new("chafa")
                .args(["-f", fmt, "--size", "20x10", "-"])
                .stdin(Stdio::piped())
                .stdout(Stdio::piped())
                .stderr(Stdio::null())
                .spawn()
            {
                Ok(c) => c,
                Err(_) => {
                    eprintln!("chafa no instalado — test saltado");
                    return;
                }
            };
            child.stdin.take().unwrap().write_all(&png).unwrap();
            let out = child.wait_with_output().unwrap();
            assert!(out.status.success(), "chafa -f {fmt} falló");

            // Alimentamos en dos mitades para ejercitar también la frontera de
            // chunk del scanner sobre datos reales.
            let mut sc = GraphicsScanner::new();
            let mut pt = Vec::new();
            let mid = out.stdout.len() / 2;
            let mut cmds = sc.feed(&out.stdout[..mid], &mut pt);
            cmds.extend(sc.feed(&out.stdout[mid..], &mut pt));
            let imgs: Vec<_> = cmds
                .iter()
                .filter_map(|c| match c {
                    GraphicsCommand::Image { image, protocol, .. } => Some((image, *protocol)),
                    _ => None,
                })
                .collect();
            assert_eq!(imgs.len(), 1, "fmt={fmt}: se esperaba 1 imagen");
            let (image, protocol) = imgs[0];
            assert_eq!(protocol, want, "fmt={fmt}: protocolo");
            assert!(image.width > 0 && image.height > 0, "fmt={fmt}: dims nulas");
            assert_eq!(
                image.rgba.len(),
                (image.width as usize) * (image.height as usize) * 4,
                "fmt={fmt}: largo del buffer RGBA"
            );
            // El passthrough debe contener la CSI de cursor (no la tragamos).
            assert!(!pt.is_empty(), "fmt={fmt}: la CSI debió ir a passthrough");
        }
    }

    /// Query de capacidad kitty → comando Query con respuesta OK.
    #[test]
    fn kitty_query_responde_ok() {
        let mut sc = GraphicsScanner::new();
        let mut pt = Vec::new();
        let cmds = sc.feed(b"\x1b_Gi=31,s=1,v=1,a=q;AAAA\x1b\\", &mut pt);
        assert_eq!(cmds.len(), 1);
        match &cmds[0] {
            GraphicsCommand::Query { response } => {
                let s = String::from_utf8_lossy(response);
                assert!(s.contains("i=31"), "respuesta: {s}");
                assert!(s.contains("OK"), "respuesta: {s}");
            }
            other => panic!("se esperaba Query, vino {other:?}"),
        }
    }
}
