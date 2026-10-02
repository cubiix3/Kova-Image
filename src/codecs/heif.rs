//! AVIF (and, through the same container code, HEIC): the primary item of the
//! file, which is either one coded picture or a grid of tiles, with an optional
//! alpha picture, a clean aperture, rotation and mirroring, and colour
//! information. Pixels come from the AV1 decoder in `av1.rs`.
use super::{
    Reader, av1, hevc,
    isobmff::{Clap, Colr, Container, Item},
    yuv::{self, Layout, Planar},
};
use crate::{
    animation::Loops,
    decoder::{Frame, Pending, Srgb, Target, photo_from_exif, srgb_transform},
    error::Error,
    format::Format,
    security::{self, Ticket},
};
use std::time::Duration;

/// A grid of at most this many tiles; real files have a few dozen.
const MAX_TILES: usize = 4096;

fn bad(message: &str) -> Error {
    Error::Corrupted(format!("HEIF: {message}"))
}

/// Pixels of the colour channels, or an alpha plane (one byte per pixel).
struct Canvas {
    width: u32,
    height: u32,
    pixels: Vec<u8>,
    /// Colour description of the first decoded picture, when the container
    /// does not give one.
    coded: Option<Colour>,
}
#[derive(Clone, Debug)]
enum Colour {
    Codes { primaries: u16, transfer: u16 },
    Icc(Vec<u8>),
}
impl From<&Colr> for Colour {
    fn from(colr: &Colr) -> Self {
        match colr {
            Colr::Nclx {
                primaries,
                transfer,
                ..
            } => Self::Codes {
                primaries: *primaries,
                transfer: *transfer,
            },
            Colr::Icc(profile) => Self::Icc(profile.clone()),
        }
    }
}

struct Context<'a> {
    container: &'a Container,
    ticket: &'a Ticket,
    /// Colour the container declares for the primary item.
    declared: Option<Colour>,
}

pub(super) fn decode(
    mut reader: Reader,
    length: u64,
    ticket: &Ticket,
    _target: Target,
    format: Format,
) -> Result<Pending, Error> {
    let container = Container::parse(&mut reader, length, ticket)?;
    let primary = container
        .item(container.primary)
        .ok_or_else(|| bad("the primary item is missing"))?;
    let declared = primary.colr().map(Colour::from);
    let context = Context {
        container: &container,
        ticket,
        declared,
    };
    let mut color = context.item(&mut reader, primary, false, format, 0)?;
    security::rgba_bytes(color.width, color.height)?;

    // Alpha is a second picture that points at the first.
    let alpha_id = container.references_to(b"auxl", primary.id).find(|id| {
        container
            .item(*id)
            .and_then(Item::aux_type)
            .is_some_and(is_alpha_urn)
    });
    let mut has_alpha = false;
    if let Some(id) = alpha_id {
        let item = container
            .item(id)
            .ok_or_else(|| bad("the alpha item is missing"))?;
        let alpha = context.item(&mut reader, item, true, format, 0)?;
        if (alpha.width, alpha.height) != (color.width, color.height) {
            return Err(bad("the alpha picture has another size"));
        }
        let premultiplied = container
            .references_from(b"prem", primary.id)
            .any(|t| t == id);
        for (pixel, a) in color.pixels.chunks_exact_mut(4).zip(&alpha.pixels) {
            if premultiplied && *a != 0 && *a != 255 {
                for c in &mut pixel[..3] {
                    *c = (u32::from(*c) * 255 / u32::from(*a)).min(255) as u8;
                }
            }
            pixel[3] = *a;
        }
        has_alpha = true;
    }
    ticket.check()?;

    let (mut width, mut height) = (color.width, color.height);
    if let Some(clap) = primary.clap()
        && let Some((x, y, w, h)) = aperture(clap, width, height)
    {
        color.pixels = crop(&color.pixels, width, (x, y, w, h));
        (width, height) = (w, h);
    }
    // Exif is a separate item that describes the primary one.
    let photo = exif_of(&context, &mut reader, primary.id).unwrap_or_default();

    let colour = context.declared.clone().or(color.coded.clone());
    Ok(Pending {
        format,
        frames: vec![Frame {
            rgba: color.pixels,
            delay: Duration::from_secs(1),
        }],
        width,
        height,
        loops: Loops(Some(1)),
        photo,
        may_have_alpha: has_alpha,
        orientation: primary.orientation(),
        srgb: colour.as_ref().and_then(transform),
        fitted: false,
        oriented: false,
        stored: None,
    })
}

impl Context<'_> {
    /// One picture, as RGBA (`alpha` false) or as a plane of alpha values.
    fn item(
        &self,
        reader: &mut Reader,
        item: &Item,
        alpha: bool,
        format: Format,
        depth: u8,
    ) -> Result<Canvas, Error> {
        self.ticket.check()?;
        match &item.kind {
            b"av01" => self.coded(reader, item, alpha),
            b"hvc1" | b"hev1" => self.coded_hevc(reader, item, alpha),
            b"grid" if depth == 0 => self.grid(reader, item, alpha, format),
            b"iden" if depth == 0 => {
                let source = self
                    .container
                    .references_from(b"dimg", item.id)
                    .next()
                    .and_then(|id| self.container.item(id))
                    .ok_or_else(|| bad("a derived image has no source"))?;
                self.item(reader, source, alpha, format, depth + 1)
            }
            _ => Err(Error::Unsupported),
        }
    }

    fn coded(&self, reader: &mut Reader, item: &Item, alpha: bool) -> Result<Canvas, Error> {
        if let Some((w, h)) = item.ispe() {
            // The declared size is checked before any data is read or decoded.
            security::rgba_bytes(w, h)?;
        }
        let data = self.container.read_item(reader, item.id, self.ticket)?;
        // The container's colour wins over the stream's own description.
        let declared = item.colr().or_else(|| {
            self.container
                .item(self.container.primary)
                .and_then(Item::colr)
        });
        let want = if alpha {
            av1::Want::Alpha
        } else if let Some(Colr::Nclx {
            matrix, full_range, ..
        }) = declared
        {
            av1::Want::Colour(Some((*matrix, *full_range)))
        } else {
            av1::Want::Colour(None)
        };
        let output = av1::decode(data, want, self.ticket)?;
        self.ticket.check()?;
        let (pw, ph) = (output.width, output.height);
        let (width, height) = match item.ispe() {
            Some((w, h)) if w > pw || h > ph => {
                return Err(bad("a picture is smaller than its declared size"));
            }
            Some(size) => size,
            None => (pw, ph),
        };
        let stride = if alpha { 1 } else { 4 };
        let pixels = if (pw, ph) == (width, height) {
            output.pixels
        } else {
            crop_bytes(&output.pixels, pw, (0, 0, width, height), stride)
        };
        Ok(Canvas {
            width,
            height,
            pixels,
            coded: Some(Colour::Codes {
                primaries: output.coded.primaries,
                transfer: output.coded.transfer,
            }),
        })
    }

    /// A picture coded with HEVC, decoded by the decoder in `hevc`.
    fn coded_hevc(&self, reader: &mut Reader, item: &Item, alpha: bool) -> Result<Canvas, Error> {
        if let Some((w, h)) = item.ispe() {
            security::rgba_bytes(w, h)?;
        }
        let config = item
            .config(b"hvcC")
            .ok_or_else(|| bad("an HEVC item has no decoder configuration"))?;
        let data = self.container.read_item(reader, item.id, self.ticket)?;
        let frame = hevc::decode(config, &data, self.ticket)?;
        drop(data);
        self.ticket.check()?;
        let (pw, ph) = (frame.width, frame.height);
        let (width, height) = match item.ispe() {
            Some((w, h)) if w > pw || h > ph => {
                return Err(bad("a picture is smaller than its declared size"));
            }
            Some(size) => size,
            None => (pw, ph),
        };
        // The container's colour wins over the stream's own description.
        let declared = item.colr().or_else(|| {
            self.container
                .item(self.container.primary)
                .and_then(Item::colr)
        });
        let (matrix, full_range) = match (declared, frame.colour) {
            (
                Some(Colr::Nclx {
                    matrix, full_range, ..
                }),
                _,
            ) => (*matrix, *full_range),
            (_, Some(c)) => (u16::from(c.matrix), c.full_range),
            // Nothing says: the usual for pictures of this kind.
            _ => (6, true),
        };
        let coded = match (declared, frame.colour) {
            (None, Some(c)) => Some(Colour::Codes {
                primaries: u16::from(c.primaries),
                transfer: u16::from(c.transfer),
            }),
            _ => None,
        };
        let stride = if alpha { 1 } else { 4 };
        let pixels = if alpha {
            yuv::to_alpha(&frame, full_range)
        } else {
            yuv::to_rgba(&frame, matrix, full_range)?
        };
        let pixels = if (pw, ph) == (width, height) {
            pixels
        } else {
            crop_bytes(&pixels, pw, (0, 0, width, height), stride)
        };
        Ok(Canvas {
            width,
            height,
            pixels,
            coded,
        })
    }

    fn grid(
        &self,
        reader: &mut Reader,
        item: &Item,
        alpha: bool,
        format: Format,
    ) -> Result<Canvas, Error> {
        let data = self.container.read_item(reader, item.id, self.ticket)?;
        if data.len() < 8 {
            return Err(bad("a grid description is cut short"));
        }
        let (rows, columns) = (usize::from(data[2]) + 1, usize::from(data[3]) + 1);
        let (width, height) = if data[1] & 1 == 1 {
            if data.len() < 12 {
                return Err(bad("a grid description is cut short"));
            }
            (
                u32::from_be_bytes(data[4..8].try_into().unwrap()),
                u32::from_be_bytes(data[8..12].try_into().unwrap()),
            )
        } else {
            (
                u32::from(u16::from_be_bytes(data[4..6].try_into().unwrap())),
                u32::from(u16::from_be_bytes(data[6..8].try_into().unwrap())),
            )
        };
        let bytes = security::rgba_bytes(width, height)?;
        if rows * columns > MAX_TILES {
            return Err(bad("the grid has too many tiles"));
        }
        let tiles: Vec<u32> = self.container.references_from(b"dimg", item.id).collect();
        if tiles.len() != rows * columns {
            return Err(bad("the grid and its tiles do not match"));
        }
        let stride = if alpha { 1 } else { 4 };
        let mut pixels = vec![0u8; bytes / 4 * stride];
        let mut size = None;
        let mut coded = None;
        for (index, id) in tiles.into_iter().enumerate() {
            let tile_item = self
                .container
                .item(id)
                .ok_or_else(|| bad("a grid tile is missing"))?;
            let tile = self.item(reader, tile_item, alpha, format, 1)?;
            let (tw, th) = *size.get_or_insert((tile.width, tile.height));
            if (tile.width, tile.height) != (tw, th) {
                return Err(bad("grid tiles differ in size"));
            }
            coded = coded.or(tile.coded);
            let (x0, y0) = (
                (index % columns) as u64 * u64::from(tw),
                (index / columns) as u64 * u64::from(th),
            );
            if x0 >= u64::from(width) || y0 >= u64::from(height) {
                continue;
            }
            let (x0, y0) = (x0 as usize, y0 as usize);
            let copy = (tw as usize).min(width as usize - x0) * stride;
            for row in 0..(th as usize).min(height as usize - y0) {
                let from = row * tw as usize * stride;
                let to = ((y0 + row) * width as usize + x0) * stride;
                pixels[to..to + copy].copy_from_slice(&tile.pixels[from..from + copy]);
            }
        }
        Ok(Canvas {
            width,
            height,
            pixels,
            coded,
        })
    }
}

/// The auxiliary picture types that mean "alpha": the AV1 and MIAF names, and
/// the one HEVC uses.
fn is_alpha_urn(urn: &str) -> bool {
    urn.contains("alpha") || urn == "urn:mpeg:hevc:2015:auxid:1"
}

/// Window of the clean aperture in whole pixels: left, top, width, height.
fn aperture(clap: Clap, width: u32, height: u32) -> Option<(u32, u32, u32, u32)> {
    let fraction = |(n, d): (u32, u32)| (d != 0).then(|| f64::from(n) / f64::from(d));
    let signed = |(n, d): (i32, u32)| (d != 0).then(|| f64::from(n) / f64::from(d));
    let (w, h) = (fraction(clap.width)?, fraction(clap.height)?);
    let (dx, dy) = (signed(clap.x)?, signed(clap.y)?);
    let left = (f64::from(width) - w) / 2.0 + dx;
    let top = (f64::from(height) - h) / 2.0 + dy;
    let (left, top) = (left.round().max(0.0) as u32, top.round().max(0.0) as u32);
    let (w, h) = (w.round() as u32, h.round() as u32);
    (w > 0 && h > 0 && left < width && top < height)
        .then(|| (left, top, w.min(width - left), h.min(height - top)))
}
fn crop(rgba: &[u8], width: u32, window: (u32, u32, u32, u32)) -> Vec<u8> {
    crop_bytes(rgba, width, window, 4)
}
fn crop_bytes(
    pixels: &[u8],
    width: u32,
    (x, y, w, h): (u32, u32, u32, u32),
    stride: usize,
) -> Vec<u8> {
    let mut out = Vec::with_capacity(w as usize * h as usize * stride);
    for row in y..y + h {
        let from = (row as usize * width as usize + x as usize) * stride;
        out.extend_from_slice(&pixels[from..from + w as usize * stride]);
    }
    out
}

/// Date, camera and exposure from the Exif item that describes `target`.
fn exif_of(
    context: &Context,
    reader: &mut Reader,
    target: u32,
) -> Option<crate::decoder::PhotoInfo> {
    let id = context
        .container
        .references_to(b"cdsc", target)
        .find(|id| {
            context
                .container
                .item(*id)
                .is_some_and(|i| &i.kind == b"Exif")
        })?;
    let data = context
        .container
        .read_item(reader, id, context.ticket)
        .ok()?;
    // A four-byte offset to the TIFF header precedes the Exif data.
    let offset = u32::from_be_bytes(data.get(..4)?.try_into().ok()?) as usize;
    Some(photo_from_exif(data.get(4usize.checked_add(offset)?..)))
}

/// Conversion from the picture's colour to sRGB, when it is not sRGB already.
fn transform(colour: &Colour) -> Srgb {
    match colour {
        Colour::Icc(profile) => srgb_transform(Some(profile)),
        Colour::Codes {
            primaries,
            transfer,
        } => {
            // BT.709 primaries with a BT.709 or sRGB curve, or nothing specified,
            // are what viewers treat as sRGB.
            if matches!(primaries, 0..=2) && matches!(transfer, 0..=2 | 6 | 13) {
                return None;
            }
            let profile = moxcms::ColorProfile::new_from_cicp(moxcms::CicpProfile {
                color_primaries: moxcms::CicpColorPrimaries::try_from(
                    u8::try_from(*primaries).ok()?,
                )
                .ok()?,
                transfer_characteristics: moxcms::TransferCharacteristics::try_from(
                    u8::try_from(*transfer).ok()?,
                )
                .ok()?,
                matrix_coefficients: moxcms::MatrixCoefficients::Identity,
                full_range: true,
            });
            profile
                .create_in_place_transform_8bit(
                    moxcms::Layout::Rgba,
                    &moxcms::ColorProfile::new_srgb(),
                    moxcms::TransformOptions::default(),
                )
                .ok()
        }
    }
}

impl Planar for hevc::Frame {
    fn size(&self) -> (usize, usize) {
        (self.width as usize, self.height as usize)
    }
    fn layout(&self) -> Layout {
        match self.chroma_format {
            0 => Layout::Mono,
            1 => Layout::Yuv420,
            2 => Layout::Yuv422,
            _ => Layout::Yuv444,
        }
    }
    fn depth(&self) -> u32 {
        u32::from(self.bit_depth)
    }
    fn row(&self, index: usize, y: usize, out: &mut [u16]) {
        let width = self.plane_width[index];
        out[..width].copy_from_slice(&self.planes[index][y * width..(y + 1) * width]);
    }
}
