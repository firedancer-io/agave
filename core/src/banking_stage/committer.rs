use {
    super::leader_slot_timing_metrics::LeaderExecuteAndCommitTimings,
    itertools::Itertools,
    solana_ledger::{
        blockstore_processor::TransactionStatusSender, token_balances::collect_token_balances,
    },
    solana_measure::measure_us,
    solana_runtime::{
        bank::{Bank, ProcessedTransactionCounts, TransactionBalancesSet},
        bank_utils,
        prioritization_fee_cache::PrioritizationFeeCache,
        transaction_batch::TransactionBatch,
        vote_sender_types::ReplayVoteSender,
    },
    solana_runtime_transaction::transaction_with_meta::TransactionWithMeta,
    solana_sdk::{pubkey::Pubkey, saturating_add_assign},
    solana_svm::{
        transaction_commit_result::{TransactionCommitResult, TransactionCommitResultExtensions},
        transaction_processing_result::{
            TransactionProcessingResult, TransactionProcessingResultExtensions
        },
    },
    solana_transaction_status::{
        token_balances::TransactionTokenBalancesSet, TransactionTokenBalance,
    },
    std::{collections::HashMap, sync::Arc, time::Duration},
};

pub(crate) static FIREDANCER_COMMITTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);
pub(crate) static FIREDANCER_BUNDLE_COMMITTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

use solana_sdk::transaction::TransactionError;
fn transaction_error_to_code(err: &TransactionError) -> i32 {
    match err {
        TransactionError::AccountInUse => 1,
        TransactionError::AccountLoadedTwice => 2,
        TransactionError::AccountNotFound => 3,
        TransactionError::ProgramAccountNotFound => 4,
        TransactionError::InsufficientFundsForFee => 5,
        TransactionError::InvalidAccountForFee => 6,
        TransactionError::AlreadyProcessed => 7,
        TransactionError::BlockhashNotFound => 8,
        TransactionError::InstructionError(_, _) => 9,
        TransactionError::CallChainTooDeep => 10,
        TransactionError::MissingSignatureForFee => 11,
        TransactionError::InvalidAccountIndex => 12,
        TransactionError::SignatureFailure => 13,
        TransactionError::InvalidProgramForExecution => 14,
        TransactionError::SanitizeFailure => 15,
        TransactionError::ClusterMaintenance => 16,
        TransactionError::AccountBorrowOutstanding => 17,
        TransactionError::WouldExceedMaxBlockCostLimit => 18,
        TransactionError::UnsupportedVersion => 19,
        TransactionError::InvalidWritableAccount => 20,
        TransactionError::WouldExceedMaxAccountCostLimit => 21,
        TransactionError::WouldExceedAccountDataBlockLimit => 22,
        TransactionError::TooManyAccountLocks => 23,
        TransactionError::AddressLookupTableNotFound => 24,
        TransactionError::InvalidAddressLookupTableOwner => 25,
        TransactionError::InvalidAddressLookupTableData => 26,
        TransactionError::InvalidAddressLookupTableIndex => 27,
        TransactionError::InvalidRentPayingAccount => 28,
        TransactionError::WouldExceedMaxVoteCostLimit => 29,
        TransactionError::WouldExceedAccountDataTotalLimit => 30,
        TransactionError::DuplicateInstruction(_) => 31,
        TransactionError::InsufficientFundsForRent { .. } => 32,
        TransactionError::MaxLoadedAccountsDataSizeExceeded => 33,
        TransactionError::InvalidLoadedAccountsDataSizeLimit => 34,
        TransactionError::ResanitizationNeeded => 35,
        TransactionError::ProgramExecutionTemporarilyRestricted { .. } => 36,
        TransactionError::UnbalancedTransaction => 37,
        TransactionError::ProgramCacheHitMaxLimit => 38,
        TransactionError::CommitCancelled => 39,
    }
}

#[no_mangle]
pub extern "C" fn fd_ext_bank_execute_and_commit_bundle(bank: *const std::ffi::c_void, txns: *const std::ffi::c_void, txn_count: u64, out_transaction_err: *mut i32, out_consumed_exec_cus: *mut u32, out_consumed_acct_data_cus: *mut u32, out_ts_load_end: *mut u64, out_ts_exec_end: *mut u64, out_tips: *mut u64) -> i32 {
    use solana_sdk::clock::MAX_PROCESSING_AGE;
    use std::sync::atomic::Ordering;
    use solana_bundle::bundle_execution::load_and_execute_bundle;
    use crate::bundle_stage::committer::Committer;
    use solana_runtime::bank::{MAINNET_TIP_ACCOUNTS, TESTNET_TIP_ACCOUNTS, EMPTY_TIP_ACCOUNTS};
    use solana_sdk::genesis_config::ClusterType;
    use solana_sdk::transaction::SanitizedTransaction;
    use solana_cost_model::cost_model::CostModel;
    use solana_runtime_transaction::runtime_transaction::RuntimeTransaction;
    use solana_svm::transaction_processing_result::{ProcessedTransaction, ProcessedTransaction::Executed};
    let _ = Duration::abs_diff;

    let txns = unsafe {
        std::slice::from_raw_parts(txns as *const RuntimeTransaction<SanitizedTransaction>, txn_count as usize)
    };
    let out_ts_load_end = unsafe {
        std::slice::from_raw_parts_mut(out_ts_load_end, txn_count as usize)
    };
    let out_ts_exec_end = unsafe {
        std::slice::from_raw_parts_mut(out_ts_exec_end, txn_count as usize)
    };

    let bank = bank as *const Bank;
    unsafe { Arc::increment_strong_count(bank) };
    let bank = unsafe { Arc::from_raw(bank as *const Bank) };

    loop {
        if FIREDANCER_BUNDLE_COMMITTER.load(Ordering::Relaxed) != 0 {
            break;
        }
        std::hint::spin_loop();
    }
    let committer: &Committer = unsafe {
        (FIREDANCER_BUNDLE_COMMITTER.load(Ordering::Acquire) as *const Committer)
            .as_ref()
            .unwrap()
    };

    let transaction_status_sender_enabled = committer.transaction_status_sender_enabled();

    let tip_accounts = if bank.cluster_type() == ClusterType::MainnetBeta {
        &*MAINNET_TIP_ACCOUNTS
    } else if bank.cluster_type() == ClusterType::Testnet {
        &*TESTNET_TIP_ACCOUNTS
    } else {
        &*EMPTY_TIP_ACCOUNTS
    };

    let default_accounts = vec![None; txns.len()];
    let mut bundle_execution_results = load_and_execute_bundle(
        &bank,
        txns,
        MAX_PROCESSING_AGE,
        transaction_status_sender_enabled,
        &None,
        None,
        &default_accounts,
        &default_accounts,
        Some(tip_accounts),
        out_ts_load_end,
        out_ts_exec_end,
    );

    for (i, result) in bundle_execution_results
        .bundle_transaction_results()
        .iter()
        .map(|x| &x.load_and_execute_transactions_output().processing_results)
        .flatten()
        .enumerate()
    {
        // FIREDANCER: If any transaction in the bundle failed, we want to keep tips + CUs up to the point of failure
        // for use by monitoring tools.
        if let Err(err) = bundle_execution_results.result() {
            match err {
                solana_bundle::bundle_execution::LoadAndExecuteBundleError::TransactionError { index, execution_result, .. } => {
                    if *index == i {
                        for j in 0..txn_count {
                            unsafe { *out_transaction_err.offset(j as isize) = transaction_error_to_code(&TransactionError::CommitCancelled) };
                        }

                        for j in (*index)..(txn_count as usize) {
                            unsafe { *out_consumed_exec_cus.offset(j as isize) = 0 };
                            unsafe { *out_consumed_acct_data_cus.offset(j as isize) = 0 };
                            unsafe { *out_tips.offset(j as isize) = 0u64 };
                        }

                        let err = match &**execution_result {
                            Ok(ProcessedTransaction::Executed(tx)) => tx.execution_details.status.as_ref().unwrap_err().clone(),
                            Ok(ProcessedTransaction::FeesOnly(tx)) => tx.load_error.clone(),
                            Err(err) => err.clone(),
                        };
        
                        unsafe { *out_transaction_err.offset(*index as isize) = transaction_error_to_code(&err) };

                        return 0;
                    }
                }
            }
        }

        let (consumed_cus, loaded_accounts_data_cost, tips) =
            match &result {
                Ok(Executed(tx)) => {
                    (
                        tx.execution_details.executed_units.try_into().unwrap(),
                        CostModel::calculate_loaded_accounts_data_size_cost(
                            tx.loaded_transaction.loaded_accounts_data_size,
                            &bank.feature_set,
                        ) as u32,
                        // jito collects a 5% fee at the end of the epoch
                        tx.execution_details.tips - tx.execution_details.tips
                            .checked_mul(5)
                            .unwrap()
                            .checked_div(100)
                            .unwrap(),
                    )
                },
                // Transactions must've have succeeded, and not be fee-only,
                // otherwise the entire bundle would have failed.
                _ => unreachable!(),
            };

        unsafe { *out_transaction_err.offset(i.try_into().unwrap()) = 0 };
        unsafe { *out_consumed_exec_cus.offset(i.try_into().unwrap()) = consumed_cus };
        unsafe { *out_consumed_acct_data_cus.offset(i.try_into().unwrap()) = loaded_accounts_data_cost };
        unsafe { *out_tips.offset(i.try_into().unwrap()) = tips };
    }

    let mut execute_and_commit_timings = LeaderExecuteAndCommitTimings::default();
    let (_commit_us, _commit_bundle_details) = committer.commit_bundle(
        &mut bundle_execution_results,
        Some(0),
        &bank,
        &mut execute_and_commit_timings,
    );

    1
}

#[no_mangle]
pub extern "C" fn fd_ext_bank_load_and_execute_txns( bank: *const std::ffi::c_void, txns: *const std::ffi::c_void, txn_count: u64, out_processing_result: *mut i32, out_transaction_err: *mut i32, out_consumed_exec_cus: *mut u32, out_consumed_acct_data_cus: *mut u32, out_ts_load_end: *mut u64, out_ts_exec_end: *mut u64 ) -> *mut std::ffi::c_void {
    use solana_timings::{ExecuteTimingType, ExecuteTimings};
    use solana_runtime::bank::LoadAndExecuteTransactionsOutput;
    use solana_runtime::transaction_batch::OwnedOrBorrowed;
    use solana_runtime_transaction::runtime_transaction::RuntimeTransaction;
    use solana_sdk::clock::MAX_PROCESSING_AGE;
    use solana_sdk::transaction::SanitizedTransaction;
    use solana_svm::transaction_error_metrics::TransactionErrorMetrics;
    use solana_svm::transaction_processing_result::ProcessedTransaction::{Executed,FeesOnly};
    use solana_svm::transaction_processor::ExecutionRecordingConfig;
    use solana_svm::transaction_processor::TransactionProcessingConfig;
    use solana_cost_model::cost_model::CostModel;
    use std::{sync::atomic::Ordering, ops::Index, time::{SystemTime, UNIX_EPOCH}};

    const FD_BANK_TRANSACTION_LANDED: i32 = 1;
    const FD_BANK_TRANSACTION_EXECUTED: i32 = 2;

    let txns = unsafe {
        std::slice::from_raw_parts(txns as *const RuntimeTransaction<SanitizedTransaction>, txn_count as usize)
    };
    let out_ts_load_end = unsafe {
        std::slice::from_raw_parts_mut(out_ts_load_end, txn_count as usize)
    };
    let out_ts_exec_end = unsafe {
        std::slice::from_raw_parts_mut(out_ts_exec_end, txn_count as usize)
    };

    let bank = bank as *const Bank;
    unsafe { Arc::increment_strong_count(bank) };
    let bank = unsafe { Arc::from_raw( bank as *const Bank ) };

    loop {
        if FIREDANCER_COMMITTER.load(Ordering::Relaxed) != 0 {
            break;
        }
        std::hint::spin_loop();
    }
    let committer: &Committer = unsafe { (FIREDANCER_COMMITTER.load(Ordering::Acquire) as *const Committer).as_ref().unwrap() };

    let lock_results = txns.iter().map(|_| Ok(()) ).collect::<Vec<_>>();
    let mut batch = TransactionBatch::new(lock_results, bank.as_ref(), OwnedOrBorrowed::Borrowed(txns));
    batch.set_needs_unlock(false);

    use solana_runtime::bank::{MAINNET_TIP_ACCOUNTS, TESTNET_TIP_ACCOUNTS, EMPTY_TIP_ACCOUNTS};
    use solana_sdk::genesis_config::ClusterType;
    let tip_accounts = if bank.cluster_type() == ClusterType::MainnetBeta {
        &*MAINNET_TIP_ACCOUNTS
    } else if bank.cluster_type() == ClusterType::Testnet {
        &*TESTNET_TIP_ACCOUNTS
    } else {
        &*EMPTY_TIP_ACCOUNTS
    };

    let mut timings = ExecuteTimings::default();
    let transaction_status_sender_enabled = committer.transaction_status_sender_enabled();

    let start_time_ns = SystemTime::now().duration_since(UNIX_EPOCH).unwrap().as_nanos() as u64;

    let output = bank.load_and_execute_transactions(&batch, MAX_PROCESSING_AGE, &mut timings,
        &mut TransactionErrorMetrics::default(),
        TransactionProcessingConfig {
            account_overrides: None,
            check_program_modification_slot: bank.check_program_modification_slot(),
            compute_budget: bank.compute_budget(),
            log_messages_bytes_limit: None,
            limit_to_load_programs: false,
            recording_config: ExecutionRecordingConfig::new_single_setting(transaction_status_sender_enabled),
            transaction_account_lock_limit: Some(64),
            tip_accounts: Some(tip_accounts),
        }
    );

    // FIREDANCER: copy accumulated timings to timestamps, used by monitoring tools in FD client
    for i in 0..(txn_count as usize) {
        out_ts_load_end[i] = start_time_ns
                       + (timings.metrics.index(ExecuteTimingType::CheckUs).0 * 1000u64)
                       + (timings.metrics.index(ExecuteTimingType::ValidateFeesUs).0 * 1000u64)
                       + (timings.metrics.index(ExecuteTimingType::FilterExecutableUs).0 * 1000u64)
                       + (timings.metrics.index(ExecuteTimingType::ProgramCacheUs).0 * 1000u64)
                       + (timings.metrics.index(ExecuteTimingType::LoadUs).0 * 1000u64);
        out_ts_exec_end[i] = out_ts_load_end[i] + (timings.metrics.index(ExecuteTimingType::ExecuteUs).0 * 1000u64);
    }

    for i in 0..txn_count {
        let (processing_result, consumed_cus, loaded_accounts_data_cost, transaction_err) =
            match &output.processing_results[i as usize] {
                Err(err) => (0, 0u32, 0u32, transaction_error_to_code(&err)),
                Ok(Executed(tx)) => {
                    (
                        FD_BANK_TRANSACTION_LANDED | FD_BANK_TRANSACTION_EXECUTED,
                        /* Executed CUs must be less than the block CU limit, which is much less
                           than UINT_MAX, so the cast should be safe */
                        tx.execution_details.executed_units.try_into().unwrap(),
                        CostModel::calculate_loaded_accounts_data_size_cost(
                            tx.loaded_transaction.loaded_accounts_data_size,
                            &bank.feature_set,
                        ) as u32,
                        match &tx.execution_details.status {
                            Ok(_) => 0,
                            Err(err) => transaction_error_to_code( &err )
                        }
                    )
                },
                Ok(FeesOnly(tx)) =>  (
                    FD_BANK_TRANSACTION_LANDED,
                    0u32,
                    CostModel::calculate_loaded_accounts_data_size_cost(
                        tx.rollback_accounts.data_size() as u32,
                        &bank.feature_set,
                    ) as u32,
                    transaction_error_to_code( &tx.load_error )
                )
            };
        unsafe { *out_processing_result.offset(i as isize) = processing_result };
        unsafe { *out_transaction_err.offset(i as isize) = transaction_err };
        unsafe { *out_consumed_exec_cus.offset(i as isize) = consumed_cus };
        unsafe { *out_consumed_acct_data_cus.offset(i as isize) = loaded_accounts_data_cost };
    }

    let load_and_execute_output: Box<LoadAndExecuteTransactionsOutput> = Box::new(output);
    Box::into_raw(load_and_execute_output) as *mut std::ffi::c_void
}

#[no_mangle]
pub extern "C" fn fd_ext_bank_pre_balance_info( bank: *const std::ffi::c_void, txns: *const std::ffi::c_void, txn_count: u64 ) -> *mut std::ffi::c_void {
    use solana_runtime::transaction_batch::OwnedOrBorrowed;
    use solana_sdk::transaction::SanitizedTransaction;
    use solana_runtime_transaction::runtime_transaction::RuntimeTransaction;
    use std::sync::atomic::Ordering;

    let txns = unsafe {
        std::slice::from_raw_parts(txns as *const RuntimeTransaction<SanitizedTransaction>, txn_count as usize)
    };
    let bank = bank as *const Bank;
    unsafe { Arc::increment_strong_count(bank) };
    let bank = unsafe { Arc::from_raw( bank as *const Bank ) };

    let lock_results = txns.iter().map(|_| Ok(()) ).collect::<Vec<_>>();
    let mut batch = TransactionBatch::new(lock_results, bank.as_ref(), OwnedOrBorrowed::Borrowed(txns));
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
    use solana_runtime::transaction_batch::OwnedOrBorrowed;
    use solana_runtime_transaction::runtime_transaction::RuntimeTransaction;
    use std::sync::atomic::Ordering;

    let txns = unsafe {
        std::slice::from_raw_parts(txns as *const RuntimeTransaction<SanitizedTransaction>, txn_count as usize)
    };
    let bank = bank as *const Bank;
    unsafe { Arc::increment_strong_count(bank) };
    let bank = unsafe { Arc::from_raw( bank as *const Bank ) };

    let load_and_execute_output: Box<LoadAndExecuteTransactionsOutput> = unsafe { Box::from_raw( load_and_execute_output as *mut LoadAndExecuteTransactionsOutput ) };

    let lock_results = txns.iter().map(|_| Ok(()) ).collect::<Vec<_>>();
    let mut batch = TransactionBatch::new(lock_results, bank.as_ref(), OwnedOrBorrowed::Borrowed(txns));
    batch.set_needs_unlock(false); /* Accounts not actually locked. */

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
        load_and_execute_output.processing_results,
        None,
        &bank,
        &mut *pre_balance_info,
        &mut timings,
        &load_and_execute_output.processed_counts);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitTransactionDetails {
    Committed {
        compute_units: u64,
        loaded_accounts_data_size: u32,
    },
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

    pub(super) fn commit_transactions(
        &self,
        batch: &TransactionBatch<impl TransactionWithMeta>,
        processing_results: Vec<TransactionProcessingResult>,
        starting_transaction_index: Option<usize>,
        bank: &Arc<Bank>,
        pre_balance_info: &mut PreBalanceInfo,
        execute_and_commit_timings: &mut LeaderExecuteAndCommitTimings,
        processed_counts: &ProcessedTransactionCounts,
    ) -> (u64, Vec<CommitTransactionDetails>) {
        let processed_transactions = processing_results
            .iter()
            .zip(batch.sanitized_transactions())
            .filter_map(|(processing_result, tx)| processing_result.was_processed().then_some(tx))
            .collect_vec();

        let (commit_results, commit_time_us) = measure_us!(bank.commit_transactions(
            batch.sanitized_transactions(),
            processing_results,
            processed_counts,
            &mut execute_and_commit_timings.execute_timings,
        ));
        execute_and_commit_timings.commit_us = commit_time_us;

        let commit_transaction_statuses = commit_results
            .iter()
            .map(|commit_result| match commit_result {
                // reports actual execution CUs, and actual loaded accounts size for
                // transaction committed to block. qos_service uses these information to adjust
                // reserved block space.
                Ok(committed_tx) => CommitTransactionDetails::Committed {
                    compute_units: committed_tx.executed_units,
                    loaded_accounts_data_size: committed_tx
                        .loaded_account_stats
                        .loaded_accounts_data_size,
                },
                Err(_) => CommitTransactionDetails::NotCommitted,
            })
            .collect();

        let ((), find_and_send_votes_us) = measure_us!({
            bank_utils::find_and_send_votes(
                batch.sanitized_transactions(),
                &commit_results,
                Some(&self.replay_vote_sender),
            );
            self.collect_balances_and_send_status_batch(
                commit_results,
                bank,
                batch,
                pre_balance_info,
                starting_transaction_index,
            );
            self.prioritization_fee_cache
                .update(bank, processed_transactions.into_iter());
        });
        execute_and_commit_timings.find_and_send_votes_us = find_and_send_votes_us;
        (commit_time_us, commit_transaction_statuses)
    }

    fn collect_balances_and_send_status_batch(
        &self,
        commit_results: Vec<TransactionCommitResult>,
        bank: &Arc<Bank>,
        batch: &TransactionBatch<impl TransactionWithMeta>,
        pre_balance_info: &mut PreBalanceInfo,
        starting_transaction_index: Option<usize>,
    ) {
        if let Some(transaction_status_sender) = &self.transaction_status_sender {
            // Clone `SanitizedTransaction` out of `RuntimeTransaction`, this is
            // done to send over the status sender.
            let txs = batch
                .sanitized_transactions()
                .iter()
                .map(|tx| tx.as_sanitized_transaction().into_owned())
                .collect_vec();
            let post_balances = bank.collect_balances(batch);
            let post_token_balances =
                collect_token_balances(bank, batch, &mut pre_balance_info.mint_decimals);
            let mut transaction_index = starting_transaction_index.unwrap_or_default();
            let batch_transaction_indexes: Vec<_> = commit_results
                .iter()
                .map(|commit_result| {
                    if commit_result.was_committed() {
                        let this_transaction_index = transaction_index;
                        saturating_add_assign!(transaction_index, 1);
                        this_transaction_index
                    } else {
                        0
                    }
                })
                .collect();
            transaction_status_sender.send_transaction_status_batch(
                bank.slot(),
                txs,
                commit_results,
                TransactionBalancesSet::new(
                    std::mem::take(&mut pre_balance_info.native),
                    post_balances,
                ),
                TransactionTokenBalancesSet::new(
                    std::mem::take(&mut pre_balance_info.token),
                    post_token_balances,
                ),
                batch_transaction_indexes,
            );
        }
    }
}
