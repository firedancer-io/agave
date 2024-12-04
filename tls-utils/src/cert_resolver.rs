use {
    rustls::{server::ResolvesServerCert, client::ResolvesClientCert, sign::CertifiedKey},
    solana_keypair::Keypair,
    std::sync::Arc,
};

#[derive(Debug)]
pub struct ResolvePublicKey {
    pub x509_key: Arc<CertifiedKey>,
    pub raw_key: Arc<CertifiedKey>,
    pub force_client_rpk: bool,
}

impl ResolvePublicKey {
    pub fn from_keypair(keypair: &Keypair) -> Self {
        let key_provider = rustls::crypto::ring::default_provider().key_provider;
        let (x509_cert, private_key) = crate::tls_certificates::new_dummy_x509_certificate(keypair);
        let rpk_cert = crate::tls_certificates::new_raw_public_key(keypair);
        let sign_key = key_provider.load_private_key(private_key).unwrap();
        Self {
            x509_key: Arc::new(CertifiedKey::new(vec![x509_cert], Arc::clone(&sign_key))),
            raw_key: Arc::new(CertifiedKey::new(vec![rpk_cert], sign_key)),
            force_client_rpk: false,
        }
    }
}

impl ResolvesServerCert for ResolvePublicKey {
    fn resolve(&self, client_hello: rustls::server::ClientHello<'_>) -> Option<Arc<CertifiedKey>> {
        let use_raw_pubkey = if let Some(client_cert_types) = client_hello.client_cert_types() {
            client_cert_types
                .iter()
                .find(|ct| u8::from(**ct) == 0x02)
                .is_some()
        } else {
            false
        };
        if use_raw_pubkey {
            Some(Arc::clone(&self.raw_key))
        } else {
            Some(Arc::clone(&self.x509_key))
        }
    }

    fn only_raw_public_keys(&self) -> bool {
        false
    }
}

impl ResolvesClientCert for ResolvePublicKey {
    fn has_certs(&self) -> bool {
        true
    }

    fn only_raw_public_keys(&self) -> bool {
        false
    }

    fn resolve(
        &self,
        _root_hint_subjects: &[&[u8]],
        _sigschemes: &[rustls::SignatureScheme],
    ) -> Option<Arc<rustls::sign::CertifiedKey>> {
        if self.force_client_rpk {
            Some(Arc::clone(&self.raw_key))
        } else {
            // For now, always provide the legacy X.509 certificate by default
            Some(Arc::clone(&self.x509_key))
        }
    }
}
