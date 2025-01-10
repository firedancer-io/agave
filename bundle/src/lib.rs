use {
    crate::bundle_execution::LoadAndExecuteBundleError,
    itertools::Itertools,
    sha2::{Digest, Sha256},
    solana_sdk::transaction::SanitizedTransaction,
    thiserror::Error,
};

pub mod bundle_execution;

pub type BundleExecutionResult<T> = Result<T, BundleExecutionError>;

#[derive(Error, Debug, Clone)]
pub enum BundleExecutionError {
    #[error("The bank has hit the max allotted time for processing transactions")]
    BankProcessingTimeLimitReached,

    #[error("Runtime error while executing the bundle: {0}")]
    TransactionFailure(#[from] LoadAndExecuteBundleError),
}

#[derive(Debug)]
pub struct SanitizedBundle {
    pub transactions: Vec<SanitizedTransaction>,
    pub bundle_id: String,
}

pub fn derive_bundle_id_from_sanitized_transactions(
    transactions: &[SanitizedTransaction],
) -> String {
    let mut hasher = Sha256::new();
    hasher.update(transactions.iter().map(|tx| tx.signature()).join(","));
    format!("{:x}", hasher.finalize())
}
