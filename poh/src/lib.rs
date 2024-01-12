#![allow(clippy::arithmetic_side_effects)]
pub mod leader_bank_notifier;
// FIREDANCER: Replace PohRecorder completely with one that goes out to
// our implementation.
// pub mod poh_recorder;
mod firedancer_poh_recorder;
mod old_poh_recorder;

pub mod poh_recorder {
  pub use crate::firedancer_poh_recorder::*;
  // use everything except PohRecorder from old_poh_recorder
  pub use crate::old_poh_recorder::{WorkingBankEntry, BankStart, PohRecorderError, RecordTransactionsSummary, RecordTransactionsTimings, TransactionRecorder, GRACE_TICKS_FACTOR, MAX_GRACE_SLOTS, PohLeaderStatus, Record };
}

pub mod poh_service;

#[macro_use]
extern crate solana_metrics;

#[cfg(test)]
#[macro_use]
extern crate assert_matches;
