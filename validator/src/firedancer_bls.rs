//! FIREDANCER: BLS helpers exposed to the C side.
//!
//! Firedancer's genesis creator (`fd_genesis_create.c`) writes the vote
//! account directly into the genesis blob. For an alpenglow genesis that
//! account must be a `VoteStateV4` carrying the validator's compressed BLS
//! pubkey. Firedancer's C BLS code can only verify (G1/G2 arithmetic,
//! pairing, proof-of-possession verify) - it has no signing and no key
//! derivation - so the derivation is done here and handed back over FFI.
//!
//! The derivation must stay identical to
//! `agave_votor::voting_utils::get_or_derive_bls_keypair`, which derives from
//! the *authorized voter* keypair using `BLS_KEYPAIR_DERIVE_SEED`. If these
//! ever diverge the validator would sign votes with a key that is not the one
//! registered in its vote account.

use {
    agave_votor_messages::consensus_message::BLS_KEYPAIR_DERIVE_SEED,
    solana_bls_signatures::keypair::Keypair as BLSKeypair, solana_keypair::Keypair,
};

/// Size of a compressed BLS12-381 pubkey, in bytes.
pub const FD_EXT_BLS_PUBKEY_COMPRESSED_SZ: usize = 48;

/// Length of an ed25519 keypair as stored on disk, in bytes.
const ED25519_KEYPAIR_SZ: usize = 64;

/// Derives the alpenglow BLS pubkey for the given ed25519 keypair.
///
/// `keypair_bytes` points to the 64 byte ed25519 keypair (32 byte secret
/// followed by the 32 byte pubkey) of the account that will act as the
/// authorized voter. `out_pubkey` receives the 48 byte compressed BLS pubkey.
///
/// Returns 0 on success and -1 on failure, in which case `out_pubkey` is left
/// untouched.
///
/// # Safety
///
/// `keypair_bytes` must point to at least 64 readable bytes and `out_pubkey`
/// to at least `FD_EXT_BLS_PUBKEY_COMPRESSED_SZ` writable bytes.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn fd_ext_bls_derive_pubkey(
    keypair_bytes: *const u8,
    out_pubkey: *mut u8,
) -> i32 {
    if keypair_bytes.is_null() || out_pubkey.is_null() {
        return -1;
    }

    let bytes = unsafe { std::slice::from_raw_parts(keypair_bytes, ED25519_KEYPAIR_SZ) };
    let Ok(keypair) = Keypair::try_from(bytes) else {
        return -1;
    };

    let Ok(bls_keypair) = BLSKeypair::derive_from_signer(&keypair, BLS_KEYPAIR_DERIVE_SEED) else {
        return -1;
    };

    let compressed = bls_keypair.public.to_bytes_compressed();
    if compressed.len() != FD_EXT_BLS_PUBKEY_COMPRESSED_SZ {
        return -1;
    }

    unsafe {
        std::ptr::copy_nonoverlapping(
            compressed.as_ptr(),
            out_pubkey,
            FD_EXT_BLS_PUBKEY_COMPRESSED_SZ,
        );
    }

    0
}
