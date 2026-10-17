use imboy_textguard::engine::Snapshot;
use imboy_textguard::model::{CheckRequest, CheckResponse, RuleSet};
use imboy_textguard::protocol::text_guard_client::TextGuardClient;
use imboy_textguard::protocol::{CheckTextRequest, TextField};
use serde::{Deserialize, Serialize};
use std::fs::File;
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::PathBuf;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::task::JoinSet;

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct Job {
    mode: String,
    output: PathBuf,
    cases: Option<PathBuf>,
    rules: Option<PathBuf>,
    secret_file: Option<PathBuf>,
    #[serde(default)]
    seconds: u64,
    #[serde(default)]
    qps: u32,
    #[serde(default)]
    concurrency: usize,
    #[serde(default)]
    arrival: String,
}

#[derive(Clone, Deserialize)]
struct Case {
    request: CheckRequest,
    expected_decision: String,
    expected_hits: Vec<Witness>,
    kind: String,
}

#[derive(Clone, Deserialize)]
struct Witness {
    rule_id: String,
    variant_id: String,
    method: imboy_textguard::model::MatchMethod,
}

#[derive(Serialize)]
struct Sample {
    event: &'static str,
    sequence: u64,
    transport: String,
    case_id: String,
    kind: String,
    elapsed_us: u64,
    schedule_delay_us: u64,
    completed_at_us: u64,
    ok: bool,
    error: String,
    hits: usize,
}

fn emit(output: &mut impl Write, value: &impl Serialize) {
    serde_json::to_writer(&mut *output, value).unwrap();
    writeln!(output).unwrap();
}

fn cases(job: &Job) -> Vec<Case> {
    let rows: Vec<Case> = BufReader::new(File::open(job.cases.as_ref().unwrap()).unwrap())
        .lines()
        .map(|line| serde_json::from_str(&line.unwrap()).unwrap())
        .collect();
    assert_eq!(rows.len(), 100);
    rows
}

fn rss() -> Option<u64> {
    std::fs::read_to_string("/proc/self/status")
        .ok()?
        .lines()
        .find_map(|line| {
            line.strip_prefix("VmRSS:")?
                .split_whitespace()
                .next()?
                .parse::<u64>()
                .ok()
                .map(|kb| kb * 1024)
        })
}

fn check_sample(case: &Case, response: Result<CheckResponse, String>) -> (bool, String, usize) {
    match response {
        Ok(response) => {
            let decision = serde_json::to_value(response.decision).unwrap();
            let ok = response.scan_complete
                && !response.hits_truncated
                && response.fields_checked as usize == case.request.fields.len()
                && response.request_id == case.request.request_id
                && decision.as_str() == Some(case.expected_decision.as_str())
                && response.hits.len() == case.expected_hits.len()
                && case.expected_hits.iter().all(|expected| {
                    response.hits.iter().any(|hit| {
                        hit.field_id == "body"
                            && hit.rule_id == expected.rule_id
                            && hit.variant_id == expected.variant_id
                            && hit.match_methods.contains(&expected.method)
                    })
                });
            (
                ok,
                if ok {
                    String::new()
                } else {
                    "ORACLE_MISMATCH".into()
                },
                response.hits.len(),
            )
        }
        Err(code) => (false, code, 0),
    }
}

fn core(job: &Job, output: &mut impl Write) {
    let rules: RuleSet =
        serde_json::from_slice(&std::fs::read(job.rules.as_ref().unwrap()).unwrap()).unwrap();
    let started = Instant::now();
    let snapshot = Snapshot::build(
        &rules,
        imboy_textguard::normalize::table_by_id(&rules.t2s_table_id).unwrap(),
    )
    .unwrap();
    emit(
        output,
        &serde_json::json!({"event":"build", "elapsed_us":started.elapsed().as_micros(), "rss_bytes":rss(), "canonical":rules.literals.len(), "alias":rules.literals.iter().map(|rule|rule.aliases.len()).sum::<usize>(), "regex":rules.regex_rules.len(), "exceptions":rules.exceptions.len(), "revision":snapshot.revision()}),
    );
    let rows = cases(job);
    for case in &rows {
        let (ok, error, _) = check_sample(
            case,
            snapshot
                .check(&case.request, &|| false)
                .map_err(|code| format!("{code:?}")),
        );
        assert!(
            ok,
            "independent fixture {}: {}",
            case.request.request_id, error
        );
    }
    let started = Instant::now();
    let mut sequence = 0;
    while started.elapsed() < Duration::from_secs(job.seconds) {
        let case = &rows[sequence as usize % rows.len()];
        let before = Instant::now();
        let result = snapshot
            .check(&case.request, &|| false)
            .map_err(|code| format!("{code:?}"));
        let elapsed_us = before.elapsed().as_micros() as u64;
        let (ok, error, hits) = check_sample(case, result);
        emit(
            output,
            &Sample {
                event: "request",
                sequence,
                transport: "core".into(),
                case_id: case.request.request_id.clone(),
                kind: case.kind.clone(),
                elapsed_us,
                schedule_delay_us: 0,
                completed_at_us: started.elapsed().as_micros() as u64,
                ok,
                error,
                hits,
            },
        );
        sequence += 1;
    }
    emit(
        output,
        &serde_json::json!({"event":"summary","offered":sequence,"completed":sequence,"elapsed_seconds":started.elapsed().as_secs_f64(),"rss_bytes":rss()}),
    );
}

async fn http(
    method: &str,
    port: u16,
    path: &str,
    secret: &str,
    etag: Option<&str>,
    data: &[u8],
) -> Result<(u16, Vec<u8>), String> {
    let mut socket = tokio::net::TcpStream::connect(("127.0.0.1", port))
        .await
        .map_err(|_| "CONNECT")?;
    let conditional = etag
        .map(|value| format!("If-Match: {value}\r\n"))
        .unwrap_or_default();
    let headers = format!("{method} {path} HTTP/1.1\r\nHost: localhost\r\nAuthorization: Bearer {secret}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n{conditional}\r\n", data.len());
    socket
        .write_all(headers.as_bytes())
        .await
        .map_err(|_| "WRITE")?;
    socket.write_all(data).await.map_err(|_| "WRITE")?;
    let mut bytes = Vec::new();
    socket
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .await
        .map_err(|_| "READ")?;
    if bytes.len() > 1024 * 1024 {
        return Err("RESPONSE_LIMIT".into());
    }
    let split = bytes
        .windows(4)
        .position(|window| window == b"\r\n\r\n")
        .ok_or("HTTP_FRAMING")?;
    let header = std::str::from_utf8(&bytes[..split]).map_err(|_| "HTTP_HEADER")?;
    let status = header
        .split_whitespace()
        .nth(1)
        .ok_or("HTTP_STATUS")?
        .parse()
        .map_err(|_| "HTTP_STATUS")?;
    let length = header
        .lines()
        .find_map(|line| {
            let (name, value) = line.split_once(':')?;
            name.eq_ignore_ascii_case("content-length")
                .then(|| value.trim().parse::<usize>().ok())
                .flatten()
        })
        .ok_or("HTTP_LENGTH")?;
    let body = bytes[split + 4..].to_vec();
    if body.len() != length {
        return Err("HTTP_LENGTH".into());
    }
    Ok((status, body))
}

async fn publish(job: &Job, output: &mut impl Write) {
    let secret = std::fs::read_to_string(job.secret_file.as_ref().unwrap()).unwrap();
    let data = std::fs::read(job.rules.as_ref().unwrap()).unwrap();
    let (status, body) = http(
        "GET",
        8083,
        "/internal/v1/rulesets/active",
        secret.trim(),
        None,
        &[],
    )
    .await
    .unwrap();
    assert_eq!(status, 200);
    let state: serde_json::Value = serde_json::from_slice(&body).unwrap();
    let started = Instant::now();
    let publication = tokio::time::timeout(
        Duration::from_secs(60),
        http(
            "PUT",
            8083,
            "/internal/v1/rulesets/active",
            secret.trim(),
            Some(state["state_etag"].as_str().unwrap()),
            &data,
        ),
    );
    tokio::pin!(publication);
    let mut sampling = tokio::time::interval(Duration::from_millis(100));
    let (status, body) = loop {
        tokio::select! {
            result = &mut publication => break result.unwrap().unwrap(),
            _ = sampling.tick() => metrics(output).await,
        }
    };
    metrics(output).await;
    assert_eq!(status, 200, "publication status must be successful");
    let state: serde_json::Value = serde_json::from_slice(&body).unwrap();
    emit(
        output,
        &serde_json::json!({"event":"publish","elapsed_us":started.elapsed().as_micros(),"state":state}),
    );
}

async fn run_one(
    job: Arc<Job>,
    rows: Arc<Vec<Case>>,
    sequence: u64,
    due: Instant,
    window_start: Instant,
    channel: Option<tonic::transport::Channel>,
    secret: Arc<String>,
) -> Sample {
    let case = &rows[sequence as usize % rows.len()];
    let transport = if job.mode == "mixed" {
        if (sequence + sequence / rows.len() as u64).is_multiple_of(2) {
            "http"
        } else {
            "grpc"
        }
    } else {
        job.mode.as_str()
    };
    let started = Instant::now();
    let result = tokio::time::timeout(Duration::from_millis(150), async {
        if transport == "http" {
            let data = serde_json::to_vec(&case.request).unwrap();
            let (status, body) =
                http("POST", 8081, "/v1/check", secret.trim(), None, &data).await?;
            if status != 200 {
                return Err(format!("HTTP_{status}"));
            }
            serde_json::from_slice(&body).map_err(|_| "HTTP_DECODE".into())
        } else {
            let mut client = TextGuardClient::new(channel.unwrap());
            let mut request = tonic::Request::new(CheckTextRequest {
                request_id: Some(case.request.request_id.clone()),
                scene: Some(1),
                fields: case
                    .request
                    .fields
                    .iter()
                    .map(|field| TextField {
                        field_id: Some(field.field_id.clone()),
                        text: Some(field.text.clone()),
                    })
                    .collect(),
            });
            request
                .metadata_mut()
                .insert("authorization", secret.trim().parse().unwrap());
            request.set_timeout(Duration::from_millis(150));
            let response = client
                .check_text(request)
                .await
                .map_err(|status| format!("GRPC_{:?}", status.code()))?
                .into_inner();
            let decision = match response.decision {
                Some(1) => "allow",
                Some(2) => "review",
                Some(3) => "block",
                _ => return Err("GRPC_DECISION".into()),
            };
            if response.scan_complete != Some(true)
                || response.hits_truncated != Some(false)
                || response.fields_checked != Some(case.request.fields.len() as u32)
                || response.request_id.as_deref() != Some(&case.request.request_id)
                || decision != case.expected_decision
                || response.hits.len() != case.expected_hits.len()
                || !case.expected_hits.iter().all(|expected| {
                    response.hits.iter().any(|hit| {
                        let method = match expected.method {
                            imboy_textguard::model::MatchMethod::Literal => 1,
                            imboy_textguard::model::MatchMethod::T2s => 2,
                            imboy_textguard::model::MatchMethod::Gap => 3,
                            imboy_textguard::model::MatchMethod::Alias => 4,
                            imboy_textguard::model::MatchMethod::Regex => 5,
                        };
                        hit.field_id.as_deref() == Some("body")
                            && hit.rule_id.as_deref() == Some(&expected.rule_id)
                            && hit.variant_id.as_deref() == Some(&expected.variant_id)
                            && hit.match_methods.contains(&method)
                    })
                })
            {
                return Err("ORACLE_MISMATCH".into());
            }
            return Ok((true, String::new(), response.hits.len()));
        }
        .map(|response| check_sample(case, Ok(response)))
    })
    .await
    .unwrap_or_else(|_| Err("CLIENT_DEADLINE".into()));
    let (ok, error, hits) = result.unwrap_or_else(|error| (false, error, 0));
    Sample {
        event: "request",
        sequence,
        transport: transport.into(),
        case_id: case.request.request_id.clone(),
        kind: case.kind.clone(),
        elapsed_us: due.elapsed().as_micros() as u64,
        schedule_delay_us: started.saturating_duration_since(due).as_micros() as u64,
        completed_at_us: window_start.elapsed().as_micros() as u64,
        ok,
        error,
        hits,
    }
}

async fn metrics(output: &mut impl Write) {
    let result = tokio::time::timeout(
        Duration::from_secs(1),
        http("GET", 8083, "/metrics", "", None, &[]),
    )
    .await;
    if let Ok(Ok((200, body))) = result {
        let text = String::from_utf8(body).unwrap();
        let values: Vec<_> = text
            .lines()
            .filter(|line| {
                line.starts_with("textguard_process_")
                    || line.starts_with("textguard_queued ")
                    || line.starts_with("textguard_running ")
            })
            .collect();
        emit(
            output,
            &serde_json::json!({"event":"resource","metrics":values,
                "unix_ms":std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).unwrap().as_millis(),
                "generator_cpu_stat":std::fs::read_to_string("/sys/fs/cgroup/cpu.stat").ok(),
                "generator_rss_bytes":rss()}),
        );
    } else {
        emit(
            output,
            &serde_json::json!({"event":"resource","error":"METRICS_UNAVAILABLE"}),
        );
    }
}

async fn load(job: Arc<Job>, output: &mut impl Write) {
    assert!((1..=64).contains(&job.concurrency));
    assert!(job.arrival == "open" || job.arrival == "closed");
    assert!((1..=5000).contains(&job.qps));
    let rows = Arc::new(cases(&job));
    let secret = Arc::new(std::fs::read_to_string(job.secret_file.as_ref().unwrap()).unwrap());
    let channel = if job.mode == "http" {
        None
    } else {
        Some(
            tonic::transport::Endpoint::from_static("http://127.0.0.1:8082")
                .connect()
                .await
                .unwrap(),
        )
    };
    let started = Instant::now();
    let end = started + Duration::from_secs(job.seconds);
    let mut running = JoinSet::new();
    let mut offered = 0u64;
    let mut completed = 0u64;
    let mut generator_rejected = 0u64;
    let mut resource_due = started;
    loop {
        let due = if job.arrival == "open" {
            started + Duration::from_nanos(offered * 1_000_000_000 / job.qps as u64)
        } else {
            Instant::now()
        };
        if due >= end || (job.arrival == "closed" && Instant::now() >= end) {
            break;
        }
        if job.arrival == "open" {
            tokio::time::sleep_until(due.into()).await
        }
        while let Some(result) = running.try_join_next() {
            emit(output, &result.unwrap());
            completed += 1;
        }
        if running.len() >= job.concurrency {
            if job.arrival == "closed" {
                emit(output, &running.join_next().await.unwrap().unwrap());
                completed += 1;
                continue;
            }
            generator_rejected += 1;
        } else {
            running.spawn(run_one(
                job.clone(),
                rows.clone(),
                offered,
                due,
                started,
                channel.clone(),
                secret.clone(),
            ));
        }
        offered += 1;
        if Instant::now() >= resource_due {
            metrics(output).await;
            resource_due = Instant::now() + Duration::from_secs(1);
        }
    }
    while let Some(result) = running.join_next().await {
        emit(output, &result.unwrap());
        completed += 1;
    }
    let elapsed_seconds = started.elapsed().as_secs_f64();
    metrics(output).await;
    emit(
        output,
        &serde_json::json!({"event":"summary","offered":offered,"completed":completed,"generator_rejected":generator_rejected,"elapsed_seconds":elapsed_seconds,"arrival_window_seconds":job.seconds,"generator_rss_bytes":rss()}),
    );
}

#[tokio::main(flavor = "multi_thread", worker_threads = 2)]
async fn main() {
    let arguments: Vec<_> = std::env::args_os().collect();
    assert_eq!(arguments.len(), 2, "supply one job JSON file");
    let job: Job = serde_json::from_slice(&std::fs::read(&arguments[1]).unwrap()).unwrap();
    assert!(job.seconds <= 4200);
    let mut output = BufWriter::new(File::create(&job.output).unwrap());
    match job.mode.as_str() {
        "core" => core(&job, &mut output),
        "publish" => publish(&job, &mut output).await,
        "http" | "grpc" | "mixed" => load(Arc::new(job), &mut output).await,
        _ => panic!("unsupported benchmark mode"),
    }
    output.flush().unwrap();
}
