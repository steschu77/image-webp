// WEBP decompression API.
use super::vp8::Vp8Decoder;
use std::io::{self};
use std::ops::Range;

#[derive(Debug)]
pub enum Error {
    IoError(io::Error),
    InvalidSignature,
    ChunkHeaderInvalid([u8; 4]),
    ChunkMissing,
    ReservedBitSet,
    InvalidAlphaPreprocessing,
    InvalidCompressionMethod,
    AlphaChunkSizeMismatch,
    ImageTooLarge,
    FrameOutsideImage,
    LosslessUnsupported,
    ExtendedUnsupported,
    VersionNumberInvalid(u8),
    InvalidColorCacheBits(u8),
    HuffmanError,
    BitStreamError,
    TransformError,
    BufferUnderrun,
    Vp8MagicInvalid([u8; 3]),
    InvalidImageSize,
    NotEnoughInitData,
    ColorSpaceInvalid(u8),
    LumaPredictionModeInvalid,
    IntraPredictionModeInvalid,
    ChromaPredictionModeInvalid,
    NonKeyframe,
    InvalidParameter,
    MemoryLimitExceeded,
    InvalidChunkSize,
    NoMoreFrames,
}

// ----------------------------------------------------------------------------
impl std::fmt::Display for Error {
    fn fmt(&self, f: &mut std::fmt::Formatter) -> std::fmt::Result {
        let err = format!("{:?}", self);
        f.write_str(&err)
    }
}

// ----------------------------------------------------------------------------
impl std::error::Error for Error {}

// ----------------------------------------------------------------------------
impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::IoError(err)
    }
}

// ----------------------------------------------------------------------------
impl From<std::array::TryFromSliceError> for Error {
    fn from(_: std::array::TryFromSliceError) -> Self {
        Error::BufferUnderrun
    }
}

// ----------------------------------------------------------------------------
pub type Result<T> = std::result::Result<T, Error>;

// ----------------------------------------------------------------------------
pub struct WebPDecoder {
    width: usize,
    height: usize,
    data: Vec<u8>,
}

// ----------------------------------------------------------------------------
impl WebPDecoder {
    pub fn new(data: Vec<u8>) -> Result<Self> {
        let (width, height, range) = Self::read_chunks(&data)?;
        Ok(Self {
            width,
            height,
            data: data[range].to_vec(),
        })
    }

    fn read_vp8_chunk(chunk: &[u8], range: Range<usize>) -> Result<(usize, usize, Range<usize>)> {
        if chunk[0] & 1 != 0 {
            return Err(Error::NonKeyframe);
        }

        let tag = chunk[3..6].try_into()?;
        if tag != [0x9d, 0x01, 0x2a] {
            return Err(Error::Vp8MagicInvalid(tag));
        }

        let width = (u16::from_le_bytes(chunk[6..8].try_into()?) & 0x3fff) as usize;
        let height = (u16::from_le_bytes(chunk[8..10].try_into()?) & 0x3fff) as usize;
        if width == 0 || height == 0 {
            return Err(Error::InvalidImageSize);
        }

        Ok((width, height, range))
    }

    fn read_chunks(data: &[u8]) -> Result<(usize, usize, Range<usize>)> {
        if &data[0..4] != b"RIFF" || &data[8..12] != b"WEBP" {
            return Err(Error::InvalidSignature);
        }

        let chunk = &data[12..];
        let chunk_fcc = chunk[0..4].try_into()?;
        let chunk_size = u32::from_le_bytes(chunk[4..8].try_into()?) as usize;
        let range = 20..20 + chunk_size;

        match &chunk_fcc {
            b"VP8 " => Self::read_vp8_chunk(&chunk[8..], range),
            b"VP8L" => Err(Error::LosslessUnsupported),
            b"VP8X" => Err(Error::ExtendedUnsupported),
            _ => Err(Error::ChunkHeaderInvalid(chunk_fcc)),
        }
    }

    pub fn dimensions(&self) -> (usize, usize) {
        (self.width, self.height)
    }

    /// Returns the number of bytes required to store the image or a single frame, or None if that
    /// would take more than `usize::MAX` bytes.
    pub fn output_buffer_size(&self) -> Option<usize> {
        let bytes_per_pixel = 3;
        (self.width)
            .checked_mul(self.height)?
            .checked_mul(bytes_per_pixel)
    }

    /// Returns the raw bytes of the image.
    /// Fails with `ImageTooLarge` if `buf` has length different than `output_buffer_size()`
    pub fn read_image(&mut self, buf: &mut [u8]) -> Result<()> {
        if Some(buf.len()) != self.output_buffer_size() {
            return Err(Error::ImageTooLarge);
        }

        let decoder = Vp8Decoder::new();
        let frame = decoder.decode_frame(&self.data)?;

        frame.fill_rgb(buf);

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    const RGB_BPP: usize = 3;

    #[test]
    fn add_with_overflow_size() {
        let bytes = vec![
            0x52, 0x49, 0x46, 0x46, 0xaf, 0x37, 0x80, 0x47, 0x57, 0x45, 0x42, 0x50, 0x6c, 0x64,
            0x00, 0x00, 0xff, 0xff, 0xff, 0xff, 0xfb, 0x7e, 0x73, 0x00, 0x06, 0x00, 0x00, 0x00,
            0x00, 0x00, 0x00, 0x00, 0x05, 0x00, 0x00, 0x00, 0x65, 0x65, 0x65, 0x65, 0x65, 0x65,
            0x40, 0xfb, 0xff, 0xff, 0x65, 0x65, 0x65, 0x65, 0x65, 0x65, 0x65, 0x65, 0x65, 0x65,
            0x00, 0x00, 0x00, 0x00, 0x62, 0x00, 0x10, 0x00, 0x00, 0x00, 0x00, 0x00, 0x00, 0x49,
            0x49, 0x54, 0x55, 0x50, 0x4c, 0x54, 0x59, 0x50, 0x45, 0x33, 0x37, 0x44, 0x4d, 0x46,
        ];

        let _ = WebPDecoder::new(bytes);
    }

    #[test]
    fn decode_2x2_single_color_image() {
        // Image data created from imagemagick and output of xxd:
        // $ convert -size 2x2 xc:#f00 red.webp
        // $ xxd -g 1 red.webp | head

        const NUM_PIXELS: usize = 2 * 2 * RGB_BPP;
        // 2x2 red pixel image
        let bytes = vec![
            0x52, 0x49, 0x46, 0x46, 0x3c, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50,
            0x38, 0x20, 0x30, 0x00, 0x00, 0x00, 0xd0, 0x01, 0x00, 0x9d, 0x01, 0x2a, 0x02, 0x00,
            0x02, 0x00, 0x02, 0x00, 0x34, 0x25, 0xa0, 0x02, 0x74, 0xba, 0x01, 0xf8, 0x00, 0x03,
            0xb0, 0x00, 0xfe, 0xf0, 0xc4, 0x0b, 0xff, 0x20, 0xb9, 0x61, 0x75, 0xc8, 0xd7, 0xff,
            0x20, 0x3f, 0xe4, 0x07, 0xfc, 0x80, 0xff, 0xf8, 0xf2, 0x00, 0x00, 0x00,
        ];

        let mut data = [0; NUM_PIXELS];
        let mut decoder = WebPDecoder::new(bytes).unwrap();
        decoder.read_image(&mut data).unwrap();

        // All pixels are the same value
        let first_pixel = &data[..RGB_BPP];
        assert!(data.chunks_exact(3).all(|ch| ch.iter().eq(first_pixel)));
    }

    #[test]
    fn decode_3x3_single_color_image() {
        // Test that any odd pixel "tail" is decoded properly

        const NUM_PIXELS: usize = 3 * 3 * RGB_BPP;
        // 3x3 red pixel image
        let bytes = vec![
            0x52, 0x49, 0x46, 0x46, 0x3c, 0x00, 0x00, 0x00, 0x57, 0x45, 0x42, 0x50, 0x56, 0x50,
            0x38, 0x20, 0x30, 0x00, 0x00, 0x00, 0xd0, 0x01, 0x00, 0x9d, 0x01, 0x2a, 0x03, 0x00,
            0x03, 0x00, 0x02, 0x00, 0x34, 0x25, 0xa0, 0x02, 0x74, 0xba, 0x01, 0xf8, 0x00, 0x03,
            0xb0, 0x00, 0xfe, 0xf0, 0xc4, 0x0b, 0xff, 0x20, 0xb9, 0x61, 0x75, 0xc8, 0xd7, 0xff,
            0x20, 0x3f, 0xe4, 0x07, 0xfc, 0x80, 0xff, 0xf8, 0xf2, 0x00, 0x00, 0x00,
        ];

        let mut data = [0; NUM_PIXELS];
        let mut decoder = WebPDecoder::new(bytes).unwrap();
        decoder.read_image(&mut data).unwrap();

        // All pixels are the same value
        let first_pixel = &data[..RGB_BPP];
        assert!(data.chunks_exact(3).all(|ch| ch.iter().eq(first_pixel)));
    }
}
