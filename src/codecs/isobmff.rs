//! The item-based ISO base media container behind AVIF and HEIC (HEIF). Only the
//! `meta` box is parsed, every length and count is checked against its parent
//! and against a limit, and item data is read on demand through seeks, so a
//! damaged or hostile container cannot make the parser run away or allocate
//! without bound.
use crate::{error::Error, security::Ticket};
use image::metadata::Orientation;
use std::io::{Read, Seek, SeekFrom};

/// Largest `meta` box that is read into memory.
const MAX_META: u64 = 16 * 1024 * 1024;
/// Largest single item (a picture, or one tile of a grid).
pub const MAX_ITEM_BYTES: u64 = 64 * 1024 * 1024;
const MAX_ITEMS: usize = 20_000;
const MAX_PROPERTIES: usize = 20_000;
const MAX_EXTENTS: usize = 1 << 16;

fn bad(message: &str) -> Error {
    Error::Corrupted(format!("HEIF container: {message}"))
}

/// The child boxes of a box: type and body.
pub type Boxes<'a> = Vec<([u8; 4], &'a [u8])>;

/// A bounds-checked reader over a box body.
pub struct Bytes<'a> {
    data: &'a [u8],
}
impl<'a> Bytes<'a> {
    pub fn new(data: &'a [u8]) -> Self {
        Self { data }
    }
    pub fn rest(&self) -> &'a [u8] {
        self.data
    }
    pub fn take(&mut self, count: usize) -> Result<&'a [u8], Error> {
        if count > self.data.len() {
            return Err(bad("a box is cut short"));
        }
        let (head, tail) = self.data.split_at(count);
        self.data = tail;
        Ok(head)
    }
    pub fn u8(&mut self) -> Result<u8, Error> {
        Ok(self.take(1)?[0])
    }
    pub fn u16(&mut self) -> Result<u16, Error> {
        Ok(u16::from_be_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32, Error> {
        Ok(u32::from_be_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn i32(&mut self) -> Result<i32, Error> {
        Ok(self.u32()? as i32)
    }
    pub fn u64(&mut self) -> Result<u64, Error> {
        Ok(u64::from_be_bytes(self.take(8)?.try_into().unwrap()))
    }
    /// An unsigned integer of 0 to 8 bytes.
    pub fn uint(&mut self, size: usize) -> Result<u64, Error> {
        if size > 8 {
            return Err(bad("an integer is wider than 64 bits"));
        }
        Ok(self
            .take(size)?
            .iter()
            .fold(0, |n, b| n << 8 | u64::from(*b)))
    }
    pub fn fourcc(&mut self) -> Result<[u8; 4], Error> {
        Ok(self.take(4)?.try_into().unwrap())
    }
    /// A zero-terminated string; a missing terminator ends it at the box end.
    pub fn cstr(&mut self) -> String {
        let end = self
            .data
            .iter()
            .position(|&b| b == 0)
            .unwrap_or(self.data.len());
        let text = String::from_utf8_lossy(&self.data[..end]).into_owned();
        self.data = &self.data[(end + 1).min(self.data.len())..];
        text
    }
    /// Version and flags of a full box.
    fn full(&mut self) -> Result<(u8, u32), Error> {
        let word = self.u32()?;
        Ok(((word >> 24) as u8, word & 0xff_ffff))
    }
    /// The child boxes of this body, in order.
    pub fn boxes(&mut self) -> Result<Boxes<'a>, Error> {
        let mut result = Vec::new();
        while !self.data.is_empty() {
            if result.len() >= MAX_PROPERTIES {
                return Err(bad("too many boxes"));
            }
            let size = self.u32()?;
            let kind = self.fourcc()?;
            let body = match size {
                0 => self.data.len(),
                1 => {
                    let wide = self.u64()?;
                    usize::try_from(
                        wide.checked_sub(16)
                            .ok_or_else(|| bad("a box is too small"))?,
                    )
                    .map_err(|_| bad("a box is too large"))?
                }
                n if n >= 8 => n as usize - 8,
                _ => return Err(bad("a box is too small")),
            };
            result.push((kind, self.take(body)?));
        }
        Ok(result)
    }
}

#[derive(Clone, Debug, PartialEq)]
pub enum Colr {
    /// Colour described by code points.
    Nclx {
        primaries: u16,
        transfer: u16,
        matrix: u16,
        full_range: bool,
    },
    Icc(Vec<u8>),
}
/// Clean aperture: the visible window, as fractions of pixels.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Clap {
    pub width: (u32, u32),
    pub height: (u32, u32),
    pub x: (i32, u32),
    pub y: (i32, u32),
}
#[derive(Clone, Debug)]
pub enum Property {
    Ispe(u32, u32),
    Colr(Colr),
    /// Counter-clockwise quarter turns.
    Irot(u8),
    /// 0 mirrors left and right, 1 mirrors top and bottom.
    Imir(u8),
    Clap(Clap),
    AuxC(String),
    /// Decoder configuration (`av1C`, `hvcC`): the box type and its payload.
    Config([u8; 4], Vec<u8>),
    Other,
}

#[derive(Clone, Debug)]
pub struct Item {
    pub id: u32,
    pub kind: [u8; 4],
    pub properties: Vec<Property>,
    /// (offset, length); an offset inside `idat` when `in_idat`.
    extents: Vec<(u64, u64)>,
    in_idat: bool,
}
impl Item {
    pub fn ispe(&self) -> Option<(u32, u32)> {
        self.properties.iter().find_map(|p| match p {
            Property::Ispe(w, h) => Some((*w, *h)),
            _ => None,
        })
    }
    pub fn colr(&self) -> Option<&Colr> {
        // An ICC profile is the better description when both are present.
        let mut found = None;
        for p in &self.properties {
            if let Property::Colr(c) = p
                && (matches!(c, Colr::Icc(_)) || found.is_none())
            {
                found = Some(c);
            }
        }
        found
    }
    pub fn config(&self, kind: &[u8; 4]) -> Option<&[u8]> {
        self.properties.iter().find_map(|p| match p {
            Property::Config(k, data) if k == kind => Some(data.as_slice()),
            _ => None,
        })
    }
    pub fn aux_type(&self) -> Option<&str> {
        self.properties.iter().find_map(|p| match p {
            Property::AuxC(urn) => Some(urn.as_str()),
            _ => None,
        })
    }
    pub fn clap(&self) -> Option<Clap> {
        self.properties.iter().find_map(|p| match p {
            Property::Clap(c) => Some(*c),
            _ => None,
        })
    }
    /// The rotation and mirroring as one EXIF-style orientation. They apply in
    /// the order the properties are listed (rotation first in practice).
    pub fn orientation(&self) -> Orientation {
        let mut clockwise = 0u8;
        let mut flip = false;
        for property in &self.properties {
            match property {
                Property::Irot(quarter_turns) => {
                    // Counter-clockwise in the file; mirroring afterwards flips
                    // the sense of the rotation that has been recorded so far.
                    let turns = (4 - quarter_turns % 4) % 4;
                    clockwise = if flip {
                        (clockwise + 4 - turns) % 4
                    } else {
                        (clockwise + turns) % 4
                    };
                }
                Property::Imir(axis) => {
                    // A top-bottom mirror is a half turn plus a left-right one.
                    if axis & 1 == 1 {
                        clockwise = (clockwise + 2) % 4;
                    }
                    flip = !flip;
                }
                _ => {}
            }
        }
        match (clockwise, flip) {
            (0, false) => Orientation::NoTransforms,
            (1, false) => Orientation::Rotate90,
            (2, false) => Orientation::Rotate180,
            (3, false) => Orientation::Rotate270,
            (0, true) => Orientation::FlipHorizontal,
            (1, true) => Orientation::Rotate90FlipH,
            (2, true) => Orientation::FlipVertical,
            _ => Orientation::Rotate270FlipH,
        }
    }
}

#[derive(Clone, Debug)]
struct Reference {
    kind: [u8; 4],
    from: u32,
    to: Vec<u32>,
}

#[derive(Debug)]
pub struct Container {
    pub primary: u32,
    items: Vec<Item>,
    references: Vec<Reference>,
    idat: Vec<u8>,
    length: u64,
}

impl Container {
    pub fn parse<R: Read + Seek>(
        reader: &mut R,
        length: u64,
        ticket: &Ticket,
    ) -> Result<Self, Error> {
        let mut at = 0u64;
        while length.saturating_sub(at) >= 8 {
            ticket.check()?;
            reader.seek(SeekFrom::Start(at))?;
            let mut header = [0u8; 16];
            reader.read_exact(&mut header[..8])?;
            let size = u32::from_be_bytes(header[..4].try_into().unwrap());
            let kind: [u8; 4] = header[4..8].try_into().unwrap();
            let (size, head) = match size {
                0 => (length - at, 8),
                1 => {
                    reader.read_exact(&mut header[8..16])?;
                    (u64::from_be_bytes(header[8..16].try_into().unwrap()), 16)
                }
                n => (u64::from(n), 8),
            };
            if size < head || size > length - at {
                return Err(bad("a box is larger than the file"));
            }
            if &kind == b"meta" {
                if size - head > MAX_META {
                    return Err(bad("the metadata box is too large"));
                }
                let mut body = vec![0u8; (size - head) as usize];
                reader.read_exact(&mut body)?;
                return Self::parse_meta(&body, length);
            }
            at += size;
        }
        Err(bad("there is no metadata box"))
    }

    fn parse_meta(body: &[u8], length: u64) -> Result<Self, Error> {
        let mut meta = Bytes::new(body);
        meta.full()?;
        let mut primary = None;
        let mut items: Vec<Item> = Vec::new();
        let mut references = Vec::new();
        let mut idat = Vec::new();
        let mut locations = None;
        let mut properties: Vec<Property> = Vec::new();
        let mut associations: Vec<(u32, Vec<u16>)> = Vec::new();
        for (kind, content) in meta.boxes()? {
            let mut b = Bytes::new(content);
            match &kind {
                b"pitm" => {
                    let (version, _) = b.full()?;
                    primary = Some(if version == 0 {
                        u32::from(b.u16()?)
                    } else {
                        b.u32()?
                    });
                }
                b"iinf" => {
                    let (version, _) = b.full()?;
                    let count = if version == 0 {
                        u32::from(b.u16()?)
                    } else {
                        b.u32()?
                    };
                    if count as usize > MAX_ITEMS {
                        return Err(bad("too many items"));
                    }
                    for (kind, entry) in b.boxes()? {
                        if &kind != b"infe" {
                            continue;
                        }
                        let mut e = Bytes::new(entry);
                        let (version, _) = e.full()?;
                        if version < 2 {
                            continue;
                        }
                        let id = if version == 2 {
                            u32::from(e.u16()?)
                        } else {
                            e.u32()?
                        };
                        e.u16()?; // protection index
                        items.push(Item {
                            id,
                            kind: e.fourcc()?,
                            properties: Vec::new(),
                            extents: Vec::new(),
                            in_idat: false,
                        });
                    }
                }
                b"iloc" => locations = Some(Self::locations(&mut b)?),
                b"iprp" => {
                    for (kind, content) in b.boxes()? {
                        match &kind {
                            b"ipco" => {
                                for (kind, content) in Bytes::new(content).boxes()? {
                                    properties.push(Self::property(&kind, content)?);
                                }
                            }
                            b"ipma" => Self::associate(content, &mut associations)?,
                            _ => {}
                        }
                    }
                }
                b"iref" => {
                    let (version, _) = b.full()?;
                    for (kind, content) in b.boxes()? {
                        let mut r = Bytes::new(content);
                        let wide = version != 0;
                        let id = |r: &mut Bytes| -> Result<u32, Error> {
                            Ok(if wide { r.u32()? } else { u32::from(r.u16()?) })
                        };
                        let from = id(&mut r)?;
                        let count = r.u16()?;
                        let mut to = Vec::new();
                        for _ in 0..count.min(4096) {
                            to.push(id(&mut r)?);
                        }
                        references.push(Reference { kind, from, to });
                    }
                }
                b"idat" => idat = content.to_vec(),
                _ => {}
            }
        }
        if items.len() > MAX_ITEMS {
            return Err(bad("too many items"));
        }
        for (id, extents, in_idat) in locations.unwrap_or_default() {
            if let Some(item) = items.iter_mut().find(|i| i.id == id) {
                item.extents = extents;
                item.in_idat = in_idat;
            }
        }
        for (id, indices) in associations {
            if let Some(item) = items.iter_mut().find(|i| i.id == id) {
                for index in indices {
                    // Index 0 means "no property"; indices count from 1.
                    if let Some(p) = (index as usize)
                        .checked_sub(1)
                        .and_then(|i| properties.get(i))
                    {
                        item.properties.push(p.clone());
                    }
                }
            }
        }
        Ok(Self {
            primary: primary.ok_or_else(|| bad("there is no primary item"))?,
            items,
            references,
            idat,
            length,
        })
    }

    #[allow(clippy::type_complexity)]
    fn locations(b: &mut Bytes) -> Result<Vec<(u32, Vec<(u64, u64)>, bool)>, Error> {
        let (version, _) = b.full()?;
        let sizes = b.u16()?;
        let (offset_size, length_size) = (usize::from(sizes >> 12), usize::from(sizes >> 8 & 15));
        let (base_size, index_size) = (usize::from(sizes >> 4 & 15), usize::from(sizes & 15));
        let count = if version < 2 {
            u32::from(b.u16()?)
        } else {
            b.u32()?
        };
        if count as usize > MAX_ITEMS {
            return Err(bad("too many item locations"));
        }
        let mut result = Vec::new();
        for _ in 0..count {
            let id = if version < 2 {
                u32::from(b.u16()?)
            } else {
                b.u32()?
            };
            let method = if version == 0 { 0 } else { b.u16()? & 15 };
            if method > 1 {
                // Data inside another item is not used by still pictures.
                return Err(Error::Unsupported);
            }
            b.u16()?; // data reference index
            let base = b.uint(base_size)?;
            let extents = b.u16()?;
            if usize::from(extents) > MAX_EXTENTS {
                return Err(bad("too many extents"));
            }
            let mut list = Vec::new();
            for _ in 0..extents {
                if version != 0 && index_size > 0 {
                    b.uint(index_size)?;
                }
                let offset = b.uint(offset_size)?;
                let len = b.uint(length_size)?;
                list.push((
                    base.checked_add(offset)
                        .ok_or_else(|| bad("an extent overflows"))?,
                    len,
                ));
            }
            result.push((id, list, method == 1));
        }
        Ok(result)
    }

    fn associate(content: &[u8], out: &mut Vec<(u32, Vec<u16>)>) -> Result<(), Error> {
        let mut b = Bytes::new(content);
        let (version, flags) = b.full()?;
        let count = b.u32()?;
        if count as usize > MAX_ITEMS {
            return Err(bad("too many property associations"));
        }
        for _ in 0..count {
            let id = if version < 1 {
                u32::from(b.u16()?)
            } else {
                b.u32()?
            };
            let n = b.u8()?;
            let mut indices = Vec::new();
            for _ in 0..n {
                indices.push(if flags & 1 == 1 {
                    b.u16()? & 0x7fff
                } else {
                    u16::from(b.u8()? & 0x7f)
                });
            }
            out.push((id, indices));
        }
        Ok(())
    }

    fn property(kind: &[u8; 4], content: &[u8]) -> Result<Property, Error> {
        let mut b = Bytes::new(content);
        Ok(match kind {
            b"ispe" => {
                b.full()?;
                Property::Ispe(b.u32()?, b.u32()?)
            }
            b"colr" => match &b.fourcc()? {
                b"nclx" => {
                    let (primaries, transfer, matrix) = (b.u16()?, b.u16()?, b.u16()?);
                    Property::Colr(Colr::Nclx {
                        primaries,
                        transfer,
                        matrix,
                        full_range: b.u8()? & 0x80 != 0,
                    })
                }
                b"prof" | b"rICC" => Property::Colr(Colr::Icc(b.rest().to_vec())),
                _ => Property::Other,
            },
            b"irot" => Property::Irot(b.u8()? & 3),
            b"imir" => Property::Imir(b.u8()? & 1),
            b"clap" => {
                let mut n = || -> Result<(u32, u32), Error> { Ok((b.u32()?, b.u32()?)) };
                let (width, height) = (n()?, n()?);
                let mut s = || -> Result<(i32, u32), Error> { Ok((b.i32()?, b.u32()?)) };
                Property::Clap(Clap {
                    width,
                    height,
                    x: s()?,
                    y: s()?,
                })
            }
            b"auxC" => {
                b.full()?;
                Property::AuxC(b.cstr())
            }
            b"av1C" | b"hvcC" => Property::Config(*kind, content.to_vec()),
            _ => Property::Other,
        })
    }

    pub fn item(&self, id: u32) -> Option<&Item> {
        self.items.iter().find(|i| i.id == id)
    }
    /// Targets of references of `kind` that start at `from`.
    pub fn references_from<'a>(
        &'a self,
        kind: &'a [u8; 4],
        from: u32,
    ) -> impl Iterator<Item = u32> + 'a {
        self.references
            .iter()
            .filter(move |r| &r.kind == kind && r.from == from)
            .flat_map(|r| r.to.iter().copied())
    }
    /// Items with a reference of `kind` that ends at `to`.
    pub fn references_to<'a>(
        &'a self,
        kind: &'a [u8; 4],
        to: u32,
    ) -> impl Iterator<Item = u32> + 'a {
        self.references
            .iter()
            .filter(move |r| &r.kind == kind && r.to.contains(&to))
            .map(|r| r.from)
    }

    /// The bytes of an item. Extents are read by seeking, within the file.
    pub fn read_item<R: Read + Seek>(
        &self,
        reader: &mut R,
        id: u32,
        ticket: &Ticket,
    ) -> Result<Vec<u8>, Error> {
        let item = self.item(id).ok_or_else(|| bad("an item is missing"))?;
        let limit = if item.in_idat {
            self.idat.len() as u64
        } else {
            self.length
        };
        let mut total = 0u64;
        let mut extents = Vec::new();
        for &(offset, len) in &item.extents {
            // A zero length runs to the end of the data.
            let len = if len == 0 {
                limit.saturating_sub(offset)
            } else {
                len
            };
            if offset.checked_add(len).is_none_or(|end| end > limit) {
                return Err(bad("an extent lies outside the file"));
            }
            total += len;
            if total > MAX_ITEM_BYTES {
                return Err(Error::TooLarge);
            }
            extents.push((offset, len));
        }
        let mut data = Vec::with_capacity(total as usize);
        for (offset, len) in extents {
            ticket.check()?;
            if item.in_idat {
                data.extend_from_slice(&self.idat[offset as usize..(offset + len) as usize]);
            } else {
                reader.seek(SeekFrom::Start(offset))?;
                let before = data.len();
                reader.take(len).read_to_end(&mut data)?;
                if (data.len() - before) as u64 != len {
                    return Err(bad("the file ends inside an item"));
                }
            }
        }
        Ok(data)
    }
}
