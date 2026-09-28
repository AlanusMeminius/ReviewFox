use std::any::type_name;
use std::sync::{LazyLock, OnceLock};
use std::{borrow::Cow, mem, pin::Pin, task::Poll, time::Duration};

use gpui_util::defer;

use anyhow::anyhow;
use bytes::{BufMut, Bytes, BytesMut};
use futures::{AsyncRead, FutureExt as _, TryStreamExt as _};
use gpui_http_client::{RedirectPolicy, Url, http};
use regex::Regex;
use zed_reqwest::{
    header::{HeaderMap, HeaderValue},
    redirect,
};

const DEFAULT_CAPACITY: usize = 4096;
static RUNTIME: OnceLock<tokio::runtime::Runtime> = OnceLock::new();
static REDACT_REGEX: LazyLock<Regex> = LazyLock::new(|| Regex::new(r"key=[^&]+").unwrap());

pub struct ReqwestClient {
    client: zed_reqwest::Client,
    proxy: Option<Url>,
    user_agent: Option<HeaderValue>,
    handle: tokio::runtime::Handle,
}

impl ReqwestClient {
    /// Shared connection-management configuration for every client this type
    /// builds. `read_timeout` sets an idle timeout on each body read (see
    /// [`ReqwestClient::proxy_user_agent_and_read_timeout`]); `None` leaves
    /// reads without a timeout.
    fn builder(read_timeout: Option<Duration>) -> zed_reqwest::ClientBuilder {
        let builder = zed_reqwest::Client::builder()
            .use_rustls_tls()
            .connect_timeout(Duration::from_secs(10))
            // Detect and drop connections that have silently gone bad on a
            // flaky path (NAT timeouts, resets) instead of reusing them. A
            // stale reused HTTP/2 connection is a common source of
            // `BadRecordMac` TLS errors against long-lived endpoints.
            .tcp_keepalive(Duration::from_secs(30))
            .pool_idle_timeout(Duration::from_secs(30))
            .http2_keep_alive_interval(Duration::from_secs(15))
            .http2_keep_alive_timeout(Duration::from_secs(10))
            .http2_keep_alive_while_idle(true);
        match read_timeout {
            Some(read_timeout) => builder.read_timeout(read_timeout),
            None => builder,
        }
    }

    pub fn new() -> Self {
        Self::builder(None)
            .build()
            .expect("Failed to initialize HTTP client")
            .into()
    }

    pub fn user_agent(agent: &str) -> anyhow::Result<Self> {
        let mut map = HeaderMap::new();
        map.insert(http::header::USER_AGENT, HeaderValue::from_str(agent)?);
        let client = Self::builder(None).default_headers(map).build()?;
        Ok(client.into())
    }
}

pub fn runtime() -> &'static tokio::runtime::Runtime {
    RUNTIME.get_or_init(|| {
        tokio::runtime::Builder::new_multi_thread()
            // Since we now have two executors, let's try to keep our footprint small
            .worker_threads(1)
            .enable_all()
            .build()
            .expect("Failed to initialize HTTP client")
    })
}

impl From<zed_reqwest::Client> for ReqwestClient {
    fn from(client: zed_reqwest::Client) -> Self {
        let handle = tokio::runtime::Handle::try_current().unwrap_or_else(|_| {
            log::debug!("no tokio runtime found, creating one for Reqwest...");
            runtime().handle().clone()
        });
        Self {
            client,
            handle,
            proxy: None,
            user_agent: None,
        }
    }
}

// This struct is essentially a re-implementation of
// https://docs.rs/tokio-util/0.7.12/tokio_util/io/struct.ReaderStream.html
// except outside of Tokio's aegis
struct StreamReader {
    reader: Option<Pin<Box<dyn futures::AsyncRead + Send + Sync>>>,
    buf: BytesMut,
    capacity: usize,
}

impl StreamReader {
    fn new(reader: Pin<Box<dyn futures::AsyncRead + Send + Sync>>) -> Self {
        Self {
            reader: Some(reader),
            buf: BytesMut::new(),
            capacity: DEFAULT_CAPACITY,
        }
    }
}

impl futures::Stream for StreamReader {
    type Item = std::io::Result<Bytes>;

    fn poll_next(
        mut self: Pin<&mut Self>,
        cx: &mut std::task::Context<'_>,
    ) -> Poll<Option<Self::Item>> {
        let mut this = self.as_mut();

        let mut reader = match this.reader.take() {
            Some(r) => r,
            None => return Poll::Ready(None),
        };

        if this.buf.capacity() == 0 {
            let capacity = this.capacity;
            this.buf.reserve(capacity);
        }

        match poll_read_buf(&mut reader, cx, &mut this.buf) {
            Poll::Pending => {
                self.reader = Some(reader);

                Poll::Pending
            }
            Poll::Ready(Err(err)) => {
                self.reader = None;

                Poll::Ready(Some(Err(err)))
            }
            Poll::Ready(Ok(0)) => {
                self.reader = None;
                Poll::Ready(None)
            }
            Poll::Ready(Ok(_)) => {
                let chunk = this.buf.split();
                self.reader = Some(reader);
                Poll::Ready(Some(Ok(chunk.freeze())))
            }
        }
    }
}

/// Implementation from <https://docs.rs/tokio-util/0.7.12/src/tokio_util/util/poll_buf.rs.html>
/// Specialized for this use case
fn poll_read_buf(
    io: &mut Pin<Box<dyn futures::AsyncRead + Send + Sync>>,
    cx: &mut std::task::Context<'_>,
    buf: &mut BytesMut,
) -> Poll<std::io::Result<usize>> {
    if !buf.has_remaining_mut() {
        return Poll::Ready(Ok(0));
    }

    let n = {
        let dst = buf.chunk_mut();

        // Safety: `chunk_mut()` returns a `&mut UninitSlice`, and `UninitSlice` is a
        // transparent wrapper around `[std::mem::MaybeUninit<u8>]`.
        let dst = unsafe { &mut *(dst as *mut _ as *mut [std::mem::MaybeUninit<u8>]) };
        let mut read_buf = tokio::io::ReadBuf::uninit(dst);
        let unfilled_portion = read_buf.initialize_unfilled();
        // SAFETY: Pin projection
        let io_pin = unsafe { Pin::new_unchecked(io) };
        // `futures::AsyncRead` reports the byte count as the poll's return
        // value; `read_buf.filled()` stays empty because the reader writes
        // through the initialized slice without advancing the `ReadBuf`.
        std::task::ready!(io_pin.poll_read(cx, unfilled_portion)?)
    };

    // Safety: `initialize_unfilled()` zero-initialized the entire spare
    // capacity, so the first `n` bytes are initialized no matter how many the
    // reader actually wrote, and `advance_mut` panics rather than exceeding
    // the capacity if `n` overstates the slice length.
    unsafe {
        buf.advance_mut(n);
    }

    Poll::Ready(Ok(n))
}

fn redact_error(mut error: zed_reqwest::Error) -> zed_reqwest::Error {
    if let Some(url) = error.url_mut()
        && let Some(query) = url.query()
        && let Cow::Owned(redacted) = REDACT_REGEX.replace_all(query, "key=REDACTED")
    {
        url.set_query(Some(redacted.as_str()));
    }
    error
}

impl gpui_http_client::HttpClient for ReqwestClient {
    fn type_name(&self) -> &'static str {
        type_name::<Self>()
    }

    fn proxy(&self) -> Option<&Url> {
        self.proxy.as_ref()
    }

    fn user_agent(&self) -> Option<&HeaderValue> {
        self.user_agent.as_ref()
    }

    fn send(
        &self,
        req: http::Request<gpui_http_client::AsyncBody>,
    ) -> futures::future::BoxFuture<
        'static,
        anyhow::Result<gpui_http_client::Response<gpui_http_client::AsyncBody>>,
    > {
        let (parts, body) = req.into_parts();

        let mut request_builder = self.client.request(parts.method, parts.uri.to_string());
        request_builder = request_builder.headers(parts.headers);
        if let Some(redirect_policy) = parts.extensions.get::<RedirectPolicy>() {
            request_builder = request_builder.redirect_policy(match redirect_policy {
                RedirectPolicy::NoFollow => redirect::Policy::none(),
                RedirectPolicy::FollowLimit(limit) => redirect::Policy::limited(*limit as usize),
                RedirectPolicy::FollowAll => redirect::Policy::limited(100),
            });
        }
        let request = request_builder.body(match body.0 {
            gpui_http_client::Inner::Empty => zed_reqwest::Body::default(),
            gpui_http_client::Inner::Bytes(cursor) => cursor.into_inner().into(),
            gpui_http_client::Inner::AsyncReader(stream) => {
                zed_reqwest::Body::wrap_stream(StreamReader::new(stream))
            }
        });

        let handle = self.handle.clone();
        async move {
            let join_handle = handle.spawn(async { request.send().await });
            let abort_handle = join_handle.abort_handle();
            let _abort_on_drop = defer(move || abort_handle.abort());

            let mut response = join_handle.await?.map_err(redact_error)?;

            let headers = mem::take(response.headers_mut());
            let mut builder = http::Response::builder()
                .status(response.status().as_u16())
                .version(response.version());
            *builder.headers_mut().unwrap() = headers;

            let bytes = response
                .bytes_stream()
                .map_err(futures::io::Error::other)
                .into_async_read();
            let body = gpui_http_client::AsyncBody::from_reader(bytes);

            builder.body(body).map_err(|e| anyhow!(e))
        }
        .boxed()
    }
}
