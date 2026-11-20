//! Byte compression for stored chunks (decision 0006): LZ4 for the warm
//! in-RAM tier, where speed matters most, and zstd at its fastest level for
//! region files on disk, where size does. Both are pure, safe Rust.
//!
//! Decompression always knows the exact output length from its container and
//! refuses anything longer or shorter, so a hostile length never allocates
//! more than the caller allowed.

use std::fmt;
use std::io::Read;

/// How a stored blob is compressed. The value is part of the file formats.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
#[repr(u8)]
pub enum Codec {
    /// Stored as is.
    Raw = 0,
    /// LZ4 block format.
    Lz4 = 1,
    /// A zstd frame at the fastest level.
    Zstd = 2,
}

impl Codec {
    /// The codec stored as `byte`, if it is one.
    pub const fn from_byte(byte: u8) -> Option<Self> {
        match byte {
            0 => Some(Self::Raw),
            1 => Some(Self::Lz4),
            2 => Some(Self::Zstd),
            _ => None,
        }
    }
}

/// Why a blob could not be decompressed.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CodecError(pub &'static str);

impl fmt::Display for CodecError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "corrupt compressed data: {}", self.0)
    }
}

impl std::error::Error for CodecError {}

/// Compresses `data`.
pub fn compress(codec: Codec, data: &[u8]) -> Vec<u8> {
    match codec {
        Codec::Raw => data.to_vec(),
        Codec::Lz4 => lz4_flex::block::compress(data),
        Codec::Zstd => {
            ruzstd::encoding::compress_to_vec(data, ruzstd::encoding::CompressionLevel::Fastest)
        }
    }
}

/// Decompresses `data`, which must expand to exactly `len` bytes.
pub fn decompress(codec: Codec, data: &[u8], len: usize) -> Result<Vec<u8>, CodecError> {
    let out = match codec {
        Codec::Raw => data.to_vec(),
        Codec::Lz4 => lz4_flex::block::decompress(data, len)
            .map_err(|_| CodecError("an invalid LZ4 block"))?,
        Codec::Zstd => {
            let mut source = data;
            let decoder = ruzstd::decoding::StreamingDecoder::new(&mut source)
                .map_err(|_| CodecError("an invalid zstd frame header"))?;
            let mut out = Vec::with_capacity(len.min(1 << 20));
            // One byte past `len` tells an over-long frame from an exact one.
            decoder
                .take(len as u64 + 1)
                .read_to_end(&mut out)
                .map_err(|_| CodecError("an invalid zstd frame"))?;
            out
        }
    };
    if out.len() != len {
        return Err(CodecError("the data does not expand to its stated length"));
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample() -> Vec<u8> {
        (0..20_000u32).map(|i| (i / 37 % 7) as u8).collect()
    }

    #[test]
    fn every_codec_round_trips_and_compresses_repetitive_data() {
        let data = sample();
        for codec in [Codec::Raw, Codec::Lz4, Codec::Zstd] {
            let packed = compress(codec, &data);
            assert_eq!(
                decompress(codec, &packed, data.len()).unwrap(),
                data,
                "{codec:?}"
            );
            if codec != Codec::Raw {
                assert!(packed.len() < data.len() / 4, "{codec:?}: {}", packed.len());
            }
            assert_eq!(Codec::from_byte(codec as u8), Some(codec));
        }
        assert_eq!(Codec::from_byte(3), None);
    }

    #[test]
    fn wrong_lengths_and_garbage_are_errors() {
        let data = sample();
        for codec in [Codec::Raw, Codec::Lz4, Codec::Zstd] {
            let packed = compress(codec, &data);
            assert!(
                decompress(codec, &packed, data.len() - 1).is_err(),
                "{codec:?}"
            );
            assert!(
                decompress(codec, &packed, data.len() + 1).is_err(),
                "{codec:?}"
            );
        }
        assert!(decompress(Codec::Zstd, &[1, 2, 3, 4, 5], 10).is_err());
        assert!(decompress(Codec::Lz4, &[0xFF; 16], 1000).is_err());
    }
}
