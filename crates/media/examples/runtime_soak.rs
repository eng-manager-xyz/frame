use std::{
    fmt::Write as _,
    path::{Path, PathBuf},
    time::Instant,
};

use frame_media::{
    CancellationToken, PipelineTeardown, RUNTIME_MANIFEST_VERSION, probe_runtime,
    record_synthetic_av_webm,
};

const MINIMUM_CYCLES: u32 = 8;
const MAXIMUM_CYCLES: u32 = 200;
const MAX_END_RSS_GROWTH_BYTES: u64 = 64 * 1024 * 1024;
const MAX_END_HANDLE_GROWTH: u64 = 8;
const MAX_END_THREAD_GROWTH: u32 = 4;
const MAX_AV_DRIFT_NS: u64 = 25_000_000;
const MAX_TEARDOWN_MS: u64 = 2_000;

#[derive(Debug, Clone, Copy)]
struct ProcessSnapshot {
    rss_bytes: u64,
    handles: Option<u64>,
    threads: u32,
}

fn main() {
    if let Err(error) = run() {
        eprintln!("media runtime soak failed: {error}");
        std::process::exit(1);
    }
}

fn run() -> Result<(), String> {
    let (cycles, evidence_path) = parse_arguments()?;
    let runtime = probe_runtime().map_err(|error| error.to_string())?;
    let artifact_directory = evidence_path
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join(format!("media-soak-{}", std::process::id()));
    std::fs::create_dir_all(&artifact_directory).map_err(|error| error.to_string())?;

    // Warm the plugin registry and encoder before taking the trend baseline.
    let warmup = artifact_directory.join("warmup.webm");
    let warmup_report = record_synthetic_av_webm(&warmup, &CancellationToken::new())
        .map_err(|error| error.to_string())?;
    if !warmup_report.completed() {
        return Err("warmup did not complete with a Null teardown".into());
    }
    remove_owned_file(&warmup)?;

    let start = required_process_snapshot()?;
    let started = Instant::now();
    let mut inter_cycle_peak = start;
    let mut completed = 0_u32;
    let mut total_output_bytes = 0_u64;
    let mut max_teardown_ms = 0_u64;
    let mut max_av_drift_ns = 0_u64;

    for cycle in 0..cycles {
        let output = artifact_directory.join(format!("cycle-{cycle:04}.webm"));
        let report = record_synthetic_av_webm(&output, &CancellationToken::new())
            .map_err(|error| format!("cycle {cycle}: {error}"))?;
        if !report.completed() || report.teardown != PipelineTeardown::NullReached {
            return Err(format!(
                "cycle {cycle} did not complete and release the graph"
            ));
        }
        let timing = report
            .diagnostics
            .av_timing
            .ok_or_else(|| format!("cycle {cycle} did not report A/V timing"))?;
        max_av_drift_ns = max_av_drift_ns.max(timing.drift_ns.unsigned_abs());
        max_teardown_ms = max_teardown_ms.max(report.teardown_elapsed_ms);
        let bytes = std::fs::metadata(&output)
            .map_err(|error| error.to_string())?
            .len();
        if bytes <= 1_024 {
            return Err(format!("cycle {cycle} emitted a trivial artifact"));
        }
        total_output_bytes = total_output_bytes.saturating_add(bytes);
        completed = completed.saturating_add(1);
        remove_owned_file(&output)?;
        inter_cycle_peak = merge_peak(inter_cycle_peak, required_process_snapshot()?);
    }

    let end = required_process_snapshot()?;
    std::fs::remove_dir(&artifact_directory).map_err(|error| error.to_string())?;
    let failures = trend_failures(start, end, max_teardown_ms, max_av_drift_ns);
    let evidence = render_evidence(Evidence {
        cycles,
        completed,
        elapsed_ms: u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX),
        total_output_bytes,
        max_teardown_ms,
        max_av_drift_ns,
        runtime_version: &runtime.version,
        start,
        inter_cycle_peak,
        end,
        failures: &failures,
    });
    if let Some(parent) = evidence_path.parent() {
        std::fs::create_dir_all(parent).map_err(|error| error.to_string())?;
    }
    std::fs::write(&evidence_path, evidence).map_err(|error| error.to_string())?;
    if failures.is_empty() {
        Ok(())
    } else {
        Err(format!("resource gates failed: {}", failures.join(",")))
    }
}

fn parse_arguments() -> Result<(u32, PathBuf), String> {
    let mut arguments = std::env::args().skip(1);
    let mut cycles = None;
    let mut evidence = None;
    while let Some(argument) = arguments.next() {
        match argument.as_str() {
            "--cycles" => {
                cycles = Some(
                    arguments
                        .next()
                        .ok_or_else(|| "--cycles requires a value".to_owned())?
                        .parse::<u32>()
                        .map_err(|_| "--cycles must be an integer".to_owned())?,
                );
            }
            "--evidence" => {
                evidence = Some(PathBuf::from(
                    arguments
                        .next()
                        .ok_or_else(|| "--evidence requires a path".to_owned())?,
                ));
            }
            _ => return Err(format!("unknown argument {argument}")),
        }
    }
    let cycles = cycles.ok_or_else(|| "--cycles is required".to_owned())?;
    if !(MINIMUM_CYCLES..=MAXIMUM_CYCLES).contains(&cycles) {
        return Err(format!(
            "--cycles must be between {MINIMUM_CYCLES} and {MAXIMUM_CYCLES}"
        ));
    }
    let evidence = evidence.ok_or_else(|| "--evidence is required".to_owned())?;
    Ok((cycles, evidence))
}

fn process_snapshot() -> Option<ProcessSnapshot> {
    #[cfg(target_os = "linux")]
    {
        let status = std::fs::read_to_string("/proc/self/status").ok()?;
        let rss_kib = status_value(&status, "VmRSS:")?;
        let threads = u32::try_from(status_value(&status, "Threads:")?).ok()?;
        let handles = u64::try_from(std::fs::read_dir("/proc/self/fd").ok()?.count()).ok()?;
        Some(ProcessSnapshot {
            rss_bytes: rss_kib.saturating_mul(1_024),
            handles: Some(handles),
            threads,
        })
    }

    #[cfg(target_os = "macos")]
    {
        let output = std::process::Command::new("/bin/ps")
            .args([
                "-o",
                "rss=",
                "-o",
                "thcount=",
                "-p",
                &std::process::id().to_string(),
            ])
            .output()
            .ok()?;
        if !output.status.success() {
            return None;
        }
        let text = String::from_utf8(output.stdout).ok()?;
        let mut fields = text.split_whitespace();
        let rss_kib = fields.next()?.parse::<u64>().ok()?;
        let threads = fields.next()?.parse::<u32>().ok()?;
        Some(ProcessSnapshot {
            rss_bytes: rss_kib.saturating_mul(1_024),
            handles: None,
            threads,
        })
    }

    #[cfg(not(any(target_os = "linux", target_os = "macos")))]
    {
        None
    }
}

fn required_process_snapshot() -> Result<Option<ProcessSnapshot>, String> {
    let snapshot = process_snapshot();
    #[cfg(target_os = "linux")]
    if snapshot.is_none() {
        return Err("resource_measurement_unavailable".into());
    }
    Ok(snapshot)
}

#[cfg(target_os = "linux")]
fn status_value(status: &str, key: &str) -> Option<u64> {
    status
        .lines()
        .find(|line| line.starts_with(key))?
        .split_whitespace()
        .nth(1)?
        .parse()
        .ok()
}

fn merge_peak(
    current: Option<ProcessSnapshot>,
    sample: Option<ProcessSnapshot>,
) -> Option<ProcessSnapshot> {
    match (current, sample) {
        (Some(current), Some(sample)) => Some(ProcessSnapshot {
            rss_bytes: current.rss_bytes.max(sample.rss_bytes),
            handles: match (current.handles, sample.handles) {
                (Some(left), Some(right)) => Some(left.max(right)),
                _ => None,
            },
            threads: current.threads.max(sample.threads),
        }),
        (None, sample) => sample,
        (current, None) => current,
    }
}

fn trend_failures(
    start: Option<ProcessSnapshot>,
    end: Option<ProcessSnapshot>,
    max_teardown_ms: u64,
    max_av_drift_ns: u64,
) -> Vec<&'static str> {
    let mut failures = Vec::new();
    if let (Some(start), Some(end)) = (start, end) {
        if end.rss_bytes.saturating_sub(start.rss_bytes) > MAX_END_RSS_GROWTH_BYTES {
            failures.push("rss_growth");
        }
        if end.threads.saturating_sub(start.threads) > MAX_END_THREAD_GROWTH {
            failures.push("thread_growth");
        }
        if let (Some(start_handles), Some(end_handles)) = (start.handles, end.handles)
            && end_handles.saturating_sub(start_handles) > MAX_END_HANDLE_GROWTH
        {
            failures.push("handle_growth");
        }
    }
    if max_teardown_ms > MAX_TEARDOWN_MS {
        failures.push("teardown_time");
    }
    if max_av_drift_ns > MAX_AV_DRIFT_NS {
        failures.push("av_drift");
    }
    failures
}

fn remove_owned_file(path: &Path) -> Result<(), String> {
    std::fs::remove_file(path).map_err(|error| error.to_string())
}

struct Evidence<'a> {
    cycles: u32,
    completed: u32,
    elapsed_ms: u64,
    total_output_bytes: u64,
    max_teardown_ms: u64,
    max_av_drift_ns: u64,
    runtime_version: &'a str,
    start: Option<ProcessSnapshot>,
    inter_cycle_peak: Option<ProcessSnapshot>,
    end: Option<ProcessSnapshot>,
    failures: &'a [&'static str],
}

fn render_evidence(evidence: Evidence<'_>) -> String {
    let mut output = String::new();
    write!(
        output,
        "{{\"schema_version\":2,\"manifest_version\":{RUNTIME_MANIFEST_VERSION},\"runtime_version\":{},\"cycles\":{},\"completed\":{},\"elapsed_ms\":{},\"total_output_bytes\":{},\"max_teardown_ms\":{},\"max_av_drift_ns\":{},\"limits\":{{\"max_end_rss_growth_bytes\":{MAX_END_RSS_GROWTH_BYTES},\"max_end_handle_growth\":{MAX_END_HANDLE_GROWTH},\"max_end_thread_growth\":{MAX_END_THREAD_GROWTH},\"max_teardown_ms\":{MAX_TEARDOWN_MS},\"max_av_drift_ns\":{MAX_AV_DRIFT_NS}}},",
        json_string(evidence.runtime_version),
        evidence.cycles,
        evidence.completed,
        evidence.elapsed_ms,
        evidence.total_output_bytes,
        evidence.max_teardown_ms,
        evidence.max_av_drift_ns
    )
    .expect("write JSON");
    output.push_str("\"resources\":{");
    write_snapshot(&mut output, "start", evidence.start);
    output.push(',');
    write_snapshot(&mut output, "inter_cycle_peak", evidence.inter_cycle_peak);
    output.push(',');
    write_snapshot(&mut output, "end", evidence.end);
    output.push_str("},\"failures\":[");
    for (index, failure) in evidence.failures.iter().enumerate() {
        if index > 0 {
            output.push(',');
        }
        output.push_str(&json_string(failure));
    }
    output.push_str("]}");
    output
}

fn write_snapshot(output: &mut String, name: &str, snapshot: Option<ProcessSnapshot>) {
    write!(output, "{}:", json_string(name)).expect("write JSON");
    match snapshot {
        Some(snapshot) => {
            write!(
                output,
                "{{\"rss_bytes\":{},\"handles\":{},\"threads\":{}}}",
                snapshot.rss_bytes,
                snapshot
                    .handles
                    .map_or_else(|| "null".to_owned(), |value| value.to_string()),
                snapshot.threads
            )
            .expect("write JSON");
        }
        None => output.push_str("null"),
    }
}

fn json_string(value: &str) -> String {
    let mut output = String::with_capacity(value.len() + 2);
    output.push('"');
    for character in value.chars() {
        match character {
            '"' => output.push_str("\\\""),
            '\\' => output.push_str("\\\\"),
            '\n' => output.push_str("\\n"),
            '\r' => output.push_str("\\r"),
            '\t' => output.push_str("\\t"),
            character if character.is_control() => output.push('?'),
            character => output.push(character),
        }
    }
    output.push('"');
    output
}
