/// FIREDANCER: Repalce PohRecorder completely with one that goes out to
///             our implementation.
use solana_pubkey::Pubkey;
use solana_hash::Hash;
use solana_clock::Slot;
use solana_runtime::{installed_scheduler_pool::BankWithScheduler,bank::Bank};
use solana_ledger::blockstore::Blockstore;
use solana_entry::block_component::{BlockFooterV1, VersionedBlockMarker};
use crossbeam_channel::{Sender, Receiver, TrySendError};

use solana_ledger::leader_schedule_cache::LeaderScheduleCache;
use solana_poh_config::PohConfig;

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use solana_clock::BankId;
use solana_transaction::versioned::VersionedTransaction;
use crate::old_poh_recorder::{self, SharedLeaderState, LeaderState, PohRecorderError};
use crate::poh_recorder::{PohLeaderStatus, WorkingBankEntryOrMarker};
use crate::poh_service::PohService;

pub(crate) type Result<T> = std::result::Result<T, PohRecorderError>;

unsafe extern "C" {
    fn fd_ext_poh_initialize(tick_duration_nanos: u64, hashcnt_per_tick: u64, ticks_per_slot: u64, tick_height: u64, last_entry_hash: *const u8, signal_leader_change: *mut c_void);
    fn fd_ext_poh_acquire_leader_bank() -> *const c_void;
    fn fd_ext_poh_reset_slot() -> u64;
    fn fd_ext_poh_reached_leader_slot(out_leader_slot: *mut u64, out_reset_slot: *mut u64) -> i32;
    fn fd_ext_poh_begin_leader(bank: *const c_void, slot: u64, epoch: u64, hashcnt_per_tick: u64, tick_duration_nanos: u64, cus_block_limit: u64, cus_vote_cost_limit: u64, cus_account_cost_limit: u64, cus_allocated_data_size_limit: u64, max_data_shreds: u64);
    fn fd_ext_poh_reset(reset_bank_slot: u64, reset_blockhash: *const u8, hashcnt_per_tick: u64, tick_duration_nanos: u64, block_id: *const u8, features_activation_slot: *const u64, shred_slot_limits: *const u64);
    fn fd_ext_poh_get_leader_after_n_slots(n: u64, out_pubkey: *mut u8) -> i32;
    fn fd_ext_poh_update_active_descendant(max_active_descendant: u64);
}

#[unsafe(no_mangle)]
pub extern "C" fn fd_ext_poh_signal_leader_change( sender: *mut c_void ) {
    if sender.is_null() {
        return;
    }

    let sender: &Sender<bool> = unsafe { &*(sender as *mut Sender<bool>) };
    match sender.try_send(true) {
        Ok(()) | Err(TrySendError::Full(_)) => (),
        err => err.unwrap(),
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn fd_ext_poh_register_tick( bank: *const c_void, hash: *const u8 ) {
    let hash = unsafe { std::slice::from_raw_parts(hash, 32) };
    let hash = Hash::new_from_array(hash.try_into().unwrap());
    unsafe { (*(bank as *const Bank)).register_tick(&hash, &BankWithScheduler::no_scheduler_available()) };
}

const FD_POH_RECORDER_FEATURES_OF_INTEREST_CNT: usize = 3usize;
static FD_POH_RECORDER_FEATURES_OF_INTEREST: [Pubkey; FD_POH_RECORDER_FEATURES_OF_INTEREST_CNT] = [
    agave_feature_set::enforce_fixed_fec_set::id(),
    agave_feature_set::switch_to_chacha8_turbine::id(),
    agave_feature_set::discard_unexpected_data_complete_shreds::id(),
];

pub struct PohRecorder {
  pub is_exited: Arc<AtomicBool>,
  pub shared_leader_state: SharedLeaderState,
  pub ticks_per_slot: u64,
  // Alpenglow related migration things
  pub is_alpenglow_enabled: bool,
}

impl PohRecorder {
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_clear_signal(
        tick_height: u64,
        last_entry_hash: Hash,
        _start_bank: Arc<Bank>,
        next_leader_slot: Option<(Slot, Slot)>,
        ticks_per_slot: u64,
        _delay_leader_block_for_pending_fork: bool,
        _blockstore: Arc<Blockstore>,
        clear_bank_signal: Option<Sender<bool>>,
        _leader_schedule_cache: &Arc<LeaderScheduleCache>,
        poh_config: &PohConfig,
        is_exited: Arc<AtomicBool>,
    ) -> (Self, Receiver<WorkingBankEntryOrMarker>) {
        /* Just silence the unused warning for old_poh_recorder, without needing to modify the file. */
        let _silence_warnings = super::old_poh_recorder::create_test_recorder;

        let clear_bank_sender: *mut Sender<bool> = match clear_bank_signal {
            Some(sender) => Box::into_raw(Box::new(sender)),
            None => std::ptr::null_mut(),
        };

        let (leader_first_tick_height, _, _) = crate::old_poh_recorder::PohRecorder::compute_leader_slot_tick_heights(next_leader_slot, ticks_per_slot);

        let target_tick_duration_nanos: u64 = PohService::target_tick_ns_adjusted(
            ticks_per_slot, poh_config.target_tick_duration.as_nanos().try_into().unwrap() );

        unsafe { fd_ext_poh_initialize(target_tick_duration_nanos, poh_config.hashes_per_tick.unwrap_or(1), ticks_per_slot, tick_height, last_entry_hash.as_ref().as_ptr(), clear_bank_sender as *mut c_void) };

        let dummy1 = crossbeam_channel::unbounded();
        /* Forget so the receiver doesn't see the channel is disconnected. */
        std::mem::forget(dummy1.0);
        (Self { is_exited: is_exited,
                shared_leader_state: SharedLeaderState::new(tick_height, leader_first_tick_height, next_leader_slot),
                ticks_per_slot,
                is_alpenglow_enabled: false }, dummy1.1)
    }

    pub fn leader_after_n_slots(&self, slots: u64) -> Option<Pubkey> {
        /* Must be implemented. Used to determine where to send our votes. */
        let mut pubkey = [0u8; 32];
        unsafe {
            if 1==fd_ext_poh_get_leader_after_n_slots(slots, pubkey.as_mut_ptr()) {
                Some(Pubkey::new_from_array(pubkey))
            } else {
                None
            }
        }
    }

    pub fn leader_and_slot_after_n_slots(
        &self,
        _slots_in_the_future: u64,
    ) -> Option<(Pubkey, Slot)> {
        /* Not needed for any important functionality, only the RPC send
           transaction service. */
        None
    }

    pub fn would_be_leader(&self, _within_next_n_ticks: u64) -> bool {
        /* The only caller asks if it's within the next ten minutes, so
            that it can forward gossiped votes to ourselves.  We can just
            always forward them. */
        true
    }

    pub fn shared_leader_state(&self) -> SharedLeaderState {
        /* Must be implemented, used by replay stage. */
        self.shared_leader_state.clone()
    }

    pub fn ticks_per_slot(&self) -> u64 {
        /* Called in banking_stage.rs */
        self.ticks_per_slot
    }

    pub fn has_bank(&self) -> bool {
        /* Must be implemented, used by replay stage. */
        self.bank().is_some()
    }

    pub fn bank(&self) -> Option<Arc<Bank>> {
        /* Must be implemented, used by replay stage. */
        let bank: *const Bank = unsafe { fd_ext_poh_acquire_leader_bank() } as *const Bank;

        if bank.is_null() {
            None
        } else {
            Some(unsafe { Arc::from_raw( bank ) })
        }
    }

    pub fn update_start_bank_active_descendants(&mut self, active_descendants: &[Slot]) {
        unsafe { fd_ext_poh_update_active_descendant(*active_descendants.iter().max().unwrap_or(&0)) };
    }

    pub fn start_slot(&self) -> Slot {
        /* Must be implemented, used by replay stage. */
        unsafe { fd_ext_poh_reset_slot() - 1 }
    }

    pub fn reached_leader_slot(&self, _pubkey: &Pubkey) -> PohLeaderStatus {
        /* Must be implemented, used by replay stage.
           The pubkey currently used here is always the
           leader pubkey only, so it can be ignored. */
        let mut leader_slot: u64 = 0;
        let mut reset_slot: u64 = 0;
        let is_leader = unsafe { fd_ext_poh_reached_leader_slot(&mut leader_slot, &mut reset_slot ) };

        if is_leader != 0 {
            PohLeaderStatus::Reached {
                poh_slot: leader_slot,
                parent_slot: reset_slot - 1,
            }
        } else {
            PohLeaderStatus::NotReached
        }
    }

    pub fn set_bank(&mut self, bank_with_scheduler: BankWithScheduler) {
        /* Must be implemented, used by replay stage. */
        let bank = bank_with_scheduler.clone_without_scheduler();
        let slot = bank.slot();
        let epoch = bank.epoch();
        let hashes_per_tick = bank.hashes_per_tick().unwrap_or(1);
        let tick_duration_nanos = PohService::target_tick_ns_adjusted(
            self.ticks_per_slot, (bank.ns_per_slot_at_slot(slot) / self.ticks_per_slot.max(1) as u128) as u64 );

        /* Removed in https://github.com/anza-xyz/agave/pull/12902 */
        let cus_vote_cost_limit =  solana_cost_model::block_cost_limits::MAX_BLOCK_UNITS;
        let cus_block_limit = bank.read_cost_tracker().unwrap().get_block_limit();
        let cus_account_cost_limit = bank.read_cost_tracker().unwrap().get_account_limit();
        let cus_allocated_data_size_limit = bank.read_cost_tracker().unwrap().get_allocated_data_size_limit();
        let max_data_shreds: u64 = bank.max_data_shreds_per_slot() as u64;

        let leader_state = self.shared_leader_state.load();
        let leader_first_tick_height = leader_state.leader_first_tick_height();
        let tick_height = leader_state.tick_height();
        let next_leader_slot = leader_state.next_leader_slot_range();
        drop(leader_state);
        self.shared_leader_state.store(Arc::new(LeaderState::new(
            Some(bank.clone()),
            tick_height,
            leader_first_tick_height,
            next_leader_slot,
        )));

        let leader_bank: *const Bank = Arc::into_raw( bank );
        unsafe { fd_ext_poh_begin_leader( leader_bank as *const c_void, slot, epoch, hashes_per_tick, tick_duration_nanos, cus_block_limit, cus_vote_cost_limit, cus_account_cost_limit, cus_allocated_data_size_limit, max_data_shreds ) };
    }

    pub fn reset(&mut self, reset_bank: Arc<Bank>, next_leader_slot: Option<(Slot, Slot)>) {
        /* Must be implemented, used by replay stage. */
        let tick_height = (self.start_slot() + 1) * self.ticks_per_slot;
        let (leader_first_tick_height, _, _) =
            crate::old_poh_recorder::PohRecorder::compute_leader_slot_tick_heights(next_leader_slot, self.ticks_per_slot);
        self.shared_leader_state.store(Arc::new(LeaderState::new(
            None,
            tick_height,
            leader_first_tick_height,
            next_leader_slot,
        )));

        let reset_bank_slot = reset_bank.slot();
        let reset_bank_blockhash = reset_bank.last_blockhash();
        let hashes_per_tick = reset_bank.hashes_per_tick().unwrap_or(1);
        let tick_duration_nanos = PohService::target_tick_ns_adjusted(
            self.ticks_per_slot, (reset_bank.ns_per_slot_at_slot(reset_bank_slot) / self.ticks_per_slot.max(1) as u128) as u64 );

        let block_id = reset_bank.block_id().unwrap_or_default();
        let block_id_ptr = if let Some(_block_id) = reset_bank.block_id() {
            /* _block_id scope ends here. We can't use _block_id.as_ref().as_ptr()
               as it points to memory with an incorrect value.
               We must use block_id, that lives beyond this scope. */
            block_id.as_ref().as_ptr()
        } else {
            std::ptr::null()
        };

        /* There is a subset of FD_POH_RECORDER_FEATURES_OF_INTEREST
           activation slots that the shred tile needs to be aware of.
           Due to the fact that their computation requires the bank,
           we are forced (so far) to implement it here, sending them
           to the poh tile as an intermediary (before forwarding them
           to the shred tile).

           This also applies to the shred_slot_limits that change with
           the reduce_slot_time feature gates, which are sent along the
           same path to the shred tile. */
        let mut features_activation_slot: [u64; FD_POH_RECORDER_FEATURES_OF_INTEREST_CNT] = [u64::MAX; FD_POH_RECORDER_FEATURES_OF_INTEREST_CNT];
        for (i, pubkey) in FD_POH_RECORDER_FEATURES_OF_INTEREST.iter().enumerate() {
            features_activation_slot[i] = match reset_bank.feature_set.activated_slot(pubkey) {
                None => u64::MAX,
                Some(feature_slot) => {
                    let epoch_schedule = reset_bank.epoch_schedule();
                    let feature_epoch = epoch_schedule.get_epoch(feature_slot);
                    epoch_schedule.get_first_slot_in_epoch(feature_epoch + 1)
                }
            }
        }

        let shred_slot_limits: [u64; 5] = reset_bank.shred_slot_limits( reset_bank_slot );
        unsafe { fd_ext_poh_reset( reset_bank_slot, reset_bank_blockhash.as_ref().as_ptr(),
                  hashes_per_tick, tick_duration_nanos, block_id_ptr, features_activation_slot.as_ref().as_ptr(),
                  shred_slot_limits.as_ref().as_ptr() ) };
    }

    pub fn track_transaction_indexes(&mut self) {
        /* No-op - handled internally */
    }

    pub fn enable_alpenglow(&mut self) {
        /* Unimplemented, Alpenglow */
        self.is_alpenglow_enabled = false;
    }

    pub fn record(
        &mut self,
        _bank_id: BankId,
        _mixin: Hash,
        _transactions: Vec<VersionedTransaction>,
    ) -> old_poh_recorder::Result<old_poh_recorder::RecordSummary> {
        /* Unimplemented, used by PohService */
        unimplemented!("firedancer does not use BlockCreationLoop")
    }

    pub fn tick_alpenglow(
        &mut self,
        _max_tick_height: u64,
        _footer: BlockFooterV1,
    ) -> old_poh_recorder::Result<()> {
        /* Unimplemented, Alpenglow */
        unimplemented!("firedancer does not use BlockCreationLoop")
    }

    pub fn clear_bank(&mut self, _set_shared_state: bool) {
        /* Unimplemented, used by BlockCreationLoop */
        unimplemented!("firedancer does not use BlockCreationLoop")
    }

    pub fn send_marker(&mut self, _marker: VersionedBlockMarker) -> Result<()> {
        /* No-op - used by BlockCreationLoop */
        Ok(())
    }
}
