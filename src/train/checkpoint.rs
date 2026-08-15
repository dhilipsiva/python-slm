//! Create-new, hash-bound trainer checkpoints and deterministic retention.

use super::trainer::{
    BackendStateArtifact, CanonicalTrainingPlan, CompletedEvaluation, TRAINER_SNAPSHOT_SCHEMA,
    TrainerIdentity, TrainerSnapshot, is_sha256, portable_relative_path, state_bundle_sha256,
};
use crate::error::{ProductError, Result};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::{Component, Path, PathBuf};
use std::sync::atomic::{AtomicU64, Ordering};

pub const CHECKPOINT_SCHEMA: &str = "python-slm-training-checkpoint-v1";
const MANIFEST_NAME: &str = "checkpoint.json";
const SEAL_NAME: &str = "SHA256SUMS";
static PARTIAL_COUNTER: AtomicU64 = AtomicU64::new(0);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointArtifactRef {
    pub role: String,
    pub path: String,
    pub bytes: u64,
    pub sha256: String,
}

#[derive(Clone, Debug, Deserialize, PartialEq, Serialize)]
#[serde(deny_unknown_fields)]
pub struct CheckpointManifestV1 {
    pub schema: String,
    pub status: String,
    pub qualification_status: String,
    pub snapshot_schema: String,
    pub identity: TrainerIdentity,
    pub plan: CanonicalTrainingPlan,
    pub consumed_targets: u64,
    pub completed_updates: u64,
    pub accumulated_targets: u64,
    pub next_training_first_target: u64,
    pub scheduler_one_based_update: u64,
    pub last_learning_rate_f32_le_hex: Option<String>,
    pub last_update_state_sha256: Option<String>,
    pub host_rng_state_hex: String,
    pub device_rng_state_hex: String,
    pub evaluations: Vec<CompletedEvaluation>,
    pub trainer_state_sha256: String,
    pub artifacts: Vec<CheckpointArtifactRef>,
    pub bundle_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PublishedCheckpoint {
    pub generation_path: PathBuf,
    pub consumed_targets: u64,
    pub manifest_sha256: String,
    pub bundle_sha256: String,
}

pub fn publish_checkpoint(root: &Path, snapshot: &TrainerSnapshot) -> Result<PublishedCheckpoint> {
    snapshot.validate()?;
    require_checkpoint_root(root, true)?;
    if snapshot.completed_updates == 0 {
        return Err(ProductError::gate(
            "P12_PREUPDATE_CHECKPOINT_REJECTED",
            "checkpoints are permitted only after a completed optimizer update",
        ));
    }

    let generations = root.join("generations");
    create_or_require_directory(&generations)?;
    let leaf = generation_leaf(snapshot.consumed_targets);
    let final_path = generations.join(&leaf);
    if final_path.exists() {
        return Err(ProductError::integrity(
            "P12_CHECKPOINT_ALREADY_EXISTS",
            "the create-new checkpoint generation already exists",
        ));
    }
    let partial = create_partial(&generations, &leaf)?;
    let result = write_checkpoint(&partial, snapshot).and_then(|manifest| {
        publish_directory(&partial, &final_path)?;
        let manifest_bytes = fs::read(final_path.join(MANIFEST_NAME)).map_err(|_| {
            ProductError::environment(
                "P12_CHECKPOINT_READ_FAILED",
                "could not reread the published checkpoint manifest",
            )
        })?;
        Ok(PublishedCheckpoint {
            generation_path: final_path.clone(),
            consumed_targets: snapshot.consumed_targets,
            manifest_sha256: sha256(&manifest_bytes),
            bundle_sha256: manifest.bundle_sha256,
        })
    });
    if result.is_err() && partial.exists() {
        let _ = fs::remove_dir_all(&partial);
    }
    result
}

fn write_checkpoint(partial: &Path, snapshot: &TrainerSnapshot) -> Result<CheckpointManifestV1> {
    let mut references = Vec::with_capacity(snapshot.backend_state.len());
    for artifact in &snapshot.backend_state {
        let path = Path::new(&artifact.relative_path);
        let destination = partial.join(path);
        if let Some(parent) = destination.parent() {
            fs::create_dir_all(parent).map_err(|_| {
                ProductError::environment(
                    "P12_CHECKPOINT_DIRECTORY_CREATE_FAILED",
                    "could not create a private checkpoint artifact directory",
                )
            })?;
        }
        write_new_sync(&destination, &artifact.bytes)?;
        references.push(CheckpointArtifactRef {
            role: artifact.role.clone(),
            path: artifact.relative_path.clone(),
            bytes: artifact.bytes.len() as u64,
            sha256: sha256(&artifact.bytes),
        });
    }
    references.sort_by(|left, right| left.role.cmp(&right.role));
    let state_sha256 = state_bundle_sha256(snapshot)?;
    let mut manifest = CheckpointManifestV1 {
        schema: CHECKPOINT_SCHEMA.to_owned(),
        status: "COMPLETE".to_owned(),
        qualification_status: "SKIPPED".to_owned(),
        snapshot_schema: TRAINER_SNAPSHOT_SCHEMA.to_owned(),
        identity: snapshot.identity.clone(),
        plan: snapshot.plan.clone(),
        consumed_targets: snapshot.consumed_targets,
        completed_updates: snapshot.completed_updates,
        accumulated_targets: snapshot.accumulated_targets,
        next_training_first_target: snapshot.next_training_first_target,
        scheduler_one_based_update: snapshot.scheduler_one_based_update,
        last_learning_rate_f32_le_hex: snapshot.last_learning_rate_f32_le_hex.clone(),
        last_update_state_sha256: snapshot.last_update_state_sha256.clone(),
        host_rng_state_hex: hex::encode(&snapshot.host_rng_state),
        device_rng_state_hex: hex::encode(&snapshot.device_rng_state),
        evaluations: snapshot.evaluations.clone(),
        trainer_state_sha256: state_sha256,
        artifacts: references,
        bundle_sha256: String::new(),
    };
    manifest.bundle_sha256 = manifest_bundle_sha256(&manifest)?;
    validate_manifest(&manifest)?;
    let manifest_bytes = compact_json_line(&manifest)?;
    write_new_sync(&partial.join(MANIFEST_NAME), &manifest_bytes)?;

    let mut seal_entries = manifest
        .artifacts
        .iter()
        .map(|reference| (reference.path.clone(), reference.sha256.clone()))
        .collect::<Vec<_>>();
    seal_entries.push((MANIFEST_NAME.to_owned(), sha256(&manifest_bytes)));
    seal_entries.sort();
    let mut seal = Vec::new();
    for (path, digest) in seal_entries {
        seal.extend_from_slice(digest.as_bytes());
        seal.extend_from_slice(b"  ");
        seal.extend_from_slice(path.as_bytes());
        seal.push(b'\n');
    }
    write_new_sync(&partial.join(SEAL_NAME), &seal)?;
    sync_directory(partial)?;
    Ok(manifest)
}

pub fn load_checkpoint(generation: &Path) -> Result<TrainerSnapshot> {
    require_checkpoint_root(generation, false)?;
    let leaf = generation
        .file_name()
        .and_then(|value| value.to_str())
        .ok_or_else(|| {
            ProductError::integrity(
                "P12_CHECKPOINT_PATH_INVALID",
                "the checkpoint generation has no portable numeric name",
            )
        })?;
    let expected_targets = parse_generation_leaf(leaf)?;
    let manifest_bytes = read_regular(generation, MANIFEST_NAME)?;
    let manifest: CheckpointManifestV1 = serde_json::from_slice(&manifest_bytes).map_err(|_| {
        ProductError::integrity(
            "P12_CHECKPOINT_MANIFEST_INVALID",
            "the checkpoint manifest is malformed or contains unknown fields",
        )
    })?;
    validate_manifest(&manifest)?;
    if manifest.consumed_targets != expected_targets {
        return Err(ProductError::integrity(
            "P12_CHECKPOINT_NAME_MISMATCH",
            "the checkpoint directory name differs from its consumed-target count",
        ));
    }
    let mut backend_state = Vec::with_capacity(manifest.artifacts.len());
    let mut expected_files = BTreeSet::from([MANIFEST_NAME.to_owned(), SEAL_NAME.to_owned()]);
    for reference in &manifest.artifacts {
        expected_files.insert(reference.path.clone());
        let bytes = read_regular(generation, &reference.path)?;
        if bytes.len() as u64 != reference.bytes || sha256(&bytes) != reference.sha256 {
            return Err(ProductError::integrity(
                "P12_CHECKPOINT_ARTIFACT_MISMATCH",
                "a checkpoint artifact differs from its bound byte length or SHA-256",
            ));
        }
        backend_state.push(BackendStateArtifact {
            role: reference.role.clone(),
            relative_path: reference.path.clone(),
            bytes,
        });
    }
    verify_inventory(generation, &expected_files)?;
    verify_seal(generation, &manifest, &manifest_bytes)?;
    let snapshot = TrainerSnapshot {
        schema: manifest.snapshot_schema,
        identity: manifest.identity,
        plan: manifest.plan,
        consumed_targets: manifest.consumed_targets,
        completed_updates: manifest.completed_updates,
        accumulated_targets: manifest.accumulated_targets,
        next_training_first_target: manifest.next_training_first_target,
        scheduler_one_based_update: manifest.scheduler_one_based_update,
        last_learning_rate_f32_le_hex: manifest.last_learning_rate_f32_le_hex,
        last_update_state_sha256: manifest.last_update_state_sha256,
        host_rng_state: decode_state(&manifest.host_rng_state_hex)?,
        device_rng_state: decode_state(&manifest.device_rng_state_hex)?,
        evaluations: manifest.evaluations,
        backend_state,
    };
    snapshot.validate()?;
    if state_bundle_sha256(&snapshot)? != manifest.trainer_state_sha256 {
        return Err(ProductError::integrity(
            "P12_CHECKPOINT_STATE_MISMATCH",
            "the reconstructed trainer state differs from its bundle identity",
        ));
    }
    Ok(snapshot)
}

fn validate_manifest(manifest: &CheckpointManifestV1) -> Result<()> {
    if manifest.schema != CHECKPOINT_SCHEMA
        || manifest.status != "COMPLETE"
        || manifest.qualification_status != "SKIPPED"
        || manifest.snapshot_schema != TRAINER_SNAPSHOT_SCHEMA
        || !is_sha256(&manifest.trainer_state_sha256)
        || !is_sha256(&manifest.bundle_sha256)
        || manifest.bundle_sha256 != manifest_bundle_sha256(manifest)?
        || manifest.host_rng_state_hex.is_empty()
        || manifest.device_rng_state_hex.is_empty()
    {
        return Err(ProductError::integrity(
            "P12_CHECKPOINT_MANIFEST_INVALID",
            "the checkpoint manifest violates its closed status, state, or bundle contract",
        ));
    }
    let mut roles = BTreeSet::new();
    let mut paths = BTreeSet::new();
    for reference in &manifest.artifacts {
        if reference.role.is_empty()
            || !roles.insert(reference.role.as_str())
            || !portable_relative_path(&reference.path)
            || matches!(reference.path.as_str(), MANIFEST_NAME | SEAL_NAME)
            || !paths.insert(reference.path.as_str())
            || reference.bytes == 0
            || !is_sha256(&reference.sha256)
        {
            return Err(ProductError::integrity(
                "P12_CHECKPOINT_ARTIFACT_REF_INVALID",
                "checkpoint artifact references are duplicated, empty, or malformed",
            ));
        }
    }
    Ok(())
}

fn manifest_bundle_sha256(manifest: &CheckpointManifestV1) -> Result<String> {
    let mut value = serde_json::to_value(manifest).map_err(|_| {
        ProductError::internal(
            "P12_CHECKPOINT_SERIALIZE_FAILED",
            "could not serialize checkpoint bundle fields",
        )
    })?;
    value
        .as_object_mut()
        .expect("serialized manifest is an object")
        .insert(
            "bundle_sha256".to_owned(),
            serde_json::Value::String(String::new()),
        );
    let bytes = serde_json::to_vec(&value).map_err(|_| {
        ProductError::internal(
            "P12_CHECKPOINT_SERIALIZE_FAILED",
            "could not serialize checkpoint bundle fields",
        )
    })?;
    Ok(sha256(&bytes))
}

fn compact_json_line<T: Serialize>(value: &T) -> Result<Vec<u8>> {
    let mut bytes = serde_json::to_vec(value).map_err(|_| {
        ProductError::internal(
            "P12_CHECKPOINT_SERIALIZE_FAILED",
            "could not serialize the checkpoint manifest",
        )
    })?;
    bytes.push(b'\n');
    Ok(bytes)
}

fn sha256(bytes: &[u8]) -> String {
    hex::encode(Sha256::digest(bytes))
}

fn decode_state(value: &str) -> Result<Vec<u8>> {
    let decoded = hex::decode(value).map_err(|_| {
        ProductError::integrity(
            "P12_CHECKPOINT_RNG_INVALID",
            "a checkpoint RNG state is not hexadecimal",
        )
    })?;
    if decoded.is_empty() {
        return Err(ProductError::integrity(
            "P12_CHECKPOINT_RNG_INVALID",
            "a checkpoint RNG state is empty",
        ));
    }
    Ok(decoded)
}

fn generation_leaf(consumed_targets: u64) -> String {
    format!("{consumed_targets:020}")
}

fn parse_generation_leaf(value: &str) -> Result<u64> {
    if value.len() != 20 || !value.bytes().all(|byte| byte.is_ascii_digit()) {
        return Err(ProductError::integrity(
            "P12_CHECKPOINT_PATH_INVALID",
            "a checkpoint generation is not a 20-digit target count",
        ));
    }
    value.parse().map_err(|_| {
        ProductError::integrity(
            "P12_CHECKPOINT_PATH_INVALID",
            "a checkpoint generation target count overflowed",
        )
    })
}

#[cfg(windows)]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink()
}

fn require_checkpoint_root(path: &Path, create: bool) -> Result<()> {
    if !path.is_absolute() {
        return Err(ProductError::usage(
            "P12_CHECKPOINT_ROOT_INVALID",
            "the checkpoint root must be an absolute path",
        ));
    }
    if create && !path.exists() {
        fs::create_dir_all(path).map_err(|_| {
            ProductError::environment(
                "P12_CHECKPOINT_ROOT_CREATE_FAILED",
                "could not create the explicit checkpoint root",
            )
        })?;
    }
    let metadata = fs::symlink_metadata(path).map_err(|_| {
        ProductError::environment(
            "P12_CHECKPOINT_ROOT_READ_FAILED",
            "could not inspect the checkpoint root",
        )
    })?;
    if !metadata.is_dir() || metadata_is_reparse(&metadata) {
        return Err(ProductError::integrity(
            "P12_CHECKPOINT_ROOT_INVALID",
            "the checkpoint root is not a regular non-reparse directory",
        ));
    }
    Ok(())
}

fn create_or_require_directory(path: &Path) -> Result<()> {
    match fs::create_dir(path) {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {
            let metadata = fs::symlink_metadata(path).map_err(|_| {
                ProductError::environment(
                    "P12_CHECKPOINT_DIRECTORY_READ_FAILED",
                    "could not inspect the checkpoint generation directory",
                )
            })?;
            if metadata.is_dir() && !metadata_is_reparse(&metadata) {
                Ok(())
            } else {
                Err(ProductError::integrity(
                    "P12_CHECKPOINT_DIRECTORY_INVALID",
                    "the checkpoint generation root is not a regular directory",
                ))
            }
        }
        Err(_) => Err(ProductError::environment(
            "P12_CHECKPOINT_DIRECTORY_CREATE_FAILED",
            "could not create the checkpoint generation root",
        )),
    }
}

fn create_partial(generations: &Path, leaf: &str) -> Result<PathBuf> {
    for _ in 0..64 {
        let sequence = PARTIAL_COUNTER.fetch_add(1, Ordering::Relaxed);
        let partial = generations.join(format!(
            ".{leaf}.p12-partial-{}-{sequence}",
            std::process::id()
        ));
        match fs::create_dir(&partial) {
            Ok(()) => return Ok(partial),
            Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => continue,
            Err(_) => {
                return Err(ProductError::environment(
                    "P12_CHECKPOINT_PARTIAL_CREATE_FAILED",
                    "could not create a private checkpoint partial generation",
                ));
            }
        }
    }
    Err(ProductError::environment(
        "P12_CHECKPOINT_PARTIAL_CREATE_FAILED",
        "could not allocate a unique checkpoint partial generation",
    ))
}

fn write_new_sync(path: &Path, bytes: &[u8]) -> Result<()> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|_| {
            ProductError::environment(
                "P12_CHECKPOINT_FILE_CREATE_FAILED",
                "could not create a checkpoint file",
            )
        })?;
    file.write_all(bytes).map_err(|_| {
        ProductError::environment(
            "P12_CHECKPOINT_FILE_WRITE_FAILED",
            "could not write a checkpoint file",
        )
    })?;
    file.sync_all().map_err(|_| {
        ProductError::environment(
            "P12_CHECKPOINT_FILE_SYNC_FAILED",
            "could not sync a checkpoint file",
        )
    })
}

fn read_regular(root: &Path, relative: &str) -> Result<Vec<u8>> {
    if !portable_relative_path(relative) {
        return Err(ProductError::integrity(
            "P12_CHECKPOINT_PATH_INVALID",
            "a checkpoint path is not portable and relative",
        ));
    }
    let path = root.join(relative);
    let metadata = fs::symlink_metadata(&path).map_err(|_| {
        ProductError::integrity(
            "P12_CHECKPOINT_FILE_MISSING",
            "a required checkpoint file is missing",
        )
    })?;
    if !metadata.is_file() || metadata_is_reparse(&metadata) {
        return Err(ProductError::integrity(
            "P12_CHECKPOINT_FILE_INVALID",
            "a checkpoint entry is not a regular non-reparse file",
        ));
    }
    fs::read(path).map_err(|_| {
        ProductError::environment(
            "P12_CHECKPOINT_READ_FAILED",
            "could not read a checkpoint file",
        )
    })
}

fn verify_inventory(root: &Path, expected: &BTreeSet<String>) -> Result<()> {
    let mut actual = BTreeSet::new();
    collect_inventory(root, root, &mut actual)?;
    if &actual != expected {
        return Err(ProductError::integrity(
            "P12_CHECKPOINT_INVENTORY_MISMATCH",
            "the checkpoint contains missing or unexpected files",
        ));
    }
    Ok(())
}

fn collect_inventory(root: &Path, directory: &Path, output: &mut BTreeSet<String>) -> Result<()> {
    for entry in fs::read_dir(directory).map_err(|_| {
        ProductError::environment(
            "P12_CHECKPOINT_INVENTORY_READ_FAILED",
            "could not enumerate the checkpoint generation",
        )
    })? {
        let entry = entry.map_err(|_| {
            ProductError::environment(
                "P12_CHECKPOINT_INVENTORY_READ_FAILED",
                "could not enumerate a checkpoint entry",
            )
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|_| {
            ProductError::environment(
                "P12_CHECKPOINT_INVENTORY_READ_FAILED",
                "could not inspect a checkpoint entry",
            )
        })?;
        if metadata_is_reparse(&metadata) {
            return Err(ProductError::integrity(
                "P12_CHECKPOINT_REPARSE_REJECTED",
                "a checkpoint contains a symbolic-link or reparse entry",
            ));
        }
        if metadata.is_dir() {
            collect_inventory(root, &path, output)?;
        } else if metadata.is_file() {
            let relative = path.strip_prefix(root).map_err(|_| {
                ProductError::internal(
                    "P12_CHECKPOINT_PATH_INVALID",
                    "a checkpoint entry escaped its generation",
                )
            })?;
            let portable = relative
                .components()
                .map(|component| match component {
                    Component::Normal(value) => value.to_str().map(str::to_owned),
                    _ => None,
                })
                .collect::<Option<Vec<_>>>()
                .ok_or_else(|| {
                    ProductError::integrity(
                        "P12_CHECKPOINT_PATH_INVALID",
                        "a checkpoint entry has a nonportable name",
                    )
                })?
                .join("/");
            output.insert(portable);
        } else {
            return Err(ProductError::integrity(
                "P12_CHECKPOINT_ENTRY_INVALID",
                "a checkpoint contains a special filesystem entry",
            ));
        }
    }
    Ok(())
}

fn verify_seal(
    generation: &Path,
    manifest: &CheckpointManifestV1,
    manifest_bytes: &[u8],
) -> Result<()> {
    let seal = read_regular(generation, SEAL_NAME)?;
    let mut expected = BTreeMap::new();
    expected.insert(MANIFEST_NAME.to_owned(), sha256(manifest_bytes));
    for reference in &manifest.artifacts {
        expected.insert(reference.path.clone(), reference.sha256.clone());
    }
    let mut actual = BTreeMap::new();
    for line in seal
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let line = std::str::from_utf8(line).map_err(|_| seal_invalid())?;
        let (digest, path) = line.split_once("  ").ok_or_else(seal_invalid)?;
        if !is_sha256(digest)
            || !portable_relative_path(path)
            || actual.insert(path.to_owned(), digest.to_owned()).is_some()
        {
            return Err(seal_invalid());
        }
    }
    if actual != expected {
        return Err(seal_invalid());
    }
    Ok(())
}

fn seal_invalid() -> ProductError {
    ProductError::integrity(
        "P12_CHECKPOINT_SEAL_INVALID",
        "the checkpoint seal is malformed or differs from the complete file inventory",
    )
}

#[cfg(not(windows))]
fn sync_directory(path: &Path) -> Result<()> {
    let directory = fs::File::open(path).map_err(|_| {
        ProductError::environment(
            "P12_CHECKPOINT_DIRECTORY_SYNC_FAILED",
            "could not open the checkpoint directory for synchronization",
        )
    })?;
    directory.sync_all().map_err(|_| {
        ProductError::environment(
            "P12_CHECKPOINT_DIRECTORY_SYNC_FAILED",
            "could not synchronize the checkpoint directory",
        )
    })
}

#[cfg(windows)]
fn sync_directory(_path: &Path) -> Result<()> {
    // Each file is flushed before publication and MoveFileExW publishes the complete
    // directory with MOVEFILE_WRITE_THROUGH; opening a directory via std::fs is unsupported.
    Ok(())
}

#[cfg(windows)]
fn publish_directory(from: &Path, to: &Path) -> Result<()> {
    use std::os::windows::ffi::OsStrExt;
    use windows_sys::Win32::Storage::FileSystem::{MOVEFILE_WRITE_THROUGH, MoveFileExW};
    let from = from
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let to = to
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    // SAFETY: both immutable UTF-16 buffers are NUL-terminated for this call.
    if unsafe { MoveFileExW(from.as_ptr(), to.as_ptr(), MOVEFILE_WRITE_THROUGH) } == 0 {
        return Err(ProductError::environment(
            "P12_CHECKPOINT_PUBLISH_FAILED",
            "could not atomically publish the checkpoint generation",
        ));
    }
    Ok(())
}

#[cfg(not(windows))]
fn publish_directory(from: &Path, to: &Path) -> Result<()> {
    fs::rename(from, to).map_err(|_| {
        ProductError::environment(
            "P12_CHECKPOINT_PUBLISH_FAILED",
            "could not atomically publish the checkpoint generation",
        )
    })
}

pub fn retained_generation_targets(generations: &[u64]) -> BTreeSet<u64> {
    let mut sorted = generations.to_vec();
    sorted.sort_unstable();
    sorted.dedup();
    let mut retained = sorted
        .iter()
        .rev()
        .take(2)
        .copied()
        .collect::<BTreeSet<_>>();
    for anchor in super::trainer::RETENTION_ANCHORS {
        if let Some(targets) = sorted.iter().copied().find(|targets| *targets >= anchor) {
            retained.insert(targets);
        }
    }
    retained
}

pub fn prune_checkpoints(root: &Path) -> Result<Vec<u64>> {
    require_checkpoint_root(root, false)?;
    let generations_root = root.join("generations");
    require_checkpoint_root(&generations_root, false)?;
    let mut generations = Vec::new();
    for entry in fs::read_dir(&generations_root).map_err(|_| {
        ProductError::environment(
            "P12_CHECKPOINT_INVENTORY_READ_FAILED",
            "could not enumerate checkpoint generations",
        )
    })? {
        let entry = entry.map_err(|_| {
            ProductError::environment(
                "P12_CHECKPOINT_INVENTORY_READ_FAILED",
                "could not enumerate a checkpoint generation",
            )
        })?;
        let name = entry.file_name().into_string().map_err(|_| {
            ProductError::integrity(
                "P12_CHECKPOINT_PATH_INVALID",
                "a checkpoint generation name is not portable UTF-8",
            )
        })?;
        if name.starts_with('.') {
            return Err(ProductError::integrity(
                "P12_CHECKPOINT_PARTIAL_PRESENT",
                "a partial checkpoint generation requires recovery before retention",
            ));
        }
        let targets = parse_generation_leaf(&name)?;
        load_checkpoint(&entry.path())?;
        generations.push(targets);
    }
    let retained = retained_generation_targets(&generations);
    for targets in generations
        .iter()
        .copied()
        .filter(|value| !retained.contains(value))
    {
        let path = generations_root.join(generation_leaf(targets));
        let canonical_parent = generations_root.canonicalize().map_err(|_| {
            ProductError::environment(
                "P12_CHECKPOINT_ROOT_READ_FAILED",
                "could not resolve the checkpoint generation root",
            )
        })?;
        let canonical_path = path.canonicalize().map_err(|_| {
            ProductError::environment(
                "P12_CHECKPOINT_ROOT_READ_FAILED",
                "could not resolve a checkpoint generation",
            )
        })?;
        if canonical_path.parent() != Some(canonical_parent.as_path()) {
            return Err(ProductError::integrity(
                "P12_CHECKPOINT_PATH_INVALID",
                "a checkpoint generation escaped the retention root",
            ));
        }
        fs::remove_dir_all(&canonical_path).map_err(|_| {
            ProductError::environment(
                "P12_CHECKPOINT_RETENTION_FAILED",
                "could not remove an expired checkpoint generation",
            )
        })?;
    }
    Ok(retained.into_iter().collect())
}
