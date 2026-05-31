use super::*;

impl XisfWriter<BufWriter<File>> {
    pub fn create<P: AsRef<Path>>(path: P) -> Result<Self> {
        let file = File::create(path)?;
        Ok(Self::new(BufWriter::new(file)))
    }
}

impl XisfWriter<Cursor<Vec<u8>>> {
    pub fn write_to_vec(mut self) -> Result<Vec<u8>> {
        self.write_internal()?;
        Ok(self.writer.into_inner())
    }
}

impl<W: Write + Seek> XisfWriter<W> {
    pub fn new(writer: W) -> Self {
        Self {
            writer,
            images: Vec::new(),
            keywords: Vec::new(),
            properties: Vec::new(),
            compression: XisfCompression::None,
            property_blocks: Vec::new(),
        }
    }

    pub fn compression(mut self, compression: XisfCompression) -> Self {
        self.compression = compression;
        self
    }

    pub fn set_compression(&mut self, compression: XisfCompression) {
        self.compression = compression;
    }

    pub fn add_image<T: XisfDataType>(
        &mut self,
        data: &[T],
        width: usize,
        height: usize,
        channels: usize,
    ) -> Result<()> {
        let bounds = T::calculate_bounds(data);
        self.add_image_with_bounds(data, width, height, channels, bounds)
    }

    pub fn add_image_with_bounds<T: XisfDataType>(
        &mut self,
        data: &[T],
        width: usize,
        height: usize,
        channels: usize,
        bounds: (f64, f64),
    ) -> Result<()> {
        let geometry = build_geometry(width, height, channels);
        let expected_pixels: usize = geometry.iter().product();
        if data.len() != expected_pixels {
            return Err(XisfError::InvalidFormat(format!(
                "Data length {} does not match geometry {:?}",
                data.len(),
                geometry
            )));
        }

        let raw_bytes = T::to_le_bytes(data);
        let uncompressed_size = raw_bytes.len();
        let (compressed_bytes, compression_used) = self.compress_data(&raw_bytes);

        let info = ImageInfo {
            geometry,
            sample_format: T::SAMPLE_FORMAT,
            bounds,
            color_space: if channels == 3 {
                ColorSpace::RGB
            } else {
                ColorSpace::Gray
            },
            pixel_storage: PixelStorage::Planar,
            location: DataLocation::new(0, compressed_bytes.len() as u64),
            compression: compression_used,
            uncompressed_size: if compression_used.is_compressed() {
                Some(uncompressed_size as u64)
            } else {
                None
            },
        };

        self.images.push(ImageData {
            info,
            data: compressed_bytes,
        });
        Ok(())
    }

    fn compress_data(&self, data: &[u8]) -> (Vec<u8>, XisfCompression) {
        match self.compression {
            XisfCompression::None => (data.to_vec(), XisfCompression::None),
            XisfCompression::Lz4 | XisfCompression::Lz4Hc => {
                let compressed = lz4_flex::compress_prepend_size(data);
                // Only use compression if it actually saves space
                if compressed.len() < data.len() {
                    (compressed, self.compression)
                } else {
                    (data.to_vec(), XisfCompression::None)
                }
            }
            XisfCompression::Zlib => {
                use flate2::write::ZlibEncoder;
                use flate2::Compression;
                let mut encoder = ZlibEncoder::new(Vec::new(), Compression::default());
                if encoder.write_all(data).is_ok() {
                    if let Ok(compressed) = encoder.finish() {
                        if compressed.len() < data.len() {
                            return (compressed, XisfCompression::Zlib);
                        }
                    }
                }
                (data.to_vec(), XisfCompression::None)
            }
            XisfCompression::Zstd => {
                // Zstd not implemented yet, fall back to no compression
                (data.to_vec(), XisfCompression::None)
            }
        }
    }

    pub fn set_keyword(&mut self, name: &str, value: &str) {
        self.keywords.push(Keyword::string(name, value));
    }

    pub fn add_keyword(&mut self, keyword: Keyword) {
        self.keywords.push(keyword);
    }

    #[deprecated(note = "Use set_keyword() instead")]
    pub fn set_fits_keyword(&mut self, name: &str, value: &str) {
        self.set_keyword(name, value);
    }

    #[deprecated(note = "Use add_keyword() instead")]
    pub fn add_fits_keyword(&mut self, keyword: Keyword) {
        self.add_keyword(keyword);
    }

    pub fn add_property(&mut self, property: XisfProperty) {
        self.properties.push(property);
    }

    pub fn add_properties(&mut self, properties: impl IntoIterator<Item = XisfProperty>) {
        self.properties.extend(properties);
    }

    pub fn write(mut self) -> Result<()> {
        self.write_internal()?;
        self.writer.flush()?;
        Ok(())
    }

    fn write_internal(&mut self) -> Result<()> {
        self.build_property_blocks();
        self.writer.write_all(XISF_SIGNATURE)?;

        let padded_xml = self.generate_final_xml()?;
        self.write_header_length(padded_xml.len() as u32)?;
        self.writer.write_all(&padded_xml)?;
        self.write_property_data()?;
        self.write_image_data()?;
        Ok(())
    }

    fn build_property_blocks(&mut self) {
        self.property_blocks.clear();
        for (index, property) in self.properties.iter().enumerate() {
            if property.value.needs_data_block() {
                if let Some(data) = property.value.to_le_bytes() {
                    self.property_blocks.push(PropertyDataBlock {
                        property_index: index,
                        data,
                        location: DataLocation::new(0, 0),
                    });
                }
            }
        }
    }

    fn write_property_data(&mut self) -> Result<()> {
        for block in &self.property_blocks {
            self.writer.write_all(&block.data)?;
        }
        Ok(())
    }

    fn write_header_length(&mut self, length: u32) -> Result<()> {
        self.writer.write_u32::<LittleEndian>(length)?;
        self.writer.write_u32::<LittleEndian>(0)?;
        Ok(())
    }

    fn write_image_data(&mut self) -> Result<()> {
        for image in &self.images {
            self.writer.write_all(&image.data)?;
        }
        Ok(())
    }

    fn generate_final_xml(&mut self) -> Result<Vec<u8>> {
        self.set_placeholder_offsets();
        let first_pass = self.generate_xml_content()?;
        let mut padded_size = align_to(first_pass.len(), HEADER_ALIGNMENT);

        loop {
            self.calculate_final_offsets(padded_size);
            let final_xml = self.generate_xml_content()?;
            let actual_padded = align_to(final_xml.len(), HEADER_ALIGNMENT);

            if actual_padded <= padded_size {
                return Ok(pad_to_alignment(&final_xml, HEADER_ALIGNMENT));
            }
            padded_size = actual_padded;
        }
    }

    fn set_placeholder_offsets(&mut self) {
        let mut offset = 0u64;
        for block in &mut self.property_blocks {
            let size = block.data.len() as u64;
            block.location = DataLocation::new(offset, size);
            offset += size;
        }
        for image in &mut self.images {
            image.info.location = DataLocation::new(offset, image.data.len() as u64);
            offset += image.data.len() as u64;
        }
    }

    fn calculate_final_offsets(&mut self, padded_xml_size: usize) {
        let header_size = XISF_SIGNATURE.len() + 8 + padded_xml_size;
        let mut offset = header_size as u64;
        for block in &mut self.property_blocks {
            let size = block.data.len() as u64;
            block.location = DataLocation::new(offset, size);
            offset += size;
        }
        for image in &mut self.images {
            image.info.location = DataLocation::new(offset, image.data.len() as u64);
            offset += image.data.len() as u64;
        }
    }

    fn generate_xml_content(&self) -> Result<Vec<u8>> {
        let mut buffer = Cursor::new(Vec::new());
        let mut writer = Writer::new_with_indent(&mut buffer, b' ', 2);

        write_xml_declaration(&mut writer)?;
        self.write_xisf_content(&mut writer)?;

        Ok(buffer.into_inner())
    }

    fn write_xisf_content<Wr: Write>(&self, writer: &mut Writer<Wr>) -> Result<()> {
        let mut xisf_start = BytesStart::new("xisf");
        xisf_start.push_attribute(("version", "1.0"));
        xisf_start.push_attribute(("xmlns", "http://www.pixinsight.com/xisf"));
        xisf_start.push_attribute(("xmlns:xsi", "http://www.w3.org/2001/XMLSchema-instance"));
        xisf_start.push_attribute((
            "xsi:schemaLocation",
            "http://www.pixinsight.com/xisf http://pixinsight.com/xisf/xisf-1.0.xsd",
        ));
        writer
            .write_event(Event::Start(xisf_start))
            .map_err(|e| XisfError::XmlParse(e.to_string()))?;

        for image in &self.images {
            self.write_image_element(writer, &image.info)?;
        }

        writer
            .write_event(Event::End(BytesEnd::new("xisf")))
            .map_err(|e| XisfError::XmlParse(e.to_string()))?;

        Ok(())
    }

    fn write_image_element<Wr: Write>(
        &self,
        writer: &mut Writer<Wr>,
        info: &ImageInfo,
    ) -> Result<()> {
        let mut elem = BytesStart::new("Image");
        elem.push_attribute((
            "geometry",
            format_geometry_with_channels(&info.geometry).as_str(),
        ));
        elem.push_attribute(("sampleFormat", info.sample_format.as_str()));
        if info.sample_format.is_floating_point() {
            elem.push_attribute(("bounds", info.format_bounds().as_str()));
        }
        elem.push_attribute(("colorSpace", info.color_space.as_str()));
        if info.pixel_storage != PixelStorage::Planar {
            elem.push_attribute(("pixelStorage", info.pixel_storage.as_str()));
        }

        // Add compression attribute if data is compressed
        if info.compression.is_compressed() {
            let compression_str = format!(
                "{}:{}",
                info.compression.as_str(),
                info.uncompressed_size.unwrap_or(0)
            );
            elem.push_attribute(("compression", compression_str.as_str()));
        }

        elem.push_attribute(("location", info.location.format().as_str()));

        let has_content = !self.keywords.is_empty() || !self.properties.is_empty();

        if !has_content {
            writer
                .write_event(Event::Empty(elem))
                .map_err(|e| XisfError::XmlParse(e.to_string()))?;
        } else {
            writer
                .write_event(Event::Start(elem))
                .map_err(|e| XisfError::XmlParse(e.to_string()))?;

            self.write_properties(writer)?;
            self.write_keywords_as_fits(writer)?;

            writer
                .write_event(Event::End(BytesEnd::new("Image")))
                .map_err(|e| XisfError::XmlParse(e.to_string()))?;
        }
        Ok(())
    }

    fn write_keywords_as_fits<Wr: Write>(&self, writer: &mut Writer<Wr>) -> Result<()> {
        for keyword in &self.keywords {
            let mut elem = BytesStart::new("FITSKeyword");
            elem.push_attribute(("name", keyword.name.as_str()));

            let value_str = keyword_value_to_string(&keyword.value);
            elem.push_attribute(("value", value_str.as_str()));

            let comment_str = keyword.comment.as_deref().unwrap_or("");
            elem.push_attribute(("comment", comment_str));

            writer
                .write_event(Event::Empty(elem))
                .map_err(|e| XisfError::XmlParse(e.to_string()))?;
        }
        Ok(())
    }

    fn write_properties<Wr: Write>(&self, writer: &mut Writer<Wr>) -> Result<()> {
        use quick_xml::events::BytesText;

        let mut block_index = 0usize;

        for (prop_index, property) in self.properties.iter().enumerate() {
            let mut elem = BytesStart::new("Property");
            elem.push_attribute(("id", property.id.as_str()));
            elem.push_attribute(("type", property.value.type_name()));

            match &property.value {
                XisfPropertyValue::Float64(v) => {
                    elem.push_attribute(("value", v.to_string().as_str()));
                    writer
                        .write_event(Event::Empty(elem))
                        .map_err(|e| XisfError::XmlParse(e.to_string()))?;
                }
                XisfPropertyValue::Int32(v) => {
                    elem.push_attribute(("value", v.to_string().as_str()));
                    writer
                        .write_event(Event::Empty(elem))
                        .map_err(|e| XisfError::XmlParse(e.to_string()))?;
                }
                XisfPropertyValue::String(s) => {
                    writer
                        .write_event(Event::Start(elem))
                        .map_err(|e| XisfError::XmlParse(e.to_string()))?;
                    writer
                        .write_event(Event::Text(BytesText::new(s)))
                        .map_err(|e| XisfError::XmlParse(e.to_string()))?;
                    writer
                        .write_event(Event::End(BytesEnd::new("Property")))
                        .map_err(|e| XisfError::XmlParse(e.to_string()))?;
                }
                XisfPropertyValue::F64Vector(v) => {
                    elem.push_attribute(("length", v.len().to_string().as_str()));
                    if let Some(block) = self.find_property_block(prop_index, &mut block_index) {
                        elem.push_attribute(("location", block.location.format().as_str()));
                    }
                    writer
                        .write_event(Event::Empty(elem))
                        .map_err(|e| XisfError::XmlParse(e.to_string()))?;
                }
                XisfPropertyValue::F64Matrix { rows, cols, .. } => {
                    elem.push_attribute(("rows", rows.to_string().as_str()));
                    elem.push_attribute(("columns", cols.to_string().as_str()));
                    if let Some(block) = self.find_property_block(prop_index, &mut block_index) {
                        elem.push_attribute(("location", block.location.format().as_str()));
                    }
                    writer
                        .write_event(Event::Empty(elem))
                        .map_err(|e| XisfError::XmlParse(e.to_string()))?;
                }
            }
        }
        Ok(())
    }

    fn find_property_block(
        &self,
        prop_index: usize,
        hint: &mut usize,
    ) -> Option<&PropertyDataBlock> {
        for i in *hint..self.property_blocks.len() {
            if self.property_blocks[i].property_index == prop_index {
                *hint = i + 1;
                return Some(&self.property_blocks[i]);
            }
        }
        None
    }
}

pub(crate) fn keyword_value_to_string(value: &Option<KeywordValue>) -> String {
    match value {
        None => String::new(),
        Some(KeywordValue::Logical(b)) => {
            if *b {
                "T".to_string()
            } else {
                "F".to_string()
            }
        }
        Some(KeywordValue::Integer(i)) => i.to_string(),
        Some(KeywordValue::Real(f)) => {
            // Ensure decimal point is preserved for whole numbers
            let s = f.to_string();
            if s.contains('.') || s.contains('e') || s.contains('E') {
                s
            } else {
                format!("{}.0", s)
            }
        }
        Some(KeywordValue::String(s)) => s.clone(),
        Some(KeywordValue::Complex(r, i)) => format!("({}, {})", r, i),
    }
}

pub(crate) fn write_xml_declaration<W: Write>(writer: &mut Writer<W>) -> Result<()> {
    let decl = BytesDecl::new("1.0", Some("UTF-8"), None);
    writer
        .write_event(Event::Decl(decl))
        .map_err(|e| XisfError::XmlParse(e.to_string()))
}

pub(crate) fn build_geometry(width: usize, height: usize, channels: usize) -> Vec<usize> {
    if channels > 1 {
        vec![width, height, channels]
    } else {
        vec![width, height]
    }
}

pub(crate) fn pad_to_alignment(data: &[u8], alignment: usize) -> Vec<u8> {
    let mut padded = data.to_vec();
    let remainder = padded.len() % alignment;
    if remainder != 0 {
        padded.resize(padded.len() + (alignment - remainder), 0);
    }
    padded
}

pub(crate) fn align_to(size: usize, alignment: usize) -> usize {
    let remainder = size % alignment;
    if remainder == 0 {
        size
    } else {
        size + (alignment - remainder)
    }
}
