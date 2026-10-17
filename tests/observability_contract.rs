use imboy_textguard::api::InfoResponse;
use imboy_textguard::model::{
    Action, CheckRequest, CheckResponse, ErrorCode, Hit, RuleAction, Scene, Severity, TextField,
    VariantType,
};
use imboy_textguard::observability::Metrics;
use imboy_textguard::runtime::{Admission, Transport, WORKERS};
use std::collections::BTreeSet;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

const CANARY: &str = "T17_PRIVATE_CANARY_62415";
const SECRET: &str = "T17_SECRET_03868_DO_NOT_LOG";

struct CapturedLog(Arc<Mutex<Vec<u8>>>);

impl std::io::Write for CapturedLog {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        self.0.lock().unwrap().extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

const ERRORS: [(ErrorCode, &str); 21] = [
    (ErrorCode::InvalidArgument, "INVALID_ARGUMENT"),
    (ErrorCode::InvalidUtf8, "INVALID_UTF8"),
    (ErrorCode::TextTooLarge, "TEXT_TOO_LARGE"),
    (ErrorCode::NormalizationLimit, "NORMALIZATION_LIMIT"),
    (ErrorCode::MatchBudgetExceeded, "MATCH_BUDGET_EXCEEDED"),
    (ErrorCode::Overloaded, "OVERLOADED"),
    (ErrorCode::NotReady, "NOT_READY"),
    (ErrorCode::ConnectionFailed, "CONNECTION_FAILED"),
    (ErrorCode::CheckDeadline, "CHECK_DEADLINE"),
    (ErrorCode::Unauthenticated, "UNAUTHENTICATED"),
    (ErrorCode::PermissionDenied, "PERMISSION_DENIED"),
    (ErrorCode::ProtocolError, "PROTOCOL_ERROR"),
    (ErrorCode::NormalizerIncompatible, "NORMALIZER_INCOMPATIBLE"),
    (ErrorCode::InternalError, "INTERNAL_ERROR"),
    (ErrorCode::UnsupportedRuleset, "UNSUPPORTED_RULESET"),
    (ErrorCode::CasConflict, "CAS_CONFLICT"),
    (ErrorCode::InvalidRuleset, "INVALID_RULESET"),
    (ErrorCode::PublishDeadline, "PUBLISH_DEADLINE"),
    (ErrorCode::PublishStateUnknown, "PUBLISH_STATE_UNKNOWN"),
    (ErrorCode::PublishBusy, "PUBLISH_BUSY"),
    (ErrorCode::RetiredSnapshotBusy, "RETIRED_SNAPSHOT_BUSY"),
];

fn request(scene: Scene) -> CheckRequest {
    CheckRequest {
        request_id: CANARY.into(),
        scene,
        fields: vec![
            TextField {
                field_id: SECRET.into(),
                text: "中a".into(),
            },
            TextField {
                field_id: CANARY.into(),
                text: "ß".into(),
            },
        ],
    }
}

fn response(decision: Action) -> CheckResponse {
    CheckResponse {
        request_id: CANARY.into(),
        ruleset_revision: SECRET.into(),
        generation: "7".into(),
        normalization_id: SECRET.into(),
        t2s_table_id: SECRET.into(),
        semantics_id: SECRET.into(),
        base_rules_digest: SECRET.into(),
        decision,
        scan_complete: true,
        hits_truncated: false,
        hits: if decision == Action::Allow {
            vec![]
        } else {
            vec![Hit {
                field_id: SECRET.into(),
                rule_id: CANARY.into(),
                variant_id: SECRET.into(),
                variant_type: VariantType::Canonical,
                word: CANARY.into(),
                category: SECRET.into(),
                severity: if decision == Action::Block {
                    Severity::High
                } else {
                    Severity::Low
                },
                effective_action: if decision == Action::Block {
                    RuleAction::Block
                } else {
                    RuleAction::Review
                },
                start_byte: 0,
                end_byte: 1,
                match_methods: vec![],
            }]
        },
        fields_checked: 2,
        hit_semantics: SECRET.into(),
        build_id: SECRET.into(),
    }
}

fn info(generation: &str) -> InfoResponse {
    InfoResponse {
        instance_id: SECRET.into(),
        ready: true,
        build_id: CANARY.into(),
        ruleset_revision: SECRET.into(),
        generation: generation.into(),
        normalization_id: SECRET.into(),
        t2s_table_id: CANARY.into(),
        base_rules_digest: SECRET.into(),
        canonical_count: 1,
        alias_count: 2,
        regex_count: 3,
        supported_semantics: vec![CANARY.into()],
    }
}

fn value(rendered: &str, name: &str, labels: &str) -> f64 {
    let prefix = format!("{name}{labels} ");
    let mut samples = rendered
        .lines()
        .filter_map(|line| line.strip_prefix(&prefix));
    let value = samples
        .next()
        .expect("required numeric sample")
        .parse::<f64>()
        .expect("numeric sample");
    assert!(samples.next().is_none(), "duplicate sample");
    value
}

fn labels(scene: &str, transport: &str) -> String {
    format!("{{scene=\"{scene}\",transport=\"{transport}\"}}")
}

#[test]
fn default_and_current_generation_omit_invalid_revision_labels() {
    let metrics = Metrics::default();
    let gate = Admission::default();
    for generation in ["0", "7", "18446744073709551615", "8"] {
        let rendered = metrics.render(&gate, &info(generation));
        assert_eq!(
            value(&rendered, "textguard_generation", ""),
            generation.parse::<u64>().unwrap() as f64
        );
        assert_eq!(value(&rendered, "textguard_publish_total", ""), 0.0);
        assert_eq!(value(&rendered, "textguard_running", ""), 0.0);
        assert!(!rendered.contains("revision"));
        assert!(!rendered.contains(CANARY));
        assert!(!rendered.contains(SECRET));
    }
    for generation in [
        "",
        "01",
        "+1",
        "-1",
        "1\nforged_metric 1",
        "18446744073709551616",
        SECRET,
    ] {
        assert!(!metrics
            .render(&gate, &info(generation))
            .contains("textguard_generation"));
    }
}

#[test]
fn only_the_current_valid_revision_is_exposed_without_retaining_history() {
    let metrics = Metrics::default();
    let gate = Admission::default();
    let mut previous = None;
    for index in 0..256 {
        let revision = format!("{index:064x}");
        let mut info = info("7");
        info.ruleset_revision = revision.clone();
        let rendered = metrics.render(&gate, &info);
        assert_eq!(
            value(
                &rendered,
                "textguard_active_ruleset_info",
                &format!("{{revision=\"{revision}\"}}")
            ),
            1.0
        );
        assert_eq!(
            rendered
                .lines()
                .filter(|line| line.starts_with("textguard_active_ruleset_info"))
                .count(),
            1
        );
        if let Some(previous) = previous {
            assert!(!rendered.contains(&previous));
        }
        assert!(!rendered.contains(CANARY));
        assert!(!rendered.contains(SECRET));
        previous = Some(revision);
    }
    for revision in [
        String::new(),
        "a".repeat(63),
        "a".repeat(65),
        "A".repeat(64),
        "g".repeat(64),
        format!("{}\"", "a".repeat(63)),
        format!("{SECRET}\nforged_metric 1"),
    ] {
        let mut info = info("7");
        info.ruleset_revision = revision;
        let rendered = metrics.render(&gate, &info);
        assert!(!rendered.contains("textguard_active_ruleset_info"));
        assert!(!rendered.contains("forged_metric"));
        assert!(!rendered.contains(SECRET));
    }
}

#[test]
fn successful_checks_count_utf8_bytes_decisions_and_latency_for_each_scene_transport() {
    let metrics = Metrics::default();
    let scenes = [
        (Scene::ChannelMessage, "channel_message"),
        (Scene::MomentPost, "moment_post"),
        (Scene::ProfileField, "profile_field"),
    ];
    for (scene, _) in scenes {
        for transport in [Transport::Http, Transport::Grpc] {
            for decision in [Action::Allow, Action::Review, Action::Block] {
                metrics.record_check(
                    &request(scene),
                    transport,
                    Duration::from_millis(25),
                    &Ok(response(decision)),
                );
            }
        }
    }
    let rendered = metrics.render(&Admission::default(), &info("7"));
    for (_, scene) in scenes {
        for transport in ["http", "grpc"] {
            let labels = labels(scene, transport);
            assert_eq!(value(&rendered, "textguard_requests_total", &labels), 3.0);
            assert_eq!(
                value(
                    &rendered,
                    "textguard_request_duration_seconds_count",
                    &labels
                ),
                3.0
            );
            assert!(
                (value(&rendered, "textguard_request_duration_seconds_sum", &labels) - 0.075).abs()
                    < 1e-12
            );
            assert_eq!(
                value(&rendered, "textguard_scan_bytes_total", &labels),
                18.0
            );
            for decision in ["allow", "review", "block"] {
                assert_eq!(
                    value(
                        &rendered,
                        &format!("textguard_decision_{decision}_total"),
                        &labels
                    ),
                    1.0
                );
            }
        }
    }
}

#[test]
fn errors_are_status_counts_and_budget_errors_do_not_become_decisions_or_scanned_bytes() {
    let metrics = Metrics::default();
    for (error, _) in ERRORS {
        metrics.record_check(
            &request(Scene::MomentPost),
            Transport::Grpc,
            Duration::from_millis(2),
            &Err(error),
        );
    }
    let rendered = metrics.render(&Admission::default(), &info("1"));
    let group = labels("moment_post", "grpc");
    assert_eq!(value(&rendered, "textguard_requests_total", &group), 21.0);
    assert_eq!(
        value(
            &rendered,
            "textguard_request_duration_seconds_count",
            &group
        ),
        21.0
    );
    assert!(
        (value(&rendered, "textguard_request_duration_seconds_sum", &group) - 0.042).abs() < 1e-12
    );
    assert_eq!(value(&rendered, "textguard_scan_bytes_total", &group), 0.0);
    for decision in ["allow", "review", "block"] {
        assert_eq!(
            value(
                &rendered,
                &format!("textguard_decision_{decision}_total"),
                &group
            ),
            0.0
        );
    }
    for (_, code) in ERRORS {
        let labels = format!("{{scene=\"moment_post\",transport=\"grpc\",error_code=\"{code}\"}}");
        assert_eq!(value(&rendered, "textguard_errors_total", &labels), 1.0);
    }
    let budget: Vec<_> = rendered
        .lines()
        .filter(|line| line.starts_with("textguard_budget_exhausted_total"))
        .collect();
    assert_eq!(budget.len(), 2);
    for code in ["NORMALIZATION_LIMIT", "MATCH_BUDGET_EXCEEDED"] {
        assert!(budget
            .iter()
            .any(|line| line.contains(code) && line.ends_with(" 1")));
    }
}

#[test]
fn failures_before_parsing_have_no_invented_scene_latency_or_scan_bytes() {
    let metrics = Metrics::default();
    for transport in [Transport::Http, Transport::Grpc] {
        for (error, _) in ERRORS {
            metrics.record_failure(transport, error);
        }
    }
    let rendered = metrics.render(&Admission::default(), &info("0"));
    for transport in ["http", "grpc"] {
        let group = labels("unknown", transport);
        assert_eq!(value(&rendered, "textguard_requests_total", &group), 21.0);
        assert_eq!(
            value(
                &rendered,
                "textguard_request_duration_seconds_count",
                &group
            ),
            0.0
        );
        assert_eq!(value(&rendered, "textguard_scan_bytes_total", &group), 0.0);
        for (_, code) in ERRORS {
            let labels =
                format!("{{scene=\"unknown\",transport=\"{transport}\",error_code=\"{code}\"}}");
            assert_eq!(value(&rendered, "textguard_errors_total", &labels), 1.0);
        }
    }
}

#[test]
fn incomplete_success_does_not_claim_fully_scanned_bytes() {
    let metrics = Metrics::default();
    let mut incomplete = response(Action::Review);
    incomplete.scan_complete = false;
    metrics.record_check(
        &request(Scene::ProfileField),
        Transport::Http,
        Duration::ZERO,
        &Ok(incomplete),
    );
    let rendered = metrics.render(&Admission::default(), &info("1"));
    let group = labels("profile_field", "http");
    assert_eq!(value(&rendered, "textguard_scan_bytes_total", &group), 0.0);
    assert_eq!(
        value(&rendered, "textguard_decision_review_total", &group),
        1.0
    );
}

#[test]
fn publication_success_and_all_errors_have_separate_counts_and_shared_duration() {
    let metrics = Metrics::default();
    metrics.record_publish(Duration::from_millis(50), None);
    for (error, _) in ERRORS {
        metrics.record_publish(Duration::from_millis(10), Some(error));
    }
    let rendered = metrics.render(&Admission::default(), &info("1"));
    assert_eq!(value(&rendered, "textguard_publish_total", ""), 22.0);
    assert_eq!(value(&rendered, "textguard_publish_success_total", ""), 1.0);
    assert_eq!(
        value(&rendered, "textguard_publish_duration_seconds_count", ""),
        22.0
    );
    assert!((value(&rendered, "textguard_publish_duration_seconds_sum", "") - 0.26).abs() < 1e-12);
    for (_, code) in ERRORS {
        assert_eq!(
            value(
                &rendered,
                "textguard_publish_errors_total",
                &format!("{{error_code=\"{code}\"}}")
            ),
            1.0
        );
    }
}

#[test]
fn concurrent_recording_does_not_lose_counts() {
    let metrics = Arc::new(Metrics::default());
    let threads: Vec<_> = (0..8)
        .map(|_| {
            let metrics = metrics.clone();
            std::thread::spawn(move || {
                let request = request(Scene::ChannelMessage);
                let response = Ok(response(Action::Block));
                for _ in 0..200 {
                    metrics.record_check(
                        &request,
                        Transport::Http,
                        Duration::from_millis(1),
                        &response,
                    );
                    metrics.record_publish(Duration::from_millis(1), None);
                }
            })
        })
        .collect();
    for thread in threads {
        thread.join().unwrap();
    }
    let rendered = metrics.render(&Admission::default(), &info("1"));
    let group = labels("channel_message", "http");
    assert_eq!(value(&rendered, "textguard_requests_total", &group), 1600.0);
    assert_eq!(
        value(&rendered, "textguard_scan_bytes_total", &group),
        9600.0
    );
    assert_eq!(
        value(&rendered, "textguard_decision_block_total", &group),
        1600.0
    );
    assert_eq!(
        value(&rendered, "textguard_publish_success_total", ""),
        1600.0
    );
}

#[test]
fn sensitive_values_cannot_create_labels_logs_or_new_series() {
    let logs = Arc::new(Mutex::new(Vec::new()));
    let writer_logs = logs.clone();
    let subscriber = tracing_subscriber::fmt()
        .without_time()
        .with_ansi(false)
        .with_max_level(tracing::Level::TRACE)
        .with_writer(move || CapturedLog(writer_logs.clone()))
        .finish();
    let _subscriber = tracing::subscriber::set_default(subscriber);
    tracing::info!("t17_capture_ready");
    let metrics = Metrics::default();
    let gate = Admission::default();
    let mut previous = None;
    for index in 0..256 {
        let mut request = request(Scene::ChannelMessage);
        request.request_id = format!("{CANARY}_{index}\nforged_metric 1");
        request.fields[0].text = format!("{CANARY}:{SECRET}:{index}");
        request.fields[0].field_id = format!("{SECRET}_{index}");
        let mut response = response(Action::Block);
        response.hits[0].rule_id = format!("{SECRET}_{index}");
        response.ruleset_revision = format!("{CANARY}_{index}");
        metrics.record_check(
            &request,
            Transport::Http,
            Duration::from_millis(1),
            &Ok(response),
        );
        metrics.record_check(
            &request,
            Transport::Grpc,
            Duration::from_millis(1),
            &Err(ErrorCode::MatchBudgetExceeded),
        );
        metrics.record_failure(Transport::Grpc, ErrorCode::Unauthenticated);
        metrics.record_publish(Duration::ZERO, Some(ErrorCode::InvalidRuleset));
        let mut info = info("7");
        info.ruleset_revision = format!("{SECRET}_{index}");
        let rendered = metrics.render(&gate, &info);
        assert!(!rendered.contains(CANARY), "private canary leaked");
        assert!(!rendered.contains(SECRET), "secret leaked");
        assert!(!rendered.contains("forged_metric"), "metric injection");
        let series: BTreeSet<_> = rendered
            .lines()
            .map(|line| line.rsplit_once(' ').unwrap().0.to_owned())
            .collect();
        for sample in &series {
            if let Some((_, labels)) = sample.split_once('{') {
                for pair in labels.trim_end_matches('}').split(',') {
                    let (key, value) = pair.split_once('=').unwrap();
                    let value = value.trim_matches('"');
                    match key {
                        "scene" => {
                            assert!(
                                ["channel_message", "moment_post", "profile_field", "unknown"]
                                    .contains(&value)
                            )
                        }
                        "transport" => assert!(["http", "grpc"].contains(&value)),
                        "error_code" => assert!(ERRORS.iter().any(|(_, code)| *code == value)),
                        _ => panic!("unexpected label key"),
                    }
                }
            }
        }
        if let Some(previous) = &previous {
            assert!(previous == &series, "unbounded series");
        }
        previous = Some(series);
    }
    let logs = logs.lock().unwrap();
    let logs = std::str::from_utf8(&logs).expect("utf8 logs");
    assert!(
        logs.contains("t17_capture_ready"),
        "log capture must be active"
    );
    assert!(!logs.contains(CANARY), "private canary leaked in logs");
    assert!(!logs.contains(SECRET), "secret leaked in logs");
}

#[test]
fn rejection_gauge_uses_the_live_shared_admission() {
    let gate = Admission::default();
    let metrics = Metrics::default();
    let readers: Vec<_> = (0..64).map(|_| gate.reader().unwrap()).collect();
    assert!(matches!(gate.reader(), Err(ErrorCode::Overloaded)));
    assert_eq!(
        value(
            &metrics.render(&gate, &info("1")),
            "textguard_rejected_total",
            ""
        ),
        1.0
    );
    drop(readers);
    assert!(gate.reader().is_ok());
    assert_eq!(
        value(
            &metrics.render(&gate, &info("1")),
            "textguard_rejected_total",
            ""
        ),
        1.0
    );
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn queued_gauge_reads_waiting_work_from_the_same_admission() {
    let gate = Admission::default();
    let metrics = Metrics::default();
    let mut releases = Vec::new();
    let mut running = Vec::new();
    for _ in 0..WORKERS {
        let held = gate.clone();
        let (started_tx, started_rx) = tokio::sync::oneshot::channel();
        let (finish_tx, finish_rx) = std::sync::mpsc::channel();
        releases.push(finish_tx);
        running.push(tokio::spawn(async move {
            held.execute(Instant::now() + Duration::from_millis(80), move |_| {
                let _ = started_tx.send(());
                let _ = finish_rx.recv_timeout(Duration::from_secs(2));
                Ok(())
            })
            .await
        }));
        tokio::time::timeout(Duration::from_secs(2), started_rx)
            .await
            .unwrap()
            .unwrap();
    }
    let mut queued = Box::pin(gate.execute(Instant::now() + Duration::from_millis(80), |_| Ok(())));
    std::future::poll_fn(|context| {
        assert!(std::future::Future::poll(queued.as_mut(), context).is_pending());
        std::task::Poll::Ready(())
    })
    .await;
    let rendered = metrics.render(&gate, &info("1"));
    assert_eq!(value(&rendered, "textguard_queued", ""), 1.0);
    assert_eq!(value(&rendered, "textguard_running", ""), WORKERS as f64);
    drop(queued);
    assert_eq!(
        value(&metrics.render(&gate, &info("1")), "textguard_queued", ""),
        0.0
    );
    for release in releases {
        let _ = release.send(());
    }
    for task in running {
        let _ = task.await.unwrap();
    }
}

#[tokio::test(flavor = "multi_thread", worker_threads = 2)]
async fn running_and_cancelled_gauges_follow_actual_work_lifetime() {
    let gate = Admission::default();
    let metrics = Metrics::default();
    let (started_tx, started_rx) = tokio::sync::oneshot::channel();
    let (finish_tx, finish_rx) = std::sync::mpsc::channel();
    let held = gate.clone();
    let request = tokio::spawn(async move {
        held.execute(Instant::now() + Duration::from_millis(80), move |_| {
            let _ = started_tx.send(());
            let _ = finish_rx.recv_timeout(Duration::from_secs(2));
            Ok(())
        })
        .await
    });
    tokio::time::timeout(Duration::from_secs(2), started_rx)
        .await
        .unwrap()
        .unwrap();
    let rendered = metrics.render(&gate, &info("1"));
    assert_eq!(value(&rendered, "textguard_running", ""), 1.0);
    assert_eq!(value(&rendered, "textguard_queued", ""), 0.0);
    assert_eq!(request.await.unwrap(), Err(ErrorCode::CheckDeadline));
    let rendered = metrics.render(&gate, &info("1"));
    assert_eq!(value(&rendered, "textguard_running", ""), 1.0);
    assert_eq!(
        value(&rendered, "textguard_cancelled_still_running", ""),
        1.0
    );
    finish_tx.send(()).unwrap();
    tokio::time::timeout(Duration::from_secs(2), async {
        while gate.counts().running != 0 {
            tokio::time::sleep(Duration::from_millis(1)).await;
        }
    })
    .await
    .unwrap();
    let rendered = metrics.render(&gate, &info("1"));
    assert_eq!(value(&rendered, "textguard_running", ""), 0.0);
    assert_eq!(
        value(&rendered, "textguard_cancelled_still_running", ""),
        0.0
    );
}

#[cfg(not(target_os = "linux"))]
#[test]
fn non_linux_omits_unavailable_process_metrics() {
    let rendered = Metrics::default().render(&Admission::default(), &info("0"));
    assert!(!rendered.contains("textguard_process_resident_memory_bytes"));
    assert!(!rendered.contains("textguard_process_cpu_seconds_total"));
}
