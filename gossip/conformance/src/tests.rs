//! Gossip conformance tests and fixture generation.

use protosol::protos::{gossip_crds_data, gossip_msg, GossipEffects};

fn get_effects(input: &[u8]) -> GossipEffects {
    crate::gossip_decode_to_effects(input)
}

fn check(input: &[u8], expect_valid: bool) {
    let effects = get_effects(input);
    assert_eq!(
        effects.valid, expect_valid,
        "effects.valid mismatch: input len={}, expected {expect_valid}",
        input.len(),
    );
    if !expect_valid {
        assert!(effects.msg.is_none(), "invalid input should have no msg");
    } else {
        assert!(effects.msg.is_some(), "valid input should have a msg");
    }
}

/// Returns the inner gossip_msg::Msg variant from effects, panicking if invalid.
fn unwrap_msg(effects: &GossipEffects) -> &gossip_msg::Msg {
    effects
        .msg
        .as_ref()
        .expect("effects.msg is None")
        .msg
        .as_ref()
        .expect("msg.msg is None")
}

// Binary layout helpers.
//
// Protocol is bincode-serialized with fixint encoding:
//   4-byte LE u32 variant discriminant, then variant fields.
//
// Variant indices (from enum declaration order):
//   0 = PullRequest(CrdsFilter, CrdsValue)
//   1 = PullResponse(Pubkey, Vec<CrdsValue>)
//   2 = PushMessage(Pubkey, Vec<CrdsValue>)
//   3 = PruneMessage(Pubkey, PruneData)
//   4 = PingMessage(Ping)
//   5 = PongMessage(Pong)
//
// Ping { from: Pubkey(32), token: [u8; 32], signature: Signature(64) }
// Pong { from: Pubkey(32), hash: Hash(32), signature: Signature(64) }
// PruneData { pubkey: Pubkey(32), prunes: Vec<Pubkey>(8+n*32),
//             signature: Signature(64), destination: Pubkey(32), wallclock: u64(8) }
// CrdsValue { signature: Signature(64), data: CrdsData(4+...) }
//
// CrdsData variant indices:
//   0 = LegacyContactInfo    (deprecated)
//   1 = Vote(u8, Vote)
//   2 = LowestSlot(u8, LowestSlot)
//   3 = LegacySnapshotHashes (deprecated)
//   4 = AccountsHashes       (deprecated)
//   5 = EpochSlots(u8, EpochSlots)
//   6 = LegacyVersion        (deprecated)
//   7 = Version              (deprecated)
//   8 = NodeInstance          (deprecated)
//   9 = DuplicateShred(u16, DuplicateShred)
//  10 = SnapshotHashes
//  11 = ContactInfo

/// Build a PingMessage (variant 4) from raw fields.
fn make_ping_bytes(from: &[u8; 32], token: &[u8; 32], signature: &[u8; 64]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(132);
    buf.extend_from_slice(&4u32.to_le_bytes());
    buf.extend_from_slice(from);
    buf.extend_from_slice(token);
    buf.extend_from_slice(signature);
    buf
}

/// Build a PongMessage (variant 5) from raw fields.
fn make_pong_bytes(from: &[u8; 32], hash: &[u8; 32], signature: &[u8; 64]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(132);
    buf.extend_from_slice(&5u32.to_le_bytes());
    buf.extend_from_slice(from);
    buf.extend_from_slice(hash);
    buf.extend_from_slice(signature);
    buf
}

/// Build a PruneMessage (variant 3) from raw fields.
fn make_prune_bytes(
    outer_pubkey: &[u8; 32],
    pubkey: &[u8; 32],
    prunes: &[[u8; 32]],
    signature: &[u8; 64],
    destination: &[u8; 32],
    wallclock: u64,
) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&3u32.to_le_bytes());
    buf.extend_from_slice(outer_pubkey);
    // PruneData
    buf.extend_from_slice(pubkey);
    buf.extend_from_slice(&(prunes.len() as u64).to_le_bytes());
    for p in prunes {
        buf.extend_from_slice(p);
    }
    buf.extend_from_slice(signature);
    buf.extend_from_slice(destination);
    buf.extend_from_slice(&wallclock.to_le_bytes());
    buf
}

/// Build a PullResponse (variant 1) from raw fields.
fn make_pull_response_bytes(pubkey: &[u8; 32], values: &[Vec<u8>]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&1u32.to_le_bytes());
    buf.extend_from_slice(pubkey);
    buf.extend_from_slice(&(values.len() as u64).to_le_bytes());
    for v in values {
        buf.extend_from_slice(v);
    }
    buf
}

/// Build a PushMessage (variant 2) from raw fields.
fn make_push_message_bytes(pubkey: &[u8; 32], values: &[Vec<u8>]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&2u32.to_le_bytes());
    buf.extend_from_slice(pubkey);
    buf.extend_from_slice(&(values.len() as u64).to_le_bytes());
    for v in values {
        buf.extend_from_slice(v);
    }
    buf
}

/// Build a raw CrdsValue: signature(64) + crds_data_bytes.
fn make_crds_value_bytes(signature: &[u8; 64], crds_data: &[u8]) -> Vec<u8> {
    let mut buf = Vec::with_capacity(64 + crds_data.len());
    buf.extend_from_slice(signature);
    buf.extend_from_slice(crds_data);
    buf
}

/// Build a raw ContactInfo CrdsData (variant 11).
/// ContactInfo is a complex serialized struct; this builds a minimal valid one.
fn make_contact_info_crds_data(pubkey: &[u8; 32], wallclock: u64) -> Vec<u8> {
    // ContactInfo is serialized by bincode as a complex struct.
    // We build the object and serialize it rather than hand-coding the layout.
    use solana_gossip::{contact_info::ContactInfo, crds_data::CrdsData};
    let ci = ContactInfo::new(
        solana_pubkey::Pubkey::from(*pubkey),
        wallclock,
        0, // shred_version
    );
    bincode::serialize(&CrdsData::ContactInfo(ci)).unwrap()
}

/// Build a raw SnapshotHashes CrdsData (variant 10).
fn make_snapshot_hashes_crds_data(
    from: &[u8; 32],
    full_slot: u64,
    full_hash: &[u8; 32],
    incremental: &[(u64, [u8; 32])],
    wallclock: u64,
) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&10u32.to_le_bytes()); // CrdsData::SnapshotHashes discriminant
    buf.extend_from_slice(from);
    // full: (u64, Hash)
    buf.extend_from_slice(&full_slot.to_le_bytes());
    buf.extend_from_slice(full_hash);
    // incremental: Vec<(u64, Hash)>
    buf.extend_from_slice(&(incremental.len() as u64).to_le_bytes());
    for (slot, hash) in incremental {
        buf.extend_from_slice(&slot.to_le_bytes());
        buf.extend_from_slice(hash);
    }
    buf.extend_from_slice(&wallclock.to_le_bytes());
    buf
}

/// Build a raw EpochSlots CrdsData (variant 5).
fn make_epoch_slots_crds_data(index: u8, from: &[u8; 32], wallclock: u64) -> Vec<u8> {
    // EpochSlots is serialized by bincode. Build and serialize.
    use solana_gossip::{crds_data::CrdsData, epoch_slots::EpochSlots};
    let mut es = EpochSlots::new(solana_pubkey::Pubkey::from(*from), wallclock);
    es.from = solana_pubkey::Pubkey::from(*from);
    bincode::serialize(&CrdsData::EpochSlots(index, es)).unwrap()
}

/// Build a raw LowestSlot CrdsData (variant 2).
fn make_lowest_slot_crds_data(
    index: u8,
    from: &[u8; 32],
    lowest: u64,
    wallclock: u64,
) -> Vec<u8> {
    use solana_gossip::crds_data::{CrdsData, LowestSlot};
    let ls = LowestSlot::new(solana_pubkey::Pubkey::from(*from), lowest, wallclock);
    let mut data = bincode::serialize(&CrdsData::LowestSlot(0, ls)).unwrap();
    // Patch the index byte (offset 4 in the serialized CrdsData)
    data[4] = index;
    data
}

/// Build a PullRequest (variant 0) from raw CrdsFilter + CrdsValue bytes.
fn make_pull_request_bytes(filter_bytes: &[u8], value_bytes: &[u8]) -> Vec<u8> {
    let mut buf = Vec::new();
    buf.extend_from_slice(&0u32.to_le_bytes());
    buf.extend_from_slice(filter_bytes);
    buf.extend_from_slice(value_bytes);
    buf
}

/// Build a serialized CrdsFilter. The Bloom filter has a complex layout,
/// so we construct and serialize the Rust object.
fn make_crds_filter_bytes() -> Vec<u8> {
    use solana_gossip::crds_gossip_pull::CrdsFilter;
    bincode::serialize(&CrdsFilter::new_rand(1, 128)).unwrap()
}

/// Build a raw Vote CrdsData (variant 1).
/// The inner Transaction is complex; we construct and serialize via Rust objects.
fn make_vote_crds_data(index: u8, from: &[u8; 32], wallclock: u64) -> Vec<u8> {
    use {
        solana_gossip::crds_data::{CrdsData, Vote as CrdsVote},
        solana_keypair::Keypair,
        solana_signer::Signer,
        solana_vote_program::{vote_instruction, vote_state::Vote},
    };
    let keypair = Keypair::new_from_array(*from);
    let vote = Vote::new(vec![1], solana_hash::Hash::default());
    let vote_ix =
        vote_instruction::vote(&keypair.pubkey(), &keypair.pubkey(), vote);
    let mut vote_tx = solana_transaction::Transaction::new_with_payer(
        &[vote_ix],
        Some(&keypair.pubkey()),
    );
    vote_tx.partial_sign(&[&keypair], solana_hash::Hash::default());
    let crds_vote =
        CrdsVote::new(solana_pubkey::Pubkey::from(*from), vote_tx, wallclock)
            .expect("valid vote tx");
    bincode::serialize(&CrdsData::Vote(index, crds_vote)).unwrap()
}

const MAX_WALLCLOCK: u64 = 1_000_000_000_000_000;

// Ping tests

#[test]
fn test_conformance_ping_valid() {
    let from = [0x11; 32];
    let token = [0x22; 32];
    let sig = [0x33; 64];
    let data = make_ping_bytes(&from, &token, &sig);
    check(&data, true);

    let effects = get_effects(&data);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::Ping(p) => {
            assert_eq!(p.from, from);
            assert_eq!(p.token, token);
            assert_eq!(p.signature, sig);
        }
        other => panic!("expected Ping, got {other:?}"),
    }
}

#[test]
fn test_conformance_ping_default_fields() {
    let data = make_ping_bytes(&[0u8; 32], &[0u8; 32], &[0u8; 64]);
    check(&data, true);

    let effects = get_effects(&data);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::Ping(p) => {
            assert_eq!(p.from, vec![0u8; 32]);
            assert_eq!(p.token, vec![0u8; 32]);
            assert_eq!(p.signature, vec![0u8; 64]);
        }
        other => panic!("expected Ping, got {other:?}"),
    }
}

// Pong tests

#[test]
fn test_conformance_pong_valid() {
    let from = [0x44; 32];
    let hash = [0x55; 32];
    let sig = [0x66; 64];
    let data = make_pong_bytes(&from, &hash, &sig);
    check(&data, true);

    let effects = get_effects(&data);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::Pong(p) => {
            assert_eq!(p.from, from);
            assert_eq!(p.hash, hash);
            assert_eq!(p.signature, sig);
        }
        other => panic!("expected Pong, got {other:?}"),
    }
}

#[test]
fn test_conformance_pong_default_fields() {
    let data = make_pong_bytes(&[0u8; 32], &[0u8; 32], &[0u8; 64]);
    check(&data, true);

    let effects = get_effects(&data);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::Pong(p) => {
            assert_eq!(p.from, vec![0u8; 32]);
            assert_eq!(p.hash, vec![0u8; 32]);
            assert_eq!(p.signature, vec![0u8; 64]);
        }
        other => panic!("expected Pong, got {other:?}"),
    }
}

// Prune tests

#[test]
fn test_conformance_prune_valid() {
    let pk = [0x11; 32];
    let prune_node = [0x22; 32];
    let sig = [0u8; 64];
    let dest = [0x33; 32];
    let wc = 1_000_000u64;
    let data = make_prune_bytes(&pk, &pk, &[prune_node], &sig, &dest, wc);
    check(&data, true);

    let effects = get_effects(&data);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::PruneMessage(pm) => {
            assert_eq!(pm.pubkey, pk);
            let pd = pm.data.as_ref().unwrap();
            assert_eq!(pd.pubkey, pk);
            assert_eq!(pd.prunes.len(), 1);
            assert_eq!(pd.prunes[0].as_slice(), &prune_node);
            assert_eq!(pd.destination, dest);
            assert_eq!(pd.wallclock, wc);
            assert_eq!(pd.signature.len(), 64);
        }
        other => panic!("expected PruneMessage, got {other:?}"),
    }
}

#[test]
fn test_conformance_prune_mismatched_pubkeys() {
    let data = make_prune_bytes(
        &[0xAA; 32],  // outer pubkey
        &[0xBB; 32],  // PruneData.pubkey (mismatch)
        &[],
        &[0u8; 64],
        &[0u8; 32],
        1_000_000,
    );
    check(&data, false);
}

#[test]
fn test_conformance_prune_wallclock_at_max() {
    let pk = [0x11; 32];
    let data = make_prune_bytes(&pk, &pk, &[], &[0u8; 64], &[0u8; 32], MAX_WALLCLOCK);
    check(&data, false);
}

#[test]
fn test_conformance_prune_wallclock_below_max() {
    let pk = [0x11; 32];
    let data = make_prune_bytes(&pk, &pk, &[], &[0u8; 64], &[0u8; 32], MAX_WALLCLOCK - 1);
    check(&data, true);
}

#[test]
fn test_conformance_prune_many_nodes() {
    let pk = [0x11; 32];
    let prunes: Vec<[u8; 32]> = (0..32u8).map(|i| [i; 32]).collect();
    let data = make_prune_bytes(&pk, &pk, &prunes, &[0u8; 64], &[0u8; 32], 1_000_000);
    check(&data, true);

    let effects = get_effects(&data);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::PruneMessage(pm) => {
            let pd = pm.data.as_ref().unwrap();
            assert_eq!(pd.prunes.len(), 32);
            for (i, proto_prune) in pd.prunes.iter().enumerate() {
                assert_eq!(proto_prune.as_slice(), &[i as u8; 32]);
            }
        }
        other => panic!("expected PruneMessage, got {other:?}"),
    }
}

// PullResponse tests

#[test]
fn test_conformance_pull_response_empty() {
    let pk = [0x44; 32];
    let data = make_pull_response_bytes(&pk, &[]);
    check(&data, true);

    let effects = get_effects(&data);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::PullResponse(pr) => {
            assert_eq!(pr.pubkey, pk);
            assert!(pr.values.is_empty());
        }
        other => panic!("expected PullResponse, got {other:?}"),
    }
}

#[test]
fn test_conformance_pull_response_valid() {
    let pk = [0x44; 32];
    let ci_data = make_contact_info_crds_data(&pk, 1_000_000);
    let crds_val = make_crds_value_bytes(&[0u8; 64], &ci_data);
    let data = make_pull_response_bytes(&pk, &[crds_val]);
    check(&data, true);

    let effects = get_effects(&data);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::PullResponse(pr) => {
            assert_eq!(pr.pubkey, pk);
            assert_eq!(pr.values.len(), 1);
            let crds_data = pr.values[0].data.as_ref().unwrap();
            match crds_data.data.as_ref().unwrap() {
                gossip_crds_data::Data::ContactInfo(ci) => {
                    assert_eq!(ci.pubkey, pk);
                }
                other => panic!("expected ContactInfo, got {other:?}"),
            }
        }
        other => panic!("expected PullResponse, got {other:?}"),
    }
}

// PushMessage tests

#[test]
fn test_conformance_push_message_valid() {
    let pk = [0x55; 32];
    let ci_data = make_contact_info_crds_data(&pk, 1_000_000);
    let crds_val = make_crds_value_bytes(&[0u8; 64], &ci_data);
    let data = make_push_message_bytes(&pk, &[crds_val]);
    check(&data, true);

    let effects = get_effects(&data);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::PushMessage(pm) => {
            assert_eq!(pm.pubkey, pk);
            assert_eq!(pm.values.len(), 1);
        }
        other => panic!("expected PushMessage, got {other:?}"),
    }
}

#[test]
fn test_conformance_push_message_multiple() {
    let pk = [0x55; 32];
    let ci_data = make_contact_info_crds_data(&pk, 1_000_000);
    let crds_val1 = make_crds_value_bytes(&[0u8; 64], &ci_data);
    let sh_data = make_snapshot_hashes_crds_data(&pk, 100, &[0xAA; 32], &[], 1_000_000);
    let crds_val2 = make_crds_value_bytes(&[0u8; 64], &sh_data);
    let data = make_push_message_bytes(&pk, &[crds_val1, crds_val2]);
    check(&data, true);

    let effects = get_effects(&data);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::PushMessage(pm) => {
            assert_eq!(pm.pubkey, pk);
            assert_eq!(pm.values.len(), 2);
            match pm.values[0].data.as_ref().unwrap().data.as_ref().unwrap() {
                gossip_crds_data::Data::ContactInfo(ci) => {
                    assert_eq!(ci.pubkey, pk);
                }
                other => panic!("expected ContactInfo, got {other:?}"),
            }
            match pm.values[1].data.as_ref().unwrap().data.as_ref().unwrap() {
                gossip_crds_data::Data::SnapshotHashes(sh) => {
                    assert_eq!(sh.from, pk);
                    assert_eq!(sh.full_slot, 100);
                    assert!(sh.incremental.is_empty());
                }
                other => panic!("expected SnapshotHashes, got {other:?}"),
            }
        }
        other => panic!("expected PushMessage, got {other:?}"),
    }
}

// CrdsData edge case tests

#[test]
fn test_conformance_epoch_slots_index_at_max() {
    let pk = [0x66; 32];
    let es_data = make_epoch_slots_crds_data(255, &pk, 1_000_000);
    let crds_val = make_crds_value_bytes(&[0u8; 64], &es_data);
    let data = make_push_message_bytes(&pk, &[crds_val]);
    check(&data, false);
}

#[test]
fn test_conformance_lowest_slot_nonzero_index() {
    let pk = [0x77; 32];
    let ls_data = make_lowest_slot_crds_data(1, &pk, 42, 1_000_000); // index=1 invalid
    let crds_val = make_crds_value_bytes(&[0u8; 64], &ls_data);
    let data = make_push_message_bytes(&pk, &[crds_val]);
    check(&data, false);
}

#[test]
fn test_conformance_snapshot_hashes_slot_at_max() {
    let pk = [0x88; 32];
    let sh_data =
        make_snapshot_hashes_crds_data(&pk, 1_000_000_000_000_000, &[0xAA; 32], &[], 1_000_000);
    let crds_val = make_crds_value_bytes(&[0u8; 64], &sh_data);
    let data = make_pull_response_bytes(&pk, &[crds_val]);
    check(&data, false);
}

#[test]
fn test_conformance_snapshot_hashes_incremental_not_above_full() {
    let pk = [0x99; 32];
    let sh_data = make_snapshot_hashes_crds_data(
        &pk,
        100,
        &[0xAA; 32],
        &[(50, [0xBB; 32])], // incremental slot 50 < full slot 100
        1_000_000,
    );
    let crds_val = make_crds_value_bytes(&[0u8; 64], &sh_data);
    let data = make_pull_response_bytes(&pk, &[crds_val]);
    check(&data, false);
}

// PullRequest tests

#[test]
fn test_conformance_pull_request_contact_info() {
    let pk = [0xAA; 32];
    let filter = make_crds_filter_bytes();
    let ci_data = make_contact_info_crds_data(&pk, 1_000_000);
    let crds_val = make_crds_value_bytes(&[0u8; 64], &ci_data);
    let data = make_pull_request_bytes(&filter, &crds_val);
    check(&data, true);

    let effects = get_effects(&data);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::PullRequest(pr) => {
            assert!(pr.filter.is_some());
            let crds_val = pr.value.as_ref().unwrap();
            assert_eq!(crds_val.signature.len(), 64);
            let crds_data = crds_val.data.as_ref().unwrap();
            match crds_data.data.as_ref().unwrap() {
                gossip_crds_data::Data::ContactInfo(ci) => {
                    assert_eq!(ci.pubkey, pk);
                }
                other => panic!("expected ContactInfo, got {other:?}"),
            }
        }
        other => panic!("expected PullRequest, got {other:?}"),
    }
}

#[test]
fn test_conformance_pull_request_snapshot_hashes_rejected() {
    let pk = [0xAA; 32];
    let filter = make_crds_filter_bytes();
    let sh_data = make_snapshot_hashes_crds_data(&pk, 100, &[0xBB; 32], &[], 1_000_000);
    let crds_val = make_crds_value_bytes(&[0u8; 64], &sh_data);
    let data = make_pull_request_bytes(&filter, &crds_val);
    check(&data, false);
}

#[test]
fn test_conformance_pull_request_lowest_slot_rejected() {
    let pk = [0xAA; 32];
    let filter = make_crds_filter_bytes();
    let ls_data = make_lowest_slot_crds_data(0, &pk, 42, 1_000_000);
    let crds_val = make_crds_value_bytes(&[0u8; 64], &ls_data);
    let data = make_pull_request_bytes(&filter, &crds_val);
    check(&data, false);
}

// Vote tests

#[test]
fn test_conformance_vote_index_valid() {
    let pk = [0xCC; 32];
    let vote_data = make_vote_crds_data(0, &pk, 1_000_000);
    let crds_val = make_crds_value_bytes(&[0u8; 64], &vote_data);
    let data = make_push_message_bytes(&pk, &[crds_val]);
    check(&data, true);

    let effects = get_effects(&data);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::PushMessage(pm) => {
            match pm.values[0].data.as_ref().unwrap().data.as_ref().unwrap() {
                gossip_crds_data::Data::Vote(v) => {
                    assert_eq!(v.index, 0);
                    assert!(!v.transaction.is_empty());
                }
                other => panic!("expected Vote, got {other:?}"),
            }
        }
        other => panic!("expected PushMessage, got {other:?}"),
    }
}

#[test]
fn test_conformance_vote_index_at_max() {
    let pk = [0xCC; 32];
    let vote_data = make_vote_crds_data(32, &pk, 1_000_000);
    let crds_val = make_crds_value_bytes(&[0u8; 64], &vote_data);
    let data = make_push_message_bytes(&pk, &[crds_val]);
    check(&data, false);
}

// Deserialization edge cases

#[test]
fn test_conformance_empty_input() {
    check(&[], false);
}

#[test]
fn test_conformance_truncated_input() {
    let data = make_ping_bytes(&[0x11; 32], &[0x22; 32], &[0x33; 64]);
    let truncated = &data[..data.len() / 2];
    check(truncated, false);
}

#[test]
fn test_conformance_trailing_bytes() {
    let mut data = make_ping_bytes(&[0x11; 32], &[0x22; 32], &[0x33; 64]);
    data.push(0xFF);
    check(&data, false);
}

// Serialization output tests
//
// Verify that gossip_decode_to_effects produces identical protobuf-encoded
// output bytes for known inputs. Both Firedancer and Agave should produce
// the same encoded GossipEffects for the same wire bytes.

fn encode_effects(input: &[u8]) -> Vec<u8> {
    use prost::Message;
    get_effects(input).encode_to_vec()
}

#[test]
fn test_conformance_encode_invalid() {
    // Any invalid input should produce: GossipEffects { valid: false, msg: None }
    // protobuf encoding: field 1 (valid) = false is default, so omitted = empty
    let encoded = encode_effects(&[]);
    assert!(encoded.is_empty(), "invalid input should encode to empty protobuf");

    let encoded = encode_effects(&[0xFF; 3]);
    assert!(encoded.is_empty(), "truncated input should encode to empty protobuf");
}

#[test]
fn test_conformance_encode_ping() {
    let from = [0x11; 32];
    let token = [0x22; 32];
    let sig = [0x33; 64];
    let input = make_ping_bytes(&from, &token, &sig);
    let encoded = encode_effects(&input);

    // Decode back and verify round-trip
    use prost::Message;
    let effects = GossipEffects::decode(encoded.as_slice()).unwrap();
    assert!(effects.valid);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::Ping(p) => {
            assert_eq!(p.from, from);
            assert_eq!(p.token, token);
            assert_eq!(p.signature, sig);
        }
        other => panic!("expected Ping, got {other:?}"),
    }
}

#[test]
fn test_conformance_encode_pong() {
    let from = [0x44; 32];
    let hash = [0x55; 32];
    let sig = [0x66; 64];
    let input = make_pong_bytes(&from, &hash, &sig);
    let encoded = encode_effects(&input);

    use prost::Message;
    let effects = GossipEffects::decode(encoded.as_slice()).unwrap();
    assert!(effects.valid);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::Pong(p) => {
            assert_eq!(p.from, from);
            assert_eq!(p.hash, hash);
            assert_eq!(p.signature, sig);
        }
        other => panic!("expected Pong, got {other:?}"),
    }
}

#[test]
fn test_conformance_encode_prune() {
    let pk = [0x11; 32];
    let prune_node = [0x22; 32];
    let sig = [0u8; 64];
    let dest = [0x33; 32];
    let wc = 1_000_000u64;
    let input = make_prune_bytes(&pk, &pk, &[prune_node], &sig, &dest, wc);
    let encoded = encode_effects(&input);

    use prost::Message;
    let effects = GossipEffects::decode(encoded.as_slice()).unwrap();
    assert!(effects.valid);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::PruneMessage(pm) => {
            assert_eq!(pm.pubkey, pk);
            let pd = pm.data.as_ref().unwrap();
            assert_eq!(pd.pubkey, pk);
            assert_eq!(pd.prunes.len(), 1);
            assert_eq!(pd.prunes[0].as_slice(), &prune_node);
            assert_eq!(pd.destination, dest);
            assert_eq!(pd.wallclock, wc);
        }
        other => panic!("expected PruneMessage, got {other:?}"),
    }
}

#[test]
fn test_conformance_encode_pull_response() {
    let pk = [0x44; 32];
    let ci_data = make_contact_info_crds_data(&pk, 1_000_000);
    let crds_val = make_crds_value_bytes(&[0u8; 64], &ci_data);
    let input = make_pull_response_bytes(&pk, &[crds_val]);
    let encoded = encode_effects(&input);

    use prost::Message;
    let effects = GossipEffects::decode(encoded.as_slice()).unwrap();
    assert!(effects.valid);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::PullResponse(pr) => {
            assert_eq!(pr.pubkey, pk);
            assert_eq!(pr.values.len(), 1);
        }
        other => panic!("expected PullResponse, got {other:?}"),
    }
}

#[test]
fn test_conformance_encode_push_message() {
    let pk = [0x55; 32];
    let ci_data = make_contact_info_crds_data(&pk, 1_000_000);
    let crds_val = make_crds_value_bytes(&[0u8; 64], &ci_data);
    let input = make_push_message_bytes(&pk, &[crds_val]);
    let encoded = encode_effects(&input);

    use prost::Message;
    let effects = GossipEffects::decode(encoded.as_slice()).unwrap();
    assert!(effects.valid);
    let msg = unwrap_msg(&effects);
    match msg {
        gossip_msg::Msg::PushMessage(pm) => {
            assert_eq!(pm.pubkey, pk);
            assert_eq!(pm.values.len(), 1);
        }
        other => panic!("expected PushMessage, got {other:?}"),
    }
}

#[test]
fn test_conformance_encode_deterministic() {
    // Same input must always produce identical encoded output
    let input = make_ping_bytes(&[0x11; 32], &[0x22; 32], &[0u8; 64]);
    let a = encode_effects(&input);
    let b = encode_effects(&input);
    assert_eq!(a, b, "encoding must be deterministic");
}

// Fixture generation

fn write_fixture(dir: &std::path::Path, name: &str, input: &[u8]) {
    use prost::Message;
    use protosol::protos::{FixtureMetadata, GossipFixture};

    let effects = get_effects(input);
    let fixture = GossipFixture {
        metadata: Some(FixtureMetadata {
            fn_entrypoint: String::new(),
        }),
        input: input.to_vec(),
        output: Some(effects),
    };
    let mut buf = Vec::new();
    fixture.encode(&mut buf).unwrap();
    let path = dir.join(format!("{name}.fix"));
    std::fs::write(&path, &buf)
        .unwrap_or_else(|e| panic!("Failed to write {}: {e}", path.display()));
}

/// Generate protobuf fixture files for cross-implementation conformance
/// testing.
#[test]
#[ignore]
fn generate_gossip_fixtures() {
    let dir = std::path::PathBuf::from(
        std::env::var("FIXTURE_DIR").unwrap_or_else(|_| "fixtures/gossip".into()),
    );
    std::fs::create_dir_all(&dir).unwrap();

    let pk = [0x11; 32];
    let sig = [0u8; 64];

    // Ping
    write_fixture(&dir, "ping_valid", &make_ping_bytes(&pk, &[0x22; 32], &sig));
    write_fixture(
        &dir,
        "ping_default_fields",
        &make_ping_bytes(&[0u8; 32], &[0u8; 32], &[0u8; 64]),
    );

    // Pong
    write_fixture(&dir, "pong_valid", &make_pong_bytes(&pk, &[0x55; 32], &sig));
    write_fixture(
        &dir,
        "pong_default_fields",
        &make_pong_bytes(&[0u8; 32], &[0u8; 32], &[0u8; 64]),
    );

    // Prune
    write_fixture(
        &dir,
        "prune_valid",
        &make_prune_bytes(&pk, &pk, &[[0x22; 32]], &sig, &[0x33; 32], 1_000_000),
    );
    write_fixture(
        &dir,
        "prune_mismatched_pubkeys",
        &make_prune_bytes(&[0xAA; 32], &[0xBB; 32], &[], &sig, &[0u8; 32], 1_000_000),
    );
    write_fixture(
        &dir,
        "prune_wallclock_at_max",
        &make_prune_bytes(&pk, &pk, &[], &sig, &[0u8; 32], MAX_WALLCLOCK),
    );
    write_fixture(
        &dir,
        "prune_wallclock_below_max",
        &make_prune_bytes(&pk, &pk, &[], &sig, &[0u8; 32], MAX_WALLCLOCK - 1),
    );
    {
        let prunes: Vec<[u8; 32]> = (0..32u8).map(|i| [i; 32]).collect();
        write_fixture(
            &dir,
            "prune_many_nodes",
            &make_prune_bytes(&pk, &pk, &prunes, &sig, &[0u8; 32], 1_000_000),
        );
    }

    // PullResponse
    {
        let ci = make_contact_info_crds_data(&pk, 1_000_000);
        let val = make_crds_value_bytes(&sig, &ci);
        write_fixture(&dir, "pull_response_valid", &make_pull_response_bytes(&pk, &[val]));
    }
    write_fixture(&dir, "pull_response_empty", &make_pull_response_bytes(&pk, &[]));

    // PushMessage
    {
        let ci = make_contact_info_crds_data(&pk, 1_000_000);
        let val = make_crds_value_bytes(&sig, &ci);
        write_fixture(&dir, "push_message_valid", &make_push_message_bytes(&pk, &[val]));
    }
    {
        let ci = make_contact_info_crds_data(&pk, 1_000_000);
        let sh = make_snapshot_hashes_crds_data(&pk, 100, &[0xAA; 32], &[], 1_000_000);
        let val1 = make_crds_value_bytes(&sig, &ci);
        let val2 = make_crds_value_bytes(&sig, &sh);
        write_fixture(
            &dir,
            "push_message_multiple",
            &make_push_message_bytes(&pk, &[val1, val2]),
        );
    }

    // CrdsData edge cases
    {
        let es = make_epoch_slots_crds_data(255, &pk, 1_000_000);
        let val = make_crds_value_bytes(&sig, &es);
        write_fixture(
            &dir,
            "epoch_slots_index_at_max",
            &make_push_message_bytes(&pk, &[val]),
        );
    }
    {
        let ls = make_lowest_slot_crds_data(1, &pk, 42, 1_000_000);
        let val = make_crds_value_bytes(&sig, &ls);
        write_fixture(
            &dir,
            "lowest_slot_nonzero_index",
            &make_push_message_bytes(&pk, &[val]),
        );
    }
    {
        let sh = make_snapshot_hashes_crds_data(&pk, 1_000_000_000_000_000, &[0xAA; 32], &[], 1_000_000);
        let val = make_crds_value_bytes(&sig, &sh);
        write_fixture(
            &dir,
            "snapshot_hashes_slot_at_max",
            &make_pull_response_bytes(&pk, &[val]),
        );
    }
    {
        let sh = make_snapshot_hashes_crds_data(&pk, 100, &[0xAA; 32], &[(50, [0xBB; 32])], 1_000_000);
        let val = make_crds_value_bytes(&sig, &sh);
        write_fixture(
            &dir,
            "snapshot_hashes_incremental_not_above_full",
            &make_pull_response_bytes(&pk, &[val]),
        );
    }

    // Deserialization edge cases
    write_fixture(&dir, "empty_input", &[]);
    write_fixture(
        &dir,
        "truncated_input",
        &make_ping_bytes(&pk, &[0x22; 32], &sig)[..66],
    );
    {
        let mut d = make_ping_bytes(&pk, &[0x22; 32], &sig);
        d.push(0xFF);
        write_fixture(&dir, "trailing_bytes", &d);
    }

    eprintln!(
        "Generated fixtures in {}",
        dir.canonicalize().unwrap_or(dir.clone()).display()
    );
}
