//! Sequence and picture parameter sets. Only what decoding a still picture
//! needs is kept; the rest is parsed to find what follows it.
use super::bits::{Bits, bad};
use crate::error::Error;

/// An explicit scaling list in diagonal scan order and its DC value.
type ParsedList = (Vec<u8>, u8);

/// A scaling matrix for one transform size and colour/prediction kind:
/// `factor[y * size + x]`.
#[derive(Clone, Debug)]
pub struct ScalingLists {
    /// `[size_id][matrix_id]`, with size 4, 8, 16 and 32.
    pub factors: [[Vec<u8>; 6]; 4],
}

const DEFAULT_8X8_INTRA: [u8; 64] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 16, 17, 16, 17, 18, 17, 18, 18, 17, 18, 21, 19, 20,
    21, 20, 19, 21, 24, 22, 22, 24, 24, 22, 22, 24, 25, 25, 27, 30, 27, 25, 25, 29, 31, 35, 35, 31,
    29, 36, 41, 44, 41, 36, 47, 54, 54, 47, 65, 70, 65, 88, 88, 115,
];
const DEFAULT_8X8_INTER: [u8; 64] = [
    16, 16, 16, 16, 16, 16, 16, 16, 16, 16, 17, 17, 17, 17, 17, 18, 18, 18, 18, 18, 18, 20, 20, 20,
    20, 20, 20, 20, 24, 24, 24, 24, 24, 24, 24, 24, 25, 25, 25, 25, 25, 25, 25, 28, 28, 28, 28, 28,
    28, 33, 33, 33, 33, 33, 41, 41, 41, 41, 54, 54, 54, 71, 71, 91,
];

/// Up-right diagonal scan of a square block of `size` (a power of two): the
/// (x, y) position of each index.
pub fn diagonal_scan(size: usize) -> Vec<(u8, u8)> {
    let mut order = Vec::with_capacity(size * size);
    let (mut x, mut y) = (0i32, 0i32);
    let n = size as i32;
    while order.len() < size * size {
        while y >= 0 {
            if x < n && y < n {
                order.push((x as u8, y as u8));
            }
            y -= 1;
            x += 1;
        }
        y = x;
        x = 0;
    }
    order
}

impl ScalingLists {
    /// The default matrices (flat for 4x4).
    pub fn default_lists() -> Self {
        let mut lists = Self::empty();
        for matrix in 0..6 {
            let coefficients: &[u8] = if matrix < 3 {
                &DEFAULT_8X8_INTRA
            } else {
                &DEFAULT_8X8_INTER
            };
            lists.set(0, matrix, &[16; 16], 16);
            for size_id in 1..4 {
                lists.set(size_id, matrix, coefficients, 16);
            }
        }
        lists
    }
    fn empty() -> Self {
        Self {
            factors: std::array::from_fn(|size_id| {
                std::array::from_fn(|_| vec![16; (4usize << size_id).pow(2)])
            }),
        }
    }
    /// Stores a list given in diagonal scan order, upsampled to the block size.
    fn set(&mut self, size_id: usize, matrix: usize, list: &[u8], dc: u8) {
        let size = 4usize << size_id;
        let base = if size_id == 0 { 4 } else { 8 };
        let scan = diagonal_scan(base);
        let ratio = size / base;
        let out = &mut self.factors[size_id][matrix];
        for (i, &(x, y)) in scan.iter().enumerate() {
            for dy in 0..ratio {
                for dx in 0..ratio {
                    out[(y as usize * ratio + dy) * size + x as usize * ratio + dx] = list[i];
                }
            }
        }
        if size_id >= 2 {
            out[0] = dc;
        }
    }

    fn parse(bits: &mut Bits, chroma_444: bool) -> Result<Self, Error> {
        let mut lists = Self::empty();
        let mut parsed: [[Option<ParsedList>; 6]; 4] = Default::default();
        for (size_id, sizes) in parsed.iter_mut().enumerate() {
            let step = if size_id == 3 { 3 } else { 1 };
            let mut matrix = 0;
            while matrix < 6 {
                let count = if size_id == 0 { 16 } else { 64 };
                let (list, dc) = if !bits.flag()? {
                    let delta = bits.ue()? as usize * step;
                    if delta > matrix {
                        return Err(bad("a scaling list refers to a later one"));
                    }
                    if delta == 0 {
                        let default: &[u8] = match (size_id, matrix) {
                            (0, _) => &[16; 16],
                            (_, 0..=2) => &DEFAULT_8X8_INTRA,
                            _ => &DEFAULT_8X8_INTER,
                        };
                        (default.to_vec(), 16)
                    } else {
                        sizes[matrix - delta]
                            .clone()
                            .ok_or_else(|| bad("a scaling list refers to a missing one"))?
                    }
                } else {
                    let mut next = 8i32;
                    let mut dc = 16;
                    if size_id > 1 {
                        let value = bits.se()?;
                        if !(-7..=247).contains(&value) {
                            return Err(bad("a scaling list DC value is out of range"));
                        }
                        next = value + 8;
                        dc = next as u8;
                    }
                    let mut list = Vec::with_capacity(count);
                    for _ in 0..count {
                        let delta = bits.se()?;
                        if !(-128..=127).contains(&delta) {
                            return Err(bad("a scaling list delta is out of range"));
                        }
                        next = (next + delta + 256) % 256;
                        list.push(next as u8);
                    }
                    (list, dc)
                };
                lists.set(size_id, matrix, &list, dc);
                sizes[matrix] = Some((list, dc));
                matrix += step;
            }
        }
        // 32x32 chroma (4:4:4) takes the 16x16 lists' coefficients.
        if chroma_444 {
            for matrix in [1, 2, 4, 5] {
                if let Some((list, dc)) = parsed[2][matrix].clone() {
                    lists.set(3, matrix, &list, dc);
                }
            }
        }
        Ok(lists)
    }
}

#[derive(Clone, Debug, Default)]
pub struct Vui {
    pub full_range: bool,
    pub primaries: u8,
    pub transfer: u8,
    pub matrix: u8,
    pub has_colour: bool,
}

#[derive(Clone, Debug, Default)]
pub struct RangeExtension {
    pub transform_skip_rotation: bool,
    pub transform_skip_context: bool,
    pub implicit_rdpcm: bool,
    pub extended_precision: bool,
    pub intra_smoothing_disabled: bool,
    pub persistent_rice_adaptation: bool,
    pub cabac_bypass_alignment: bool,
}

#[derive(Clone, Debug)]
pub struct Pcm {
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
    pub log2_min_size: u8,
    pub log2_max_size: u8,
    pub loop_filter_disabled: bool,
}

#[derive(Clone, Debug)]
pub struct Sps {
    pub id: u32,
    pub separate_colour_planes: bool,
    /// 0 for monochrome (or separate planes), else the chroma format.
    pub chroma_array_type: u8,
    pub width: u32,
    pub height: u32,
    /// Left, right, top and bottom crop in luma samples.
    pub crop: [u32; 4],
    pub bit_depth_luma: u8,
    pub bit_depth_chroma: u8,
    pub num_short_term_ref_pic_sets: u32,
    /// Pictures in each short-term set, to follow a predicted one in a slice header.
    pub rps_sizes: Vec<u32>,
    pub log2_poc_lsb: u32,
    pub long_term_ref_pics_present: bool,
    pub num_long_term_ref_pics_sps: u32,
    pub temporal_mvp: bool,
    pub log2_min_cb: u8,
    pub log2_ctb: u8,
    pub log2_min_tb: u8,
    pub log2_max_tb: u8,
    pub max_transform_hierarchy_depth_intra: u8,
    pub scaling_lists: Option<ScalingLists>,
    pub sao: bool,
    pub pcm: Option<Pcm>,
    pub strong_intra_smoothing: bool,
    pub vui: Option<Vui>,
    pub range: RangeExtension,
}
impl Sps {
    pub fn sub_width(&self) -> u32 {
        if matches!(self.chroma_array_type, 1 | 2) {
            2
        } else {
            1
        }
    }
    pub fn sub_height(&self) -> u32 {
        if self.chroma_array_type == 1 { 2 } else { 1 }
    }
    pub fn ctb_size(&self) -> u32 {
        1 << self.log2_ctb
    }
    pub fn width_in_ctbs(&self) -> u32 {
        self.width.div_ceil(self.ctb_size())
    }
    pub fn height_in_ctbs(&self) -> u32 {
        self.height.div_ceil(self.ctb_size())
    }
    pub fn qp_bd_offset_luma(&self) -> i32 {
        6 * (i32::from(self.bit_depth_luma) - 8)
    }
    pub fn qp_bd_offset_chroma(&self) -> i32 {
        6 * (i32::from(self.bit_depth_chroma) - 8)
    }
}

/// Skips `profile_tier_level(1, max_sub_layers_minus1)`.
fn profile_tier_level(bits: &mut Bits, max_sub_layers_minus1: u32) -> Result<(), Error> {
    bits.skip(88)?; // general profile, 32 compatibility flags, constraint flags
    bits.skip(8)?; // general_level_idc
    let mut profile = [false; 8];
    let mut level = [false; 8];
    for i in 0..max_sub_layers_minus1 as usize {
        profile[i] = bits.flag()?;
        level[i] = bits.flag()?;
    }
    if max_sub_layers_minus1 > 0 {
        for _ in max_sub_layers_minus1..8 {
            bits.skip(2)?;
        }
    }
    for i in 0..max_sub_layers_minus1 as usize {
        if profile[i] {
            bits.skip(88)?;
        }
        if level[i] {
            bits.skip(8)?;
        }
    }
    Ok(())
}

/// Parses a short-term reference picture set, only to move past it. `sizes`
/// holds the number of pictures in each earlier set, which a predicted set
/// needs to know how many flags follow.
pub fn short_term_ref_pic_set(
    bits: &mut Bits,
    index: u32,
    count: u32,
    sizes: &mut Vec<u32>,
) -> Result<(), Error> {
    let predicted = index != 0 && bits.flag()?;
    if predicted {
        let delta_idx = if index == count { bits.ue()? + 1 } else { 1 };
        if delta_idx > index {
            return Err(bad("a reference picture set refers to a later one"));
        }
        let reference = (index - delta_idx) as usize;
        bits.skip(1)?; // delta_rps_sign
        bits.ue()?; // abs_delta_rps_minus1
        let pictures = *sizes
            .get(reference)
            .ok_or_else(|| bad("a reference picture set refers to a missing one"))?;
        let mut kept = 0;
        for _ in 0..=pictures {
            let used = bits.flag()?;
            if used || bits.flag()? {
                kept += 1;
            }
        }
        sizes.push(kept);
    } else {
        let negatives = bits.ue()?;
        let positives = bits.ue()?;
        if negatives > 16 || positives > 16 {
            return Err(bad("a reference picture set is too large"));
        }
        for _ in 0..negatives + positives {
            bits.ue()?; // delta_poc_minus1
            bits.skip(1)?; // used_by_curr_pic_flag
        }
        sizes.push(negatives + positives);
    }
    Ok(())
}

fn hrd_parameters(bits: &mut Bits, common: bool, max_sub_layers_minus1: u32) -> Result<(), Error> {
    let (mut nal, mut vcl, mut sub_pic) = (false, false, false);
    if common {
        nal = bits.flag()?;
        vcl = bits.flag()?;
        if nal || vcl {
            sub_pic = bits.flag()?;
            if sub_pic {
                bits.skip(8 + 5 + 1 + 5)?;
            }
            bits.skip(4 + 4)?;
            if sub_pic {
                bits.skip(4)?;
            }
            bits.skip(5 + 5 + 5)?;
        }
    }
    for _ in 0..=max_sub_layers_minus1 {
        let fixed_general = bits.flag()?;
        let fixed_cvs = if fixed_general { true } else { bits.flag()? };
        let mut low_delay = false;
        if fixed_cvs {
            bits.ue()?;
        } else {
            low_delay = bits.flag()?;
        }
        let mut cpb_count = 1;
        if !low_delay {
            cpb_count = bits.ue()? + 1;
            if cpb_count > 32 {
                return Err(bad("too many CPB specifications"));
            }
        }
        for present in [nal, vcl] {
            if present {
                for _ in 0..cpb_count {
                    bits.ue()?;
                    bits.ue()?;
                    if sub_pic {
                        bits.ue()?;
                        bits.ue()?;
                    }
                    bits.skip(1)?;
                }
            }
        }
    }
    Ok(())
}

fn vui(bits: &mut Bits, max_sub_layers_minus1: u32) -> Result<Vui, Error> {
    let mut vui = Vui::default();
    if bits.flag()? && bits.u(8)? == 255 {
        bits.skip(32)?;
    }
    if bits.flag()? {
        bits.skip(1)?;
    }
    if bits.flag()? {
        bits.skip(3)?;
        vui.full_range = bits.flag()?;
        if bits.flag()? {
            vui.has_colour = true;
            vui.primaries = bits.u(8)? as u8;
            vui.transfer = bits.u(8)? as u8;
            vui.matrix = bits.u(8)? as u8;
        }
    }
    if bits.flag()? {
        bits.ue()?;
        bits.ue()?;
    }
    bits.skip(3)?; // neutral chroma, field sequence, frame/field info
    if bits.flag()? {
        for _ in 0..4 {
            bits.ue()?;
        }
    }
    if bits.flag()? {
        bits.skip(64)?;
        if bits.flag()? {
            bits.ue()?;
        }
        if bits.flag()? {
            hrd_parameters(bits, true, max_sub_layers_minus1)?;
        }
    }
    if bits.flag()? {
        bits.skip(3)?;
        for _ in 0..5 {
            bits.ue()?;
        }
    }
    Ok(vui)
}

pub fn parse_sps(rbsp: &[u8]) -> Result<Sps, Error> {
    let mut bits = Bits::new(rbsp);
    bits.skip(4)?; // vps id
    let max_sub_layers_minus1 = bits.u(3)?;
    if max_sub_layers_minus1 > 6 {
        return Err(bad("too many sub-layers"));
    }
    bits.skip(1)?;
    profile_tier_level(&mut bits, max_sub_layers_minus1)?;
    let id = bits.ue()?;
    let chroma_format_idc = bits.ue()?;
    if chroma_format_idc > 3 {
        return Err(bad("unknown chroma format"));
    }
    let separate_colour_planes = chroma_format_idc == 3 && bits.flag()?;
    let width = bits.ue()?;
    let height = bits.ue()?;
    if width == 0 || height == 0 || width > 65535 || height > 65535 {
        return Err(Error::Dimensions);
    }
    let chroma_array_type = if separate_colour_planes {
        0
    } else {
        chroma_format_idc as u8
    };
    let (unit_x, unit_y) = match chroma_array_type {
        1 => (2, 2),
        2 => (2, 1),
        _ => (1, 1),
    };
    let mut crop = [0; 4];
    if bits.flag()? {
        for (i, value) in crop.iter_mut().enumerate() {
            *value = bits.ue()? * if i < 2 { unit_x } else { unit_y };
        }
        if crop[0] + crop[1] >= width || crop[2] + crop[3] >= height {
            return Err(bad("the crop window covers the whole picture"));
        }
    }
    let bit_depth_luma = bits.ue()? + 8;
    let bit_depth_chroma = bits.ue()? + 8;
    if bit_depth_luma > 16 || bit_depth_chroma > 16 {
        return Err(Error::Unsupported);
    }
    let log2_poc_lsb = bits.ue()? + 4;
    if log2_poc_lsb > 16 {
        return Err(bad("the picture order count width is out of range"));
    }
    let ordering_info = bits.flag()?;
    let first = if ordering_info {
        0
    } else {
        max_sub_layers_minus1
    };
    for _ in first..=max_sub_layers_minus1 {
        bits.ue()?;
        bits.ue()?;
        bits.ue()?;
    }
    let log2_min_cb = bits.ue()? + 3;
    let log2_diff_cb = bits.ue()?;
    let log2_min_tb = bits.ue()? + 2;
    let log2_diff_tb = bits.ue()?;
    if log2_min_cb > 6 || log2_min_cb + log2_diff_cb > 6 || log2_min_tb + log2_diff_tb > 5 {
        return Err(bad("coding block sizes are out of range"));
    }
    let log2_ctb = log2_min_cb + log2_diff_cb;
    let log2_max_tb = log2_min_tb + log2_diff_tb;
    if log2_ctb < 4 || log2_min_tb >= log2_min_cb || log2_max_tb > log2_ctb.min(5) {
        return Err(bad("block sizes are inconsistent"));
    }
    bits.ue()?; // max_transform_hierarchy_depth_inter
    let max_transform_hierarchy_depth_intra = bits.ue()?;
    if max_transform_hierarchy_depth_intra > log2_ctb - log2_min_tb {
        return Err(bad("the transform hierarchy is too deep"));
    }
    let mut scaling_lists = None;
    if bits.flag()? {
        scaling_lists = Some(if bits.flag()? {
            ScalingLists::parse(&mut bits, chroma_array_type == 3)?
        } else {
            ScalingLists::default_lists()
        });
    }
    bits.skip(1)?; // amp
    let sao = bits.flag()?;
    let mut pcm = None;
    if bits.flag()? {
        let bit_depth_luma_pcm = bits.u(4)? as u8 + 1;
        let bit_depth_chroma_pcm = bits.u(4)? as u8 + 1;
        let log2_min_size = bits.ue()? as u8 + 3;
        let log2_max_size = log2_min_size + bits.ue()? as u8;
        let loop_filter_disabled = bits.flag()?;
        if log2_max_size > 5 || u32::from(bit_depth_luma_pcm) > bit_depth_luma {
            return Err(bad("PCM parameters are out of range"));
        }
        pcm = Some(Pcm {
            bit_depth_luma: bit_depth_luma_pcm,
            bit_depth_chroma: bit_depth_chroma_pcm,
            log2_min_size,
            log2_max_size,
            loop_filter_disabled,
        });
    }
    let num_short_term_ref_pic_sets = bits.ue()?;
    if num_short_term_ref_pic_sets > 64 {
        return Err(bad("too many reference picture sets"));
    }
    let mut rps_sizes = Vec::new();
    for index in 0..num_short_term_ref_pic_sets {
        short_term_ref_pic_set(
            &mut bits,
            index,
            num_short_term_ref_pic_sets,
            &mut rps_sizes,
        )?;
    }
    let long_term_ref_pics_present = bits.flag()?;
    let mut num_long_term_ref_pics_sps = 0;
    if long_term_ref_pics_present {
        num_long_term_ref_pics_sps = bits.ue()?;
        if num_long_term_ref_pics_sps > 32 {
            return Err(bad("too many long-term reference pictures"));
        }
        // lt_ref_pic_poc_lsb_sps and used_by_curr_pic_lt_sps_flag for each.
        bits.skip(num_long_term_ref_pics_sps as usize * (log2_poc_lsb as usize + 1))?;
    }
    let temporal_mvp = bits.flag()?;
    let strong_intra_smoothing = bits.flag()?;
    let mut vui_result = None;
    let mut extensions_readable = true;
    if bits.flag()? {
        match vui(&mut bits, max_sub_layers_minus1) {
            Ok(v) => vui_result = Some(v),
            // A damaged VUI costs the colour hint and any extension after it.
            Err(_) => extensions_readable = false,
        }
    }
    let mut range = RangeExtension::default();
    if extensions_readable && bits.flag().unwrap_or(false) {
        let range_flag = bits.flag().unwrap_or(false);
        bits.skip(7).ok(); // multilayer, 3D, SCC, 4 more bits
        if range_flag {
            range = RangeExtension {
                transform_skip_rotation: bits.flag()?,
                transform_skip_context: bits.flag()?,
                implicit_rdpcm: bits.flag()?,
                extended_precision: {
                    bits.skip(1)?; // explicit_rdpcm_enabled_flag: inter pictures only
                    bits.flag()?
                },
                intra_smoothing_disabled: bits.flag()?,
                persistent_rice_adaptation: {
                    bits.skip(1)?; // high_precision_offsets_enabled_flag: inter only
                    bits.flag()?
                },
                cabac_bypass_alignment: bits.flag()?,
            };
        }
    }
    Ok(Sps {
        id,
        separate_colour_planes,
        chroma_array_type,
        width,
        height,
        crop,
        bit_depth_luma: bit_depth_luma as u8,
        bit_depth_chroma: bit_depth_chroma as u8,
        num_short_term_ref_pic_sets,
        rps_sizes,
        log2_poc_lsb,
        long_term_ref_pics_present,
        num_long_term_ref_pics_sps,
        temporal_mvp,
        log2_min_cb: log2_min_cb as u8,
        log2_ctb: log2_ctb as u8,
        log2_min_tb: log2_min_tb as u8,
        log2_max_tb: log2_max_tb as u8,
        max_transform_hierarchy_depth_intra: max_transform_hierarchy_depth_intra as u8,
        scaling_lists,
        sao,
        pcm,
        strong_intra_smoothing,
        vui: vui_result,
        range,
    })
}

#[derive(Clone, Debug)]
pub struct Pps {
    pub dependent_slice_segments: bool,
    pub output_flag_present: bool,
    pub num_extra_slice_header_bits: u32,
    pub sign_data_hiding: bool,
    pub init_qp: i32,
    pub transform_skip: bool,
    pub cu_qp_delta: bool,
    pub diff_cu_qp_delta_depth: u32,
    pub cb_qp_offset: i32,
    pub cr_qp_offset: i32,
    pub slice_chroma_qp_offsets_present: bool,
    pub transquant_bypass: bool,
    /// More than one tile, which is not supported.
    pub multiple_tiles: bool,
    pub entropy_coding_sync: bool,
    pub loop_filter_across_slices: bool,
    pub deblocking_override_enabled: bool,
    pub deblocking_disabled: bool,
    pub beta_offset_div2: i32,
    pub tc_offset_div2: i32,
    pub scaling_lists: Option<ScalingLists>,
    pub slice_header_extension: bool,
    pub log2_max_transform_skip_size: u32,
    pub cross_component_prediction: bool,
    pub chroma_qp_offset_list_enabled: bool,
    pub diff_cu_chroma_qp_offset_depth: u32,
    pub cb_qp_offset_list: Vec<i32>,
    pub cr_qp_offset_list: Vec<i32>,
    pub log2_sao_offset_scale_luma: u32,
    pub log2_sao_offset_scale_chroma: u32,
}

/// `chroma_444` (from the SPS the PPS belongs to) tells how the 32x32 chroma
/// scaling lists of an explicit list are derived.
pub fn parse_pps(rbsp: &[u8], chroma_444: bool) -> Result<Pps, Error> {
    let mut bits = Bits::new(rbsp);
    let id = bits.ue()?;
    let sps_id = bits.ue()?;
    if id > 63 || sps_id > 15 {
        return Err(bad("parameter set id out of range"));
    }
    let dependent_slice_segments = bits.flag()?;
    let output_flag_present = bits.flag()?;
    let num_extra_slice_header_bits = bits.u(3)?;
    let sign_data_hiding = bits.flag()?;
    bits.skip(1)?; // cabac_init_present_flag: inter slices only
    bits.ue()?; // num_ref_idx_l0_default_active_minus1
    bits.ue()?;
    let init_qp = 26 + bits.se()?;
    bits.skip(1)?; // constrained_intra_pred_flag: nothing to constrain without inter blocks
    let transform_skip = bits.flag()?;
    let cu_qp_delta = bits.flag()?;
    let diff_cu_qp_delta_depth = if cu_qp_delta { bits.ue()? } else { 0 };
    let cb_qp_offset = bits.se()?;
    let cr_qp_offset = bits.se()?;
    let slice_chroma_qp_offsets_present = bits.flag()?;
    bits.skip(2)?; // weighted prediction
    let transquant_bypass = bits.flag()?;
    let tiles = bits.flag()?;
    let entropy_coding_sync = bits.flag()?;
    let mut multiple_tiles = false;
    if tiles {
        let columns = bits.ue()? + 1;
        let rows = bits.ue()? + 1;
        let uniform = bits.flag()?;
        if !uniform {
            for _ in 0..columns - 1 + rows - 1 {
                bits.ue()?;
            }
        }
        multiple_tiles = columns * rows > 1;
        bits.skip(1)?; // loop_filter_across_tiles_enabled_flag
    }
    let loop_filter_across_slices = bits.flag()?;
    let (mut deblocking_override_enabled, mut deblocking_disabled) = (false, false);
    let (mut beta_offset_div2, mut tc_offset_div2) = (0, 0);
    if bits.flag()? {
        deblocking_override_enabled = bits.flag()?;
        deblocking_disabled = bits.flag()?;
        if !deblocking_disabled {
            beta_offset_div2 = bits.se()?;
            tc_offset_div2 = bits.se()?;
        }
    }
    let scaling_lists = if bits.flag()? {
        Some(ScalingLists::parse(&mut bits, chroma_444)?)
    } else {
        None
    };
    bits.skip(1)?; // lists_modification_present_flag
    bits.ue()?; // log2_parallel_merge_level_minus2
    let slice_header_extension = bits.flag()?;
    let mut pps = Pps {
        dependent_slice_segments,
        output_flag_present,
        num_extra_slice_header_bits,
        sign_data_hiding,
        init_qp,
        transform_skip,
        cu_qp_delta,
        diff_cu_qp_delta_depth,
        cb_qp_offset,
        cr_qp_offset,
        slice_chroma_qp_offsets_present,
        transquant_bypass,
        multiple_tiles,
        entropy_coding_sync,
        loop_filter_across_slices,
        deblocking_override_enabled,
        deblocking_disabled,
        beta_offset_div2,
        tc_offset_div2,
        scaling_lists,
        slice_header_extension,
        log2_max_transform_skip_size: 2,
        cross_component_prediction: false,
        chroma_qp_offset_list_enabled: false,
        diff_cu_chroma_qp_offset_depth: 0,
        cb_qp_offset_list: Vec::new(),
        cr_qp_offset_list: Vec::new(),
        log2_sao_offset_scale_luma: 0,
        log2_sao_offset_scale_chroma: 0,
    };
    if bits.flag().unwrap_or(false) {
        let range_flag = bits.flag().unwrap_or(false);
        bits.skip(7).ok();
        if range_flag {
            if pps.transform_skip {
                pps.log2_max_transform_skip_size = bits.ue()? + 2;
            }
            pps.cross_component_prediction = bits.flag()?;
            pps.chroma_qp_offset_list_enabled = bits.flag()?;
            if pps.chroma_qp_offset_list_enabled {
                pps.diff_cu_chroma_qp_offset_depth = bits.ue()?;
                let length = bits.ue()? + 1;
                if length > 6 {
                    return Err(bad("the chroma QP offset list is too long"));
                }
                for _ in 0..length {
                    pps.cb_qp_offset_list.push(bits.se()?);
                    pps.cr_qp_offset_list.push(bits.se()?);
                }
            }
            pps.log2_sao_offset_scale_luma = bits.ue()?;
            pps.log2_sao_offset_scale_chroma = bits.ue()?;
        }
    }
    Ok(pps)
}

/// A picture parameter set with every tool off, for tests.
#[cfg(test)]
pub fn test_pps() -> Pps {
    Pps {
        dependent_slice_segments: false,
        output_flag_present: false,
        num_extra_slice_header_bits: 0,
        sign_data_hiding: false,
        init_qp: 26,
        transform_skip: false,
        cu_qp_delta: false,
        diff_cu_qp_delta_depth: 0,
        cb_qp_offset: 0,
        cr_qp_offset: 0,
        slice_chroma_qp_offsets_present: false,
        transquant_bypass: false,
        multiple_tiles: false,
        entropy_coding_sync: false,
        loop_filter_across_slices: false,
        deblocking_override_enabled: false,
        deblocking_disabled: false,
        beta_offset_div2: 0,
        tc_offset_div2: 0,
        scaling_lists: None,
        slice_header_extension: false,
        log2_max_transform_skip_size: 2,
        cross_component_prediction: false,
        chroma_qp_offset_list_enabled: false,
        diff_cu_chroma_qp_offset_depth: 0,
        cb_qp_offset_list: Vec::new(),
        cr_qp_offset_list: Vec::new(),
        log2_sao_offset_scale_luma: 0,
        log2_sao_offset_scale_chroma: 0,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn diagonal_scans() {
        let scan = diagonal_scan(4);
        assert_eq!(
            &scan[..6],
            &[(0, 0), (0, 1), (1, 0), (0, 2), (1, 1), (2, 0)]
        );
        assert_eq!(scan[15], (3, 3));
        assert_eq!(diagonal_scan(8).len(), 64);
        assert_eq!(diagonal_scan(2), [(0, 0), (0, 1), (1, 0), (1, 1)]);
    }
    #[test]
    fn default_scaling_lists_are_symmetric_around_dc() {
        let lists = ScalingLists::default_lists();
        assert!(lists.factors[0][0].iter().all(|&v| v == 16));
        assert_eq!(lists.factors[1][0][0], 16);
        // The bottom-right coefficient of the intra list is the largest.
        assert_eq!(*lists.factors[1][0].last().unwrap(), 115);
        assert_eq!(*lists.factors[1][3].last().unwrap(), 91);
        assert_eq!(lists.factors[3][0].len(), 32 * 32);
    }
}
