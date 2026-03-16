use {
    crate::client_ids::ClientId,
    serde::{Deserialize, Serialize},
    solana_sanitize::Sanitize,
    solana_serde_varint as serde_varint,
    std::{convert::TryInto, fmt},
};
#[cfg_attr(feature = "frozen-abi", derive(AbiExample))]
#[derive(Serialize, Deserialize, Clone, PartialEq, Eq)]
pub struct Version {
    #[serde(with = "serde_varint")]
    pub major: u16,
    #[serde(with = "serde_varint")]
    pub minor: u16,
    #[serde(with = "serde_varint")]
    pub patch: u16,
    pub commit: u32,      // first 4 bytes of the sha1 commit hash
    pub feature_set: u32, // first 4 bytes of the FeatureSet identifier
    #[serde(with = "serde_varint")]
    pub client: u16,
}

impl Version {
    pub fn as_semver_version(&self) -> semver::Version {
        semver::Version::new(self.major as u64, self.minor as u64, self.patch as u64)
    }

    pub fn client(&self) -> ClientId {
        ClientId::from(self.client)
    }
}

impl Default for Version {
    fn default() -> Self {
        // FIREDANCER: Report firedancer version to gossip
        unsafe extern "C" {
            static fdctl_major_version: u64;
            static fdctl_minor_version: u64;
            static fdctl_patch_version: u64;
            static fdctl_commit_ref: u32;
        }
        let feature_set =
            u32::from_le_bytes(agave_feature_set::ID.as_ref()[..4].try_into().unwrap());
        Self {
            major: unsafe { fdctl_major_version as u16 },
            minor: unsafe { fdctl_minor_version as u16 },
            patch: unsafe { fdctl_patch_version as u16 },
            commit: unsafe { fdctl_commit_ref },
            feature_set,
            client: u16::try_from(ClientId::this_client()).unwrap(),
        }
    }
}

impl fmt::Display for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}.{}", self.major, self.minor, self.patch,)
    }
}

impl fmt::Debug for Version {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "{}.{}.{} (src:{:08x}; feat:{}, client:{:?})",
            self.major,
            self.minor,
            self.patch,
            self.commit,
            self.feature_set,
            self.client(),
        )
    }
}

impl Sanitize for Version {}
