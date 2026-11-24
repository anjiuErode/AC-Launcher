use crate::util::fetch::DOWNLOAD_META_HEADER;
use anyhow::{anyhow, bail};
use bytes::{Bytes, BytesMut};
use futures::{Stream, StreamExt, stream};
use reqwest::header::{self, HeaderMap, HeaderName, HeaderValue};
use reqwest::{
    Client, Method, Request, RequestBuilder, Response, StatusCode,
};
use serde::{Deserialize, Serialize};
use std::{pin::Pin, sync::Arc, time::Duration};
use tokio::sync::Semaphore;

const MIN_SEGMENTED_SIZE: u64 = 8 * 1024 * 1024;
const PROBE_SIZE: u64 = 256 * 1024;
const SEGMENT_SIZE: u64 = 2 * 1024 * 1024;
const PARALLEL_SEGMENTS: usize = 4;
const PROBE_TIMEOUT: Duration = Duration::from_secs(10);
static SEGMENT_CREDITS: Semaphore = Semaphore::const_new(16);

/// An ordered byte stream for a file download.
pub struct DownloadBody {
    pub total: Option<u64>,
    pub stream: Pin<Box<dyn Stream<Item = crate::Result<Bytes>> + Send>>,
    source: Option<DownloadSource>,
    segmented: bool,
}

impl DownloadBody {
    /// Whether this stream is assembled from HTTP byte ranges.
    pub fn is_segmented(&self) -> bool {
        self.segmented
    }

    pub fn source(&self) -> Option<&DownloadSource> {
        self.source.as_ref()
    }

    /// Streams the original response without issuing any range requests.
    pub fn sequential(response: Response) -> Self {
        Self {
            total: response.content_length(),
            source: DownloadSource::from_response(&response),
            stream: Box::pin(response.bytes_stream().map(|result| {
                result.map_err(|error| error.without_url().into())
            })),
            segmented: false,
        }
    }
}

/// Identifies the representation that produced an on-disk download prefix.
#[derive(Clone, Deserialize, Serialize)]
pub struct DownloadSource {
    url: String,
    etag: Vec<u8>,
    pub total: u64,
}

impl DownloadSource {
    fn from_response(response: &Response) -> Option<Self> {
        let headers = response.headers();
        let total = response.content_length()?;
        if response.status() != StatusCode::OK
            || !identity_encoding(headers)
            || !file_content(headers)
            || headers.contains_key(header::CONTENT_RANGE)
            || !exact_length(response, total)
        {
            return None;
        }
        Some(Self {
            url: response.url().as_str().to_owned(),
            etag: strong_etag(headers)?.as_bytes().to_vec(),
            total,
        })
    }

    /// Returns a stream starting at the saved offset when the server confirms it.
    /// An unconfirmed prefix must be discarded before consuming a new full body.
    pub async fn resume(
        &self,
        request: RequestBuilder,
        offset: u64,
        parallel: bool,
    ) -> crate::Result<Option<DownloadBody>> {
        if offset == 0 || offset >= self.total {
            return Ok(None);
        }
        let Some(source) = RangeSource::new(request, self.clone()) else {
            return Ok(None);
        };
        if parallel
            && self.total - offset >= MIN_SEGMENTED_SIZE
            && let Ok(credits) =
                SEGMENT_CREDITS.try_acquire_many(PARALLEL_SEGMENTS as u32)
        {
            let next = offset + PROBE_SIZE;
            let Some(response) = source.response(offset, next).await? else {
                return Ok(None);
            };
            let first = read_range(response, next - offset).await?;
            return Ok(Some(segmented_body(source, first, next, credits)));
        }
        Ok(source.response(offset, self.total).await?.map(|response| {
            let mut body = DownloadBody::sequential(response);
            body.source = Some(self.clone());
            body.total = Some(self.total);
            body
        }))
    }
}

/// Streams a file from its original response or parallel validated ranges.
///
/// Supply the original GET request so ranges inherit its client, proxy,
/// timeout and redirect policy. Use this only for known file endpoints.
pub async fn download_body(
    request: RequestBuilder,
    response: Response,
) -> DownloadBody {
    let Some(checkpoint) = DownloadSource::from_response(&response)
        .filter(|source| source.total >= MIN_SEGMENTED_SIZE)
    else {
        return DownloadBody::sequential(response);
    };
    if response
        .headers()
        .get_all(header::ACCEPT_RANGES)
        .iter()
        .any(|value| value.as_bytes().eq_ignore_ascii_case(b"none"))
    {
        return DownloadBody::sequential(response);
    }
    let Some(source) = RangeSource::new(request, checkpoint) else {
        return DownloadBody::sequential(response);
    };
    let Ok(credits) = SEGMENT_CREDITS.try_acquire_many(PARALLEL_SEGMENTS as u32)
    else {
        return DownloadBody::sequential(response);
    };

    let Ok(Ok(first)) =
        tokio::time::timeout(PROBE_TIMEOUT, source.read(0, PROBE_SIZE)).await
    else {
        return DownloadBody::sequential(response);
    };
    drop(response);

    segmented_body(source, first, PROBE_SIZE, credits)
}

fn segmented_body(
    source: RangeSource,
    first: Bytes,
    next: u64,
    credits: tokio::sync::SemaphorePermit<'static>,
) -> DownloadBody {
    let checkpoint = source.checkpoint.clone();
    let total = checkpoint.total;
    let source = Arc::new(source);
    let bounds = std::iter::successors(Some(next), move |start| {
        start.checked_add(SEGMENT_SIZE).filter(|next| *next < total)
    });
    let ranges = stream::iter(bounds)
        .map(move |start| {
            let source = Arc::clone(&source);
            async move {
                let end = start.saturating_add(SEGMENT_SIZE).min(total);
                source.read(start, end).await
            }
        })
        .buffered(PARALLEL_SEGMENTS);
    let stream = Box::pin(stream::once(async { Ok(first) }).chain(ranges));
    let stream = stream::try_unfold((stream, credits), |(mut stream, credits)| {
        async move {
            match stream.next().await {
                Some(bytes) => Ok(Some((bytes?, (stream, credits)))),
                None => Ok(None),
            }
        }
    });
    DownloadBody {
        total: Some(total),
        stream: Box::pin(stream),
        source: Some(checkpoint),
        segmented: true,
    }
}

struct RangeSource {
    client: Client,
    request: Request,
    checkpoint: DownloadSource,
    etag: HeaderValue,
}

impl RangeSource {
    fn new(
        request: RequestBuilder,
        checkpoint: DownloadSource,
    ) -> Option<Self> {
        let (client, request) = request.build_split();
        let mut request = request.ok()?;
        if request.method() != Method::GET
            || request.body().is_some()
            || !matches!(request.url().scheme(), "http" | "https")
            || [
                header::RANGE,
                header::IF_RANGE,
                header::IF_MATCH,
                header::IF_NONE_MATCH,
                header::IF_MODIFIED_SINCE,
                header::IF_UNMODIFIED_SINCE,
            ]
            .iter()
            .any(|name| request.headers().contains_key(name))
        {
            return None;
        }
        let etag = HeaderValue::from_bytes(&checkpoint.etag).ok()?;
        let headers = request.headers_mut();
        headers.remove(DOWNLOAD_META_HEADER);
        headers.insert(
            header::ACCEPT_ENCODING,
            HeaderValue::from_static("identity"),
        );
        headers.insert(header::IF_RANGE, etag.clone());
        Some(Self {
            client,
            request,
            checkpoint,
            etag,
        })
    }

    async fn response(
        &self,
        start: u64,
        end: u64,
    ) -> crate::Result<Option<Response>> {
        let mut request = self.request.try_clone().ok_or_else(|| {
            anyhow!("file request cannot be cloned for a byte range")
        })?;
        let last = end - 1;
        let range = HeaderValue::from_str(&format!("bytes={start}-{last}"))
            .map_err(|_| anyhow!("invalid HTTP byte range"))?;
        request.headers_mut().insert(header::RANGE, range);
        let response = self
            .client
            .execute(request)
            .await
            .map_err(reqwest::Error::without_url)?;
        if !matches!(
            response.status(),
            StatusCode::PRECONDITION_FAILED | StatusCode::RANGE_NOT_SATISFIABLE
        ) {
            response
                .error_for_status_ref()
                .map_err(reqwest::Error::without_url)?;
        }
        let headers = response.headers();
        if response.status() != StatusCode::PARTIAL_CONTENT
            || response.url().as_str() != self.checkpoint.url
            || strong_etag(headers) != Some(&self.etag)
            || !identity_encoding(headers)
            || !file_content(headers)
            || !exact_length(&response, end - start)
            || byte_range(headers) != Some((start, last, self.checkpoint.total))
        {
            return Ok(None);
        }
        Ok(Some(response))
    }

    async fn read(&self, start: u64, end: u64) -> crate::Result<Bytes> {
        let Some(response) = self.response(start, end).await? else {
            bail!("server returned an invalid response for an HTTP byte range");
        };
        read_range(response, end - start).await
    }
}

async fn read_range(mut response: Response, length: u64) -> crate::Result<Bytes> {
    let length = length as usize;
    let mut body = BytesMut::with_capacity(length);
    while let Some(chunk) = response
        .chunk()
        .await
        .map_err(reqwest::Error::without_url)?
    {
        if chunk.len() > length - body.len() {
            bail!("server returned too many bytes for an HTTP byte range");
        }
        body.extend_from_slice(&chunk);
    }
    if body.len() != length {
        bail!("server returned too few bytes for an HTTP byte range");
    }
    Ok(body.freeze())
}

fn single_header(
    headers: &HeaderMap,
    name: HeaderName,
) -> Option<&HeaderValue> {
    let mut values = headers.get_all(name).into_iter();
    let value = values.next()?;
    values.next().is_none().then_some(value)
}

fn byte_range(headers: &HeaderMap) -> Option<(u64, u64, u64)> {
    let value = single_header(headers, header::CONTENT_RANGE)?.to_str().ok()?;
    let (unit, value) = value.split_once(' ')?;
    if !unit.eq_ignore_ascii_case("bytes") {
        return None;
    }
    let (range, total) = value.split_once('/')?;
    let (start, last) = range.split_once('-')?;
    Some((decimal(start)?, decimal(last)?, decimal(total)?))
}

fn decimal(value: &str) -> Option<u64> {
    if value.bytes().all(|byte| byte.is_ascii_digit()) {
        value.parse().ok()
    } else {
        None
    }
}

fn strong_etag(headers: &HeaderMap) -> Option<&HeaderValue> {
    let value = single_header(headers, header::ETAG)?;
    let [b'"', tag @ .., b'"'] = value.as_bytes() else {
        return None;
    };
    tag.iter()
        .all(|byte| matches!(byte, 0x21 | 0x23..=0x7e | 0x80..=0xff))
        .then_some(value)
}

fn identity_encoding(headers: &HeaderMap) -> bool {
    !headers.contains_key(header::CONTENT_ENCODING)
        || single_header(headers, header::CONTENT_ENCODING).is_some_and(
            |value| value.as_bytes().eq_ignore_ascii_case(b"identity"),
        )
}

fn exact_length(response: &Response, expected: u64) -> bool {
    // reqwest removes Content-Length when it decodes an encoded body.
    response.content_length() == Some(expected)
        && single_header(response.headers(), header::CONTENT_LENGTH)
            .and_then(|value| decimal(value.to_str().ok()?))
            == Some(expected)
}

fn file_content(headers: &HeaderMap) -> bool {
    !headers.contains_key(header::CONTENT_TYPE)
        || single_header(headers, header::CONTENT_TYPE)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| {
                let media = value
                    .split(';')
                    .next()
                    .unwrap_or_default()
                    .trim()
                    .to_ascii_lowercase();
                !matches!(media.as_str(), "application/json" | "text/html")
                    && !media.ends_with("+json")
                    && !media.starts_with("multipart/")
            })
}

#[cfg(test)]
mod tests {
    use super::*;
    use futures::TryStreamExt;
    use tokio::io::{AsyncBufReadExt, AsyncWriteExt, BufReader};
    use tokio::net::{TcpListener, TcpStream};
    use tokio::sync::{Barrier, mpsc};

    const FILE_SIZE: usize = MIN_SEGMENTED_SIZE as usize + 257;
    const ETAG: &str = "\"file-v1\"";

    #[derive(Clone, Copy)]
    enum RangeBehavior {
        Supported,
        Ignored,
        Changed,
    }

    async fn file_server(
        behavior: RangeBehavior,
    ) -> (
        RequestBuilder,
        mpsc::UnboundedReceiver<(usize, bool)>,
        tokio::task::JoinHandle<()>,
    ) {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let client = Client::builder().no_proxy().build().unwrap();
        let request = client
            .get(format!("http://{}", listener.local_addr().unwrap()))
            .header(DOWNLOAD_META_HEADER, "test");
        let (observations, received) = mpsc::unbounded_channel();
        let bytes: Arc<[u8]> = (0..FILE_SIZE)
            .map(|offset| (offset / (64 * 1024)) as u8)
            .collect();
        let barrier = Arc::new(Barrier::new(PARALLEL_SEGMENTS));
        let task = tokio::spawn(async move {
            while let Ok((socket, _)) = listener.accept().await {
                let bytes = Arc::clone(&bytes);
                let barrier = Arc::clone(&barrier);
                let observations = observations.clone();
                tokio::spawn(async move {
                    let _ = serve_file(
                        socket,
                        &bytes,
                        behavior,
                        &barrier,
                        &observations,
                    )
                    .await;
                });
            }
        });
        (request, received, task)
    }

    async fn serve_file(
        mut socket: TcpStream,
        bytes: &[u8],
        behavior: RangeBehavior,
        barrier: &Barrier,
        observations: &mpsc::UnboundedSender<(usize, bool)>,
    ) -> std::io::Result<()> {
        let mut lines = BufReader::new(&mut socket).lines();
        let mut range = None;
        let mut has_metadata = false;
        let mut identity = false;
        let mut if_range = false;
        while let Some(line) = lines.next_line().await? {
            if line.is_empty() {
                break;
            }
            if let Some(value) = line.strip_prefix("range: bytes=") {
                let (start, last) = value.split_once('-').unwrap();
                range = Some((
                    start.parse::<usize>().unwrap(),
                    last.parse::<usize>().unwrap(),
                ));
            } else if line.starts_with("modrinth-download-meta:") {
                has_metadata = true;
            } else if line == "accept-encoding: identity" {
                identity = true;
            } else if line == format!("if-range: {ETAG}") {
                if_range = true;
            }
        }
        let _ = observations.send((
            range.map_or(usize::MAX, |(start, _)| start),
            has_metadata,
        ));
        let partial = range.filter(|_| {
            identity && if_range && !matches!(behavior, RangeBehavior::Ignored)
        });
        let (status, extra, etag, body) = if let Some((start, last)) = partial {
            if matches!(behavior, RangeBehavior::Supported)
                && start >= PROBE_SIZE as usize
                && start < (PROBE_SIZE + PARALLEL_SEGMENTS as u64 * SEGMENT_SIZE) as usize
            {
                barrier.wait().await;
            }
            let etag =
                if start > 0 && matches!(behavior, RangeBehavior::Changed) {
                    "\"file-v2\""
                } else {
                    ETAG
                };
            (
                "206 Partial Content",
                format!("Content-Range: bytes {start}-{last}/{FILE_SIZE}\r\n"),
                etag,
                &bytes[start..=last],
            )
        } else {
            ("200 OK", String::new(), ETAG, bytes)
        };
        let headers = format!(
            "HTTP/1.1 {status}\r\nAccept-Ranges: bytes\r\nETag: {etag}\r\n\
             Content-Length: {}\r\n{extra}Connection: close\r\n\r\n",
            body.len(),
        );
        socket.write_all(headers.as_bytes()).await?;
        socket.write_all(body).await
    }

    fn assert_file(chunks: &[Bytes]) {
        let bytes = chunks.iter().flatten().copied();
        assert_eq!(chunks.iter().map(Bytes::len).sum::<usize>(), FILE_SIZE);
        assert!(
            bytes
                .enumerate()
                .all(|(offset, byte)| byte == (offset / (64 * 1024)) as u8)
        );
    }

    #[tokio::test]
    async fn streams_concurrent_ranges_in_order_without_replaying_metadata() {
        let (request, mut observations, server) =
            file_server(RangeBehavior::Supported).await;
        let response = request.try_clone().unwrap().send().await.unwrap();
        let body = download_body(request, response).await;
        assert!(body.is_segmented());
        assert_eq!(body.total, Some(FILE_SIZE as u64));
        let chunks: Vec<Bytes> = tokio::time::timeout(
            Duration::from_secs(10),
            body.stream.try_collect(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_file(&chunks);
        server.abort();

        assert_eq!(observations.try_recv().unwrap(), (usize::MAX, true));
        assert_eq!(observations.try_recv().unwrap(), (0, false));
        let mut starts = Vec::new();
        while let Ok((start, has_metadata)) = observations.try_recv() {
            assert!(!has_metadata);
            starts.push(start);
        }
        starts.sort_unstable();
        assert_eq!(
            starts,
            (PROBE_SIZE as usize..FILE_SIZE)
                .step_by(SEGMENT_SIZE as usize)
                .collect::<Vec<_>>(),
        );
    }

    #[tokio::test]
    async fn resumed_large_file_keeps_parallel_ranges() {
        let (request, mut observations, server) =
            file_server(RangeBehavior::Supported).await;
        let response = request.try_clone().unwrap().send().await.unwrap();
        let source = DownloadSource::from_response(&response).unwrap();
        drop(response);
        let offset = 137;
        let body = source.resume(request, offset, true).await.unwrap().unwrap();
        assert!(body.is_segmented());
        assert_eq!(body.total, Some(FILE_SIZE as u64));

        let chunks: Vec<Bytes> = tokio::time::timeout(
            Duration::from_secs(10),
            body.stream.try_collect(),
        )
        .await
        .unwrap()
        .unwrap();
        assert_eq!(
            chunks.iter().map(Bytes::len).sum::<usize>(),
            FILE_SIZE - offset as usize,
        );
        assert!(chunks.iter().flatten().copied().enumerate().all(
            |(position, byte)| byte == ((position + offset as usize) / (64 * 1024)) as u8
        ));
        assert_eq!(observations.try_recv().unwrap(), (usize::MAX, true));
        assert_eq!(observations.try_recv().unwrap(), (offset as usize, false));
        server.abort();
    }

    #[tokio::test]
    async fn ignored_range_keeps_the_original_response() {
        let (request, mut observations, server) =
            file_server(RangeBehavior::Ignored).await;
        let response = request.try_clone().unwrap().send().await.unwrap();
        let body = download_body(request, response).await;
        assert!(!body.is_segmented());
        let chunks: Vec<Bytes> = body.stream.try_collect().await.unwrap();
        assert_file(&chunks);
        server.abort();

        assert_eq!(observations.try_recv().unwrap(), (usize::MAX, true));
        assert_eq!(observations.try_recv().unwrap(), (0, false));
        assert!(observations.try_recv().is_err());
    }

    #[tokio::test]
    async fn changed_etag_stops_the_segmented_stream() {
        let (request, _, server) = file_server(RangeBehavior::Changed).await;
        let response = request.try_clone().unwrap().send().await.unwrap();
        let mut body = download_body(request, response).await;
        assert!(body.is_segmented());
        assert_eq!(
            body.stream.try_next().await.unwrap().unwrap().len(),
            PROBE_SIZE as usize
        );
        assert!(body.stream.try_next().await.is_err());
        assert!(body.stream.try_next().await.unwrap().is_none());
        server.abort();
    }
}
