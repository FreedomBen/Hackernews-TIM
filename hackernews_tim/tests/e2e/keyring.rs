//! Keyring mock backend for the Phase 3 e2e suite. Linux-only.
//!
//! Auth keyring tests should never reach a real OS credential manager
//! (no D-Bus session on CI, no Keychain unlock prompts, no leftover
//! state between runs). [`init_mock_keyring`] swaps in a stateful
//! in-memory credential builder so `keyring::Entry` operations succeed
//! without a backing OS keychain.
//!
//! See TEST_PLAN.md §3.1.3.
//!
//! ## Why stateful and not `keyring::mock`
//!
//! The upstream `keyring::mock::default_credential_builder` returns a
//! fresh `MockCredential` for every `Entry::new` call, which means a
//! `set_password` on one entry is **not** visible to a later
//! `Entry::new(service, user).get_password()` on a sibling entry. Code
//! paths that read and write through different `Entry` instances —
//! including [`config::migrate_auth`], which writes via `Auth::write_to_file`
//! and later reads via `Auth::from_file` — silently drop credentials
//! under that backend. The builder installed here keys credentials by
//! `(service, user)` in a process-global `HashMap`, so cross-entry
//! reads see prior writes.
//!
//! ## Scope: in-test-process only
//!
//! `keyring::set_default_credential_builder` mutates a process-global
//! `Mutex` inside the `keyring` crate. That covers the **test process**
//! (where helpers and any in-process auth code run) but does **not**
//! propagate to child processes spawned via [`super::spawn_app`] —
//! each binary the harness launches resets the builder back to
//! `keyring`'s compiled-in default. Scenarios that need to observe
//! keyring writes performed by the spawned binary (e.g.
//! `--migrate-auth`, TEST_PLAN.md §3.2.13) must drive the relevant
//! code paths from the test process directly.
//!
//! ## Concurrency
//!
//! The `keyring` crate's default-builder slot is process-global, so
//! tests that depend on the mock must run with `--test-threads=1`.
//! `make e2e` enforces this for the whole PTY suite.

#![cfg(target_os = "linux")]
#![allow(dead_code)]

use std::any::Any;
use std::collections::HashMap;
use std::sync::{Arc, Mutex, Once, OnceLock};

use ::keyring::credential::{
    Credential, CredentialApi, CredentialBuilderApi, CredentialPersistence,
};
use ::keyring::{Error as KeyringError, Result as KeyringResult};

type Store = Arc<Mutex<HashMap<(String, String), Vec<u8>>>>;

static STORE: OnceLock<Store> = OnceLock::new();

fn store() -> Store {
    STORE
        .get_or_init(|| Arc::new(Mutex::new(HashMap::new())))
        .clone()
}

#[derive(Debug)]
struct StatefulCredential {
    key: (String, String),
    store: Store,
}

impl CredentialApi for StatefulCredential {
    fn set_secret(&self, password: &[u8]) -> KeyringResult<()> {
        let mut g = self.store.lock().expect("stateful keyring store poisoned");
        g.insert(self.key.clone(), password.to_vec());
        Ok(())
    }

    fn get_secret(&self) -> KeyringResult<Vec<u8>> {
        let g = self.store.lock().expect("stateful keyring store poisoned");
        match g.get(&self.key) {
            Some(v) => Ok(v.clone()),
            None => Err(KeyringError::NoEntry),
        }
    }

    fn delete_credential(&self) -> KeyringResult<()> {
        let mut g = self.store.lock().expect("stateful keyring store poisoned");
        match g.remove(&self.key) {
            Some(_) => Ok(()),
            None => Err(KeyringError::NoEntry),
        }
    }

    fn as_any(&self) -> &dyn Any {
        self
    }
}

#[derive(Debug)]
struct StatefulBuilder {
    store: Store,
}

impl CredentialBuilderApi for StatefulBuilder {
    fn build(
        &self,
        _target: Option<&str>,
        service: &str,
        user: &str,
    ) -> KeyringResult<Box<Credential>> {
        Ok(Box::new(StatefulCredential {
            key: (service.to_string(), user.to_string()),
            store: self.store.clone(),
        }))
    }

    fn as_any(&self) -> &dyn Any {
        self
    }

    fn persistence(&self) -> CredentialPersistence {
        CredentialPersistence::ProcessOnly
    }
}

static INIT: Once = Once::new();

/// Install the stateful in-memory credential builder as the
/// process-wide default. Idempotent — repeat calls are a no-op, so
/// every test that touches keyring code can call this in its setup
/// without coordinating with siblings.
pub fn init_mock_keyring() {
    INIT.call_once(|| {
        let builder = Box::new(StatefulBuilder { store: store() });
        ::keyring::set_default_credential_builder(builder);
    });
}

/// Drop every credential the in-memory store currently holds.
/// Useful at the start of a test that wants a clean slate; the
/// store survives across tests in the same binary because
/// `keyring::set_default_credential_builder` is process-global.
pub fn clear_mock_keyring() {
    if let Some(s) = STORE.get() {
        s.lock().expect("stateful keyring store poisoned").clear();
    }
}
