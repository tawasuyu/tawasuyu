use super::*;
use crate::fits::header::KeywordValue;

use crate::test_utils::*;
use std::io::Cursor;

#[test]
fn image_format_from_extension() {
    let fits_extensions = ["fits", "FITS", "fit", "FIT", "fts", "FTS"];
    let xisf_extensions = ["xisf", "XISF"];

    for ext in fits_extensions {
        assert_eq!(ImageFormat::from_extension(ext), Some(ImageFormat::Fits));
    }

    for ext in xisf_extensions {
        assert_eq!(ImageFormat::from_extension(ext), Some(ImageFormat::Xisf));
    }

    assert_eq!(ImageFormat::from_extension("jpg"), None);
    assert_eq!(ImageFormat::from_extension(""), None);
    assert_eq!(ImageFormat::from_extension("unknown"), None);
}

#[test]
fn image_format_from_magic_bytes() {
    let fits_magic = b"SIMPLE  =                    T";
    let xisf_magic = b"XISF0100";

    assert_eq!(
        ImageFormat::from_magic_bytes(fits_magic),
        Some(ImageFormat::Fits)
    );
    assert_eq!(
        ImageFormat::from_magic_bytes(xisf_magic),
        Some(ImageFormat::Xisf)
    );

    assert_eq!(ImageFormat::from_magic_bytes(b"INVALID"), None);
    assert_eq!(ImageFormat::from_magic_bytes(b""), None);
    assert_eq!(ImageFormat::from_magic_bytes(&[0xFF, 0xFE]), None);
}

#[test]
fn image_format_detect_from_reader() {
    let fits_data = create_minimal_fits();
    let mut cursor = Cursor::new(fits_data);
    let format = ImageFormat::detect(&mut cursor).unwrap();
    assert_eq!(format, ImageFormat::Fits);

    let invalid_data = vec![0x00, 0x01, 0x02];
    let mut invalid_cursor = Cursor::new(invalid_data);
    let result = ImageFormat::detect(&mut invalid_cursor);
    assert!(result.is_err());
}

#[test]
fn image_info_creation_and_methods() {
    let dims = vec![1024, 1024];
    let bitpix = BitPix::I16;
    let info = ImageInfo::new(dims.clone(), bitpix);

    assert_eq!(info.dimensions, dims);
    assert_eq!(info.bitpix, bitpix);
    assert_eq!(info.data_size_bytes(), 1024 * 1024 * 2);
    assert!(info.is_2d());
    assert_eq!(info.width(), Some(1024));
    assert_eq!(info.height(), Some(1024));
}

#[test]
fn image_info_edge_cases() {
    let info_1d = ImageInfo::new(vec![1000], BitPix::U8);
    assert!(!info_1d.is_2d());
    assert_eq!(info_1d.width(), Some(1000));
    assert_eq!(info_1d.height(), None);

    let info_3d = ImageInfo::new(vec![10, 10, 10], BitPix::F32);
    assert!(!info_3d.is_2d());
    assert_eq!(info_3d.width(), Some(10));
    assert_eq!(info_3d.height(), Some(10));

    let info_empty = ImageInfo::new(vec![], BitPix::I32);
    assert!(!info_empty.is_2d());
    assert_eq!(info_empty.width(), None);
    assert_eq!(info_empty.height(), None);
    assert_eq!(info_empty.data_size_bytes(), 0);
}

#[test]
fn image_writer_build_telescope_keywords() {
    use crate::fits::header::KeywordValue;

    let params = TelescopeParams {
        exposure_time: 30.0,
        temperature: Some(-15.5),
        gain: Some(100.0),
        binning: Some((2, 2)),
        filter: Some("Ha".to_string()),
    };
    let keywords = ImageWriter::build_telescope_keywords(&params);

    assert_eq!(keywords.len(), 6);
    assert_eq!(keywords[0].name, "EXPTIME");
    assert_eq!(keywords[0].value, Some(KeywordValue::Real(30.0)));

    let params_none = TelescopeParams {
        exposure_time: 60.0,
        temperature: None,
        gain: None,
        binning: None,
        filter: None,
    };
    let keywords_none = ImageWriter::build_telescope_keywords(&params_none);

    assert_eq!(keywords_none.len(), 1);
    assert_eq!(keywords_none[0].name, "EXPTIME");
}

#[test]
fn telescope_keywords_extreme_binning() {
    use crate::fits::header::KeywordValue;

    let extreme_binning_cases: [(u32, u32); 5] = [
        (0, 0),
        (1, 1),
        (u32::MAX, u32::MAX),
        (1, u32::MAX),
        (u32::MAX, 1),
    ];

    for (x, y) in extreme_binning_cases {
        let params = TelescopeParams {
            exposure_time: 1.0,
            temperature: None,
            gain: None,
            binning: Some((x, y)),
            filter: None,
        };
        let keywords = ImageWriter::build_telescope_keywords(&params);

        assert_eq!(keywords.len(), 3);
        assert_eq!(keywords[1].name, "XBINNING");
        assert_eq!(keywords[1].value, Some(KeywordValue::Integer(x as i64)));
        assert_eq!(keywords[2].name, "YBINNING");
        assert_eq!(keywords[2].value, Some(KeywordValue::Integer(y as i64)));
    }
}

#[test]
fn telescope_keywords_filter_names() {
    use crate::fits::header::KeywordValue;

    let filter_names = ["Ha", "V", "R", "test"];

    for filter_name in filter_names {
        let params = TelescopeParams {
            exposure_time: 1.0,
            temperature: None,
            gain: None,
            binning: None,
            filter: Some(filter_name.to_string()),
        };
        let keywords = ImageWriter::build_telescope_keywords(&params);

        assert_eq!(keywords.len(), 2);
        assert_eq!(keywords[1].name, "FILTER");
        assert_eq!(
            keywords[1].value,
            Some(KeywordValue::String(filter_name.to_string()))
        );
    }
}

#[test]
fn image_format_detect_error_cases() {
    let empty_data: Vec<u8> = vec![];
    let mut cursor = Cursor::new(empty_data);
    let result = ImageFormat::detect(&mut cursor);
    assert!(result.is_err());

    let short_data = vec![0x00, 0x01];
    let mut short_cursor = Cursor::new(short_data);
    let result = ImageFormat::detect(&mut short_cursor);
    assert!(result.is_err());
}

#[test]
fn image_format_extension() {
    assert_eq!(ImageFormat::Fits.extension(), "fits");
    assert_eq!(ImageFormat::Xisf.extension(), "xisf");
}

#[test]
fn image_format_from_magic_bytes_xml() {
    let xml_magic = b"<?xml version=\"1.0\"?>";
    assert_eq!(
        ImageFormat::from_magic_bytes(xml_magic),
        Some(ImageFormat::Xisf)
    );
}

#[test]
fn image_writer_new() {
    use std::path::Path;
    let path = Path::new("/tmp/test.fits");
    let writer = ImageWriter::new(path, ImageFormat::Fits);
    assert_eq!(writer.format, ImageFormat::Fits);
    assert_eq!(writer.path, path);
}

#[test]
fn image_writer_write_telescope_image() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let path = dir.path().join("telescope.fits");

    let writer = ImageWriter::new(&path, ImageFormat::Fits);
    let data = vec![100i16, 200, 300, 400];
    let info = ImageInfo::new(vec![2, 2], BitPix::I16);
    let params = TelescopeParams {
        exposure_time: 60.0,
        temperature: Some(-20.0),
        gain: Some(200.0),
        binning: Some((1, 1)),
        filter: Some("R".to_string()),
    };

    let result = writer.write_telescope_image(&data, &info, &params);
    assert!(result.is_ok());
}

#[test]
fn image_writer_write_image_xisf_unsupported() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let path = dir.path().join("test.xisf");

    let writer = ImageWriter::new(&path, ImageFormat::Xisf);
    let data = vec![1u8, 2, 3, 4];
    let info = ImageInfo::new(vec![2, 2], BitPix::U8);

    let result = writer.write_image(&data, &info, &[]);
    assert!(result.is_err());
}

#[test]
fn telescope_params() {
    let params = TelescopeParams {
        exposure_time: 30.0,
        temperature: Some(-15.0),
        gain: None,
        binning: Some((2, 2)),
        filter: Some("Ha".to_string()),
    };

    let debug_str = format!("{:?}", params);
    assert!(debug_str.contains("TelescopeParams"));
    assert!(debug_str.contains("exposure_time: 30.0"));
}

#[test]
fn image_format_and_partial_eq() {
    assert_eq!(ImageFormat::Fits, ImageFormat::Fits);
    assert_ne!(ImageFormat::Fits, ImageFormat::Xisf);

    let debug_str = format!("{:?}", ImageFormat::Fits);
    assert!(debug_str.contains("Fits"));
}

#[test]
fn image_info_and_clone() {
    let info = ImageInfo::new(vec![100, 100], BitPix::F32);
    let cloned = info.clone();

    assert_eq!(info.dimensions, cloned.dimensions);
    assert_eq!(info.bitpix, cloned.bitpix);

    let debug_str = format!("{:?}", info);
    assert!(debug_str.contains("ImageInfo"));
}

#[test]
fn image_info_signed_types() {
    let info_i16 = ImageInfo::new(vec![10], BitPix::I16);
    assert!(info_i16.is_signed);

    let info_i32 = ImageInfo::new(vec![10], BitPix::I32);
    assert!(info_i32.is_signed);

    let info_i64 = ImageInfo::new(vec![10], BitPix::I64);
    assert!(info_i64.is_signed);

    let info_u8 = ImageInfo::new(vec![10], BitPix::U8);
    assert!(!info_u8.is_signed);

    let info_f32 = ImageInfo::new(vec![10], BitPix::F32);
    assert!(!info_f32.is_signed);

    let info_f64 = ImageInfo::new(vec![10], BitPix::F64);
    assert!(!info_f64.is_signed);
}

#[test]
fn image_writer_write_fits_with_keywords() {
    use tempfile::tempdir;
    let dir = tempdir().unwrap();
    let path = dir.path().join("test_keywords.fits");

    let writer = ImageWriter::new(&path, ImageFormat::Fits);
    let data = vec![1u8, 2, 3, 4];
    let info = ImageInfo::new(vec![2, 2], BitPix::U8);
    let keywords = vec![
        Keyword::string("OBJECT", "M31"),
        Keyword::real("EXPTIME", 30.0).with_comment("Exposure time in seconds"),
    ];

    let result = writer.write_image(&data, &info, &keywords);
    assert!(result.is_ok());
}

// ==================== AstroImage tests ====================

// Note: AstroImage requires types that implement both DataArray (FITS) and XisfDataType (XISF).
// Common types: u8, f32, f64. FITS-only: i16, i32, i64. XISF-only: u16, u32.

#[test]
fn astro_image_write_fits() {
    use tempfile::NamedTempFile;

    let temp_file = NamedTempFile::with_suffix(".fits").unwrap();
    let data: Vec<f32> = (0..100).map(|i| i as f32).collect();

    let result = AstroImage::new(&data, [10, 10])
        .compressed(false)
        .write_fits(temp_file.path());

    assert!(result.is_ok());
}

#[test]
fn astro_image_write_xisf() {
    use tempfile::NamedTempFile;

    let temp_file = NamedTempFile::with_suffix(".xisf").unwrap();
    let data: Vec<f32> = (0..100).map(|i| i as f32).collect();

    let result = AstroImage::new(&data, [10, 10]).write_xisf(temp_file.path());

    assert!(result.is_ok());
}

#[test]
fn astro_image_write_to_by_extension() {
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let data: Vec<f32> = (0..100).map(|i| i as f32).collect();

    // Write to FITS by extension
    let fits_path = dir.path().join("test.fits");
    let result = AstroImage::new(&data, [10, 10])
        .compressed(false)
        .write_to(&fits_path);
    assert!(result.is_ok());

    // Write to XISF by extension
    let xisf_path = dir.path().join("test.xisf");
    let result = AstroImage::new(&data, [10, 10]).write_to(&xisf_path);
    assert!(result.is_ok());
}

#[test]
fn astro_image_with_keywords() {
    use tempfile::NamedTempFile;

    let temp_file = NamedTempFile::with_suffix(".fits").unwrap();
    let data: Vec<f32> = (0..100).map(|i| i as f32).collect();

    let result = AstroImage::new(&data, [10, 10])
        .keyword(Keyword::string("OBJECT", "M31"))
        .keyword(Keyword::real("EXPTIME", 30.0).with_comment("Exposure time"))
        .keyword(Keyword::integer("GAIN", 100))
        .keyword(Keyword::logical("PREVIEW", true))
        .compressed(false)
        .write_fits(temp_file.path());

    assert!(result.is_ok());

    // Verify the keywords were written
    let mut fits = crate::fits::FitsFile::open(temp_file.path()).unwrap();
    let header = fits.get_header(0).unwrap();

    assert!(header.get_keyword_value("OBJECT").is_some());
    assert!(header.get_keyword_value("EXPTIME").is_some());
    assert!(header.get_keyword_value("GAIN").is_some());
    assert!(header.get_keyword_value("PREVIEW").is_some());
}

#[test]
fn astro_image_with_wcs() {
    use cosmos_wcs::{Projection, WcsBuilder};
    use tempfile::NamedTempFile;

    let temp_file = NamedTempFile::with_suffix(".fits").unwrap();
    let data: Vec<f32> = (0..100).map(|i| i as f32).collect();

    let wcs = WcsBuilder::new()
        .crpix(5.0, 5.0)
        .crval(180.0, 45.0)
        .cd_matrix([[0.001, 0.0], [0.0, 0.001]])
        .projection(Projection::tan())
        .build()
        .unwrap();

    let result = AstroImage::new(&data, [10, 10])
        .wcs(&wcs)
        .compressed(false)
        .write_fits(temp_file.path());

    assert!(result.is_ok());

    // Verify WCS keywords were written
    let mut fits = crate::fits::FitsFile::open(temp_file.path()).unwrap();
    let header = fits.get_header(0).unwrap();

    assert!(header.get_keyword_value("CTYPE1").is_some());
    assert!(header.get_keyword_value("CRPIX1").is_some());
    assert!(header.get_keyword_value("CRVAL1").is_some());
    assert!(header.get_keyword_value("CD1_1").is_some());
}

#[test]
fn astro_image_xisf_with_keywords() {
    use tempfile::NamedTempFile;

    let temp_file = NamedTempFile::with_suffix(".xisf").unwrap();
    let data: Vec<f32> = (0..100).map(|i| i as f32).collect();

    let result = AstroImage::new(&data, [10, 10])
        .keyword(Keyword::string("OBJECT", "M42"))
        .keyword(Keyword::real("EXPTIME", 60.0))
        .write_xisf(temp_file.path());

    assert!(result.is_ok());

    // Verify the keywords were written
    let reader = crate::xisf::XisfFile::open(temp_file.path()).unwrap();
    let keywords = reader.keywords();

    assert!(keywords.iter().any(
        |k| k.name == "OBJECT" && k.value == Some(KeywordValue::String("M42".to_string()))
    ));
    assert!(keywords.iter().any(|k| k.name == "EXPTIME"));
}

#[test]
fn astro_image_rgb() {
    use tempfile::NamedTempFile;

    let temp_file = NamedTempFile::with_suffix(".xisf").unwrap();
    let data: Vec<f32> = (0..300).map(|i| i as f32).collect();

    let image = AstroImage::new(&data, [10, 10, 3]);
    assert_eq!(image.image_kind(), ImageKind::Rgb);

    let result = image.write_xisf(temp_file.path());
    assert!(result.is_ok());
}

#[test]
fn astro_image_u8() {
    use tempfile::NamedTempFile;

    // u8 works for both FITS and XISF
    let temp_file = NamedTempFile::with_suffix(".fits").unwrap();
    let data: Vec<u8> = (0..100).collect();

    let result = AstroImage::new(&data, [10, 10])
        .compressed(false)
        .write_fits(temp_file.path());
    assert!(result.is_ok());

    let temp_file = NamedTempFile::with_suffix(".xisf").unwrap();
    let result = AstroImage::new(&data, [10, 10]).write_xisf(temp_file.path());
    assert!(result.is_ok());
}

#[test]
fn image_kind_detection() {
    assert_eq!(ImageKind::from_dimensions(&[100]), ImageKind::Mono);
    assert_eq!(ImageKind::from_dimensions(&[100, 100]), ImageKind::Mono);
    assert_eq!(ImageKind::from_dimensions(&[100, 100, 3]), ImageKind::Rgb);
    assert_eq!(ImageKind::from_dimensions(&[100, 100, 4]), ImageKind::Cube);
    assert_eq!(ImageKind::from_dimensions(&[100, 100, 10]), ImageKind::Cube);
}

#[test]
fn keyword_types() {
    let real = Keyword::real("EXPTIME", 30.0);
    assert!(matches!(real.value, Some(KeywordValue::Real(_))));

    let int = Keyword::integer("GAIN", 100);
    assert!(matches!(int.value, Some(KeywordValue::Integer(100))));

    let string = Keyword::string("OBJECT", "M31");
    if let Some(KeywordValue::String(s)) = &string.value {
        assert_eq!(s, "M31");
    } else {
        panic!("Expected String variant");
    }

    let boolean = Keyword::logical("PREVIEW", true);
    assert!(matches!(boolean.value, Some(KeywordValue::Logical(true))));

    let with_comment = Keyword::real("TEMP", -15.0).with_comment("CCD temperature");
    assert_eq!(with_comment.comment, Some("CCD temperature".to_string()));
}

#[test]
fn astro_image_unknown_extension_error() {
    use tempfile::tempdir;

    let dir = tempdir().unwrap();
    let data: Vec<f32> = (0..100).map(|i| i as f32).collect();

    let bad_path = dir.path().join("test.unknown");
    let result = AstroImage::new(&data, [10, 10]).write_to(&bad_path);

    assert!(result.is_err());
}

#[cfg(feature = "standard-formats")]
#[test]
fn png_roundtrip_u8_grayscale() {
    use tempfile::NamedTempFile;

    let temp_file = NamedTempFile::with_suffix(".png").unwrap();
    let original_data: Vec<u8> = (0..100).collect();
    let img = Image::new(PixelData::U8(original_data.clone()), vec![10, 10]);

    img.save(temp_file.path()).unwrap();
    let loaded = Image::open(temp_file.path()).unwrap();

    assert_eq!(loaded.dimensions, vec![10, 10]);
    assert_eq!(loaded.pixels.as_u8().unwrap(), &original_data);
}

#[cfg(feature = "standard-formats")]
#[test]
fn png_roundtrip_u16_grayscale() {
    use tempfile::NamedTempFile;

    let temp_file = NamedTempFile::with_suffix(".png").unwrap();
    let original_data: Vec<u16> = (0..100).map(|i| i * 100).collect();
    let img = Image::new(PixelData::U16(original_data.clone()), vec![10, 10]);

    img.save(temp_file.path()).unwrap();
    let loaded = Image::open(temp_file.path()).unwrap();

    assert_eq!(loaded.dimensions, vec![10, 10]);
    assert_eq!(loaded.pixels.as_u16().unwrap(), &original_data);
}

#[cfg(feature = "standard-formats")]
#[test]
fn png_roundtrip_u8_rgb() {
    use tempfile::NamedTempFile;

    let temp_file = NamedTempFile::with_suffix(".png").unwrap();
    let original_data: Vec<u8> = (0..75).collect();
    let img = Image::new(PixelData::U8(original_data.clone()), vec![5, 5, 3]);

    img.save(temp_file.path()).unwrap();
    let loaded = Image::open(temp_file.path()).unwrap();

    assert_eq!(loaded.dimensions, vec![5, 5, 3]);
    assert_eq!(loaded.pixels.as_u8().unwrap(), &original_data);
}

#[cfg(feature = "standard-formats")]
#[test]
fn tiff_roundtrip_u8_grayscale() {
    use tempfile::NamedTempFile;

    let temp_file = NamedTempFile::with_suffix(".tiff").unwrap();
    let original_data: Vec<u8> = (0..100).collect();
    let img = Image::new(PixelData::U8(original_data.clone()), vec![10, 10]);

    img.save(temp_file.path()).unwrap();
    let loaded = Image::open(temp_file.path()).unwrap();

    assert_eq!(loaded.dimensions, vec![10, 10]);
    assert_eq!(loaded.pixels.as_u8().unwrap(), &original_data);
}

#[cfg(feature = "standard-formats")]
#[test]
fn tiff_roundtrip_u16_grayscale() {
    use tempfile::NamedTempFile;

    let temp_file = NamedTempFile::with_suffix(".tiff").unwrap();
    let original_data: Vec<u16> = (0..100).map(|i| i * 100).collect();
    let img = Image::new(PixelData::U16(original_data.clone()), vec![10, 10]);

    img.save(temp_file.path()).unwrap();
    let loaded = Image::open(temp_file.path()).unwrap();

    assert_eq!(loaded.dimensions, vec![10, 10]);
    assert_eq!(loaded.pixels.as_u16().unwrap(), &original_data);
}

#[cfg(feature = "standard-formats")]
#[test]
fn tiff_roundtrip_f32_grayscale() {
    use tempfile::NamedTempFile;

    let temp_file = NamedTempFile::with_suffix(".tiff").unwrap();
    let original_data: Vec<f32> = (0..100).map(|i| i as f32 * 0.01).collect();
    let img = Image::new(PixelData::F32(original_data.clone()), vec![10, 10]);

    img.save(temp_file.path()).unwrap();
    let loaded = Image::open(temp_file.path()).unwrap();

    assert_eq!(loaded.dimensions, vec![10, 10]);
    let loaded_data = loaded.pixels.as_f32().unwrap();
    for (a, b) in original_data.iter().zip(loaded_data.iter()) {
        assert!((a - b).abs() < 1e-6);
    }
}

#[cfg(feature = "standard-formats")]
#[test]
fn image_format_png_tiff_extension() {
    assert_eq!(ImageFormat::from_extension("png"), Some(ImageFormat::Png));
    assert_eq!(ImageFormat::from_extension("PNG"), Some(ImageFormat::Png));
    assert_eq!(ImageFormat::from_extension("tiff"), Some(ImageFormat::Tiff));
    assert_eq!(ImageFormat::from_extension("tif"), Some(ImageFormat::Tiff));
    assert_eq!(ImageFormat::from_extension("TIFF"), Some(ImageFormat::Tiff));
}

#[cfg(feature = "standard-formats")]
#[test]
fn image_format_png_tiff_magic_bytes() {
    let png_magic = b"\x89PNG\r\n\x1a\n";
    let tiff_le_magic = b"II*\0";
    let tiff_be_magic = b"MM\0*";

    assert_eq!(
        ImageFormat::from_magic_bytes(png_magic),
        Some(ImageFormat::Png)
    );
    assert_eq!(
        ImageFormat::from_magic_bytes(tiff_le_magic),
        Some(ImageFormat::Tiff)
    );
    assert_eq!(
        ImageFormat::from_magic_bytes(tiff_be_magic),
        Some(ImageFormat::Tiff)
    );
}

#[cfg(feature = "standard-formats")]
#[test]
fn image_format_png_tiff_extension_method() {
    assert_eq!(ImageFormat::Png.extension(), "png");
    assert_eq!(ImageFormat::Tiff.extension(), "tiff");
}

#[test]
fn interleaved_to_planar_u8() {
    // 2x2 RGB image: [R0,G0,B0, R1,G1,B1, R2,G2,B2, R3,G3,B3]
    let interleaved: Vec<u8> = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    let mut img = Image::new(PixelData::U8(interleaved), vec![2, 2, 3]);

    img.interleaved_to_planar();

    // Expected planar: [R0,R1,R2,R3, G0,G1,G2,G3, B0,B1,B2,B3]
    let expected: Vec<u8> = vec![1, 4, 7, 10, 2, 5, 8, 11, 3, 6, 9, 12];
    assert_eq!(img.pixels.as_u8().unwrap(), &expected);
}

#[test]
fn planar_to_interleaved_u8() {
    // 2x2 RGB planar: [R0,R1,R2,R3, G0,G1,G2,G3, B0,B1,B2,B3]
    let planar: Vec<u8> = vec![1, 4, 7, 10, 2, 5, 8, 11, 3, 6, 9, 12];
    let mut img = Image::new(PixelData::U8(planar), vec![2, 2, 3]);

    img.planar_to_interleaved();

    // Expected interleaved: [R0,G0,B0, R1,G1,B1, R2,G2,B2, R3,G3,B3]
    let expected: Vec<u8> = vec![1, 2, 3, 4, 5, 6, 7, 8, 9, 10, 11, 12];
    assert_eq!(img.pixels.as_u8().unwrap(), &expected);
}

#[test]
fn interleaved_planar_roundtrip() {
    let original: Vec<u16> = vec![100, 200, 300, 400, 500, 600, 700, 800, 900];
    let mut img = Image::new(PixelData::U16(original.clone()), vec![3, 1, 3]);

    img.interleaved_to_planar();
    img.planar_to_interleaved();

    assert_eq!(img.pixels.as_u16().unwrap(), &original);
}

#[test]
fn interleaved_to_planar_non_rgb_unchanged() {
    let mono: Vec<u8> = vec![1, 2, 3, 4];
    let mut img = Image::new(PixelData::U8(mono.clone()), vec![2, 2]);

    img.interleaved_to_planar();

    assert_eq!(img.pixels.as_u8().unwrap(), &mono);
}

#[test]
fn planar_to_interleaved_non_rgb_unchanged() {
    let mono: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0];
    let mut img = Image::new(PixelData::F32(mono.clone()), vec![2, 2]);

    img.planar_to_interleaved();

    assert_eq!(img.pixels.as_f32().unwrap(), &mono);
}

#[test]
fn interleaved_to_planar_f32() {
    let interleaved: Vec<f32> = vec![1.0, 2.0, 3.0, 4.0, 5.0, 6.0];
    let mut img = Image::new(PixelData::F32(interleaved), vec![2, 1, 3]);

    img.interleaved_to_planar();

    let expected: Vec<f32> = vec![1.0, 4.0, 2.0, 5.0, 3.0, 6.0];
    assert_eq!(img.pixels.as_f32().unwrap(), &expected);
}

#[test]
fn interleaved_to_planar_i16() {
    let interleaved: Vec<i16> = vec![-1, -2, -3, 4, 5, 6];
    let mut img = Image::new(PixelData::I16(interleaved), vec![2, 1, 3]);

    img.interleaved_to_planar();

    let expected: Vec<i16> = vec![-1, 4, -2, 5, -3, 6];
    assert_eq!(img.pixels.as_i16().unwrap(), &expected);
}

#[test]
fn normalize_f32() {
    let data: Vec<f32> = vec![10.0, 20.0, 30.0, 40.0];
    let mut img = Image::new(PixelData::F32(data), vec![2, 2]);

    img.normalize();

    let result = img.pixels.as_f32().unwrap();
    assert_eq!(result[0], 0.0);
    assert_eq!(result[3], 1.0);
    assert!((result[1] - 1.0 / 3.0).abs() < 1e-6);
    assert!((result[2] - 2.0 / 3.0).abs() < 1e-6);
}

#[test]
fn normalize_to_f32_converts_type() {
    let data: Vec<u16> = vec![0, 100, 200, 300];
    let mut img = Image::new(PixelData::U16(data), vec![2, 2]);

    img.normalize_to_f32();

    let result = img.pixels.as_f32().unwrap();
    assert_eq!(result[0], 0.0);
    assert_eq!(result[3], 1.0);
}

#[test]
fn normalize_constant_image() {
    let data: Vec<f32> = vec![5.0, 5.0, 5.0, 5.0];
    let mut img = Image::new(PixelData::F32(data), vec![2, 2]);

    img.normalize();

    let result = img.pixels.as_f32().unwrap();
    for &v in result {
        assert!(v.is_finite());
    }
}

#[test]
fn pixel_range_f32() {
    let data: Vec<f32> = vec![-5.0, 0.0, 10.0, 100.0];
    let img = Image::new(PixelData::F32(data), vec![2, 2]);

    let (min, max) = img.pixel_range();
    assert_eq!(min, -5.0);
    assert_eq!(max, 100.0);
}

#[test]
fn image_debayer_u8() {
    use crate::debayer::BayerPattern;

    let raw: Vec<u8> = vec![100, 50, 60, 200];
    let mut img = Image::new(PixelData::U8(raw), vec![2, 2]);

    img.debayer(BayerPattern::Rggb);

    assert_eq!(img.dimensions, vec![2, 2, 3]);
    assert_eq!(img.channels(), 3);
    assert!(img.is_rgb());

    let pixels = img.pixels.as_u8().unwrap();
    assert_eq!(pixels.len(), 12);
    assert_eq!(pixels[0], 100); // R at (0,0) preserved
    assert_eq!(pixels[11], 200); // B at (1,1) preserved
}

#[test]
fn image_debayer_u16() {
    use crate::debayer::BayerPattern;

    let raw: Vec<u16> = vec![1000, 500, 600, 2000];
    let mut img = Image::new(PixelData::U16(raw), vec![2, 2]);

    img.debayer(BayerPattern::Rggb);

    assert_eq!(img.dimensions, vec![2, 2, 3]);
    let pixels = img.pixels.as_u16().unwrap();
    assert_eq!(pixels[0], 1000); // R at (0,0)
    assert_eq!(pixels[11], 2000); // B at (1,1)
}

#[test]
fn image_debayer_ignores_rgb() {
    use crate::debayer::BayerPattern;

    let rgb: Vec<u8> = vec![1, 2, 3, 4, 5, 6];
    let mut img = Image::new(PixelData::U8(rgb.clone()), vec![2, 1, 3]);

    img.debayer(BayerPattern::Rggb);

    // Should be unchanged
    assert_eq!(img.dimensions, vec![2, 1, 3]);
    assert_eq!(img.pixels.as_u8().unwrap(), &rgb);
}
