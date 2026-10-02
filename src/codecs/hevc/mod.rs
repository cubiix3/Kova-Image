//! A decoder for HEVC (H.265) intra pictures, as stored in HEIC files. It covers
//! what a still picture needs: intra prediction, the integer transforms, the
//! CABAC entropy decoder and the in-loop filters, for the Main, Main 10, Main
//! Still Picture and the 4:0:0, 4:2:2 and 4:4:4 range extension profiles.
//!
//! Not supported (and reported as unsupported): predicted slices, tiles,
//! separate colour planes, cross-component prediction and the extended
//! precision and bypass alignment tools of the range extensions.
//!
//! HEVC is covered by patents of the Access Advance and Via LA pools, and of
//! others. This code implements the public specification and carries no
//! licence for them; see docs/DEPENDENCIES.md.
mod bits;
mod cabac;
mod ctu;
mod filter;
mod intra;
mod params;
mod picture;
mod slice;
mod transform;

use crate::{
    error::Error,
    security::{self, Ticket},
};
use bits::{bad, unescape};
use ctu::{Segment, Stores};
use params::{Pps, Sps, parse_pps, parse_sps};
use picture::{Picture, SliceInfo};
use std::collections::HashMap;

/// The colour description inside the stream (VUI), when it has one.
#[derive(Clone, Copy, Debug)]
pub struct StreamColour {
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub full_range: bool,
}

/// A decoded picture, cropped to its output window.
pub struct Frame {
    pub width: u32,
    pub height: u32,
    /// 0 monochrome, 1 for 4:2:0, 2 for 4:2:2, 3 for 4:4:4.
    pub chroma_format: u8,
    pub bit_depth: u8,
    pub planes: [Vec<u16>; 3],
    pub plane_width: [usize; 3],
    pub colour: Option<StreamColour>,
}

struct Nal<'a> {
    kind: u8,
    layer: u8,
    /// The unit without its two header bytes, still with emulation prevention.
    payload: &'a [u8],
}

fn parse_nal(unit: &[u8]) -> Result<Nal<'_>, Error> {
    if unit.len() < 2 || unit[0] & 0x80 != 0 {
        return Err(bad("a NAL unit header is invalid"));
    }
    Ok(Nal {
        kind: unit[0] >> 1 & 0x3f,
        layer: (unit[0] & 1) << 5 | unit[1] >> 3,
        payload: &unit[2..],
    })
}

/// The NAL units of an `hvcC` box: (type, unit) pairs, and the size in bytes of
/// the length prefix that item data uses.
fn configuration(config: &[u8]) -> Result<(Vec<Vec<u8>>, usize), Error> {
    if config.len() < 23 || config[0] != 1 {
        return Err(bad("the decoder configuration is invalid"));
    }
    let length_size = usize::from(config[21] & 3) + 1;
    let arrays = usize::from(config[22]);
    let mut at = 23;
    let mut units = Vec::new();
    for _ in 0..arrays {
        let header = config
            .get(at..at + 3)
            .ok_or_else(|| bad("the configuration is cut short"))?;
        let count = usize::from(u16::from_be_bytes([header[1], header[2]]));
        at += 3;
        for _ in 0..count {
            let size = config
                .get(at..at + 2)
                .map(|b| usize::from(u16::from_be_bytes([b[0], b[1]])))
                .ok_or_else(|| bad("the configuration is cut short"))?;
            let unit = config
                .get(at + 2..at + 2 + size)
                .ok_or_else(|| bad("the configuration is cut short"))?;
            units.push(unit.to_vec());
            at += 2 + size;
        }
    }
    Ok((units, length_size))
}

/// Decodes the picture of one HEIF item: `config` is the `hvcC` payload and
/// `data` the item data, a sequence of length-prefixed NAL units.
pub fn decode(config: &[u8], data: &[u8], ticket: &Ticket) -> Result<Frame, Error> {
    let (parameter_units, length_size) = configuration(config)?;
    let mut sps_units = Vec::new();
    let mut pps_units: HashMap<u32, Vec<u8>> = HashMap::new();
    let mut slices: Vec<Vec<u8>> = Vec::new();
    // Parameter sets may also be repeated in the item data itself.
    let mut units: Vec<Vec<u8>> = parameter_units;
    let mut at = 0;
    while at < data.len() {
        let prefix = data
            .get(at..at + length_size)
            .ok_or_else(|| bad("a NAL unit length is cut short"))?;
        let size = prefix.iter().fold(0usize, |n, b| n << 8 | usize::from(*b));
        at += length_size;
        let unit = data
            .get(
                at..at
                    .checked_add(size)
                    .ok_or_else(|| bad("a NAL unit is too large"))?,
            )
            .ok_or_else(|| bad("a NAL unit runs past the data"))?;
        units.push(unit.to_vec());
        at += size;
        if units.len() > 100_000 {
            return Err(bad("too many NAL units"));
        }
    }
    for unit in units {
        let nal = parse_nal(&unit)?;
        if nal.layer != 0 {
            continue;
        }
        match nal.kind {
            33 => sps_units.push(unescape(nal.payload).0),
            34 => {
                let rbsp = unescape(nal.payload).0;
                let id = bits::Bits::new(&rbsp).ue()?;
                pps_units.insert(id, rbsp);
            }
            0..=9 | 16..=21 => slices.push(unit),
            _ => {}
        }
    }
    if slices.is_empty() {
        return Err(bad("the item holds no picture data"));
    }
    // The SPS that the first slice's PPS names.
    let first = parse_nal(&slices[0])?;
    let first_rbsp = unescape(first.payload).0;
    let pps_id = slice::peek_pps_id(&first_rbsp, first.kind)?;
    let pps_rbsp = pps_units
        .get(&pps_id)
        .ok_or_else(|| bad("a slice refers to a missing picture parameter set"))?;
    let sps_id = {
        let mut bits = bits::Bits::new(pps_rbsp);
        bits.ue()?;
        bits.ue()?
    };
    let sps = sps_units
        .iter()
        .map(|rbsp| parse_sps(rbsp))
        .collect::<Result<Vec<Sps>, _>>()?
        .into_iter()
        .find(|s| s.id == sps_id)
        .ok_or_else(|| bad("a picture refers to a missing sequence parameter set"))?;
    let pps = parse_pps(pps_rbsp, sps.chroma_array_type == 3)?;
    check_supported(&sps, &pps)?;
    security::rgba_bytes(sps.width, sps.height)?;
    let samples = u64::from(sps.width) * u64::from(sps.height);
    let planes = if sps.chroma_array_type == 0 { 1 } else { 3 };
    if samples * 2 * planes > security::DECODE_BUDGET {
        return Err(Error::MemoryBudget);
    }

    let mut picture = Picture::new(&sps, &pps);
    let mut stores = Stores::default();
    let mut last_independent: Option<slice::SliceHeader> = None;
    let mut slice_index = -1i32;
    let mut slice_address = 0u32;
    for unit in &slices {
        ticket.check()?;
        let nal = parse_nal(unit)?;
        let rbsp = unescape(nal.payload).0;
        if slice::peek_pps_id(&rbsp, nal.kind)? != pps_id {
            return Err(Error::Unsupported);
        }
        let header = slice::parse(&rbsp, nal.kind, &sps, &pps, last_independent.as_ref())?;
        if !header.dependent {
            slice_address = header.segment_address;
            picture.slices.push(SliceInfo {
                address: slice_address,
                deblocking_disabled: header.deblocking_disabled,
                beta_offset_div2: header.beta_offset_div2,
                tc_offset_div2: header.tc_offset_div2,
                loop_filter_across_slices: header.loop_filter_across_slices,
            });
            slice_index = picture.slices.len() as i32 - 1;
            last_independent = Some(header.clone());
            stores.segment = None;
        } else if slice_index < 0 {
            return Err(bad("a dependent slice has no slice before it"));
        }
        let carry = if header.dependent {
            stores.segment.take()
        } else {
            None
        };
        Segment::new(
            &mut picture,
            &header,
            slice_index,
            slice_address,
            &rbsp,
            carry,
            ticket,
        )
        .decode(&mut stores)?;
    }
    if picture.ctb_slice.contains(&-1) {
        return Err(bad("the picture is not complete"));
    }
    ticket.check()?;
    picture.deblock();
    picture.apply_sao();
    Ok(output(&sps, picture))
}

fn check_supported(sps: &Sps, pps: &Pps) -> Result<(), Error> {
    if sps.separate_colour_planes
        || sps.bit_depth_luma > 12
        || sps.bit_depth_chroma > 12
        || sps.bit_depth_luma != sps.bit_depth_chroma && sps.chroma_array_type != 0
        || sps.range.extended_precision
        || sps.range.cabac_bypass_alignment
        || pps.cross_component_prediction
        || pps.multiple_tiles
    {
        return Err(Error::Unsupported);
    }
    Ok(())
}

/// Crops to the conformance window and detaches the samples.
fn output(sps: &Sps, picture: Picture<'_>) -> Frame {
    let [left, right, top, bottom] = sps.crop.map(|v| v as usize);
    let width = sps.width as usize - left - right;
    let height = sps.height as usize - top - bottom;
    let mut planes: [Vec<u16>; 3] = Default::default();
    let mut plane_width = [0; 3];
    let components = if sps.chroma_array_type == 0 { 1 } else { 3 };
    for c in 0..components {
        let (sw, sh) = if c == 0 {
            (1, 1)
        } else {
            (sps.sub_width() as usize, sps.sub_height() as usize)
        };
        let stride = picture.plane_width[c];
        let (cw, ch) = (width / sw, height / sh);
        let (cl, ct) = (left / sw, top / sh);
        let mut out = Vec::with_capacity(cw * ch);
        for y in 0..ch {
            let start = (ct + y) * stride + cl;
            out.extend_from_slice(&picture.planes[c][start..start + cw]);
        }
        planes[c] = out;
        plane_width[c] = cw;
    }
    Frame {
        width: width as u32,
        height: height as u32,
        chroma_format: sps.chroma_array_type,
        bit_depth: sps.bit_depth_luma,
        planes,
        plane_width,
        colour: sps
            .vui
            .as_ref()
            .filter(|v| v.has_colour)
            .map(|v| StreamColour {
                primaries: v.primaries,
                transfer: v.transfer,
                matrix: v.matrix,
                full_range: v.full_range,
            }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::security::Generation;
    use std::path::PathBuf;

    /// Splits an Annex B stream into an `hvcC` configuration and length-prefixed
    /// item data, as a HEIF file holds them.
    fn annex_b_to_item(stream: &[u8]) -> (Vec<u8>, Vec<u8>) {
        let mut starts = Vec::new();
        let mut i = 0;
        while i + 3 <= stream.len() {
            if stream[i..i + 3] == [0, 0, 1] {
                starts.push(i + 3);
                i += 3;
            } else {
                i += 1;
            }
        }
        let mut units = Vec::new();
        for (n, &start) in starts.iter().enumerate() {
            let mut end = starts.get(n + 1).map_or(stream.len(), |next| next - 3);
            while end > start && stream[end - 1] == 0 {
                end -= 1;
            }
            units.push(&stream[start..end]);
        }
        let mut config = vec![
            1u8, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0xf0, 0, 0xfc, 0xfd, 0xf8, 0xf8, 0, 0, 0x03,
        ];
        let mut data = Vec::new();
        let mut arrays: Vec<(u8, Vec<&[u8]>)> = Vec::new();
        for unit in units {
            let kind = unit[0] >> 1 & 0x3f;
            match kind {
                32..=34 => match arrays.iter_mut().find(|a| a.0 == kind) {
                    Some(a) => a.1.push(unit),
                    None => arrays.push((kind, vec![unit])),
                },
                0..=9 | 16..=21 => {
                    data.extend_from_slice(&(unit.len() as u32).to_be_bytes());
                    data.extend_from_slice(unit);
                }
                _ => {}
            }
        }
        config.push(arrays.len() as u8);
        for (kind, list) in arrays {
            config.push(0x80 | kind);
            config.extend_from_slice(&(list.len() as u16).to_be_bytes());
            for unit in list {
                config.extend_from_slice(&(unit.len() as u16).to_be_bytes());
                config.extend_from_slice(unit);
            }
        }
        (config, data)
    }

    /// Damaged streams must end in an error, never in a panic or a hang.
    #[test]
    #[ignore = "needs the corpus from scripts/hevc-reference.py"]
    fn damaged_streams_never_panic() {
        let Some(folder) = std::env::var_os("KOVA_HEVC_REFERENCE").map(PathBuf::from) else {
            panic!("set KOVA_HEVC_REFERENCE to the output folder of scripts/hevc-reference.py");
        };
        let mut seed = 0x2545_f491_4f6c_dd1du64;
        let mut next = move || {
            seed ^= seed << 13;
            seed ^= seed >> 7;
            seed ^= seed << 17;
            seed
        };
        let mut panics = Vec::new();
        let mut runs = 0;
        let mut paths: Vec<_> = std::fs::read_dir(&folder)
            .unwrap()
            .filter_map(|e| e.ok().map(|e| e.path()))
            .filter(|p| p.extension().is_some_and(|e| e == "hevc"))
            .collect();
        paths.sort();
        for path in paths {
            let stream = std::fs::read(&path).unwrap();
            if stream.len() > 20_000 {
                continue;
            }
            let (config, data) = annex_b_to_item(&stream);
            for _ in 0..60 {
                let mut bad = data.clone();
                for _ in 0..1 + next() % 4 {
                    let at = (next() as usize) % bad.len();
                    bad[at] ^= 1 << (next() % 8);
                }
                if next() % 5 == 0 {
                    bad.truncate((next() as usize) % bad.len());
                }
                let ticket = Generation::default().next();
                runs += 1;
                let result = std::panic::catch_unwind(|| decode(&config, &bad, &ticket));
                if result.is_err() {
                    panics.push(path.file_name().unwrap().to_string_lossy().into_owned());
                }
            }
        }
        assert!(runs > 0);
        assert!(
            panics.is_empty(),
            "{} of {runs} damaged streams panicked: {panics:?}",
            panics.len()
        );
    }

    /// Compares every stream in the folder named by KOVA_HEVC_REFERENCE with its
    /// reference decoding (see scripts/hevc-reference.py).
    #[test]
    #[ignore = "needs the corpus from scripts/hevc-reference.py"]
    fn matches_the_reference_decoder_bit_for_bit() {
        let Some(folder) = std::env::var_os("KOVA_HEVC_REFERENCE").map(PathBuf::from) else {
            panic!("set KOVA_HEVC_REFERENCE to the output folder of scripts/hevc-reference.py");
        };
        let only = std::env::var("KOVA_HEVC_ONLY").ok();
        let mut names: Vec<_> = std::fs::read_dir(&folder)
            .unwrap()
            .filter_map(|e| e.ok())
            .filter_map(|e| {
                e.path()
                    .file_stem()
                    .map(|s| s.to_string_lossy().into_owned())
            })
            .collect();
        names.sort();
        names.dedup();
        let mut failures = Vec::new();
        let mut checked = 0;
        for name in names {
            if only.as_ref().is_some_and(|o| *o != name) {
                continue;
            }
            let (Ok(stream), Ok(reference), Ok(info)) = (
                std::fs::read(folder.join(format!("{name}.hevc"))),
                std::fs::read(folder.join(format!("{name}.yuv"))),
                std::fs::read_to_string(folder.join(format!("{name}.json"))),
            ) else {
                continue;
            };
            let field = |key: &str| -> String {
                let at = info.find(&format!("\"{key}\"")).unwrap();
                info[at..]
                    .split([':', ',', '}'])
                    .nth(1)
                    .unwrap()
                    .trim()
                    .trim_matches('"')
                    .to_string()
            };
            let (width, height): (usize, usize) = (
                field("width").parse().unwrap(),
                field("height").parse().unwrap(),
            );
            let (config, data) = annex_b_to_item(&stream);
            let ticket = Generation::default().next();
            checked += 1;
            let frame = match decode(&config, &data, &ticket) {
                Ok(frame) => frame,
                Err(error) => {
                    failures.push(format!("{name}: {error}"));
                    continue;
                }
            };
            if (frame.width as usize, frame.height as usize) != (width, height) {
                failures.push(format!(
                    "{name}: size {}x{} instead of {width}x{height}",
                    frame.width, frame.height
                ));
                continue;
            }
            let wide = frame.bit_depth > 8;
            let mut offset = 0;
            let mut report = String::new();
            for c in 0..if frame.chroma_format == 0 { 1 } else { 3 } {
                let pw = frame.plane_width[c];
                let ph = frame.planes[c].len() / pw;
                let (sw, sh) = if c == 0 {
                    (1usize, 1usize)
                } else {
                    match frame.chroma_format {
                        1 => (2, 2),
                        2 => (2, 1),
                        _ => (1, 1),
                    }
                };
                let ctb = 16usize;
                let mut count = 0;
                let mut earliest: Option<(usize, usize, Vec<String>)> = None;
                for y in 0..ph {
                    for x in 0..pw {
                        let at = offset + (y * pw + x) * if wide { 2 } else { 1 };
                        let expected = if wide {
                            u16::from_le_bytes([reference[at], reference[at + 1]])
                        } else {
                            u16::from(reference[at])
                        };
                        let got = frame.planes[c][y * pw + x];
                        if got != expected {
                            count += 1;
                            let key = (y * sh / ctb, x * sw / ctb);
                            match &mut earliest {
                                Some((ky, kx, list)) if (*ky, *kx) == key => {
                                    if list.len() < 10 {
                                        list.push(format!("({x},{y}) {got}/{expected}"));
                                    }
                                }
                                Some((ky, kx, _)) if (*ky, *kx) < key => {}
                                _ => {
                                    earliest = Some((
                                        key.0,
                                        key.1,
                                        vec![format!("({x},{y}) {got}/{expected}")],
                                    ))
                                }
                            }
                        }
                    }
                }
                if let Some((ky, kx, list)) = earliest {
                    report += &format!(
                        " plane {c}: {count} differ; first 16x16 area (row {ky}, col {kx}): {}",
                        list.join(" ")
                    );
                }
                offset += pw * ph * if wide { 2 } else { 1 };
            }
            if !report.is_empty() {
                failures.push(format!("{name}:{report}"));
            }
        }
        assert!(checked > 0, "no streams were found in {}", folder.display());
        assert!(
            failures.is_empty(),
            "{} of {checked} differ:\n{}",
            failures.len(),
            failures.join("\n")
        );
    }
}
