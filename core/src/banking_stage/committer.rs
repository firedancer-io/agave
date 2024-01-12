use {
    super::leader_slot_timing_metrics::LeaderExecuteAndCommitTimings,
    itertools::Itertools,
    solana_accounts_db::{
        accounts::TransactionLoadResult,
        transaction_results::{TransactionExecutionResult, TransactionResults},
    },
    solana_ledger::{
        blockstore_processor::TransactionStatusSender, token_balances::collect_token_balances,
    },
    solana_measure::measure_us,
    solana_runtime::{
        bank::{Bank, CommitTransactionCounts, TransactionBalancesSet},
        bank_utils,
        prioritization_fee_cache::PrioritizationFeeCache,
        transaction_batch::TransactionBatch,
    },
    solana_sdk::{hash::Hash, pubkey::Pubkey, saturating_add_assign},
    solana_transaction_status::{
        token_balances::TransactionTokenBalancesSet, TransactionTokenBalance,
    },
    solana_vote::vote_sender_types::ReplayVoteSender,
    std::{collections::HashMap, sync::Arc},
};

pub(crate) static FIREDANCER_COMMITTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

#[no_mangle]
pub extern "C" fn fd_ext_bank_pre_balance_info( bank: *const std::ffi::c_void, txns: *const std::ffi::c_void, txn_count: u64 ) -> *mut std::ffi::c_void {
    use solana_sdk::transaction::SanitizedTransaction;
    use std::borrow::Cow;
    use std::sync::atomic::Ordering;

    let txns = unsafe {
        std::slice::from_raw_parts(txns as *const SanitizedTransaction, txn_count as usize)
    };
    let bank = bank as *const Bank;
    unsafe { Arc::increment_strong_count(bank) };
    let bank = unsafe { Arc::from_raw( bank as *const Bank ) };

    let lock_results = txns.iter().map(|_| Ok(()) ).collect::<Vec<_>>();
    let mut batch = TransactionBatch::new(lock_results, bank.as_ref(), Cow::Borrowed(txns));
    batch.set_needs_unlock(false); /* Accounts not actually locked. */

    loop {
        if FIREDANCER_COMMITTER.load(Ordering::Relaxed) != 0 {
            break;
        }
        std::hint::spin_loop();
    }
    let committer: &Committer = unsafe { (FIREDANCER_COMMITTER.load(Ordering::Acquire) as *const Committer).as_ref().unwrap() };

    let mut pre_balance_info = Box::new(PreBalanceInfo::default());
    if committer.transaction_status_sender_enabled() {
        pre_balance_info.native = bank.collect_balances(&batch);
        pre_balance_info.token =
            collect_token_balances(&bank, &batch, &mut pre_balance_info.mint_decimals)
    }
    Box::into_raw(pre_balance_info) as *mut std::ffi::c_void
}

#[no_mangle]
pub extern "C" fn fd_ext_bank_release_pre_balance_info( pre_balance_info: *mut std::ffi::c_void ) {
    let pre_balance_info = unsafe { Box::from_raw( pre_balance_info as *mut PreBalanceInfo ) };
    drop(pre_balance_info);
}

#[no_mangle]
pub extern "C" fn fd_ext_bank_commit_txns( bank: *const std::ffi::c_void, txns: *const std::ffi::c_void, txn_count: u64, load_and_execute_output: *mut std::ffi::c_void, pre_balance_info: *mut std::ffi::c_void ) {
    use solana_sdk::transaction::SanitizedTransaction;
    use solana_runtime::bank::LoadAndExecuteTransactionsOutput;
    use std::borrow::Cow;
    use std::sync::atomic::Ordering;

    let txns = unsafe {
        std::slice::from_raw_parts(txns as *const SanitizedTransaction, txn_count as usize)
    };
    let bank = bank as *const Bank;
    unsafe { Arc::increment_strong_count(bank) };
    let bank = unsafe { Arc::from_raw( bank as *const Bank ) };

    let mut load_and_execute_output: Box<LoadAndExecuteTransactionsOutput> = unsafe { Box::from_raw( load_and_execute_output as *mut LoadAndExecuteTransactionsOutput ) };

    let lock_results = txns.iter().map(|_| Ok(()) ).collect::<Vec<_>>();
    let mut batch = TransactionBatch::new(lock_results, bank.as_ref(), Cow::Borrowed(txns));
    batch.set_needs_unlock(false); /* Accounts not actually locked. */
    let (last_blockhash, lamports_per_signature) = bank.last_blockhash_and_lamports_per_signature();

    loop {
        // This isn't strictly necessary because the banking stage has to have
        // retrieved a committer in order for this function to be called (to get
        // pre_balance_info), but keep it because we plan on removing
        // pre_balance_info.
        if FIREDANCER_COMMITTER.load(Ordering::Relaxed) != 0 {
            break;
        }
        std::hint::spin_loop();
    }
    let committer: &Committer = unsafe { (FIREDANCER_COMMITTER.load(Ordering::Acquire) as *const Committer).as_ref().unwrap() };
    let mut timings = LeaderExecuteAndCommitTimings::default();
    let mut pre_balance_info = unsafe { Box::from_raw( pre_balance_info as *mut PreBalanceInfo ) };
    let _ = committer.commit_transactions(
        &batch,
        &mut load_and_execute_output.loaded_transactions,
        load_and_execute_output.execution_results,
        last_blockhash,
        lamports_per_signature,
        None,
        &bank,
        &mut *pre_balance_info,
        &mut timings,
        load_and_execute_output.signature_count,
        load_and_execute_output.executed_transactions_count,
        load_and_execute_output.executed_non_vote_transactions_count,
        load_and_execute_output.executed_with_successful_result_count);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitTransactionDetails {
    Committed { compute_units: u64 },
    NotCommitted,
}

#[derive(Default)]
pub(super) struct PreBalanceInfo {
    pub native: Vec<Vec<u64>>,
    pub token: Vec<Vec<TransactionTokenBalance>>,
    pub mint_decimals: HashMap<Pubkey, u8>,
}

#[derive(Clone)]
pub struct Committer {
    transaction_status_sender: Option<TransactionStatusSender>,
    replay_vote_sender: ReplayVoteSender,
    prioritization_fee_cache: Arc<PrioritizationFeeCache>,
}

impl Committer {
    pub fn new(
        transaction_status_sender: Option<TransactionStatusSender>,
        replay_vote_sender: ReplayVoteSender,
        prioritization_fee_cache: Arc<PrioritizationFeeCache>,
    ) -> Self {
        Self {
            transaction_status_sender,
            replay_vote_sender,
            prioritization_fee_cache,
        }
    }

    pub(super) fn transaction_status_sender_enabled(&self) -> bool {
        self.transaction_status_sender.is_some()
    }

    #[allow(clippy::too_many_arguments)]
    pub(super) fn commit_transactions(
        &self,
        batch: &TransactionBatch,
        loaded_transactions: &mut [TransactionLoadResult],
        execution_results: Vec<TransactionExecutionResult>,
        last_blockhash: Hash,
        lamports_per_signature: u64,
        starting_transaction_index: Option<usize>,
        bank: &Arc<Bank>,
        pre_balance_info: &mut PreBalanceInfo,
        execute_and_commit_timings: &mut LeaderExecuteAndCommitTimings,
        signature_count: u64,
        executed_transactions_count: usize,
        executed_non_vote_transactions_count: usize,
        executed_with_successful_result_count: usize,
    ) -> (u64, Vec<CommitTransactionDetails>) {
        let executed_transactions = execution_results
            .iter()
            .zip(batch.sanitized_transactions())
            .filter_map(|(execution_result, tx)| execution_result.was_executed().then_some(tx))
            .collect_vec();

        let (tx_results, commit_time_us) = measure_us!(bank.commit_transactions(
            batch.sanitized_transactions(),
            loaded_transactions,
            execution_results,
            last_blockhash,
            lamports_per_signature,
            CommitTransactionCounts {
                committed_transactions_count: executed_transactions_count as u64,
                committed_non_vote_transactions_count: executed_non_vote_transactions_count as u64,
                committed_with_failure_result_count: executed_transactions_count
                    .saturating_sub(executed_with_successful_result_count)
                    as u64,
                signature_count,
            },
            &mut execute_and_commit_timings.execute_timings,
        ));
        execute_and_commit_timings.commit_us = commit_time_us;

        let commit_transaction_statuses = tx_results
            .execution_results
            .iter()
            .map(|execution_result| match execution_result.details() {
                Some(details) => CommitTransactionDetails::Committed {
                    compute_units: details.executed_units,
                },
                None => CommitTransactionDetails::NotCommitted,
            })
            .collect();

        let ((), find_and_send_votes_us) = measure_us!({
            bank_utils::find_and_send_votes(
                batch.sanitized_transactions(),
                &tx_results,
                Some(&self.replay_vote_sender),
            );
            self.collect_balances_and_send_status_batch(
                tx_results,
                bank,
                batch,
                pre_balance_info,
                starting_transaction_index,
            );
            self.prioritization_fee_cache
                .update(bank, executed_transactions.into_iter());
        });
        execute_and_commit_timings.find_and_send_votes_us = find_and_send_votes_us;
        (commit_time_us, commit_transaction_statuses)
    }

    fn collect_balances_and_send_status_batch(
        &self,
        tx_results: TransactionResults,
        bank: &Arc<Bank>,
        batch: &TransactionBatch,
        pre_balance_info: &mut PreBalanceInfo,
        starting_transaction_index: Option<usize>,
    ) {
        if let Some(transaction_status_sender) = &self.transaction_status_sender {
            let txs = batch.sanitized_transactions().to_vec();
            let post_balances = bank.collect_balances(batch);
            let post_token_balances =
                collect_token_balances(bank, batch, &mut pre_balance_info.mint_decimals);
            let mut transaction_index = starting_transaction_index.unwrap_or_default();
            let batch_transaction_indexes: Vec<_> = tx_results
                .execution_results
                .iter()
                .map(|result| {
                    if result.was_executed() {
                        let this_transaction_index = transaction_index;
                        saturating_add_assign!(transaction_index, 1);
                        this_transaction_index
                    } else {
                        0
                    }
                })
                .collect();
            transaction_status_sender.send_transaction_status_batch(
                bank.clone(),
                txs,
                tx_results.execution_results,
                TransactionBalancesSet::new(
                    std::mem::take(&mut pre_balance_info.native),
                    post_balances,
                ),
                TransactionTokenBalancesSet::new(
                    std::mem::take(&mut pre_balance_info.token),
                    post_token_balances,
                ),
                tx_results.rent_debits,
                batch_transaction_indexes,
            );
        }
    }
}
