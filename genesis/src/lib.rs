// FIREDANCER: Allow special_module_name linter here to prevent warnings
// about the gross hack we did below, which minimizes merge conflicts.
#![allow(special_module_name)]
#![allow(clippy::arithmetic_side_effects)]
pub mod address_generator;
pub mod genesis_accounts;
pub mod stakes;
pub mod unlocks;

use serde::{Deserialize, Serialize};

/// An account where the data is encoded as a Base64 string.
#[derive(Serialize, Deserialize, Debug)]
pub struct Base64Account {
    pub balance: u64,
    pub owner: String,
    pub data: String,
    pub executable: bool,
}

/// FIREDANCER: Kind of hacky but we do this to make the change as surgical as
/// possible so we don't keep generating merge conflicts. Main is treated as
/// a module that's imported by the library.
mod main;

/// FIREDANCER: Firedancer links directly to the Solana Labs client so that it can
/// build and distribute one binary. This exported function is what it calls to
/// start up the Solana Labs child process side.
#[no_mangle]
pub extern "C" fn fd_ext_genesis_main(argv: *const *const i8) {
    use std::os::unix::ffi::OsStringExt;
    use std::ffi::{CStr, OsString};

    // FIREDANCER: Prevent warnings about unused code in the main.rs,
    // which we don't want to change to avoid merge conflicts.
    let _ = main::AccountFileFormat::Keypair;
    let _ = main::AccountFileFormat::Pubkey;

    let mut args = vec![];

    let mut index = 0;
    unsafe {
        while !(*argv.offset(index)).is_null() {
            args.push(OsString::from_vec(CStr::from_ptr(*argv.offset(index)).to_bytes().to_vec()));

            index += 1;
        }
    }

    main::main(args.into_iter().map(OsString::from)).unwrap();
}
