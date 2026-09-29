//! Whole-state publication using only the standard library.
//!
//! Readers hold the lock only long enough to clone an `Arc`; work proceeds
//! after it is released, while publication replaces the whole immutable state.

use std::sync::{Arc, RwLock};

pub struct Published<T>(RwLock<Arc<T>>);

impl<T> Published<T> {
    pub fn from_pointee(value: T) -> Self {
        Self(RwLock::new(Arc::new(value)))
    }

    pub fn load(&self) -> Arc<T> {
        self.0.read().unwrap_or_else(|e| e.into_inner()).clone()
    }

    /// Compatibility name for publication sites that need an owned snapshot.
    pub fn load_full(&self) -> Arc<T> {
        self.load()
    }

    pub fn store(&self, value: Arc<T>) {
        *self.0.write().unwrap_or_else(|e| e.into_inner()) = value;
    }

    /// Publishes only when `expected` is still current, returning the state
    /// observed while holding the write lock.
    pub fn compare_and_swap(&self, expected: &Arc<T>, next: Arc<T>) -> Arc<T> {
        let mut current = self.0.write().unwrap_or_else(|e| e.into_inner());
        let previous = current.clone();
        if Arc::ptr_eq(&previous, expected) {
            *current = next;
        }
        previous
    }
}
