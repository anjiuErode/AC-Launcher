use super::*;
use std::io::Write;
use tokio::io::{AsyncBufReadExt, AsyncReadExt, BufReader};
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use tokio::time::{sleep, timeout};

const TOTAL: usize = 128 * 1024;
const PREFIX: usize = 48 * 1024;
const ORIGINAL_TAG: &str = "\"first\"";
const UPDATED_TAG: &str = "\"changed\"";
const TIMEOUT: Duration = Duration::from_secs(10);

struct Reply {
    status: &'static str,
    etag: &'static str,
    range: Option<String>,
    length: usize,
    bytes: Vec<u8>,
    hold: bool,
}

impl Reply {
    fn full(etag: &'static str, bytes: &[u8]) -> Self {
        Self {
            status: "200 OK",
            etag,
            range: None,
            length: TOTAL,
            bytes: bytes.to_vec(),
            hold: false,
        }
    }

    fn prefix(bytes: &[u8], hold: bool) -> Self {
        Self {
            hold,
            ..Self::full(ORIGINAL_TAG, &bytes[..PREFIX])
        }
    }

    fn partial(etag: &'static str, bytes: &[u8]) -> Self {
        Self {
            status: "206 Partial Content",
            etag,
            range: Some(format!("bytes {PREFIX}-{}/{TOTAL}", TOTAL - 1)),
            length: TOTAL - PREFIX,
            bytes: bytes[PREFIX..].to_vec(),
            hold: false,
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
struct RequestHeaders {
    range: Option<String>,
    if_range: Option<String>,
}

struct Server {
    url: String,
    requests: mpsc::UnboundedReceiver<RequestHeaders>,
    task: JoinHandle<()>,
}

impl Server {
    async fn start(replies: Vec<Reply>) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let url = format!("http://{}/target.bin", listener.local_addr().unwrap());
        let (sender, requests) = mpsc::unbounded_channel();
        let task = tokio::spawn(async move {
            for reply in replies {
                let (socket, _) = listener.accept().await.unwrap();
                let sender = sender.clone();
                tokio::spawn(async move {
                    let _ = reply_to(socket, sender, reply).await;
                });
            }
        });
        Self {
            url,
            requests,
            task,
        }
    }

    async fn request(&mut self, expected_range: bool) {
        let actual = timeout(TIMEOUT, self.requests.recv())
            .await
            .unwrap()
            .unwrap();
        assert_eq!(
            actual,
            RequestHeaders {
                range: expected_range.then(|| format!("bytes={PREFIX}-{}", TOTAL - 1)),
                if_range: expected_range.then(|| ORIGINAL_TAG.to_owned()),
            }
        );
    }
}

impl Drop for Server {
    fn drop(&mut self) {
        self.task.abort();
    }
}

async fn reply_to(
    mut socket: TcpStream,
    sender: mpsc::UnboundedSender<RequestHeaders>,
    reply: Reply,
) -> std::io::Result<()> {
    let mut lines = BufReader::new(&mut socket).lines();
    let mut request = RequestHeaders {
        range: None,
        if_range: None,
    };
    while let Some(line) = lines.next_line().await? {
        if line.is_empty() {
            break;
        }
        if let Some((name, value)) = line.split_once(':') {
            if name.eq_ignore_ascii_case("range") {
                request.range = Some(value.trim().to_owned());
            } else if name.eq_ignore_ascii_case("if-range") {
                request.if_range = Some(value.trim().to_owned());
            }
        }
    }
    let _ = sender.send(request);
    let mut headers = format!(
        "HTTP/1.1 {}\r\nETag: {}\r\nContent-Length: {}\r\nContent-Type: application/octet-stream\r\nConnection: close\r\n",
        reply.status, reply.etag, reply.length,
    );
    if let Some(range) = reply.range {
        headers.push_str(&format!("Content-Range: {range}\r\n"));
    }
    headers.push_str("\r\n");
    socket.write_all(headers.as_bytes()).await?;
    socket.write_all(&reply.bytes).await?;
    if reply.hold {
        let _ = socket.read(&mut [0]).await?;
    }
    Ok(())
}

fn test_client() -> Client {
    Client::builder().no_proxy().build().unwrap()
}

fn original() -> Vec<u8> {
    (0..TOTAL).map(|byte| (byte % 251) as u8).collect()
}

fn observe() -> (Notify, mpsc::UnboundedReceiver<DownloadSnapshot>) {
    let (sender, receiver) = mpsc::unbounded_channel();
    let notify = Arc::new(move |snapshot| {
        let _ = sender.send(snapshot);
    });
    (notify, receiver)
}

async fn await_status(
    receiver: &mut mpsc::UnboundedReceiver<DownloadSnapshot>,
    status: Status,
) -> DownloadSnapshot {
    timeout(TIMEOUT, async {
        while let Some(snapshot) = receiver.recv().await {
            if snapshot.status == status {
                return snapshot;
            }
            assert_eq!(snapshot.status, Status::Running, "{:?}", snapshot.error);
        }
        panic!("download notification stream closed");
    })
    .await
    .unwrap()
}

async fn await_prefix(part: &Path) {
    timeout(TIMEOUT, async {
        while !fs::metadata(part)
            .await
            .is_ok_and(|metadata| metadata.len() == PREFIX as u64)
        {
            sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
}

#[tokio::test]
async fn reopens_a_running_job_and_resumes_exactly_at_its_durable_checkpoint() {
    let bytes = original();
    let mut server = Server::start(vec![
        Reply::prefix(&bytes, true),
        Reply::partial(ORIGINAL_TAG, &bytes),
    ])
    .await;
    let directory = tempfile::tempdir().unwrap();
    let metadata = directory.path().join("jobs");
    let output = directory.path().join("output.bin");
    let manager = ExternalDownloads::new(metadata.clone());
    let job = manager.create(output.clone(), server.url.clone()).await.unwrap();
    let part = job.state.lock().await.journal.part();
    let (notify, mut updates) = observe();
    let running = job.clone().start(test_client(), notify.clone()).await.unwrap();
    server.request(false).await;
    await_prefix(&part).await;
    let paused = job
        .control(DownloadAction::Pause, &notify)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(paused.status, Status::Paused);
    assert_eq!(paused.downloaded_bytes, PREFIX as u64);
    assert!(!output.exists());

    // Recreate the durable state of an exit before the next checkpoint.
    let mut journal = job.state.lock().await.journal.clone();
    journal.status = Status::Running;
    persist(&job.path, &journal).unwrap();
    std::fs::OpenOptions::new()
        .append(true)
        .open(&part)
        .unwrap()
        .write_all(b"uncommitted bytes")
        .unwrap();
    drop(job);
    drop(manager);
    while updates.try_recv().is_ok() {}

    let manager = ExternalDownloads::new(metadata);
    let restored = manager.list().await.unwrap();
    assert_eq!(restored.len(), 1);
    assert_eq!(restored[0].id, running.id);
    assert_eq!(restored[0].status, Status::Paused);
    assert_eq!(restored[0].downloaded_bytes, PREFIX as u64);
    let restored = manager.find(running.id).await.unwrap();
    restored.start(test_client(), notify.clone()).await.unwrap();
    server.request(true).await;
    await_status(&mut updates, Status::Succeeded).await;
    assert_eq!(fs::read(&output).await.unwrap(), bytes);
    assert!(!part.exists());
}

#[tokio::test]
async fn starts_over_after_an_ignored_range_or_invalid_partial_response() {
    let bytes = original();
    let replacement: Vec<_> = bytes.iter().map(|byte| byte ^ 255).collect();
    let mut wrong_range = Reply::partial(ORIGINAL_TAG, &replacement);
    wrong_range.range = Some(format!("bytes {}-{}/{TOTAL}", PREFIX - 1, TOTAL - 2));
    let mut wrong_length = Reply::partial(ORIGINAL_TAG, &replacement);
    wrong_length.length -= 1;
    let mut wrong_total = Reply::partial(ORIGINAL_TAG, &replacement);
    wrong_total.range = Some(format!("bytes {PREFIX}-{}/{}", TOTAL - 1, TOTAL + 1));
    for rejected_reply in [
        Reply::full(UPDATED_TAG, &replacement),
        Reply::partial(UPDATED_TAG, &replacement),
        wrong_range,
        wrong_length,
        wrong_total,
    ] {
        let mut server = Server::start(vec![
            Reply::prefix(&bytes, true),
            rejected_reply,
            Reply::full(UPDATED_TAG, &replacement),
        ])
        .await;
        let directory = tempfile::tempdir().unwrap();
        let output = directory.path().join("output.bin");
        let manager = ExternalDownloads::new(directory.path().join("jobs"));
        let job = manager.create(output.clone(), server.url.clone()).await.unwrap();
        let part = job.state.lock().await.journal.part();
        let (notify, mut updates) = observe();
        job.clone().start(test_client(), notify.clone()).await.unwrap();
        server.request(false).await;
        await_prefix(&part).await;
        let paused = job
            .control(DownloadAction::Pause, &notify)
            .await
            .unwrap()
            .unwrap();
        assert_eq!(paused.status, Status::Paused);
        assert!(!output.exists());
        while updates.try_recv().is_ok() {}

        job.start(test_client(), notify.clone()).await.unwrap();
        server.request(true).await;
        server.request(false).await;
        await_status(&mut updates, Status::Succeeded).await;
        assert_eq!(fs::read(&output).await.unwrap(), replacement);
        assert!(!part.exists());
    }
}

#[tokio::test]
async fn cancel_wins_over_pause_and_preserves_an_existing_target() {
    let bytes = original();
    let mut server = Server::start(vec![Reply::prefix(&bytes, true)]).await;
    let directory = tempfile::tempdir().unwrap();
    let metadata = directory.path().join("jobs");
    let output = directory.path().join("output.bin");
    let previous = b"preexisting output";
    fs::write(&output, previous).await.unwrap();
    let manager = ExternalDownloads::new(metadata.clone());
    let job = manager.create(output.clone(), server.url.clone()).await.unwrap();
    let part = job.state.lock().await.journal.part();
    let (notify, _updates) = observe();
    job.clone().start(test_client(), notify.clone()).await.unwrap();
    server.request(false).await;
    await_prefix(&part).await;
    let (_, canceled) = tokio::join!(
        job.control(DownloadAction::Pause, &notify),
        job.control(DownloadAction::Cancel, &notify),
    );
    assert_eq!(canceled.unwrap().unwrap().status, Status::Canceled);
    assert!(!part.exists());
    assert_eq!(fs::read(output).await.unwrap(), previous);
    drop(manager);
    assert_eq!(
        ExternalDownloads::new(metadata).list().await.unwrap()[0].status,
        Status::Canceled,
    );
}

#[tokio::test]
async fn retries_an_interrupted_body_without_early_publication() {
    let bytes = original();
    let mut server = Server::start(vec![
        Reply::prefix(&bytes, false),
        Reply::partial(ORIGINAL_TAG, &bytes),
    ])
    .await;
    let directory = tempfile::tempdir().unwrap();
    let output = directory.path().join("output.bin");
    let manager = ExternalDownloads::new(directory.path().join("jobs"));
    let job = manager.create(output.clone(), server.url.clone()).await.unwrap();
    let (notify, mut updates) = observe();
    job.clone().start(test_client(), notify.clone()).await.unwrap();
    server.request(false).await;
    let failed = await_status(&mut updates, Status::Failed).await;
    assert_eq!(failed.downloaded_bytes, PREFIX as u64);
    assert!(!output.exists());

    job.start(test_client(), notify.clone()).await.unwrap();
    server.request(true).await;
    await_status(&mut updates, Status::Succeeded).await;
    assert_eq!(fs::read(output).await.unwrap(), bytes);
}
