// To use this file, place it in ledger/src/shred_proof.rs, add the following to ledger/src/lib.rs
//      #[cfg(kani)]
//      pub mod shred_proof;
//
// Install Kani, then run with 
//      cargo kani --manifest-path ledger/Cargo.toml --tests -Z stubbing
//
// It takes about 5-10 minutes on a Zen 4 CPU to run the proofs.  It prints many warning-seeming
// messages, but in the end, shows: 
// Complete - 4 successfully verified harnesses, 0 failures, 4 total.

#[cfg(kani)]
mod verification {
    use crate::shred::*;

    use solana_perf::packet::Packet;

    // Byte offsets of fields in the shred layout
    const OFF_VARIANT: usize = 64;
    const OFF_SLOT: usize = 65;
    const OFF_INDEX: usize = 73;
    const OFF_VERSION: usize = 77;
    const OFF_FEC_SET_INDEX: usize = 79;
    // Data-shred specific
    const OFF_PARENT_OFFSET: usize = 83;
    const OFF_DATA_FLAGS: usize = 85;
    const OFF_DATA_SIZE: usize = 86;
    // Code-shred specific
    const OFF_NUM_DATA: usize = 83;
    const OFF_NUM_CODING: usize = 85;
    const OFF_POSITION: usize = 87;

    const DATA_PAYLOAD_SZ: usize = 1203;
    const CODE_PAYLOAD_SZ: usize = 1228;

    // Kani gets stuck on the drop if it is not stubbed out
    #[allow(dead_code)]
    unsafe fn noop_drop_error<T>(_: *mut T) {}

    /// Helper: write a little-endian u16 at the given offset.
    fn write_u16(buf: &mut [u8], off: usize, v: u16) {
        buf[off..off + 2].copy_from_slice(&v.to_le_bytes());
    }

    /// Helper: write a little-endian u32 at the given offset.
    fn write_u32(buf: &mut [u8], off: usize, v: u32) {
        buf[off..off + 4].copy_from_slice(&v.to_le_bytes());
    }

    /// Helper: write a little-endian u64 at the given offset.
    fn write_u64(buf: &mut [u8], off: usize, v: u64) {
        buf[off..off + 8].copy_from_slice(&v.to_le_bytes());
    }

    #[kani::proof]
    #[kani::stub(std::ptr::drop_in_place::<crate::shred::Error>, noop_drop_error)]
    fn data_shred_invariants() {
        // Symbolic inputs
        let slot: u64 = kani::any();
        let index: u32 = kani::any();
        let version: u16 = kani::any();
        let fec_set_index: u32 = kani::any();
        let parent_offset: u16 = kani::any();
        let flags: u8 = kani::any();
        let variant: u8 = kani::any();
        let data_sz: u16 = kani::any();

        let root: u64 = 0;
        let max_slot: u64 = u64::MAX;

        // This proof is not for coding shreds
        kani::assume(variant & 0xC0!=0x40);

        // This is only needed because should_discard_shred does an addition that can (harmlessly)
        // overflow.  You can run with --no-overflow-checks and comment out this line.
        kani::assume(fec_set_index<0xFFFFFFE0);

        kani::assume(index < 32_768); // 2
        kani::assume((fec_set_index <= index) && (index<fec_set_index+32)); // 3
        kani::assume(fec_set_index % 32 == 0); // 4
        kani::assume(fec_set_index<=32_736); // 5
        kani::assume((variant&0xF0==0x90)||(variant&0xF0==0xB0)); // 6

        kani::assume(flags & 0xC0 != 0x80); // 8
        kani::assume((flags&0x40==0) || (index%32==31)); // 9
        kani::assume((parent_offset as u64) <= slot); // 10
        kani::assume(!((parent_offset==0)^(slot==0))); // 11

        // 12
        if variant & 0xF0 == 0x90 {
            // Chained
            kani::assume((88 <= data_sz) && (data_sz <= 1171 - 20*((variant as u16)& 0x0F)));
        } else {
            // Chained resigned
            kani::assume((88 <= data_sz) && (data_sz <= 1107 - 20*((variant as u16)& 0x0F)));
        }

        // Now construct the packet
        let mut buf = [0u8; DATA_PAYLOAD_SZ];

        buf[OFF_VARIANT] = variant;
        write_u64(&mut buf, OFF_SLOT, slot);
        write_u32(&mut buf, OFF_INDEX, index);
        write_u16(&mut buf, OFF_VERSION, version);
        write_u32(&mut buf, OFF_FEC_SET_INDEX, fec_set_index);
        write_u16(&mut buf, OFF_PARENT_OFFSET, parent_offset);
        buf[OFF_DATA_FLAGS] = flags;
        write_u16(&mut buf, OFF_DATA_SIZE, data_sz);

        let mut packet = Packet::default();
        packet.buffer_mut()[..DATA_PAYLOAD_SZ].copy_from_slice(&buf);
        packet.meta_mut().size = DATA_PAYLOAD_SZ;

        let mut stats = ShredFetchStats::default();

        let discarded = should_discard_shred(
            &packet,
            root,
            max_slot,
            version,
            |_| true, // discard_unexpected_data_complete_shreds
            &mut stats,
            );

        kani::assert(
            !discarded,
            "should_discard_shred returned true for a data shred satisfying all invariants"
            );
        let result = Shred::new_from_serialized_shred(buf.to_vec());
        let is_ok = result.is_ok();
        std::mem::forget(result);
        kani::assert(is_ok, "new_from_serialized_shred rejected a data shred satisfying all invariants");
    }

    #[kani::proof]
    #[kani::stub(std::ptr::drop_in_place::<crate::shred::Error>, noop_drop_error)]
    fn code_shred_invariants() {
        // Symbolic inputs
        let slot: u64 = kani::any();
        let index: u32 = kani::any();
        let version: u16 = kani::any();
        let fec_set_index: u32 = kani::any();
        let position: u16 = kani::any(); // code.idx
        let num_data: u16 = kani::any();
        let num_coding: u16 = kani::any();
        let variant: u8 = kani::any();

        // Root and max_slot are not shred-level concepts, so make them maximally permissive.
        // However, even that is not enough, because should_discard_shred requires slot>root for
        // coding shreds, which means it always rejects coding shreds for slot 0.
        let root: u64 = 0;
        let max_slot: u64 = u64::MAX;
        kani::assume(slot>0);
        
        // This proof is just for coding shreds
        kani::assume(variant & 0xC0==0x40);

        kani::assume(index < 32_768); // 2
        kani::assume((fec_set_index <= index) && (index<fec_set_index+32)); // 3
        kani::assume(fec_set_index % 32 == 0); // 4
        kani::assume(fec_set_index<=32_736); // 5
        kani::assume((variant&0xF0==0x60)||(variant&0xF0==0x70)); // 6

        kani::assume(position < 32); // 13
        kani::assume((position as u32) <= index); // 14
        kani::assume((num_coding==32) && (num_data==32)); // 15
        kani::assume( index - (position as u32) <= 32_736); // 16

        // Now construct the packet
        let mut buf = [0u8; CODE_PAYLOAD_SZ];

        buf[OFF_VARIANT] = variant;
        write_u64(&mut buf, OFF_SLOT, slot);
        write_u32(&mut buf, OFF_INDEX, index);
        write_u16(&mut buf, OFF_VERSION, version);
        write_u32(&mut buf, OFF_FEC_SET_INDEX, fec_set_index);
        write_u16(&mut buf, OFF_NUM_DATA, num_data);
        write_u16(&mut buf, OFF_NUM_CODING, num_coding);
        write_u16(&mut buf, OFF_POSITION, position);

        let mut packet = Packet::default();
        packet.buffer_mut()[..CODE_PAYLOAD_SZ].copy_from_slice(&buf);
        packet.meta_mut().size = CODE_PAYLOAD_SZ;

        let mut stats = ShredFetchStats::default();

        let discarded = should_discard_shred(
            &packet,
            root,
            max_slot,
            version,
            |_| true, // discard_unexpected_data_complete_shreds
            &mut stats,
            );

        kani::assert(
            !discarded,
            "should_discard_shred returned true for a code shred satisfying all invariants"
            );
        let result = Shred::new_from_serialized_shred(buf.to_vec());
        let is_ok = result.is_ok();
        std::mem::forget(result);
        kani::assert(is_ok, "new_from_serialized_shred rejected a code shred satisfying all invariants");
    }

    #[kani::proof]
    #[kani::stub(std::ptr::drop_in_place::<crate::shred::Error>, noop_drop_error)]
    fn data_shred_invariants_converse() {
        // Symbolic inputs
        let slot: u64 = kani::any();
        let index: u32 = kani::any();
        let version: u16 = kani::any();
        let fec_set_index: u32 = kani::any();
        let parent_offset: u16 = kani::any();
        let flags: u8 = kani::any();
        let variant: u8 = kani::any();
        let data_sz: u16 = kani::any();

        // Root and max_slot are not shred-level concepts, so make them maximally permissive
        let root: u64 = 0;
        let max_slot: u64 = u64::MAX;

        // This proof is not for coding shreds
        kani::assume(variant & 0xC0!=0x40);

        // This is only needed because should_discard_shred does an addition that can (harmlessly)
        // overflow.  You can run with --no-overflow-checks and comment out this line.
        kani::assume(fec_set_index<0xFFFFFFE0);


        // Construct the packet using symbolic values
        let mut buf = [0u8; DATA_PAYLOAD_SZ];

        buf[OFF_VARIANT] = variant;
        write_u64(&mut buf, OFF_SLOT, slot);
        write_u32(&mut buf, OFF_INDEX, index);
        write_u16(&mut buf, OFF_VERSION, version);
        write_u32(&mut buf, OFF_FEC_SET_INDEX, fec_set_index);
        write_u16(&mut buf, OFF_PARENT_OFFSET, parent_offset);
        buf[OFF_DATA_FLAGS] = flags;
        write_u16(&mut buf, OFF_DATA_SIZE, data_sz);

        let mut packet = Packet::default();
        packet.buffer_mut()[..DATA_PAYLOAD_SZ].copy_from_slice(&buf);
        packet.meta_mut().size = DATA_PAYLOAD_SZ;

        let mut stats = ShredFetchStats::default();

        let discarded = should_discard_shred(
            &packet,
            root,
            max_slot,
            version,
            |_| true, // discard_unexpected_data_complete_shreds
            &mut stats,
            );

        kani::assume(
            !discarded
            );
        let result = Shred::new_from_serialized_shred(buf.to_vec());
        let is_ok = result.is_ok();
        std::mem::forget(result);
        kani::assume(is_ok);

        kani::assert(index < 32_768, "2");
        kani::assert((fec_set_index <= index) && (index<fec_set_index+32), "3" );
        kani::assert(fec_set_index % 32 == 0, "4");
        kani::assert(fec_set_index<=32_736, "5");
        kani::assert((variant&0xF0==0x90)||(variant&0xF0==0xB0), "6");

        kani::assert(flags & 0xC0 != 0x80, "8");
        kani::assert((flags&0x40==0) || (index%32==31), "9");
        kani::assert((parent_offset as u64) <= slot, "10");
        kani::assert(!((parent_offset==0)^(slot==0)), "11");

        if variant & 0xF0 == 0x90 {
            // Chained
            kani::assert((88 <= data_sz) && (data_sz <= 1171 - 20*((variant as u16)& 0x0F)), "12a");
        } else {
            // Chained resigned
            kani::assert((88 <= data_sz) && (data_sz <= 1107 - 20*((variant as u16)& 0x0F)), "12b");
        }
    }

    #[kani::proof]
    #[kani::stub(std::ptr::drop_in_place::<crate::shred::Error>, noop_drop_error)]
    fn code_shred_invariants_converse() {
        // Symbolic inputs
        let slot: u64 = kani::any();
        let index: u32 = kani::any();
        let version: u16 = kani::any();
        let fec_set_index: u32 = kani::any();
        let position: u16 = kani::any(); // code.idx
        let num_data: u16 = kani::any();
        let num_coding: u16 = kani::any();
        let variant: u8 = kani::any();

        // Root and max_slot are not shred-level concepts, so make them maximally permissive
        let root: u64 = 0;
        let max_slot: u64 = u64::MAX;
        
        // This proof is just for coding shreds
        kani::assume(variant & 0xC0==0x40);

        // This is only needed because should_discard_shred does an addition that can (harmlessly)
        // overflow.  You can run with --no-overflow-checks and comment out this line.
        kani::assume(fec_set_index<0xFFFFFFE0);

        // Construct the packet using symbolic values
        let mut buf = [0u8; CODE_PAYLOAD_SZ];

        buf[OFF_VARIANT] = variant;
        write_u64(&mut buf, OFF_SLOT, slot);
        write_u32(&mut buf, OFF_INDEX, index);
        write_u16(&mut buf, OFF_VERSION, version);
        write_u32(&mut buf, OFF_FEC_SET_INDEX, fec_set_index);
        write_u16(&mut buf, OFF_NUM_DATA, num_data);
        write_u16(&mut buf, OFF_NUM_CODING, num_coding);
        write_u16(&mut buf, OFF_POSITION, position);

        let mut packet = Packet::default();
        packet.buffer_mut()[..CODE_PAYLOAD_SZ].copy_from_slice(&buf);
        packet.meta_mut().size = CODE_PAYLOAD_SZ;

        let mut stats = ShredFetchStats::default();

        let discarded = should_discard_shred(
            &packet,
            root,
            max_slot,
            version,
            |_| true, // discard_unexpected_data_complete_shreds
            &mut stats,
            );

        kani::assume(
            !discarded,
            );
        let result = Shred::new_from_serialized_shred(buf.to_vec());
        let is_ok = result.is_ok();
        std::mem::forget(result);
        kani::assume(is_ok);


        kani::assert(index < 32_768, "2");
        kani::assert((fec_set_index <= index) && (index<fec_set_index+32), "3" );
        kani::assert(fec_set_index % 32 == 0, "4");
        kani::assert(fec_set_index<=32_736, "5");
        kani::assert((variant&0xF0==0x60)||(variant&0xF0==0x70), "6");

        kani::assert(position < 32, "13");
        kani::assert((position as u32) <= index, "14");
        kani::assert((num_coding==32) && (num_data==32), "15");
        kani::assert( index - (position as u32) <= 32_736, "16");
    }
}
