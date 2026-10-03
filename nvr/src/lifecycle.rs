//! Per-key asynchronous operation gates. The synchronous registry lock is
//! released before awaiting a gate, and expired gates are not retained.

use std::{
    collections::HashMap,
    sync::{Arc, Mutex, Weak},
};
use tokio::sync::{Mutex as AsyncMutex, OwnedMutexGuard};

#[derive(Default)]
pub(crate) struct KeyedLocks {
    locks: Mutex<HashMap<String, Weak<AsyncMutex<()>>>>,
}

impl KeyedLocks {
    pub(crate) async fn lock(&self, key: &str) -> OwnedMutexGuard<()> {
        let gate = {
            // Only registry bookkeeping runs here; no fallible user code.
            let mut locks = self.locks.lock().expect("lifecycle gate registry poisoned");
            locks.retain(|_, gate| gate.strong_count() > 0);
            match locks.get(key).and_then(Weak::upgrade) {
                Some(gate) => gate,
                None => {
                    let gate = Arc::new(AsyncMutex::new(()));
                    locks.insert(key.to_string(), Arc::downgrade(&gate));
                    gate
                }
            }
        };
        // This async gate serializes the whole operation, including teardown;
        // no synchronous mutex guard or entry-map guard is held across await.
        gate.lock_owned().await
    }
}

#[cfg(test)]
#[path = "lifecycle_test.rs"]
mod lifecycle_test;
