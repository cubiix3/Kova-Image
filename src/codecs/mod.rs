//! Decoders for formats the `image` crate does not provide (or does not cover well). Each produces a
//! [`Pending`] canvas that the common path in `decoder.rs` orients, fits and
//! converts to sRGB, so limits and cancellation behave the same everywhere.
use crate::{
    decoder::{Cancellable, Pending, Source, Stamp, Target},
    error::Error,
    format::Format,
    security::Ticket,
};
use std::io::BufReader;

mod av1;
mod dds;
mod heif;
mod hevc;
mod isobmff;
mod jxl;
mod raw;
mod svg;
mod yuv;

/// The file, read in large blocks, that stops reading when the request is stale.
type Reader = BufReader<Cancellable>;

pub(crate) fn decode(
    format: Format,
    source: Box<dyn Source>,
    length: u64,
    stamp: &Stamp,
    ticket: &Ticket,
    target: Target,
) -> Result<Pending, Error> {
    let reader = BufReader::with_capacity(
        64 * 1024,
        Cancellable {
            source,
            ticket: ticket.clone(),
        },
    );
    match format {
        Format::Dds => dds::decode(reader, ticket),
        Format::JpegXl => jxl::decode(reader, ticket, target),
        Format::Svg => svg::decode(reader, ticket, target),
        Format::Raw => raw::decode(reader, length, stamp, ticket, target),
        Format::Avif | Format::Heic => heif::decode(reader, length, ticket, target, format),
        _ => Err(Error::Unsupported),
    }
}
