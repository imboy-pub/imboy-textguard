use crate::api::InfoResponse;
use crate::model::{Action, CheckRequest, CheckResponse, ErrorCode, Scene};
use crate::runtime::{Admission, Transport};
use std::fmt::{Display, Write};
use std::sync::Mutex;
use std::time::Duration;

const SCENES: [&str; 4] = ["channel_message", "moment_post", "profile_field", "unknown"];
const TRANSPORTS: [&str; 2] = ["http", "grpc"];
const ERRORS: [ErrorCode; 21] = [
    ErrorCode::InvalidArgument,
    ErrorCode::InvalidUtf8,
    ErrorCode::TextTooLarge,
    ErrorCode::NormalizationLimit,
    ErrorCode::MatchBudgetExceeded,
    ErrorCode::Overloaded,
    ErrorCode::NotReady,
    ErrorCode::ConnectionFailed,
    ErrorCode::CheckDeadline,
    ErrorCode::Unauthenticated,
    ErrorCode::PermissionDenied,
    ErrorCode::ProtocolError,
    ErrorCode::NormalizerIncompatible,
    ErrorCode::InternalError,
    ErrorCode::UnsupportedRuleset,
    ErrorCode::CasConflict,
    ErrorCode::InvalidRuleset,
    ErrorCode::PublishDeadline,
    ErrorCode::PublishStateUnknown,
    ErrorCode::PublishBusy,
    ErrorCode::RetiredSnapshotBusy,
];

#[derive(Clone, Copy, Default)]
struct Timing {
    count: u64,
    total: Duration,
}

impl Timing {
    fn record(&mut self, elapsed: Duration) {
        self.count = self.count.saturating_add(1);
        self.total = self.total.saturating_add(elapsed);
    }
}

#[derive(Clone, Copy, Default)]
struct CheckStats {
    requests: u64,
    timing: Timing,
    scan_bytes: u64,
    decisions: [u64; 3],
    errors: [u64; ERRORS.len()],
}

#[derive(Clone, Copy, Default)]
struct Totals {
    checks: [[CheckStats; TRANSPORTS.len()]; SCENES.len()],
    publish: Timing,
    published: u64,
    publish_errors: [u64; ERRORS.len()],
}

#[derive(Default)]
pub struct Metrics {
    totals: Mutex<Totals>,
}

impl Metrics {
    pub fn record_check(
        &self,
        request: &CheckRequest,
        transport: Transport,
        elapsed: Duration,
        result: &Result<CheckResponse, ErrorCode>,
    ) {
        let scene = match request.scene {
            Scene::ChannelMessage => 0,
            Scene::MomentPost => 1,
            Scene::ProfileField => 2,
        };
        let mut totals = self
            .totals
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let stats = &mut totals.checks[scene][transport_index(transport)];
        stats.requests = stats.requests.saturating_add(1);
        stats.timing.record(elapsed);
        match result {
            Ok(response) => {
                let decision = match response.decision {
                    Action::Allow => 0,
                    Action::Review => 1,
                    Action::Block => 2,
                };
                stats.decisions[decision] = stats.decisions[decision].saturating_add(1);
                if response.scan_complete {
                    for field in &request.fields {
                        stats.scan_bytes = stats.scan_bytes.saturating_add(field.text.len() as u64);
                    }
                }
            }
            Err(error) => {
                let count = &mut stats.errors[error_index(*error)];
                *count = count.saturating_add(1);
            }
        }
    }

    pub fn record_failure(&self, transport: Transport, error: ErrorCode) {
        let mut totals = self
            .totals
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let stats = &mut totals.checks[3][transport_index(transport)];
        stats.requests = stats.requests.saturating_add(1);
        let count = &mut stats.errors[error_index(error)];
        *count = count.saturating_add(1);
    }

    pub fn record_publish(&self, elapsed: Duration, error: Option<ErrorCode>) {
        let mut totals = self
            .totals
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        totals.publish.record(elapsed);
        let count = match error {
            Some(error) => &mut totals.publish_errors[error_index(error)],
            None => &mut totals.published,
        };
        *count = count.saturating_add(1);
    }

    pub fn render(&self, admission: &Admission, info: &InfoResponse) -> String {
        let totals = *self
            .totals
            .lock()
            .unwrap_or_else(|error| error.into_inner());
        let mut output = String::new();
        for (scene, transports) in SCENES.iter().zip(totals.checks) {
            for (transport, stats) in TRANSPORTS.iter().zip(transports) {
                render_check(&mut output, scene, transport, stats);
            }
        }
        render_publish(&mut output, totals);
        let counts = admission.counts();
        sample(&mut output, "textguard_queued", "", counts.queued);
        sample(&mut output, "textguard_running", "", counts.running);
        sample(
            &mut output,
            "textguard_cancelled_still_running",
            "",
            counts.cancelled_still_running,
        );
        sample(&mut output, "textguard_rejected_total", "", counts.rejected);
        render_versions(&mut output, info);
        #[cfg(target_os = "linux")]
        render_process(
            &mut output,
            std::fs::read_to_string("/proc/self/status").ok().as_deref(),
            std::fs::read_to_string("/proc/self/stat").ok().as_deref(),
            i128::from(unsafe { libc::sysconf(libc::_SC_CLK_TCK) }),
        );
        output
    }
}

fn transport_index(transport: Transport) -> usize {
    match transport {
        Transport::Http => 0,
        Transport::Grpc => 1,
    }
}

fn error_index(error: ErrorCode) -> usize {
    match error {
        ErrorCode::InvalidArgument => 0,
        ErrorCode::InvalidUtf8 => 1,
        ErrorCode::TextTooLarge => 2,
        ErrorCode::NormalizationLimit => 3,
        ErrorCode::MatchBudgetExceeded => 4,
        ErrorCode::Overloaded => 5,
        ErrorCode::NotReady => 6,
        ErrorCode::ConnectionFailed => 7,
        ErrorCode::CheckDeadline => 8,
        ErrorCode::Unauthenticated => 9,
        ErrorCode::PermissionDenied => 10,
        ErrorCode::ProtocolError => 11,
        ErrorCode::NormalizerIncompatible => 12,
        ErrorCode::InternalError => 13,
        ErrorCode::UnsupportedRuleset => 14,
        ErrorCode::CasConflict => 15,
        ErrorCode::InvalidRuleset => 16,
        ErrorCode::PublishDeadline => 17,
        ErrorCode::PublishStateUnknown => 18,
        ErrorCode::PublishBusy => 19,
        ErrorCode::RetiredSnapshotBusy => 20,
    }
}

fn canonical_generation(value: &str) -> bool {
    value == "0"
        || (!value.is_empty()
            && value.len() <= 20
            && !value.starts_with('0')
            && value.bytes().all(|byte| byte.is_ascii_digit()))
}

fn render_versions(output: &mut String, info: &InfoResponse) {
    if canonical_generation(&info.generation) {
        if let Ok(generation) = info.generation.parse::<u64>() {
            sample(output, "textguard_generation", "", generation);
        }
    }
    if info.ruleset_revision.len() == 64
        && info
            .ruleset_revision
            .bytes()
            .all(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(&byte))
    {
        sample(
            output,
            "textguard_active_ruleset_info",
            &format!("{{revision=\"{}\"}}", info.ruleset_revision),
            1,
        );
    }
}

fn render_publish(output: &mut String, totals: Totals) {
    sample(output, "textguard_publish_total", "", totals.publish.count);
    sample(
        output,
        "textguard_publish_success_total",
        "",
        totals.published,
    );
    timing(
        output,
        "textguard_publish_duration_seconds",
        "",
        totals.publish,
    );
    for (error, count) in ERRORS.iter().zip(totals.publish_errors) {
        if count != 0 {
            let code = serde_json::to_string(error).expect("fixed error enum serializes");
            sample(
                output,
                "textguard_publish_errors_total",
                &format!("{{error_code={code}}}"),
                count,
            );
        }
    }
}

fn render_check(output: &mut String, scene: &str, transport: &str, stats: CheckStats) {
    let labels = format!("{{scene=\"{scene}\",transport=\"{transport}\"}}");
    sample(output, "textguard_requests_total", &labels, stats.requests);
    timing(
        output,
        "textguard_request_duration_seconds",
        &labels,
        stats.timing,
    );
    sample(
        output,
        "textguard_scan_bytes_total",
        &labels,
        stats.scan_bytes,
    );
    for (name, count) in ["allow", "review", "block"].iter().zip(stats.decisions) {
        sample(
            output,
            &format!("textguard_decision_{name}_total"),
            &labels,
            count,
        );
    }
    for (error, count) in ERRORS.iter().zip(stats.errors) {
        if count == 0 {
            continue;
        }
        let code = serde_json::to_string(error).expect("fixed error enum serializes");
        let labels = format!("{{scene=\"{scene}\",transport=\"{transport}\",error_code={code}}}");
        sample(output, "textguard_errors_total", &labels, count);
        if matches!(
            error,
            ErrorCode::NormalizationLimit | ErrorCode::MatchBudgetExceeded
        ) {
            sample(output, "textguard_budget_exhausted_total", &labels, count);
        }
    }
}

fn timing(output: &mut String, name: &str, labels: &str, value: Timing) {
    sample(output, &format!("{name}_count"), labels, value.count);
    sample(
        output,
        &format!("{name}_sum"),
        labels,
        value.total.as_secs_f64(),
    );
}

fn sample(output: &mut String, name: &str, labels: &str, value: impl Display) {
    writeln!(output, "{name}{labels} {value}").expect("writing a string cannot fail");
}

#[cfg(any(target_os = "linux", test))]
fn render_process(
    output: &mut String,
    status: Option<&str>,
    stat: Option<&str>,
    ticks_per_second: i128,
) {
    if let Some(status) = status {
        if let Some(line) = status.lines().find_map(|line| line.strip_prefix("VmRSS:")) {
            let mut fields = line.split_whitespace();
            if let (Some(value), Some("kB"), None) = (fields.next(), fields.next(), fields.next()) {
                if let Some(bytes) = value
                    .parse::<u64>()
                    .ok()
                    .and_then(|value| value.checked_mul(1024))
                {
                    sample(output, "textguard_process_resident_memory_bytes", "", bytes);
                }
            }
        }
    }
    if let Some(seconds) = stat.and_then(|stat| process_cpu_seconds(stat, ticks_per_second)) {
        sample(output, "textguard_process_cpu_seconds_total", "", seconds);
    }
}

#[cfg(any(target_os = "linux", test))]
fn process_cpu_seconds(stat: &str, ticks_per_second: i128) -> Option<f64> {
    if ticks_per_second <= 0 {
        return None;
    }
    let (pid, command_fields) = stat.split_once(" (")?;
    if !pid.bytes().all(|byte| byte.is_ascii_digit()) || pid.parse::<u32>().ok()? == 0 {
        return None;
    }
    let (_, fields) = command_fields.rsplit_once(") ")?;
    let mut fields = fields.split_whitespace();
    if !matches!(
        fields.next()?,
        "R" | "S" | "D" | "Z" | "T" | "t" | "W" | "X" | "x" | "K" | "P" | "I"
    ) {
        return None;
    }
    let utime = fields.nth(10)?;
    let stime = fields.next()?;
    if ![utime, stime]
        .iter()
        .all(|value| value.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    let ticks = utime
        .parse::<u64>()
        .ok()?
        .checked_add(stime.parse::<u64>().ok()?)?;
    Some(ticks as f64 / ticks_per_second as f64)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn stat(comm: &str, utime: &str, stime: &str) -> String {
        format!("123 ({comm}) R 1 2 3 0 -1 0 100 200 300 400 {utime} {stime} 900 800 20 0 2 0\n")
    }

    #[test]
    fn linux_process_units_are_converted() {
        let mut output = String::new();
        render_process(
            &mut output,
            Some("Name:\tguard\nVmRSS:\t2048 kB\n"),
            Some(&stat("guard", "125", "125")),
            100,
        );
        assert_eq!(output, "textguard_process_resident_memory_bytes 2097152\ntextguard_process_cpu_seconds_total 2.5\n");
    }

    #[test]
    fn unavailable_or_malformed_process_values_are_omitted() {
        for status in [
            None,
            Some(""),
            Some("VmRSS: secret kB"),
            Some("VmRSS: 1 MB"),
            Some("VmRSS: 18446744073709551615 kB"),
            Some("VmRSS: 1 kB extra"),
        ] {
            for stat in [
                None,
                Some(""),
                Some("secret 1 2"),
                Some("1 invalid 2"),
                Some("1 2"),
                Some("1 2 3 4"),
            ] {
                let mut output = String::new();
                render_process(&mut output, status, stat, 100);
                assert!(output.is_empty());
            }
        }
    }

    #[test]
    fn missing_process_files_are_independent() {
        let mut rss = String::new();
        render_process(&mut rss, Some("VmRSS: 3 kB"), None, -1);
        assert_eq!(rss, "textguard_process_resident_memory_bytes 3072\n");
        let mut cpu = String::new();
        render_process(&mut cpu, None, Some(&stat("guard", "75", "25")), 100);
        assert_eq!(cpu, "textguard_process_cpu_seconds_total 1\n");
    }

    #[test]
    fn stat_comm_spaces_and_right_parentheses_do_not_shift_cpu_fields() {
        for comm in [
            "guard",
            "guard worker",
            "guard)worker",
            "guard ) R 0 1",
            ") ( )",
        ] {
            assert_eq!(
                process_cpu_seconds(&stat(comm, "125", "125"), 100),
                Some(2.5)
            );
            assert_eq!(process_cpu_seconds(&stat(comm, "0", "0"), 250), Some(0.0));
            assert_eq!(
                process_cpu_seconds(&stat(comm, "500", "250"), 250),
                Some(3.0)
            );
        }
    }

    #[test]
    fn invalid_cpu_fields_or_clock_tick_rate_omit_cpu_without_hiding_rss() {
        let valid = stat("guard worker)", "125", "125");
        let invalid = [
            String::new(),
            "123 guard R 1 2".into(),
            "123 (guard R 1 2".into(),
            "123 (guard) R 1 2 3".into(),
            valid.replacen("123 ", "0 ", 1),
            valid.replacen("123 ", "-1 ", 1),
            valid.replacen("123 ", "secret ", 1),
            valid.replacen(") R ", ") RUN ", 1),
            valid.replacen(") R ", ") ? ", 1),
            stat("guard", "secret", "2"),
            stat("guard", "-1", "2"),
            stat("guard", "1", "+2"),
            stat("guard", "1", "-2"),
            stat("guard", "18446744073709551616", "0"),
            stat("guard", "18446744073709551615", "1"),
        ];
        for invalid in invalid {
            assert_eq!(process_cpu_seconds(&invalid, 100), None);
            let mut output = String::new();
            render_process(&mut output, Some("VmRSS: 3 kB"), Some(&invalid), 100);
            assert_eq!(output, "textguard_process_resident_memory_bytes 3072\n");
        }
        for ticks in [-1, 0] {
            assert_eq!(process_cpu_seconds(&valid, ticks), None);
            let mut output = String::new();
            render_process(&mut output, Some("VmRSS: 3 kB"), Some(&valid), ticks);
            assert_eq!(output, "textguard_process_resident_memory_bytes 3072\n");
        }
    }
}
