use super::*;

#[derive(Debug, Clone)]
pub struct ImageInfo {
    pub dimensions: Vec<usize>,
    pub bitpix: BitPix,
    pub is_signed: bool,
    pub bytes_per_pixel: usize,
    pub total_pixels: usize,
}

impl ImageInfo {
    pub fn new(dimensions: Vec<usize>, bitpix: BitPix) -> Self {
        let total_pixels = if dimensions.is_empty() {
            0
        } else {
            dimensions.iter().product()
        };
        let bytes_per_pixel = bitpix.bytes_per_pixel();
        let is_signed = matches!(bitpix, BitPix::I16 | BitPix::I32 | BitPix::I64);

        Self {
            dimensions,
            bitpix,
            is_signed,
            bytes_per_pixel,
            total_pixels,
        }
    }

    pub fn data_size_bytes(&self) -> usize {
        self.total_pixels * self.bytes_per_pixel
    }

    pub fn is_2d(&self) -> bool {
        self.dimensions.len() == 2
    }

    pub fn width(&self) -> Option<usize> {
        self.dimensions.first().copied()
    }

    pub fn height(&self) -> Option<usize> {
        self.dimensions.get(1).copied()
    }
}

pub struct ImageWriter {
    pub(crate) format: ImageFormat,
    pub(crate) path: std::path::PathBuf,
}

impl ImageWriter {
    pub fn new<P: AsRef<Path>>(path: P, format: ImageFormat) -> Self {
        Self {
            format,
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn write_image<T>(&self, data: &[T], info: &ImageInfo, keywords: &[Keyword]) -> Result<()>
    where
        T: crate::fits::data::array::DataArray + Clone,
    {
        match self.format {
            ImageFormat::Fits => {
                let mut writer = crate::fits::FitsWriter::create(&self.path)
                    .map_err(crate::core::ImageError::Fits)?;

                writer
                    .write_primary_image(data, &info.dimensions, keywords)
                    .map_err(crate::core::ImageError::Fits)?;

                Ok(())
            }
            ImageFormat::Xisf => {
                use crate::core::ImageError;
                Err(ImageError::UnsupportedFormat)
            }
            #[cfg(feature = "standard-formats")]
            ImageFormat::Png | ImageFormat::Tiff => {
                use crate::core::ImageError;
                Err(ImageError::UnsupportedFormat)
            }
        }
    }

    pub fn write_telescope_image<T>(
        &self,
        data: &[T],
        info: &ImageInfo,
        params: &TelescopeParams,
    ) -> Result<()>
    where
        T: crate::fits::data::array::DataArray + Clone,
    {
        let keywords = Self::build_telescope_keywords(params);
        self.write_image(data, info, &keywords)
    }

    pub(crate) fn build_telescope_keywords(params: &TelescopeParams) -> Vec<Keyword> {
        let mut keywords = Vec::new();
        keywords.push(Keyword::real("EXPTIME", params.exposure_time));

        if let Some(temp) = params.temperature {
            keywords.push(Keyword::real("CCD-TEMP", temp));
        }
        if let Some(gain) = params.gain {
            keywords.push(Keyword::real("GAIN", gain));
        }
        if let Some((x, y)) = params.binning {
            keywords.push(Keyword::integer("XBINNING", x as i64));
            keywords.push(Keyword::integer("YBINNING", y as i64));
        }
        if let Some(ref filt) = params.filter {
            keywords.push(Keyword::string("FILTER", filt));
        }

        keywords
    }
}
