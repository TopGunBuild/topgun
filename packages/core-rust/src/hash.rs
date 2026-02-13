//! FNV-1a hash utilities for MerkleTree bucket routing.
//!
//! Provides a 32-bit FNV-1a hash that iterates over UTF-16 code units to match
//! the TypeScript `String.charCodeAt()` behavior, ensuring identical hashes
//! across Rust and TypeScript for cross-language MerkleTree synchronization.
