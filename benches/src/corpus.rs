//! Deterministic, content-pinned benchmark corpora and workload states.

use std::fs::{self, File, OpenOptions};
use std::io::{self, Read, Seek, SeekFrom, Write};
use std::path::{Path, PathBuf};

use clap::ValueEnum;
use filetime::{set_file_mtime, set_symlink_file_times, FileTime};
use serde::{Deserialize, Serialize};

use crate::manifest::{build_manifest, Manifest, ManifestError};
use crate::scratch::{OwnedScratch, ScratchError};

/// Versioned corpus definition schema.
pub const CORPUS_SCHEMA: &str = "xsync.corpus.v1";
/// Versioned generated workload descriptor schema.
pub const SCENARIO_SCHEMA: &str = "xsync.corpus.scenario.v1";
/// Full-tier one-large-file size: 10 GiB.
pub const FULL_LARGE_FILE_BYTES: u64 = 10 * 1024 * 1024 * 1024;
const REGRESSION_LARGE_FILE_BYTES: u64 = 1024 * 1024 * 1024;
const SMOKE_LARGE_FILE_BYTES: u64 = 8 * 1024 * 1024;
const BUFFER_BYTES: usize = 1024 * 1024;

/// Named deterministic corpus classes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum CorpusClass {
    /// One directory containing small regular files.
    FlatSmall,
    /// Small files spread across deterministic ten-level branches.
    DeepSmall,
    /// A large number of empty regular files.
    ZeroByteStorm,
    /// Directories, small files, empty files, and symlinks.
    Mixed,
    /// Repetitive files intended to compress well.
    Compressible,
    /// Deterministic pseudo-random files intended not to compress well.
    Incompressible,
    /// One allocated deterministic file.
    OneLargeFile,
}

/// Corpus scale tier.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "snake_case")]
#[value(rename_all = "kebab-case")]
pub enum Tier {
    /// Ordinary-CI and developer-loop scale.
    Smoke,
    /// Repeatable local regression scale.
    Regression,
    /// Explicit expensive scale, including the 10 GiB large file.
    Full,
}

/// Initial source/destination state presented to a sync implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
#[value(rename_all = "kebab-case")]
pub enum Workload {
    /// Populated source and empty destination.
    InitialCopy,
    /// Source and destination already match.
    NoOpSecondSync,
    /// Destination has the prior version; one percent of source file contents changed.
    ContentChurn,
    /// Destination has the prior version; one percent of source file metadata changed.
    MetadataOnlyChurn,
    /// Destination has files where one percent of source entries are now directories.
    TypeReplacement,
    /// Destination retains one percent of entries deleted from the source.
    Delete,
    /// Destination contains a deterministic partial copy and an interrupted staging file.
    InterruptedResume,
}

/// User-selected deterministic corpus request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CorpusRequest {
    /// Corpus class.
    pub class: CorpusClass,
    /// Scale tier.
    pub tier: Tier,
    /// Workload state.
    pub workload: Workload,
    /// Seed affecting bytes and normalized metadata.
    pub seed: u64,
    /// Optional entry-count override for tests and targeted runs.
    pub entry_count: Option<u64>,
    /// Optional one-large-file byte-size override.
    pub large_file_bytes: Option<u64>,
}

/// Fully resolved parameters recorded with a generated scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusParameters {
    /// Number of generated entries below the source root before workload mutation.
    pub entry_count: u64,
    /// Regular-file bytes for uniform compressible/incompressible corpora.
    pub file_bytes: u64,
    /// Large-file size for `one-large-file`, otherwise zero.
    pub large_file_bytes: u64,
}

/// Compact identity for a separately written independent manifest.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ManifestIdentity {
    /// Manifest schema.
    pub schema: String,
    /// BLAKE3 digest over every manifest entry field.
    pub digest: String,
    /// Count including the manifested root.
    pub item_count: u64,
    /// Sum of regular-file logical lengths.
    pub logical_bytes: u64,
    /// Path relative to the owned run root containing the full manifest.
    pub file: String,
}

/// Reproducible description of one generated sync scenario.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct CorpusScenario {
    /// Scenario descriptor schema.
    pub schema: String,
    /// Corpus generator schema.
    pub corpus_schema: String,
    /// Corpus class.
    pub class: CorpusClass,
    /// Selected tier.
    pub tier: Tier,
    /// Prepared workload.
    pub workload: Workload,
    /// Deterministic seed.
    pub seed: u64,
    /// Resolved sizing parameters.
    pub parameters: CorpusParameters,
    /// Entries changed or deleted by the selected workload.
    pub changed_entries: u64,
    /// Complete entries materialized for an interrupted/resume destination.
    pub interrupted_entries: u64,
    /// Expected final destination identity (the post-workload source).
    pub expected: ManifestIdentity,
    /// Destination identity before the sync begins.
    pub initial_destination: ManifestIdentity,
    /// Source path relative to the owned run root.
    pub source: String,
    /// Destination path relative to the owned run root.
    pub destination: String,
}

/// A generated scenario retaining ownership of its scratch run.
#[derive(Debug)]
pub struct GeneratedCorpus {
    scratch: OwnedScratch,
    scenario: CorpusScenario,
}

impl GeneratedCorpus {
    /// Marker-owned run root.
    #[must_use]
    pub fn root(&self) -> &Path {
        self.scratch.path()
    }

    /// Generated source path.
    #[must_use]
    pub fn source(&self) -> PathBuf {
        self.root().join(&self.scenario.source)
    }

    /// Prepared destination path.
    #[must_use]
    pub fn destination(&self) -> PathBuf {
        self.root().join(&self.scenario.destination)
    }

    /// Reproducible descriptor.
    #[must_use]
    pub fn scenario(&self) -> &CorpusScenario {
        &self.scenario
    }

    /// Defensively remove the owned run.
    ///
    /// # Errors
    ///
    /// Returns an error if the marker or containment checks no longer pass.
    pub fn clean(self) -> Result<(), ScratchError> {
        self.scratch.clean()
    }
}

/// Corpus generation failures.
#[derive(Debug, thiserror::Error)]
pub enum CorpusError {
    /// Scratch ownership failure.
    #[error(transparent)]
    Scratch(#[from] ScratchError),
    /// Independent manifest failure.
    #[error(transparent)]
    Manifest(#[from] ManifestError),
    /// Filesystem operation failure.
    #[error("cannot {operation} '{}': {source}", path.display())]
    Io {
        /// Short operation description.
        operation: &'static str,
        /// Failing path.
        path: PathBuf,
        /// Underlying error.
        #[source]
        source: io::Error,
    },
    /// Scenario JSON serialization failure.
    #[error("cannot serialize scenario: {0}")]
    Json(#[from] serde_json::Error),
    /// Invalid zero entry or byte override.
    #[error("{field} must be greater than zero")]
    InvalidSize {
        /// Invalid option name.
        field: &'static str,
    },
    /// A mutation was requested for a corpus without eligible entries.
    #[error("workload {workload:?} has no eligible entries in this corpus")]
    NoMutationCandidates {
        /// Requested workload.
        workload: Workload,
    },
}

/// Create a deterministic scenario below a new marker-owned scratch run.
///
/// # Errors
///
/// Returns an error when sizing is invalid, scratch allocation fails, fixture
/// bytes or metadata cannot be created, or an independent manifest cannot be
/// produced.
pub fn create_corpus(
    base: impl AsRef<Path>,
    request: &CorpusRequest,
) -> Result<GeneratedCorpus, CorpusError> {
    let parameters = resolve_parameters(request)?;
    let scratch = OwnedScratch::create(base)?;
    let source = scratch.path().join("source");
    let destination = scratch.path().join("destination");

    generate_tree(&source, request.class, &parameters, request.seed)?;
    let (changed_entries, interrupted_entries) =
        prepare_workload(&source, &destination, request, &parameters)?;
    let expected_manifest = build_manifest(&source)?;
    let destination_manifest = build_manifest(&destination)?;
    let expected_file = "source.manifest.json";
    let destination_file = "destination-initial.manifest.json";
    write_json(&scratch.path().join(expected_file), &expected_manifest)?;
    write_json(
        &scratch.path().join(destination_file),
        &destination_manifest,
    )?;

    let scenario = CorpusScenario {
        schema: SCENARIO_SCHEMA.to_owned(),
        corpus_schema: CORPUS_SCHEMA.to_owned(),
        class: request.class,
        tier: request.tier,
        workload: request.workload,
        seed: request.seed,
        parameters,
        changed_entries,
        interrupted_entries,
        expected: manifest_identity(&expected_manifest, expected_file),
        initial_destination: manifest_identity(&destination_manifest, destination_file),
        source: "source".to_owned(),
        destination: "destination".to_owned(),
    };
    write_json(&scratch.path().join("scenario.json"), &scenario)?;
    Ok(GeneratedCorpus { scratch, scenario })
}

fn resolve_parameters(request: &CorpusRequest) -> Result<CorpusParameters, CorpusError> {
    if request.entry_count == Some(0) {
        return Err(CorpusError::InvalidSize {
            field: "entry-count",
        });
    }
    if request.large_file_bytes == Some(0) {
        return Err(CorpusError::InvalidSize {
            field: "large-file-bytes",
        });
    }
    let (default_entries, file_bytes) = match (request.tier, request.class) {
        (
            Tier::Smoke,
            CorpusClass::FlatSmall | CorpusClass::DeepSmall | CorpusClass::ZeroByteStorm,
        ) => (1_000, 0),
        (
            Tier::Regression | Tier::Full,
            CorpusClass::FlatSmall | CorpusClass::DeepSmall | CorpusClass::ZeroByteStorm,
        )
        | (Tier::Full, CorpusClass::Mixed) => (100_000, 0),
        (Tier::Smoke, CorpusClass::Mixed) => (512, 0),
        (Tier::Regression, CorpusClass::Mixed) => (10_000, 0),
        (Tier::Smoke, CorpusClass::Compressible | CorpusClass::Incompressible) => (32, 64 << 10),
        (Tier::Regression, CorpusClass::Compressible | CorpusClass::Incompressible) => {
            (256, 1 << 20)
        }
        (Tier::Full, CorpusClass::Compressible | CorpusClass::Incompressible) => (1_024, 1 << 20),
        (_, CorpusClass::OneLargeFile) => (1, 0),
    };
    let large_file_bytes = if request.class == CorpusClass::OneLargeFile {
        request.large_file_bytes.unwrap_or(match request.tier {
            Tier::Smoke => SMOKE_LARGE_FILE_BYTES,
            Tier::Regression => REGRESSION_LARGE_FILE_BYTES,
            Tier::Full => FULL_LARGE_FILE_BYTES,
        })
    } else {
        0
    };
    Ok(CorpusParameters {
        entry_count: if request.class == CorpusClass::OneLargeFile {
            1
        } else {
            request.entry_count.unwrap_or(default_entries)
        },
        file_bytes,
        large_file_bytes,
    })
}

fn generate_tree(
    root: &Path,
    class: CorpusClass,
    parameters: &CorpusParameters,
    seed: u64,
) -> Result<(), CorpusError> {
    create_dir(root)?;
    match class {
        CorpusClass::FlatSmall => generate_flat(root, parameters.entry_count, seed)?,
        CorpusClass::DeepSmall => generate_deep(root, parameters.entry_count, seed)?,
        CorpusClass::ZeroByteStorm => generate_zeroes(root, parameters.entry_count)?,
        CorpusClass::Mixed => generate_mixed(root, parameters.entry_count, seed)?,
        CorpusClass::Compressible => generate_uniform(
            root,
            parameters.entry_count,
            parameters.file_bytes,
            seed,
            ContentKind::Compressible,
        )?,
        CorpusClass::Incompressible => generate_uniform(
            root,
            parameters.entry_count,
            parameters.file_bytes,
            seed,
            ContentKind::Incompressible,
        )?,
        CorpusClass::OneLargeFile => write_sized_file(
            &root.join("large.bin"),
            parameters.large_file_bytes,
            seed,
            ContentKind::Incompressible,
        )?,
    }
    normalize_tree(root, seed)
}

fn generate_flat(root: &Path, count: u64, seed: u64) -> Result<(), CorpusError> {
    for index in 0..count {
        let path = root.join(format!("file-{index:06}.dat"));
        let payload = format!("xsync-flat-v1 seed={seed:016x} item={index:020}\n");
        write_new(&path, payload.as_bytes())?;
    }
    Ok(())
}

fn generate_deep(root: &Path, count: u64, seed: u64) -> Result<(), CorpusError> {
    if count == 1 {
        return write_new(
            &root.join("file-000000.dat"),
            format!("xsync-deep-v1 seed={seed:016x} item=0\n").as_bytes(),
        );
    }
    let directory_count = (count / 100).clamp(10, 1_000).min(count - 1);
    let branch_count = directory_count.min(100).min((directory_count / 10).max(1));
    let mut leaves = vec![root.to_path_buf(); usize_from_u64(branch_count)?];
    for directory_index in 0..directory_count {
        let branch = directory_index % branch_count;
        let level = directory_index / branch_count;
        let branch_usize = usize_from_u64(branch)?;
        let next = leaves[branch_usize].join(format!("b{branch:03}-d{level:03}"));
        create_dir(&next)?;
        leaves[branch_usize] = next;
    }
    let file_count = count - directory_count;
    for index in 0..file_count {
        let branch = usize_from_u64(index % branch_count)?;
        let path = leaves[branch].join(format!("file-{index:06}.dat"));
        let payload = format!("xsync-deep-v1 seed={seed:016x} item={index:020}\n");
        write_new(&path, payload.as_bytes())?;
    }
    Ok(())
}

fn generate_zeroes(root: &Path, count: u64) -> Result<(), CorpusError> {
    for index in 0..count {
        File::create(root.join(format!("zero-{index:06}"))).map_err(|source| CorpusError::Io {
            operation: "create zero-byte file",
            path: root.join(format!("zero-{index:06}")),
            source,
        })?;
    }
    Ok(())
}

fn generate_mixed(root: &Path, count: u64, seed: u64) -> Result<(), CorpusError> {
    if count == 1 {
        return write_new(&root.join("item-000000.dat"), b"");
    }
    let directory_count = (count / 20).max(1).min(count - 1);
    let leaf_count = count - directory_count;
    let symlink_count = if cfg!(unix) { leaf_count / 20 } else { 0 };
    let regular_count = leaf_count - symlink_count;
    let mut directories = Vec::with_capacity(usize_from_u64(directory_count)?);
    for index in 0..directory_count {
        let path = root.join(format!("group-{index:04}"));
        create_dir(&path)?;
        directories.push(path);
    }
    for index in 0..regular_count {
        let directory = &directories[usize_from_u64(index % directory_count)?];
        let path = directory.join(format!("item-{index:06}.dat"));
        if index.is_multiple_of(11) {
            write_new(&path, b"")?;
        } else {
            let size = 257 + (mix64(seed ^ index) % 8_192);
            let kind = if index.is_multiple_of(2) {
                ContentKind::Compressible
            } else {
                ContentKind::Incompressible
            };
            write_sized_file(&path, size, seed ^ index, kind)?;
        }
    }
    #[cfg(unix)]
    for index in 0..symlink_count {
        use std::os::unix::fs::symlink;

        let target_index = index % regular_count.max(1);
        let target_group = target_index % directory_count;
        let target = format!("group-{target_group:04}/item-{target_index:06}.dat");
        let link = root.join(format!("link-{index:06}"));
        symlink(&target, &link).map_err(|source| CorpusError::Io {
            operation: "create symlink",
            path: link,
            source,
        })?;
    }
    Ok(())
}

fn generate_uniform(
    root: &Path,
    count: u64,
    bytes: u64,
    seed: u64,
    kind: ContentKind,
) -> Result<(), CorpusError> {
    for index in 0..count {
        write_sized_file(
            &root.join(format!("payload-{index:06}.bin")),
            bytes,
            seed ^ index,
            kind,
        )?;
    }
    Ok(())
}

fn prepare_workload(
    source: &Path,
    destination: &Path,
    request: &CorpusRequest,
    parameters: &CorpusParameters,
) -> Result<(u64, u64), CorpusError> {
    match request.workload {
        Workload::InitialCopy => {
            create_dir(destination)?;
            normalize_tree(destination, request.seed)?;
            Ok((0, 0))
        }
        Workload::NoOpSecondSync => {
            generate_tree(destination, request.class, parameters, request.seed)?;
            Ok((0, 0))
        }
        Workload::ContentChurn => {
            generate_tree(destination, request.class, parameters, request.seed)?;
            let selected = selected_regular_files(source, request.seed, request.workload)?;
            for path in &selected {
                mutate_content(path)?;
            }
            normalize_tree(source, request.seed)?;
            Ok((selected.len() as u64, 0))
        }
        Workload::MetadataOnlyChurn => {
            generate_tree(destination, request.class, parameters, request.seed)?;
            let selected = selected_regular_files(source, request.seed, request.workload)?;
            for path in &selected {
                mutate_metadata(path, request.seed)?;
            }
            Ok((selected.len() as u64, 0))
        }
        Workload::TypeReplacement => {
            generate_tree(destination, request.class, parameters, request.seed)?;
            let selected = selected_regular_files(source, request.seed, request.workload)?;
            for path in &selected {
                remove_file(path)?;
                create_dir(path)?;
            }
            normalize_tree(source, request.seed)?;
            Ok((selected.len() as u64, 0))
        }
        Workload::Delete => {
            generate_tree(destination, request.class, parameters, request.seed)?;
            let selected = selected_regular_files(source, request.seed, request.workload)?;
            for path in &selected {
                remove_file(path)?;
            }
            normalize_tree(source, request.seed)?;
            Ok((selected.len() as u64, 0))
        }
        Workload::InterruptedResume => {
            create_dir(destination)?;
            let copied = populate_interrupted_destination(source, destination, request.seed)?;
            let staging = destination.join(".xsync.tmp.interrupted-fixture");
            write_new(&staging, b"deterministic interrupted transfer\n")?;
            normalize_tree(destination, request.seed)?;
            Ok((0, copied))
        }
    }
}

fn selected_regular_files(
    root: &Path,
    seed: u64,
    workload: Workload,
) -> Result<Vec<PathBuf>, CorpusError> {
    let files = walk(root)?
        .into_iter()
        .filter(|path| {
            fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_file())
        })
        .collect::<Vec<_>>();
    if files.is_empty() {
        return Err(CorpusError::NoMutationCandidates { workload });
    }
    let target = (files.len() / 100).max(1);
    let offset = usize_from_u64(seed % files.len() as u64)?;
    let mut selected = (0..target)
        .map(|index| files[(offset + index * files.len() / target) % files.len()].clone())
        .collect::<Vec<_>>();
    selected.sort();
    selected.dedup();
    Ok(selected)
}

fn populate_interrupted_destination(
    source: &Path,
    destination: &Path,
    seed: u64,
) -> Result<u64, CorpusError> {
    let paths = walk(source)?;
    for path in paths.iter().filter(|path| {
        fs::symlink_metadata(path).is_ok_and(|metadata| metadata.file_type().is_dir())
    }) {
        let relative = path
            .strip_prefix(source)
            .expect("walk remains below source");
        if !relative.as_os_str().is_empty() {
            create_dir(&destination.join(relative))?;
        }
    }
    let leaves = paths
        .into_iter()
        .filter(|path| {
            fs::symlink_metadata(path).is_ok_and(|metadata| !metadata.file_type().is_dir())
        })
        .collect::<Vec<_>>();
    let parity = usize_from_u64(seed & 1)?;
    let mut copied = 0_u64;
    for (index, path) in leaves.iter().enumerate() {
        if index % 2 == parity {
            let relative = path
                .strip_prefix(source)
                .expect("walk remains below source");
            copy_leaf(path, &destination.join(relative))?;
            copied += 1;
        }
    }
    Ok(copied)
}

fn copy_leaf(source: &Path, destination: &Path) -> Result<(), CorpusError> {
    let metadata = fs::symlink_metadata(source).map_err(|source_error| CorpusError::Io {
        operation: "inspect interrupted source entry",
        path: source.to_path_buf(),
        source: source_error,
    })?;
    if metadata.file_type().is_symlink() {
        let target = fs::read_link(source).map_err(|source_error| CorpusError::Io {
            operation: "read interrupted source symlink",
            path: source.to_path_buf(),
            source: source_error,
        })?;
        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(target, destination).map_err(|source_error| {
                CorpusError::Io {
                    operation: "copy interrupted symlink",
                    path: destination.to_path_buf(),
                    source: source_error,
                }
            })?;
        }
        #[cfg(windows)]
        {
            std::os::windows::fs::symlink_file(target, destination).map_err(|source_error| {
                CorpusError::Io {
                    operation: "copy interrupted symlink",
                    path: destination.to_path_buf(),
                    source: source_error,
                }
            })?;
        }
    } else {
        fs::copy(source, destination).map_err(|source_error| CorpusError::Io {
            operation: "copy interrupted file",
            path: destination.to_path_buf(),
            source: source_error,
        })?;
    }
    Ok(())
}

fn mutate_content(path: &Path) -> Result<(), CorpusError> {
    let mut file = OpenOptions::new()
        .read(true)
        .write(true)
        .open(path)
        .map_err(|source| CorpusError::Io {
            operation: "open content churn file",
            path: path.to_path_buf(),
            source,
        })?;
    let mut byte = [0_u8; 1];
    if file.read(&mut byte).map_err(|source| CorpusError::Io {
        operation: "read content churn file",
        path: path.to_path_buf(),
        source,
    })? == 0
    {
        byte[0] = 0xa5;
    } else {
        byte[0] ^= 0xa5;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| CorpusError::Io {
                operation: "seek content churn file",
                path: path.to_path_buf(),
                source,
            })?;
    }
    file.write_all(&byte).map_err(|source| CorpusError::Io {
        operation: "write content churn file",
        path: path.to_path_buf(),
        source,
    })
}

fn mutate_metadata(path: &Path, seed: u64) -> Result<(), CorpusError> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;

        fs::set_permissions(path, fs::Permissions::from_mode(0o600)).map_err(|source| {
            CorpusError::Io {
                operation: "change churn mode",
                path: path.to_path_buf(),
                source,
            }
        })?;
    }
    #[cfg(windows)]
    {
        let mut permissions = fs::metadata(path)
            .map_err(|source| CorpusError::Io {
                operation: "inspect churn permissions",
                path: path.to_path_buf(),
                source,
            })?
            .permissions();
        permissions.set_readonly(true);
        fs::set_permissions(path, permissions).map_err(|source| CorpusError::Io {
            operation: "change churn permissions",
            path: path.to_path_buf(),
            source,
        })?;
    }
    let time = FileTime::from_unix_time(normalized_seconds(seed) + 60, 0);
    set_file_mtime(path, time).map_err(|source| CorpusError::Io {
        operation: "change churn mtime",
        path: path.to_path_buf(),
        source,
    })
}

fn normalize_tree(root: &Path, seed: u64) -> Result<(), CorpusError> {
    let mut paths = walk(root)?;
    paths.sort_by_key(|path| std::cmp::Reverse(path.components().count()));
    let time = FileTime::from_unix_time(normalized_seconds(seed), 0);
    for path in paths {
        let metadata = fs::symlink_metadata(&path).map_err(|source| CorpusError::Io {
            operation: "inspect generated metadata",
            path: path.clone(),
            source,
        })?;
        if metadata.file_type().is_symlink() {
            set_symlink_file_times(&path, time, time).map_err(|source| CorpusError::Io {
                operation: "normalize symlink time",
                path: path.clone(),
                source,
            })?;
            continue;
        }
        set_mode(&path, metadata.file_type().is_dir())?;
        set_file_mtime(&path, time).map_err(|source| CorpusError::Io {
            operation: "normalize mtime",
            path,
            source,
        })?;
    }
    Ok(())
}

#[cfg(unix)]
fn set_mode(path: &Path, directory: bool) -> Result<(), CorpusError> {
    use std::os::unix::fs::PermissionsExt;

    let mode = if directory { 0o755 } else { 0o644 };
    fs::set_permissions(path, fs::Permissions::from_mode(mode)).map_err(|source| CorpusError::Io {
        operation: "normalize mode",
        path: path.to_path_buf(),
        source,
    })
}

#[cfg(windows)]
fn set_mode(path: &Path, _directory: bool) -> Result<(), CorpusError> {
    let mut permissions = fs::metadata(path)
        .map_err(|source| CorpusError::Io {
            operation: "inspect generated permissions",
            path: path.to_path_buf(),
            source,
        })?
        .permissions();
    permissions.set_readonly(false);
    fs::set_permissions(path, permissions).map_err(|source| CorpusError::Io {
        operation: "normalize permissions",
        path: path.to_path_buf(),
        source,
    })
}

fn walk(root: &Path) -> Result<Vec<PathBuf>, CorpusError> {
    let mut result = Vec::new();
    let mut pending = vec![root.to_path_buf()];
    while let Some(path) = pending.pop() {
        let metadata = fs::symlink_metadata(&path).map_err(|source| CorpusError::Io {
            operation: "walk generated fixture",
            path: path.clone(),
            source,
        })?;
        result.push(path.clone());
        if metadata.file_type().is_dir() {
            let mut children = fs::read_dir(&path)
                .map_err(|source| CorpusError::Io {
                    operation: "read generated directory",
                    path: path.clone(),
                    source,
                })?
                .map(|entry| {
                    entry
                        .map(|entry| entry.path())
                        .map_err(|source| CorpusError::Io {
                            operation: "read generated directory entry",
                            path: path.clone(),
                            source,
                        })
                })
                .collect::<Result<Vec<_>, _>>()?;
            children.sort();
            pending.extend(children.into_iter().rev());
        }
    }
    Ok(result)
}

#[derive(Debug, Clone, Copy)]
enum ContentKind {
    Compressible,
    Incompressible,
}

fn write_sized_file(
    path: &Path,
    bytes: u64,
    seed: u64,
    kind: ContentKind,
) -> Result<(), CorpusError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| CorpusError::Io {
            operation: "create corpus payload",
            path: path.to_path_buf(),
            source,
        })?;
    let mut buffer = vec![0_u8; BUFFER_BYTES];
    let mut generator = DeterministicBytes::new(seed);
    let pattern = format!("xsync-compressible-v1 seed={seed:016x}\n").into_bytes();
    let mut written = 0_u64;
    while written < bytes {
        let count = usize_from_u64((bytes - written).min(BUFFER_BYTES as u64))?;
        match kind {
            ContentKind::Compressible => {
                for (index, byte) in buffer[..count].iter_mut().enumerate() {
                    *byte = pattern[(usize_from_u64(written)? + index) % pattern.len()];
                }
            }
            ContentKind::Incompressible => generator.fill(&mut buffer[..count]),
        }
        file.write_all(&buffer[..count])
            .map_err(|source| CorpusError::Io {
                operation: "write corpus payload",
                path: path.to_path_buf(),
                source,
            })?;
        written += count as u64;
    }
    Ok(())
}

fn write_new(path: &Path, bytes: &[u8]) -> Result<(), CorpusError> {
    fs::write(path, bytes).map_err(|source| CorpusError::Io {
        operation: "write generated file",
        path: path.to_path_buf(),
        source,
    })
}

fn create_dir(path: &Path) -> Result<(), CorpusError> {
    fs::create_dir(path).map_err(|source| CorpusError::Io {
        operation: "create generated directory",
        path: path.to_path_buf(),
        source,
    })
}

fn remove_file(path: &Path) -> Result<(), CorpusError> {
    fs::remove_file(path).map_err(|source| CorpusError::Io {
        operation: "remove workload file",
        path: path.to_path_buf(),
        source,
    })
}

fn write_json(path: &Path, value: &impl Serialize) -> Result<(), CorpusError> {
    let mut file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(path)
        .map_err(|source| CorpusError::Io {
            operation: "create corpus metadata",
            path: path.to_path_buf(),
            source,
        })?;
    serde_json::to_writer_pretty(&mut file, value)?;
    file.write_all(b"\n").map_err(|source| CorpusError::Io {
        operation: "write corpus metadata",
        path: path.to_path_buf(),
        source,
    })
}

fn manifest_identity(manifest: &Manifest, file: &str) -> ManifestIdentity {
    ManifestIdentity {
        schema: manifest.schema.clone(),
        digest: manifest.manifest_digest.clone(),
        item_count: manifest.item_count,
        logical_bytes: manifest.logical_bytes,
        file: file.to_owned(),
    }
}

fn normalized_seconds(seed: u64) -> i64 {
    1_700_000_000 + i64::try_from(seed % 10_000_000).expect("modulo fits i64")
}

fn usize_from_u64(value: u64) -> Result<usize, CorpusError> {
    usize::try_from(value).map_err(|_| CorpusError::InvalidSize {
        field: "entry-count or byte-count",
    })
}

fn mix64(mut value: u64) -> u64 {
    value = value.wrapping_add(0x9e37_79b9_7f4a_7c15);
    value = (value ^ (value >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
    value = (value ^ (value >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
    value ^ (value >> 31)
}

struct DeterministicBytes {
    state: u64,
}

impl DeterministicBytes {
    fn new(seed: u64) -> Self {
        Self { state: seed }
    }

    fn fill(&mut self, output: &mut [u8]) {
        for chunk in output.chunks_mut(8) {
            self.state = mix64(self.state);
            chunk.copy_from_slice(&self.state.to_le_bytes()[..chunk.len()]);
        }
    }
}

#[cfg(test)]
mod tests {
    use tempfile::tempdir;

    use crate::manifest::{build_manifest, ManifestKind};

    use super::*;

    fn request(class: CorpusClass, workload: Workload, seed: u64) -> CorpusRequest {
        CorpusRequest {
            class,
            tier: Tier::Smoke,
            workload,
            seed,
            entry_count: Some(120),
            large_file_bytes: Some(64 << 10),
        }
    }

    #[test]
    fn tier_defaults_pin_required_scales() {
        for class in [CorpusClass::FlatSmall, CorpusClass::DeepSmall] {
            for tier in [Tier::Regression, Tier::Full] {
                let mut request = request(class, Workload::InitialCopy, 0);
                request.tier = tier;
                request.entry_count = None;
                assert_eq!(resolve_parameters(&request).unwrap().entry_count, 100_000);
            }
        }
        let mut large = request(CorpusClass::OneLargeFile, Workload::InitialCopy, 0);
        large.tier = Tier::Full;
        large.large_file_bytes = None;
        assert_eq!(
            resolve_parameters(&large).unwrap().large_file_bytes,
            FULL_LARGE_FILE_BYTES
        );
        large.tier = Tier::Smoke;
        assert!(resolve_parameters(&large).unwrap().large_file_bytes < FULL_LARGE_FILE_BYTES);
    }

    #[test]
    fn all_classes_are_seed_deterministic_and_content_pinned() {
        let parent = tempdir().unwrap();
        for class in [
            CorpusClass::FlatSmall,
            CorpusClass::DeepSmall,
            CorpusClass::ZeroByteStorm,
            CorpusClass::Mixed,
            CorpusClass::Compressible,
            CorpusClass::Incompressible,
            CorpusClass::OneLargeFile,
        ] {
            let mut first_request = request(class, Workload::InitialCopy, 7);
            first_request.entry_count = Some(24);
            let first = create_corpus(parent.path(), &first_request).unwrap();
            let second = create_corpus(parent.path(), &first_request).unwrap();
            let changed = create_corpus(
                parent.path(),
                &CorpusRequest {
                    seed: 8,
                    ..first_request
                },
            )
            .unwrap();
            assert_eq!(
                first.scenario().expected.digest,
                second.scenario().expected.digest,
                "{class:?}"
            );
            assert_ne!(
                first.scenario().expected.digest,
                changed.scenario().expected.digest,
                "{class:?}"
            );
        }
    }

    #[test]
    fn default_smoke_tier_generates_every_class() {
        let parent = tempdir().unwrap();
        for class in [
            CorpusClass::FlatSmall,
            CorpusClass::DeepSmall,
            CorpusClass::ZeroByteStorm,
            CorpusClass::Mixed,
            CorpusClass::Compressible,
            CorpusClass::Incompressible,
            CorpusClass::OneLargeFile,
        ] {
            let mut smoke = request(class, Workload::InitialCopy, 9);
            smoke.entry_count = None;
            smoke.large_file_bytes = None;
            let generated = create_corpus(parent.path(), &smoke).unwrap();
            assert_eq!(generated.scenario().tier, Tier::Smoke);
            assert!(generated.scenario().expected.item_count > 1);
            assert_eq!(generated.scenario().initial_destination.item_count, 1);
        }
    }

    #[test]
    fn flat_and_deep_have_exactly_distinct_topologies() {
        let parent = tempdir().unwrap();
        let flat = create_corpus(
            parent.path(),
            &request(CorpusClass::FlatSmall, Workload::InitialCopy, 1),
        )
        .unwrap();
        let deep = create_corpus(
            parent.path(),
            &request(CorpusClass::DeepSmall, Workload::InitialCopy, 1),
        )
        .unwrap();
        let flat_manifest = build_manifest(flat.source()).unwrap();
        let deep_manifest = build_manifest(deep.source()).unwrap();
        assert_eq!(flat_manifest.item_count, 121);
        assert_eq!(deep_manifest.item_count, 121);
        assert_eq!(
            flat_manifest
                .entries
                .iter()
                .filter(|entry| entry.kind == ManifestKind::Directory)
                .count(),
            1
        );
        assert!(
            deep_manifest
                .entries
                .iter()
                .filter(|entry| entry.kind == ManifestKind::Directory)
                .count()
                > 10
        );
        assert_ne!(flat_manifest.manifest_digest, deep_manifest.manifest_digest);
    }

    #[test]
    fn workload_matrix_prepares_all_required_initial_states() {
        let parent = tempdir().unwrap();
        for workload in [
            Workload::InitialCopy,
            Workload::NoOpSecondSync,
            Workload::ContentChurn,
            Workload::MetadataOnlyChurn,
            Workload::TypeReplacement,
            Workload::Delete,
            Workload::InterruptedResume,
        ] {
            let mut workload_request = request(CorpusClass::FlatSmall, workload, 11);
            workload_request.entry_count = Some(200);
            let generated = create_corpus(parent.path(), &workload_request).unwrap();
            let scenario = generated.scenario();
            match workload {
                Workload::InitialCopy => {
                    assert_eq!(scenario.initial_destination.item_count, 1);
                }
                Workload::NoOpSecondSync => {
                    assert_eq!(
                        scenario.expected.digest,
                        scenario.initial_destination.digest
                    );
                }
                Workload::InterruptedResume => {
                    assert!(scenario.interrupted_entries > 0);
                    assert!(scenario.initial_destination.item_count > 1);
                    assert_ne!(
                        scenario.expected.digest,
                        scenario.initial_destination.digest
                    );
                }
                _ => {
                    assert_eq!(scenario.changed_entries, 2);
                    assert_ne!(
                        scenario.expected.digest,
                        scenario.initial_destination.digest
                    );
                }
            }
        }
    }

    #[test]
    fn generated_run_retains_owned_cleanup_guards() {
        let parent = tempdir().unwrap();
        let generated = create_corpus(
            parent.path(),
            &request(CorpusClass::Mixed, Workload::InitialCopy, 3),
        )
        .unwrap();
        let root = generated.root().to_path_buf();
        assert!(root.join(".xsync-bench-owned").is_file());
        assert!(root.join("scenario.json").is_file());
        generated.clean().unwrap();
        assert!(!root.exists());
    }
}
