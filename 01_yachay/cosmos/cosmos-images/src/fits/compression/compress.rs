use super::*;

pub struct CompressionParams {
    pub algorithm: CompressionAlgorithm,
    pub tile_width: usize,
    pub tile_height: usize,
    pub bits_per_pixel: i32,
}

impl CompressionParams {
    pub fn rice(tile_width: usize, tile_height: usize, bits_per_pixel: i32) -> Self {
        Self {
            algorithm: CompressionAlgorithm::Rice,
            tile_width,
            tile_height,
            bits_per_pixel,
        }
    }
}

pub fn compress_tile(data: &[u8], params: &CompressionParams) -> Result<Vec<u8>> {
    match params.algorithm {
        CompressionAlgorithm::Rice => compress_rice(data, params),
        CompressionAlgorithm::Gzip => compress_gzip(data),
        CompressionAlgorithm::HCompress => compress_hcompress(data, params),
        CompressionAlgorithm::Plio => compress_plio(data, params),
    }
}

pub(crate) fn compress_gzip(data: &[u8]) -> Result<Vec<u8>> {
    use flate2::write::GzEncoder;
    use flate2::Compression;
    use std::io::Write;

    let mut encoder = GzEncoder::new(Vec::new(), Compression::default());
    encoder
        .write_all(data)
        .map_err(|e| FitsError::InvalidFormat(format!("GZIP compression failed: {}", e)))?;
    encoder
        .finish()
        .map_err(|e| FitsError::InvalidFormat(format!("GZIP compression failed: {}", e)))
}

pub(crate) fn compress_rice(data: &[u8], params: &CompressionParams) -> Result<Vec<u8>> {
    match params.bits_per_pixel {
        8 => compress_rice_i8(data),
        16 => compress_rice_i16(data),
        32 => compress_rice_i32(data),
        _ => Err(FitsError::InvalidFormat(format!(
            "Unsupported BITPIX {} for Rice compression",
            params.bits_per_pixel
        ))),
    }
}

pub(crate) fn compress_rice_i8(data: &[u8]) -> Result<Vec<u8>> {
    let pixels: Vec<i8> = data.iter().map(|&b| b as i8).collect();
    i8::compress(&pixels, DEFAULT_RICE_BLOCK_SIZE)
}

pub(crate) fn compress_rice_i16(data: &[u8]) -> Result<Vec<u8>> {
    let pixels: Vec<i16> = data
        .chunks_exact(2)
        .map(|chunk| i16::from_be_bytes([chunk[0], chunk[1]]))
        .collect();
    i16::compress(&pixels, DEFAULT_RICE_BLOCK_SIZE)
}

pub(crate) fn compress_rice_i32(data: &[u8]) -> Result<Vec<u8>> {
    let pixels: Vec<i32> = data
        .chunks_exact(4)
        .map(|chunk| i32::from_be_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]))
        .collect();
    i32::compress(&pixels, DEFAULT_RICE_BLOCK_SIZE)
}

pub(crate) fn compress_plio(data: &[u8], params: &CompressionParams) -> Result<Vec<u8>> {
    let pixels = bytes_to_pixels(data, params.bits_per_pixel)?;
    plio_encode(&pixels)
}

pub(crate) fn bytes_to_pixels(data: &[u8], bits_per_pixel: i32) -> Result<Vec<i32>> {
    match bits_per_pixel {
        8 => Ok(data.iter().map(|&b| b as i32).collect()),
        16 => Ok(data
            .chunks_exact(2)
            .map(|c| i16::from_be_bytes([c[0], c[1]]) as i32)
            .collect()),
        32 => Ok(data
            .chunks_exact(4)
            .map(|c| i32::from_be_bytes([c[0], c[1], c[2], c[3]]))
            .collect()),
        _ => Err(FitsError::InvalidFormat(format!(
            "Unsupported BITPIX {} for PLIO",
            bits_per_pixel
        ))),
    }
}

pub(crate) fn plio_encode(pixels: &[i32]) -> Result<Vec<u8>> {
    let mut words: Vec<i16> = Vec::with_capacity(pixels.len());
    let mut pv: i32 = 0;
    let mut run_start: usize = 0;

    while run_start < pixels.len() {
        let current = pixels[run_start];
        let run_len = plio_count_run(pixels, run_start);

        if current == 0 && run_len > 0 {
            plio_emit_zeros(&mut words, run_len);
        } else {
            plio_emit_value_change(&mut words, &mut pv, current);
            if run_len > 1 {
                plio_emit_fill(&mut words, run_len);
            } else {
                plio_emit_fill(&mut words, 1);
            }
        }
        run_start += run_len;
    }

    let mut bytes = Vec::with_capacity(words.len() * 2);
    for w in words {
        bytes.extend_from_slice(&w.to_be_bytes());
    }
    Ok(bytes)
}

pub(crate) fn plio_count_run(pixels: &[i32], start: usize) -> usize {
    let val = pixels[start];
    let mut len = 1;
    while start + len < pixels.len() && pixels[start + len] == val && len < 4095 {
        len += 1;
    }
    len
}

pub(crate) fn plio_emit_zeros(words: &mut Vec<i16>, count: usize) {
    let count = count.min(4095);
    words.push(count as i16);
}

pub(crate) fn plio_emit_fill(words: &mut Vec<i16>, count: usize) {
    let count = count.min(4095);
    words.push(0x1000 | count as i16);
}

pub(crate) fn plio_emit_value_change(words: &mut Vec<i16>, pv: &mut i32, new_val: i32) {
    let diff = new_val - *pv;
    if (0..4096).contains(&diff) {
        words.push(0x3000 | diff as i16);
    } else if diff < 0 && diff > -4096 {
        words.push(0x4000 | (-diff) as i16);
    } else {
        let low = (new_val & 0x0FFF) as i16;
        let high = ((new_val >> 12) & 0xFFFF) as i16;
        words.push(0x2000 | low);
        words.push(high);
    }
    *pv = new_val;
}

pub(crate) fn compress_hcompress(data: &[u8], params: &CompressionParams) -> Result<Vec<u8>> {
    let pixels = bytes_to_pixels(data, params.bits_per_pixel)?;
    hcomp_encode(&pixels, params.tile_width, params.tile_height)
}

pub(crate) fn hcomp_encode(pixels: &[i32], nx: usize, ny: usize) -> Result<Vec<u8>> {
    let mut a: Vec<i64> = pixels.iter().map(|&p| p as i64).collect();
    hcomp_htrans(&mut a, nx, ny);

    let scale = 1i32;
    let sum = a[0];
    let nbitplanes = hcomp_count_bitplanes(&a, nx, ny);

    let mut output = Vec::with_capacity(pixels.len());
    output.extend_from_slice(&HCOMP_MAGIC);
    output.extend_from_slice(&(nx as i32).to_be_bytes());
    output.extend_from_slice(&(ny as i32).to_be_bytes());
    output.extend_from_slice(&scale.to_be_bytes());
    output.extend_from_slice(&sum.to_be_bytes());
    output.extend_from_slice(&nbitplanes);

    let mut writer = HCompWriter::new();
    hcomp_encode_bitplanes(&mut writer, &a, nx, ny, &nbitplanes)?;
    output.extend_from_slice(&writer.finish());

    Ok(output)
}

pub(crate) fn hcomp_htrans(a: &mut [i64], nx: usize, ny: usize) {
    let nmax = nx.max(ny);
    let log2n = ilog2_ceil(nmax);

    let mut workspace = vec![0i64; nx * ny];

    let mut nxtop = nx;
    let mut nytop = ny;

    for _ in 0..log2n {
        if nxtop <= 1 && nytop <= 1 {
            break;
        }
        hcomp_htrans_step(a, nx, nxtop, nytop, &mut workspace);
        nxtop = nxtop.div_ceil(2);
        nytop = nytop.div_ceil(2);
    }
}

pub(crate) fn hcomp_htrans_step(a: &mut [i64], nx: usize, nxtop: usize, nytop: usize, workspace: &mut [i64]) {
    let nx2 = nxtop.div_ceil(2);
    let ny2 = nytop.div_ceil(2);
    let tmp = &mut workspace[..nxtop * nytop];

    for j in 0..ny2 {
        for i in 0..nx2 {
            let j2 = j * 2;
            let i2 = i * 2;

            let a00 = a[j2 * nx + i2];
            let a01 = if i2 + 1 < nxtop {
                a[j2 * nx + i2 + 1]
            } else {
                a00
            };
            let a10 = if j2 + 1 < nytop {
                a[(j2 + 1) * nx + i2]
            } else {
                a00
            };
            let a11 = if i2 + 1 < nxtop && j2 + 1 < nytop {
                a[(j2 + 1) * nx + i2 + 1]
            } else if i2 + 1 < nxtop {
                a01
            } else if j2 + 1 < nytop {
                a10
            } else {
                a00
            };

            let h0 = a00 + a01 + a10 + a11;
            let hx = a00 + a01 - a10 - a11;
            let hy = a00 - a01 + a10 - a11;
            let hc = a00 - a01 - a10 + a11;

            tmp[j * nxtop + i] = h0;
            if i2 + 1 < nxtop {
                tmp[j * nxtop + nx2 + i] = hx;
            }
            if j2 + 1 < nytop {
                tmp[(ny2 + j) * nxtop + i] = hy;
            }
            if i2 + 1 < nxtop && j2 + 1 < nytop {
                tmp[(ny2 + j) * nxtop + nx2 + i] = hc;
            }
        }
    }

    for j in 0..nytop {
        for i in 0..nxtop {
            a[j * nx + i] = tmp[j * nxtop + i];
        }
    }
}

pub(crate) fn hcomp_count_bitplanes(a: &[i64], nx: usize, ny: usize) -> [u8; 3] {
    let nx2 = nx.div_ceil(2);
    let ny2 = ny.div_ceil(2);

    let mut max0: i64 = 0;
    let mut max1: i64 = 0;
    let mut max2: i64 = 0;

    for j in 0..ny2 {
        for i in 0..nx2 {
            max0 = max0.max(a[j * nx + i].abs());
        }
    }
    for j in 0..ny2 {
        for i in nx2..nx {
            max1 = max1.max(a[j * nx + i].abs());
        }
    }
    for j in ny2..ny {
        for i in 0..nx2 {
            max1 = max1.max(a[j * nx + i].abs());
        }
    }
    for j in ny2..ny {
        for i in nx2..nx {
            max2 = max2.max(a[j * nx + i].abs());
        }
    }

    [count_bits(max0), count_bits(max1), count_bits(max2)]
}

pub(crate) fn count_bits(v: i64) -> u8 {
    if v == 0 {
        0
    } else {
        (64 - v.leading_zeros()) as u8
    }
}

pub(crate) struct HCompWriter {
    data: Vec<u8>,
    buffer: u8,
    bits_used: u8,
}

impl HCompWriter {
    pub(crate) fn new() -> Self {
        Self {
            data: Vec::new(),
            buffer: 0,
            bits_used: 0,
        }
    }

    pub(crate) fn write_nybble(&mut self, nyb: u8) {
        self.buffer = (self.buffer << 4) | (nyb & 0x0F);
        self.bits_used += 4;
        if self.bits_used >= 8 {
            self.data.push(self.buffer);
            self.buffer = 0;
            self.bits_used = 0;
        }
    }

    pub(crate) fn write_bit(&mut self, bit: u8) {
        self.buffer = (self.buffer << 1) | (bit & 1);
        self.bits_used += 1;
        if self.bits_used >= 8 {
            self.data.push(self.buffer);
            self.buffer = 0;
            self.bits_used = 0;
        }
    }

    pub(crate) fn finish(mut self) -> Vec<u8> {
        if self.bits_used > 0 {
            self.data.push(self.buffer << (8 - self.bits_used));
        }
        self.data
    }
}

pub(crate) fn hcomp_encode_bitplanes(
    writer: &mut HCompWriter,
    a: &[i64],
    nx: usize,
    ny: usize,
    nbitplanes: &[u8; 3],
) -> Result<()> {
    let nx2 = nx.div_ceil(2);
    let ny2 = ny.div_ceil(2);
    let max_bits = *nbitplanes.iter().max().unwrap_or(&0) as usize;

    for bit in (0..max_bits).rev() {
        let plane_bit = bit as u8;
        if plane_bit < nbitplanes[0] {
            hcomp_encode_quadrant(
                writer,
                a,
                nx,
                &QuadrantBounds::new(0, ny2, 0, nx2),
                plane_bit,
            );
        }
        if plane_bit < nbitplanes[1] {
            hcomp_encode_quadrant(
                writer,
                a,
                nx,
                &QuadrantBounds::new(0, ny2, nx2, nx),
                plane_bit,
            );
            hcomp_encode_quadrant(
                writer,
                a,
                nx,
                &QuadrantBounds::new(ny2, ny, 0, nx2),
                plane_bit,
            );
        }
        if plane_bit < nbitplanes[2] {
            hcomp_encode_quadrant(
                writer,
                a,
                nx,
                &QuadrantBounds::new(ny2, ny, nx2, nx),
                plane_bit,
            );
        }
    }
    Ok(())
}

pub(crate) fn hcomp_encode_quadrant(
    writer: &mut HCompWriter,
    a: &[i64],
    nx: usize,
    bounds: &QuadrantBounds,
    bit: u8,
) {
    let mut any_set = false;
    for y in bounds.y0..bounds.y1 {
        for x in bounds.x0..bounds.x1 {
            if (a[y * nx + x].abs() >> bit) & 1 != 0 {
                any_set = true;
                break;
            }
        }
        if any_set {
            break;
        }
    }

    if !any_set {
        writer.write_nybble(0);
        return;
    }

    writer.write_nybble(0xF);
    for y in bounds.y0..bounds.y1 {
        for x in bounds.x0..bounds.x1 {
            let b = ((a[y * nx + x].abs() >> bit) & 1) as u8;
            writer.write_bit(b);
        }
    }
}
