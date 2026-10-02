//! Image format identification. Content decides; the file extension is used
//! only for formats that have no signature of their own (TGA, SVG) or share one
//! with another format (camera RAW files are TIFF containers).
use image::ImageFormat;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Format {
    Jpeg,
    Png,
    Gif,
    WebP,
    Bmp,
    Tiff,
    Ico,
    Tga,
    Pnm,
    Qoi,
    Dds,
    Hdr,
    Exr,
    Farbfeld,
    JpegXl,
    Avif,
    Heic,
    Svg,
    Raw,
}
impl Format {
    /// Upper-case label for the file information panel.
    pub const fn name(self) -> &'static str {
        match self {
            Self::Jpeg => "JPEG",
            Self::Png => "PNG",
            Self::Gif => "GIF",
            Self::WebP => "WEBP",
            Self::Bmp => "BMP",
            Self::Tiff => "TIFF",
            Self::Ico => "ICO",
            Self::Tga => "TGA",
            Self::Pnm => "PNM",
            Self::Qoi => "QOI",
            Self::Dds => "DDS",
            Self::Hdr => "HDR",
            Self::Exr => "EXR",
            Self::Farbfeld => "FARBFELD",
            Self::JpegXl => "JPEG XL",
            Self::Avif => "AVIF",
            Self::Heic => "HEIC",
            Self::Svg => "SVG",
            Self::Raw => "RAW",
        }
    }
    /// The `image` crate codec that decodes this format, if it is one of theirs.
    pub const fn image(self) -> Option<ImageFormat> {
        Some(match self {
            Self::Jpeg => ImageFormat::Jpeg,
            Self::Png => ImageFormat::Png,
            Self::Gif => ImageFormat::Gif,
            Self::WebP => ImageFormat::WebP,
            Self::Bmp => ImageFormat::Bmp,
            Self::Tiff => ImageFormat::Tiff,
            Self::Ico => ImageFormat::Ico,
            Self::Tga => ImageFormat::Tga,
            Self::Pnm => ImageFormat::Pnm,
            Self::Qoi => ImageFormat::Qoi,
            Self::Hdr => ImageFormat::Hdr,
            Self::Exr => ImageFormat::OpenExr,
            Self::Farbfeld => ImageFormat::Farbfeld,
            _ => return None,
        })
    }
    fn from_image(format: ImageFormat) -> Option<Self> {
        Some(match format {
            ImageFormat::Jpeg => Self::Jpeg,
            ImageFormat::Png => Self::Png,
            ImageFormat::Gif => Self::Gif,
            ImageFormat::WebP => Self::WebP,
            ImageFormat::Bmp => Self::Bmp,
            ImageFormat::Tiff => Self::Tiff,
            ImageFormat::Ico => Self::Ico,
            ImageFormat::Pnm => Self::Pnm,
            ImageFormat::Qoi => Self::Qoi,
            ImageFormat::Dds => Self::Dds,
            ImageFormat::Hdr => Self::Hdr,
            ImageFormat::OpenExr => Self::Exr,
            ImageFormat::Farbfeld => Self::Farbfeld,
            // TGA has no signature, so it is never guessed from content.
            _ => return None,
        })
    }
}

/// Bytes read from the start of a file to identify it. Enough for the list of
/// compatible brands in an ISO base media `ftyp` box and a typical SVG prologue.
pub const SNIFF_BYTES: usize = 2048;

/// Camera RAW extensions. A RAW file is a TIFF (or, for CR3, ISO base media)
/// container, so only the extension tells it from an ordinary TIFF.
pub const RAW_EXTENSIONS: &[&str] = &[
    "3fr", "ari", "arw", "cr2", "cr3", "crw", "dcr", "dng", "erf", "iiq", "kdc", "mef", "mrw",
    "nef", "nrw", "orf", "pef", "raf", "rw2", "rwl", "sr2", "srf", "srw", "x3f",
];
/// Extensions of every image format, for folder listings, the file picker and
/// Open with registration.
pub const IMAGE_EXTENSIONS: &[&str] = &[
    "jpg", "jpeg", "jpe", "png", "apng", "gif", "webp", "bmp", "tif", "tiff", "ico", "tga", "pbm",
    "pgm", "ppm", "pnm", "pam", "qoi", "dds", "hdr", "exr", "ff", "jxl", "avif", "heic", "heif",
    "svg", "svgz", "3fr", "ari", "arw", "cr2", "cr3", "crw", "dcr", "dng", "erf", "iiq", "kdc",
    "mef", "mrw", "nef", "nrw", "orf", "pef", "raf", "rw2", "rwl", "sr2", "srf", "srw", "x3f",
];

fn is(extension: Option<&str>, list: &[&str]) -> bool {
    extension.is_some_and(|e| list.iter().any(|x| e.eq_ignore_ascii_case(x)))
}

/// Brands of an ISO base media file: the major brand and every compatible one.
fn brands(head: &[u8]) -> Option<Vec<[u8; 4]>> {
    if head.get(4..8)? != b"ftyp" {
        return None;
    }
    let size = u32::from_be_bytes(head.get(..4)?.try_into().ok()?) as usize;
    let end = if size < 16 { 16 } else { size.min(head.len()) };
    let mut result = vec![head.get(8..12)?.try_into().ok()?];
    let mut at = 16;
    while let Some(brand) = head.get(at..at + 4).filter(|_| at + 4 <= end) {
        result.push(brand.try_into().ok()?);
        at += 4;
    }
    Some(result)
}
fn is_jpeg_xl(head: &[u8]) -> bool {
    head.starts_with(&[0xff, 0x0a])
        || head.starts_with(&[
            0, 0, 0, 0x0c, b'J', b'X', b'L', b' ', 0x0d, 0x0a, 0x87, 0x0a,
        ])
}
/// `<svg` after an optional byte order mark, XML declaration, comments and
/// DOCTYPE. A prologue longer than the sniffed bytes falls back to the extension.
fn is_svg(head: &[u8]) -> bool {
    let mut rest = head.strip_prefix(&[0xef, 0xbb, 0xbf]).unwrap_or(head);
    loop {
        rest = rest.trim_ascii_start();
        if rest.starts_with(b"<svg") {
            return true;
        }
        let skip = |rest: &[u8], close: &[u8]| {
            rest.windows(close.len())
                .position(|w| w == close)
                .map(|at| at + close.len())
        };
        let next = if rest.starts_with(b"<?") {
            skip(rest, b"?>")
        } else if rest.starts_with(b"<!--") {
            skip(rest, b"-->")
        } else if rest.starts_with(b"<!DOCTYPE") {
            // A DOCTYPE may carry an internal subset in brackets.
            match rest.iter().position(|&b| b == b'[') {
                Some(open) if rest[..open].iter().all(|&b| b != b'>') => skip(rest, b"]>"),
                _ => skip(rest, b">"),
            }
        } else {
            None
        };
        match next {
            Some(n) => rest = &rest[n..],
            None => return false,
        }
    }
}
fn is_pnm(head: &[u8]) -> bool {
    head.len() > 2
        && head[0] == b'P'
        && (b'1'..=b'7').contains(&head[1])
        && head[2].is_ascii_whitespace()
}

/// Identifies a file from its first bytes and extension.
pub fn sniff(head: &[u8], extension: Option<&str>) -> Option<Format> {
    if is_jpeg_xl(head) {
        return Some(Format::JpegXl);
    }
    if let Some(brands) = brands(head) {
        let has = |names: &[&[u8; 4]]| brands.iter().any(|b| names.contains(&b));
        if has(&[b"crx "]) {
            return Some(Format::Raw);
        }
        if has(&[b"avif", b"avis"]) {
            return Some(Format::Avif);
        }
        if has(&[
            b"heic", b"heix", b"hevc", b"hevx", b"heim", b"heis", b"mif1", b"msf1",
        ]) {
            return Some(Format::Heic);
        }
        return None;
    }
    // A RAW file starts like a TIFF, so its extension must win over the signature.
    if is(extension, RAW_EXTENSIONS) {
        return Some(Format::Raw);
    }
    if is(extension, &["svgz"]) && head.starts_with(&[0x1f, 0x8b]) {
        return Some(Format::Svg);
    }
    if head.starts_with(&[0x1f, 0x8b]) {
        return None;
    }
    if let Ok(guess) = image::guess_format(head)
        && let Some(format) = Format::from_image(guess)
    {
        // Seven bytes of "P1".."P7" match many text files; demand the whole token.
        if format != Format::Pnm || is_pnm(head) {
            return Some(format);
        }
    }
    if is_svg(head) || (is(extension, &["svg"]) && !head.contains(&0)) {
        return Some(Format::Svg);
    }
    if is(extension, &["tga"]) {
        return Some(Format::Tga);
    }
    None
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn signatures_decide_before_extensions() {
        assert_eq!(sniff(b"\x89PNG\r\n\x1a\n", Some("svg")), Some(Format::Png));
        assert_eq!(sniff(b"\xff\x0a\0\0", None), Some(Format::JpegXl));
        assert_eq!(
            sniff(b"\0\0\0\x0cJXL \r\n\x87\n", Some("jpg")),
            Some(Format::JpegXl)
        );
        assert_eq!(sniff(b"P6\n1 1\n255\n", None), Some(Format::Pnm));
        assert_eq!(sniff(b"P6 is a text file", Some("txt")), Some(Format::Pnm));
        assert_eq!(sniff(b"P6x not a netpbm header", None), None);
        assert_eq!(sniff(b"qoif\0\0\0\x01", None), Some(Format::Qoi));
    }
    #[test]
    fn iso_base_media_brands_pick_the_image_format() {
        let ftyp = |major: &[u8; 4], compatible: &[&[u8; 4]]| {
            let mut v = Vec::new();
            v.extend_from_slice(&((16 + 4 * compatible.len()) as u32).to_be_bytes());
            v.extend_from_slice(b"ftyp");
            v.extend_from_slice(major);
            v.extend_from_slice(&[0; 4]);
            for brand in compatible {
                v.extend_from_slice(*brand);
            }
            v
        };
        assert_eq!(sniff(&ftyp(b"avif", &[]), None), Some(Format::Avif));
        assert_eq!(sniff(&ftyp(b"avis", &[b"avif"]), None), Some(Format::Avif));
        assert_eq!(sniff(&ftyp(b"heic", &[b"mif1"]), None), Some(Format::Heic));
        // A generic HEIF file that lists avif among its brands is an AVIF.
        assert_eq!(
            sniff(&ftyp(b"mif1", &[b"miaf", b"avif"]), None),
            Some(Format::Avif)
        );
        assert_eq!(sniff(&ftyp(b"crx ", &[b"isom"]), None), Some(Format::Raw));
        // Video brands are not images.
        assert_eq!(sniff(&ftyp(b"isom", &[b"mp42"]), None), None);
    }
    #[test]
    fn raw_and_tga_need_their_extension() {
        let tiff = b"II*\0\x08\0\0\0";
        assert_eq!(sniff(tiff, Some("tif")), Some(Format::Tiff));
        assert_eq!(sniff(tiff, Some("ARW")), Some(Format::Raw));
        assert_eq!(sniff(tiff, Some("dng")), Some(Format::Raw));
        assert_eq!(sniff(&[0; 18], Some("tga")), Some(Format::Tga));
        assert_eq!(sniff(&[0; 18], Some("dat")), None);
    }
    #[test]
    fn svg_is_found_after_a_prologue() {
        assert_eq!(sniff(b"<svg xmlns='x'/>", None), Some(Format::Svg));
        let prologue = b"\xef\xbb\xbf<?xml version='1.0'?>\n<!-- c -->\n<!DOCTYPE svg [<!ENTITY a 'b'>]>\n<svg/>";
        assert_eq!(sniff(prologue, None), Some(Format::Svg));
        assert_eq!(sniff(b"<html><svg/></html>", None), None);
        // A prologue longer than the sniffed bytes is accepted by extension only.
        assert_eq!(sniff(b"<?xml version", Some("svg")), Some(Format::Svg));
        assert_eq!(sniff(b"<?xml version", None), None);
        assert_eq!(sniff(b"MZ\0\0", Some("svg")), None);
        assert_eq!(sniff(b"\x1f\x8b\x08", Some("svgz")), Some(Format::Svg));
        assert_eq!(sniff(b"\x1f\x8b\x08", Some("gz")), None);
    }
    #[test]
    fn every_listed_extension_is_known() {
        for extension in IMAGE_EXTENSIONS {
            let known = match *extension {
                "jpg" | "jpeg" | "jpe" | "png" | "apng" | "gif" | "webp" | "bmp" | "tif"
                | "tiff" | "ico" | "tga" | "pbm" | "pgm" | "ppm" | "pnm" | "pam" | "qoi"
                | "dds" | "hdr" | "exr" | "ff" | "jxl" | "avif" | "heic" | "heif" | "svg"
                | "svgz" => true,
                other => RAW_EXTENSIONS.contains(&other),
            };
            assert!(known, "{extension}");
        }
        for extension in RAW_EXTENSIONS {
            assert!(IMAGE_EXTENSIONS.contains(extension), "{extension}");
        }
    }
}
