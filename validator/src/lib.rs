// FIREDANCER: Allow special_module_name linter here to prevent warnings
// about the gross hack we did below, which minimizes merge conflicts.
#![allow(special_module_name)]
#![allow(clippy::arithmetic_side_effects)]
pub use solana_test_validator as test_validator;
use {
    console::style,
    fd_lock::{RwLock, RwLockWriteGuard},
    indicatif::{ProgressDrawTarget, ProgressStyle},
    std::{
        borrow::Cow,
        fmt::Display,
        fs::{File, OpenOptions},
        path::Path,
        process::exit,
        time::Duration,
    },
};

pub mod admin_rpc_service;
pub mod bootstrap;
pub mod cli;
pub mod commands;
pub mod dashboard;

pub fn format_name_value(name: &str, value: &str) -> String {
    format!("{} {}", style(name).bold(), value)
}
/// Pretty print a "name value"
pub fn println_name_value(name: &str, value: &str) {
    println!("{}", format_name_value(name, value));
}

/// Creates a new process bar for processing that will take an unknown amount of time
pub fn new_spinner_progress_bar() -> ProgressBar {
    let progress_bar = indicatif::ProgressBar::new(42);
    progress_bar.set_draw_target(ProgressDrawTarget::stdout());
    progress_bar.set_style(
        ProgressStyle::default_spinner()
            .template("{spinner:.green} {wide_msg}")
            .expect("ProgresStyle::template direct input to be correct"),
    );
    progress_bar.enable_steady_tick(Duration::from_millis(100));

    ProgressBar {
        progress_bar,
        is_term: console::Term::stdout().is_term(),
    }
}

pub struct ProgressBar {
    progress_bar: indicatif::ProgressBar,
    is_term: bool,
}

impl ProgressBar {
    pub fn set_message<T: Into<Cow<'static, str>> + Display>(&self, msg: T) {
        if self.is_term {
            self.progress_bar.set_message(msg);
        } else {
            println!("{msg}");
        }
    }

    pub fn println<I: AsRef<str>>(&self, msg: I) {
        self.progress_bar.println(msg);
    }

    pub fn abandon_with_message<T: Into<Cow<'static, str>> + Display>(&self, msg: T) {
        if self.is_term {
            self.progress_bar.abandon_with_message(msg);
        } else {
            println!("{msg}");
        }
    }
}

pub fn ledger_lockfile(ledger_path: &Path) -> RwLock<File> {
    let lockfile = ledger_path.join("ledger.lock");
    fd_lock::RwLock::new(
        OpenOptions::new()
            .write(true)
            .create(true)
            .truncate(false)
            .open(lockfile)
            .unwrap(),
    )
}

pub fn lock_ledger<'lock>(
    ledger_path: &Path,
    ledger_lockfile: &'lock mut RwLock<File>,
) -> RwLockWriteGuard<'lock, File> {
    ledger_lockfile.try_write().unwrap_or_else(|_| {
        println!(
            "Error: Unable to lock {} directory. Check if another validator is running",
            ledger_path.display()
        );
        exit(1);
    })
}

/// FIREDANCER: Kind of hacky but we do this to make the change as surgical as
/// possible so we don't keep generating merge conflicts. Main is treated as
/// a module that's imported by the library.
mod main;

/// FIREDANCER: Firedancer links directly to the Solana Labs client so that it can
/// build and distribute one binary. This exported function is what it calls to
/// start up the Solana Labs child process side.
#[unsafe(no_mangle)]
pub extern "C" fn fd_ext_validator_main(argv: *const *const i8) {
    use std::os::unix::ffi::OsStringExt;
    use std::ffi::{CStr, OsString};

    let mut args = vec![];

    let mut index = 0;
    unsafe {
        while !(*argv.offset(index)).is_null() {
            args.push(OsString::from_vec(CStr::from_ptr(*argv.offset(index)).to_bytes().to_vec()));

            index += 1;
        }
    }

    main::main(args.into_iter().map(OsString::from));
}
