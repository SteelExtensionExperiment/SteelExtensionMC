//! HTTP delivery for the generated pack only.

use http_body_util::Full;
use hyper::{
    Method, Request, Response, StatusCode, body::Bytes, header, server::conn::http1,
    service::service_fn,
};
use hyper_util::rt::{TokioIo, TokioTimer};
use std::sync::Arc;
use tokio::net::TcpListener;
use tokio_util::{sync::CancellationToken, task::TaskTracker};

pub(crate) struct ResourcePackHost {
    pub(super) listener: TcpListener,
    pub(super) path: Arc<str>,
    pub(super) bytes: Bytes,
}

impl ResourcePackHost {
    pub async fn run(self, cancel: CancellationToken, tasks: TaskTracker) {
        loop {
            let accepted = tokio::select! {
                () = cancel.cancelled() => break,
                accepted = self.listener.accept() => accepted,
            };
            let (stream, _) = match accepted {
                Ok(accepted) => accepted,
                Err(error) => {
                    log::error!("Resource pack listener stopped: {error}");
                    break;
                }
            };
            let path = Arc::clone(&self.path);
            let bytes = self.bytes.clone();
            let cancel = cancel.clone();
            tasks.spawn(async move {
                let service = service_fn(move |request| {
                    std::future::ready(pack_response(&request, &path, &bytes))
                });
                let mut builder = http1::Builder::new();
                builder.timer(TokioTimer::new()).keep_alive(false);
                tokio::select! {
                    () = cancel.cancelled() => {}
                    result = builder.serve_connection(TokioIo::new(stream), service) => {
                        if let Err(error) = result {
                            log::debug!("Resource pack HTTP request failed: {error}");
                        }
                    }
                }
            });
        }
    }
}

fn pack_response(
    request: &Request<hyper::body::Incoming>,
    path: &str,
    bytes: &Bytes,
) -> Result<Response<Full<Bytes>>, hyper::http::Error> {
    if request.uri().path() != path {
        return Response::builder()
            .status(StatusCode::NOT_FOUND)
            .body(Full::default());
    }
    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        return Response::builder()
            .status(StatusCode::METHOD_NOT_ALLOWED)
            .header(header::ALLOW, "GET, HEAD")
            .body(Full::default());
    }
    let body = if *request.method() == Method::GET {
        Full::new(bytes.clone())
    } else {
        Full::default()
    };
    Response::builder()
        .header(header::CONTENT_TYPE, "application/zip")
        .header(header::CONTENT_LENGTH, bytes.len())
        .header(header::CACHE_CONTROL, "no-cache")
        .body(body)
}
