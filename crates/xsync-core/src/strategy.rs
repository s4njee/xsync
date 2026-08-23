//! Size-adaptive transfer work creation and bounded dispatch.
//!
//! Files are converted to metadata-only work items. Local small and medium
//! work can use one shared bounded queue, while large-file ranges retain
//! stable stream ownership for transports that require ordering.

use std::mem;
use std::sync::Arc;

use crossbeam_channel::{bounded, Receiver, Sender};

use crate::scanner::{EntryKind, FileEntry};

/// One mebibyte, the exclusive upper bound for small files.
pub const SMALL_FILE_LIMIT: u64 = 1024 * 1024;
/// Thirty-two mebibytes, the inclusive upper bound for whole-file work.
pub const WHOLE_FILE_LIMIT: u64 = 32 * 1024 * 1024;
/// Target data size for one small-file batch.
pub const BATCH_TARGET_SIZE: u64 = 32 * 1024 * 1024;
/// Maximum files in one batch, bounding metadata even for empty files.
pub const MAX_BATCH_FILES: usize = 8_192;
/// Data size of one chunk from a huge file.
pub const CHUNK_SIZE: u64 = 16 * 1024 * 1024;
/// Default pending work items per worker queue.
pub const DEFAULT_QUEUE_CAPACITY: usize = 2;

/// Calibratable logical strategy thresholds.
///
/// These values describe logical work, not protocol frame sizes. A transport
/// may split a logical batch or chunk into any number of bounded wire frames
/// without changing membership or stream ownership.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StrategyConfig {
    /// Exclusive upper bound for small files.
    pub small_file_limit: u64,
    /// Inclusive upper bound for whole-file work.
    pub whole_file_limit: u64,
    /// Target logical bytes in one small-file batch.
    pub batch_target_size: u64,
    /// Maximum entries in one small-file batch.
    pub max_batch_files: usize,
    /// Logical bytes in one large-file range.
    pub chunk_size: u64,
}

impl Default for StrategyConfig {
    fn default() -> Self {
        Self {
            small_file_limit: SMALL_FILE_LIMIT,
            whole_file_limit: WHOLE_FILE_LIMIT,
            batch_target_size: BATCH_TARGET_SIZE,
            max_batch_files: MAX_BATCH_FILES,
            chunk_size: CHUNK_SIZE,
        }
    }
}

impl StrategyConfig {
    fn validate(self) -> Result<Self, DispatchError> {
        if self.small_file_limit == 0 {
            return Err(DispatchError::InvalidStrategyConfig {
                field: "small_file_limit",
            });
        }
        if self.whole_file_limit < self.small_file_limit {
            return Err(DispatchError::InvalidStrategyConfig {
                field: "whole_file_limit",
            });
        }
        if self.batch_target_size == 0 {
            return Err(DispatchError::InvalidStrategyConfig {
                field: "batch_target_size",
            });
        }
        if self.max_batch_files == 0 {
            return Err(DispatchError::InvalidStrategyConfig {
                field: "max_batch_files",
            });
        }
        if self.chunk_size == 0 {
            return Err(DispatchError::InvalidStrategyConfig {
                field: "chunk_size",
            });
        }
        Ok(self)
    }
}

/// A coalesced group of small files.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SmallBatch {
    /// Files included in this batch.
    pub files: Vec<FileEntry>,
    /// Sum of the files' data sizes.
    pub total_size: u64,
}

/// One disjoint range of a huge file.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChunk {
    /// Shared metadata for the file containing this range.
    pub file: Arc<FileEntry>,
    /// Byte offset where this range begins.
    pub offset: u64,
    /// Number of bytes in this range.
    pub length: u64,
}

/// A unit of transfer work consumed by one worker.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorkItem {
    /// A coalesced small-file batch.
    SmallBatch(SmallBatch),
    /// A medium file transferred as one message.
    WholeFile(FileEntry),
    /// A disjoint chunk of a huge file.
    Chunk(FileChunk),
}

/// Observable counts produced while assigning work.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct DispatchStats {
    /// Number of small-file batch work items.
    pub batches: usize,
    /// Number of files contained across all small-file batches.
    pub batched_files: usize,
    /// Number of whole-file work items.
    pub whole_files: usize,
    /// Number of huge-file chunk work items.
    pub chunks: usize,
}

/// Errors produced while configuring or running work dispatch.
#[derive(Debug, thiserror::Error, PartialEq, Eq)]
pub enum DispatchError {
    /// At least one worker is required.
    #[error("work dispatcher requires at least one worker")]
    ZeroWorkers,
    /// Queue capacity must permit backpressure without being a rendezvous.
    #[error("worker queue capacity must be at least 1")]
    ZeroQueueCapacity,
    /// Only regular files can be assigned to transfer strategies.
    #[error("cannot dispatch non-file entry '{path}' ({kind:?})")]
    NonFile {
        /// Protocol-canonical entry path.
        path: String,
        /// Actual entry kind.
        kind: EntryKind,
    },
    /// A worker stopped receiving before dispatch completed.
    #[error("worker {worker_id} disconnected during dispatch")]
    WorkerDisconnected {
        /// Zero-based worker identifier, matching the queue's vector index.
        worker_id: usize,
    },
    /// All local consumers disconnected from the shared local queue.
    #[error("shared local work queue has no consumers")]
    LocalWorkersDisconnected,
    /// A stable transport stream stopped receiving work.
    #[error("transport stream {stream_id} disconnected during dispatch")]
    StreamDisconnected {
        /// Stable zero-based transport stream identifier.
        stream_id: usize,
    },
    /// A strategy calibration value is invalid.
    #[error("invalid strategy configuration field '{field}'")]
    InvalidStrategyConfig {
        /// Invalid configuration field.
        field: &'static str,
    },
    /// A configured logical queue bound exceeds the representable range.
    #[error("logical strategy queue bound overflows")]
    ArithmeticOverflow,
}

/// Receiver for one worker's bounded work queue.
pub type WorkQueue = Receiver<WorkItem>;

/// Streaming size-strategy dispatcher backed by bounded worker queues.
pub struct WorkDispatcher {
    senders: Vec<Sender<WorkItem>>,
    next_worker: usize,
    config: StrategyConfig,
}

/// Create a dispatcher and one bounded queue per worker.
///
/// Queue vector indices are stable worker identifiers. Consumers should start
/// draining every queue before calling [`WorkDispatcher::dispatch`], which may
/// block when a worker's queue reaches `queue_capacity`.
///
/// # Errors
///
/// Returns [`DispatchError::ZeroWorkers`] or
/// [`DispatchError::ZeroQueueCapacity`] for invalid bounds.
pub fn bounded_work_queues(
    workers: usize,
    queue_capacity: usize,
) -> Result<(WorkDispatcher, Vec<WorkQueue>), DispatchError> {
    bounded_work_queues_with_config(workers, queue_capacity, StrategyConfig::default())
}

/// Create per-worker queues with explicit logical strategy thresholds.
///
/// This is the transport-oriented scheduler. Small and medium work is
/// assigned round-robin to the stable worker queues; use
/// [`shared_bounded_work_queues`] for local workers whose scheduling should be
/// independent from transport stream ownership.
///
/// # Errors
/// Returns an error for zero workers, zero queue capacity, or invalid strategy
/// thresholds.
pub fn bounded_work_queues_with_config(
    workers: usize,
    queue_capacity: usize,
    config: StrategyConfig,
) -> Result<(WorkDispatcher, Vec<WorkQueue>), DispatchError> {
    if workers == 0 {
        return Err(DispatchError::ZeroWorkers);
    }
    if queue_capacity == 0 {
        return Err(DispatchError::ZeroQueueCapacity);
    }
    let config = config.validate()?;

    let (senders, queues) = (0..workers).map(|_| bounded(queue_capacity)).unzip();
    Ok((
        WorkDispatcher {
            senders,
            next_worker: 0,
            config,
        },
        queues,
    ))
}

impl WorkDispatcher {
    /// Classify and dispatch a stream of regular files, then close all queues.
    ///
    /// The returned statistics are metadata-only debug event counts. File data
    /// is read later by workers, keeping dispatch memory independent of input
    /// file sizes.
    ///
    /// # Errors
    ///
    /// Returns [`DispatchError::NonFile`] for an entry that is not a regular
    /// file, or [`DispatchError::WorkerDisconnected`] if a queue loses all
    /// consumers before dispatch finishes.
    pub fn dispatch(
        mut self,
        files: impl IntoIterator<Item = FileEntry>,
    ) -> Result<DispatchStats, DispatchError> {
        let config = self.config;
        DispatchState::new(config).dispatch(files, &mut self)
    }
}

impl DispatchSink for WorkDispatcher {
    fn send_local(&mut self, work: WorkItem) -> Result<(), DispatchError> {
        let worker_id = self.next_worker;
        self.next_worker = (self.next_worker + 1) % self.senders.len();
        self.send_to(worker_id, work)
    }

    fn send_stream(&mut self, stream_id: usize, work: WorkItem) -> Result<(), DispatchError> {
        self.senders[stream_id]
            .send(work)
            .map_err(|_| DispatchError::WorkerDisconnected {
                worker_id: stream_id,
            })
    }

    fn stream_count(&self) -> usize {
        self.senders.len()
    }
}

impl WorkDispatcher {
    fn send_to(&self, worker_id: usize, work: WorkItem) -> Result<(), DispatchError> {
        self.senders[worker_id]
            .send(work)
            .map_err(|_| DispatchError::WorkerDisconnected { worker_id })
    }
}

/// Receivers for shared local scheduling and stable transport streams.
#[derive(Debug)]
pub struct SharedWorkQueues {
    /// Cloned receivers for local workers. All receivers drain one queue.
    pub local: Vec<WorkQueue>,
    /// One receiver per stable transport stream.
    pub streams: Vec<WorkQueue>,
}

/// Dispatcher using a shared local queue and stable per-stream chunk queues.
pub struct SharedWorkDispatcher {
    local_sender: Sender<WorkItem>,
    stream_senders: Vec<Sender<WorkItem>>,
    config: StrategyConfig,
}

/// Create a shared local queue and stable transport stream queues.
///
/// `queue_capacity` applies to the shared local queue and each stream queue.
/// Local worker count controls only the number of cloned local receivers; it
/// does not affect large-file stream ownership.
///
/// # Errors
/// Returns an error for zero workers, zero queue capacity, zero streams, or
/// invalid strategy thresholds.
pub fn shared_bounded_work_queues(
    local_workers: usize,
    queue_capacity: usize,
    streams: usize,
) -> Result<(SharedWorkDispatcher, SharedWorkQueues), DispatchError> {
    shared_bounded_work_queues_with_config(
        local_workers,
        queue_capacity,
        streams,
        StrategyConfig::default(),
    )
}

/// Create shared local and stable stream queues with explicit thresholds.
///
/// # Errors
/// Returns an error for zero workers, zero queue capacity, zero streams, or
/// invalid strategy thresholds.
pub fn shared_bounded_work_queues_with_config(
    local_workers: usize,
    queue_capacity: usize,
    streams: usize,
    config: StrategyConfig,
) -> Result<(SharedWorkDispatcher, SharedWorkQueues), DispatchError> {
    if local_workers == 0 {
        return Err(DispatchError::ZeroWorkers);
    }
    if queue_capacity == 0 {
        return Err(DispatchError::ZeroQueueCapacity);
    }
    if streams == 0 {
        return Err(DispatchError::InvalidStrategyConfig { field: "streams" });
    }
    let config = config.validate()?;
    let (local_sender, local_receiver) = bounded(queue_capacity);
    let local = (0..local_workers).map(|_| local_receiver.clone()).collect();
    let (stream_senders, stream_receivers) = (0..streams).map(|_| bounded(queue_capacity)).unzip();
    Ok((
        SharedWorkDispatcher {
            local_sender,
            stream_senders,
            config,
        },
        SharedWorkQueues {
            local,
            streams: stream_receivers,
        },
    ))
}

/// Calculate the worst-case logical bytes represented by queued work.
///
/// Work items contain metadata only; transfer workers read file payloads after
/// dequeue. This bound is therefore a conservative scheduling reservation,
/// not an allocation performed by this module.
///
/// # Errors
/// Returns an error for zero queue capacity, zero streams, invalid thresholds,
/// or arithmetic overflow.
pub fn logical_queue_bound_bytes(
    queue_capacity: usize,
    streams: usize,
    config: StrategyConfig,
) -> Result<u64, DispatchError> {
    if queue_capacity == 0 {
        return Err(DispatchError::ZeroQueueCapacity);
    }
    if streams == 0 {
        return Err(DispatchError::InvalidStrategyConfig { field: "streams" });
    }
    let config = config.validate()?;
    let queue_capacity =
        u64::try_from(queue_capacity).map_err(|_| DispatchError::ArithmeticOverflow)?;
    let streams = u64::try_from(streams).map_err(|_| DispatchError::ArithmeticOverflow)?;
    let local_item = config
        .batch_target_size
        .max(config.whole_file_limit)
        .max(config.small_file_limit.saturating_sub(1));
    queue_capacity
        .checked_mul(local_item)
        .and_then(|local| {
            streams
                .checked_mul(queue_capacity)
                .and_then(|stream_count| stream_count.checked_mul(config.chunk_size))
                .and_then(|stream| local.checked_add(stream))
        })
        .ok_or(DispatchError::ArithmeticOverflow)
}

impl SharedWorkDispatcher {
    /// Dispatch metadata-only work and close all queues when complete.
    ///
    /// # Errors
    /// Returns an error for non-file entries, disconnected local consumers, or
    /// disconnected stable transport streams.
    pub fn dispatch(
        mut self,
        files: impl IntoIterator<Item = FileEntry>,
    ) -> Result<DispatchStats, DispatchError> {
        DispatchState::new(self.config).dispatch(files, &mut self)
    }
}

impl DispatchSink for SharedWorkDispatcher {
    fn send_local(&mut self, work: WorkItem) -> Result<(), DispatchError> {
        self.local_sender
            .send(work)
            .map_err(|_| DispatchError::LocalWorkersDisconnected)
    }

    fn send_stream(&mut self, stream_id: usize, work: WorkItem) -> Result<(), DispatchError> {
        self.stream_senders[stream_id]
            .send(work)
            .map_err(|_| DispatchError::StreamDisconnected { stream_id })
    }

    fn stream_count(&self) -> usize {
        self.stream_senders.len()
    }
}

trait DispatchSink {
    fn send_local(&mut self, work: WorkItem) -> Result<(), DispatchError>;
    fn send_stream(&mut self, stream_id: usize, work: WorkItem) -> Result<(), DispatchError>;
    fn stream_count(&self) -> usize;
}

struct DispatchState {
    config: StrategyConfig,
    batch_files: Vec<FileEntry>,
    batch_size: u64,
    stats: DispatchStats,
}

impl DispatchState {
    fn new(config: StrategyConfig) -> Self {
        Self {
            config,
            batch_files: Vec::new(),
            batch_size: 0,
            stats: DispatchStats::default(),
        }
    }

    fn dispatch(
        mut self,
        files: impl IntoIterator<Item = FileEntry>,
        sink: &mut impl DispatchSink,
    ) -> Result<DispatchStats, DispatchError> {
        for file in files {
            if file.kind != EntryKind::File {
                return Err(DispatchError::NonFile {
                    path: file.path,
                    kind: file.kind,
                });
            }

            if file.size < self.config.small_file_limit {
                self.add_small_file(file, sink)?;
            } else if file.size <= self.config.whole_file_limit {
                sink.send_local(WorkItem::WholeFile(file))?;
                self.stats.whole_files += 1;
            } else {
                self.send_chunks(file, sink)?;
            }
        }

        self.flush_batch(sink)?;
        Ok(self.stats)
    }

    fn add_small_file(
        &mut self,
        file: FileEntry,
        sink: &mut impl DispatchSink,
    ) -> Result<(), DispatchError> {
        let would_exceed_target = !self.batch_files.is_empty()
            && self.batch_size.saturating_add(file.size) > self.config.batch_target_size;
        if would_exceed_target || self.batch_files.len() == self.config.max_batch_files {
            self.flush_batch(sink)?;
        }

        self.batch_size = self.batch_size.saturating_add(file.size);
        self.batch_files.push(file);
        if self.batch_size == self.config.batch_target_size {
            self.flush_batch(sink)?;
        }
        Ok(())
    }

    fn flush_batch(&mut self, sink: &mut impl DispatchSink) -> Result<(), DispatchError> {
        if self.batch_files.is_empty() {
            return Ok(());
        }

        let files = mem::take(&mut self.batch_files);
        let total_size = mem::take(&mut self.batch_size);
        let file_count = files.len();
        sink.send_local(WorkItem::SmallBatch(SmallBatch { files, total_size }))?;
        self.stats.batches += 1;
        self.stats.batched_files += file_count;
        Ok(())
    }

    fn send_chunks(
        &mut self,
        file: FileEntry,
        sink: &mut impl DispatchSink,
    ) -> Result<(), DispatchError> {
        let file = Arc::new(file);
        let mut offset = 0;
        let stream_count =
            u64::try_from(sink.stream_count()).map_err(|_| DispatchError::ArithmeticOverflow)?;
        while offset < file.size {
            let length = self.config.chunk_size.min(file.size - offset);
            let stream_id = usize::try_from((offset / self.config.chunk_size) % stream_count)
                .map_err(|_| DispatchError::ArithmeticOverflow)?;
            sink.send_stream(
                stream_id,
                WorkItem::Chunk(FileChunk {
                    file: Arc::clone(&file),
                    offset,
                    length,
                }),
            )?;
            self.stats.chunks += 1;
            offset += length;
        }
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::collections::HashSet;
    use std::time::{Duration, UNIX_EPOCH};

    use super::*;

    fn file(path: impl Into<String>, size: u64) -> FileEntry {
        FileEntry {
            path: path.into(),
            kind: EntryKind::File,
            size,
            mtime: UNIX_EPOCH,
            mode: 0o644,
            fingerprint: crate::scanner::SourceFingerprint::synthetic(
                EntryKind::File,
                size,
                UNIX_EPOCH,
            ),
        }
    }

    fn run_dispatch(
        files: impl IntoIterator<Item = FileEntry>,
        workers: usize,
        queue_capacity: usize,
    ) -> (DispatchStats, Vec<Vec<WorkItem>>) {
        let (dispatcher, queues) = bounded_work_queues(workers, queue_capacity).unwrap();
        std::thread::scope(|scope| {
            let handles: Vec<_> = queues
                .into_iter()
                .map(|queue| scope.spawn(move || queue.iter().collect()))
                .collect();
            let stats = dispatcher.dispatch(files).unwrap();
            let work = handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect();
            (stats, work)
        })
    }

    fn run_shared_dispatch(
        files: impl IntoIterator<Item = FileEntry>,
        local_workers: usize,
        streams: usize,
        queue_capacity: usize,
        config: StrategyConfig,
    ) -> (DispatchStats, Vec<WorkItem>, Vec<Vec<WorkItem>>) {
        let (dispatcher, queues) =
            shared_bounded_work_queues_with_config(local_workers, queue_capacity, streams, config)
                .unwrap();
        std::thread::scope(|scope| {
            let local_handles: Vec<_> = queues
                .local
                .into_iter()
                .map(|queue| scope.spawn(move || queue.iter().collect::<Vec<_>>()))
                .collect();
            let stream_handles: Vec<_> = queues
                .streams
                .into_iter()
                .map(|queue| scope.spawn(move || queue.iter().collect::<Vec<_>>()))
                .collect();
            let stats = dispatcher.dispatch(files).unwrap();
            let local = local_handles
                .into_iter()
                .flat_map(|handle| handle.join().unwrap())
                .collect();
            let streams = stream_handles
                .into_iter()
                .map(|handle| handle.join().unwrap())
                .collect();
            (stats, local, streams)
        })
    }

    #[test]
    fn applies_size_boundaries_and_chunk_lengths() {
        let files = [
            file("small", SMALL_FILE_LIMIT - 1),
            file("medium-low", SMALL_FILE_LIMIT),
            file("medium-high", WHOLE_FILE_LIMIT),
            file("huge", WHOLE_FILE_LIMIT + 1),
        ];

        let (stats, work) = run_dispatch(files, 2, DEFAULT_QUEUE_CAPACITY);
        let all_work: Vec<_> = work.into_iter().flatten().collect();

        assert_eq!(stats.batches, 1);
        assert_eq!(stats.batched_files, 1);
        assert_eq!(stats.whole_files, 2);
        assert_eq!(stats.chunks, 3);
        let mut chunks: Vec<_> = all_work
            .iter()
            .filter_map(|item| match item {
                WorkItem::Chunk(chunk) => Some((chunk.offset, chunk.length)),
                _ => None,
            })
            .collect();
        chunks.sort_unstable();
        assert_eq!(
            chunks,
            [
                (0, CHUNK_SIZE),
                (CHUNK_SIZE, CHUNK_SIZE),
                (2 * CHUNK_SIZE, 1)
            ]
        );
    }

    #[test]
    fn coalesces_50k_small_files_into_target_sized_batches() {
        let files = (0..50_000).map(|index| file(format!("small-{index}"), 4 * 1024));

        let (stats, work) = run_dispatch(files, 4, DEFAULT_QUEUE_CAPACITY);

        assert_eq!(stats.batched_files, 50_000);
        assert_eq!(stats.batches, 7);
        assert_eq!(stats.whole_files, 0);
        assert_eq!(stats.chunks, 0);
        assert_eq!(work.into_iter().flatten().count(), 7);
    }

    #[test]
    fn stripes_a_one_gib_file_across_every_worker() {
        let workers = 8;
        let (stats, work) = run_dispatch([file("huge", 1024 * 1024 * 1024)], workers, 1);

        assert_eq!(stats.chunks, 64);
        let active_workers: HashSet<_> = work
            .iter()
            .enumerate()
            .filter_map(|(worker_id, items)| (!items.is_empty()).then_some(worker_id))
            .collect();
        assert_eq!(active_workers, (0..workers).collect());

        let mut ranges: Vec<_> = work
            .into_iter()
            .flatten()
            .map(|item| match item {
                WorkItem::Chunk(chunk) => (chunk.offset, chunk.length),
                _ => panic!("huge file produced non-chunk work"),
            })
            .collect();
        ranges.sort_unstable();
        assert!(ranges.iter().enumerate().all(|(index, &(offset, length))| {
            offset == index as u64 * CHUNK_SIZE && length == CHUNK_SIZE
        }));
    }

    #[test]
    fn worker_queues_are_bounded_and_empty_batches_are_capped() {
        let (dispatcher, queues) = bounded_work_queues(3, 2).unwrap();
        assert!(queues.iter().all(|queue| queue.capacity() == Some(2)));

        let files = (0..20_000).map(|index| file(format!("empty-{index}"), 0));
        let (stats, work) = std::thread::scope(|scope| {
            let handles: Vec<_> = queues
                .into_iter()
                .map(|queue| scope.spawn(move || queue.iter().collect::<Vec<_>>()))
                .collect();
            let stats = dispatcher.dispatch(files).unwrap();
            let work = handles
                .into_iter()
                .flat_map(|handle| handle.join().unwrap())
                .collect::<Vec<_>>();
            (stats, work)
        });

        assert_eq!(stats.batches, 3);
        assert!(work.into_iter().all(|item| match item {
            WorkItem::SmallBatch(batch) => batch.files.len() <= MAX_BATCH_FILES,
            _ => false,
        }));
    }

    #[test]
    fn rejects_invalid_configuration_and_non_files() {
        assert!(matches!(
            bounded_work_queues(0, 1),
            Err(DispatchError::ZeroWorkers)
        ));
        assert!(matches!(
            bounded_work_queues(1, 0),
            Err(DispatchError::ZeroQueueCapacity)
        ));
        assert!(matches!(
            shared_bounded_work_queues(1, 1, 0),
            Err(DispatchError::InvalidStrategyConfig { field: "streams" })
        ));
        assert!(matches!(
            shared_bounded_work_queues_with_config(
                1,
                1,
                1,
                StrategyConfig {
                    chunk_size: 0,
                    ..StrategyConfig::default()
                }
            ),
            Err(DispatchError::InvalidStrategyConfig {
                field: "chunk_size"
            })
        ));

        let (dispatcher, queues) = bounded_work_queues(1, 1).unwrap();
        let consumer = std::thread::spawn(move || queues[0].iter().count());
        let directory = FileEntry {
            path: "dir".to_owned(),
            kind: EntryKind::Directory,
            size: 0,
            mtime: UNIX_EPOCH,
            mode: 0o755,
            fingerprint: crate::scanner::SourceFingerprint::synthetic(
                EntryKind::Directory,
                0,
                UNIX_EPOCH,
            ),
        };
        assert!(matches!(
            dispatcher.dispatch([directory]),
            Err(DispatchError::NonFile { .. })
        ));
        assert_eq!(consumer.join().unwrap(), 0);
    }

    #[test]
    fn shared_dispatch_keeps_local_work_separate_from_stable_streams() {
        let config = StrategyConfig {
            small_file_limit: 8,
            whole_file_limit: 16,
            batch_target_size: 8,
            chunk_size: 4,
            ..StrategyConfig::default()
        };
        let huge = file("huge", 17);
        let (stats, local, streams) = run_shared_dispatch(
            [file("small-a", 4), file("small-b", 4), huge],
            2,
            3,
            2,
            config,
        );

        assert_eq!(stats.batches, 1);
        assert_eq!(stats.batched_files, 2);
        assert_eq!(stats.chunks, usize::try_from(17_u64.div_ceil(4)).unwrap());
        assert_eq!(local.len(), 1);
        assert!(matches!(local[0], WorkItem::SmallBatch(_)));
        assert_eq!(streams.iter().map(Vec::len).sum::<usize>(), stats.chunks);
        let assigned_streams: Vec<_> = streams
            .iter()
            .enumerate()
            .flat_map(|(stream_id, items)| {
                items.iter().map(move |item| match item {
                    WorkItem::Chunk(chunk) => (stream_id, chunk.offset, chunk.length),
                    _ => panic!("stable stream received non-chunk work"),
                })
            })
            .collect();
        let mut ranges: Vec<_> = assigned_streams
            .iter()
            .map(|(_, offset, length)| (*offset, *length))
            .collect();
        ranges.sort_unstable();
        assert!(ranges
            .windows(2)
            .all(|pair| { pair[0].0.saturating_add(pair[0].1) <= pair[1].0 }));
        assert!(assigned_streams.iter().all(|(stream_id, offset, _)| {
            *stream_id == (usize::try_from(*offset / config.chunk_size).unwrap() % streams.len())
        }));
    }

    #[test]
    fn logical_queue_bound_accounts_for_shared_local_and_stable_stream_queues() {
        let bound = logical_queue_bound_bytes(2, 16, StrategyConfig::default()).unwrap();
        assert_eq!(bound, 576 * 1024 * 1024);
        assert!(matches!(
            logical_queue_bound_bytes(0, 16, StrategyConfig::default()),
            Err(DispatchError::ZeroQueueCapacity)
        ));
    }

    #[test]
    fn shared_local_queue_allows_fast_worker_to_drain_while_one_sleeps() {
        let (dispatcher, queues) = shared_bounded_work_queues(2, 1, 1).unwrap();
        let mut local = queues.local.into_iter();
        let slow = local.next().unwrap();
        let fast = local.next().unwrap();
        let (started_sender, started_receiver) = crossbeam_channel::bounded(1);
        let files = (0..32).map(|index| file(format!("medium-{index}"), SMALL_FILE_LIMIT));

        std::thread::scope(|scope| {
            let slow_handle = scope.spawn(move || {
                let _first = slow.recv().unwrap();
                started_sender.send(()).unwrap();
                std::thread::sleep(Duration::from_millis(50));
                1 + slow.iter().count()
            });
            let dispatch_handle = scope.spawn(move || dispatcher.dispatch(files).unwrap());
            started_receiver.recv().unwrap();
            let fast_handle = scope.spawn(move || fast.iter().count());

            let stats = dispatch_handle.join().unwrap();
            let slow_count = slow_handle.join().unwrap();
            let fast_count = fast_handle.join().unwrap();
            assert_eq!(stats.whole_files, 32);
            assert!(fast_count > slow_count);
        });
    }
}
