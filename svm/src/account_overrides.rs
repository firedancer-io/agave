use {
    solana_account::AccountSharedData, solana_pubkey::Pubkey, solana_sdk_ids::sysvar,
    std::collections::HashMap,
};

/// Encapsulates overridden accounts, typically used for transaction
/// simulations. Account overrides are currently not used when loading the
/// durable nonce account or when constructing the instructions sysvar account.
#[derive(Default)]
pub struct AccountOverrides {
    accounts: HashMap<Pubkey, AccountSharedData>,
}

impl AccountOverrides {
    //  FIREDANCER: This is made public for convenient use by code that provides an
    //  an interface for executing bundles.
    /// Insert or remove an account with a given pubkey to/from the list of overrides.
    pub fn set_account(&mut self, pubkey: &Pubkey, account: Option<AccountSharedData>) {
        match account {
            Some(account) => self.accounts.insert(*pubkey, account),
            None => self.accounts.remove(pubkey),
        };
    }

    /// Sets in the slot history
    ///
    /// Note: no checks are performed on the correctness of the contained data
    pub fn set_slot_history(&mut self, slot_history: Option<AccountSharedData>) {
        self.set_account(&sysvar::slot_history::id(), slot_history);
    }

    //  FIREDANCER: This is made public for convenient use by code that provides an
    //  an interface for executing bundles.
    /// Gets the account if it's found in the list of overrides
    pub fn get(&self, pubkey: &Pubkey) -> Option<&AccountSharedData> {
        self.accounts.get(pubkey)
    }
}

#[cfg(test)]
mod test {
    use {
        crate::account_overrides::AccountOverrides, solana_account::AccountSharedData,
        solana_pubkey::Pubkey, solana_sdk_ids::sysvar,
    };

    #[test]
    fn test_set_account() {
        let mut accounts = AccountOverrides::default();
        let data = AccountSharedData::default();
        let key = Pubkey::new_unique();
        accounts.set_account(&key, Some(data.clone()));
        assert_eq!(accounts.get(&key), Some(&data));

        accounts.set_account(&key, None);
        assert!(accounts.get(&key).is_none());
    }

    #[test]
    fn test_slot_history() {
        let mut accounts = AccountOverrides::default();
        let data = AccountSharedData::default();

        assert_eq!(accounts.get(&sysvar::slot_history::id()), None);
        accounts.set_slot_history(Some(data.clone()));

        assert_eq!(accounts.get(&sysvar::slot_history::id()), Some(&data));
    }
}
