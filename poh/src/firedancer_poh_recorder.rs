/// FIREDANCER: Repalce PohRecorder completely with one that goes out to
///             our implementation.
use solana_pubkey::Pubkey;
use solana_hash::Hash;
use solana_clock::Slot;
use solana_runtime::{installed_scheduler_pool::BankWithScheduler,bank::Bank};
use solana_ledger::blockstore::Blockstore;
use solana_entry::block_component::{BlockComponent, BlockFooterV1, VersionedBlockMarker};
use crossbeam_channel::{Sender, Receiver, TrySendError};

use solana_ledger::leader_schedule_cache::LeaderScheduleCache;
use solana_poh_config::PohConfig;

use log::{error, trace};
use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

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
    fn fd_ext_poh_begin_leader(bank: *const c_void, slot: u64, epoch: u64, hashcnt_per_tick: u64, tick_duration_nanos: u64, cus_block_limit: u64, cus_vote_cost_limit: u64, cus_account_cost_limit: u64, cus_allocated_data_size_limit: u64, max_data_shreds: u64, vote_only: i32, block_deadline_nanos: u64);
    fn fd_ext_poh_reset(reset_bank_slot: u64, reset_blockhash: *const u8, hashcnt_per_tick: u64, tick_duration_nanos: u64, block_id: *const u8, features_activation_slot: *const u64, shred_slot_limits: *const u64, alpenglow: i32);
    fn fd_ext_poh_get_leader_after_n_slots(n: u64, out_pubkey: *mut u8) -> i32;
    fn fd_ext_poh_update_active_descendant(max_active_descendant: u64);
    fn fd_ext_poh_alpenglow_enable();
    fn fd_ext_poh_alpenglow_begin_tick();
    fn fd_ext_poh_alpenglow_try_get_tick(out_tick_hash: *mut u8) -> i32;
    fn fd_ext_poh_alpenglow_publish_footer(footer: *const u8, footer_sz: u64) -> i32;
    fn fd_ext_poh_alpenglow_publish_marker(marker: *const u8, marker_sz: u64) -> i32;
    fn fd_ext_poh_alpenglow_clear_bank();
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
  /* Nanoseconds left until BlockCreationLoop will end the next block it
     opens, as BCL itself measures it.  Set by BCL immediately before
     set_bank, consumed by the following fd_ext_poh_begin_leader, and
     reset so a set_bank from anywhere else cannot inherit a stale one.

     Without it the poh tile derives pack's deadline from its own
     idealized slot grid, which is anchored at the last reset and does
     not track BCL's window at all.  The two then only agree by
     accident, and the accident can go either way: pack ending late
     leaves microblocks in flight when the tick freezes the bank, which
     panics in Bank::commit_transactions.

     u64::MAX means "not set" -- the legacy grid is used, which is what
     every non-alpenglow caller wants. */
  pub alpenglow_block_deadline_ns: u64,

  /* A clone of the signal handed to the poh tile at initialization.
     Registering the tick only drives tick_height to max; replay is what
     actually freezes the bank, and it will not look until something
     wakes it.  Upstream pokes this before waiting (old_poh_recorder.rs
     wait_for_freeze_and_send_footer) and the port dropped it, so the
     freeze wait was landing on replay's own schedule -- measured 0.4 to
     100.4ms, median 56.4, which was 99% of the entire block completion
     tail. */
  clear_bank_signal: Option<Sender<bool>>,

  /* Identity of the bank the poh tile was last reset onto.  The tile only
     knows the reset slot (fd_ext_poh_reset_slot), and fast leader handover
     can hand the same slot back on a different parent, so BlockCreationLoop
     compares bank ids rather than slots before deciding to reset.  Tracked
     here because nothing on the C side carries a bank id.

     Only meaningful under alpenglow, where BlockCreationLoop is the sole
     resetter.  Under TowerBFT the tile advances its own reset slot without
     telling us, but nothing calls start_bank_id() on that path. */
  start_bank_id: BankId,
}

impl PohRecorder {
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_clear_signal(
        tick_height: u64,
        last_entry_hash: Hash,
        start_bank: Arc<Bank>,
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

        /* Keep a clone: the tile takes ownership of the boxed sender, but
           tick_alpenglow needs to poke replay itself. */
        let clear_bank_signal_local = clear_bank_signal.clone();
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
                is_alpenglow_enabled: false,
                alpenglow_block_deadline_ns: u64::MAX,
                clear_bank_signal: clear_bank_signal_local,
                start_bank_id: start_bank.bank_id() }, dummy1.1)
    }

    /* Wake replay so it freezes the bank now rather than whenever it next
       happens to look.  Copied from old_poh_recorder::notify_replay_wakeup;
       upstream calls this at the top of wait_for_freeze_and_send_footer,
       immediately after the tick is registered. */
    fn notify_replay_wakeup(&self) {
        if let Some(signal) = &self.clear_bank_signal {
            match signal.try_send(true) {
                Ok(()) => {}
                Err(TrySendError::Full(_)) => {
                    trace!("replay wake up signal channel is full.")
                }
                Err(TrySendError::Disconnected(_)) => {
                    trace!("replay wake up signal channel is disconnected.")
                }
            }
        }
    }

    /* Hand the poh tile the deadline BlockCreationLoop is going to hold
       the next block to, so pack stops against BCL's clock instead of
       its own.  Call immediately before set_bank, under the same write
       lock, or the deadline and the bank it belongs to can be separated
       by another writer. */
    pub fn set_alpenglow_block_deadline_ns(&mut self, deadline_ns: u64) {
        self.alpenglow_block_deadline_ns = deadline_ns;
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

        /* Banks created during the alpenglow migration are vote only, and
           a replaying peer rejects the whole block if it carries anything
           else (BlockstoreProcessorError::UserTransactionsInVoteOnlyBank).
           Agave's banking stage checks the bank itself; pack cannot, so it
           has to be told not to schedule non vote transactions. */
        let vote_only = bank.vote_only_bank() as i32;

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

        /* One shot: whoever set it meant it for this bank.  Clearing it
           here keeps a set_bank from the TowerBFT path (banking_stage)
           from inheriting a deadline BCL left behind. */
        let block_deadline_nanos = std::mem::replace(&mut self.alpenglow_block_deadline_ns, u64::MAX);

        let leader_bank: *const Bank = Arc::into_raw( bank );
        unsafe { fd_ext_poh_begin_leader( leader_bank as *const c_void, slot, epoch, hashes_per_tick, tick_duration_nanos, cus_block_limit, cus_vote_cost_limit, cus_account_cost_limit, cus_allocated_data_size_limit, max_data_shreds, vote_only, block_deadline_nanos ) };
    }

    pub fn reset(&mut self, reset_bank: Arc<Bank>, next_leader_slot: Option<(Slot, Slot)>) {
        /* Must be implemented, used by replay stage. */
        self.start_bank_id = reset_bank.bank_id();
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

        /* Whether the cluster is running alpenglow, which selects the
           shape of the blocks the poh tile produces.  A bank is
           alpenglow once it carries the genesis certificate: set on the
           genesis bank when the certificate is already in the accounts
           (alpenglow activated at genesis), or on the first alpenglow
           bank when the marker is processed (migration), and inherited
           by every descendant.  Deliberately not derived from the
           feature activation slot, which lands on an epoch boundary and
           is not where alpenglow actually starts.

           Once enable_alpenglow has been called the tile stays in
           alpenglow mode regardless.  On the migration path the banks
           around the transition are still TowerBFT banks -- the alpenglow
           genesis block is the last of them -- and BlockCreationLoop
           resets onto one of those immediately after enabling, and again
           before every leader slot whose parent is not where poh sits.
           Taking the bank alone would switch the tile straight back. */
        let alpenglow = (reset_bank.is_alpenglow() || self.is_alpenglow_enabled) as i32;

        unsafe { fd_ext_poh_reset( reset_bank_slot, reset_bank_blockhash.as_ref().as_ptr(),
                  hashes_per_tick, tick_duration_nanos, block_id_ptr, features_activation_slot.as_ref().as_ptr(),
                  shred_slot_limits.as_ref().as_ptr(), alpenglow ) };
    }

    pub fn track_transaction_indexes(&mut self) {
        /* No-op - handled internally */
    }

    pub fn enable_alpenglow(&mut self) {
        /* Upstream clears the tick cache and drops poh into low power mode
           here, which is how it stops producing ticks.  The equivalent is
           switching the poh tile, which owns the hash chain and the entry
           stream.

           The tile cannot work this out for itself.  It learns alpenglow
           from the flag on fd_ext_poh_reset, which is driven by the reset
           bank, and on the migration path every bank up to and including
           the alpenglow genesis block is still a TowerBFT bank -- a slot is
           only an alpenglow block once it is strictly after the genesis
           certificate's block.  So the first block the tile is asked to
           produce under alpenglow would be shaped as a TowerBFT one.

           Called both here and from BlockCreationLoop, and idempotent. */
        self.is_alpenglow_enabled = true;
        unsafe { fd_ext_poh_alpenglow_enable() };
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

    /* Ends an alpenglow block.  Mirrors the sequence upstream performs in
       flush_cache: register the tick, wait for the bank to freeze, send
       the footer carrying that bank's own hash, then send the tick.

       The split across the FFI exists because neither side can do it
       alone.  Only the poh tile knows the hash chain, so it computes the
       tick; only this side can build and serialize the footer, and it
       cannot do so until registering the tick has driven tick_height to
       max_tick_height and replay has frozen the bank.

       Blocking here is fine and is what upstream does.  This runs on the
       BlockCreationLoop thread, not on the tile, and the poh lock is not
       held across the wait, so the tile keeps running throughout. */
    pub fn tick_alpenglow(
        &mut self,
        max_tick_height: u64,
        mut footer: BlockFooterV1,
    ) -> old_poh_recorder::Result<()> {
        let Some(bank) = self.bank() else {
            return Err(PohRecorderError::MaxHeightReached);
        };

        /* Ask the poh tile to end the block, then wait for the tick hash.
           It only appears once every microblock pack sent for this slot has
           been mixed in: registering the tick freezes the bank, and a
           microblock still in flight would then be committed into a frozen
           bank, which panics in Bank::commit_transactions.  Draining is the
           tile's own work, so it is polled from here rather than blocking
           the tile.  Blocking is fine on this thread, and bounded by the
           same slot duration used for the freeze wait below. */
        let mut tick_hash = [0u8; 32];
        unsafe { fd_ext_poh_alpenglow_begin_tick() };

        let drain_start = Instant::now();
        let delta_block = Duration::from_nanos( bank.ns_per_slot as u64 );
        while 0 == unsafe { fd_ext_poh_alpenglow_try_get_tick( tick_hash.as_mut_ptr() ) } {
            if self.is_exited.load(Ordering::Relaxed) {
                return Err(PohRecorderError::ChannelDisconnected);
            }
            if drain_start.elapsed() > delta_block {
                error!(
                    "slot = {} block production failure. timed out draining in flight \
                     microblocks before the tick.",
                    bank.slot()
                );
                return Err(PohRecorderError::BankFreezeTimeout(bank.slot()));
            }
            std::hint::spin_loop();
        }
        let tick_hash = Hash::new_from_array( tick_hash );

        /* Drives tick_height to max_tick_height, which is what makes
           replay freeze the bank.  BlockCreationLoop has already set the
           height to max_tick_height-1 for exactly this reason. */
        bank.register_tick( &tick_hash, &BankWithScheduler::no_scheduler_available() );

        /* Registering the tick only drives tick_height to max_tick_height;
           replay is what freezes the bank, and it has to be told to look.
           Without this the wait below lands on replay's own schedule --
           measured 0.4 to 100.4ms, median 56.4, which was 99% of the whole
           block completion tail and the single largest cost in the leader
           path.  Upstream does the same thing in the same place
           (old_poh_recorder.rs wait_for_freeze_and_send_footer). */
        self.notify_replay_wakeup();

        let start = Instant::now();
        while !bank.is_frozen() && !self.is_exited.load(Ordering::Relaxed) {
            if start.elapsed() > delta_block {
                break;
            }
            std::hint::spin_loop();
        }
        if !bank.is_frozen() {
            if self.is_exited.load(Ordering::Relaxed) {
                return Err(PohRecorderError::ChannelDisconnected);
            }
            error!(
                "slot = {} block production failure. bank freezing timed out.",
                bank.slot()
            );
            return Err(PohRecorderError::BankFreezeTimeout(bank.slot()));
        }

        footer.bank_hash = bank.hash();
        debug_assert_eq!( max_tick_height, bank.max_tick_height() );

        /* The poh tile publishes these bytes verbatim, so serialize the
           whole BlockComponent: that is what carries the zero entry count
           marking the batch as a marker rather than a run of entries.
           wincode, not bincode - see entry/src/block_component.rs. */
        let component = BlockComponent::new_block_marker(
            VersionedBlockMarker::from_block_footer( footer ) );
        let bytes = wincode::serialize( &component )
            .map_err(|_| PohRecorderError::MaxHeightReached)?;

        if unsafe { fd_ext_poh_alpenglow_publish_footer( bytes.as_ptr(), bytes.len() as u64 ) } != 0 {
            error!(
                "slot = {} block production failure. footer of {} bytes rejected by the poh tile.",
                bank.slot(),
                bytes.len()
            );
            return Err(PohRecorderError::MaxHeightReached);
        }

        Ok(())
    }

    /* Abandons the block in progress.  BlockCreationLoop calls this when a
       window is aborted, for example once the cluster has moved on to a
       later parent.  set_shared_state is ignored: the leader state this
       recorder exposes is derived from the tile, which is cleared below. */
    pub fn clear_bank(&mut self, _set_shared_state: bool) {
        unsafe { fd_ext_poh_alpenglow_clear_bank() };
    }

    /* Identity of the bank the current poh state is based on.  Under
       alpenglow BlockCreationLoop calls this before every leader slot,
       so it cannot stay unimplemented. */
    pub fn start_bank_id(&self) -> BankId {
        self.start_bank_id
    }

    /* Queues a block marker for the poh tile to publish.  Every alpenglow
       block opens with a block header, and a sad handover adds an update
       parent marker.  The footer does not come through here; it is sent by
       tick_alpenglow, which has to interleave it with the tick. */
    pub fn send_marker(&mut self, marker: VersionedBlockMarker) -> Result<()> {
        let component = BlockComponent::new_block_marker( marker );
        let bytes = wincode::serialize( &component )
            .map_err(|_| PohRecorderError::MaxHeightReached)?;

        if unsafe { fd_ext_poh_alpenglow_publish_marker( bytes.as_ptr(), bytes.len() as u64 ) } != 0 {
            error!( "failed to queue a {} byte alpenglow block marker", bytes.len() );
            return Err(PohRecorderError::MaxHeightReached);
        }
        Ok(())
    }
}
