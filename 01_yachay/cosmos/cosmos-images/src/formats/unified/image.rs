use super::*;

/// A fully-loaded, mutable astronomical image.
///
/// `Image` owns all pixel data and metadata in memory, allowing full
/// read/write access. Load from any supported format and save to any format.
///
/// # Example
/// ```ignore
/// let mut img = Image::open("input.fits")?;
///
/// // Modify pixels
/// if let Some(pixels) = img.pixels.as_f32_mut() {
///     for p in pixels.iter_mut() {
///         *p = (*p * 1.5).min(1.0);
///     }
/// }
///
/// // Modify metadata
/// img.set_keyword(Keyword::string("OBJECT", "M31"));
///
/// // Save to different format
/// img.save("output.xisf")?;
/// ```
#[derive(Debug, Clone)]
pub struct Image {
    pub pixels: PixelData,
    pub dimensions: Vec<usize>,
    pub keywords: Vec<Keyword>,
}

impl Image {
    /// Create a new image from pixel data.
    pub fn new(pixels: PixelData, dimensions: impl Into<Vec<usize>>) -> Self {
        Self {
            pixels,
            dimensions: dimensions.into(),
            keywords: Vec::new(),
        }
    }

    /// Open an image from a file (FITS or XISF).
    pub fn open<P: AsRef<Path>>(path: P) -> Result<Self> {
        let path_ref = path.as_ref();
        let extension = path_ref
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");

        let format = match ImageFormat::from_extension(extension) {
            Some(f) => f,
            None => {
                let mut file = std::fs::File::open(path_ref)?;
                ImageFormat::detect(&mut file)?
            }
        };

        match format {
            ImageFormat::Fits => Self::open_fits(path_ref),
            ImageFormat::Xisf => Self::open_xisf(path_ref),
            #[cfg(feature = "standard-formats")]
            ImageFormat::Png => Self::open_png(path_ref),
            #[cfg(feature = "standard-formats")]
            ImageFormat::Tiff => Self::open_tiff(path_ref),
        }
    }

    fn open_fits(path: &Path) -> Result<Self> {
        let mut fits = crate::fits::FitsFile::open(path).map_err(ImageError::Fits)?;
        let (dimensions, bitpix) = fits.get_image_info(0).map_err(ImageError::Fits)?;

        let pixels = match bitpix {
            BitPix::U8 => {
                let (_, data): (_, Vec<u8>) =
                    fits.primary_hdu_with_data().map_err(ImageError::Fits)?;
                PixelData::U8(data)
            }
            BitPix::I16 => {
                let (_, data): (_, Vec<i16>) =
                    fits.primary_hdu_with_data().map_err(ImageError::Fits)?;
                PixelData::I16(data)
            }
            BitPix::I32 => {
                let (_, data): (_, Vec<i32>) =
                    fits.primary_hdu_with_data().map_err(ImageError::Fits)?;
                PixelData::I32(data)
            }
            BitPix::I64 => {
                let (_, data): (_, Vec<i32>) =
                    fits.primary_hdu_with_data().map_err(ImageError::Fits)?;
                PixelData::I32(data)
            }
            BitPix::F32 => {
                let (_, data): (_, Vec<f32>) =
                    fits.primary_hdu_with_data().map_err(ImageError::Fits)?;
                PixelData::F32(data)
            }
            BitPix::F64 => {
                let (_, data): (_, Vec<f64>) =
                    fits.primary_hdu_with_data().map_err(ImageError::Fits)?;
                PixelData::F64(data)
            }
        };

        let header = fits.get_header(0).map_err(ImageError::Fits)?;
        let keywords = header.keywords().to_vec();

        Ok(Self {
            pixels,
            dimensions,
            keywords,
        })
    }

    fn open_xisf(path: &Path) -> Result<Self> {
        let mut xisf = crate::xisf::XisfFile::open(path).map_err(ImageError::Xisf)?;

        let info = xisf.image_info(0).ok_or_else(|| {
            ImageError::Xisf(crate::xisf::XisfError::InvalidFormat(
                "No images in file".to_string(),
            ))
        })?;

        let dimensions = info.geometry.clone();
        let sample_format = info.sample_format.clone();

        // Read raw bytes and convert based on sample format
        let raw_bytes = xisf.read_image_data_raw(0).map_err(ImageError::Xisf)?;

        let pixels = match sample_format {
            crate::xisf::SampleFormat::UInt8 => PixelData::U8(raw_bytes),
            crate::xisf::SampleFormat::UInt16 => {
                let data: Vec<u16> = raw_bytes
                    .chunks_exact(2)
                    .map(|b| u16::from_le_bytes([b[0], b[1]]))
                    .collect();
                PixelData::U16(data)
            }
            crate::xisf::SampleFormat::UInt32 => {
                let data: Vec<i32> = raw_bytes
                    .chunks_exact(4)
                    .map(|b| i32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                PixelData::I32(data)
            }
            crate::xisf::SampleFormat::Float32 => {
                let data: Vec<f32> = raw_bytes
                    .chunks_exact(4)
                    .map(|b| f32::from_le_bytes([b[0], b[1], b[2], b[3]]))
                    .collect();
                PixelData::F32(data)
            }
            crate::xisf::SampleFormat::Float64 => {
                let data: Vec<f64> = raw_bytes
                    .chunks_exact(8)
                    .map(|b| f64::from_le_bytes([b[0], b[1], b[2], b[3], b[4], b[5], b[6], b[7]]))
                    .collect();
                PixelData::F64(data)
            }
        };

        let keywords = xisf.keywords().to_vec();

        Ok(Self {
            pixels,
            dimensions,
            keywords,
        })
    }

    #[cfg(feature = "standard-formats")]
    pub fn open_png<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = std::fs::File::open(path)?;
        let buf_reader = BufReader::new(file);
        let decoder = png::Decoder::new(buf_reader);
        let mut reader = decoder
            .read_info()
            .map_err(|e| ImageError::FormatDetectionFailed(format!("PNG decode error: {}", e)))?;

        let buf_size = reader.output_buffer_size().ok_or_else(|| {
            ImageError::FormatDetectionFailed("Cannot determine PNG output buffer size".to_string())
        })?;
        let mut buf = vec![0; buf_size];
        let info = reader
            .next_frame(&mut buf)
            .map_err(|e| ImageError::FormatDetectionFailed(format!("PNG frame error: {}", e)))?;
        buf.truncate(info.buffer_size());

        let (pixels, dimensions) = Self::parse_png_data(&buf, &info)?;
        Ok(Self {
            pixels,
            dimensions,
            keywords: Vec::new(),
        })
    }

    #[cfg(feature = "standard-formats")]
    fn parse_png_data(buf: &[u8], info: &png::OutputInfo) -> Result<(PixelData, Vec<usize>)> {
        use png::ColorType;

        let channels = match info.color_type {
            ColorType::Grayscale => 1,
            ColorType::Rgb => 3,
            ColorType::GrayscaleAlpha => 2,
            ColorType::Rgba => 4,
            ColorType::Indexed => {
                return Err(ImageError::UnsupportedFormat);
            }
        };

        let dimensions = if channels == 1 {
            vec![info.width as usize, info.height as usize]
        } else {
            vec![info.width as usize, info.height as usize, channels]
        };

        let pixels = match info.bit_depth {
            png::BitDepth::Eight => PixelData::U8(buf.to_vec()),
            png::BitDepth::Sixteen => {
                let data: Vec<u16> = buf
                    .chunks_exact(2)
                    .map(|b| u16::from_be_bytes([b[0], b[1]]))
                    .collect();
                PixelData::U16(data)
            }
            _ => return Err(ImageError::UnsupportedFormat),
        };

        Ok((pixels, dimensions))
    }

    #[cfg(feature = "standard-formats")]
    pub fn open_tiff<P: AsRef<Path>>(path: P) -> Result<Self> {
        use tiff::decoder::Decoder;

        let file = std::fs::File::open(path)?;
        let mut decoder = Decoder::new(file)
            .map_err(|e| ImageError::FormatDetectionFailed(format!("TIFF decode error: {}", e)))?;

        let (width, height) = decoder.dimensions().map_err(|e| {
            ImageError::FormatDetectionFailed(format!("TIFF dimensions error: {}", e))
        })?;

        let image = decoder
            .read_image()
            .map_err(|e| ImageError::FormatDetectionFailed(format!("TIFF read error: {}", e)))?;

        let (pixels, channels) = Self::decode_tiff_image(image)?;
        let dimensions = Self::build_tiff_dimensions(width, height, channels);

        Ok(Self {
            pixels,
            dimensions,
            keywords: Vec::new(),
        })
    }

    #[cfg(feature = "standard-formats")]
    fn decode_tiff_image(image: tiff::decoder::DecodingResult) -> Result<(PixelData, usize)> {
        use tiff::decoder::DecodingResult;

        match image {
            DecodingResult::U8(data) => Ok((PixelData::U8(data), 1)),
            DecodingResult::U16(data) => Ok((PixelData::U16(data), 1)),
            DecodingResult::U32(data) => {
                let converted: Vec<i32> = data.iter().map(|&v| v as i32).collect();
                Ok((PixelData::I32(converted), 1))
            }
            DecodingResult::F32(data) => Ok((PixelData::F32(data), 1)),
            DecodingResult::F64(data) => Ok((PixelData::F64(data), 1)),
            _ => Err(ImageError::UnsupportedFormat),
        }
    }

    #[cfg(feature = "standard-formats")]
    fn build_tiff_dimensions(width: u32, height: u32, channels: usize) -> Vec<usize> {
        if channels == 1 {
            vec![width as usize, height as usize]
        } else {
            vec![width as usize, height as usize, channels]
        }
    }

    /// Save the image to a file. Format is determined by extension.
    pub fn save<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        let path_ref = path.as_ref();
        let extension = path_ref
            .extension()
            .and_then(|ext| ext.to_str())
            .unwrap_or("");

        match ImageFormat::from_extension(extension) {
            Some(ImageFormat::Fits) => self.save_fits(path_ref),
            Some(ImageFormat::Xisf) => self.save_xisf(path_ref),
            #[cfg(feature = "standard-formats")]
            Some(ImageFormat::Png) => self.save_png(path_ref),
            #[cfg(feature = "standard-formats")]
            Some(ImageFormat::Tiff) => self.save_tiff(path_ref),
            None => Err(ImageError::UnsupportedFormat),
        }
    }

    fn save_fits(&self, path: &Path) -> Result<()> {
        let mut writer = FitsWriter::create(path).map_err(ImageError::Fits)?;

        match &self.pixels {
            PixelData::U8(data) => writer
                .write_primary_image(data, &self.dimensions, &self.keywords)
                .map_err(ImageError::Fits)?,
            PixelData::U16(data) => {
                // FITS doesn't have u16, convert to i16 with offset
                let converted: Vec<i16> = data.iter().map(|&v| v as i16).collect();
                writer
                    .write_primary_image(&converted, &self.dimensions, &self.keywords)
                    .map_err(ImageError::Fits)?
            }
            PixelData::I16(data) => writer
                .write_primary_image(data, &self.dimensions, &self.keywords)
                .map_err(ImageError::Fits)?,
            PixelData::I32(data) => writer
                .write_primary_image(data, &self.dimensions, &self.keywords)
                .map_err(ImageError::Fits)?,
            PixelData::F32(data) => writer
                .write_primary_image(data, &self.dimensions, &self.keywords)
                .map_err(ImageError::Fits)?,
            PixelData::F64(data) => writer
                .write_primary_image(data, &self.dimensions, &self.keywords)
                .map_err(ImageError::Fits)?,
        }

        Ok(())
    }

    fn save_xisf(&self, path: &Path) -> Result<()> {
        use crate::xisf::wcs_to_xisf_properties;

        let mut writer = XisfWriter::create(path).map_err(ImageError::Xisf)?;

        let (width, height, channels) = self.extract_dimensions();

        // XISF expects planar format for RGB images - convert from interleaved
        if channels == 3 {
            let mut planar_image = self.clone();
            planar_image.interleaved_to_planar();
            Self::write_xisf_pixels(&mut writer, &planar_image.pixels, width, height, channels)?;
        } else {
            Self::write_xisf_pixels(&mut writer, &self.pixels, width, height, channels)?;
        }

        for kw in &self.keywords {
            writer.add_keyword(kw.clone());
        }

        // Convert WCS keywords to native XISF properties for PixInsight compatibility
        let wcs_keywords: Vec<WcsKeyword> = self
            .keywords
            .iter()
            .filter_map(|kw| {
                let value = match &kw.value {
                    Some(crate::fits::header::KeywordValue::Real(v)) => WcsKeywordValue::Real(*v),
                    Some(crate::fits::header::KeywordValue::Integer(v)) => {
                        WcsKeywordValue::Integer(*v)
                    }
                    Some(crate::fits::header::KeywordValue::String(v)) => {
                        WcsKeywordValue::String(v.clone())
                    }
                    _ => return None,
                };
                Some(WcsKeyword {
                    name: kw.name.clone(),
                    value,
                })
            })
            .collect();

        let properties = wcs_to_xisf_properties(&wcs_keywords);
        writer.add_properties(properties);

        writer.write().map_err(ImageError::Xisf)
    }

    fn write_xisf_pixels<W: std::io::Write + std::io::Seek>(
        writer: &mut XisfWriter<W>,
        pixels: &PixelData,
        width: usize,
        height: usize,
        channels: usize,
    ) -> Result<()> {
        match pixels {
            PixelData::U8(data) => writer
                .add_image(data, width, height, channels)
                .map_err(ImageError::Xisf)?,
            PixelData::U16(data) => writer
                .add_image(data, width, height, channels)
                .map_err(ImageError::Xisf)?,
            PixelData::I16(data) => {
                let converted: Vec<u16> = data.iter().map(|&v| v as u16).collect();
                writer
                    .add_image(&converted, width, height, channels)
                    .map_err(ImageError::Xisf)?
            }
            PixelData::I32(data) => {
                let converted: Vec<u32> = data.iter().map(|&v| v as u32).collect();
                writer
                    .add_image(&converted, width, height, channels)
                    .map_err(ImageError::Xisf)?
            }
            PixelData::F32(data) => writer
                .add_image(data, width, height, channels)
                .map_err(ImageError::Xisf)?,
            PixelData::F64(data) => writer
                .add_image(data, width, height, channels)
                .map_err(ImageError::Xisf)?,
        }
        Ok(())
    }

    #[cfg(feature = "standard-formats")]
    fn save_png(&self, path: &Path) -> Result<()> {
        let (width, height, channels) = self.extract_dimensions();
        let color_type = Self::channels_to_png_color_type(channels)?;
        let file = std::fs::File::create(path)?;

        match &self.pixels {
            PixelData::U8(data) => Self::write_png_u8(file, width, height, color_type, data),
            PixelData::U16(data) => Self::write_png_u16(file, width, height, color_type, data),
            _ => Err(ImageError::UnsupportedFormat),
        }
    }

    #[cfg(feature = "standard-formats")]
    fn channels_to_png_color_type(channels: usize) -> Result<png::ColorType> {
        match channels {
            1 => Ok(png::ColorType::Grayscale),
            2 => Ok(png::ColorType::GrayscaleAlpha),
            3 => Ok(png::ColorType::Rgb),
            4 => Ok(png::ColorType::Rgba),
            _ => Err(ImageError::UnsupportedFormat),
        }
    }

    #[cfg(feature = "standard-formats")]
    fn write_png_u8(
        file: std::fs::File,
        width: usize,
        height: usize,
        color_type: png::ColorType,
        data: &[u8],
    ) -> Result<()> {
        let mut encoder = png::Encoder::new(file, width as u32, height as u32);
        encoder.set_color(color_type);
        encoder.set_depth(png::BitDepth::Eight);
        let mut writer = encoder
            .write_header()
            .map_err(|e| ImageError::FormatDetectionFailed(format!("PNG header error: {}", e)))?;
        writer
            .write_image_data(data)
            .map_err(|e| ImageError::FormatDetectionFailed(format!("PNG write error: {}", e)))
    }

    #[cfg(feature = "standard-formats")]
    fn write_png_u16(
        file: std::fs::File,
        width: usize,
        height: usize,
        color_type: png::ColorType,
        data: &[u16],
    ) -> Result<()> {
        let mut encoder = png::Encoder::new(file, width as u32, height as u32);
        encoder.set_color(color_type);
        encoder.set_depth(png::BitDepth::Sixteen);
        let mut writer = encoder
            .write_header()
            .map_err(|e| ImageError::FormatDetectionFailed(format!("PNG header error: {}", e)))?;
        let bytes: Vec<u8> = data.iter().flat_map(|&v| v.to_be_bytes()).collect();
        writer
            .write_image_data(&bytes)
            .map_err(|e| ImageError::FormatDetectionFailed(format!("PNG write error: {}", e)))
    }

    #[cfg(feature = "standard-formats")]
    fn save_tiff(&self, path: &Path) -> Result<()> {
        let file = std::fs::File::create(path)?;
        let mut encoder = TiffEncoder::new(file)
            .map_err(|e| ImageError::FormatDetectionFailed(format!("TIFF encoder error: {}", e)))?;

        let (width, height, channels) = self.extract_dimensions();
        self.write_tiff_image(&mut encoder, width as u32, height as u32, channels)
    }

    #[cfg(feature = "standard-formats")]
    fn write_tiff_image(
        &self,
        encoder: &mut tiff::encoder::TiffEncoder<std::fs::File>,
        width: u32,
        height: u32,
        channels: usize,
    ) -> Result<()> {
        match (&self.pixels, channels) {
            (PixelData::U8(data), 1) => {
                encoder.write_image::<colortype::Gray8>(width, height, data)
            }
            (PixelData::U8(data), 3) => encoder.write_image::<colortype::RGB8>(width, height, data),
            (PixelData::U16(data), 1) => {
                encoder.write_image::<colortype::Gray16>(width, height, data)
            }
            (PixelData::U16(data), 3) => {
                encoder.write_image::<colortype::RGB16>(width, height, data)
            }
            (PixelData::I32(data), 1) => {
                let converted: Vec<u32> = data.iter().map(|&v| v as u32).collect();
                encoder.write_image::<colortype::Gray32>(width, height, &converted)
            }
            (PixelData::F32(data), 1) => {
                encoder.write_image::<colortype::Gray32Float>(width, height, data)
            }
            _ => return Err(ImageError::UnsupportedFormat),
        }
        .map_err(|e| ImageError::FormatDetectionFailed(format!("TIFF write error: {}", e)))
    }

    fn extract_dimensions(&self) -> (usize, usize, usize) {
        let width = self.dimensions.first().copied().unwrap_or(1);
        let height = self.dimensions.get(1).copied().unwrap_or(1);
        let channels = self.dimensions.get(2).copied().unwrap_or(1);
        (width, height, channels)
    }

    // Dimension accessors
    pub fn width(&self) -> usize {
        self.dimensions.first().copied().unwrap_or(0)
    }

    pub fn height(&self) -> usize {
        self.dimensions.get(1).copied().unwrap_or(1)
    }

    pub fn channels(&self) -> usize {
        self.dimensions.get(2).copied().unwrap_or(1)
    }

    pub fn is_rgb(&self) -> bool {
        self.channels() == 3
    }

    pub fn kind(&self) -> ImageKind {
        ImageKind::from_dimensions(&self.dimensions)
    }

    // Keyword helpers
    pub fn get_keyword(&self, name: &str) -> Option<&Keyword> {
        self.keywords.iter().find(|k| k.name == name)
    }

    pub fn set_keyword(&mut self, kw: Keyword) {
        if let Some(existing) = self.keywords.iter_mut().find(|k| k.name == kw.name) {
            *existing = kw;
        } else {
            self.keywords.push(kw);
        }
    }

    pub fn remove_keyword(&mut self, name: &str) {
        self.keywords.retain(|k| k.name != name);
    }

    pub fn interleaved_to_planar(&mut self) {
        if self.channels() != 3 {
            return;
        }
        let pixel_count = self.width() * self.height();

        macro_rules! convert {
            ($data:expr) => {{
                let mut planar = vec![Default::default(); $data.len()];
                for i in 0..pixel_count {
                    planar[i] = $data[i * 3];
                    planar[pixel_count + i] = $data[i * 3 + 1];
                    planar[pixel_count * 2 + i] = $data[i * 3 + 2];
                }
                *$data = planar;
            }};
        }

        match &mut self.pixels {
            PixelData::U8(data) => convert!(data),
            PixelData::U16(data) => convert!(data),
            PixelData::I16(data) => convert!(data),
            PixelData::I32(data) => convert!(data),
            PixelData::F32(data) => convert!(data),
            PixelData::F64(data) => convert!(data),
        }
    }

    pub fn planar_to_interleaved(&mut self) {
        if self.channels() != 3 {
            return;
        }
        let pixel_count = self.width() * self.height();

        macro_rules! convert {
            ($data:expr) => {{
                let mut interleaved = vec![Default::default(); $data.len()];
                for i in 0..pixel_count {
                    interleaved[i * 3] = $data[i];
                    interleaved[i * 3 + 1] = $data[pixel_count + i];
                    interleaved[i * 3 + 2] = $data[pixel_count * 2 + i];
                }
                *$data = interleaved;
            }};
        }

        match &mut self.pixels {
            PixelData::U8(data) => convert!(data),
            PixelData::U16(data) => convert!(data),
            PixelData::I16(data) => convert!(data),
            PixelData::I32(data) => convert!(data),
            PixelData::F32(data) => convert!(data),
            PixelData::F64(data) => convert!(data),
        }
    }

    pub fn normalize(&mut self) {
        let (min, max) = self.pixel_range();
        let range = (max - min).max(f64::MIN_POSITIVE);

        macro_rules! normalize {
            ($data:expr, $t:ty) => {{
                for v in $data.iter_mut() {
                    *v = ((*v as f64 - min) / range) as $t;
                }
            }};
        }

        match &mut self.pixels {
            PixelData::U8(data) => normalize!(data, u8),
            PixelData::U16(data) => normalize!(data, u16),
            PixelData::I16(data) => normalize!(data, i16),
            PixelData::I32(data) => normalize!(data, i32),
            PixelData::F32(data) => normalize!(data, f32),
            PixelData::F64(data) => normalize!(data, f64),
        }
    }

    pub fn normalize_to_f32(&mut self) {
        let (min, max) = self.pixel_range();
        let range = (max - min).max(f64::MIN_POSITIVE);

        let normalized: Vec<f32> = match &self.pixels {
            PixelData::U8(d) => d
                .iter()
                .map(|&v| ((v as f64 - min) / range) as f32)
                .collect(),
            PixelData::U16(d) => d
                .iter()
                .map(|&v| ((v as f64 - min) / range) as f32)
                .collect(),
            PixelData::I16(d) => d
                .iter()
                .map(|&v| ((v as f64 - min) / range) as f32)
                .collect(),
            PixelData::I32(d) => d
                .iter()
                .map(|&v| ((v as f64 - min) / range) as f32)
                .collect(),
            PixelData::F32(d) => d
                .iter()
                .map(|&v| ((v as f64 - min) / range) as f32)
                .collect(),
            PixelData::F64(d) => d.iter().map(|&v| ((v - min) / range) as f32).collect(),
        };
        self.pixels = PixelData::F32(normalized);
    }

    pub(crate) fn pixel_range(&self) -> (f64, f64) {
        match &self.pixels {
            PixelData::U8(d) => {
                let min = d.iter().copied().min().unwrap_or(0) as f64;
                let max = d.iter().copied().max().unwrap_or(0) as f64;
                (min, max)
            }
            PixelData::U16(d) => {
                let min = d.iter().copied().min().unwrap_or(0) as f64;
                let max = d.iter().copied().max().unwrap_or(0) as f64;
                (min, max)
            }
            PixelData::I16(d) => {
                let min = d.iter().copied().min().unwrap_or(0) as f64;
                let max = d.iter().copied().max().unwrap_or(0) as f64;
                (min, max)
            }
            PixelData::I32(d) => {
                let min = d.iter().copied().min().unwrap_or(0) as f64;
                let max = d.iter().copied().max().unwrap_or(0) as f64;
                (min, max)
            }
            PixelData::F32(d) => {
                let min = d.iter().copied().fold(f32::INFINITY, f32::min) as f64;
                let max = d.iter().copied().fold(f32::NEG_INFINITY, f32::max) as f64;
                (min, max)
            }
            PixelData::F64(d) => {
                let min = d.iter().copied().fold(f64::INFINITY, f64::min);
                let max = d.iter().copied().fold(f64::NEG_INFINITY, f64::max);
                (min, max)
            }
        }
    }

    pub fn debayer(&mut self, pattern: BayerPattern) {
        if self.channels() != 1 {
            return;
        }

        let width = self.width();
        let height = self.height();

        match &self.pixels {
            PixelData::U8(data) => {
                let rgb = debayer_bilinear_u8(data, width, height, pattern);
                self.pixels = PixelData::U8(rgb);
            }
            PixelData::U16(data) => {
                let rgb = debayer_bilinear_u16(data, width, height, pattern);
                self.pixels = PixelData::U16(rgb);
            }
            _ => return,
        }

        self.dimensions = vec![width, height, 3];
    }
}
