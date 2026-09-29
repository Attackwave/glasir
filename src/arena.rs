//! Global string arena. File and symbol paths repeat constantly across a
//! codebase; interning them keeps nodes at a `u32` key.

use std::collections::HashMap;

#[derive(Default)]
pub struct SymbolArena {
    by_text: HashMap<String, u32>,
    text: Vec<String>,
}

impl SymbolArena {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn intern(&mut self, s: &str) -> u32 {
        if let Some(&key) = self.by_text.get(s) {
            return key;
        }
        let key = u32::try_from(self.text.len()).expect("too many interned symbols");
        let owned = s.to_owned();
        self.text.push(owned.clone());
        self.by_text.insert(owned, key);
        key
    }

    pub fn resolve(&self, key: u32) -> Option<&str> {
        self.text.get(key as usize).map(String::as_str)
    }
}
