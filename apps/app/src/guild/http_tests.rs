use super::*;
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::io::{AsyncBufReadExt, BufReader};
use tokio::task::JoinHandle;

const TEST_BUDGET: Duration = Duration::from_secs(3);
const OK: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\nok";
const PARTIAL: &[u8] = b"HTTP/1.1 200 OK\r\nContent-Length: 2\r\nConnection: close\r\n\r\no";
const UNAVAILABLE: &[u8] =
	b"HTTP/1.1 503 Service Unavailable\r\nContent-Length: 0\r\nConnection: close\r\n\r\n";

#[derive(Clone, Copy)]
enum Reply {
	Send(&'static [u8]),
	Stall(&'static [u8]),
	Reset,
}

struct Server {
	url: String,
	requests: Arc<AtomicUsize>,
	task: JoinHandle<()>,
}

impl Server {
	async fn start(replies: impl IntoIterator<Item = Reply>) -> Self {
		let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
		let url = format!("http://{}", listener.local_addr().unwrap());
		let requests = Arc::new(AtomicUsize::new(0));
		let counter = requests.clone();
		let replies: Vec<_> = replies.into_iter().collect();
		let task = tokio::spawn(async move {
			for reply in replies {
				let (socket, _) = listener.accept().await.unwrap();
				let mut socket = BufReader::new(socket);
				let mut line = Vec::new();
				loop {
					line.clear();
					assert_ne!(socket.read_until(b'\n', &mut line).await.unwrap(), 0);
					if line == b"\r\n" {
						break;
					}
				}
				counter.fetch_add(1, Ordering::Relaxed);
				let mut socket = socket.into_inner();
				match reply {
					Reply::Send(bytes) => socket.write_all(bytes).await.unwrap(),
					Reply::Stall(bytes) => {
						socket.write_all(bytes).await.unwrap();
						sleep(TEST_BUDGET).await;
					}
					Reply::Reset => socket.set_zero_linger().unwrap(),
				}
			}
		});
		Self { url, requests, task }
	}

	fn count(&self) -> usize {
		self.requests.load(Ordering::Relaxed)
	}

	async fn read(&self, budget: Duration) -> Result<String, GuildApiError> {
		get_with_retry(client().get(&self.url), budget, read_attachment_text).await
	}
}

impl Drop for Server {
	fn drop(&mut self) {
		self.task.abort();
	}
}

fn client_builder() -> reqwest::ClientBuilder {
	reqwest::Client::builder().no_proxy().retry(reqwest::retry::never())
}

fn client() -> reqwest::Client {
	client_builder().build().unwrap()
}

#[tokio::test]
async fn get_recovers_from_a_reset_or_incomplete_body_once() {
	for fault in [Reply::Reset, Reply::Send(PARTIAL)] {
		let server = Server::start([fault, Reply::Send(OK)]).await;
		assert_eq!(server.read(TEST_BUDGET).await.unwrap(), "ok");
		assert_eq!(server.count(), 2);
	}
}

#[tokio::test]
async fn get_stops_after_the_second_reset_and_post_sends_once() {
	let server = Server::start([Reply::Reset, Reply::Reset, Reply::Send(OK)]).await;
	let error = server.read(TEST_BUDGET).await.unwrap_err();
	assert_eq!(error.code, "network");
	assert!(error.message.contains("连接被重置"), "{error:?}");
	assert_eq!(server.count(), 2);

	let server = Server::start([Reply::Reset, Reply::Send(OK)]).await;
	let failure = send_request(client().post(&server.url), TEST_BUDGET).await.unwrap_err();
	assert_eq!(failure, NetworkFailure::Reset);
	assert_eq!(server.count(), 1);
}

#[tokio::test]
async fn get_preserves_http_error_without_retrying() {
	let server = Server::start([Reply::Send(UNAVAILABLE), Reply::Send(OK)]).await;
	let error = server.read(TEST_BUDGET).await.unwrap_err();
	assert_eq!(error.code, "server");
	assert!(error.message.contains("503"));
	assert_eq!(server.count(), 1);
}

#[tokio::test]
async fn get_distinguishes_response_and_body_timeouts_within_its_budget() {
	let budget = Duration::from_millis(800);
	for (bytes, message) in [(b"".as_slice(), "等待官网响应超时"), (PARTIAL, "读取官网响应超时")] {
		let server = Server::start([Reply::Stall(bytes)]).await;
		let error = server.read(budget).await.unwrap_err();
		assert_eq!(error.code, "network");
		assert!(error.message.contains(message), "{error:?}");
		assert_eq!(server.count(), 1);
	}
	let server = Server::start([Reply::Reset, Reply::Stall(PARTIAL)]).await;
	let budget = Duration::from_secs(2);
	let start = Instant::now();
	let error = server.read(budget).await.unwrap_err();
	assert!(error.message.contains("读取官网响应超时"), "{error:?}");
	assert!(start.elapsed() < budget + RETRY_DELAY, "{:?}", start.elapsed());
	assert_eq!(server.count(), 2);
}

#[tokio::test]
async fn stalled_tls_handshake_reports_a_connection_timeout() {
	let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
	let url = format!("https://{}/?private=local-fixture", listener.local_addr().unwrap());
	let client = client_builder()
		.https_only(true)
		.connect_timeout(Duration::from_millis(200))
		.build()
		.unwrap();
	let (connection, result) = tokio::join!(
		listener.accept(),
		send_request(client.get(url), TEST_BUDGET)
	);
	let _connection = connection.unwrap();
	let failure = result.unwrap_err();
	assert_eq!(failure, NetworkFailure::ConnectTimeout);
	assert!(!failure.into_api_error().message.contains("local-fixture"));
}
