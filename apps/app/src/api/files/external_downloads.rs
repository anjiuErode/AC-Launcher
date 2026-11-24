use super::{ExternalSaveError, select_external_destination};
use chrono::{DateTime, Utc};
use dashmap::DashMap;
use futures_util::StreamExt;
use serde::{Deserialize, Serialize};
use std::io::ErrorKind;
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant};
use tauri::{Emitter, Runtime};
use tauri_plugin_http::reqwest::{self, Client, RequestBuilder, StatusCode};
use theseus::{DownloadBody, DownloadSource};
use tokio::fs::{self, File, OpenOptions};
use tokio::io::{AsyncSeekExt, AsyncWriteExt};
use tokio::sync::{Mutex, OnceCell, watch};
use tokio_util::sync::CancellationToken;
use uuid::Uuid;

const PROGRESS_EVENT: &str = "external-url-download";
const CHECKPOINT_BYTES: u64 = 8 * 1024 * 1024;
const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(2);
const REPORT_INTERVAL: Duration = Duration::from_millis(200);

type DownloadResult<T> = Result<T, ExternalSaveError>;
type Jobs = DashMap<Uuid, Arc<Job>>;
type Notify = Arc<dyn Fn(DownloadSnapshot) + Send + Sync>;

#[derive(Clone, Copy, Debug, Deserialize, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
enum Status {
    Running,
    Paused,
    Failed,
    Publishing,
    Succeeded,
    Canceled,
}

#[derive(Clone, Deserialize, Serialize)]
struct Journal {
    id: Uuid,
    destination: PathBuf,
    url: Option<String>,
    status: Status,
    created_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    error: Option<String>,
    checkpoint: Checkpoint,
}

#[derive(Clone, Default, Deserialize, Serialize)]
struct Checkpoint {
    bytes: u64,
    total: Option<u64>,
    source: Option<DownloadSource>,
    complete: bool,
}

impl Journal {
    fn filename(&self) -> String {
        self.destination
            .file_name()
            .unwrap_or_default()
            .to_string_lossy()
            .into_owned()
    }

    fn part(&self) -> PathBuf {
        self.destination
            .with_file_name(format!(".acl-download-{}.part", self.id))
    }

    fn recover(&mut self) {
        match self.status {
            Status::Running => self.status = Status::Paused,
            Status::Publishing => {
                if !matches!(
                    std::fs::symlink_metadata(self.part()),
                    Err(error) if error.kind() == ErrorKind::NotFound
                ) {
                    self.status = Status::Paused;
                } else if std::fs::symlink_metadata(&self.destination)
                    .is_ok_and(|file| {
                        file.is_file() && file.len() == self.checkpoint.bytes
                    })
                {
                    self.finish(Status::Succeeded);
                } else {
                    self.status = Status::Paused;
                    self.checkpoint = Checkpoint::default();
                }
            }
            _ => {}
        }
    }

    fn finish(&mut self, status: Status) {
        self.status = status;
        self.finished_at = Some(Utc::now());
        if matches!(status, Status::Succeeded | Status::Canceled) {
            self.url = None;
            self.checkpoint.source = None;
        }
    }
}

#[derive(Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DownloadSnapshot {
    id: Uuid,
    filename: String,
    status: Status,
    stage: &'static str,
    downloaded_bytes: u64,
    total_bytes: Option<u64>,
    created_at: DateTime<Utc>,
    finished_at: Option<DateTime<Utc>>,
    error: Option<String>,
    revision: u32,
}

#[derive(Clone, Copy, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum DownloadAction {
    Pause,
    Cancel,
    Dismiss,
}

struct Active {
    token: CancellationToken,
    done: watch::Receiver<bool>,
    stop: Option<DownloadAction>,
}

struct Job {
    path: PathBuf,
    state: Mutex<JobState>,
}

struct JobState {
    journal: Journal,
    active: Option<Active>,
    snapshot: DownloadSnapshot,
    dismissed: bool,
}

impl JobState {
    fn new(journal: Journal) -> Self {
        Self {
            snapshot: DownloadSnapshot {
                id: journal.id,
                filename: journal.filename(),
                status: journal.status,
                stage: "downloading",
                downloaded_bytes: journal.checkpoint.bytes,
                total_bytes: journal.checkpoint.total,
                created_at: journal.created_at,
                finished_at: journal.finished_at,
                error: journal.error.clone(),
                revision: 0,
            },
            journal,
            active: None,
            dismissed: false,
        }
    }

    fn snapshot(&mut self) -> DownloadSnapshot {
        self.snapshot.status = self.journal.status;
        self.snapshot.finished_at = self.journal.finished_at;
        self.snapshot.error.clone_from(&self.journal.error);
        self.snapshot.revision += 1;
        self.snapshot.clone()
    }

    fn apply_checkpoint(&mut self) {
        self.snapshot.downloaded_bytes = self.journal.checkpoint.bytes;
        self.snapshot.total_bytes = self.journal.checkpoint.total;
    }
}

pub struct ExternalDownloads {
    directory: PathBuf,
    jobs: OnceCell<Jobs>,
}

impl ExternalDownloads {
    pub(super) fn new(directory: PathBuf) -> Self {
        Self {
            directory,
            jobs: OnceCell::new(),
        }
    }

    async fn jobs(&self) -> DownloadResult<&Jobs> {
        self.jobs
            .get_or_try_init(|| async {
                fs::create_dir_all(&self.directory).await?;
                let jobs = Jobs::new();
                let mut files = fs::read_dir(&self.directory).await?;
                while let Some(entry) = files.next_entry().await? {
                    let path = entry.path();
                    let Some(id) = path
                        .file_stem()
                        .and_then(|name| Uuid::try_parse(name.to_str()?).ok())
                        .filter(|_| path.extension().is_some_and(|ext| ext == "json"))
                    else {
                        continue;
                    };
                    let bytes = fs::read(&path).await?;
                    let Ok(mut journal) = serde_json::from_slice::<Journal>(&bytes)
                    else {
                        tracing::warn!(%id, "Invalid external download checkpoint");
                        continue;
                    };
                    if journal.id != id
                        || !journal.destination.is_absolute()
                        || journal.destination.file_name().is_none()
                    {
                        continue;
                    }
                    let previous = journal.status;
                    journal.recover();
                    if journal.status != previous {
                        persist(&path, &journal)?;
                    }
                    jobs.insert(id, Arc::new(Job::new(path, journal)));
                }
                Ok(jobs)
            })
            .await
    }

    async fn list(&self) -> DownloadResult<Vec<DownloadSnapshot>> {
        let jobs: Vec<_> = self
            .jobs()
            .await?
            .iter()
            .map(|job| Arc::clone(job.value()))
            .collect();
        let mut snapshots = Vec::with_capacity(jobs.len());
        for job in jobs {
            let state = job.state.lock().await;
            if !state.dismissed {
                snapshots.push(state.snapshot.clone());
            }
        }
        Ok(snapshots)
    }

    async fn create(
        &self,
        destination: PathBuf,
        url: String,
    ) -> DownloadResult<Arc<Job>> {
        if !destination.is_absolute() || destination.file_name().is_none() {
            return Err(ExternalSaveError::new("Invalid save destination"));
        }
        let jobs = self.jobs().await?;
        let id = Uuid::new_v4();
        let path = self.directory.join(format!("{id}.json"));
        let journal = Journal {
            id,
            destination,
            url: Some(url),
            status: Status::Paused,
            created_at: Utc::now(),
            finished_at: None,
            error: None,
            checkpoint: Checkpoint::default(),
        };
        persist(&path, &journal)?;
        let job = Arc::new(Job::new(path, journal));
        jobs.insert(id, Arc::clone(&job));
        Ok(job)
    }

    async fn find(&self, id: Uuid) -> DownloadResult<Arc<Job>> {
        self.jobs()
            .await?
            .get(&id)
            .map(|job| Arc::clone(job.value()))
            .ok_or_else(|| ExternalSaveError::new("Unknown download"))
    }
}

impl Job {
    fn new(path: PathBuf, journal: Journal) -> Self {
        Self {
            path,
            state: Mutex::new(JobState::new(journal)),
        }
    }

    async fn start(
        self: Arc<Self>,
        client: Client,
        notify: Notify,
    ) -> DownloadResult<DownloadSnapshot> {
        let mut state = self.state.lock().await;
        if state.dismissed
            || !matches!(state.journal.status, Status::Paused | Status::Failed)
            || state.journal.url.is_none()
        {
            return Ok(state.snapshot.clone());
        }
        let mut journal = state.journal.clone();
        journal.status = Status::Running;
        journal.finished_at = None;
        journal.error = None;
        persist(&self.path, &journal)?;
        state.journal = journal.clone();
        let token = CancellationToken::new();
        let (completed, done) = watch::channel(false);
        state.active = Some(Active {
            token: token.clone(),
            done,
            stop: None,
        });
        state.snapshot.stage = "waiting";
        let snapshot = state.snapshot();
        drop(state);

        notify(snapshot.clone());
        tokio::spawn(async move {
            let result = self.download(&journal, client, &token, &notify).await;
            let snapshot = self.settle(result).await;
            notify(snapshot);
            let _ = completed.send(true);
        });
        Ok(snapshot)
    }

    async fn control(
        &self,
        action: DownloadAction,
        notify: &Notify,
    ) -> DownloadResult<Option<DownloadSnapshot>> {
        let mut state = self.state.lock().await;
        if state.dismissed {
            return Ok(None);
        }
        if let Some(active) = &mut state.active {
            if action == DownloadAction::Dismiss {
                return Err(ExternalSaveError::new("Download is still running"));
            }
            if active.stop.is_none() || action == DownloadAction::Cancel {
                active.stop = Some(action);
            }
            active.token.cancel();
            let mut done = active.done.clone();
            drop(state);
            let _ = done.wait_for(|done| *done).await;
            return Ok(Some(self.state.lock().await.snapshot.clone()));
        }
        match action {
            DownloadAction::Cancel if state.journal.status == Status::Paused => {
                let result = self.cancel(&mut state).await;
                let snapshot = state.snapshot();
                drop(state);
                notify(snapshot.clone());
                result?;
                Ok(Some(snapshot))
            }
            DownloadAction::Dismiss if state.journal.status != Status::Paused => {
                remove_if_present(&state.journal.part()).await?;
                remove_if_present(&self.path).await?;
                state.dismissed = true;
                Ok(None)
            }
            _ => Ok(Some(state.snapshot.clone())),
        }
    }

    async fn cancel(&self, state: &mut JobState) -> DownloadResult<()> {
        let mut journal = state.journal.clone();
        journal.finish(Status::Canceled);
        persist(&self.path, &journal)?;
        state.journal = journal;
        remove_if_present(&state.journal.part()).await
    }

    async fn report(
        &self,
        stage: &'static str,
        checkpoint: &Checkpoint,
        notify: &Notify,
    ) {
        let mut state = self.state.lock().await;
        state.snapshot.stage = stage;
        state.snapshot.downloaded_bytes = checkpoint.bytes;
        state.snapshot.total_bytes = checkpoint.total;
        let snapshot = state.snapshot();
        drop(state);
        notify(snapshot);
    }

    async fn save_checkpoint(
        &self,
        checkpoint: &Checkpoint,
        file: &mut File,
    ) -> DownloadResult<()> {
        file.flush().await?;
        file.sync_all().await?;
        let mut state = self.state.lock().await;
        let mut journal = state.journal.clone();
        journal.checkpoint = checkpoint.clone();
        persist(&self.path, &journal)?;
        state.journal = journal;
        Ok(())
    }

    async fn download(
        &self,
        journal: &Journal,
        client: Client,
        token: &CancellationToken,
        notify: &Notify,
    ) -> DownloadResult<()> {
        let mut checkpoint = journal.checkpoint.clone();
        let mut file = open_part(&journal.part()).await?;
        let length = file.metadata().await?.len();
        if checkpoint.complete && length == checkpoint.bytes {
            return Ok(());
        }
        checkpoint.complete = false;
        checkpoint.bytes = checkpoint.bytes.min(length);
        if length != checkpoint.bytes {
            file.set_len(checkpoint.bytes).await?;
        }
        if checkpoint.bytes != journal.checkpoint.bytes || journal.checkpoint.complete {
            self.save_checkpoint(&checkpoint, &mut file).await?;
        }

        let url = download_url(journal.url.as_deref().unwrap_or_default())?;
        let request = || {
            client
                .get(url.clone())
                .header(reqwest::header::ACCEPT_ENCODING, "identity")
        };
        let result = self
            .receive(&request, &mut file, &mut checkpoint, token, notify)
            .await;
        if result.as_ref().is_ok_and(|complete| *complete) {
            checkpoint.complete = true;
            self.report("saving", &checkpoint, notify).await;
        }
        self.save_checkpoint(&checkpoint, &mut file).await?;
        result.map(|_| ())
    }

    async fn receive(
        &self,
        request: &impl Fn() -> RequestBuilder,
        file: &mut File,
        checkpoint: &mut Checkpoint,
        token: &CancellationToken,
        notify: &Notify,
    ) -> DownloadResult<bool> {
        let mut parallel = true;
        loop {
            let download = tokio::select! {
                biased;
                _ = token.cancelled() => return Ok(false),
                body = open_body(request, checkpoint, parallel) => body?,
            };
            let (mut body, offset) = download;
            if offset == 0 && checkpoint.bytes != 0 {
                file.set_len(0).await?;
                // Clear the old prefix on disk before allowing another representation to write.
                *checkpoint = Checkpoint::default();
                self.save_checkpoint(checkpoint, file).await?;
            }
            file.seek(std::io::SeekFrom::Start(offset)).await?;
            checkpoint.bytes = offset;
            checkpoint.total = body.total;
            checkpoint.source = body.source().cloned();
            self.report("downloading", checkpoint, notify).await;

            let mut last_report = Instant::now();
            let mut last_checkpoint = Instant::now();
            let mut saved = offset;
            loop {
                let chunk = tokio::select! {
                    biased;
                    _ = token.cancelled() => return Ok(false),
                    chunk = body.stream.next() => chunk,
                };
                let Some(chunk) = chunk else {
                    if checkpoint.total.is_some_and(|total| checkpoint.bytes != total) {
                        return Err(ExternalSaveError::new("Incomplete download"));
                    }
                    return Ok(true);
                };
                let chunk = match chunk {
                    Ok(chunk) => chunk,
                    Err(_) if body.is_segmented() => {
                        parallel = false;
                        break;
                    }
                    Err(_) => return Err(ExternalSaveError::new("Download interrupted")),
                };
                let next = checkpoint.bytes + chunk.len() as u64;
                if checkpoint.total.is_some_and(|total| next > total) {
                    return Err(ExternalSaveError::new("Invalid download length"));
                }
                file.write_all(&chunk).await?;
                checkpoint.bytes = next;
                if last_report.elapsed() >= REPORT_INTERVAL {
                    self.report("downloading", checkpoint, notify).await;
                    last_report = Instant::now();
                }
                if last_checkpoint.elapsed() >= CHECKPOINT_INTERVAL
                    || checkpoint.bytes - saved >= CHECKPOINT_BYTES
                {
                    self.save_checkpoint(checkpoint, file).await?;
                    saved = checkpoint.bytes;
                    last_checkpoint = Instant::now();
                }
            }
        }
    }

    async fn settle(&self, result: DownloadResult<()>) -> DownloadSnapshot {
        let mut state = self.state.lock().await;
        let action = state.active.take().and_then(|active| active.stop);
        let stop_result = match action {
            Some(DownloadAction::Cancel) => self.cancel(&mut state).await,
            Some(DownloadAction::Pause) if result.is_ok() => {
                state.journal.status = Status::Paused;
                persist(&self.path, &state.journal)
            }
            _ => match result {
                Ok(()) if state.journal.checkpoint.complete => {
                    self.publish(&mut state).await
                }
                Ok(()) => Err(ExternalSaveError::new("Incomplete download")),
                Err(error) => Err(error),
            },
        };
        if let Err(error) = stop_result {
            state.journal.error = Some(error.message);
            if state.journal.status != Status::Canceled {
                state.journal.finish(Status::Failed);
            }
            if persist(&self.path, &state.journal).is_err() {
                tracing::warn!(id = %state.journal.id, "Could not save external download state");
            }
        }
        state.apply_checkpoint();
        state.snapshot()
    }

    async fn publish(&self, state: &mut JobState) -> DownloadResult<()> {
        state.journal.status = Status::Publishing;
        persist(&self.path, &state.journal)?;
        fs::rename(state.journal.part(), &state.journal.destination).await?;
        state.journal.finish(Status::Succeeded);
        if persist(&self.path, &state.journal).is_err() {
            tracing::warn!(id = %state.journal.id, "Could not save completed external download state");
        }
        Ok(())
    }
}

fn persist(path: &Path, journal: &Journal) -> DownloadResult<()> {
    let directory = path
        .parent()
        .ok_or_else(|| ExternalSaveError::new("Invalid checkpoint directory"))?;
    let mut temporary = tempfile::NamedTempFile::new_in(directory)?;
    serde_json::to_writer(temporary.as_file_mut(), journal)
        .map_err(|_| ExternalSaveError::new("Could not save download checkpoint"))?;
    temporary.as_file().sync_all()?;
    temporary.persist(path).map_err(|error| error.error)?;
    Ok(())
}

async fn open_part(path: &Path) -> DownloadResult<File> {
    let mut options = OpenOptions::new();
    options.read(true).write(true);
    match fs::symlink_metadata(path).await {
        Ok(metadata) if metadata.is_file() => {}
        Ok(_) => return Err(ExternalSaveError::new("Invalid download part file")),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            options.create_new(true);
            #[cfg(unix)]
            options.mode(0o600);
        }
        Err(error) => return Err(error.into()),
    }
    Ok(options.open(path).await?)
}

async fn remove_if_present(path: &Path) -> DownloadResult<()> {
    match fs::remove_file(path).await {
        Ok(()) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error.into()),
    }
}

async fn open_body(
    request: &impl Fn() -> RequestBuilder,
    checkpoint: &Checkpoint,
    parallel: bool,
) -> DownloadResult<(DownloadBody, u64)> {
    if let Some(source) = &checkpoint.source
        && let Some(body) = source
            .resume(request(), checkpoint.bytes, parallel)
            .await
            .map_err(|_| ExternalSaveError::new("Download interrupted"))?
    {
        return Ok((body, checkpoint.bytes));
    }
    let response = request().send().await?.error_for_status()?;
    if response.status() != StatusCode::OK
        || response.headers().contains_key(reqwest::header::CONTENT_RANGE)
    {
        return Err(ExternalSaveError::new("Invalid full download response"));
    }
    let body = if parallel {
        theseus::download_body(request(), response).await
    } else {
        DownloadBody::sequential(response)
    };
    Ok((body, 0))
}

fn download_url(value: &str) -> DownloadResult<url::Url> {
    let mut url = url::Url::parse(value)
        .map_err(|_| ExternalSaveError::new("Invalid download URL"))?;
    if !matches!(url.scheme(), "http" | "https")
        || !url.username().is_empty()
        || url.password().is_some()
    {
        return Err(ExternalSaveError::new("Invalid download URL"));
    }
    url.set_fragment(None);
    Ok(url)
}

fn download_client() -> DownloadResult<Client> {
    Ok(theseus::client_builder_with(&theseus::http_proxy_config())
        .connect_timeout(Duration::from_secs(30))
        .read_timeout(Duration::from_secs(60))
        .build()?)
}

fn notify_window<R: Runtime>(window: tauri::WebviewWindow<R>) -> Notify {
    Arc::new(move |snapshot| {
        let _ = window.emit(PROGRESS_EVENT, snapshot);
    })
}

#[tauri::command]
pub async fn files_create_url_download<R: Runtime>(
    window: tauri::WebviewWindow<R>,
    state: tauri::State<'_, ExternalDownloads>,
    url: String,
    filename: String,
) -> DownloadResult<Option<DownloadSnapshot>> {
    let url = download_url(&url)?;
    let Some(destination) = select_external_destination(&window, &filename, None).await?
    else {
        return Ok(None);
    };
    let client = download_client()?;
    let job = state.create(destination, url.to_string()).await?;
    job.start(client, notify_window(window)).await.map(Some)
}

#[tauri::command]
pub async fn files_list_url_downloads(
    state: tauri::State<'_, ExternalDownloads>,
) -> DownloadResult<Vec<DownloadSnapshot>> {
    state.list().await
}

#[tauri::command]
pub async fn files_start_url_download<R: Runtime>(
    window: tauri::WebviewWindow<R>,
    state: tauri::State<'_, ExternalDownloads>,
    id: Uuid,
) -> DownloadResult<DownloadSnapshot> {
    state.find(id).await?.start(download_client()?, notify_window(window)).await
}

#[tauri::command]
pub async fn files_control_url_download<R: Runtime>(
    window: tauri::WebviewWindow<R>,
    state: tauri::State<'_, ExternalDownloads>,
    id: Uuid,
    action: DownloadAction,
) -> DownloadResult<Option<DownloadSnapshot>> {
    let result = state.find(id).await?.control(action, &notify_window(window)).await?;
    if result.is_none() {
        state.jobs().await?.remove(&id);
    }
    Ok(result)
}

#[cfg(test)]
mod tests;
