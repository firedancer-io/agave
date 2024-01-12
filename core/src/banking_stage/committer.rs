use {
    super::leader_slot_timing_metrics::LeaderExecuteAndCommitTimings,
    itertools::Itertools,
    solana_cost_model::cost_model::CostModel,
    solana_ledger::{
        blockstore_processor::TransactionStatusSender,
        transaction_balances::compile_collected_balances,
    },
    solana_measure::measure_us,
    solana_runtime::{
        bank::{Bank, ProcessedTransactionCounts},
        bank_utils,
        prioritization_fee_cache::PrioritizationFeeCache,
        transaction_batch::TransactionBatch,
        vote_sender_types::{ReplayVoteSendType, ReplayVoteSender},
    },
    solana_runtime_transaction::transaction_with_meta::TransactionWithMeta,
    solana_svm::{
        transaction_balances::BalanceCollector,
        transaction_commit_result::{TransactionCommitResult, TransactionCommitResultExtensions},
        transaction_processing_result::TransactionProcessingResult,
    },
    solana_transaction_error::TransactionError,
    std::{num::Saturating, sync::Arc},
};

pub(crate) static FIREDANCER_COMMITTER: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(0);

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

#[unsafe(no_mangle)]
pub extern "C" fn fd_ext_bank_load_and_execute_txns( bank: *const std::ffi::c_void, txns: *const std::ffi::c_void, txn_count: u64, out_processing_result: *mut i32, out_transaction_err: *mut i32, out_consumed_exec_cus: *mut u32, out_consumed_acct_data_cus: *mut u32 ) -> *mut std::ffi::c_void {
    use solana_svm_timings::ExecuteTimings;
    use solana_runtime::bank::LoadAndExecuteTransactionsOutput;
    use solana_runtime::transaction_batch::OwnedOrBorrowed;
    use solana_runtime_transaction::runtime_transaction::RuntimeTransaction;
    use solana_clock::MAX_PROCESSING_AGE;
    use solana_transaction::sanitized::SanitizedTransaction;
    use solana_svm::transaction_error_metrics::TransactionErrorMetrics;
    use solana_svm::transaction_processing_result::ProcessedTransaction::{Executed,FeesOnly};
    use solana_svm::transaction_processor::ExecutionRecordingConfig;
    use solana_svm::transaction_processor::TransactionProcessingConfig;
    use solana_cost_model::cost_model::CostModel;
    use solana_runtime_transaction::transaction_meta::StaticMeta;
    use std::sync::atomic::Ordering;
    use solana_vote_program::vote_processor::DEFAULT_COMPUTE_UNITS;

    const FD_BANK_TRANSACTION_LANDED: i32 = 1;
    const FD_BANK_TRANSACTION_EXECUTED: i32 = 2;

    let txns = unsafe {
        std::slice::from_raw_parts(txns as *const RuntimeTransaction<SanitizedTransaction>, txn_count as usize)
    };
    let bank = bank as *const Bank;
    unsafe { Arc::increment_strong_count(bank) };
    let bank = unsafe { Arc::from_raw( bank as *const Bank ) };

    let remove_simple_vote_from_cost_model =
        bank.feature_set.is_active(&agave_feature_set::remove_simple_vote_from_cost_model::id());

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

    let mut timings = ExecuteTimings::default();
    let transaction_status_sender_enabled = committer.transaction_status_sender_enabled();
    let output = bank.load_and_execute_transactions(&batch, MAX_PROCESSING_AGE, &mut timings,
        &mut TransactionErrorMetrics::default(),
        TransactionProcessingConfig {
            account_overrides: None,
            check_program_deployment_slot: bank.check_program_deployment_slot(),
            log_messages_bytes_limit: None,
            limit_to_load_programs: false,
            recording_config: ExecutionRecordingConfig::new_single_setting(transaction_status_sender_enabled),
            drop_on_failure: false,
            all_or_nothing: false,
            strict_nonce_size_check: true,
            tip_accounts: Some(tip_accounts),
        }
    );

    for i in 0..txn_count {
        let (processing_result, consumed_cus, loaded_accounts_data_cost, transaction_err) =
            match &output.processing_results[i as usize] {
                Err(err) => (0, 0u32, 0u32, transaction_error_to_code(&err)),
                Ok(Executed(tx)) => {
                    let (consumed_cus, loaded_accounts_data_cost) =
                        if !remove_simple_vote_from_cost_model
                            && txns[i as usize].is_simple_vote_transaction()
                        {
                            // See TransactionCost::SimpleVote
                            // - programs_execution_cost        = DEFAULT_COMPUTE_UNITS (2100)
                            // - loaded_accounts_data_size_cost = 8
                            (DEFAULT_COMPUTE_UNITS as u32, 8u32)
                        } else {
                            (
                                /* Executed CUs must be less than the block CU limit, which is much less
                                   than UINT_MAX, so the cast should be safe */
                                tx.execution_details.executed_units.try_into().unwrap(),
                                CostModel::calculate_loaded_accounts_data_size_cost(
                                    tx.loaded_transaction.loaded_accounts_data_size,
                                    &bank.feature_set,
                                ) as u32,
                            )
                        };
                    (
                        FD_BANK_TRANSACTION_LANDED | FD_BANK_TRANSACTION_EXECUTED,
                        consumed_cus,
                        loaded_accounts_data_cost,
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

#[unsafe(no_mangle)]
pub extern "C" fn fd_ext_bank_commit_txns( bank: *const std::ffi::c_void, txns: *const std::ffi::c_void, txn_count: u64, load_and_execute_output: *mut std::ffi::c_void ) {
    use solana_transaction::sanitized::SanitizedTransaction;
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
        if FIREDANCER_COMMITTER.load(Ordering::Relaxed) != 0 {
            break;
        }
        std::hint::spin_loop();
    }
    let committer: &Committer = unsafe { (FIREDANCER_COMMITTER.load(Ordering::Acquire) as *const Committer).as_ref().unwrap() };
    let mut timings = LeaderExecuteAndCommitTimings::default();

    let _ = committer.commit_transactions(
        &batch,
        load_and_execute_output.processing_results,
        None,
        &bank,
        load_and_execute_output.balance_collector,
        &mut timings,
        &load_and_execute_output.processed_counts);
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum CommitTransactionDetails {
    Committed {
        compute_units: u64,
        loaded_accounts_data_size: u32,
        fee_payer_post_balance: u64,
        result: Result<(), TransactionError>,
    },
    NotCommitted(TransactionError),
}

#[derive(Clone)]
pub struct Committer {
    transaction_status_sender: Option<TransactionStatusSender>,
    replay_vote_sender: ReplayVoteSender,
    prioritization_fee_cache: Option<Arc<PrioritizationFeeCache>>,
}

impl Committer {
    pub fn new(
        transaction_status_sender: Option<TransactionStatusSender>,
        replay_vote_sender: ReplayVoteSender,
        prioritization_fee_cache: Option<Arc<PrioritizationFeeCache>>,
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
        bank: &Bank,
        balance_collector: Option<BalanceCollector>,
        execute_and_commit_timings: &mut LeaderExecuteAndCommitTimings,
        processed_counts: &ProcessedTransactionCounts,
    ) -> (u64, Vec<CommitTransactionDetails>) {
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
                    result: committed_tx.status.clone(),
                    fee_payer_post_balance: committed_tx.fee_payer_post_balance,
                },
                Err(err) => CommitTransactionDetails::NotCommitted(err.clone()),
            })
            .collect();

        let ((), find_and_send_votes_us) = measure_us!({
            bank_utils::find_and_send_votes(
                batch.sanitized_transactions(),
                &commit_results,
                Some(&self.replay_vote_sender),
                ReplayVoteSendType::VerifiedExecuted,
            );

            if let Some(prioritization_fee_cache) = self.prioritization_fee_cache.as_ref() {
                let committed_transactions = commit_results
                    .iter()
                    .zip(batch.sanitized_transactions())
                    .filter_map(|(commit_result, tx)| commit_result.was_committed().then_some(tx));
                prioritization_fee_cache.update(bank, committed_transactions);
            }

            self.collect_balances_and_send_status_batch(
                commit_results,
                bank,
                batch,
                balance_collector,
                starting_transaction_index,
            );
        });
        execute_and_commit_timings.find_and_send_votes_us = find_and_send_votes_us;
        (commit_time_us, commit_transaction_statuses)
    }

    fn collect_balances_and_send_status_batch(
        &self,
        commit_results: Vec<TransactionCommitResult>,
        bank: &Bank,
        batch: &TransactionBatch<impl TransactionWithMeta>,
        balance_collector: Option<BalanceCollector>,
        starting_transaction_index: Option<usize>,
    ) {
        if let Some(transaction_status_sender) = &self.transaction_status_sender {
            let sanitized_transactions = batch.sanitized_transactions();

            // Clone `SanitizedTransaction` out of `RuntimeTransaction`, this is
            // done to send over the status sender.
            let txs = sanitized_transactions
                .iter()
                .map(|tx| tx.as_sanitized_transaction().into_owned())
                .collect_vec();
            let mut transaction_index = Saturating(starting_transaction_index.unwrap_or_default());
            let (batch_transaction_indexes, tx_costs): (Vec<_>, Vec<_>) = commit_results
                .iter()
                .zip(sanitized_transactions.iter())
                .map(|(commit_result, tx)| {
                    if let Ok(committed_tx) = commit_result {
                        let Saturating(this_transaction_index) = transaction_index;
                        transaction_index += 1;

                        let tx_cost = Some(
                            CostModel::calculate_cost_for_executed_transaction(
                                tx,
                                committed_tx.executed_units,
                                committed_tx.loaded_account_stats.loaded_accounts_data_size,
                                &bank.feature_set,
                            )
                            .sum(),
                        );

                        (this_transaction_index, tx_cost)
                    } else {
                        (0, Some(0))
                    }
                })
                .unzip();

            // There are two cases where balance_collector could be None:
            // * Balance recording is disabled. If that were the case, there would
            //   be no TransactionStatusSender, and we would not be in this branch.
            // * The batch was aborted in its entirety in SVM. In that case, there
            //   would be zero processed transactions, and commit_transactions()
            //   would not have been called at all.
            // Therefore this should always be true.
            debug_assert!(balance_collector.is_some());

            let (balances, token_balances) =
                compile_collected_balances(balance_collector.unwrap_or_default());

            transaction_status_sender.send_transaction_status_batch(
                bank.slot(),
                txs,
                commit_results,
                balances,
                token_balances,
                tx_costs,
                batch_transaction_indexes,
            );
        }
    }
}
