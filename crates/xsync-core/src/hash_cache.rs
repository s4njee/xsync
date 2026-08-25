//! Persistent, repairable BLAKE3 content-hash cache for `--checksum`.

use std::collections::HashMap;
use std::fs::{self, File};
use std::io::{self, Read};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Mutex;
use std::time::{SystemTime, UNIX_EPOCH};

use redb::{Database, Durability, ReadableTable, TableDefinition};

const CACHE_SCHEMA: &[u8] = b"xsync.hash-cache.v1\0";

/// Digests buffered in memory before a batched commit.
///
/// Committing per entry costs one transaction — and, at redb's default
/// durability, one fsync — for every hashed file. At roughly 4 ms per fsync that
/// dominated `--checksum` entirely: 4.3 s of wall time against 0.29 s of CPU for
/// 513 files. Buffering means a normal run commits once.
const FLUSH_THRESHOLD: usize = 4096;

/// Upper bound on the per-file read buffer.
const READ_CHUNK_BYTES: usize = 1024 * 1024;
const HASH_TABLE: TableDefinition<&[u8], &[u8]> = TableDefinition::new("file_hashes");

/// A stable cache fingerprint assembled from filesystem identity and metadata.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct HashFingerprint {
    /// Device or volume identity.
    pub device: u64,
    /// Inode or file-index identity.
    pub file: u64,
    /// File length.
    pub size: u64,
    /// Precise modification time.
    pub mtime: SystemTime,
    /// Change time, where available.
    pub ctime: Option<SystemTime>,
}

/// Persistent cache handle. Invalid databases are replaced on open.
pub struct HashCache {
    path: PathBuf,
    database: Database,
    pending: Mutex<HashMap<Vec<u8>, [u8; blake3::OUT_LEN]>>,
    hits: AtomicUsize,
    misses: AtomicUsize,
}

impl HashCache {
    /// Open or recreate a cache at `path`.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the parent or replacement database cannot be created.
    pub fn open(path: impl AsRef<Path>) -> io::Result<Self> {
        let path = path.as_ref().to_path_buf();
        if let Some(parent) = path.parent() {
            fs::create_dir_all(parent)?;
        }
        let database = if let Ok(database) = Database::create(&path) {
            database
        } else {
            let backup = path.with_extension("corrupt");
            let _ = fs::rename(&path, backup);
            Database::create(&path).map_err(redb_io)?
        };
        let cache = Self {
            path,
            database,
            pending: Mutex::new(HashMap::new()),
            hits: AtomicUsize::new(0),
            misses: AtomicUsize::new(0),
        };
        cache.ensure_schema()?;
        Ok(cache)
    }

    /// Return a cached hash or hash a stable file and publish the result.
    ///
    /// # Errors
    ///
    /// Returns an I/O error when the file cannot be read.
    pub fn hash_file(&self, path: &Path, fingerprint: HashFingerprint) -> io::Result<blake3::Hash> {
        let key = fingerprint_key(fingerprint);

        // Entries written earlier in this run are not yet committed, so the
        // in-memory buffer has to be consulted before the database.
        if let Ok(pending) = self.pending.lock() {
            if let Some(digest) = pending.get(&key) {
                self.hits.fetch_add(1, Ordering::Relaxed);
                return Ok(blake3::Hash::from(*digest));
            }
        }
        if let Ok(transaction) = self.database.begin_read() {
            if let Ok(table) = transaction.open_table(HASH_TABLE) {
                if let Ok(Some(value)) = table.get(key.as_slice()) {
                    let bytes = value.value();
                    if bytes.len() == blake3::OUT_LEN {
                        self.hits.fetch_add(1, Ordering::Relaxed);
                        let mut digest = [0; blake3::OUT_LEN];
                        digest.copy_from_slice(bytes);
                        return Ok(blake3::Hash::from(digest));
                    }
                }
            }
        }

        let mut file = File::open(path)?;
        self.misses.fetch_add(1, Ordering::Relaxed);
        let mut hasher = blake3::Hasher::new();
        // Sized from the fingerprint the caller already holds, so no extra
        // stat is issued. A fixed 1 MiB buffer costs an allocation and a
        // megabyte of page-faulting zeroes per file regardless of size, which
        // dominates a tree of small files.
        let capacity = usize::try_from(fingerprint.size)
            .unwrap_or(READ_CHUNK_BYTES)
            .clamp(4096, READ_CHUNK_BYTES);
        let mut buffer = vec![0_u8; capacity];
        loop {
            let read = file.read(&mut buffer)?;
            if read == 0 {
                break;
            }
            hasher.update(&buffer[..read]);
        }
        let digest = hasher.finalize();

        let full = if let Ok(mut pending) = self.pending.lock() {
            pending.insert(key, *digest.as_bytes());
            pending.len() >= FLUSH_THRESHOLD
        } else {
            false
        };
        if full {
            // A failed flush loses cached work but never a result: the digest
            // was computed from the file and is returned regardless.
            let _ = self.flush();
        }
        Ok(digest)
    }

    /// Return cache hits and misses observed by this handle.
    #[must_use]
    pub fn stats(&self) -> (usize, usize) {
        (
            self.hits.load(Ordering::Relaxed),
            self.misses.load(Ordering::Relaxed),
        )
    }

    /// Commit every buffered digest in a single transaction.
    ///
    /// Called automatically when the buffer fills and again on drop, so callers
    /// do not normally need it. The cache is rebuildable, so this commits at
    /// [`Durability::Eventual`]: it does not block on an fsync, while still
    /// letting redb free pages. `Durability::None` is deliberately not used —
    /// redb only frees pages at higher durability levels, so using it
    /// exclusively grows the database file without bound.
    ///
    /// # Errors
    ///
    /// Returns an I/O error if the transaction cannot be opened or committed.
    pub fn flush(&self) -> io::Result<()> {
        let entries = match self.pending.lock() {
            Ok(mut pending) if !pending.is_empty() => std::mem::take(&mut *pending),
            _ => return Ok(()),
        };
        let mut transaction = self.database.begin_write().map_err(redb_io)?;
        transaction.set_durability(Durability::Eventual);
        {
            let mut table = transaction.open_table(HASH_TABLE).map_err(redb_io)?;
            for (key, digest) in &entries {
                table.insert(key.as_slice(), &digest[..]).map_err(redb_io)?;
            }
        }
        transaction.commit().map_err(redb_io)
    }

    /// Location used by the default local cache.
    pub fn default_path() -> PathBuf {
        std::env::var_os("XDG_CACHE_HOME")
            .map(PathBuf::from)
            .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".cache")))
            .unwrap_or_else(std::env::temp_dir)
            .join("xsync")
            .join("hashes.redb")
    }

    fn ensure_schema(&self) -> io::Result<()> {
        let transaction = self.database.begin_write().map_err(redb_io)?;
        {
            let mut table = transaction.open_table(HASH_TABLE).map_err(redb_io)?;
            let key = b"__schema__";
            let has_schema = table.get(key.as_slice()).map_err(redb_io)?.is_some();
            if !has_schema {
                table
                    .insert(key.as_slice(), CACHE_SCHEMA)
                    .map_err(redb_io)?;
            }
        }
        transaction.commit().map_err(redb_io)
    }

    /// Expose the backing path for diagnostics and tests.
    #[must_use]
    pub fn path(&self) -> &Path {
        &self.path
    }
}

impl Drop for HashCache {
    fn drop(&mut self) {
        // Best effort: an unwritten cache costs the next run a rehash, never
        // correctness.
        let _ = self.flush();
    }
}

fn fingerprint_key(fingerprint: HashFingerprint) -> Vec<u8> {
    let mut key = Vec::with_capacity(8 * 7 + CACHE_SCHEMA.len());
    key.extend_from_slice(CACHE_SCHEMA);
    key.extend_from_slice(&fingerprint.device.to_le_bytes());
    key.extend_from_slice(&fingerprint.file.to_le_bytes());
    key.extend_from_slice(&fingerprint.size.to_le_bytes());
    append_time(&mut key, fingerprint.mtime);
    match fingerprint.ctime {
        Some(time) => {
            key.push(1);
            append_time(&mut key, time);
        }
        None => key.push(0),
    }
    key
}

fn append_time(output: &mut Vec<u8>, time: SystemTime) {
    match time.duration_since(UNIX_EPOCH) {
        Ok(duration) => {
            output.push(1);
            output.extend_from_slice(&duration.as_secs().to_le_bytes());
            output.extend_from_slice(&duration.subsec_nanos().to_le_bytes());
        }
        Err(error) => {
            output.push(0);
            let duration = error.duration();
            output.extend_from_slice(&duration.as_secs().to_le_bytes());
            output.extend_from_slice(&duration.subsec_nanos().to_le_bytes());
        }
    }
}

fn redb_io(error: impl std::fmt::Display) -> io::Error {
    io::Error::other(error.to_string())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tempfile::tempdir;

    #[test]
    fn buffered_digests_survive_drop_and_reopen() {
        // Digests are buffered in memory and committed in batches, so the flush
        // on drop is the only thing that makes them durable. If it regresses,
        // every run silently rehashes and the cache is dead weight.
        let directory = tempdir().unwrap();
        let database = directory.path().join("hashes.redb");
        let mut fingerprints = Vec::new();
        for index in 0..8_u64 {
            let path = directory.path().join(format!("file-{index}"));
            fs::write(&path, format!("contents {index}").as_bytes()).unwrap();
            fingerprints.push((
                path,
                HashFingerprint {
                    device: 1,
                    file: index,
                    size: index,
                    mtime: UNIX_EPOCH,
                    ctime: Some(UNIX_EPOCH),
                },
            ));
        }

        let expected: Vec<blake3::Hash> = {
            let cache = HashCache::open(&database).unwrap();
            let expected = fingerprints
                .iter()
                .map(|(path, fingerprint)| cache.hash_file(path, *fingerprint).unwrap())
                .collect();
            assert_eq!(cache.stats(), (0, fingerprints.len()));
            expected
            // cache drops here, which must flush
        };

        // Remove the sources: a cache miss now fails to open the file, so a
        // returned digest proves it came from the committed database.
        for (path, _) in &fingerprints {
            fs::remove_file(path).unwrap();
        }

        let reopened = HashCache::open(&database).unwrap();
        for ((path, fingerprint), want) in fingerprints.iter().zip(&expected) {
            assert_eq!(
                reopened.hash_file(path, *fingerprint).unwrap(),
                *want,
                "digest for {} was not persisted",
                path.display()
            );
        }
        assert_eq!(reopened.stats(), (fingerprints.len(), 0));
    }

    #[test]
    fn cache_round_trip_and_fingerprint_invalidation() {
        let directory = tempdir().unwrap();
        let path = directory.path().join("file");
        fs::write(&path, b"one").unwrap();
        let fingerprint = HashFingerprint {
            device: 1,
            file: 2,
            size: 3,
            mtime: UNIX_EPOCH,
            ctime: Some(UNIX_EPOCH),
        };
        let cache = HashCache::open(directory.path().join("hashes.redb")).unwrap();
        assert_eq!(
            cache.hash_file(&path, fingerprint).unwrap(),
            blake3::hash(b"one")
        );
        fs::write(&path, b"two").unwrap();
        let changed = HashFingerprint {
            ctime: Some(UNIX_EPOCH + std::time::Duration::from_secs(1)),
            ..fingerprint
        };
        assert_eq!(
            cache.hash_file(&path, changed).unwrap(),
            blake3::hash(b"two")
        );
    }
}
