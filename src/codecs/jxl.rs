//! JPEG XL through jxl-oxide, a pure Rust decoder. The file is read through the
//! cancellable reader, its size is checked from the header before any pixel is
//! rendered, and the decoder's own allocations are capped.
use super::Reader;
use crate::{
    animation::Loops,
    decoder::{Frame, Pending, Target, fit_frame, photo_from_exif},
    error::Error,
    format::Format,
    security::{self, Ticket},
};
use jxl_oxide::{
    AllocTracker, EnumColourEncoding, ExtraChannelType, InitializeResult, JxlImage, Moxcms,
    PixelFormat, RenderingIntent,
};
use std::{io::Read, time::Duration};

fn corrupted(error: impl std::fmt::Display) -> Error {
    let message = error.to_string();
    // The decoder reports a refused allocation as text only.
    if message.contains("failed to allocate") || message.contains("out of memory") {
        return Error::MemoryBudget;
    }
    Error::Corrupted(format!("JPEG XL: {message}"))
}

pub(super) fn decode(
    mut reader: Reader,
    ticket: &Ticket,
    target: Target,
) -> Result<Pending, Error> {
    let mut uninit = JxlImage::builder()
        .alloc_tracker(AllocTracker::with_limit(security::JXL_BUDGET as usize))
        .build_uninit();
    let mut chunk = vec![0u8; 64 * 1024];
    let mut image = loop {
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            return Err(corrupted("the file ends before the image header"));
        }
        // The container parser may take fewer bytes than offered; it keeps what
        // it consumed, and the rest is offered again.
        let mut fed = 0;
        while fed < count {
            let used = uninit.feed_bytes(&chunk[fed..count]).map_err(corrupted)?;
            if used == 0 {
                break;
            }
            fed += used;
        }
        match uninit.try_init().map_err(corrupted)? {
            InitializeResult::NeedMoreData(next) => uninit = next,
            InitializeResult::Initialized(mut image) => {
                // The header is known: refuse a canvas that is too large before
                // reading on, and before any pixel buffer exists.
                security::rgba_bytes(image.width(), image.height())?;
                if fed < count {
                    image.feed_bytes(&chunk[fed..count]).map_err(corrupted)?;
                }
                break image;
            }
        }
    };
    loop {
        ticket.check()?;
        let count = reader.read(&mut chunk)?;
        if count == 0 {
            break;
        }
        image.feed_bytes(&chunk[..count]).map_err(corrupted)?;
    }
    image.finalize().map_err(corrupted)?;
    drop(chunk);

    let (width, height) = (image.width(), image.height());
    let bytes = security::rgba_bytes(width, height)?;
    image.set_cms(Moxcms);
    // The viewer shows sRGB: let the decoder convert, so no profile is left over.
    let gray = image.pixel_format().is_grayscale();
    image.request_color_encoding(if gray {
        EnumColourEncoding::gray_srgb(RenderingIntent::Relative)
    } else {
        EnumColourEncoding::srgb(RenderingIntent::Relative)
    });
    let photo = match image.aux_boxes().first_exif() {
        Ok(data) if data.has_data() => photo_from_exif(Some(data.unwrap().payload())),
        _ => Default::default(),
    };
    let animation = image.image_header().metadata.animation.as_ref();
    let tick = animation.map(|a| {
        Duration::from_secs_f64(
            f64::from(a.tps_denominator.max(1)) / f64::from(a.tps_numerator.max(1)),
        )
    });
    let loops = animation.map_or(Loops(Some(1)), |a| {
        Loops(if a.num_loops == 0 {
            None
        } else {
            Some(a.num_loops)
        })
    });
    let keyframes = image.num_loaded_keyframes();
    if keyframes == 0 {
        return Err(corrupted("no image frames"));
    }
    let animated = keyframes > 1 && tick.is_some();
    let mut frames: Vec<Frame> = Vec::new();
    let mut may_have_alpha = false;
    for index in 0..if animated {
        keyframes.min(security::MAX_FRAMES)
    } else {
        1
    } {
        ticket.check()?;
        if animated && frames.len().saturating_add(1).saturating_mul(bytes) > security::FRAME_BUDGET
        {
            return Err(Error::MemoryBudget);
        }
        let render = image.render_frame(index).map_err(corrupted)?;
        let (alpha, premultiplied) = render
            .extra_channels()
            .0
            .iter()
            .find_map(|c| match c.ty() {
                ExtraChannelType::Alpha { alpha_associated } => Some((true, alpha_associated)),
                _ => None,
            })
            .unwrap_or((false, false));
        may_have_alpha |= alpha;
        let rgba = to_rgba(&render, (width, height), premultiplied, ticket)?;
        let delay = tick.map_or(Duration::from_secs(1), |tick| {
            tick.saturating_mul(render.duration())
                .clamp(Duration::from_millis(10), Duration::from_secs(60))
        });
        frames.push(Frame {
            rgba: if animated {
                fit_frame(rgba, (width, height), target, &None)?
            } else {
                rgba
            },
            delay,
        });
    }
    Ok(Pending {
        format: Format::JpegXl,
        frames,
        width,
        height,
        loops,
        photo,
        may_have_alpha,
        orientation: image::metadata::Orientation::NoTransforms,
        srgb: None,
        fitted: animated,
        // The decoder has already rotated the canvas.
        oriented: true,
        stored: None,
    })
}

/// Interleaved 8-bit RGBA of a rendered frame, converted row by row from the
/// decoder's float planes.
fn to_rgba(
    render: &jxl_oxide::Render,
    (width, height): (u32, u32),
    premultiplied: bool,
    ticket: &Ticket,
) -> Result<Vec<u8>, Error> {
    let mut stream = render.stream();
    if (stream.width(), stream.height()) != (width, height) {
        return Err(Error::Dimensions);
    }
    let channels = stream.channels() as usize;
    let format = match channels {
        1 => PixelFormat::Gray,
        2 => PixelFormat::Graya,
        3 => PixelFormat::Rgb,
        4 => PixelFormat::Rgba,
        _ => return Err(Error::Unsupported),
    };
    let mut rgba = vec![0u8; security::rgba_bytes(width, height)?];
    let mut row = vec![0u8; width as usize * channels];
    for out in rgba.chunks_exact_mut(width as usize * 4) {
        if stream.write_to_buffer(&mut row) != row.len() {
            return Err(corrupted("the frame ends early"));
        }
        for (pixel, source) in out.chunks_exact_mut(4).zip(row.chunks_exact(channels)) {
            let (color, alpha) = match format {
                PixelFormat::Gray => ([source[0]; 3], 255),
                PixelFormat::Graya => ([source[0]; 3], source[1]),
                PixelFormat::Rgb => ([source[0], source[1], source[2]], 255),
                _ => ([source[0], source[1], source[2]], source[3]),
            };
            let color = if premultiplied && alpha != 255 && alpha != 0 {
                color.map(|c| (u32::from(c) * 255 / u32::from(alpha)).min(255) as u8)
            } else {
                color
            };
            pixel.copy_from_slice(&[color[0], color[1], color[2], alpha]);
        }
        ticket.check()?;
    }
    Ok(rgba)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_refused_allocation_is_a_memory_error_not_a_damaged_file() {
        assert!(matches!(
            corrupted("failed to allocate 48000028 byte(s)"),
            Error::MemoryBudget
        ));
        assert!(matches!(corrupted("out of memory"), Error::MemoryBudget));
        assert!(matches!(corrupted("invalid header"), Error::Corrupted(_)));
    }
}
