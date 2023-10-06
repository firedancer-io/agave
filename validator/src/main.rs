#![allow(clippy::arithmetic_side_effects)]
#[cfg(not(any(target_env = "msvc", target_os = "freebsd")))]
use jemallocator::Jemalloc;
use {
    // FIREDANCER: We have inverted the dependency, so the library depends on main.rs so that this
    // import now just needs to refer to the crate.
    crate::{
        cli::{app, warn_for_deprecated_arguments, DefaultArgs},
        commands,
    },
    log::error,
    std::{path::PathBuf, process::exit},
};

#[cfg(not(any(target_env = "msvc", target_os = "freebsd")))]
#[global_allocator]
static GLOBAL: Jemalloc = Jemalloc;

// FIREDANCER: Switch main to be a function that takes arguments, rather than
// an actual entrypoint for the binary.
pub fn main<I, T>(itr: I)
where
        I: IntoIterator<Item = T>,
        T: Into<std::ffi::OsString> + Clone {
    let args: Vec<std::ffi::OsString> = itr.into_iter().map(|x| x.into()).collect();

    let default_args = DefaultArgs::new();
    let solana_version = solana_version::version!();
    let cli_app = app(solana_version, &default_args);
    // FIREDANCER: Parse matches from the provided arguments rather than env_os()
    let matches = cli_app.get_matches_from(&args);
    warn_for_deprecated_arguments(&matches);

    let ledger_path = PathBuf::from(matches.value_of("ledger_path").unwrap());

    match matches.subcommand() {
        ("init", _) => commands::run::execute(
            &matches,
            solana_version,
            commands::run::execute::Operation::Initialize,
            &args,
        )
        .inspect_err(|err| error!("Failed to initialize validator: {err}"))
        .map_err(commands::Error::Dynamic),
        ("", _) | ("run", _) => commands::run::execute(
            &matches,
            solana_version,
            commands::run::execute::Operation::Run,
            &args,
        )
        .inspect_err(|err| error!("Failed to start validator: {err}"))
        .map_err(commands::Error::Dynamic),
        ("authorized-voter", Some(authorized_voter_subcommand_matches)) => {
            commands::authorized_voter::execute(authorized_voter_subcommand_matches, &ledger_path)
        }
        ("plugin", Some(plugin_subcommand_matches)) => {
            commands::plugin::execute(plugin_subcommand_matches, &ledger_path)
        }
        ("contact-info", Some(subcommand_matches)) => {
            commands::contact_info::execute(subcommand_matches, &ledger_path)
        }
        ("exit", Some(subcommand_matches)) => {
            commands::exit::execute(subcommand_matches, &ledger_path)
        }
        ("monitor", _) => commands::monitor::execute(&matches, &ledger_path),
        ("staked-nodes-overrides", Some(subcommand_matches)) => {
            commands::staked_nodes_overrides::execute(subcommand_matches, &ledger_path)
        }
        ("set-identity", Some(subcommand_matches)) => {
            commands::set_identity::execute(subcommand_matches, &ledger_path)
        }
        ("set-log-filter", Some(subcommand_matches)) => {
            commands::set_log_filter::execute(subcommand_matches, &ledger_path)
        }
        ("wait-for-restart-window", Some(subcommand_matches)) => {
            commands::wait_for_restart_window::execute(subcommand_matches, &ledger_path)
        }
        ("repair-shred-from-peer", Some(subcommand_matches)) => {
            commands::repair_shred_from_peer::execute(subcommand_matches, &ledger_path)
        }
        ("repair-whitelist", Some(repair_whitelist_subcommand_matches)) => {
            commands::repair_whitelist::execute(repair_whitelist_subcommand_matches, &ledger_path)
        }
        ("set-public-address", Some(subcommand_matches)) => {
            commands::set_public_address::execute(subcommand_matches, &ledger_path)
        }
        ("manage-block-production", Some(subcommand_matches)) => {
            commands::manage_block_production::execute(subcommand_matches, &ledger_path)
        }
        _ => unreachable!(),
    }
    .unwrap_or_else(|err| {
        println!("Validator command failed: {err}");
        exit(1);
    })
}
