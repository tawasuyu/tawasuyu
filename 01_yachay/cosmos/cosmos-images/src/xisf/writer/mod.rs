use crate::fits::header::{Keyword, KeywordValue};
use crate::xisf::header::{
    format_geometry_with_channels, ColorSpace, DataLocation, ImageInfo, PixelStorage, SampleFormat,
    XisfCompression, XisfProperty, XisfPropertyValue,
};
use crate::xisf::{Result, XisfError};
use byteorder::{LittleEndian, WriteBytesExt};
use quick_xml::events::{BytesDecl, BytesEnd, BytesStart, Event};
use quick_xml::Writer;
use std::fs::File;
use std::io::{BufWriter, Cursor, Seek, Write};
use std::path::Path;

const XISF_SIGNATURE: &[u8] = b"XISF0100";
const HEADER_ALIGNMENT: usize = 16;

pub struct XisfWriter<W> {
    writer: W,
    images: Vec<ImageData>,
    keywords: Vec<Keyword>,
    properties: Vec<XisfProperty>,
    compression: XisfCompression,
    property_blocks: Vec<PropertyDataBlock>,
}

struct ImageData {
    info: ImageInfo,
    data: Vec<u8>,
}

struct PropertyDataBlock {
    property_index: usize,
    data: Vec<u8>,
    location: DataLocation,
}

mod datatype;
#[cfg(test)]
mod tests;
mod wcs;
mod write;

pub use datatype::*;
pub use wcs::*;
// Las free-fns de `write` (build_geometry, pad_to_alignment, align_to) sólo las
// consumen los tests; los métodos públicos de XisfWriter llegan vía el tipo.
#[cfg(test)]
pub(crate) use write::*;
