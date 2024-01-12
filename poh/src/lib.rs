#![cfg(feature = "agave-unstable-api")]
#![allow(clippy::arithmetic_side_effects)]
pub mod poh_controller;
// FIREDANCER: Replace PohRecorder completely with one that goes out to
// our implementation.
// pub mod poh_recorder;
mod firedancer_poh_recorder;
pub mod old_poh_recorder;

pub mod poh_recorder {
  pub use crate::firedancer_poh_recorder::*;
  // use everything except PohRecorder from old_poh_recorder
  pub use crate::old_poh_recorder::{WorkingBankEntryOrMarker, PohRecorderError, GRACE_TICKS_FACTOR, MAX_GRACE_SLOTS, PohLeaderStatus, Record, LeaderState, SharedLeaderState };
  pub use crate::transaction_recorder::{RecordTransactionsSummary, RecordTransactionsTimings, TransactionRecorder};
}

pub mod poh_service;
pub mod record_channels;
pub mod transaction_recorder;

#[macro_use]
extern crate solana_metrics;

#[cfg(test)]
#[macro_use]
extern crate assert_matches;
