use crate::shred::{
    Error, MAX_DATA_SHREDS_PER_SLOT, ShredType, traits::ShredData as ShredDataTrait,
};

#[inline]
pub(super) fn erasure_shard_index<T: ShredDataTrait>(shred: &T) -> Option<usize> {
    let fec_set_index = shred.common_header().fec_set_index;
    let index = shred.common_header().index.checked_sub(fec_set_index)?;
    usize::try_from(index).ok()
}

pub(super) fn sanitize<T: ShredDataTrait>(shred: &T) -> Result<(), Error> {
    use crate::shred::ShredFlags;
    if shred.payload().len() != T::SIZE_OF_PAYLOAD {
        return Err(Error::InvalidPayloadSize(shred.payload().len()));
    }
    let common_header = shred.common_header();
    let data_header = shred.data_header();
    // FIREDANCER: We support an option to increase the max shreds
    // per block, even though these blocks would violate consensus
    // limits.  Otherwise, these limits can limit performance during
    // benchmarking.
    unsafe extern "C" {
        fn fd_ext_larger_shred_limits_per_block() -> i32;
    }
    let max_data_shred_per_slot= if unsafe { fd_ext_larger_shred_limits_per_block() } != 0 {
        32 * MAX_DATA_SHREDS_PER_SLOT
    } else { 
        MAX_DATA_SHREDS_PER_SLOT
    };
    if common_header.index as usize >= max_data_shred_per_slot {
        return Err(Error::InvalidShredIndex(
            ShredType::Data,
            common_header.index,
        ));
    }
    let flags = data_header.flags;
    if flags.intersects(ShredFlags::LAST_SHRED_IN_SLOT)
        && !flags.contains(ShredFlags::DATA_COMPLETE_SHRED)
    {
        return Err(Error::InvalidShredFlags(data_header.flags.bits()));
    }
    let _data = shred.data()?;
    let _parent = shred.parent()?;
    let _shard_index = shred.erasure_shard_index()?;
    let _erasure_shard = shred.erasure_shard()?;
    Ok(())
}
