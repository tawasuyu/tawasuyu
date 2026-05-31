use super::*;

pub struct DecompressionParams {
    pub algorithm: CompressionAlgorithm,
    pub quantization_level: Option<i64>,
    pub tile_dimensions: (usize, usize),
    pub bits_per_pixel: i32,
}

impl DecompressionParams {
    pub fn new(
        algorithm: CompressionAlgorithm,
        quantization_level: Option<i64>,
        tile_dimensions: (usize, usize),
        bits_per_pixel: i32,
    ) -> Self {
        Self {
            algorithm,
            quantization_level,
            tile_dimensions,
            bits_per_pixel,
        }
    }
}

pub fn decompress_tile(compressed_data: &[u8], params: &DecompressionParams) -> Result<Vec<u8>> {
    match params.algorithm {
        CompressionAlgorithm::Gzip => decompress_gzip(compressed_data, params),
        CompressionAlgorithm::Rice => decompress_rice(compressed_data, params),
        CompressionAlgorithm::HCompress => decompress_hcompress(compressed_data, params),
        CompressionAlgorithm::Plio => decompress_plio(compressed_data, params),
    }
}

pub(crate) fn decompress_gzip(compressed_data: &[u8], params: &DecompressionParams) -> Result<Vec<u8>> {
    use flate2::read::GzDecoder;
    use std::io::Read;

    let expected_size = params.tile_dimensions.0
        * params.tile_dimensions.1
        * (params.bits_per_pixel.abs() / 8) as usize;

    let mut decoder = GzDecoder::new(compressed_data);
    let mut decompressed = Vec::with_capacity(expected_size);

    decoder
        .read_to_end(&mut decompressed)
        .map_err(|e| FitsError::InvalidFormat(format!("GZIP decompression failed: {}", e)))?;

    if decompressed.len() != expected_size {
        return Err(FitsError::InvalidFormat(format!(
            "Decompressed size {} doesn't match expected {}",
            decompressed.len(),
            expected_size
        )));
    }

    Ok(decompressed)
}

pub(crate) fn decompress_rice(compressed_data: &[u8], params: &DecompressionParams) -> Result<Vec<u8>> {
    let pixel_count = params.tile_dimensions.0 * params.tile_dimensions.1;

    match params.bits_per_pixel {
        8 => decompress_rice_i8(compressed_data, pixel_count),
        16 => decompress_rice_i16(compressed_data, pixel_count),
        32 => decompress_rice_i32(compressed_data, pixel_count),
        _ => Err(FitsError::InvalidFormat(format!(
            "Unsupported BITPIX {} for Rice compression",
            params.bits_per_pixel
        ))),
    }
}

pub(crate) fn decompress_rice_i8(compressed_data: &[u8], pixel_count: usize) -> Result<Vec<u8>> {
    let pixels: Vec<i8> = i8::decompress(compressed_data, pixel_count, DEFAULT_RICE_BLOCK_SIZE)?;
    Ok(pixels.into_iter().map(|p| p as u8).collect())
}

pub(crate) fn decompress_rice_i16(compressed_data: &[u8], pixel_count: usize) -> Result<Vec<u8>> {
    let pixels: Vec<i16> = i16::decompress(compressed_data, pixel_count, DEFAULT_RICE_BLOCK_SIZE)?;
    let mut bytes = vec![0u8; pixel_count * 2];
    for (i, pixel) in pixels.iter().enumerate() {
        let be = pixel.to_be_bytes();
        bytes[i * 2] = be[0];
        bytes[i * 2 + 1] = be[1];
    }
    Ok(bytes)
}

pub(crate) fn decompress_rice_i32(compressed_data: &[u8], pixel_count: usize) -> Result<Vec<u8>> {
    let pixels: Vec<i32> = i32::decompress(compressed_data, pixel_count, DEFAULT_RICE_BLOCK_SIZE)?;
    let mut bytes = vec![0u8; pixel_count * 4];
    for (i, pixel) in pixels.iter().enumerate() {
        let be = pixel.to_be_bytes();
        bytes[i * 4] = be[0];
        bytes[i * 4 + 1] = be[1];
        bytes[i * 4 + 2] = be[2];
        bytes[i * 4 + 3] = be[3];
    }
    Ok(bytes)
}

pub(crate) fn decompress_plio(compressed_data: &[u8], params: &DecompressionParams) -> Result<Vec<u8>> {
    let pixel_count = params.tile_dimensions.0 * params.tile_dimensions.1;
    let pixels = plio_decode(compressed_data, pixel_count)?;
    pixels_to_bytes(&pixels, params.bits_per_pixel)
}

pub(crate) fn plio_decode(data: &[u8], pixel_count: usize) -> Result<Vec<i32>> {
    if data.len() < 2 {
        return Ok(vec![0i32; pixel_count]);
    }

    let words: Vec<i16> = data
        .chunks_exact(2)
        .map(|c| i16::from_be_bytes([c[0], c[1]]))
        .collect();
    let mut output = vec![0i32; pixel_count];
    let mut pv: i32 = 0;
    let mut op: usize = 0;
    let mut wp: usize = 0;

    while wp < words.len() && op < pixel_count {
        let (new_op, new_pv, new_wp) = plio_process_word(&words, wp, &mut output, op, pv)?;
        op = new_op;
        pv = new_pv;
        wp = new_wp;
    }

    Ok(output)
}

pub(crate) fn plio_process_word(
    words: &[i16],
    wp: usize,
    output: &mut [i32],
    op: usize,
    pv: i32,
) -> Result<(usize, i32, usize)> {
    let word = words[wp] as u16;
    let opcode = (word >> 12) as u8;
    let data = (word & 0x0FFF) as i32;

    match opcode {
        0 => Ok((plio_fill_zeros(output, op, data as usize), pv, wp + 1)),
        1 | 5 | 6 => Ok((plio_fill_value(output, op, data as usize, pv), pv, wp + 1)),
        2 => plio_set_value(words, wp, data),
        3 => Ok((op, pv + data, wp + 1)),
        4 => Ok((op, pv - data, wp + 1)),
        7 => Ok((plio_output_one(output, op, pv + data), pv + data, wp + 1)),
        8 => Ok((plio_output_one(output, op, pv - data), pv - data, wp + 1)),
        _ => Err(FitsError::InvalidFormat(format!(
            "Unknown PLIO opcode: {}",
            opcode
        ))),
    }
}

pub(crate) fn plio_fill_zeros(output: &mut [i32], op: usize, count: usize) -> usize {
    let end = (op + count).min(output.len());
    output[op..end].fill(0);
    end
}

pub(crate) fn plio_fill_value(output: &mut [i32], op: usize, count: usize, value: i32) -> usize {
    let end = (op + count).min(output.len());
    output[op..end].fill(value);
    end
}

pub(crate) fn plio_set_value(words: &[i16], wp: usize, data: i32) -> Result<(usize, i32, usize)> {
    if wp + 1 >= words.len() {
        return Err(FitsError::InvalidFormat("PLIO: truncated set-value".into()));
    }
    let high = (words[wp + 1] as u16) as i32;
    let new_pv = (high << 12) | data;
    Ok((0, new_pv, wp + 2))
}

pub(crate) fn plio_output_one(output: &mut [i32], op: usize, value: i32) -> usize {
    if op < output.len() {
        output[op] = value;
        op + 1
    } else {
        op
    }
}

pub(crate) fn pixels_to_bytes(pixels: &[i32], bits_per_pixel: i32) -> Result<Vec<u8>> {
    match bits_per_pixel {
        8 => Ok(pixels.iter().map(|&p| p as u8).collect()),
        16 => {
            let mut bytes = vec![0u8; pixels.len() * 2];
            for (i, &p) in pixels.iter().enumerate() {
                let be = (p as i16).to_be_bytes();
                bytes[i * 2..i * 2 + 2].copy_from_slice(&be);
            }
            Ok(bytes)
        }
        32 => {
            let mut bytes = vec![0u8; pixels.len() * 4];
            for (i, &p) in pixels.iter().enumerate() {
                let be = p.to_be_bytes();
                bytes[i * 4..i * 4 + 4].copy_from_slice(&be);
            }
            Ok(bytes)
        }
        _ => Err(FitsError::InvalidFormat(format!(
            "Unsupported BITPIX {} for PLIO",
            bits_per_pixel
        ))),
    }
}

pub(crate) fn decompress_hcompress(compressed_data: &[u8], params: &DecompressionParams) -> Result<Vec<u8>> {
    let (nx, ny) = params.tile_dimensions;
    let pixels = hcomp_decode(compressed_data, nx, ny)?;
    pixels_to_bytes(&pixels, params.bits_per_pixel)
}


pub(crate) fn hcomp_decode(data: &[u8], nx: usize, ny: usize) -> Result<Vec<i32>> {
    if data.len() < 14 {
        return Err(FitsError::InvalidFormat("HCompress: data too short".into()));
    }
    if data[0..2] != HCOMP_MAGIC {
        return Err(FitsError::InvalidFormat("HCompress: invalid magic".into()));
    }

    let mut reader = HCompReader::new(&data[2..]);
    let file_nx = reader.read_i32()? as usize;
    let file_ny = reader.read_i32()? as usize;
    let scale = reader.read_i32()?;

    if file_nx != nx || file_ny != ny {
        return Err(FitsError::InvalidFormat(format!(
            "HCompress: dimension mismatch: file={}x{} expected={}x{}",
            file_nx, file_ny, nx, ny
        )));
    }

    let mut output = vec![0i64; nx * ny];
    hcomp_decode_stream(&mut reader, &mut output, nx, ny, scale)?;
    Ok(output.into_iter().map(|v| v as i32).collect())
}

pub(crate) struct HCompReader<'a> {
    data: &'a [u8],
    pos: usize,
    bit_buffer: u32,
    bits_in_buffer: u8,
}

impl<'a> HCompReader<'a> {
    pub(crate) fn new(data: &'a [u8]) -> Self {
        Self {
            data,
            pos: 0,
            bit_buffer: 0,
            bits_in_buffer: 0,
        }
    }

    pub(crate) fn read_i32(&mut self) -> Result<i32> {
        if self.pos + 4 > self.data.len() {
            return Err(FitsError::InvalidFormat("HCompress: unexpected EOF".into()));
        }
        let val = i32::from_be_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
        ]);
        self.pos += 4;
        Ok(val)
    }

    pub(crate) fn read_i64(&mut self) -> Result<i64> {
        if self.pos + 8 > self.data.len() {
            return Err(FitsError::InvalidFormat("HCompress: unexpected EOF".into()));
        }
        let val = i64::from_be_bytes([
            self.data[self.pos],
            self.data[self.pos + 1],
            self.data[self.pos + 2],
            self.data[self.pos + 3],
            self.data[self.pos + 4],
            self.data[self.pos + 5],
            self.data[self.pos + 6],
            self.data[self.pos + 7],
        ]);
        self.pos += 8;
        Ok(val)
    }

    pub(crate) fn read_byte(&mut self) -> Result<u8> {
        if self.pos >= self.data.len() {
            return Err(FitsError::InvalidFormat("HCompress: unexpected EOF".into()));
        }
        let b = self.data[self.pos];
        self.pos += 1;
        Ok(b)
    }

    pub(crate) fn read_nybble(&mut self) -> Result<u8> {
        if self.bits_in_buffer == 0 {
            self.bit_buffer = self.read_byte()? as u32;
            self.bits_in_buffer = 8;
        }
        self.bits_in_buffer -= 4;
        let nyb = ((self.bit_buffer >> self.bits_in_buffer) & 0x0F) as u8;
        Ok(nyb)
    }

    pub(crate) fn read_bits(&mut self, n: u8) -> Result<u32> {
        let mut result = 0u32;
        let mut remaining = n;
        while remaining > 0 {
            if self.bits_in_buffer == 0 {
                self.bit_buffer = self.read_byte()? as u32;
                self.bits_in_buffer = 8;
            }
            let take = remaining.min(self.bits_in_buffer);
            self.bits_in_buffer -= take;
            result =
                (result << take) | ((self.bit_buffer >> self.bits_in_buffer) & ((1 << take) - 1));
            remaining -= take;
        }
        Ok(result)
    }
}

pub(crate) fn hcomp_decode_stream(
    reader: &mut HCompReader,
    output: &mut [i64],
    nx: usize,
    ny: usize,
    scale: i32,
) -> Result<()> {
    let sum = reader.read_i64()?;
    output[0] = sum;

    let nbitplanes = [
        reader.read_byte()?,
        reader.read_byte()?,
        reader.read_byte()?,
    ];
    let max_bits = *nbitplanes.iter().max().unwrap_or(&0) as usize;

    if max_bits > 0 {
        hcomp_decode_bitplanes(reader, output, nx, ny, max_bits, &nbitplanes)?;
    }

    hcomp_undigitize(output, scale);
    hcomp_hinv(output, nx, ny);
    Ok(())
}

pub(crate) fn hcomp_decode_bitplanes(
    reader: &mut HCompReader,
    output: &mut [i64],
    nx: usize,
    ny: usize,
    max_bits: usize,
    nbitplanes: &[u8; 3],
) -> Result<()> {
    let nx2 = nx.div_ceil(2);
    let ny2 = ny.div_ceil(2);

    for bit in (0..max_bits).rev() {
        let plane_bit = bit as u8;
        if plane_bit < nbitplanes[0] {
            hcomp_decode_quadrant(
                reader,
                output,
                nx,
                &QuadrantBounds::new(0, ny2, 0, nx2),
                plane_bit,
            )?;
        }
        if plane_bit < nbitplanes[1] {
            hcomp_decode_quadrant(
                reader,
                output,
                nx,
                &QuadrantBounds::new(0, ny2, nx2, nx),
                plane_bit,
            )?;
            hcomp_decode_quadrant(
                reader,
                output,
                nx,
                &QuadrantBounds::new(ny2, ny, 0, nx2),
                plane_bit,
            )?;
        }
        if plane_bit < nbitplanes[2] {
            hcomp_decode_quadrant(
                reader,
                output,
                nx,
                &QuadrantBounds::new(ny2, ny, nx2, nx),
                plane_bit,
            )?;
        }
    }
    Ok(())
}

pub(crate) fn hcomp_decode_quadrant(
    reader: &mut HCompReader,
    output: &mut [i64],
    nx: usize,
    bounds: &QuadrantBounds,
    bit: u8,
) -> Result<()> {
    let code = reader.read_nybble()?;
    if code == 0 {
        return Ok(());
    }

    let mut bit_buffer = 0u8;
    let mut bits_remaining = 0u8;

    for y in bounds.y0..bounds.y1 {
        for x in bounds.x0..bounds.x1 {
            if bits_remaining == 0 {
                bit_buffer = reader.read_bits(8)? as u8;
                bits_remaining = 8;
            }
            bits_remaining -= 1;
            let b = ((bit_buffer >> bits_remaining) & 1) as i64;
            output[y * nx + x] |= b << bit;
        }
    }
    Ok(())
}

pub(crate) fn hcomp_undigitize(output: &mut [i64], scale: i32) {
    if scale <= 1 {
        return;
    }
    let scale64 = scale as i64;
    for v in output.iter_mut() {
        *v *= scale64;
    }
}

pub(crate) fn hcomp_hinv(a: &mut [i64], nx: usize, ny: usize) {
    let nmax = nx.max(ny);
    let log2n = ilog2_ceil(nmax);
    let mut nxtop = 1usize;
    let mut nytop = 1usize;

    for _ in 0..log2n {
        let nxf = nxtop.min(nx);
        let nyf = nytop.min(ny);
        let nxe = (nxtop * 2).min(nx);
        let nye = (nytop * 2).min(ny);

        hcomp_hinv_step(a, nx, nxf, nyf, nxe, nye);

        nxtop = nxe;
        nytop = nye;
    }
}

pub(crate) fn hcomp_hinv_step(a: &mut [i64], nx: usize, nxf: usize, nyf: usize, nxe: usize, nye: usize) {
    for j in 0..nyf {
        for i in 0..nxf {
            hcomp_hinv_2x2(a, nx, i, j, nxe, nye);
        }
    }
}

pub(crate) fn hcomp_hinv_2x2(a: &mut [i64], nx: usize, i: usize, j: usize, nxe: usize, nye: usize) {
    let i2 = i * 2;
    let j2 = j * 2;
    if i2 >= nxe || j2 >= nye {
        return;
    }

    let h0 = a[j * nx + i];
    let hx = if i2 + 1 < nxe {
        a[j * nx + nxe / 2 + i]
    } else {
        0
    };
    let hy = if j2 + 1 < nye {
        a[(nye / 2 + j) * nx + i]
    } else {
        0
    };
    let hc = if i2 + 1 < nxe && j2 + 1 < nye {
        a[(nye / 2 + j) * nx + nxe / 2 + i]
    } else {
        0
    };

    let sum = h0 + hx + hy + hc;
    a[j2 * nx + i2] = (sum + 2) >> 2;

    if i2 + 1 < nxe {
        let sum = h0 + hx - hy - hc;
        a[j2 * nx + i2 + 1] = (sum + 2) >> 2;
    }
    if j2 + 1 < nye {
        let sum = h0 - hx + hy - hc;
        a[(j2 + 1) * nx + i2] = (sum + 2) >> 2;
    }
    if i2 + 1 < nxe && j2 + 1 < nye {
        let sum = h0 - hx - hy + hc;
        a[(j2 + 1) * nx + i2 + 1] = (sum + 2) >> 2;
    }
}
