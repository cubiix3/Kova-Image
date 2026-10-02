//! DirectDraw Surface textures (`.dds`), as games and texture tools write them.
//! Only the first picture is shown: the largest mip level of the first image of
//! an array, cube map or volume.
//!
//! Supported: block compression BC1 to BC5 and BC7 (the legacy FourCC codes
//! DXT1 to DXT5, ATI1, ATI2, and the DX10 header's DXGI formats), uncompressed
//! pixels described by channel masks (RGB, RGBA, luminance, alpha, 8 to 32 bits)
//! or by common DXGI formats, and 8 bit palettized textures, which older games
//! used for effects. Not supported, and reported as unsupported: BC6H and
//! floating point or 16-bit formats, signed formats, and YUV or bump map layouts.
//!
//! The data is read strip by strip, so only the RGBA result is kept in memory.
//! The BC7 partition tables follow the Microsoft BC7 specification; they were
//! extracted from bcdec by Sergii Kudlai (MIT or Unlicense), which also served
//! as the reference the decoder is tested against.
use super::Reader;
use crate::{
    animation::Loops,
    decoder::{Frame, Pending},
    error::Error,
    format::Format,
    security::{self, Ticket},
};
use std::{io::Read, time::Duration};

fn corrupted(message: &str) -> Error {
    Error::Corrupted(format!("DDS: {message}"))
}

const DDPF_ALPHAPIXELS: u32 = 0x1;
const DDPF_ALPHA: u32 = 0x2;
const DDPF_FOURCC: u32 = 0x4;
const DDPF_PALETTE_INDEXED8: u32 = 0x20;
const DDPF_RGB: u32 = 0x40;
const DDPF_LUMINANCE: u32 = 0x2_0000;

/// Magic, the 124 byte header and the optional DX10 extension.
const HEADER: usize = 128;
const HEADER_DX10: usize = 148;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Compressed {
    Bc1,
    Bc2,
    Bc3,
    Bc4,
    Bc5,
    Bc7,
}

impl Compressed {
    fn block_bytes(self) -> usize {
        match self {
            Self::Bc1 | Self::Bc4 => 8,
            _ => 16,
        }
    }
    fn has_alpha(self) -> bool {
        !matches!(self, Self::Bc4 | Self::Bc5)
    }
}

/// Where one colour channel sits in a pixel, and how many bits it has.
#[derive(Clone, Copy, Debug, Default)]
struct Channel {
    shift: u32,
    bits: u32,
}

impl Channel {
    fn from_mask(mask: u32) -> Self {
        if mask == 0 {
            return Self::default();
        }
        let shift = mask.trailing_zeros();
        Self {
            shift,
            bits: (mask >> shift).count_ones(),
        }
    }
    fn present(self) -> bool {
        self.bits > 0
    }
    /// The channel of `pixel`, scaled to 0 to 255.
    fn scale(self, pixel: u32) -> u8 {
        if self.bits == 0 {
            return 0;
        }
        let max = if self.bits >= 32 {
            u64::from(u32::MAX)
        } else {
            (1u64 << self.bits) - 1
        };
        let value = u64::from(pixel >> self.shift) & max;
        ((value * 255 + max / 2) / max) as u8
    }
}

/// Pixels of 1 to 4 bytes, little endian, with the channels given by masks.
#[derive(Clone, Copy, Debug)]
struct Packed {
    bytes: usize,
    r: Channel,
    g: Channel,
    b: Channel,
    a: Channel,
    /// One channel (in `r`) stands for all three colours.
    grey: bool,
}

impl Packed {
    fn from_masks(bytes: usize, r: u32, g: u32, b: u32, a: u32) -> Self {
        Self {
            bytes,
            r: Channel::from_mask(r),
            g: Channel::from_mask(g),
            b: Channel::from_mask(b),
            a: Channel::from_mask(a),
            grey: false,
        }
    }
    fn grey(bytes: usize, value: u32, a: u32) -> Self {
        Self {
            grey: true,
            ..Self::from_masks(bytes, value, 0, 0, a)
        }
    }
    fn rgba(self, pixel: u32) -> [u8; 4] {
        let alpha = if self.a.present() {
            self.a.scale(pixel)
        } else {
            255
        };
        if self.grey {
            let value = self.r.scale(pixel);
            [value, value, value, alpha]
        } else {
            [
                self.r.scale(pixel),
                self.g.scale(pixel),
                self.b.scale(pixel),
                alpha,
            ]
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum Layout {
    Blocks {
        kind: Compressed,
        /// DXT2 and DXT4 store colours multiplied by alpha.
        premultiplied: bool,
    },
    Packed(Packed),
    /// 8 bit indices into a palette of 256 RGBA entries that follows the header.
    Indexed,
}

struct Header {
    width: u32,
    height: u32,
    layout: Layout,
}

fn le32(bytes: &[u8], at: usize) -> u32 {
    u32::from_le_bytes([bytes[at], bytes[at + 1], bytes[at + 2], bytes[at + 3]])
}

/// The layout of a texture described by a DXGI format of the DX10 header.
fn dxgi_layout(format: u32) -> Result<Layout, Error> {
    let blocks = |kind| Layout::Blocks {
        kind,
        premultiplied: false,
    };
    let packed = |bytes, r, g, b, a| Layout::Packed(Packed::from_masks(bytes, r, g, b, a));
    Ok(match format {
        70..=72 => blocks(Compressed::Bc1),
        73..=75 => blocks(Compressed::Bc2),
        76..=78 => blocks(Compressed::Bc3),
        79 | 80 => blocks(Compressed::Bc4),
        82 | 83 => blocks(Compressed::Bc5),
        97..=99 => blocks(Compressed::Bc7),
        // R8G8B8A8, with and without sRGB
        27..=29 => packed(4, 0xff, 0xff00, 0xff_0000, 0xff00_0000),
        // R10G10B10A2_UNORM
        24 => packed(4, 0x3ff, 0xffc00, 0x3ff0_0000, 0xc000_0000),
        // B8G8R8A8, B8G8R8X8 (typeless and sRGB variants)
        87 | 90 | 91 => packed(4, 0xff_0000, 0xff00, 0xff, 0xff00_0000),
        88 | 92 | 93 => packed(4, 0xff_0000, 0xff00, 0xff, 0),
        // B5G6R5, B5G5R5A1, B4G4R4A4
        85 => packed(2, 0xf800, 0x07e0, 0x001f, 0),
        86 => packed(2, 0x7c00, 0x03e0, 0x001f, 0x8000),
        115 => packed(2, 0x0f00, 0x00f0, 0x000f, 0xf000),
        // R8G8
        49 => packed(2, 0xff, 0xff00, 0, 0),
        // R8_UNORM, and A8_UNORM which is shown as grey too
        61 | 65 => Layout::Packed(Packed::grey(1, 0xff, 0)),
        _ => return Err(Error::Unsupported),
    })
}

fn parse_header(head: &[u8]) -> Result<Header, Error> {
    if head.len() < HEADER || &head[..4] != b"DDS " || le32(head, 4) != 124 {
        return Err(corrupted("not a DDS header"));
    }
    let (height, width) = (le32(head, 12), le32(head, 16));
    if width == 0
        || height == 0
        || width > security::MAX_DIMENSION
        || height > security::MAX_DIMENSION
    {
        return Err(Error::Dimensions);
    }
    if le32(head, 76) != 32 {
        return Err(corrupted("the pixel format block has the wrong size"));
    }
    let (flags, four_cc, bit_count) = (le32(head, 80), &head[84..88], le32(head, 88));
    let masks = [
        le32(head, 92),
        le32(head, 96),
        le32(head, 100),
        le32(head, 104),
    ];
    let layout = if flags & DDPF_FOURCC != 0 {
        let blocks = |kind, premultiplied| Layout::Blocks {
            kind,
            premultiplied,
        };
        match four_cc {
            b"DXT1" => blocks(Compressed::Bc1, false),
            b"DXT2" => blocks(Compressed::Bc2, true),
            b"DXT3" => blocks(Compressed::Bc2, false),
            b"DXT4" => blocks(Compressed::Bc3, true),
            b"DXT5" => blocks(Compressed::Bc3, false),
            b"ATI1" | b"BC4U" => blocks(Compressed::Bc4, false),
            b"ATI2" | b"BC5U" => blocks(Compressed::Bc5, false),
            b"DX10" => {
                if head.len() < HEADER_DX10 {
                    return Err(corrupted("the DX10 header is cut short"));
                }
                dxgi_layout(le32(head, HEADER))?
            }
            _ => return Err(Error::Unsupported),
        }
    } else if flags & (DDPF_RGB | DDPF_LUMINANCE | DDPF_ALPHA) != 0 {
        if !matches!(bit_count, 8 | 16 | 24 | 32) {
            return Err(Error::Unsupported);
        }
        let bytes = bit_count as usize / 8;
        let alpha = if flags & DDPF_ALPHAPIXELS != 0 {
            masks[3]
        } else {
            0
        };
        if flags & DDPF_RGB != 0 {
            Layout::Packed(Packed::from_masks(
                bytes, masks[0], masks[1], masks[2], alpha,
            ))
        } else if flags & DDPF_LUMINANCE != 0 {
            Layout::Packed(Packed::grey(bytes, masks[0], alpha))
        } else {
            // Alpha only: shown as grey, so that the shape is visible.
            Layout::Packed(Packed::grey(bytes, masks[3], 0))
        }
    } else if flags & DDPF_PALETTE_INDEXED8 != 0 && bit_count == 8 {
        Layout::Indexed
    } else {
        return Err(Error::Unsupported);
    };
    Ok(Header {
        width,
        height,
        layout,
    })
}

type Block = [[u8; 4]; 16];

fn rgb565(value: u16) -> [u8; 3] {
    let (r, g, b) = (value >> 11, (value >> 5) & 63, value & 31);
    [
        (r << 3 | r >> 2) as u8,
        (g << 2 | g >> 4) as u8,
        (b << 3 | b >> 2) as u8,
    ]
}

/// A BC1 colour block. In BC2 and BC3 the block always has four colours; in BC1
/// proper, the order of the end points chooses between four colours and three
/// colours plus transparent black.
fn bc1(block: &[u8], always_four: bool) -> Block {
    let c0 = u16::from_le_bytes([block[0], block[1]]);
    let c1 = u16::from_le_bytes([block[2], block[3]]);
    let (p0, p1) = (rgb565(c0), rgb565(c1));
    // The opaque colour (a * p0 + b * p1) / divisor.
    let mix = |a: u16, b: u16, divisor: u16| -> [u8; 4] {
        let c: [u8; 3] = std::array::from_fn(|i| {
            ((a * u16::from(p0[i]) + b * u16::from(p1[i])) / divisor) as u8
        });
        [c[0], c[1], c[2], 255]
    };
    let palette = if c0 > c1 || always_four {
        [
            [p0[0], p0[1], p0[2], 255],
            [p1[0], p1[1], p1[2], 255],
            mix(2, 1, 3),
            mix(1, 2, 3),
        ]
    } else {
        [
            [p0[0], p0[1], p0[2], 255],
            [p1[0], p1[1], p1[2], 255],
            mix(1, 1, 2),
            [0; 4],
        ]
    };
    let indices = u32::from_le_bytes([block[4], block[5], block[6], block[7]]);
    std::array::from_fn(|i| palette[(indices >> (2 * i) & 3) as usize])
}

/// A BC3 alpha block, which BC4 also uses for its single channel.
fn smooth(block: &[u8]) -> [u8; 16] {
    let (a0, a1) = (u16::from(block[0]), u16::from(block[1]));
    let mut table = [0u8; 8];
    table[0] = a0 as u8;
    table[1] = a1 as u8;
    if a0 > a1 {
        for i in 1..7u16 {
            table[i as usize + 1] = (((7 - i) * a0 + i * a1) / 7) as u8;
        }
    } else {
        for i in 1..5u16 {
            table[i as usize + 1] = (((5 - i) * a0 + i * a1) / 5) as u8;
        }
        table[6] = 0;
        table[7] = 255;
    }
    let mut bytes = [0u8; 8];
    bytes[..6].copy_from_slice(&block[2..8]);
    let bits = u64::from_le_bytes(bytes);
    std::array::from_fn(|i| table[(bits >> (3 * i) & 7) as usize])
}

/// A BC5 block as a tangent space normal map: red and green are stored, blue is
/// rebuilt from them. The result is the usual blue-ish picture of such maps.
fn normal(x: u8, y: u8) -> [u8; 4] {
    let (fx, fy) = (f32::from(x) / 127.5 - 1.0, f32::from(y) / 127.5 - 1.0);
    let z = (1.0 - fx * fx - fy * fy).max(0.0).sqrt();
    [x, y, ((z * 0.5 + 0.5) * 255.0 + 0.5) as u8, 255]
}

fn decode_block(kind: Compressed, premultiplied: bool, block: &[u8]) -> Block {
    match kind {
        Compressed::Bc1 => bc1(block, false),
        Compressed::Bc2 => {
            let mut pixels = bc1(&block[8..], true);
            let alpha = u64::from_le_bytes(block[..8].try_into().expect("eight bytes"));
            for (i, pixel) in pixels.iter_mut().enumerate() {
                pixel[3] = ((alpha >> (4 * i) & 15) * 17) as u8;
            }
            unpremultiply(&mut pixels, premultiplied);
            pixels
        }
        Compressed::Bc3 => {
            let mut pixels = bc1(&block[8..], true);
            for (pixel, alpha) in pixels.iter_mut().zip(smooth(block)) {
                pixel[3] = alpha;
            }
            unpremultiply(&mut pixels, premultiplied);
            pixels
        }
        Compressed::Bc4 => {
            let red = smooth(block);
            std::array::from_fn(|i| [red[i], red[i], red[i], 255])
        }
        Compressed::Bc5 => {
            let (red, green) = (smooth(block), smooth(&block[8..]));
            std::array::from_fn(|i| normal(red[i], green[i]))
        }
        Compressed::Bc7 => bc7(block),
    }
}

fn unpremultiply(pixels: &mut Block, premultiplied: bool) {
    if !premultiplied {
        return;
    }
    for pixel in pixels {
        let alpha = u32::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for channel in &mut pixel[..3] {
                *channel = (u32::from(*channel) * 255 / alpha).min(255) as u8;
            }
        }
    }
}

#[rustfmt::skip]
const PARTITIONS_2: [[u8; 16]; 64] = [
    [128, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 129],
    [128, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 129],
    [128, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 0, 1, 1, 129],
    [128, 0, 0, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 1, 1, 129],
    [128, 0, 0, 0, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 1, 129],
    [128, 0, 1, 1, 0, 1, 1, 1, 0, 1, 1, 1, 1, 1, 1, 129],
    [128, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 1, 1, 1, 1, 129],
    [128, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 1, 0, 1, 1, 129],
    [128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 0, 1, 129],
    [128, 0, 1, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 129],
    [128, 0, 0, 0, 0, 0, 0, 1, 0, 1, 1, 1, 1, 1, 1, 129],
    [128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0, 1, 1, 129],
    [128, 0, 0, 1, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 129],
    [128, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 129],
    [128, 0, 0, 0, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 1, 129],
    [128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 1, 1, 129],
    [128, 0, 0, 0, 1, 0, 0, 0, 1, 1, 1, 0, 1, 1, 1, 129],
    [128, 1, 129, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [128, 0, 0, 0, 0, 0, 0, 0, 129, 0, 0, 0, 1, 1, 1, 0],
    [128, 1, 129, 1, 0, 0, 1, 1, 0, 0, 0, 1, 0, 0, 0, 0],
    [128, 0, 129, 1, 0, 0, 0, 1, 0, 0, 0, 0, 0, 0, 0, 0],
    [128, 0, 0, 0, 1, 0, 0, 0, 129, 1, 0, 0, 1, 1, 1, 0],
    [128, 0, 0, 0, 0, 0, 0, 0, 129, 0, 0, 0, 1, 1, 0, 0],
    [128, 1, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 0, 129],
    [128, 0, 129, 1, 0, 0, 0, 1, 0, 0, 0, 1, 0, 0, 0, 0],
    [128, 0, 0, 0, 1, 0, 0, 0, 129, 0, 0, 0, 1, 1, 0, 0],
    [128, 1, 129, 0, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 0],
    [128, 0, 129, 1, 0, 1, 1, 0, 0, 1, 1, 0, 1, 1, 0, 0],
    [128, 0, 0, 1, 0, 1, 1, 1, 129, 1, 1, 0, 1, 0, 0, 0],
    [128, 0, 0, 0, 1, 1, 1, 1, 129, 1, 1, 1, 0, 0, 0, 0],
    [128, 1, 129, 1, 0, 0, 0, 1, 1, 0, 0, 0, 1, 1, 1, 0],
    [128, 0, 129, 1, 1, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0, 0],
    [128, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 1, 0, 129],
    [128, 0, 0, 0, 1, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 129],
    [128, 1, 0, 1, 1, 0, 129, 0, 0, 1, 0, 1, 1, 0, 1, 0],
    [128, 0, 1, 1, 0, 0, 1, 1, 129, 1, 0, 0, 1, 1, 0, 0],
    [128, 0, 129, 1, 1, 1, 0, 0, 0, 0, 1, 1, 1, 1, 0, 0],
    [128, 1, 0, 1, 0, 1, 0, 1, 129, 0, 1, 0, 1, 0, 1, 0],
    [128, 1, 1, 0, 1, 0, 0, 1, 0, 1, 1, 0, 1, 0, 0, 129],
    [128, 1, 0, 1, 1, 0, 1, 0, 1, 0, 1, 0, 0, 1, 0, 129],
    [128, 1, 129, 1, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 1, 0],
    [128, 0, 0, 1, 0, 0, 1, 1, 129, 1, 0, 0, 1, 0, 0, 0],
    [128, 0, 129, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 1, 0, 0],
    [128, 0, 129, 1, 1, 0, 1, 1, 1, 1, 0, 1, 1, 1, 0, 0],
    [128, 1, 129, 0, 1, 0, 0, 1, 1, 0, 0, 1, 0, 1, 1, 0],
    [128, 0, 1, 1, 1, 1, 0, 0, 1, 1, 0, 0, 0, 0, 1, 129],
    [128, 1, 1, 0, 0, 1, 1, 0, 1, 0, 0, 1, 1, 0, 0, 129],
    [128, 0, 0, 0, 0, 1, 129, 0, 0, 1, 1, 0, 0, 0, 0, 0],
    [128, 1, 0, 0, 1, 1, 129, 0, 0, 1, 0, 0, 0, 0, 0, 0],
    [128, 0, 129, 0, 0, 1, 1, 1, 0, 0, 1, 0, 0, 0, 0, 0],
    [128, 0, 0, 0, 0, 0, 129, 0, 0, 1, 1, 1, 0, 0, 1, 0],
    [128, 0, 0, 0, 0, 1, 0, 0, 129, 1, 1, 0, 0, 1, 0, 0],
    [128, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 1, 0, 0, 1, 129],
    [128, 0, 1, 1, 0, 1, 1, 0, 1, 1, 0, 0, 1, 0, 0, 129],
    [128, 1, 129, 0, 0, 0, 1, 1, 1, 0, 0, 1, 1, 1, 0, 0],
    [128, 0, 129, 1, 1, 0, 0, 1, 1, 1, 0, 0, 0, 1, 1, 0],
    [128, 1, 1, 0, 1, 1, 0, 0, 1, 1, 0, 0, 1, 0, 0, 129],
    [128, 1, 1, 0, 0, 0, 1, 1, 0, 0, 1, 1, 1, 0, 0, 129],
    [128, 1, 1, 1, 1, 1, 1, 0, 1, 0, 0, 0, 0, 0, 0, 129],
    [128, 0, 0, 1, 1, 0, 0, 0, 1, 1, 1, 0, 0, 1, 1, 129],
    [128, 0, 0, 0, 1, 1, 1, 1, 0, 0, 1, 1, 0, 0, 1, 129],
    [128, 0, 129, 1, 0, 0, 1, 1, 1, 1, 1, 1, 0, 0, 0, 0],
    [128, 0, 129, 0, 0, 0, 1, 0, 1, 1, 1, 0, 1, 1, 1, 0],
    [128, 1, 0, 0, 0, 1, 0, 0, 0, 1, 1, 1, 0, 1, 1, 129],
];
#[rustfmt::skip]
const PARTITIONS_3: [[u8; 16]; 64] = [
    [128, 0, 1, 129, 0, 0, 1, 1, 0, 2, 2, 1, 2, 2, 2, 130],
    [128, 0, 0, 129, 0, 0, 1, 1, 130, 2, 1, 1, 2, 2, 2, 1],
    [128, 0, 0, 0, 2, 0, 0, 1, 130, 2, 1, 1, 2, 2, 1, 129],
    [128, 2, 2, 130, 0, 0, 2, 2, 0, 0, 1, 1, 0, 1, 1, 129],
    [128, 0, 0, 0, 0, 0, 0, 0, 129, 1, 2, 2, 1, 1, 2, 130],
    [128, 0, 1, 129, 0, 0, 1, 1, 0, 0, 2, 2, 0, 0, 2, 130],
    [128, 0, 2, 130, 0, 0, 2, 2, 1, 1, 1, 1, 1, 1, 1, 129],
    [128, 0, 1, 1, 0, 0, 1, 1, 130, 2, 1, 1, 2, 2, 1, 129],
    [128, 0, 0, 0, 0, 0, 0, 0, 129, 1, 1, 1, 2, 2, 2, 130],
    [128, 0, 0, 0, 1, 1, 1, 1, 129, 1, 1, 1, 2, 2, 2, 130],
    [128, 0, 0, 0, 1, 1, 129, 1, 2, 2, 2, 2, 2, 2, 2, 130],
    [128, 0, 1, 2, 0, 0, 129, 2, 0, 0, 1, 2, 0, 0, 1, 130],
    [128, 1, 1, 2, 0, 1, 129, 2, 0, 1, 1, 2, 0, 1, 1, 130],
    [128, 1, 2, 2, 0, 129, 2, 2, 0, 1, 2, 2, 0, 1, 2, 130],
    [128, 0, 1, 129, 0, 1, 1, 2, 1, 1, 2, 2, 1, 2, 2, 130],
    [128, 0, 1, 129, 2, 0, 0, 1, 130, 2, 0, 0, 2, 2, 2, 0],
    [128, 0, 0, 129, 0, 0, 1, 1, 0, 1, 1, 2, 1, 1, 2, 130],
    [128, 1, 1, 129, 0, 0, 1, 1, 130, 0, 0, 1, 2, 2, 0, 0],
    [128, 0, 0, 0, 1, 1, 2, 2, 129, 1, 2, 2, 1, 1, 2, 130],
    [128, 0, 2, 130, 0, 0, 2, 2, 0, 0, 2, 2, 1, 1, 1, 129],
    [128, 1, 1, 129, 0, 1, 1, 1, 0, 2, 2, 2, 0, 2, 2, 130],
    [128, 0, 0, 129, 0, 0, 0, 1, 130, 2, 2, 1, 2, 2, 2, 1],
    [128, 0, 0, 0, 0, 0, 129, 1, 0, 1, 2, 2, 0, 1, 2, 130],
    [128, 0, 0, 0, 1, 1, 0, 0, 130, 2, 129, 0, 2, 2, 1, 0],
    [128, 1, 2, 130, 0, 129, 2, 2, 0, 0, 1, 1, 0, 0, 0, 0],
    [128, 0, 1, 2, 0, 0, 1, 2, 129, 1, 2, 2, 2, 2, 2, 130],
    [128, 1, 1, 0, 1, 2, 130, 1, 129, 2, 2, 1, 0, 1, 1, 0],
    [128, 0, 0, 0, 0, 1, 129, 0, 1, 2, 130, 1, 1, 2, 2, 1],
    [128, 0, 2, 2, 1, 1, 0, 2, 129, 1, 0, 2, 0, 0, 2, 130],
    [128, 1, 1, 0, 0, 129, 1, 0, 2, 0, 0, 2, 2, 2, 2, 130],
    [128, 0, 1, 1, 0, 1, 2, 2, 0, 1, 130, 2, 0, 0, 1, 129],
    [128, 0, 0, 0, 2, 0, 0, 0, 130, 2, 1, 1, 2, 2, 2, 129],
    [128, 0, 0, 0, 0, 0, 0, 2, 129, 1, 2, 2, 1, 2, 2, 130],
    [128, 2, 2, 130, 0, 0, 2, 2, 0, 0, 1, 2, 0, 0, 1, 129],
    [128, 0, 1, 129, 0, 0, 1, 2, 0, 0, 2, 2, 0, 2, 2, 130],
    [128, 1, 2, 0, 0, 129, 2, 0, 0, 1, 130, 0, 0, 1, 2, 0],
    [128, 0, 0, 0, 1, 1, 129, 1, 2, 2, 130, 2, 0, 0, 0, 0],
    [128, 1, 2, 0, 1, 2, 0, 1, 130, 0, 129, 2, 0, 1, 2, 0],
    [128, 1, 2, 0, 2, 0, 1, 2, 129, 130, 0, 1, 0, 1, 2, 0],
    [128, 0, 1, 1, 2, 2, 0, 0, 1, 1, 130, 2, 0, 0, 1, 129],
    [128, 0, 1, 1, 1, 1, 130, 2, 2, 2, 0, 0, 0, 0, 1, 129],
    [128, 1, 0, 129, 0, 1, 0, 1, 2, 2, 2, 2, 2, 2, 2, 130],
    [128, 0, 0, 0, 0, 0, 0, 0, 130, 1, 2, 1, 2, 1, 2, 129],
    [128, 0, 2, 2, 1, 129, 2, 2, 0, 0, 2, 2, 1, 1, 2, 130],
    [128, 0, 2, 130, 0, 0, 1, 1, 0, 0, 2, 2, 0, 0, 1, 129],
    [128, 2, 2, 0, 1, 2, 130, 1, 0, 2, 2, 0, 1, 2, 2, 129],
    [128, 1, 0, 1, 2, 2, 130, 2, 2, 2, 2, 2, 0, 1, 0, 129],
    [128, 0, 0, 0, 2, 1, 2, 1, 130, 1, 2, 1, 2, 1, 2, 129],
    [128, 1, 0, 129, 0, 1, 0, 1, 0, 1, 0, 1, 2, 2, 2, 130],
    [128, 2, 2, 130, 0, 1, 1, 1, 0, 2, 2, 2, 0, 1, 1, 129],
    [128, 0, 0, 2, 1, 129, 1, 2, 0, 0, 0, 2, 1, 1, 1, 130],
    [128, 0, 0, 0, 2, 129, 1, 2, 2, 1, 1, 2, 2, 1, 1, 130],
    [128, 2, 2, 2, 0, 129, 1, 1, 0, 1, 1, 1, 0, 2, 2, 130],
    [128, 0, 0, 2, 1, 1, 1, 2, 129, 1, 1, 2, 0, 0, 0, 130],
    [128, 1, 1, 0, 0, 129, 1, 0, 0, 1, 1, 0, 2, 2, 2, 130],
    [128, 0, 0, 0, 0, 0, 0, 0, 2, 1, 129, 2, 2, 1, 1, 130],
    [128, 1, 1, 0, 0, 129, 1, 0, 2, 2, 2, 2, 2, 2, 2, 130],
    [128, 0, 2, 2, 0, 0, 1, 1, 0, 0, 129, 1, 0, 0, 2, 130],
    [128, 0, 2, 2, 1, 1, 2, 2, 129, 1, 2, 2, 0, 0, 2, 130],
    [128, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 2, 129, 1, 130],
    [128, 0, 0, 130, 0, 0, 0, 1, 0, 0, 0, 2, 0, 0, 0, 129],
    [128, 2, 2, 2, 1, 2, 2, 2, 0, 2, 2, 2, 129, 2, 2, 130],
    [128, 1, 0, 129, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 2, 130],
    [128, 1, 1, 129, 2, 0, 1, 1, 130, 2, 0, 1, 2, 2, 2, 0],
];

const WEIGHTS_2: [u32; 4] = [0, 21, 43, 64];
const WEIGHTS_3: [u32; 8] = [0, 9, 18, 27, 37, 46, 55, 64];
const WEIGHTS_4: [u32; 16] = [0, 4, 9, 13, 17, 21, 26, 30, 34, 38, 43, 47, 51, 55, 60, 64];
/// Bits per colour channel and per alpha channel of each BC7 mode, without p-bit.
const COLOUR_BITS: [u32; 8] = [4, 6, 5, 7, 5, 7, 7, 5];
const ALPHA_BITS: [u32; 8] = [0, 0, 0, 0, 6, 8, 7, 5];
/// Modes with a p-bit: 0, 1, 3, 6 and 7.
const HAS_P_BITS: u8 = 0b1100_1011;

struct Bits {
    value: u128,
    position: u32,
}

impl Bits {
    fn take(&mut self, count: u32) -> u32 {
        if count == 0 {
            return 0;
        }
        let value = (self.value >> self.position) & ((1u128 << count) - 1);
        self.position += count;
        value as u32
    }
}

fn interpolate(a: u32, b: u32, weight: u32) -> u32 {
    (a * (64 - weight) + b * weight + 32) >> 6
}

fn weights(bits: u32) -> &'static [u32] {
    match bits {
        2 => &WEIGHTS_2,
        3 => &WEIGHTS_3,
        _ => &WEIGHTS_4,
    }
}

/// A BC7 block: eight modes with one to three subsets, partitions, p-bits,
/// separate alpha indices and channel rotation.
fn bc7(block: &[u8]) -> Block {
    let mut bytes = [0u8; 16];
    bytes.copy_from_slice(&block[..16]);
    let mut bits = Bits {
        value: u128::from_le_bytes(bytes),
        position: 0,
    };
    let mode = bits.value.trailing_zeros();
    if mode >= 8 {
        // A reserved mode: transparent black, as the specification asks.
        return [[0; 4]; 16];
    }
    let mode = mode as usize;
    bits.position = mode as u32 + 1;

    let subsets = match mode {
        0 | 2 => 3,
        1 | 3 | 7 => 2,
        _ => 1,
    };
    let partition = match mode {
        0 => bits.take(4),
        1 | 2 | 3 | 7 => bits.take(6),
        _ => 0,
    } as usize;
    let (mut rotation, mut selection) = (0, 0);
    if mode == 4 || mode == 5 {
        rotation = bits.take(2);
        if mode == 4 {
            selection = bits.take(1);
        }
    }

    let endpoints_count = subsets * 2;
    let mut endpoints = [[0u32; 4]; 6];
    for channel in 0..3 {
        for endpoint in endpoints.iter_mut().take(endpoints_count) {
            endpoint[channel] = bits.take(COLOUR_BITS[mode]);
        }
    }
    if ALPHA_BITS[mode] > 0 {
        for endpoint in endpoints.iter_mut().take(endpoints_count) {
            endpoint[3] = bits.take(ALPHA_BITS[mode]);
        }
    }
    let p_bits = HAS_P_BITS >> mode & 1 == 1;
    if p_bits {
        for endpoint in endpoints.iter_mut().take(endpoints_count) {
            for value in endpoint.iter_mut() {
                *value <<= 1;
            }
        }
        if mode == 1 {
            // One p-bit shared by both end points of a subset.
            let shared = [bits.take(1), bits.take(1)];
            for (index, endpoint) in endpoints.iter_mut().take(4).enumerate() {
                for value in &mut endpoint[..3] {
                    *value |= shared[index / 2];
                }
            }
        } else {
            for endpoint in endpoints.iter_mut().take(endpoints_count) {
                let bit = bits.take(1);
                for value in endpoint.iter_mut() {
                    *value |= bit;
                }
            }
        }
    }
    // Widen every channel to eight bits by repeating its top bits.
    let extra = u32::from(p_bits);
    for endpoint in endpoints.iter_mut().take(endpoints_count) {
        let colour = COLOUR_BITS[mode] + extra;
        for value in &mut endpoint[..3] {
            *value = *value << (8 - colour) | *value >> (2 * colour - 8);
        }
        if ALPHA_BITS[mode] > 0 {
            let alpha = ALPHA_BITS[mode] + extra;
            endpoint[3] = endpoint[3] << (8 - alpha) | endpoint[3] >> (2 * alpha - 8);
        } else {
            endpoint[3] = 255;
        }
    }

    let primary_bits = match mode {
        0 | 1 => 3,
        6 => 4,
        _ => 2,
    };
    let secondary_bits = match mode {
        4 => 3,
        5 => 2,
        _ => 0,
    };
    let subset_of = |pixel: usize| -> u8 {
        match subsets {
            1 => {
                if pixel == 0 {
                    128
                } else {
                    0
                }
            }
            2 => PARTITIONS_2[partition][pixel],
            _ => PARTITIONS_3[partition][pixel],
        }
    };
    // The index of the first pixel of each subset has one bit less (its top
    // bit is known to be zero). They are all stored before the secondary indices.
    let mut indices = [0u32; 16];
    for (pixel, index) in indices.iter_mut().enumerate() {
        let count = primary_bits - u32::from(subset_of(pixel) & 0x80 != 0);
        *index = bits.take(count);
    }
    let primary = weights(primary_bits);
    let mut out = [[0u8; 4]; 16];
    for (pixel, out) in out.iter_mut().enumerate() {
        let subset = usize::from(subset_of(pixel) & 3);
        let (low, high) = (endpoints[subset * 2], endpoints[subset * 2 + 1]);
        let mix = |channel: usize, weight: u32| interpolate(low[channel], high[channel], weight);
        let index = indices[pixel] as usize;
        let (colour_weight, alpha_weight) = if secondary_bits == 0 {
            (primary[index], primary[index])
        } else {
            let count = secondary_bits - u32::from(pixel == 0);
            let second = bits.take(count) as usize;
            if selection == 0 {
                (primary[index], weights(secondary_bits)[second])
            } else {
                (weights(secondary_bits)[second], primary[index])
            }
        };
        let (mut r, mut g, mut b, mut a) = (
            mix(0, colour_weight),
            mix(1, colour_weight),
            mix(2, colour_weight),
            mix(3, alpha_weight),
        );
        match rotation {
            1 => std::mem::swap(&mut a, &mut r),
            2 => std::mem::swap(&mut a, &mut g),
            3 => std::mem::swap(&mut a, &mut b),
            _ => {}
        }
        *out = [r as u8, g as u8, b as u8, a as u8];
    }
    out
}

fn fill(reader: &mut Reader, buffer: &mut [u8], ticket: &Ticket) -> Result<(), Error> {
    reader.read_exact(buffer).map_err(|error| {
        if let Err(stale) = ticket.check() {
            stale
        } else if error.kind() == std::io::ErrorKind::UnexpectedEof {
            corrupted("the file ends inside the texture data")
        } else {
            error.into()
        }
    })
}

pub(super) fn decode(mut reader: Reader, ticket: &Ticket) -> Result<Pending, Error> {
    let mut head = [0u8; HEADER_DX10];
    fill(&mut reader, &mut head[..HEADER], ticket)?;
    // The DX10 extension follows the header only when the FourCC says so.
    let extended = &head[84..88] == b"DX10" && le32(&head, 80) & DDPF_FOURCC != 0;
    if extended {
        fill(&mut reader, &mut head[HEADER..], ticket)?;
    }
    let header = parse_header(&head[..if extended { HEADER_DX10 } else { HEADER }])?;
    let (width, height) = (header.width as usize, header.height as usize);
    let mut rgba = vec![0u8; security::rgba_bytes(header.width, header.height)?];
    let alpha = match header.layout {
        Layout::Blocks {
            kind,
            premultiplied,
        } => {
            let size = kind.block_bytes();
            let mut strip = vec![0u8; width.div_ceil(4) * size];
            for block_y in 0..height.div_ceil(4) {
                ticket.check()?;
                fill(&mut reader, &mut strip, ticket)?;
                for (block_x, block) in strip.chunks_exact(size).enumerate() {
                    let pixels = decode_block(kind, premultiplied, block);
                    for (i, pixel) in pixels.iter().enumerate() {
                        let (x, y) = (block_x * 4 + i % 4, block_y * 4 + i / 4);
                        if x < width && y < height {
                            rgba[(y * width + x) * 4..][..4].copy_from_slice(pixel);
                        }
                    }
                }
            }
            kind.has_alpha()
        }
        Layout::Packed(packed) => {
            let mut row = vec![0u8; width * packed.bytes];
            for (y, out) in rgba.chunks_exact_mut(width * 4).enumerate() {
                if y % 16 == 0 {
                    ticket.check()?;
                }
                fill(&mut reader, &mut row, ticket)?;
                for (pixel, bytes) in out.chunks_exact_mut(4).zip(row.chunks_exact(packed.bytes)) {
                    let mut value = [0u8; 4];
                    value[..packed.bytes].copy_from_slice(bytes);
                    pixel.copy_from_slice(&packed.rgba(u32::from_le_bytes(value)));
                }
            }
            packed.a.present()
        }
        Layout::Indexed => {
            // The palette entries are red, green, blue and alpha. Writers that leave
            // the fourth byte unused set it to zero for every entry: opaque then.
            let mut table = [0u8; 1024];
            fill(&mut reader, &mut table, ticket)?;
            let unused = table.chunks_exact(4).all(|entry| entry[3] == 0);
            let mut row = vec![0u8; width];
            let mut translucent = false;
            for (y, out) in rgba.chunks_exact_mut(width * 4).enumerate() {
                if y % 16 == 0 {
                    ticket.check()?;
                }
                fill(&mut reader, &mut row, ticket)?;
                for (pixel, &index) in out.chunks_exact_mut(4).zip(&row) {
                    let entry = &table[usize::from(index) * 4..][..4];
                    let alpha = if unused { 255 } else { entry[3] };
                    translucent |= alpha != 255;
                    pixel.copy_from_slice(&[entry[0], entry[1], entry[2], alpha]);
                }
            }
            translucent
        }
    };
    Ok(Pending {
        format: Format::Dds,
        frames: vec![Frame {
            rgba,
            delay: Duration::from_secs(1),
        }],
        width: header.width,
        height: header.height,
        loops: Loops(Some(1)),
        photo: Default::default(),
        may_have_alpha: alpha,
        orientation: image::metadata::Orientation::NoTransforms,
        srgb: None,
        fitted: false,
        oriented: true,
        stored: None,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    /// Random BC7 blocks of every mode and what the bcdec reference makes of them
    /// (`scripts/bc7-reference.py`).
    const BLOCKS: &[u8] = include_bytes!("../../tests/fixtures/bc7.blocks");
    const EXPECTED: &[u8] = include_bytes!("../../tests/fixtures/bc7.rgba");

    #[test]
    fn bc7_matches_the_reference_decoder_in_every_mode() {
        assert_eq!(BLOCKS.len() / 16, EXPECTED.len() / 64);
        for (index, (block, expected)) in BLOCKS
            .chunks_exact(16)
            .zip(EXPECTED.chunks_exact(64))
            .enumerate()
        {
            let got: Vec<u8> = bc7(block).iter().flatten().copied().collect();
            assert_eq!(
                got,
                expected,
                "block {index} (mode {})",
                block[0].trailing_zeros()
            );
        }
    }

    #[test]
    fn bc1_uses_four_colours_or_three_and_transparent_black() {
        let (red, blue) = (0xf800u16.to_le_bytes(), 0x001fu16.to_le_bytes());
        // Indices 0, 1, 2, 3 in the first four texels.
        let indices = 0b11_10_01_00u32.to_le_bytes();
        let block = [red[0], red[1], blue[0], blue[1], indices[0], 0, 0, 0];
        let pixels = bc1(&block, false);
        assert_eq!(pixels[0], [255, 0, 0, 255]);
        assert_eq!(pixels[1], [0, 0, 255, 255]);
        assert_eq!(pixels[2], [170, 0, 85, 255]);
        assert_eq!(pixels[3], [85, 0, 170, 255]);
        // With the end points the other way round, there are three colours.
        let block = [blue[0], blue[1], red[0], red[1], indices[0], 0, 0, 0];
        let pixels = bc1(&block, false);
        assert_eq!(pixels[2], [127, 0, 127, 255]);
        assert_eq!(pixels[3], [0, 0, 0, 0]);
        // BC2 and BC3 colour blocks never use that mode.
        assert_eq!(bc1(&block, true)[3], [170, 0, 85, 255]);
    }

    fn alpha_block(a0: u8, a1: u8, indices: &[u64]) -> [u8; 8] {
        let bits = indices
            .iter()
            .enumerate()
            .fold(0u64, |bits, (i, index)| bits | index << (3 * i));
        let mut block = [0u8; 8];
        (block[0], block[1]) = (a0, a1);
        block[2..].copy_from_slice(&bits.to_le_bytes()[..6]);
        block
    }

    #[test]
    fn bc3_alpha_has_eight_steps_or_six_with_zero_and_full() {
        // Index 2 is (6 * a0 + a1) / 7 when a0 > a1, and (4 * a0 + a1) / 5 otherwise.
        assert_eq!(smooth(&alpha_block(255, 0, &[2; 16]))[5], 218);
        assert_eq!(smooth(&alpha_block(0, 255, &[2; 16]))[5], 51);
        // In the six step mode, indices 6 and 7 mean zero and full.
        let texels = smooth(&alpha_block(10, 200, &[0, 1, 6, 7]));
        assert_eq!(texels[..4], [10, 200, 0, 255]);
    }

    #[test]
    fn bc2_alpha_is_four_bits_per_texel() {
        let mut block = [0u8; 16];
        block[0] = 0xf0; // texel 0: 0, texel 1: 15
        let pixels = decode_block(Compressed::Bc2, false, &block);
        assert_eq!((pixels[0][3], pixels[1][3], pixels[2][3]), (0, 255, 0));
    }

    #[test]
    fn premultiplied_colours_are_divided_by_alpha() {
        let mut block = [0u8; 16];
        block[0] = 0x88; // alpha 8 * 17 = 136 for texels 0 and 1
        // Colour 0xffff is white; premultiplied by 136/255 it should be restored.
        block[8..10].copy_from_slice(&0xffffu16.to_le_bytes());
        let straight = decode_block(Compressed::Bc2, false, &block);
        let restored = decode_block(Compressed::Bc2, true, &block);
        assert_eq!(straight[0], [255, 255, 255, 136]);
        assert_eq!(restored[0], [255, 255, 255, 136]);
        // A mid grey becomes brighter.
        block[8..10].copy_from_slice(&0x8410u16.to_le_bytes());
        let darker = decode_block(Compressed::Bc2, false, &block)[0];
        let brighter = decode_block(Compressed::Bc2, true, &block)[0];
        assert!(brighter[0] > darker[0]);
    }

    #[test]
    fn bc5_rebuilds_the_blue_of_a_normal_map() {
        // Red and green at the middle: the normal points straight at the viewer.
        let flat = [128u8, 128, 0, 0, 0, 0, 0, 0];
        let mut block = [0u8; 16];
        block[..8].copy_from_slice(&flat);
        block[8..].copy_from_slice(&flat);
        let pixel = decode_block(Compressed::Bc5, false, &block)[0];
        assert_eq!(pixel[3], 255);
        assert!(pixel[2] > 250, "{pixel:?}");
    }

    #[test]
    fn masks_scale_every_width_to_eight_bits() {
        let packed = Packed::from_masks(2, 0xf800, 0x07e0, 0x001f, 0);
        assert_eq!(packed.rgba(0xf800), [255, 0, 0, 255]);
        assert_eq!(packed.rgba(0x07e0), [0, 255, 0, 255]);
        assert_eq!(packed.rgba(0x001f), [0, 0, 255, 255]);
        assert_eq!(packed.rgba(0x8410), [132, 130, 132, 255]);
        let tenbit = Packed::from_masks(4, 0x3ff, 0xffc00, 0x3ff0_0000, 0xc000_0000);
        assert_eq!(tenbit.rgba(0xc000_03ff), [255, 0, 0, 255]);
        assert_eq!(tenbit.rgba(0x4000_0000)[3], 85);
    }
}
