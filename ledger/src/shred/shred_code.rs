use crate::shred::{
    DATA_SHREDS_PER_FEC_BLOCK, Error, MAX_CODE_SHREDS_PER_SLOT, MAX_DATA_SHREDS_PER_SLOT,
    ShredType, traits::ShredCode as ShredCodeTrait,
};

#[inline]
pub(super) fn erasure_shard_index<T: ShredCodeTrait>(shred: &T) -> Option<usize> {
    // Assert that the last shred index in the erasure set does not
    // overshoot MAX_{DATA,CODE}_SHREDS_PER_SLOT.

    // FIREDANCER: We support an option to increase the max shreds
    // per block, even though these blocks would violate consensus
    // limits.  Otherwise, these limits can limit performance during
    // benchmarking.
    unsafe extern "C" {
        fn fd_ext_larger_shred_limits_per_block() -> i32;
    }
    let (max_data_shred_per_slot, max_code_shreds_per_slot) = if unsafe { fd_ext_larger_shred_limits_per_block() } != 0 {
        (32 * MAX_DATA_SHREDS_PER_SLOT, 32 * MAX_CODE_SHREDS_PER_SLOT)
    } else { 
        (MAX_DATA_SHREDS_PER_SLOT, MAX_CODE_SHREDS_PER_SLOT)
    };

    let common_header = shred.common_header();
    let coding_header = shred.coding_header();
    if common_header
        .fec_set_index
        .checked_add(u32::from(coding_header.num_data_shreds.checked_sub(1)?))? as usize
        // FIREDANCER: Allow increasing the limit during benchmarking
        >= max_data_shred_per_slot
    {
        return None;
    }
    if shred
        .first_coding_index()?
        .checked_add(u32::from(coding_header.num_coding_shreds.checked_sub(1)?))? as usize
        // FIREDANCER: Allow increasing the limit during benchmarking
        >= max_code_shreds_per_slot
    {
        return None;
    }
    let num_data_shreds = usize::from(coding_header.num_data_shreds);
    let num_coding_shreds = usize::from(coding_header.num_coding_shreds);
    let position = usize::from(coding_header.position);
    let fec_set_size = num_data_shreds.checked_add(num_coding_shreds)?;
    let index = position.checked_add(num_data_shreds)?;
    (index < fec_set_size).then_some(index)
}

pub(super) fn sanitize<T: ShredCodeTrait>(shred: &T) -> Result<(), Error> {
    if shred.payload().len() != T::SIZE_OF_PAYLOAD {
        return Err(Error::InvalidPayloadSize(shred.payload().len()));
    }
    let common_header = shred.common_header();
    let coding_header = shred.coding_header();
    // FIREDANCER: We support an option to increase the max shreds
    // per block, even though these blocks would violate consensus
    // limits.  Otherwise, these limits can limit performance during
    // benchmarking.
    unsafe extern "C" {
        fn fd_ext_larger_shred_limits_per_block() -> i32;
    }
    let max_code_shreds_per_slot = if unsafe { fd_ext_larger_shred_limits_per_block() } != 0 {
        32 * MAX_CODE_SHREDS_PER_SLOT
    } else {
        MAX_CODE_SHREDS_PER_SLOT
    };
    if common_header.index as usize >= max_code_shreds_per_slot {
        return Err(Error::InvalidShredIndex(
            ShredType::Code,
            common_header.index,
        ));
    }
    let num_coding_shreds = usize::from(coding_header.num_coding_shreds);
    if num_coding_shreds > 8 * DATA_SHREDS_PER_FEC_BLOCK {
        return Err(Error::InvalidNumCodingShreds(
            coding_header.num_coding_shreds,
        ));
    }
    let _shard_index = shred.erasure_shard_index()?;
    let _erasure_shard = shred.erasure_shard()?;
    Ok(())
}
