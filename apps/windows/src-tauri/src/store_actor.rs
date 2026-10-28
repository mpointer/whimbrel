//! The store actor (plan Decision 1): one OS thread owns `memory::Store` and runs
//! closure jobs in order.
//!
//! A `std::thread`, not a tokio task, because `sync_engine::sync_content` and
//! `sync_settings` `block_on` a private runtime, which panics inside another runtime.
//! Jobs sent here run on a plain thread where that is legal. `memory::Store` is `Send`
//! but `!Sync`; it is created on the actor thread and never leaves it.
//!
//! Every job's wait in the queue is recorded (plan Decision 9: some core calls hold the
//! store across network I/O, and everything else queues behind them).
//!
//! The app runs two actors on two connections to the same database (M3.3 Decision 2):
//! the main store, and a "slow lane" for the calls that hold a store across a provider
//! call (blocking asks, titles, the daily recap, meeting summaries), so capture, sync and
//! the Library don't queue behind them. `core/memory/tests/two_connections.rs` pins what
//! that relies on.

use std::collections::VecDeque;
use std::panic::{self, AssertUnwindSafe};
use std::path::PathBuf;
use std::sync::{mpsc, Arc, Mutex};
use std::time::{Duration, Instant};

use memory::Store;
use schemars::JsonSchema;
use serde::Serialize;

use crate::error::CommandError;

type Job = Box<dyn FnOnce(&Store) + Send + 'static>;

struct Envelope {
    job: Job,
    enqueued: Instant,
}

/// A cloneable handle to the store actor. Dropping the last handle stops the actor
/// after it drains its queue.
#[derive(Clone)]
pub struct StoreHandle {
    tx: mpsc::Sender<Envelope>,
    stats: Arc<Mutex<QueueStats>>,
}

impl StoreHandle {
    /// Opens the memory store the way a shell must (`api::store::open`, which runs the
    /// one-time startup sweep) on a new actor thread. `models_dir` is the MiniLM
    /// directory (plan Decision 12); the store opens fine before the model exists.
    pub fn open(db_path: PathBuf, models_dir: PathBuf) -> Result<Self, CommandError> {
        Self::spawn(move || api::store::open(&db_path, &models_dir).map_err(|e| e.to_string()))
    }

    /// Opens a second connection to `db_path` on its own actor (the slow lane), sharing
    /// `primary`'s embedder so the MiniLM model loads once. Plain `Store::open` semantics:
    /// no startup sweep, which runs once, on the primary. Open it at launch before capture
    /// or sync write: opening takes the write lock briefly and waits behind a writer.
    pub fn open_secondary(db_path: PathBuf, primary: &StoreHandle) -> Result<Self, CommandError> {
        let embedder = primary.call_blocking(|s| s.embedder())?;
        Self::spawn_named("whimbrel-store-slow", move || {
            Store::open_with_embedder(&db_path, embedder).map_err(|e| e.to_string())
        })
    }

    /// Starts an actor around whatever `open` returns (tests use an in-memory store).
    /// Blocks until `open` has run, so an open failure is reported here.
    pub fn spawn(open: impl FnOnce() -> Result<Store, String> + Send + 'static) -> Result<Self, CommandError> {
        Self::spawn_named("whimbrel-store", open)
    }

    fn spawn_named(
        thread_name: &str,
        open: impl FnOnce() -> Result<Store, String> + Send + 'static,
    ) -> Result<Self, CommandError> {
        let (tx, rx) = mpsc::channel::<Envelope>();
        let (opened_tx, opened_rx) = mpsc::sync_channel::<Result<(), String>>(1);
        let stats = Arc::new(Mutex::new(QueueStats::default()));
        let actor_stats = stats.clone();
        std::thread::Builder::new()
            .name(thread_name.into())
            .spawn(move || {
                let store = match open() {
                    Ok(store) => {
                        let _ = opened_tx.send(Ok(()));
                        store
                    }
                    Err(e) => {
                        let _ = opened_tx.send(Err(e));
                        return;
                    }
                };
                for envelope in rx {
                    actor_stats.lock().unwrap().record(envelope.enqueued.elapsed());
                    // `call` already catches a panicking job; this is a backstop so a
                    // bug in a raw `send` can never take the actor down.
                    let _ = panic::catch_unwind(AssertUnwindSafe(|| (envelope.job)(&store)));
                }
                tracing::info!("store actor stopped");
            })
            .map_err(|e| CommandError::store(format!("could not start the store thread: {e}")))?;
        match opened_rx.recv() {
            Ok(Ok(())) => Ok(StoreHandle { tx, stats }),
            Ok(Err(e)) => Err(CommandError::store(format!("could not open the store: {e}"))),
            Err(_) => Err(CommandError::store("the store thread exited while opening")),
        }
    }

    fn send(&self, job: Job) -> Result<(), CommandError> {
        self.tx
            .send(Envelope { job, enqueued: Instant::now() })
            .map_err(|_| CommandError::store("the store actor has stopped"))
    }

    /// Runs `f` on the actor and awaits its result. Safe from async code: the caller's
    /// task only awaits a oneshot; `f` runs on the actor thread, where a core function
    /// that `block_on`s is legal. A panic in `f` comes back as `CommandError::Store`.
    pub async fn call<R, F>(&self, f: F) -> Result<R, CommandError>
    where
        F: FnOnce(&Store) -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = tokio::sync::oneshot::channel();
        self.send(Box::new(move |store| {
            let _ = tx.send(panic::catch_unwind(AssertUnwindSafe(|| f(store))));
        }))?;
        let result = rx.await.map_err(|_| CommandError::store("the store actor dropped the job"))?;
        result.map_err(|panic| CommandError::store(format!("a store job panicked: {}", panic_message(&panic))))
    }

    /// [`call`](Self::call) for callers on a plain thread (setup, timers that already
    /// run off the async runtime). Never call it from an async task: it blocks.
    pub fn call_blocking<R, F>(&self, f: F) -> Result<R, CommandError>
    where
        F: FnOnce(&Store) -> R + Send + 'static,
        R: Send + 'static,
    {
        let (tx, rx) = mpsc::sync_channel(1);
        self.send(Box::new(move |store| {
            let _ = tx.send(panic::catch_unwind(AssertUnwindSafe(|| f(store))));
        }))?;
        let result = rx.recv().map_err(|_| CommandError::store("the store actor dropped the job"))?;
        result.map_err(|panic| CommandError::store(format!("a store job panicked: {}", panic_message(&panic))))
    }

    pub fn queue_wait(&self) -> QueueWaitStats {
        self.stats.lock().unwrap().snapshot()
    }

    /// Logs each interval's queue-wait percentiles every `every`, so a minute with a
    /// content sync or a blocking answer in flight shows its own p95 (Decision 9), not one
    /// diluted by the rolling window `queue_wait` reports.
    pub fn log_queue_wait_every(&self, lane: &'static str, every: Duration) {
        let stats = self.stats.clone();
        let spawned = std::thread::Builder::new().name("whimbrel-queue-log".into()).spawn(move || loop {
            std::thread::sleep(every);
            let s = stats.lock().unwrap().take_interval();
            if s.jobs > 0 {
                tracing::info!(lane, jobs = s.jobs, p50_ms = s.p50_ms, p95_ms = s.p95_ms, max_ms = s.max_ms, "store queue wait");
            }
        });
        if let Err(e) = spawned {
            tracing::warn!(error = %e, "could not start the queue-wait log");
        }
    }
}

fn panic_message(panic: &Box<dyn std::any::Any + Send>) -> String {
    panic
        .downcast_ref::<&str>()
        .map(|s| s.to_string())
        .or_else(|| panic.downcast_ref::<String>().cloned())
        .unwrap_or_else(|| "unknown panic".into())
}

/// The most recent queue waits, for percentiles (Task 10's measurement).
#[derive(Default)]
struct QueueStats {
    jobs: u64,
    recent: VecDeque<Duration>,
    /// Waits since the last `take_interval`, for the periodic log.
    interval: Vec<Duration>,
}

const RECENT_WAITS: usize = 4096;
/// A bound on one interval's waits, far above a minute's jobs.
const INTERVAL_WAITS: usize = 100_000;

impl QueueStats {
    fn record(&mut self, wait: Duration) {
        self.jobs += 1;
        if self.recent.len() == RECENT_WAITS {
            self.recent.pop_front();
        }
        self.recent.push_back(wait);
        if self.interval.len() < INTERVAL_WAITS {
            self.interval.push(wait);
        }
    }

    fn snapshot(&self) -> QueueWaitStats {
        percentiles(self.jobs, self.recent.iter().copied().collect())
    }

    /// This interval's percentiles (`jobs` counts the interval's jobs), then a new interval.
    fn take_interval(&mut self) -> QueueWaitStats {
        let waits = std::mem::take(&mut self.interval);
        percentiles(waits.len() as u64, waits)
    }
}

fn percentiles(jobs: u64, mut waits: Vec<Duration>) -> QueueWaitStats {
    waits.sort();
    let pct = |p: f64| -> f64 {
        if waits.is_empty() {
            return 0.0;
        }
        let i = ((waits.len() - 1) as f64 * p).round() as usize;
        waits[i].as_secs_f64() * 1000.0
    };
    QueueWaitStats { jobs, p50_ms: pct(0.5), p95_ms: pct(0.95), max_ms: pct(1.0) }
}

/// How long jobs waited for the store, over the last 4,096 jobs.
#[derive(Debug, Clone, Copy, PartialEq, Serialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
pub struct QueueWaitStats {
    /// Jobs run since launch.
    pub jobs: u64,
    pub p50_ms: f64,
    pub p95_ms: f64,
    pub max_ms: f64,
}

#[cfg(test)]
mod tests {
    use super::*;

    fn in_memory() -> StoreHandle {
        StoreHandle::spawn(|| Store::open_in_memory().map_err(|e| e.to_string())).unwrap()
    }

    #[test]
    fn jobs_run_in_fifo_order() {
        let store = in_memory();
        let order = Arc::new(Mutex::new(Vec::new()));
        for i in 0..50 {
            let order = order.clone();
            store.send(Box::new(move |_| order.lock().unwrap().push(i))).unwrap();
        }
        store.call_blocking(|_| ()).unwrap();
        assert_eq!(*order.lock().unwrap(), (0..50).collect::<Vec<_>>());
    }

    #[test]
    fn a_panicking_job_is_an_error_and_the_actor_survives() {
        let store = in_memory();
        let err = store.call_blocking(|_| -> u8 { panic!("boom") }).unwrap_err();
        assert_eq!(err, CommandError::store("a store job panicked: boom"));
        assert_eq!(store.call_blocking(|_| 7).unwrap(), 7);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn call_from_async_can_run_a_block_on_core_function() {
        // The property Decision 1 exists for: sync_engine::sync_content builds and
        // block_on's its own runtime. Doing that inside a tokio task panics; inside a
        // store job it must not.
        let store = in_memory();
        let value = store
            .call(|_| {
                let rt = tokio::runtime::Builder::new_current_thread().build().unwrap();
                rt.block_on(async { 41 + 1 })
            })
            .await
            .unwrap();
        assert_eq!(value, 42);
    }

    #[tokio::test(flavor = "multi_thread")]
    async fn jobs_see_the_real_store() {
        let store = in_memory();
        let conversations = store.call(|s| s.list_conversations().map(|c| c.len())).await.unwrap().unwrap();
        assert_eq!(conversations, 0);
    }

    #[test]
    fn an_open_failure_is_reported_by_spawn() {
        let err = StoreHandle::spawn(|| Err("disk on fire".into())).err().unwrap();
        assert_eq!(err, CommandError::store("could not open the store: disk on fire"));
    }

    #[test]
    fn opens_a_real_database_file_with_no_embedding_model() {
        let dir = std::env::temp_dir().join(format!("whimbrel-store-actor-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let store = StoreHandle::open(dir.join("kestrel.sqlite"), dir.join("models").join("minilm")).unwrap();
        assert!(!store.call_blocking(|s| s.embedding_model_loaded()).unwrap());
        drop(store);
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn a_secondary_store_sees_the_primarys_writes_and_shares_its_embedder() {
        let dir = std::env::temp_dir().join(format!("whimbrel-store-secondary-{}", std::process::id()));
        std::fs::create_dir_all(&dir).unwrap();
        let db = dir.join("kestrel.sqlite");
        let primary = StoreHandle::open(db.clone(), dir.join("models").join("minilm")).unwrap();
        let slow = StoreHandle::open_secondary(db, &primary).unwrap();

        let id = primary.call_blocking(|s| s.insert_conversation("from the main lane", 1).unwrap()).unwrap();
        let title = slow.call_blocking(move |s| s.get_conversation(id).unwrap().unwrap().title).unwrap();
        assert_eq!(title, "from the main lane");

        let (a, b) = (primary.call_blocking(|s| s.embedder()).unwrap(), slow.call_blocking(|s| s.embedder()).unwrap());
        assert!(Arc::ptr_eq(&a, &b));
        drop((primary, slow));
        let _ = std::fs::remove_dir_all(&dir);
    }

    #[test]
    fn an_interval_reports_only_its_own_waits() {
        let mut stats = QueueStats::default();
        for ms in [1, 2, 30] {
            stats.record(Duration::from_millis(ms));
        }
        let first = stats.take_interval();
        assert_eq!((first.jobs, first.max_ms), (3, 30.0));
        stats.record(Duration::from_millis(4));
        let second = stats.take_interval();
        assert_eq!((second.jobs, second.max_ms), (1, 4.0), "the first interval's 30 ms is gone");
        assert_eq!(stats.snapshot().jobs, 4, "the rolling view still counts every job");
    }

    #[test]
    fn queue_wait_stats_count_jobs_and_order_percentiles() {
        let store = in_memory();
        for _ in 0..20 {
            store.call_blocking(|_| ()).unwrap();
        }
        let stats = store.queue_wait();
        assert_eq!(stats.jobs, 20);
        assert!(stats.p50_ms <= stats.p95_ms && stats.p95_ms <= stats.max_ms);
    }
}
