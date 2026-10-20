use super::{config::Config, Admission, Transport};
use crate::api::auth::{AuthTokens, Scope};
use crate::api::{self, CheckCore};
use crate::model::ErrorCode;
use crate::observability::Metrics;
use crate::rules::{PublishedCore, RulesState};
use axum::body::{Body, Bytes};
use axum::extract::{Request, State};
use axum::middleware::{self, Next};
use axum::response::{IntoResponse, Response};
use axum::Router;
use http_body::{Body as HttpBody, Frame, SizeHint};
use hyper_util::rt::{TokioExecutor, TokioIo, TokioTimer};
use hyper_util::server::conn::auto::Builder;
use hyper_util::service::TowerToHyperService;
use std::future::Future;
use std::pin::Pin;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;
use tokio::io::{AsyncRead, AsyncWrite, ReadBuf};
use tokio::net::TcpListener;
use tokio::sync::{watch, OwnedSemaphorePermit, Semaphore};
use tokio::task::JoinSet;

struct DeadlineIo<Socket> {
    socket: Socket,
    writing: Option<Pin<Box<tokio::time::Sleep>>>,
    first_header_deadline: Pin<Box<tokio::time::Sleep>>,
    received_headers: Arc<AtomicBool>,
}

impl<Socket: AsyncRead + Unpin> AsyncRead for DeadlineIo<Socket> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if !self.received_headers.load(Ordering::Acquire)
            && self.first_header_deadline.as_mut().poll(context).is_ready()
        {
            return Poll::Ready(Err(std::io::ErrorKind::TimedOut.into()));
        }
        Pin::new(&mut self.socket).poll_read(context, buffer)
    }
}

impl<Socket: AsyncWrite + Unpin> AsyncWrite for DeadlineIo<Socket> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        bytes: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        if self.writing.is_none() {
            self.writing = Some(Box::pin(tokio::time::sleep(Duration::from_secs(1))));
        }
        if self
            .writing
            .as_mut()
            .unwrap()
            .as_mut()
            .poll(context)
            .is_ready()
        {
            return Poll::Ready(Err(std::io::ErrorKind::TimedOut.into()));
        }
        Pin::new(&mut self.socket).poll_write(context, bytes)
    }

    fn poll_flush(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self
            .writing
            .as_mut()
            .is_some_and(|timer| timer.as_mut().poll(context).is_ready())
        {
            return Poll::Ready(Err(std::io::ErrorKind::TimedOut.into()));
        }
        let result = Pin::new(&mut self.socket).poll_flush(context);
        if result.is_ready() {
            self.writing = None;
        }
        result
    }

    fn poll_shutdown(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.socket).poll_shutdown(context)
    }
}

struct DeadlineBody {
    body: Body,
    deadline: Option<Pin<Box<tokio::time::Sleep>>>,
    permits: Vec<OwnedSemaphorePermit>,
    timed_out: Arc<AtomicBool>,
    done: bool,
    grpc_deadline: bool,
}

impl DeadlineBody {
    fn wrap(
        body: Body,
        budget: Duration,
        permits: Vec<OwnedSemaphorePermit>,
        timed_out: Arc<AtomicBool>,
    ) -> Body {
        Body::new(Self {
            body,
            deadline: Some(Box::pin(tokio::time::sleep(budget))),
            permits,
            timed_out,
            done: false,
            grpc_deadline: false,
        })
    }

    fn holding(
        body: Body,
        permit: OwnedSemaphorePermit,
        deadline: Option<std::time::Instant>,
    ) -> Body {
        Body::new(Self {
            body,
            deadline: deadline.map(|deadline| Box::pin(tokio::time::sleep_until(deadline.into()))),
            permits: vec![permit],
            timed_out: Arc::new(AtomicBool::new(false)),
            done: false,
            grpc_deadline: true,
        })
    }
}

impl HttpBody for DeadlineBody {
    type Data = Bytes;
    type Error = axum::Error;

    fn poll_frame(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
    ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
        if self.done {
            return Poll::Ready(None);
        }
        if self
            .deadline
            .as_mut()
            .is_some_and(|deadline| deadline.as_mut().poll(context).is_ready())
        {
            self.done = true;
            self.timed_out.store(true, Ordering::Release);
            self.permits.clear();
            if self.grpc_deadline {
                let trailers = api::grpc::status_from_error(ErrorCode::CheckDeadline)
                    .into_http()
                    .into_parts()
                    .0
                    .headers;
                return Poll::Ready(Some(Ok(Frame::trailers(trailers))));
            }
            return Poll::Ready(Some(Err(axum::Error::new(std::io::Error::new(
                std::io::ErrorKind::TimedOut,
                "TRANSPORT_DEADLINE",
            )))));
        }
        let frame = Pin::new(&mut self.body).poll_frame(context);
        if matches!(frame, Poll::Ready(None) | Poll::Ready(Some(Err(_)))) {
            self.done = true;
            self.permits.clear();
        }
        frame
    }

    fn is_end_stream(&self) -> bool {
        self.done || self.body.is_end_stream()
    }
    fn size_hint(&self) -> SizeHint {
        self.body.size_hint()
    }
}

#[derive(Clone)]
struct GrpcBoundary {
    admission: Admission,
    tokens: Arc<AuthTokens>,
    metrics: Arc<Metrics>,
}

async fn grpc_boundary(
    State(state): State<GrpcBoundary>,
    mut request: Request,
    next: Next,
) -> Response {
    let health_watch = request.uri().path() == "/grpc.health.v1.Health/Watch";
    let check = request.uri().path() == "/imboy.textguard.v1.TextGuard/CheckText";
    let accounting = api::grpc::CheckAccounting::default();
    if check {
        request.extensions_mut().insert(accounting.clone());
    }
    let record_failure = |code| {
        if check && accounting.claim() {
            state.metrics.record_failure(Transport::Grpc, code);
        }
    };
    let deadline = request
        .headers()
        .get("grpc-timeout")
        .and_then(|value| value.to_str().ok())
        .and_then(api::grpc::parse_grpc_timeout)
        .map(|duration| std::time::Instant::now() + duration);
    if let Some(deadline) = deadline {
        request.extensions_mut().insert(deadline);
    }
    let mut credentials = request.headers().get_all("authorization").iter();
    let value = match (credentials.next(), credentials.next()) {
        (Some(value), None) => value.to_str().ok(),
        _ => None,
    };
    let value = value.map(|value| {
        if value.len() >= 7 && value.as_bytes()[..7].eq_ignore_ascii_case(b"bearer ") {
            &value[7..]
        } else {
            value
        }
    });
    if !matches!(
        request.uri().path(),
        "/grpc.health.v1.Health/Check" | "/grpc.health.v1.Health/Watch"
    ) {
        if let Err(code) = state.tokens.verify(Scope::Check, value) {
            record_failure(code);
            return grpc_error(code);
        }
    }
    let stream = match state.admission.connection(Transport::Grpc) {
        Ok(permit) => permit,
        Err(code) => {
            record_failure(code);
            return grpc_error(code);
        }
    };
    let reader = match state.admission.reader() {
        Ok(permit) => permit,
        Err(code) => {
            record_failure(code);
            return grpc_error(code);
        }
    };
    let timed_out = Arc::new(AtomicBool::new(false));
    let (parts, body) = request.into_parts();
    let body = DeadlineBody::wrap(
        body,
        Duration::from_secs(1),
        vec![reader],
        timed_out.clone(),
    );
    let dispatch = next.run(Request::from_parts(parts, body));
    let response = if let Some(deadline) = deadline {
        match tokio::time::timeout_at(deadline.into(), dispatch).await {
            Ok(response) => response,
            Err(_) => {
                record_failure(ErrorCode::CheckDeadline);
                grpc_error(ErrorCode::CheckDeadline)
            }
        }
    } else {
        dispatch.await
    };
    let response = if timed_out.load(Ordering::Acquire) {
        record_failure(ErrorCode::CheckDeadline);
        grpc_error(ErrorCode::CheckDeadline)
    } else {
        response
    };
    if check {
        record_failure(
            match response
                .headers()
                .get("grpc-status")
                .and_then(|value| value.to_str().ok())
            {
                Some("11") => ErrorCode::TextTooLarge,
                Some("3") => ErrorCode::InvalidArgument,
                Some("4") => ErrorCode::CheckDeadline,
                _ => ErrorCode::ProtocolError,
            },
        );
    }
    let (parts, body) = response.into_parts();
    if health_watch {
        return Response::from_parts(parts, DeadlineBody::holding(body, stream, deadline));
    }
    Response::from_parts(
        parts,
        DeadlineBody::wrap(
            body,
            Duration::from_secs(1),
            vec![stream],
            Arc::new(AtomicBool::new(false)),
        ),
    )
}

fn grpc_error(code: ErrorCode) -> Response {
    crate::api::grpc::status_from_error(code)
        .into_http()
        .into_response()
}

async fn send_deadline(request: Request, next: Next) -> Response {
    let response = next.run(request).await;
    let (parts, body) = response.into_parts();
    Response::from_parts(
        parts,
        DeadlineBody::wrap(
            body,
            Duration::from_secs(1),
            vec![],
            Arc::new(AtomicBool::new(false)),
        ),
    )
}

pub async fn serve(
    config: Config,
    shutdown: impl Future<Output = ()> + Send,
) -> Result<(), &'static str> {
    if [config.http, config.grpc, config.manage]
        .iter()
        .any(|address| !address.ip().is_loopback())
    {
        return Err("TLS_GATEWAY_REQUIRES_LOOPBACK_BACKEND");
    }
    let table = crate::normalize::table_by_id(&config.table_id).ok_or("UNSUPPORTED_RULESET")?;
    let rules =
        Arc::new(RulesState::load(&config.data_dir, table).map_err(|_| "RULE_STORE_UNAVAILABLE")?);
    // First-run bootstrap only. Never replace an existing ACTIVE ruleset or
    // recover from a corrupt manifest by silently activating the seed.
    if matches!(rules.status(), crate::rules::StoreStatus::Empty)
        && config.table_id == crate::T2S_TABLE_ID
    {
        let seed = include_bytes!("../../data/dictionaries/default_review_seed.json");
        let (_, etag) = rules.active_state().map_err(|_| "RULE_STORE_UNAVAILABLE")?;
        rules
            .publish(seed, Some(&etag))
            .map_err(|_| "SEED_BOOTSTRAP_FAILED")?;
    }
    let core: Arc<dyn CheckCore> = Arc::new(PublishedCore::new(rules.clone()));
    let admission = Admission::default();
    let metrics = Arc::new(Metrics::default());
    let (mut reporter, health) = tonic_health::server::health_reporter();
    let mut was_ready = core.ready();
    for service in ["", "imboy.textguard.v1.TextGuard"] {
        reporter
            .set_service_status(
                service,
                if core.ready() {
                    tonic_health::ServingStatus::Serving
                } else {
                    tonic_health::ServingStatus::NotServing
                },
            )
            .await;
    }
    reporter
        .set_service_status("liveness", tonic_health::ServingStatus::Serving)
        .await;
    let http = api::http::router_observed(
        core.clone(),
        config.tokens.clone(),
        admission.clone(),
        metrics.clone(),
    )
    .layer(middleware::from_fn(send_deadline));
    let grpc = Router::new()
        .route_service("/grpc.health.v1.Health/Check", health.clone())
        .route_service("/grpc.health.v1.Health/Watch", health)
        .fallback_service(
            crate::protocol::text_guard_server::TextGuardServer::new(
                api::grpc::TextGuardSvc::observed(
                    core.clone(),
                    config.tokens.clone(),
                    admission.clone(),
                    metrics.clone(),
                ),
            )
            .max_decoding_message_size(api::grpc::MAX_MESSAGE_BYTES)
            .max_encoding_message_size(api::grpc::MAX_MESSAGE_BYTES),
        )
        .layer(middleware::from_fn_with_state(
            GrpcBoundary {
                admission: admission.clone(),
                tokens: config.tokens.clone(),
                metrics: metrics.clone(),
            },
            grpc_boundary,
        ));
    let metrics_core = core.clone();
    let metrics_admission = admission.clone();
    let rendered_metrics = metrics.clone();
    let manage = api::http::manage::router_observed(rules, config.tokens, metrics)
        .route(
            "/metrics",
            axum::routing::get(move || {
                let text = rendered_metrics.render(&metrics_admission, &metrics_core.info());
                async {
                    (
                        [("content-type", "text/plain; version=0.0.4; charset=utf-8")],
                        text,
                    )
                }
            }),
        )
        .layer(middleware::from_fn(send_deadline));
    let listeners = [
        (
            TcpListener::bind(config.http)
                .await
                .map_err(|_| "HTTP_BIND_FAILED")?,
            http,
            true,
        ),
        (
            TcpListener::bind(config.grpc)
                .await
                .map_err(|_| "GRPC_BIND_FAILED")?,
            grpc,
            false,
        ),
        (
            TcpListener::bind(config.manage)
                .await
                .map_err(|_| "MANAGE_BIND_FAILED")?,
            manage,
            true,
        ),
    ];
    let (stop, stopped) = watch::channel(false);
    let mut servers = JoinSet::new();
    for (listener, router, http) in listeners {
        servers.spawn(serve_listener(
            listener,
            router,
            admission.clone(),
            stopped.clone(),
            http,
        ));
    }
    tokio::pin!(shutdown);
    let mut health_tick = tokio::time::interval(Duration::from_millis(200));
    let failed = loop {
        tokio::select! {
            _ = &mut shutdown => break false,
            _ = servers.join_next() => break true,
            _ = health_tick.tick() => {
                if core.ready() == was_ready { continue; }
                was_ready = core.ready();
                for service in ["", "imboy.textguard.v1.TextGuard"] {
                    reporter.set_service_status(service, if was_ready {
                        tonic_health::ServingStatus::Serving
                    } else { tonic_health::ServingStatus::NotServing }).await;
                }
            }
        }
    };
    for service in ["", "liveness", "imboy.textguard.v1.TextGuard"] {
        reporter
            .set_service_status(service, tonic_health::ServingStatus::NotServing)
            .await;
    }
    let _ = stop.send(true);
    while let Some(result) = servers.join_next().await {
        result.map_err(|_| "SERVER_TASK_FAILED")??;
    }
    if failed {
        Err("SERVER_STOPPED")
    } else {
        Ok(())
    }
}

async fn serve_listener(
    listener: TcpListener,
    router: Router,
    admission: Admission,
    mut stopped: watch::Receiver<bool>,
    http: bool,
) -> Result<(), &'static str> {
    let grpc_connections = Arc::new(Semaphore::new(128));
    let mut connections = JoinSet::new();
    loop {
        tokio::select! {
            _ = stopped.changed() => break,
            _ = connections.join_next(), if !connections.is_empty() => (),
            accepted = listener.accept() => {
                let (stream, _) = accepted.map_err(|_| "ACCEPT_FAILED")?;
                if stream.set_nodelay(true).is_err() { continue; }
                let permit = if http { admission.connection(Transport::Http).ok() } else { grpc_connections.clone().try_acquire_owned().ok() };
                let Some(permit) = permit else { continue; };
                let router = router.clone();
                let mut stopped = stopped.clone();
                connections.spawn(async move {
                    let _permit = permit;
                    let mut initial = [0u8; 1];
                    if !matches!(tokio::time::timeout(Duration::from_secs(1), stream.peek(&mut initial)).await, Ok(Ok(1))) {
                        return;
                    }
                    let mut builder = Builder::new(TokioExecutor::new());
                    builder.http1().timer(TokioTimer::new()).header_read_timeout(Duration::from_secs(1));
                    builder.http2().max_concurrent_streams(128).max_header_list_size(16 * 1024);
                    let received_headers = Arc::new(AtomicBool::new(false));
                    let seen = received_headers.clone();
                    let router = router.layer(middleware::from_fn(move |request: Request, next: Next| {
                        seen.store(true, Ordering::Release);
                        async move { next.run(request).await }
                    }));
                    let stream = DeadlineIo { socket: stream, writing: None,
                        first_header_deadline: Box::pin(tokio::time::sleep(Duration::from_secs(1))), received_headers };
                    let connection = builder.serve_connection(TokioIo::new(stream), TowerToHyperService::new(router));
                    tokio::pin!(connection);
                    tokio::select! {
                        _ = &mut connection => (),
                        _ = stopped.changed() => {
                            connection.as_mut().graceful_shutdown();
                            let _ = connection.await;
                        }
                    }
                });
            }
        }
    }
    tokio::time::timeout(Duration::from_secs(65), async {
        while connections.join_next().await.is_some() {}
    })
    .await
    .map_err(|_| "SHUTDOWN_DEADLINE")?;
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::io::AsyncWriteExt;

    struct DelayedBody {
        delay: Pin<Box<tokio::time::Sleep>>,
        bytes: Option<Bytes>,
    }

    impl HttpBody for DelayedBody {
        type Data = Bytes;
        type Error = std::io::Error;
        fn poll_frame(
            mut self: Pin<&mut Self>,
            context: &mut Context<'_>,
        ) -> Poll<Option<Result<Frame<Bytes>, Self::Error>>> {
            if self.delay.as_mut().poll(context).is_pending() {
                return Poll::Pending;
            }
            Poll::Ready(self.bytes.take().map(|bytes| Ok(Frame::data(bytes))))
        }
    }

    #[tokio::test]
    async fn grpc_deadline_includes_body_receive_before_dispatch() {
        use std::sync::atomic::AtomicUsize;
        use tonic::codegen::Service;
        let dispatched = Arc::new(AtomicUsize::new(0));
        let captured = dispatched.clone();
        let mut router = Router::new()
            .fallback(move |request: Request| {
                let dispatched = captured.clone();
                async move {
                    axum::body::to_bytes(request.into_body(), 1024)
                        .await
                        .unwrap();
                    dispatched.fetch_add(1, Ordering::SeqCst);
                    axum::http::StatusCode::OK
                }
            })
            .layer(middleware::from_fn_with_state(
                GrpcBoundary {
                    admission: Admission::default(),
                    tokens: Arc::new(AuthTokens::new("check", "publish")),
                    metrics: Arc::new(Metrics::default()),
                },
                grpc_boundary,
            ));
        let body = Body::new(DelayedBody {
            delay: Box::pin(tokio::time::sleep(Duration::from_millis(60))),
            bytes: Some(Bytes::from_static(b"payload")),
        });
        let request = Request::builder()
            .uri("/imboy.textguard.v1.TextGuard/CheckText")
            .header("authorization", "check")
            .header("grpc-timeout", "5m")
            .body(body)
            .unwrap();
        let started = std::time::Instant::now();
        let response = router.call(request).await.unwrap();
        assert_eq!(response.headers().get("grpc-status").unwrap(), "4");
        assert_eq!(dispatched.load(Ordering::SeqCst), 0);
        assert!(started.elapsed() < Duration::from_millis(50));
    }

    #[tokio::test]
    async fn watch_deadline_releases_stream_without_bounding_unlimited_subscriptions() {
        let permits = Arc::new(Semaphore::new(1));
        let mut body = DeadlineBody::holding(
            Body::new(DelayedBody {
                delay: Box::pin(tokio::time::sleep(Duration::from_secs(60))),
                bytes: Some(Bytes::new()),
            }),
            permits.clone().try_acquire_owned().unwrap(),
            Some(std::time::Instant::now() + Duration::from_millis(5)),
        );
        assert_eq!(permits.available_permits(), 0);
        let frame = std::future::poll_fn(|context| Pin::new(&mut body).poll_frame(context))
            .await
            .unwrap()
            .unwrap();
        assert_eq!(frame.into_trailers().unwrap()["grpc-status"], "4");
        assert_eq!(permits.available_permits(), 1);
        let mut body = DeadlineBody::holding(
            Body::new(DelayedBody {
                delay: Box::pin(tokio::time::sleep(Duration::from_secs(60))),
                bytes: Some(Bytes::new()),
            }),
            permits.clone().try_acquire_owned().unwrap(),
            None,
        );
        assert!(tokio::time::timeout(
            Duration::from_millis(1100),
            std::future::poll_fn(|context| Pin::new(&mut body).poll_frame(context))
        )
        .await
        .is_err());
        assert_eq!(permits.available_permits(), 0);
        drop(body);
        assert_eq!(permits.available_permits(), 1);
    }

    #[tokio::test]
    async fn a_blocked_socket_write_has_an_actual_io_deadline() {
        let (socket, _reader) = tokio::io::duplex(1);
        let mut socket = DeadlineIo {
            socket,
            writing: None,
            first_header_deadline: Box::pin(tokio::time::sleep(Duration::from_secs(1))),
            received_headers: Arc::new(AtomicBool::new(true)),
        };
        let error = socket.write_all(&[1, 2]).await.unwrap_err();
        assert_eq!(error.kind(), std::io::ErrorKind::TimedOut);
    }

    #[tokio::test]
    async fn expired_body_releases_the_real_reader_permit() {
        let admission = Admission::default();
        let body = DeadlineBody::wrap(
            Body::from("canary-must-never-be-logged"),
            Duration::ZERO,
            vec![admission.reader().unwrap()],
            Arc::new(AtomicBool::new(false)),
        );
        tokio::time::sleep(Duration::from_millis(1)).await;
        assert!(axum::body::to_bytes(body, 128).await.is_err());
        let permits: Vec<_> = (0..64).map(|_| admission.reader().unwrap()).collect();
        assert!(admission.reader().is_err());
        drop(permits);
    }
}
