/// # Safety
///
/// Must not be called while other threads may access the process environment.
#[unsafe(no_mangle)]
pub unsafe extern "C" fn sol_compat_init(_log_level: i32) {
    // Safety: The caller guarantees exclusive access to the environment.
    unsafe {
        std::env::set_var("SOLANA_RAYON_THREADS", "1");
        std::env::set_var("RAYON_NUM_THREADS", "1");
    }
    if std::env::var("ENABLE_SOLANA_LOGGER").is_ok() {
        agave_logger::setup();
    }
}

#[unsafe(no_mangle)]
pub extern "C" fn sol_compat_fini() {}
