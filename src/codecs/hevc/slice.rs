//! Slice segment headers of intra pictures.
use super::{
    bits::{Bits, bad},
    params::{Pps, Sps, short_term_ref_pic_set},
};
use crate::error::Error;

#[derive(Clone, Debug)]
pub struct SliceHeader {
    pub first_in_picture: bool,
    pub pps_id: u32,
    pub dependent: bool,
    pub segment_address: u32,
    pub sao_luma: bool,
    pub sao_chroma: bool,
    /// `SliceQpY`.
    pub qp: i32,
    pub cb_qp_offset: i32,
    pub cr_qp_offset: i32,
    pub cu_chroma_qp_offset_enabled: bool,
    pub deblocking_disabled: bool,
    pub beta_offset_div2: i32,
    pub tc_offset_div2: i32,
    pub loop_filter_across_slices: bool,
    /// Sizes of the substreams, in bytes of the escaped payload.
    pub entry_points: Vec<u32>,
    /// Where the slice data starts, in bytes of the unescaped payload.
    pub data_offset: usize,
}

fn ceil_log2(value: u32) -> u32 {
    32 - value.saturating_sub(1).leading_zeros()
}

/// Reads the id of the picture parameter set a slice refers to, without the rest.
pub fn peek_pps_id(rbsp: &[u8], nal_type: u8) -> Result<u32, Error> {
    let mut bits = Bits::new(rbsp);
    bits.skip(1)?;
    if (16..=23).contains(&nal_type) {
        bits.skip(1)?;
    }
    bits.ue()
}

/// Parses a header. `previous` is the header of the independent slice segment
/// that a dependent one continues.
pub fn parse(
    rbsp: &[u8],
    nal_type: u8,
    sps: &Sps,
    pps: &Pps,
    previous: Option<&SliceHeader>,
) -> Result<SliceHeader, Error> {
    let mut bits = Bits::new(rbsp);
    let first_in_picture = bits.flag()?;
    if (16..=23).contains(&nal_type) {
        bits.skip(1)?; // no_output_of_prior_pics_flag
    }
    let pps_id = bits.ue()?;
    let mut dependent = false;
    let mut segment_address = 0;
    if !first_in_picture {
        if pps.dependent_slice_segments {
            dependent = bits.flag()?;
        }
        let ctbs = sps.width_in_ctbs() * sps.height_in_ctbs();
        segment_address = bits.u(ceil_log2(ctbs))?;
        if segment_address >= ctbs {
            return Err(bad("a slice starts outside the picture"));
        }
    }
    let mut header = if dependent {
        previous
            .cloned()
            .ok_or_else(|| bad("a dependent slice has no slice before it"))?
    } else {
        for _ in 0..pps.num_extra_slice_header_bits {
            bits.skip(1)?;
        }
        let slice_type = bits.ue()?;
        if slice_type != 2 {
            // Predicted pictures need earlier pictures, which a still has not.
            return Err(Error::Unsupported);
        }
        if pps.output_flag_present {
            bits.skip(1)?;
        }
        if sps.separate_colour_planes {
            return Err(Error::Unsupported);
        }
        if nal_type != 19 && nal_type != 20 {
            bits.skip(sps.log2_poc_lsb as usize)?;
            if !bits.flag()? {
                let mut sizes = sps.rps_sizes.clone();
                short_term_ref_pic_set(
                    &mut bits,
                    sps.num_short_term_ref_pic_sets,
                    sps.num_short_term_ref_pic_sets,
                    &mut sizes,
                )?;
            } else if sps.num_short_term_ref_pic_sets > 1 {
                bits.skip(ceil_log2(sps.num_short_term_ref_pic_sets) as usize)?;
            }
            if sps.long_term_ref_pics_present {
                let mut from_sps = 0;
                if sps.num_long_term_ref_pics_sps > 0 {
                    from_sps = bits.ue()?;
                }
                let explicit = bits.ue()?;
                if from_sps + explicit > 32 {
                    return Err(bad("too many long-term reference pictures"));
                }
                for i in 0..from_sps + explicit {
                    if i < from_sps {
                        if sps.num_long_term_ref_pics_sps > 1 {
                            bits.skip(ceil_log2(sps.num_long_term_ref_pics_sps) as usize)?;
                        }
                    } else {
                        bits.skip(sps.log2_poc_lsb as usize + 1)?;
                    }
                    if bits.flag()? {
                        bits.ue()?;
                    }
                }
            }
            if sps.temporal_mvp {
                bits.skip(1)?;
            }
        }
        let (mut sao_luma, mut sao_chroma) = (false, false);
        if sps.sao {
            sao_luma = bits.flag()?;
            if sps.chroma_array_type != 0 {
                sao_chroma = bits.flag()?;
            }
        }
        let qp = pps.init_qp + bits.se()?;
        let (mut cb_qp_offset, mut cr_qp_offset) = (0, 0);
        if pps.slice_chroma_qp_offsets_present {
            cb_qp_offset = bits.se()?;
            cr_qp_offset = bits.se()?;
        }
        let cu_chroma_qp_offset_enabled = pps.chroma_qp_offset_list_enabled && bits.flag()?;
        let mut deblocking_disabled = pps.deblocking_disabled;
        let (mut beta_offset_div2, mut tc_offset_div2) = (pps.beta_offset_div2, pps.tc_offset_div2);
        if pps.deblocking_override_enabled && bits.flag()? {
            deblocking_disabled = bits.flag()?;
            if !deblocking_disabled {
                beta_offset_div2 = bits.se()?;
                tc_offset_div2 = bits.se()?;
            }
        }
        let mut loop_filter_across_slices = pps.loop_filter_across_slices;
        if pps.loop_filter_across_slices && (sao_luma || sao_chroma || !deblocking_disabled) {
            loop_filter_across_slices = bits.flag()?;
        }
        let min_qp = -sps.qp_bd_offset_luma();
        if !(min_qp..=51).contains(&qp) {
            return Err(bad("the slice quantization parameter is out of range"));
        }
        SliceHeader {
            first_in_picture,
            pps_id,
            dependent,
            segment_address,
            sao_luma,
            sao_chroma,
            qp,
            cb_qp_offset,
            cr_qp_offset,
            cu_chroma_qp_offset_enabled,
            deblocking_disabled,
            beta_offset_div2,
            tc_offset_div2,
            loop_filter_across_slices,
            entry_points: Vec::new(),
            data_offset: 0,
        }
    };
    header.first_in_picture = first_in_picture;
    header.pps_id = pps_id;
    header.dependent = dependent;
    header.segment_address = segment_address;
    header.entry_points.clear();
    if pps.multiple_tiles {
        return Err(Error::Unsupported);
    }
    if pps.entropy_coding_sync {
        let count = bits.ue()?;
        if count > sps.height_in_ctbs() {
            return Err(bad("too many entry points"));
        }
        if count > 0 {
            let length = bits.ue()? + 1;
            if length > 32 {
                return Err(bad("entry point offsets are too wide"));
            }
            for _ in 0..count {
                header.entry_points.push(bits.u(length)?.wrapping_add(1));
            }
        }
    }
    if pps.slice_header_extension {
        let length = bits.ue()? as usize;
        bits.skip(length * 8)?;
    }
    // byte_alignment(): a one bit, then zeros up to the next byte.
    if !bits.flag()? {
        return Err(bad("the slice header is not terminated"));
    }
    bits.byte_align();
    header.data_offset = bits.byte_position();
    Ok(header)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn ceil_log2_values() {
        assert_eq!(ceil_log2(0), 0);
        assert_eq!(ceil_log2(1), 0);
        assert_eq!(ceil_log2(2), 1);
        assert_eq!(ceil_log2(3), 2);
        assert_eq!(ceil_log2(1024), 10);
        assert_eq!(ceil_log2(1025), 11);
    }
}
