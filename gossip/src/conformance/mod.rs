//! Gossip differential conformance testing.

#[cfg(test)]
mod tests;

use {
    crate::{
        crds_data::CrdsData,
        crds_gossip_pull::CrdsFilter as NativeCrdsFilter,
        crds_value::CrdsValue as NativeCrdsValue,
        ping_pong::Pong,
        protocol::{Ping, Protocol, PruneData},
    },
    protosol::protos::{
        gossip_crds_data, gossip_msg, GossipBloom, GossipContactInfo, GossipCrdsData,
        GossipCrdsFilter, GossipCrdsValue, GossipDuplicateShred, GossipEffects,
        GossipEpochSlots, GossipIncrementalHash, GossipLowestSlot, GossipMsg, GossipPing,
        GossipPong, GossipPruneData, GossipPruneMessage, GossipPullRequest,
        GossipPullResponse, GossipPushMessage, GossipSnapshotHashes, GossipVote,
    },
    solana_keypair::signable::Signable,
};
use bv::Bits;

fn convert_ping(ping: &Ping) -> gossip_msg::Msg {
    gossip_msg::Msg::Ping(GossipPing {
        from: ping.pubkey().to_bytes().to_vec(),
        token: ping.signable_data().to_vec(),
        signature: ping.get_signature().as_ref().to_vec(),
    })
}

fn convert_pong(pong: &Pong) -> gossip_msg::Msg {
    gossip_msg::Msg::Pong(GossipPong {
        from: pong.from().to_bytes().to_vec(),
        hash: pong.signable_data().to_vec(),
        signature: pong.signature().as_ref().to_vec(),
    })
}

fn convert_bloom(bloom: &solana_bloom::bloom::Bloom<solana_hash::Hash>) -> GossipBloom {
    // Convert BitVec<u64> to bytes (little-endian u64 blocks)
    let mut bits_bytes = Vec::new();
    for i in 0..bloom.bits.block_len() {
        bits_bytes.extend_from_slice(&bloom.bits.get_block(i).to_le_bytes());
    }
    GossipBloom {
        keys: bloom.keys.clone(),
        bits: bits_bytes,
        num_bits_set: bloom.num_bits_set(),
    }
}

fn convert_crds_filter(filter: &NativeCrdsFilter) -> GossipCrdsFilter {
    GossipCrdsFilter {
        filter: Some(convert_bloom(&filter.filter)),
        mask: filter.mask(),
        mask_bits: filter.mask_bits(),
    }
}

fn convert_crds_data(data: &CrdsData) -> GossipCrdsData {
    let converted = match data {
        CrdsData::ContactInfo(ci) => {
            Some(gossip_crds_data::Data::ContactInfo(GossipContactInfo {
                pubkey: ci.pubkey().to_bytes().to_vec(),
                wallclock: ci.wallclock(),
                outset: ci.outset(),
                shred_version: ci.shred_version() as u32,
            }))
        }
        CrdsData::Vote(idx, vote) => {
            let tx_bytes = bincode::serialize(vote.transaction()).unwrap_or_default();
            Some(gossip_crds_data::Data::Vote(GossipVote {
                index: *idx as u32,
                from: vote.from().to_bytes().to_vec(),
                wallclock: vote.vote_wallclock(),
                transaction: tx_bytes,
            }))
        }
        CrdsData::LowestSlot(_, ls) => {
            Some(gossip_crds_data::Data::LowestSlot(GossipLowestSlot {
                index: 0,
                from: ls.from().to_bytes().to_vec(),
                lowest: ls.lowest,
                wallclock: ls.wallclock(),
            }))
        }
        CrdsData::EpochSlots(idx, es) => {
            Some(gossip_crds_data::Data::EpochSlots(GossipEpochSlots {
                index: *idx as u32,
                from: es.from.to_bytes().to_vec(),
                wallclock: es.wallclock,
            }))
        }
        CrdsData::SnapshotHashes(sh) => {
            Some(gossip_crds_data::Data::SnapshotHashes(
                GossipSnapshotHashes {
                    from: sh.from.to_bytes().to_vec(),
                    full_slot: sh.full.0,
                    full_hash: sh.full.1.to_bytes().to_vec(),
                    incremental: sh
                        .incremental
                        .iter()
                        .map(|(slot, hash)| GossipIncrementalHash {
                            slot: *slot,
                            hash: hash.to_bytes().to_vec(),
                        })
                        .collect(),
                    wallclock: sh.wallclock,
                },
            ))
        }
        CrdsData::DuplicateShred(idx, ds) => {
            Some(gossip_crds_data::Data::DuplicateShred(
                GossipDuplicateShred {
                    index: *idx as u32,
                    from: ds.from().to_bytes().to_vec(),
                    wallclock: ds.wallclock(),
                    slot: ds.slot(),
                    shred_index: 0,
                    shred_type: 0,
                    num_chunks: ds.num_chunks() as u32,
                    chunk_index: ds.chunk_index() as u32,
                    chunk: ds.chunk().to_vec(),
                },
            ))
        }
        // Deprecated variants
        CrdsData::LegacyContactInfo(..)
        | CrdsData::LegacySnapshotHashes(..)
        | CrdsData::AccountsHashes(..)
        | CrdsData::LegacyVersion(..)
        | CrdsData::Version(..)
        | CrdsData::NodeInstance(..)
        | CrdsData::RestartLastVotedForkSlots(..)
        | CrdsData::RestartHeaviestFork(..) => None,
    };
    GossipCrdsData { data: converted }
}

fn convert_crds_value(value: &NativeCrdsValue) -> GossipCrdsValue {
    GossipCrdsValue {
        signature: value.get_signature().as_ref().to_vec(),
        data: Some(convert_crds_data(value.data())),
    }
}

fn convert_prune_data(pd: &PruneData) -> GossipPruneData {
    GossipPruneData {
        pubkey: pd.pubkey.to_bytes().to_vec(),
        prunes: pd.prunes.iter().map(|p| p.to_bytes().to_vec()).collect(),
        signature: pd.signature.as_ref().to_vec(),
        destination: pd.destination.to_bytes().to_vec(),
        wallclock: pd.wallclock,
    }
}

fn convert_protocol(proto: &Protocol) -> gossip_msg::Msg {
    match proto {
        Protocol::PingMessage(ping) => convert_ping(ping),
        Protocol::PongMessage(pong) => convert_pong(pong),
        Protocol::PullRequest(filter, value) => {
            gossip_msg::Msg::PullRequest(GossipPullRequest {
                filter: Some(convert_crds_filter(filter)),
                value: Some(convert_crds_value(value)),
            })
        }
        Protocol::PullResponse(pubkey, values) => {
            gossip_msg::Msg::PullResponse(GossipPullResponse {
                pubkey: pubkey.to_bytes().to_vec(),
                values: values.iter().map(convert_crds_value).collect(),
            })
        }
        Protocol::PushMessage(pubkey, values) => {
            gossip_msg::Msg::PushMessage(GossipPushMessage {
                pubkey: pubkey.to_bytes().to_vec(),
                values: values.iter().map(convert_crds_value).collect(),
            })
        }
        Protocol::PruneMessage(pubkey, data) => {
            gossip_msg::Msg::PruneMessage(GossipPruneMessage {
                pubkey: pubkey.to_bytes().to_vec(),
                data: Some(convert_prune_data(data)),
            })
        }
    }
}

pub fn gossip_decode_to_effects(input: &[u8]) -> GossipEffects {
    use {bincode::Options, solana_perf::packet::PACKET_DATA_SIZE, solana_sanitize::Sanitize};

    let result = bincode::options()
        .with_limit(PACKET_DATA_SIZE as u64)
        .with_fixint_encoding()
        .reject_trailing_bytes()
        .deserialize::<Protocol>(input);

    match result {
        Ok(proto) if proto.sanitize().is_ok() => {
            let msg = convert_protocol(&proto);
            GossipEffects {
                valid: true,
                msg: Some(GossipMsg { msg: Some(msg) }),
            }
        }
        _ => GossipEffects {
            valid: false,
            msg: None,
        },
    }
}
