/// FIREDANCER: PohService is not constructed -- the poh tile is the tick
///             producer -- but PohService owns two responsibilities in the
///             TowerBFT to alpenglow migration that nothing else performs.
///             This thread performs them in its place.
use {
    crate::{poh_recorder::PohRecorder, record_channels::RecordReceiver},
    agave_votor_messages::migration::MigrationStatus,
    crossbeam_channel::Sender,
    log::*,
    std::{
        sync::{
            Arc, RwLock,
            atomic::{AtomicBool, Ordering},
        },
        thread::{self, Builder, JoinHandle},
        time::Duration,
    },
};

/* How often to look at shutdown_poh.  The migration is a once in a cluster
   lifetime event and replay_stage is blocked in wait_for_migration_or_exit
   until this thread responds, so this trades an irrelevant amount of latency
   for a thread that costs nothing while it waits. */
const POLL_INTERVAL: Duration = Duration::from_millis(1);

/* The two things upstream's PohService does at the ReadyToEnable to
   AlpenglowEnabled transition (poh_service.rs, end of tick_producer):

     1. It stops producing ticks.  Here the tick producer is the poh tile,
        so the equivalent is switching the tile into alpenglow mode.  It has
        to happen before BlockCreationLoop can open a block, or the first
        alpenglow block is produced in TowerBFT shape.

     2. It hands the RecordReceiver to BlockCreationLoop, which is what
        releases the loop, and then calls poh_service_is_shutting_down(),
        which is what moves the phase to AlpenglowEnabled and wakes
        replay_stage, votor's event handler and its timer manager.

   Without (2) nothing ever leaves ReadyToEnable, so a migration on a running
   cluster stops there for good.  Starting up already past the migration
   works either way, because enable_alpenglow_during_startup() sees that
   PohService was never started and enables inline. */
pub struct FiredancerPohShutdown {
    _thread: JoinHandle<()>,
}

impl FiredancerPohShutdown {
    pub fn new(
        migration_status: Arc<MigrationStatus>,
        poh_recorder: Arc<RwLock<PohRecorder>>,
        record_receiver_sender: Sender<RecordReceiver>,
        record_receiver: RecordReceiver,
        exit: Arc<AtomicBool>,
    ) -> Self {
        let thread = Builder::new()
            .name("solFdPohShutdn".to_string())
            .spawn(move || {
                /* On a cluster that was already running alpenglow at boot,
                   MigrationStatus::new starts shutdown_poh true, so this
                   falls straight through and the handoff happens during
                   startup exactly as it did when it was unconditional. */
                while !migration_status.shutdown_poh.load(Ordering::Acquire) {
                    if exit.load(Ordering::Relaxed) {
                        return;
                    }
                    thread::sleep(POLL_INTERVAL);
                }

                poh_recorder.write().unwrap().enable_alpenglow();

                if let Err(e) = record_receiver_sender.send(record_receiver) {
                    /* Only reachable if BlockCreationLoop has already gone
                       away, which means we are shutting down. */
                    error!("Unable to send record receiver, already shutting down {e:?}");
                    return;
                }

                /* Only the live migration path is still in ReadyToEnable
                   here.  The startup path enables inline, so the phase has
                   already moved and calling this again would hit the
                   unreachable! inside it. */
                if migration_status.is_ready_to_enable() {
                    migration_status.poh_service_is_shutting_down();
                }

                info!("PoH has stopped producing ticks, alpenglow is enabled");
            })
            .unwrap();

        Self { _thread: thread }
    }
}
