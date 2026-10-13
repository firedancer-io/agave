#[cfg(feature = "dev-context-only-utils")]
use qualifier_attr::qualifiers;
use thiserror::Error;

/// Errors that the [`SigVerifier`] can experience.
#[derive(Error, Debug)]
#[cfg_attr(feature = "dev-context-only-utils", qualifiers(pub))]
pub(super) enum SigVerifyError {
    #[error("verifying votes failed with {0}")]
    SigverifyVotes(#[from] SigVerifyVoteError),
    #[error("verifying certs failed with {0}")]
    SigverifyCerts(#[from] SigVerifyCertError),
}

/// Different types of errors that sig verifying votes can fail with.
#[derive(Debug, Error)]
#[allow(clippy::enum_variant_names)]
#[cfg_attr(feature = "dev-context-only-utils", qualifiers(pub))]
pub(super) enum SigVerifyVoteError {
    #[error("channel \"{0}\" disconnected")]
    ChannelDisconnected(&'static str),
}

/// Different types of errors that sig verifying certs can fail with.
#[derive(Debug, Error)]
#[cfg_attr(feature = "dev-context-only-utils", qualifiers(pub))]
pub(super) enum SigVerifyCertError {
    #[error("channel \"{0}\" disconnected")]
    ChannelDisconnected(&'static str),
}
