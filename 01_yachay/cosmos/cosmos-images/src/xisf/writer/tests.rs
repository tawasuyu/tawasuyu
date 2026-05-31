    use super::*;
    use std::io::Cursor;

    #[test]
    fn build_geometry_single_channel() {
        let g = build_geometry(100, 50, 1);
        assert_eq!(g, vec![100, 50]);
    }

    #[test]
    fn build_geometry_rgb() {
        let g = build_geometry(100, 50, 3);
        assert_eq!(g, vec![100, 50, 3]);
    }

    #[test]
    fn pad_to_alignment_no_padding_needed() {
        let data = vec![0u8; 16];
        let padded = pad_to_alignment(&data, 16);
        assert_eq!(padded.len(), 16);
    }

    #[test]
    fn pad_to_alignment_needs_padding() {
        let data = vec![0u8; 10];
        let padded = pad_to_alignment(&data, 16);
        assert_eq!(padded.len(), 16);
    }

    #[test]
    fn align_to_already_aligned() {
        assert_eq!(align_to(32, 16), 32);
    }

    #[test]
    fn align_to_needs_alignment() {
        assert_eq!(align_to(17, 16), 32);
    }

    #[test]
    fn u8_roundtrip() {
        let data = vec![0u8, 127, 255];
        let bytes = <u8 as XisfDataType>::to_le_bytes(&data);
        let restored = <u8 as XisfDataType>::from_le_bytes(&bytes);
        assert_eq!(data, restored);
    }

    #[test]
    fn u16_roundtrip() {
        let data = vec![0u16, 32768, 65535];
        let bytes = <u16 as XisfDataType>::to_le_bytes(&data);
        let restored = <u16 as XisfDataType>::from_le_bytes(&bytes);
        assert_eq!(data, restored);
    }

    #[test]
    fn f32_roundtrip() {
        let data = vec![0.0f32, 0.5, 1.0];
        let bytes = <f32 as XisfDataType>::to_le_bytes(&data);
        let restored = <f32 as XisfDataType>::from_le_bytes(&bytes);
        assert_eq!(data, restored);
    }

    #[test]
    fn writer_add_image_valid() {
        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);
        let data = vec![0u16; 100];
        let result = writer.add_image(&data, 10, 10, 1);
        assert!(result.is_ok());
    }

    #[test]
    fn writer_add_image_invalid_size() {
        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);
        let data = vec![0u16; 50];
        let result = writer.add_image(&data, 10, 10, 1);
        assert!(result.is_err());
    }

    #[test]
    fn writer_set_keyword() {
        use crate::fits::header::KeywordValue;

        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);
        writer.set_keyword("TELESCOP", "Test");
        assert_eq!(writer.keywords.len(), 1);
        assert_eq!(writer.keywords[0].name, "TELESCOP");
        assert_eq!(
            writer.keywords[0].value,
            Some(KeywordValue::String("Test".to_string()))
        );
    }

    #[test]
    fn write_complete_xisf() {
        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);

        let data: Vec<u16> = (0..100).collect();
        writer.add_image(&data, 10, 10, 1).unwrap();
        writer.set_keyword("TELESCOP", "TestScope");
        writer.set_keyword("FILTER", "R");

        let result = writer.write();
        assert!(result.is_ok());
    }

    #[test]
    fn write_rgb_image() {
        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);

        let data: Vec<f32> = vec![0.5; 300];
        writer.add_image(&data, 10, 10, 3).unwrap();

        let result = writer.write();
        assert!(result.is_ok());
    }

    #[test]
    fn write_multiple_images() {
        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);

        let data1: Vec<u8> = vec![128; 100];
        let data2: Vec<u16> = vec![32768; 100];

        writer.add_image(&data1, 10, 10, 1).unwrap();
        writer.add_image(&data2, 10, 10, 1).unwrap();

        let result = writer.write();
        assert!(result.is_ok());
    }

    #[test]
    fn roundtrip_u16_image() {
        use crate::xisf::{PixelStorage, XisfFile};

        let original_data: Vec<u16> = (0..100).collect();
        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);
        writer.add_image(&original_data, 10, 10, 1).unwrap();
        writer.set_keyword("TELESCOP", "RoundtripTest");

        let inner = writer.write_to_vec().unwrap();
        let mut reader = XisfFile::new(Cursor::new(inner)).unwrap();

        assert_eq!(reader.num_images(), 1);
        let info = reader.image_info(0).unwrap();
        assert_eq!(info.geometry, vec![10, 10]);
        assert!(matches!(
            info.sample_format,
            crate::xisf::SampleFormat::UInt16
        ));
        assert_eq!(info.pixel_storage, PixelStorage::Planar);

        let raw_bytes = reader.read_image_data_raw(0).unwrap();
        let restored = <u16 as XisfDataType>::from_le_bytes(&raw_bytes);
        assert_eq!(original_data, restored);

        let telescop = reader.get_keyword("TELESCOP");
        assert!(telescop.is_some());
        assert_eq!(
            telescop.unwrap().value,
            Some(KeywordValue::String("RoundtripTest".to_string()))
        );
    }

    #[test]
    fn roundtrip_f32_rgb_image() {
        use crate::xisf::{PixelStorage, XisfFile};

        let original_data: Vec<f32> = (0..300).map(|i| i as f32 / 300.0).collect();
        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);
        writer.add_image(&original_data, 10, 10, 3).unwrap();

        let inner = writer.write_to_vec().unwrap();
        let mut reader = XisfFile::new(Cursor::new(inner)).unwrap();

        assert_eq!(reader.num_images(), 1);
        let info = reader.image_info(0).unwrap();
        assert_eq!(info.geometry, vec![10, 10, 3]);
        assert!(matches!(
            info.sample_format,
            crate::xisf::SampleFormat::Float32
        ));
        assert!(matches!(info.color_space, crate::xisf::ColorSpace::RGB));
        assert_eq!(info.pixel_storage, PixelStorage::Planar);

        let raw_bytes = reader.read_image_data_raw(0).unwrap();
        let restored = <f32 as XisfDataType>::from_le_bytes(&raw_bytes);
        assert_eq!(original_data, restored);
    }

    #[test]
    fn roundtrip_multiple_images() {
        use crate::xisf::XisfFile;

        let data1: Vec<u8> = (0..100).collect();
        let data2: Vec<f64> = (0..100).map(|i| i as f64 * 0.01).collect();

        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);
        writer.add_image(&data1, 10, 10, 1).unwrap();
        writer.add_image(&data2, 10, 10, 1).unwrap();

        let inner = writer.write_to_vec().unwrap();
        let mut reader = XisfFile::new(Cursor::new(inner)).unwrap();

        assert_eq!(reader.num_images(), 2);

        let raw1 = reader.read_image_data_raw(0).unwrap();
        let restored1 = <u8 as XisfDataType>::from_le_bytes(&raw1);
        assert_eq!(data1, restored1);

        let raw2 = reader.read_image_data_raw(1).unwrap();
        let restored2 = <f64 as XisfDataType>::from_le_bytes(&raw2);
        assert_eq!(data2, restored2);
    }

    #[test]
    fn roundtrip_file_io() {
        use crate::xisf::XisfFile;
        use tempfile::NamedTempFile;

        let original_data: Vec<u16> = (0..256).collect();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        let writer = XisfWriter::create(path).unwrap();
        let mut writer = writer;
        writer.add_image(&original_data, 16, 16, 1).unwrap();
        writer.set_keyword("OBSERVER", "Test");
        writer.write().unwrap();

        let mut reader = XisfFile::open(path).unwrap();
        assert_eq!(reader.num_images(), 1);

        let raw_bytes = reader.read_image_data_raw(0).unwrap();
        let restored = <u16 as XisfDataType>::from_le_bytes(&raw_bytes);
        assert_eq!(original_data, restored);

        let observer = reader.get_keyword("OBSERVER");
        assert!(observer.is_some());
        assert_eq!(
            observer.unwrap().value,
            Some(KeywordValue::String("Test".to_string()))
        );
    }

    #[test]
    fn roundtrip_history_and_comment_keywords() {
        use crate::fits::header::Keyword;
        use crate::xisf::XisfFile;

        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);

        let data: Vec<u16> = (0..100).collect();
        writer.add_image(&data, 10, 10, 1).unwrap();

        writer
            .add_keyword(Keyword::real("EXPTIME", 300.0).with_comment("Exposure time in seconds"));
        writer.add_keyword(Keyword::history("Dark frame subtracted"));
        writer.add_keyword(Keyword::history("Flat field corrected"));
        writer.add_keyword(Keyword::comment("Processed with Celestial"));

        let inner = writer.write_to_vec().unwrap();
        let reader = XisfFile::new(Cursor::new(inner)).unwrap();

        let keywords = reader.keywords();
        assert_eq!(keywords.len(), 4);

        let exptime = &keywords[0];
        assert_eq!(exptime.name, "EXPTIME");
        assert_eq!(exptime.value, Some(KeywordValue::Real(300.0)));
        assert_eq!(
            exptime.comment,
            Some("Exposure time in seconds".to_string())
        );

        let history1 = &keywords[1];
        assert_eq!(history1.name, "HISTORY");
        assert_eq!(history1.value, None);
        assert_eq!(history1.comment, Some("Dark frame subtracted".to_string()));

        let history2 = &keywords[2];
        assert_eq!(history2.name, "HISTORY");
        assert_eq!(history2.value, None);
        assert_eq!(history2.comment, Some("Flat field corrected".to_string()));

        let comment = &keywords[3];
        assert_eq!(comment.name, "COMMENT");
        assert_eq!(comment.value, None);
        assert_eq!(
            comment.comment,
            Some("Processed with Celestial".to_string())
        );
    }

    #[test]
    fn single_quote_in_keyword_value() {
        use crate::xisf::XisfFile;

        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);

        let data: Vec<u16> = (0..100).collect();
        writer.add_image(&data, 10, 10, 1).unwrap();
        writer.set_keyword("STACKMTH", "Sigma Clip (2.5σ)");
        writer.set_keyword("INSTRUME", "Apollo-M MINI (IMX429)");

        let inner = writer.write_to_vec().unwrap();

        // Check XML doesn't contain &apos;
        let xml_str = String::from_utf8_lossy(&inner);
        assert!(
            !xml_str.contains("&apos;"),
            "XML should not contain &apos; escape sequence"
        );

        // Verify roundtrip
        let reader = XisfFile::new(Cursor::new(inner)).unwrap();
        let stackmth = reader.get_keyword("STACKMTH");
        assert!(stackmth.is_some());
        assert_eq!(
            stackmth.unwrap().value,
            Some(KeywordValue::String("Sigma Clip (2.5σ)".to_string()))
        );

        let instrume = reader.get_keyword("INSTRUME");
        assert!(instrume.is_some());
        assert_eq!(
            instrume.unwrap().value,
            Some(KeywordValue::String("Apollo-M MINI (IMX429)".to_string()))
        );
    }

    #[test]
    fn roundtrip_lz4_compressed_u16() {
        use crate::xisf::XisfFile;

        // Create data that should compress well (repeating pattern)
        let original_data: Vec<u16> = (0..1000).map(|i| (i % 256) as u16).collect();

        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer).compression(XisfCompression::Lz4);
        writer.add_image(&original_data, 100, 10, 1).unwrap();
        writer.set_keyword("COMPRESS", "LZ4");

        let inner = writer.write_to_vec().unwrap();
        let mut reader = XisfFile::new(Cursor::new(inner)).unwrap();

        assert_eq!(reader.num_images(), 1);
        let info = reader.image_info(0).unwrap();
        assert_eq!(info.compression, XisfCompression::Lz4);
        assert!(info.uncompressed_size.is_some());

        let raw_bytes = reader.read_image_data_raw(0).unwrap();
        let restored = <u16 as XisfDataType>::from_le_bytes(&raw_bytes);
        assert_eq!(original_data, restored);

        let compress = reader.get_keyword("COMPRESS");
        assert!(compress.is_some());
        assert_eq!(
            compress.unwrap().value,
            Some(KeywordValue::String("LZ4".to_string()))
        );
    }

    #[test]
    fn roundtrip_lz4_compressed_f32_rgb() {
        use crate::xisf::XisfFile;

        // Create RGB float data with patterns that should compress well
        let original_data: Vec<f32> = (0..3000).map(|i| (i % 256) as f32 / 255.0).collect();

        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer).compression(XisfCompression::Lz4);
        writer.add_image(&original_data, 100, 10, 3).unwrap();

        let inner = writer.write_to_vec().unwrap();
        let mut reader = XisfFile::new(Cursor::new(inner)).unwrap();

        let info = reader.image_info(0).unwrap();
        assert_eq!(info.geometry, vec![100, 10, 3]);
        assert!(matches!(info.color_space, crate::xisf::ColorSpace::RGB));

        let raw_bytes = reader.read_image_data_raw(0).unwrap();
        let restored = <f32 as XisfDataType>::from_le_bytes(&raw_bytes);
        assert_eq!(original_data, restored);
    }

    #[test]
    fn roundtrip_zlib_compressed_u16() {
        use crate::xisf::XisfFile;

        let original_data: Vec<u16> = (0..1000).map(|i| (i % 256) as u16).collect();

        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer).compression(XisfCompression::Zlib);
        writer.add_image(&original_data, 100, 10, 1).unwrap();

        let inner = writer.write_to_vec().unwrap();
        let mut reader = XisfFile::new(Cursor::new(inner)).unwrap();

        let info = reader.image_info(0).unwrap();
        assert_eq!(info.compression, XisfCompression::Zlib);

        let raw_bytes = reader.read_image_data_raw(0).unwrap();
        let restored = <u16 as XisfDataType>::from_le_bytes(&raw_bytes);
        assert_eq!(original_data, restored);
    }

    #[test]
    fn compression_fallback_for_incompressible_data() {
        use crate::xisf::XisfFile;

        // Random-like data that won't compress well
        let original_data: Vec<u8> = (0..100).map(|i| (i * 17 + 31) as u8).collect();

        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer).compression(XisfCompression::Lz4);
        writer.add_image(&original_data, 10, 10, 1).unwrap();

        let inner = writer.write_to_vec().unwrap();
        let mut reader = XisfFile::new(Cursor::new(inner)).unwrap();

        // Small random data might not compress, so compression could be None
        let _info = reader.image_info(0).unwrap();
        // Data should still round-trip correctly regardless of compression
        let raw_bytes = reader.read_image_data_raw(0).unwrap();
        let restored = <u8 as XisfDataType>::from_le_bytes(&raw_bytes);
        assert_eq!(original_data, restored);
    }

    #[test]
    fn compression_with_file_io() {
        use crate::xisf::XisfFile;
        use tempfile::NamedTempFile;

        let original_data: Vec<u16> = (0..1000).map(|i| (i % 256) as u16).collect();
        let temp_file = NamedTempFile::new().unwrap();
        let path = temp_file.path();

        let writer = XisfWriter::create(path)
            .unwrap()
            .compression(XisfCompression::Lz4);
        let mut writer = writer;
        writer.add_image(&original_data, 100, 10, 1).unwrap();
        writer.write().unwrap();

        let mut reader = XisfFile::open(path).unwrap();
        let info = reader.image_info(0).unwrap();
        assert_eq!(info.compression, XisfCompression::Lz4);

        let raw_bytes = reader.read_image_data_raw(0).unwrap();
        let restored = <u16 as XisfDataType>::from_le_bytes(&raw_bytes);
        assert_eq!(original_data, restored);
    }

    #[test]
    fn write_string_property() {
        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);

        let data: Vec<u16> = (0..100).collect();
        writer.add_image(&data, 10, 10, 1).unwrap();
        writer.add_property(XisfProperty::string(
            "PCL:AstrometricSolution:ProjectionSystem",
            "TAN",
        ));

        let inner = writer.write_to_vec().unwrap();
        let xml_str = String::from_utf8_lossy(&inner);
        assert!(xml_str.contains(r#"id="PCL:AstrometricSolution:ProjectionSystem""#));
        assert!(xml_str.contains(r#"type="String""#));
        assert!(xml_str.contains(">TAN</Property>"));
    }

    #[test]
    fn write_scalar_float64_property() {
        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);

        let data: Vec<u16> = (0..100).collect();
        writer.add_image(&data, 10, 10, 1).unwrap();
        writer.add_property(XisfProperty::float64("Observation:CenterRA", 191.758));

        let inner = writer.write_to_vec().unwrap();
        let xml_str = String::from_utf8_lossy(&inner);
        assert!(xml_str.contains(r#"id="Observation:CenterRA""#));
        assert!(xml_str.contains(r#"type="Float64""#));
        assert!(xml_str.contains(r#"value="191.758""#));
        assert!(xml_str.contains(r#"/>"#));
        assert!(
            !xml_str.contains("</Property>")
                || xml_str.contains(">TAN</Property>")
                || xml_str.contains(">Gnomonic</Property>")
        );
    }

    #[test]
    fn write_scalar_int32_property() {
        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);

        let data: Vec<u16> = (0..100).collect();
        writer.add_image(&data, 10, 10, 1).unwrap();
        writer.add_property(XisfProperty::int32("Observation:FrameCount", 42));

        let inner = writer.write_to_vec().unwrap();
        let xml_str = String::from_utf8_lossy(&inner);
        assert!(xml_str.contains(r#"id="Observation:FrameCount""#));
        assert!(xml_str.contains(r#"type="Int32""#));
        assert!(xml_str.contains(r#"value="42""#));
    }

    #[test]
    fn write_f64_vector_property() {
        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);

        let data: Vec<u16> = (0..100).collect();
        writer.add_image(&data, 10, 10, 1).unwrap();
        writer.add_property(XisfProperty::f64_vector(
            "PCL:AstrometricSolution:ReferenceCelestialCoordinates",
            vec![191.758, -5.048],
        ));

        let inner = writer.write_to_vec().unwrap();
        let xml_str = String::from_utf8_lossy(&inner);
        assert!(xml_str.contains(r#"id="PCL:AstrometricSolution:ReferenceCelestialCoordinates""#));
        assert!(xml_str.contains(r#"type="F64Vector""#));
        assert!(xml_str.contains(r#"length="2""#));
        assert!(xml_str.contains(r#"location="attachment:"#));
        assert!(xml_str.contains(r#"/>"#));
    }

    #[test]
    fn write_f64_matrix_property() {
        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);

        let data: Vec<u16> = (0..100).collect();
        writer.add_image(&data, 10, 10, 1).unwrap();
        writer.add_property(XisfProperty::f64_matrix(
            "PCL:AstrometricSolution:LinearTransformationMatrix",
            2,
            2,
            vec![-0.000409, 0.0, 0.0, 0.000409],
        ));

        let inner = writer.write_to_vec().unwrap();
        let xml_str = String::from_utf8_lossy(&inner);
        assert!(xml_str.contains(r#"id="PCL:AstrometricSolution:LinearTransformationMatrix""#));
        assert!(xml_str.contains(r#"type="F64Matrix""#));
        assert!(xml_str.contains(r#"rows="2""#));
        assert!(xml_str.contains(r#"columns="2""#));
        assert!(xml_str.contains(r#"location="attachment:"#));
    }

    #[test]
    fn wcs_keywords_to_properties() {
        use cosmos_wcs::WcsKeyword;

        let wcs_keywords = vec![
            WcsKeyword::string("CTYPE1", "RA---TAN"),
            WcsKeyword::string("CTYPE2", "DEC--TAN"),
            WcsKeyword::real("CRVAL1", 191.758),
            WcsKeyword::real("CRVAL2", -5.048),
            WcsKeyword::real("CRPIX1", 512.5),
            WcsKeyword::real("CRPIX2", 512.5),
            WcsKeyword::real("CD1_1", -0.000409),
            WcsKeyword::real("CD1_2", 0.0),
            WcsKeyword::real("CD2_1", 0.0),
            WcsKeyword::real("CD2_2", 0.000409),
        ];

        let properties = wcs_to_xisf_properties(&wcs_keywords);

        assert_eq!(properties.len(), 6);

        assert_eq!(properties[0].id, "PCL:AstrometricSolution:ProjectionSystem");
        assert_eq!(
            properties[0].value,
            XisfPropertyValue::String("Gnomonic".to_string())
        );

        assert_eq!(
            properties[1].id,
            "PCL:AstrometricSolution:ReferenceCelestialCoordinates"
        );
        assert_eq!(
            properties[1].value,
            XisfPropertyValue::F64Vector(vec![191.758, -5.048])
        );

        assert_eq!(
            properties[2].id,
            "PCL:AstrometricSolution:ReferenceImageCoordinates"
        );
        assert_eq!(
            properties[2].value,
            XisfPropertyValue::F64Vector(vec![512.5, 512.5])
        );

        assert_eq!(
            properties[3].id,
            "PCL:AstrometricSolution:LinearTransformationMatrix"
        );
        assert_eq!(
            properties[3].value,
            XisfPropertyValue::F64Matrix {
                rows: 2,
                cols: 2,
                data: vec![-0.000409, 0.0, 0.0, 0.000409]
            }
        );

        assert_eq!(
            properties[4].id,
            "PCL:AstrometricSolution:ReferenceNativeCoordinates"
        );
        assert_eq!(
            properties[4].value,
            XisfPropertyValue::F64Vector(vec![0.0, 90.0])
        );

        assert_eq!(
            properties[5].id,
            "PCL:AstrometricSolution:CelestialPoleNativeCoordinates"
        );
        assert_eq!(
            properties[5].value,
            XisfPropertyValue::F64Vector(vec![180.0, -5.048])
        );
    }

    #[test]
    fn extract_projection_code_tan() {
        assert_eq!(
            extract_projection_code("RA---TAN"),
            Some("Gnomonic".to_string())
        );
    }

    #[test]
    fn extract_projection_code_sin() {
        assert_eq!(
            extract_projection_code("RA---SIN"),
            Some("Orthographic".to_string())
        );
    }

    #[test]
    fn extract_projection_code_with_spaces() {
        assert_eq!(
            extract_projection_code("RA---TAN "),
            Some("Gnomonic".to_string())
        );
    }

    #[test]
    fn extract_projection_code_dec() {
        assert_eq!(
            extract_projection_code("DEC--TAN"),
            Some("Gnomonic".to_string())
        );
    }

    #[test]
    fn extract_projection_code_too_short() {
        assert_eq!(extract_projection_code("RA-TAN"), None);
    }

    #[test]
    fn write_properties_and_keywords_together() {
        let buffer = Cursor::new(Vec::new());
        let mut writer = XisfWriter::new(buffer);

        let data: Vec<u16> = (0..100).collect();
        writer.add_image(&data, 10, 10, 1).unwrap();
        writer.add_property(XisfProperty::string("Test:Property", "TestValue"));
        writer.set_keyword("TELESCOP", "TestScope");

        let inner = writer.write_to_vec().unwrap();
        let xml_str = String::from_utf8_lossy(&inner);

        assert!(xml_str.contains(r#"<Property id="Test:Property""#));
        assert!(xml_str.contains(r#"<FITSKeyword name="TELESCOP""#));
    }
