use super::*;

/// Unified astronomical image builder that can write to FITS or XISF format.
///
/// # Example
/// ```ignore
/// let data: Vec<u16> = capture_image();
/// AstroImage::new(&data, [1920, 1080])
///     .wcs(&wcs)
///     .keyword(Keyword::string("OBJECT", "M31"))
///     .keyword(Keyword::real("EXPTIME", 30.0))
///     .write_fits("output.fits")?;
/// ```
pub struct AstroImage<'a, T> {
    data: &'a [T],
    dimensions: Vec<usize>,
    keywords: Vec<Keyword>,
    wcs: Option<&'a Wcs>,
    compressed: bool,
    tile_size: Option<(usize, usize)>,
}

impl<'a, T> AstroImage<'a, T>
where
    T: DataArray + XisfDataType + Clone,
{
    pub fn new(data: &'a [T], dimensions: impl Into<Vec<usize>>) -> Self {
        Self {
            data,
            dimensions: dimensions.into(),
            keywords: Vec::new(),
            wcs: None,
            compressed: true,
            tile_size: None,
        }
    }

    pub fn wcs(mut self, wcs: &'a Wcs) -> Self {
        self.wcs = Some(wcs);
        self
    }

    pub fn keyword(mut self, kw: Keyword) -> Self {
        self.keywords.push(kw);
        self
    }

    pub fn keywords(mut self, keywords: impl IntoIterator<Item = Keyword>) -> Self {
        self.keywords.extend(keywords);
        self
    }

    pub fn compressed(mut self, compress: bool) -> Self {
        self.compressed = compress;
        self
    }

    pub fn tile_size(mut self, width: usize, height: usize) -> Self {
        self.tile_size = Some((width, height));
        self
    }

    pub fn image_kind(&self) -> ImageKind {
        ImageKind::from_dimensions(&self.dimensions)
    }

    /// Write to FITS format.
    pub fn write_fits<P: AsRef<Path>>(&self, path: P) -> crate::fits::Result<()> {
        let all_keywords = self.build_keywords();
        let mut writer = FitsWriter::create(path)?;

        if self.compressed {
            let tile_size = self.compute_tile_size();
            writer.write_compressed_image(
                self.data,
                &self.dimensions,
                tile_size,
                CompressionAlgorithm::Rice,
                &all_keywords,
            )
        } else {
            writer.write_primary_image_with_checksum(self.data, &self.dimensions, &all_keywords)
        }
    }

    /// Write to XISF format.
    pub fn write_xisf<P: AsRef<Path>>(&self, path: P) -> crate::xisf::Result<()> {
        let mut writer = XisfWriter::create(path)?;

        let (width, height, channels) = self.extract_dimensions();
        writer.add_image(self.data, width, height, channels)?;

        for kw in self.build_keywords() {
            writer.add_keyword(kw);
        }

        writer.write()
    }

    /// Write to the format determined by file extension.
    pub fn write_to<P: AsRef<Path>>(&self, path: P) -> Result<()> {
        use crate::core::ImageError;

        let path_ref = path.as_ref();
        let ext = path_ref.extension().and_then(|e| e.to_str()).unwrap_or("");

        match ImageFormat::from_extension(ext) {
            Some(ImageFormat::Fits) => self.write_fits(path).map_err(ImageError::Fits),
            Some(ImageFormat::Xisf) => self.write_xisf(path).map_err(ImageError::Xisf),
            #[cfg(feature = "standard-formats")]
            Some(ImageFormat::Png) | Some(ImageFormat::Tiff) => Err(ImageError::UnsupportedFormat),
            None => Err(ImageError::FormatDetectionFailed(format!(
                "Unknown extension: {}",
                ext
            ))),
        }
    }

    fn extract_dimensions(&self) -> (usize, usize, usize) {
        let width = self.dimensions.first().copied().unwrap_or(1);
        let height = self.dimensions.get(1).copied().unwrap_or(1);
        let channels = self.dimensions.get(2).copied().unwrap_or(1);
        (width, height, channels)
    }

    fn build_keywords(&self) -> Vec<Keyword> {
        let mut keywords = Vec::new();

        if let Some(wcs) = self.wcs {
            keywords.extend(wcs_to_keywords(wcs));
        }

        keywords.extend(self.keywords.iter().cloned());

        keywords
    }

    fn compute_tile_size(&self) -> (usize, usize) {
        if let Some(size) = self.tile_size {
            return size;
        }

        let width = self.dimensions.first().copied().unwrap_or(1);
        let height = self.dimensions.get(1).copied().unwrap_or(1);

        let tile_w = width.min(DEFAULT_TILE_SIZE);
        let tile_h = height.min(DEFAULT_TILE_SIZE);

        (tile_w, tile_h)
    }
}

fn wcs_to_keywords(wcs: &Wcs) -> Vec<Keyword> {
    wcs.to_keywords()
        .into_iter()
        .map(wcs_keyword_to_keyword)
        .collect()
}

fn wcs_keyword_to_keyword(wk: WcsKeyword) -> Keyword {
    match wk.value {
        WcsKeywordValue::Real(v) => Keyword::real(wk.name, v),
        WcsKeywordValue::Integer(v) => Keyword::integer(wk.name, v),
        WcsKeywordValue::String(v) => Keyword::string(wk.name, v),
    }
}
