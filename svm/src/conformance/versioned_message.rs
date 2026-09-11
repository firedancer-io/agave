use {
    protosol::protos::{
        TransactionMessage as ProtoTransactionMessage,
        TransactionVersion as ProtoTransactionVersion,
    },
    solana_hash::Hash,
    solana_message::{
        MessageHeader, VersionedMessage,
        compiled_instruction::CompiledInstruction,
        legacy,
        v0::{self, MessageAddressTableLookup},
        v1::{self, TransactionConfig},
    },
    solana_pubkey::Pubkey,
};

pub fn versioned_message_from_proto(value: &ProtoTransactionMessage) -> VersionedMessage {
    let header = value
        .header
        .map(|header| MessageHeader {
            // A valid message has at least one signature.
            // Truncate to the u8 header field *before* clamping to >= 1 (matching
            // protosol): a u32 that is a nonzero multiple of 256 truncates to 0, and
            // must still clamp up to 1 rather than staying 0 (no fee payer).
            num_required_signatures: (header.num_required_signatures as u8).max(1),
            num_readonly_signed_accounts: header.num_readonly_signed_accounts as u8,
            num_readonly_unsigned_accounts: header.num_readonly_unsigned_accounts as u8,
        })
        .unwrap_or(MessageHeader {
            num_required_signatures: 1,
            num_readonly_signed_accounts: 0,
            num_readonly_unsigned_accounts: 0,
        });
    let account_keys = value
        .account_keys
        .iter()
        .filter_map(|key| Pubkey::try_from(key.as_slice()).ok())
        .collect::<Vec<_>>();
    let recent_blockhash = <[u8; 32]>::try_from(value.recent_blockhash.as_slice())
        .map(Hash::new_from_array)
        .unwrap_or_default();
    let instructions = value
        .instructions
        .iter()
        .map(|instruction| CompiledInstruction {
            program_id_index: instruction.program_id_index as u8,
            accounts: instruction
                .accounts
                .iter()
                .map(|index| *index as u8)
                .collect(),
            data: instruction.data.clone(),
        })
        .collect::<Vec<_>>();

    let version = value.version();

    if version == ProtoTransactionVersion::V1 {
        let config = value.v1_config.as_ref();
        return VersionedMessage::V1(v1::Message {
            header,
            config: TransactionConfig {
                priority_fee: config.and_then(|c| c.priority_fee),
                compute_unit_limit: config.and_then(|c| c.compute_unit_limit),
                loaded_accounts_data_size_limit: config
                    .and_then(|c| c.loaded_accounts_data_size_limit),
                heap_size: config.and_then(|c| c.heap_size),
            },
            lifetime_specifier: recent_blockhash,
            account_keys,
            instructions,
        });
    }

    if version == ProtoTransactionVersion::Legacy {
        VersionedMessage::Legacy(legacy::Message {
            header,
            account_keys,
            recent_blockhash,
            instructions,
        })
    } else {
        let address_table_lookups = value
            .address_table_lookups
            .iter()
            .filter_map(|lookup| {
                Pubkey::try_from(lookup.account_key.as_slice())
                    .ok()
                    .map(|account_key| MessageAddressTableLookup {
                        account_key,
                        writable_indexes: lookup
                            .writable_indexes
                            .iter()
                            .map(|index| *index as u8)
                            .collect(),
                        readonly_indexes: lookup
                            .readonly_indexes
                            .iter()
                            .map(|index| *index as u8)
                            .collect(),
                    })
            })
            .collect::<Vec<_>>();
        VersionedMessage::V0(v0::Message {
            header,
            account_keys,
            recent_blockhash,
            instructions,
            address_table_lookups,
        })
    }
}

#[cfg(test)]
mod tests {
    use {
        super::versioned_message_from_proto,
        protosol::protos::{
            CompiledInstruction as ProtoCompiledInstruction, MessageHeader as ProtoMessageHeader,
            TransactionConfig as ProtoTransactionConfig,
            TransactionMessage as ProtoTransactionMessage,
            TransactionVersion as ProtoTransactionVersion,
        },
        solana_message::VersionedMessage,
    };

    fn proto_message(
        version: ProtoTransactionVersion,
        v1_config: Option<ProtoTransactionConfig>,
    ) -> ProtoTransactionMessage {
        ProtoTransactionMessage {
            version: version as i32,
            header: Some(ProtoMessageHeader {
                num_required_signatures: 1,
                num_readonly_signed_accounts: 0,
                num_readonly_unsigned_accounts: 1,
            }),
            account_keys: vec![vec![0x11; 32], vec![0x22; 32]],
            recent_blockhash: vec![0xB0; 32],
            instructions: vec![ProtoCompiledInstruction {
                program_id_index: 1,
                accounts: vec![0],
                data: vec![9],
            }],
            address_table_lookups: vec![],
            v1_config,
        }
    }

    #[test]
    fn v1_version_selects_v1() {
        let config = ProtoTransactionConfig {
            priority_fee: Some(0),
            compute_unit_limit: None,
            loaded_accounts_data_size_limit: Some(65_536),
            heap_size: Some(64 * 1024),
        };
        let VersionedMessage::V1(message) =
            versioned_message_from_proto(&proto_message(ProtoTransactionVersion::V1, Some(config)))
        else {
            panic!("expected V1");
        };
        assert_eq!(message.config.priority_fee, Some(0));
        assert_eq!(message.config.compute_unit_limit, None);
        assert_eq!(message.config.loaded_accounts_data_size_limit, Some(65_536));
        assert_eq!(message.config.heap_size, Some(64 * 1024));
        assert_eq!(message.lifetime_specifier.to_bytes(), [0xB0; 32]);
        assert_eq!(message.account_keys.len(), 2);
        assert_eq!(message.instructions[0].data, vec![9]);
        assert_eq!(message.header.num_readonly_unsigned_accounts, 1);
    }

    #[test]
    fn v1_without_config_has_empty_config() {
        let VersionedMessage::V1(message) =
            versioned_message_from_proto(&proto_message(ProtoTransactionVersion::V1, None))
        else {
            panic!("expected V1");
        };
        assert_eq!(
            message.config,
            solana_message::v1::TransactionConfig::empty()
        );
    }

    #[test]
    fn default_version_is_v0_and_ignores_stray_config() {
        let stray = Some(ProtoTransactionConfig {
            heap_size: Some(64 * 1024),
            ..Default::default()
        });
        assert!(matches!(
            versioned_message_from_proto(&proto_message(ProtoTransactionVersion::V0, stray)),
            VersionedMessage::V0(_)
        ));
        assert!(matches!(
            versioned_message_from_proto(&ProtoTransactionMessage::default()),
            VersionedMessage::V0(_)
        ));
    }

    #[test]
    fn legacy_version_selects_legacy() {
        assert!(matches!(
            versioned_message_from_proto(&proto_message(ProtoTransactionVersion::Legacy, None)),
            VersionedMessage::Legacy(_)
        ));
    }
}
