use std::{
    fmt,
    fs::{self, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    str::FromStr,
};

use half::bf16;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use thiserror::Error;

const FIXTURE_SCHEMA: &str = "python-slm-backend-fixture-v1";
const DOMAIN: &[u8] = b"python-slm/p2/fixture/v1\0";

#[derive(Debug, Error)]
pub enum FixtureError {
    #[error("fixture I/O failed: {0}")]
    Io(#[from] std::io::Error),
    #[error("fixture JSON failed: {0}")]
    Json(#[from] serde_json::Error),
    #[error("fixture manifest is invalid: {0}")]
    Invalid(String),
}

#[derive(Clone, Copy, Debug, Deserialize, Eq, Ord, PartialEq, PartialOrd, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum Workload {
    Allocation,
    Correctness,
    Projection,
    FfnExpansion,
}

impl Workload {
    pub const ALL: [Self; 4] = [
        Self::Allocation,
        Self::Correctness,
        Self::Projection,
        Self::FfnExpansion,
    ];

    pub const fn shape(self) -> WorkloadShape {
        match self {
            Self::Allocation => WorkloadShape::Allocation([16, 2_048, 768]),
            Self::Correctness => WorkloadShape::Matmul {
                m: 17,
                k: 31,
                n: 29,
            },
            Self::Projection => WorkloadShape::Matmul {
                m: 8_192,
                k: 768,
                n: 768,
            },
            Self::FfnExpansion => WorkloadShape::Matmul {
                m: 8_192,
                k: 768,
                n: 2_432,
            },
        }
    }

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Allocation => "allocation",
            Self::Correctness => "correctness",
            Self::Projection => "projection",
            Self::FfnExpansion => "ffn-expansion",
        }
    }
}

impl fmt::Display for Workload {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.as_str())
    }
}

impl FromStr for Workload {
    type Err = String;

    fn from_str(value: &str) -> Result<Self, Self::Err> {
        match value {
            "allocation" => Ok(Self::Allocation),
            "correctness" => Ok(Self::Correctness),
            "projection" => Ok(Self::Projection),
            "ffn-expansion" => Ok(Self::FfnExpansion),
            _ => Err(format!("unsupported workload {value:?}")),
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum WorkloadShape {
    Allocation([usize; 3]),
    Matmul { m: usize, k: usize, n: usize },
}

impl WorkloadShape {
    pub const fn operand_elements(self) -> (usize, usize) {
        match self {
            Self::Allocation(shape) => (shape[0] * shape[1] * shape[2], 0),
            Self::Matmul { m, k, n } => (m * k, k * n),
        }
    }
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureManifest {
    pub schema: String,
    pub workload: Workload,
    pub generator: String,
    pub conversion: String,
    pub a: FixtureFile,
    pub b: Option<FixtureFile>,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureFile {
    pub relative_path: String,
    pub elements: u64,
    pub sha256: String,
    pub seed_sha256: String,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct FixtureHashes {
    pub algorithm: String,
    pub a_sha256: String,
    pub b_sha256: Option<String>,
    pub a_elements: u64,
    pub b_elements: u64,
}

#[derive(Clone, Debug)]
pub struct Fixture {
    pub manifest: FixtureManifest,
    pub a: Vec<bf16>,
    pub b: Vec<bf16>,
}

impl Fixture {
    pub fn hashes(&self) -> FixtureHashes {
        FixtureHashes {
            algorithm: "sha256".to_owned(),
            a_sha256: self.manifest.a.sha256.clone(),
            b_sha256: self.manifest.b.as_ref().map(|file| file.sha256.clone()),
            a_elements: self.manifest.a.elements,
            b_elements: self.manifest.b.as_ref().map_or(0, |file| file.elements),
        }
    }
}

pub fn generate_all(root: &Path) -> Result<Vec<FixtureManifest>, FixtureError> {
    fs::create_dir_all(root)?;
    Workload::ALL
        .into_iter()
        .map(|workload| generate(root, workload))
        .collect()
}

pub fn generate(root: &Path, workload: Workload) -> Result<FixtureManifest, FixtureError> {
    if !root.is_dir() {
        return Err(FixtureError::Invalid(format!(
            "fixture root {} must already be a directory",
            root.display()
        )));
    }
    let (a_elements, b_elements) = workload.shape().operand_elements();
    let a = generate_operand(workload, "a", a_elements);
    let b = (b_elements != 0).then(|| generate_operand(workload, "b", b_elements));
    let dir = root.join(workload.as_str());
    fs::create_dir(&dir)?;

    let a_file = write_operand(&dir, "a.bf16le", &a)?;
    let b_file = b
        .as_ref()
        .map(|values| write_operand(&dir, "b.bf16le", values))
        .transpose()?;
    let manifest = FixtureManifest {
        schema: FIXTURE_SCHEMA.to_owned(),
        workload,
        generator: "sha256-domain-seed+splitmix64-high-byte-v1".to_owned(),
        conversion: "signed-high-byte/128.0f32-to-bf16-rne".to_owned(),
        a: FixtureFile {
            relative_path: "a.bf16le".to_owned(),
            elements: a_elements as u64,
            sha256: sha256_bf16(&a),
            seed_sha256: seed_digest(workload, "a", a_elements),
        },
        b: b_file.map(|_| FixtureFile {
            relative_path: "b.bf16le".to_owned(),
            elements: b_elements as u64,
            sha256: sha256_bf16(b.as_ref().expect("present operand")),
            seed_sha256: seed_digest(workload, "b", b_elements),
        }),
    };
    let bytes = serde_json::to_vec_pretty(&manifest)?;
    write_new(&dir.join("manifest.json"), &bytes)?;

    // The file records are deliberately built after successful writes. Keep
    // this assertion local so a future filename change cannot desynchronize
    // the manifest from the files.
    debug_assert_eq!(a_file, dir.join("a.bf16le"));
    Ok(manifest)
}

pub fn load(root: &Path, workload: Workload) -> Result<Fixture, FixtureError> {
    let dir = root.join(workload.as_str());
    let manifest_path = dir.join("manifest.json");
    let raw = fs::read(&manifest_path)?;
    let manifest: FixtureManifest = serde_json::from_slice(&raw)?;
    if manifest.schema != FIXTURE_SCHEMA || manifest.workload != workload {
        return Err(FixtureError::Invalid(format!(
            "manifest identity mismatch in {}",
            manifest_path.display()
        )));
    }
    let (expected_a, expected_b) = workload.shape().operand_elements();
    let a = load_operand(&dir, &manifest.a, expected_a)?;
    let b = match (&manifest.b, expected_b) {
        (Some(file), elements) if elements > 0 => load_operand(&dir, file, elements)?,
        (None, 0) => Vec::new(),
        _ => {
            return Err(FixtureError::Invalid(
                "operand B presence does not match workload".to_owned(),
            ));
        }
    };
    Ok(Fixture { manifest, a, b })
}

fn generate_operand(workload: Workload, operand: &str, elements: usize) -> Vec<bf16> {
    let digest = seed_bytes(workload, operand, elements);
    let mut seed = u64::from_le_bytes(digest[..8].try_into().expect("SHA-256 prefix"));
    (0..elements)
        .map(|_| {
            let word = splitmix64(&mut seed);
            let signed = (word >> 56) as u8 as i8;
            bf16::from_f32(f32::from(signed) / 128.0)
        })
        .collect()
}

fn splitmix64(state: &mut u64) -> u64 {
    *state = state.wrapping_add(0x9e37_79b9_7f4a_7c15);
    let mut value = *state;
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

fn seed_bytes(workload: Workload, operand: &str, elements: usize) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(DOMAIN);
    hasher.update(workload.as_str().as_bytes());
    hasher.update(b"\0");
    hasher.update(operand.as_bytes());
    hasher.update(b"\0");
    hasher.update(elements.to_string().as_bytes());
    hasher.finalize().into()
}

fn seed_digest(workload: Workload, operand: &str, elements: usize) -> String {
    hex::encode(seed_bytes(workload, operand, elements))
}

fn write_operand(dir: &Path, filename: &str, values: &[bf16]) -> Result<PathBuf, FixtureError> {
    let path = dir.join(filename);
    let mut bytes = Vec::with_capacity(values.len() * 2);
    for value in values {
        bytes.extend_from_slice(&value.to_bits().to_le_bytes());
    }
    write_new(&path, &bytes)?;
    Ok(path)
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), FixtureError> {
    let mut file = OpenOptions::new().write(true).create_new(true).open(path)?;
    file.write_all(bytes)?;
    file.sync_all()?;
    Ok(())
}

fn load_operand(
    dir: &Path,
    file: &FixtureFile,
    expected_elements: usize,
) -> Result<Vec<bf16>, FixtureError> {
    let path = contained_file(dir, &file.relative_path)?;
    if file.elements != expected_elements as u64 {
        return Err(FixtureError::Invalid(format!(
            "element count mismatch for {}",
            path.display()
        )));
    }
    let expected_bytes = expected_elements
        .checked_mul(2)
        .ok_or_else(|| FixtureError::Invalid("fixture byte count overflow".to_owned()))?;
    let metadata = fs::metadata(&path)?;
    if metadata.len() != expected_bytes as u64 {
        return Err(FixtureError::Invalid(format!(
            "byte count mismatch for {}",
            path.display()
        )));
    }
    let mut raw = Vec::with_capacity(expected_bytes);
    fs::File::open(&path)?.read_to_end(&mut raw)?;
    let digest = hex::encode(Sha256::digest(&raw));
    if digest != file.sha256 {
        return Err(FixtureError::Invalid(format!(
            "SHA-256 mismatch for {}",
            path.display()
        )));
    }
    Ok(raw
        .chunks_exact(2)
        .map(|bytes| bf16::from_bits(u16::from_le_bytes([bytes[0], bytes[1]])))
        .collect())
}

fn contained_file(root: &Path, relative: &str) -> Result<PathBuf, FixtureError> {
    let relative = Path::new(relative);
    if relative.is_absolute()
        || relative
            .components()
            .any(|part| !matches!(part, std::path::Component::Normal(_)))
    {
        return Err(FixtureError::Invalid(
            "fixture path must be a single relative filename".to_owned(),
        ));
    }
    Ok(root.join(relative))
}

pub fn sha256_bf16(values: &[bf16]) -> String {
    let mut hasher = Sha256::new();
    for value in values {
        hasher.update(value.to_bits().to_le_bytes());
    }
    hex::encode(hasher.finalize())
}

pub fn sha256_bytes(values: &[u8]) -> String {
    hex::encode(Sha256::digest(values))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn splitmix64_matches_published_reference_vector() {
        let mut state = 0_u64;
        assert_eq!(splitmix64(&mut state), 0xe220_a839_7b1d_cdaf);
        assert_eq!(splitmix64(&mut state), 0x6e78_9e6a_a1b9_65f4);
    }

    #[test]
    fn fixture_generation_is_domain_separated_and_repeatable() {
        let a = generate_operand(Workload::Correctness, "a", 32);
        let again = generate_operand(Workload::Correctness, "a", 32);
        let b = generate_operand(Workload::Correctness, "b", 32);
        assert_eq!(a, again);
        assert_ne!(a, b);
        assert_eq!(sha256_bf16(&a), sha256_bf16(&again));
    }

    #[test]
    fn generated_values_are_exact_signed_byte_fractions() {
        for value in generate_operand(Workload::Correctness, "a", 1_024) {
            let scaled = value.to_f32() * 128.0;
            assert_eq!(scaled, scaled.round());
            assert!((-128.0..=127.0).contains(&scaled));
        }
    }
}
