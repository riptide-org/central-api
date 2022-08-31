//! a timelocked hashmap

use std::{hash::Hash, time::Instant};

use linked_hash_map::LinkedHashMap;

/// A timed hashmap, which tracks the time items were inserted into the hashmap
#[derive(Debug, Default)]
pub struct TimedHashMap<K: Hash + Eq, V> {
    #[doc(hidden)]
    map: LinkedHashMap<K, (V, Instant)>,
}

//TODO: find a better solution than this, many functions in this hashmap are O(n) and will be TERRIBLE under load
//switching to LinkedHashMap is probably the best replacement: https://docs.rs/linked-hash-map/latest/linked_hash_map/struct.LinkedHashMap.html
impl<K: Hash + Eq, V> TimedHashMap<K, V> {
    /// Create a new, empty TimedHashMap
    pub fn new() -> Self {
        Self {
            map: Default::default(),
        }
    }

    /// Check whether the hashmap contains a key
    pub fn contains_key(&self, key: &K) -> bool {
        self.map.contains_key(key)
    }

    /// Insert a key-value pair into the hashmap
    pub fn insert(&mut self, key: K, value: V) {
        self.map.insert(key, (value, Instant::now()));
    }

    /// Remove a key-value pair from the hashmap
    pub fn remove(&mut self, key: &K) -> Option<V> {
        self.map.remove(key).map(|(v, _)| v)
    }

    /// Get all keys in the hashmap
    pub fn keys(&self) -> impl Iterator<Item = &K> {
        self.map.keys()
    }

    /// Get all values in the hashmap
    pub fn values(&self) -> impl Iterator<Item = &V> {
        self.map.values().map(|(v, _)| v)
    }

    /// Get the first element of the hashmap, with timestamp
    pub fn front_with_time(&self) -> Option<(&K, &V, &Instant)> {
        self.map.front().map(|(k, (v, i))| (k, v, i))
    }

    /// Remove the first element of the hashmap
    pub fn pop_front(&mut self) -> Option<(K, V, Instant)> {
        self.map.pop_front().map(|(k, (v, i))| (k, v, i))
    }
}
