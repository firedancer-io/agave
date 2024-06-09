use {
    solana_gossip::{
        cluster_info::ClusterInfo, legacy_contact_info::LegacyContactInfo as ContactInfo,
    },
    solana_poh::poh_recorder::PohRecorder,
    solana_sdk::{clock::FORWARD_TRANSACTIONS_TO_LEADER_AT_SLOT_OFFSET, pubkey::Pubkey},
    std::{net::SocketAddr, sync::RwLock},
};

pub(crate) fn next_leader_tpu_vote(
    cluster_info: &ClusterInfo,
    poh_recorder: &RwLock<PohRecorder>,
) -> Option<(Pubkey, std::net::SocketAddr)> {
    next_leader_n(
        cluster_info,
        poh_recorder,
        FORWARD_TRANSACTIONS_TO_LEADER_AT_SLOT_OFFSET,
        ContactInfo::tpu_vote,
    )
}

pub(crate) fn next_leader_tpu_vote_maybe_vote_n(
    cluster_info: &ClusterInfo,
    poh_recorder: &RwLock<PohRecorder>,
    n: u64,
) -> Option<(Pubkey, std::net::SocketAddr)> {
    next_leader_n(cluster_info, poh_recorder, n, ContactInfo::tpu_vote)
}

pub(crate) fn next_leader<F, E>(
    cluster_info: &ClusterInfo,
    poh_recorder: &RwLock<PohRecorder>,
    port_selector: F,
) -> Option<(Pubkey, SocketAddr)>
where
    F: FnOnce(&ContactInfo) -> Result<SocketAddr, E>,
{
    next_leader_n(
        cluster_info,
        poh_recorder,
        FORWARD_TRANSACTIONS_TO_LEADER_AT_SLOT_OFFSET,
        port_selector,
    )
}

fn next_leader_n<F, E>(
    cluster_info: &ClusterInfo,
    poh_recorder: &RwLock<PohRecorder>,
    n: u64,
    port_selector: F,
) -> Option<(Pubkey, SocketAddr)>
where
    F: FnOnce(&ContactInfo) -> Result<SocketAddr, E>,
{
    let leader_pubkey = poh_recorder.read().unwrap().leader_after_n_slots(n)?;
    cluster_info
        .lookup_contact_info(&leader_pubkey, port_selector)?
        .map(|addr| (leader_pubkey, addr))
        .ok()
}
