use anyhow::{Result, ensure};
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use xxhash_rust::xxh3::{Xxh3, xxh3_64, xxh3_64_with_seed};

pub const MINHASH_SIZE: usize = 128;

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(default)]
pub struct DedupConfig {
    pub shingle_tokens: usize,
    pub bands: usize,
    pub jaccard_threshold: f64,
    /// Exact verification retains sorted shingle hashes for accepted documents.
    /// It is correctness-oriented but host-memory intensive at multi-million-file scale.
    pub exact_verification: bool,
    pub seed: u64,
}

impl Default for DedupConfig {
    fn default() -> Self {
        Self {
            shingle_tokens: 5,
            bands: 16,
            jaccard_threshold: 0.85,
            exact_verification: true,
            seed: 0x6a09_e667_f3bc_c909,
        }
    }
}

impl DedupConfig {
    fn validate(&self) -> Result<()> {
        ensure!(self.shingle_tokens > 0, "shingle_tokens must be positive");
        ensure!(self.bands > 0, "bands must be positive");
        ensure!(
            MINHASH_SIZE.is_multiple_of(self.bands),
            "bands must divide the 128-entry signature"
        );
        ensure!(
            (0.0..1.0).contains(&self.jaccard_threshold),
            "Jaccard threshold must be in [0, 1)"
        );
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct MinHashSignature {
    values: [u64; MINHASH_SIZE],
    shingles: Box<[u64]>,
}

impl MinHashSignature {
    pub fn from_code(code: &str, config: &DedupConfig) -> Self {
        let token_hashes = lexical_token_hashes(code.as_bytes());
        let mut shingles = shingle_hashes(&token_hashes, config.shingle_tokens, config.seed);
        shingles.sort_unstable();
        shingles.dedup();

        let mut values = [u64::MAX; MINHASH_SIZE];
        for &shingle in &shingles {
            for (index, minimum) in values.iter_mut().enumerate() {
                let permutation_seed = splitmix64(config.seed.wrapping_add(index as u64));
                *minimum = (*minimum).min(splitmix64(shingle ^ permutation_seed));
            }
        }
        Self {
            values,
            shingles: shingles.into_boxed_slice(),
        }
    }

    pub fn estimated_jaccard(&self, other: &Self) -> f64 {
        let equal = self
            .values
            .iter()
            .zip(other.values)
            .filter(|(left, right)| **left == *right)
            .count();
        equal as f64 / MINHASH_SIZE as f64
    }

    pub fn exact_jaccard(&self, other: &Self) -> f64 {
        let mut left = 0;
        let mut right = 0;
        let mut intersection = 0_u64;
        while left < self.shingles.len() && right < other.shingles.len() {
            match self.shingles[left].cmp(&other.shingles[right]) {
                std::cmp::Ordering::Less => left += 1,
                std::cmp::Ordering::Greater => right += 1,
                std::cmp::Ordering::Equal => {
                    intersection += 1;
                    left += 1;
                    right += 1;
                }
            }
        }
        let union = self.shingles.len() as u64 + other.shingles.len() as u64 - intersection;
        if union == 0 {
            1.0
        } else {
            intersection as f64 / union as f64
        }
    }

    fn band_key(&self, band: usize, rows: usize, seed: u64) -> u64 {
        let mut hasher = Xxh3::with_seed(seed.wrapping_add(band as u64));
        for value in &self.values[band * rows..(band + 1) * rows] {
            hasher.update(&value.to_le_bytes());
        }
        hasher.digest()
    }

    fn drop_shingles(&mut self) {
        self.shingles = Box::default();
    }
}

pub struct DedupIndex {
    config: DedupConfig,
    entries: Vec<MinHashSignature>,
    buckets: HashMap<u64, Vec<usize>>,
}

impl DedupIndex {
    pub fn new(config: DedupConfig) -> Result<Self> {
        config.validate()?;
        Ok(Self {
            config,
            entries: Vec::new(),
            buckets: HashMap::new(),
        })
    }

    pub fn config(&self) -> &DedupConfig {
        &self.config
    }

    /// Deterministically keeps the first document in input order.
    pub fn insert_if_unique(&mut self, mut signature: MinHashSignature) -> bool {
        let rows = MINHASH_SIZE / self.config.bands;
        let keys: Vec<u64> = (0..self.config.bands)
            .map(|band| signature.band_key(band, rows, self.config.seed))
            .collect();
        let mut candidates = HashSet::new();
        for key in &keys {
            if let Some(indices) = self.buckets.get(key) {
                candidates.extend(indices.iter().copied());
            }
        }
        let duplicate = candidates.into_iter().any(|index| {
            let previous = &self.entries[index];
            let similarity = if self.config.exact_verification {
                signature.exact_jaccard(previous)
            } else {
                signature.estimated_jaccard(previous)
            };
            similarity > self.config.jaccard_threshold
        });
        if duplicate {
            return false;
        }
        if !self.config.exact_verification {
            signature.drop_shingles();
        }
        let index = self.entries.len();
        self.entries.push(signature);
        for key in keys {
            self.buckets.entry(key).or_default().push(index);
        }
        true
    }

    pub fn accepted_documents(&self) -> usize {
        self.entries.len()
    }
}

fn lexical_token_hashes(code: &[u8]) -> Vec<u64> {
    let mut hashes = Vec::new();
    let mut index = 0;
    while index < code.len() {
        if code[index].is_ascii_whitespace() {
            index += 1;
            continue;
        }
        let start = index;
        if code[index].is_ascii_alphanumeric() || code[index] == b'_' {
            index += 1;
            while index < code.len() && (code[index].is_ascii_alphanumeric() || code[index] == b'_')
            {
                index += 1;
            }
        } else {
            // Punctuation remains a token; repeated operators retain their structure.
            index += 1;
        }
        hashes.push(xxh3_64(&code[start..index]));
    }
    hashes
}

fn shingle_hashes(tokens: &[u64], width: usize, seed: u64) -> Vec<u64> {
    if tokens.is_empty() {
        return vec![xxh3_64_with_seed(&[], seed)];
    }
    if tokens.len() < width {
        return vec![hash_u64_slice(tokens, seed)];
    }
    tokens
        .windows(width)
        .map(|window| hash_u64_slice(window, seed))
        .collect()
}

fn hash_u64_slice(values: &[u64], seed: u64) -> u64 {
    let mut hasher = Xxh3::with_seed(seed);
    for value in values {
        hasher.update(&value.to_le_bytes());
    }
    hasher.digest()
}

fn splitmix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn exact_duplicates_are_removed() {
        let config = DedupConfig::default();
        let signature =
            MinHashSignature::from_code("def f(x):\n    return x + 1\nprint(f(2))\n", &config);
        let mut index = DedupIndex::new(config.clone()).unwrap();
        assert!(index.insert_if_unique(signature));
        assert!(!index.insert_if_unique(MinHashSignature::from_code(
            "def f(x): return x + 1\nprint(f(2))",
            &config,
        )));
    }

    #[test]
    fn unrelated_documents_survive() {
        let config = DedupConfig::default();
        let mut index = DedupIndex::new(config.clone()).unwrap();
        assert!(index.insert_if_unique(MinHashSignature::from_code(
            "def parse(data): return [int(v) for v in data.split(',')]",
            &config,
        )));
        assert!(index.insert_if_unique(MinHashSignature::from_code(
            "class SocketPool:\n    def close(self):\n        self.connections.clear()",
            &config,
        )));
    }
}
