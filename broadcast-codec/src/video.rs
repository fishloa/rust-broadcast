//! Decoded video: pixel format, plane storage and one frame.

use alloc::vec::Vec;
use core::num::{NonZeroU32, NonZeroUsize};

/// Bytes per 8-bit sample.
const BYTE_SAMPLES: usize = 1;
/// Bytes per 10-bit sample in a 16-bit container.
const WORD_SAMPLES: usize = 2;
/// Half resolution, rounded up.
const HALF_RES: usize = 2;
/// Packed 24-bit RGB pixel bytes.
const RGB24_PIXEL_BYTES: usize = 3;
/// Packed 32-bit RGBA pixel bytes.
const RGBA32_PIXEL_BYTES: usize = 4;
/// NV12's interleaved UV pair (two 8-bit samples).
const NV12_UV_PAIR_BYTES: usize = 2;
/// P010's interleaved UV pair (two 10-bit samples in 16-bit containers).
const P010_UV_PAIR_BYTES: usize = 4;
/// A single-plane packed format.
const PACKED_PLANES: usize = 1;
/// A luma plane plus one interleaved chroma plane.
const SEMI_PLANAR_PLANES: usize = 2;
/// A luma plane plus two chroma planes.
const PLANAR_PLANES: usize = 3;

/// How the pixels of a [`VideoFrame`] are sampled and stored.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    /// One 8-bit luma plane.
    Gray8,
    /// 8-bit luma plus half-width, half-height chroma.
    Yuv420p8,
    /// 8-bit luma plus half-width chroma.
    Yuv422p8,
    /// 8-bit luma plus full-resolution chroma.
    Yuv444p8,
    /// 8-bit luma plus one interleaved half-width, half-height chroma plane.
    Nv12,
    /// 10-bit luma plus half-width, half-height chroma.
    Yuv420p10,
    /// 10-bit luma plus half-width chroma.
    Yuv422p10,
    /// 10-bit luma plus full-resolution chroma.
    Yuv444p10,
    /// 10-bit luma plus one interleaved half-width, half-height chroma plane.
    P010,
    /// Packed 8-bit RGB.
    Rgb24,
    /// Packed 8-bit RGBA.
    Rgba32,
}

impl PixelFormat {
    /// Label of the pixel format.
    #[must_use]
    pub const fn name(&self) -> &'static str {
        match self {
            Self::Gray8 => "gray8",
            Self::Yuv420p8 => "yuv420p",
            Self::Yuv422p8 => "yuv422p",
            Self::Yuv444p8 => "yuv444p",
            Self::Nv12 => "nv12",
            Self::Yuv420p10 => "yuv420p10",
            Self::Yuv422p10 => "yuv422p10",
            Self::Yuv444p10 => "yuv444p10",
            Self::P010 => "p010",
            Self::Rgb24 => "rgb24",
            Self::Rgba32 => "rgba32",
        }
    }

    /// Number of planes a frame of this format carries.
    #[must_use]
    pub const fn plane_count(&self) -> usize {
        match self {
            Self::Gray8 | Self::Rgb24 | Self::Rgba32 => PACKED_PLANES,
            Self::Nv12 | Self::P010 => SEMI_PLANAR_PLANES,
            Self::Yuv420p8
            | Self::Yuv422p8
            | Self::Yuv444p8
            | Self::Yuv420p10
            | Self::Yuv422p10
            | Self::Yuv444p10 => PLANAR_PLANES,
        }
    }

    /// Rows and bytes per row of `plane` for a `width` by `height` frame.
    ///
    /// `None` when the plane index does not exist for this format, or when the
    /// geometry overflows `usize`.
    #[must_use]
    pub(crate) fn plane_geometry(
        &self,
        width: usize,
        height: usize,
        plane: usize,
    ) -> Option<(usize, usize)> {
        if plane >= self.plane_count() {
            return None;
        }
        let (rows, row_bytes) = match self {
            Self::Gray8 => (height, width.checked_mul(BYTE_SAMPLES)?),
            Self::Yuv420p8 => match plane {
                0 => (height, width.checked_mul(BYTE_SAMPLES)?),
                _ => (
                    height.div_ceil(HALF_RES),
                    width.div_ceil(HALF_RES).checked_mul(BYTE_SAMPLES)?,
                ),
            },
            Self::Yuv422p8 => match plane {
                0 => (height, width.checked_mul(BYTE_SAMPLES)?),
                _ => (height, width.div_ceil(HALF_RES).checked_mul(BYTE_SAMPLES)?),
            },
            Self::Yuv444p8 => (height, width.checked_mul(BYTE_SAMPLES)?),
            Self::Nv12 => match plane {
                0 => (height, width.checked_mul(BYTE_SAMPLES)?),
                _ => (
                    height.div_ceil(HALF_RES),
                    width.div_ceil(HALF_RES).checked_mul(NV12_UV_PAIR_BYTES)?,
                ),
            },
            Self::Yuv420p10 => match plane {
                0 => (height, width.checked_mul(WORD_SAMPLES)?),
                _ => (
                    height.div_ceil(HALF_RES),
                    width.div_ceil(HALF_RES).checked_mul(WORD_SAMPLES)?,
                ),
            },
            Self::Yuv422p10 => match plane {
                0 => (height, width.checked_mul(WORD_SAMPLES)?),
                _ => (height, width.div_ceil(HALF_RES).checked_mul(WORD_SAMPLES)?),
            },
            Self::Yuv444p10 => (height, width.checked_mul(WORD_SAMPLES)?),
            Self::P010 => match plane {
                0 => (height, width.checked_mul(WORD_SAMPLES)?),
                _ => (
                    height.div_ceil(HALF_RES),
                    width.div_ceil(HALF_RES).checked_mul(P010_UV_PAIR_BYTES)?,
                ),
            },
            Self::Rgb24 => (height, width.checked_mul(RGB24_PIXEL_BYTES)?),
            Self::Rgba32 => (height, width.checked_mul(RGBA32_PIXEL_BYTES)?),
        };
        Some((rows, row_bytes))
    }

    /// Bytes `plane` needs, unpadded, for a `width` by `height` frame.
    #[must_use]
    pub fn plane_size(&self, width: usize, height: usize, plane: usize) -> Option<usize> {
        let (rows, row_bytes) = self.plane_geometry(width, height, plane)?;
        rows.checked_mul(row_bytes)
    }
}

broadcast_core::impl_spec_display!(PixelFormat);

/// One plane of a decoded frame.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoPlane {
    data: Vec<u8>,
    stride: NonZeroUsize,
}

impl VideoPlane {
    /// A plane over `data` with `stride` bytes per row.
    #[must_use]
    pub const fn new(data: Vec<u8>, stride: NonZeroUsize) -> Self {
        Self { data, stride }
    }

    /// The plane bytes, including any row padding.
    #[must_use]
    pub fn data(&self) -> &[u8] {
        &self.data
    }

    /// Bytes per row.
    #[must_use]
    pub const fn stride(&self) -> NonZeroUsize {
        self.stride
    }
}

/// Where a frame's planes live.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VideoStorage {
    /// Planes in host memory.
    Cpu(Vec<VideoPlane>),
}

impl VideoStorage {
    /// Whether every plane matches `format` and the coded size.
    fn fits(&self, format: PixelFormat, width: usize, height: usize) -> bool {
        match self {
            Self::Cpu(planes) => {
                if planes.len() != format.plane_count() {
                    return false;
                }
                planes.iter().enumerate().all(|(index, plane)| {
                    let Some((rows, row_bytes)) = format.plane_geometry(width, height, index)
                    else {
                        return false;
                    };
                    let stride = plane.stride().get();
                    if stride < row_bytes {
                        return false;
                    }
                    let Some(last_row) = stride.checked_mul(rows - 1) else {
                        return false;
                    };
                    let Some(needed) = last_row.checked_add(row_bytes) else {
                        return false;
                    };
                    plane.data().len() >= needed
                })
            }
        }
    }
}

/// The colour properties a stream declares.
#[non_exhaustive]
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ColourDescription {
    primaries: u8,
    transfer: u8,
    matrix: u8,
    full_range: bool,
}

impl ColourDescription {
    /// A colour description from the three code points and the range flag.
    #[must_use]
    pub const fn new(primaries: u8, transfer: u8, matrix: u8, full_range: bool) -> Self {
        Self {
            primaries,
            transfer,
            matrix,
            full_range,
        }
    }

    /// Colour primaries code point.
    #[must_use]
    pub const fn primaries(&self) -> u8 {
        self.primaries
    }

    /// Transfer characteristics code point.
    #[must_use]
    pub const fn transfer(&self) -> u8 {
        self.transfer
    }

    /// Matrix coefficients code point.
    #[must_use]
    pub const fn matrix(&self) -> u8 {
        self.matrix
    }

    /// Whether the stream uses full-range samples.
    #[must_use]
    pub const fn full_range(&self) -> bool {
        self.full_range
    }
}

/// One decoded video frame.
///
/// The checked constructor refuses a plane count or plane size that does not
/// fit the format and coded size, so a frame is always self-consistent.
#[non_exhaustive]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VideoFrame {
    width: NonZeroU32,
    height: NonZeroU32,
    format: PixelFormat,
    storage: VideoStorage,
    colour: Option<ColourDescription>,
    pts: Option<i64>,
    key: bool,
}

impl VideoFrame {
    /// A frame from its coded size, format and storage.
    ///
    /// Returns `None` when the storage does not hold exactly the planes the
    /// format names, or a plane is too small or too narrow for the coded size.
    #[must_use]
    pub fn new(
        width: NonZeroU32,
        height: NonZeroU32,
        format: PixelFormat,
        storage: VideoStorage,
    ) -> Option<Self> {
        if !storage.fits(format, width.get() as usize, height.get() as usize) {
            return None;
        }
        Some(Self {
            width,
            height,
            format,
            storage,
            colour: None,
            pts: None,
            key: false,
        })
    }

    /// Attach the colour description.
    #[must_use]
    pub const fn with_colour(mut self, colour: ColourDescription) -> Self {
        self.colour = Some(colour);
        self
    }

    /// Attach the presentation timestamp in the stream's time base.
    #[must_use]
    pub const fn with_pts(mut self, pts: i64) -> Self {
        self.pts = Some(pts);
        self
    }

    /// Mark the frame as, or not as, a random-access point.
    #[must_use]
    pub const fn with_key(mut self, key: bool) -> Self {
        self.key = key;
        self
    }

    /// Coded width in pixels.
    #[must_use]
    pub const fn width(&self) -> NonZeroU32 {
        self.width
    }

    /// Coded height in pixels.
    #[must_use]
    pub const fn height(&self) -> NonZeroU32 {
        self.height
    }

    /// Pixel format.
    #[must_use]
    pub const fn format(&self) -> PixelFormat {
        self.format
    }

    /// Where the planes live.
    #[must_use]
    pub const fn storage(&self) -> &VideoStorage {
        &self.storage
    }

    /// Colour description, when the stream declares one.
    #[must_use]
    pub const fn colour(&self) -> Option<ColourDescription> {
        self.colour
    }

    /// Presentation timestamp in the stream's time base, when known.
    #[must_use]
    pub const fn pts(&self) -> Option<i64> {
        self.pts
    }

    /// Whether this is a random-access point.
    #[must_use]
    pub const fn key(&self) -> bool {
        self.key
    }
}
