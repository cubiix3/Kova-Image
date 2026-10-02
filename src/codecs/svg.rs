//! SVG through resvg. A vector image has no pixel size, so it is rendered at the
//! size the view needs (sharp at any zoom the view asks for) instead of at its
//! own size. The renderer is given nothing from the outside world: files and
//! network addresses referenced by the document are not read, only pictures
//! embedded in the document itself, and those are size-checked first.
use super::Reader;
use crate::{
    animation::Loops,
    decoder::{Frame, Pending, Target},
    error::Error,
    format::Format,
    security::{self, Ticket},
};
use resvg::{tiny_skia, usvg};
use std::{
    io::Read,
    sync::{
        Arc, OnceLock,
        atomic::{AtomicU64, Ordering},
    },
    time::Duration,
};

/// Largest SVG document, and largest result of unpacking an `.svgz`.
const MAX_BYTES: u64 = 16 * 1024 * 1024;
/// Pixels of all pictures embedded in one document together.
const MAX_EMBEDDED_PIXELS: u64 = 64 * 1024 * 1024;
/// Longest side the document is rendered at, however large the view is.
const MAX_SIDE: f32 = 8192.0;
/// Without a size from the view, a vector image is rendered at least this wide.
const FULL_SIDE: f32 = 2048.0;

fn bad(message: impl std::fmt::Display) -> Error {
    Error::Corrupted(format!("SVG: {message}"))
}

/// System fonts, loaded once and only for a document that contains text.
fn fonts() -> Arc<usvg::fontdb::Database> {
    static FONTS: OnceLock<Arc<usvg::fontdb::Database>> = OnceLock::new();
    FONTS
        .get_or_init(|| {
            let mut database = usvg::fontdb::Database::new();
            database.load_system_fonts();
            database.set_serif_family("Times New Roman");
            database.set_sans_serif_family("Arial");
            database.set_monospace_family("Courier New");
            Arc::new(database)
        })
        .clone()
}

/// Accepts only pictures that are part of the document (`data:` URLs), and only
/// while their combined pixel count stays bounded. Everything else, such as a
/// path or address, resolves to nothing.
fn resolver() -> usvg::ImageHrefResolver<'static> {
    let embedded = Arc::new(AtomicU64::new(0));
    let default = usvg::ImageHrefResolver::default_data_resolver();
    usvg::ImageHrefResolver {
        resolve_data: Box::new(move |mime, data, options| {
            // Nested documents are parsed by usvg with the same restrictions.
            if mime != "image/svg+xml" {
                let size = image::ImageReader::new(std::io::Cursor::new(data.as_slice()))
                    .with_guessed_format()
                    .ok()?
                    .into_dimensions()
                    .ok()?;
                let pixels = u64::from(size.0) * u64::from(size.1);
                let total = embedded.fetch_add(pixels, Ordering::Relaxed) + pixels;
                if total > MAX_EMBEDDED_PIXELS {
                    return None;
                }
            }
            default(mime, data, options)
        }),
        resolve_string: Box::new(|_, _| None),
    }
}

/// Size of the bitmap for a document of `size`: filled into the view when there
/// is one (enlarging a small drawing, since vectors stay sharp), otherwise
/// large enough to zoom into. Always within the pixel limits.
fn render_size(size: (f32, f32), target: Target) -> (u32, u32) {
    let (width, height) = (size.0.max(1.0), size.1.max(1.0));
    let mut scale = if target.max_width != 0 && target.max_height != 0 {
        (target.max_width as f32 / width).min(target.max_height as f32 / height)
    } else {
        (FULL_SIDE / width.max(height)).clamp(1.0, 64.0)
    };
    scale = scale.min(MAX_SIDE / width.max(height));
    let side =
        |value: f32, scale: f32| ((value * scale).round() as u32).clamp(1, security::MAX_DIMENSION);
    let mut size = (side(width, scale), side(height, scale));
    // Rounding up can step over the limit; shrink a little until it fits.
    while u64::from(size.0) * u64::from(size.1) > security::MAX_PIXELS {
        scale *= 0.99;
        size = (side(width, scale), side(height, scale));
    }
    size
}

pub(super) fn decode(
    mut reader: Reader,
    ticket: &Ticket,
    target: Target,
) -> Result<Pending, Error> {
    let mut data = Vec::new();
    (&mut reader).take(MAX_BYTES + 1).read_to_end(&mut data)?;
    if data.len() as u64 > MAX_BYTES {
        return Err(Error::Io("SVG files over 16 MiB are not opened".into()));
    }
    if data.starts_with(&[0x1f, 0x8b]) {
        let mut unpacked = Vec::new();
        flate2::read::GzDecoder::new(data.as_slice())
            .take(MAX_BYTES + 1)
            .read_to_end(&mut unpacked)
            .map_err(|_| bad("the compressed file is damaged"))?;
        if unpacked.len() as u64 > MAX_BYTES {
            return Err(Error::Io("SVG files over 16 MiB are not opened".into()));
        }
        data = unpacked;
    }
    ticket.check()?;
    let mut options = usvg::Options {
        resources_dir: None,
        image_href_resolver: resolver(),
        ..Default::default()
    };
    // Text needs fonts, and finding them is slow, so only a document with text
    // pays for it.
    if data.windows(5).any(|w| w == b"<text") {
        options.fontdb = fonts();
    }
    let tree = usvg::Tree::from_data(&data, &options).map_err(bad)?;
    drop(data);
    ticket.check()?;
    let size = tree.size();
    let (iw, ih) = (size.width(), size.height());
    if !(iw.is_finite() && ih.is_finite()) || iw <= 0.0 || ih <= 0.0 {
        return Err(bad("the drawing has no size"));
    }
    let (width, height) = render_size((iw, ih), target);
    let scale = (width as f32 / iw, height as f32 / ih);
    let mut pixmap = tiny_skia::Pixmap::new(width, height).ok_or(Error::Dimensions)?;
    resvg::render(
        &tree,
        tiny_skia::Transform::from_scale(scale.0, scale.1),
        &mut pixmap.as_mut(),
    );
    ticket.check()?;
    // The renderer works in premultiplied alpha.
    let mut rgba = pixmap.take();
    for pixel in rgba.chunks_exact_mut(4) {
        let alpha = u32::from(pixel[3]);
        if alpha != 0 && alpha != 255 {
            for c in &mut pixel[..3] {
                *c = ((u32::from(*c) * 255 + alpha / 2) / alpha).min(255) as u8;
            }
        }
    }
    // The size the picture counts as having: its own, within the limits.
    let natural = |value: f32| value.round().clamp(1.0, security::MAX_DIMENSION as f32) as u32;
    let source = (natural(iw), natural(ih));
    Ok(Pending {
        format: Format::Svg,
        frames: vec![Frame {
            rgba,
            delay: Duration::from_secs(1),
        }],
        width: source.0,
        height: source.1,
        loops: Loops(Some(1)),
        photo: Default::default(),
        may_have_alpha: true,
        orientation: image::metadata::Orientation::NoTransforms,
        srgb: None,
        fitted: true,
        oriented: true,
        stored: Some((width, height)),
    })
}
