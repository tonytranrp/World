//! An 8-bit RGBA image and its PNG encoding.

use std::fmt;
use std::path::Path;

/// An 8-bit sRGB RGBA image, row-major from the top row, 4 bytes per pixel.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Image {
    width: u32,
    height: u32,
    rgba: Vec<u8>,
}

/// Why an image could not be built or written.
#[derive(Debug)]
pub enum OutputError {
    /// The pixel data is not `width * height * 4` bytes.
    SizeMismatch {
        width: u32,
        height: u32,
        len: usize,
    },
    Png(png::EncodingError),
    Io(std::io::Error),
}

impl fmt::Display for OutputError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::SizeMismatch { width, height, len } => {
                write!(f, "{len} bytes is not a {width}x{height} RGBA image")
            }
            Self::Png(e) => write!(f, "PNG encoding failed: {e}"),
            Self::Io(e) => write!(f, "writing the image failed: {e}"),
        }
    }
}

impl std::error::Error for OutputError {}

impl Image {
    pub fn new(width: u32, height: u32, rgba: Vec<u8>) -> Result<Self, OutputError> {
        if rgba.len() as u64 != u64::from(width) * u64::from(height) * 4 {
            return Err(OutputError::SizeMismatch {
                width,
                height,
                len: rgba.len(),
            });
        }
        Ok(Self {
            width,
            height,
            rgba,
        })
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    /// The pixel bytes, row-major, RGBA.
    pub fn rgba(&self) -> &[u8] {
        &self.rgba
    }

    pub fn into_rgba(self) -> Vec<u8> {
        self.rgba
    }

    /// Pixel `(x, y)`; `None` outside the image.
    pub fn pixel(&self, x: u32, y: u32) -> Option<[u8; 4]> {
        if x >= self.width || y >= self.height {
            return None;
        }
        let i = ((y as usize) * (self.width as usize) + x as usize) * 4;
        Some([
            self.rgba[i],
            self.rgba[i + 1],
            self.rgba[i + 2],
            self.rgba[i + 3],
        ])
    }

    /// The image as PNG bytes: 8-bit RGBA, the encoder's default compression
    /// and filter. For a given `png` crate version the bytes are always the
    /// same for the same pixels.
    pub fn encode_png(&self) -> Result<Vec<u8>, OutputError> {
        let mut bytes = Vec::new();
        {
            let mut encoder = png::Encoder::new(&mut bytes, self.width, self.height);
            encoder.set_color(png::ColorType::Rgba);
            encoder.set_depth(png::BitDepth::Eight);
            let mut writer = encoder.write_header().map_err(OutputError::Png)?;
            writer
                .write_image_data(&self.rgba)
                .map_err(OutputError::Png)?;
            writer.finish().map_err(OutputError::Png)?;
        }
        Ok(bytes)
    }

    /// Writes the PNG to `path`, creating its folder.
    pub fn write_png(&self, path: &Path) -> Result<(), OutputError> {
        let bytes = self.encode_png()?;
        if let Some(dir) = path.parent() {
            std::fs::create_dir_all(dir).map_err(OutputError::Io)?;
        }
        std::fs::write(path, bytes).map_err(OutputError::Io)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn png_encoding_is_deterministic_and_has_the_signature() {
        let img = Image::new(3, 2, (0..24).collect()).unwrap();
        let a = img.encode_png().unwrap();
        assert_eq!(&a[..8], b"\x89PNG\r\n\x1a\n");
        assert_eq!(a, img.encode_png().unwrap());
        assert_eq!(img.pixel(1, 0), Some([4, 5, 6, 7]));
        assert_eq!(img.pixel(3, 0), None);
        assert!(Image::new(3, 2, vec![0; 5]).is_err());
    }
}
