/// FIREDANCER: Repalce PohRecorder completely with one that goes out to
///             our implementation.
use crate::leader_bank_notifier::LeaderBankNotifier;
use solana_sdk::pubkey::Pubkey;
use solana_sdk::hash::Hash;
use solana_sdk::slot_history::Slot;
use solana_runtime::{installed_scheduler_pool::BankWithScheduler,bank::Bank};
use solana_ledger::blockstore::Blockstore;
use crossbeam_channel::{Sender, Receiver, TrySendError};

use solana_ledger::leader_schedule_cache::LeaderScheduleCache;
use solana_sdk::poh_config::PohConfig;
use solana_metrics::poh_timing_point::PohTimingSender;

use std::ffi::c_void;
use std::sync::Arc;
use std::sync::atomic::AtomicBool;

use crate::poh_recorder::{BankStart, TransactionRecorder, PohLeaderStatus, WorkingBankEntry, Record};

extern "C" {
    fn fd_ext_poh_initialize(tick_duration_nanos: u64, hashcnt_per_tick: u64, ticks_per_slot: u64, tick_height: u64, last_entry_hash: *const u8, signal_leader_change: *mut c_void);
    fn fd_ext_poh_acquire_leader_bank() -> *const c_void;
    fn fd_ext_poh_reset_slot() -> u64;
    fn fd_ext_poh_reached_leader_slot(out_leader_slot: *mut u64, out_reset_slot: *mut u64) -> i32;
    fn fd_ext_poh_begin_leader(bank: *const c_void, slot: u64, hashcnt_per_tick: u64);
    fn fd_ext_poh_reset(reset_bank_slot: u64, reset_blockhash: *const u8, hashcnt_per_tick: u64);
    fn fd_ext_poh_get_leader_after_n_slots(n: u64, out_pubkey: *mut u8) -> i32;
    fn fd_ext_poh_update_active_descendant(max_active_descendant: u64);
}

#[no_mangle]
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

#[no_mangle]
pub extern "C" fn fd_ext_poh_register_tick( bank: *const c_void, hash: *const u8 ) {
    let hash = unsafe { std::slice::from_raw_parts(hash, 32) };
    let hash = Hash::new(hash);
    unsafe { (*(bank as *const Bank)).register_tick(&hash, &BankWithScheduler::no_scheduler_available()) };
}

pub struct PohRecorder {}

impl PohRecorder {
    #[allow(clippy::too_many_arguments)]
    pub fn new_with_clear_signal(
        tick_height: u64,
        last_entry_hash: Hash,
        _start_bank: Arc<Bank>,
        _next_leader_slot: Option<(Slot, Slot)>,
        ticks_per_slot: u64,
        _delay_leader_block_for_pending_fork: bool,
        _blockstore: Arc<Blockstore>,
        clear_bank_signal: Option<Sender<bool>>,
        _leader_schedule_cache: &Arc<LeaderScheduleCache>,
        poh_config: &PohConfig,
        _poh_timing_point_sender: Option<PohTimingSender>,
        _is_exited: Arc<AtomicBool>,
    ) -> (Self, Receiver<WorkingBankEntry>, Receiver<Record>) {
        /* Just silence the unused warning for old_poh_recorder, without needing to modify the file. */
        let _silence_warnings = super::old_poh_recorder::create_test_recorder;

        let clear_bank_sender: *mut Sender<bool> = match clear_bank_signal {
            Some(sender) => Box::into_raw(Box::new(sender)),
            None => std::ptr::null_mut(),
        };

        unsafe { fd_ext_poh_initialize(poh_config.target_tick_duration.as_nanos().try_into().unwrap(), poh_config.hashes_per_tick.unwrap_or(1), ticks_per_slot, tick_height, last_entry_hash.as_ref().as_ptr(), clear_bank_sender as *mut c_void) };

        let dummy1 = crossbeam_channel::unbounded();
        let dummy2 = crossbeam_channel::unbounded();
        /* Forget so the receiver doesn't see the channel is disconnected. */
        std::mem::forget(dummy1.0);
        std::mem::forget(dummy2.0);
        (Self {}, dummy1.1, dummy2.1)
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

    pub fn bank_start(&self) -> Option<BankStart> {
        /* Unused, only called by old TPU. */
        None
    }

    pub fn new_recorder(&self) -> TransactionRecorder {
        /* Just needs a dummy value, the only caller will never use it. */
        let (sender, _) = crossbeam_channel::unbounded();
        TransactionRecorder::new(sender, Arc::new(AtomicBool::new(false)))
    }

    pub fn would_be_leader(&self, _within_next_n_ticks: u64) -> bool {
        /* The only caller asks if it's within the next ten minutes, so
            that it can forward gossiped votes to ourselves.  We can just
            always forward them. */
        true
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

    pub fn new_leader_bank_notifier(&self) -> Arc<LeaderBankNotifier> {
        /* Unsued, only called by old TPU. */
        Arc::default()
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

    pub fn set_bank(&mut self, bank_with_scheduler: BankWithScheduler, _track_transaction_indexes: bool) {
        /* Must be implemented, used by replay stage. */
        let bank = bank_with_scheduler.clone_without_scheduler();
        let slot = bank.slot();
        let hashes_per_tick = bank.hashes_per_tick().unwrap_or(1);
        let leader_bank: *const Bank = Arc::into_raw( bank );
        unsafe { fd_ext_poh_begin_leader( leader_bank as *const c_void, slot, hashes_per_tick ) };
    }

    pub fn reset(&mut self, reset_bank: Arc<Bank>, _next_leader_slot: Option<(Slot, Slot)>) {
        /* Must be implemented, used by replay stage. */
        let reset_bank_slot = reset_bank.slot();
        let reset_bank_blockhash = reset_bank.last_blockhash();
        let hashes_per_tick = reset_bank.hashes_per_tick().unwrap_or(1);
        unsafe { fd_ext_poh_reset( reset_bank_slot, reset_bank_blockhash.as_ref().as_ptr(), hashes_per_tick ) };
    }
}
