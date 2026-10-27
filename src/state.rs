use crate::{config::Config, error::Result};
use sqlx::{PgPool, postgres::PgPoolOptions};
use std::{
    collections::HashSet,
    future::Future,
    sync::{Arc, Mutex, MutexGuard, PoisonError},
    time::Duration,
};
use tokio::sync::Semaphore;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

const DATABASE_ACQUIRE_TIMEOUT: Duration = Duration::from_secs(5);
const HTTP_CONNECT_TIMEOUT: Duration = Duration::from_secs(10);
const WORKER_SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);
/// Pictures drawn at once for one lesson.
pub(crate) const DRAWING_SLOTS: usize = 8;

#[derive(Clone)]
pub struct AppState {
    pub(crate) pool: PgPool,
    /// The AI gateway and tmpfile, both configured and trusted.
    pub(crate) http: reqwest::Client,
    pub(crate) config: Arc<Config>,
    /// What runs in the background now: XABC writing, or a course being designed.
    jobs: Arc<Mutex<HashSet<&'static str>>>,
    pub(crate) drawing_slots: Arc<Semaphore>,
    /// Cancels the schedule and the workers before the process exits.
    pub(crate) shutdown: CancellationToken,
    workers: TaskTracker,
}

impl AppState {
    pub async fn new(config: Config) -> Result<Self> {
        run_migrations(&config).await?;
        let pool = PgPoolOptions::new()
            .max_connections(config.database_max_connections)
            .acquire_timeout(DATABASE_ACQUIRE_TIMEOUT)
            .after_connect(|connection, _| {
                Box::pin(async move {
                    // Runtime queries are bounded; migrations use their own pool below.
                    sqlx::query("SET statement_timeout = '10s'")
                        .execute(connection)
                        .await?;
                    Ok(())
                })
            })
            .connect(&config.database_url)
            .await?;
        let http = reqwest::Client::builder()
            .connect_timeout(HTTP_CONNECT_TIMEOUT)
            .timeout(config.ai_timeout)
            .build()?;
        Ok(Self {
            pool,
            http,
            config: Arc::new(config),
            jobs: Arc::default(),
            drawing_slots: Arc::new(Semaphore::new(DRAWING_SLOTS)),
            shutdown: CancellationToken::new(),
            workers: TaskTracker::new(),
        })
    }

    /// Whether XABC writes on its schedule (`LEARN_WRITER`).
    pub fn writes(&self) -> bool {
        self.config.writer
    }

    /// Takes the mark for `job`, unless it is taken: one of each runs at a time.
    pub(crate) fn begin(&self, job: &'static str) -> bool {
        self.jobs().insert(job)
    }

    pub(crate) fn end(&self, job: &'static str) {
        self.jobs().remove(job);
    }

    pub(crate) fn running(&self, job: &'static str) -> bool {
        self.jobs().contains(job)
    }

    fn jobs(&self) -> MutexGuard<'_, HashSet<&'static str>> {
        self.jobs.lock().unwrap_or_else(PoisonError::into_inner)
    }

    pub(crate) fn spawn_worker<F>(&self, future: F)
    where
        F: Future<Output = ()> + Send + 'static,
    {
        // Once shutdown has begun, new work is not started.
        if !self.workers.is_closed() {
            self.workers.spawn(future);
        }
    }

    /// Workers stop at the shutdown signal; any still busy after the window end with the process.
    pub async fn shutdown_workers(&self) {
        self.shutdown.cancel();
        self.workers.close();
        let _ = tokio::time::timeout(WORKER_SHUTDOWN_TIMEOUT, self.workers.wait()).await;
    }
}

async fn run_migrations(config: &Config) -> Result<()> {
    let migrator = sqlx::migrate::Migrator::new(config.migrations_dir.clone()).await?;
    // Schema changes may outlast the runtime statement timeout, so they get a pool of their own.
    let pool = PgPoolOptions::new()
        .max_connections(1)
        .acquire_timeout(DATABASE_ACQUIRE_TIMEOUT)
        .connect(&config.database_url)
        .await?;
    let result = migrator.run(&pool).await;
    pool.close().await;
    Ok(result?)
}
