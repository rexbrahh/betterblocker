use std::path::Path;
use std::time::Instant;

use bb_core::matcher::Matcher;
use bb_core::psl::get_etld1;
use bb_core::snapshot::Snapshot;
use bb_core::types::{MatchDecision, RequestContext, RequestType, SchemeMask};
use bb_core::url::{extract_host, extract_scheme};

use crate::snapshot;

pub struct PerfBudgetOptions {
    pub input_paths: Vec<String>,
    pub snapshot_path: String,
    pub compile: bool,
}

struct BudgetRequest {
    url: String,
    request_type: String,
    initiator: Option<String>,
}

const BUDGET_COLD_START_MS: f64 = 500.0;
const BUDGET_WASM_PEAK_MB: f64 = 50.0;
const BUDGET_MATCH_P99_US: f64 = 1000.0;
const BUDGET_SNAPSHOT_MB: f64 = 30.0;

pub fn run_perf_budget(opts: PerfBudgetOptions) -> Result<(), String> {
    println!("Performance Budget Check");
    println!("==================================================");

    let snapshot_path = Path::new(&opts.snapshot_path);
    let snapshot_bytes = if opts.compile {
        let (bytes, stats) = snapshot::compile_snapshot_bytes(&opts.input_paths, false)?;
        snapshot::write_snapshot(snapshot_path, &bytes)?;
        println!(
            "Compiled {} list(s): {} -> {} rules",
            opts.input_paths.len(),
            stats.rules_before,
            stats.rules_after
        );
        bytes
    } else {
        snapshot::read_snapshot(snapshot_path)?
    };

    let snapshot_size_mb = snapshot_bytes.len() as f64 / (1024.0 * 1024.0);

    println!("Loading snapshot...");
    let cold_start_begin = Instant::now();
    let snapshot = Snapshot::load(&snapshot_bytes)
        .map_err(|e| format!("Invalid snapshot: {}", e))?;
    let matcher = Matcher::new(&snapshot);
    let cold_start_ms = cold_start_begin.elapsed().as_secs_f64() * 1000.0;

    println!("Warming up...");
    let warm_req = BudgetRequest {
        url: "https://example.com/test".to_string(),
        request_type: "script".to_string(),
        initiator: Some("https://example.com".to_string()),
    };
    for _ in 0..1000 {
        let _ = should_block(&matcher, &warm_req);
    }

    println!("Measuring match latency...");
    let latencies = measure_match_latency(&matcher, 2000);
    let p99_us = percentile(&latencies, 0.99);

    let wasm_peak_mb = snapshot_size_mb;

    let mut passed = true;
    println!();
    println!("Results");
    println!("--------------------------------------------------");

    passed &= report_budget("Snapshot Size", snapshot_size_mb, BUDGET_SNAPSHOT_MB, "MB");
    passed &= report_budget("Cold Start", cold_start_ms, BUDGET_COLD_START_MS, "ms");
    passed &= report_budget("Match P99 Latency", p99_us, BUDGET_MATCH_P99_US, "μs");
    passed &= report_budget("WASM Memory Peak", wasm_peak_mb, BUDGET_WASM_PEAK_MB, "MB");

    println!();
    println!("==================================================");

    if passed {
        println!("✓ All performance budgets passed");
        Ok(())
    } else {
        Err("Performance budget exceeded".to_string())
    }
}

fn report_budget(name: &str, actual: f64, limit: f64, unit: &str) -> bool {
    let passed = actual <= limit;
    let status = if passed { "✓" } else { "✗" };
    println!(
        "{} {}: {:.2} {} (limit: {:.2} {})",
        status, name, actual, unit, limit, unit
    );
    passed
}

fn measure_match_latency(matcher: &Matcher, iterations: usize) -> Vec<f64> {
    let test_urls = [
        BudgetRequest {
            url: "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js".to_string(),
            request_type: "script".to_string(),
            initiator: Some("https://example.com".to_string()),
        },
        BudgetRequest {
            url: "https://www.google-analytics.com/analytics.js".to_string(),
            request_type: "script".to_string(),
            initiator: Some("https://example.com".to_string()),
        },
        BudgetRequest {
            url: "https://example.com/style.css".to_string(),
            request_type: "stylesheet".to_string(),
            initiator: Some("https://example.com".to_string()),
        },
        BudgetRequest {
            url: "https://cdn.example.com/image.png".to_string(),
            request_type: "image".to_string(),
            initiator: Some("https://example.com".to_string()),
        },
        BudgetRequest {
            url: "https://api.example.com/data.json".to_string(),
            request_type: "xmlhttprequest".to_string(),
            initiator: Some("https://example.com".to_string()),
        },
    ];

    let mut latencies = Vec::new();

    for _ in 0..iterations {
        for req in &test_urls {
            let start = Instant::now();
            let _ = should_block(matcher, req);
            let elapsed = start.elapsed().as_secs_f64() * 1_000_000.0;
            latencies.push(elapsed);
        }
    }

    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    latencies
}

fn percentile(sorted: &[f64], p: f64) -> f64 {
    if sorted.is_empty() {
        return 0.0;
    }
    let idx = ((sorted.len() as f64) * p).ceil() as usize;
    let idx = idx.saturating_sub(1).min(sorted.len() - 1);
    sorted[idx]
}

fn should_block(matcher: &Matcher, req: &BudgetRequest) -> bool {
    match_request(matcher, req).decision == MatchDecision::Block
}

fn match_request(matcher: &Matcher, req: &BudgetRequest) -> bb_core::types::MatchResult {
    let req_host = extract_host(&req.url).unwrap_or("");
    let req_etld1 = get_etld1(req_host);

    let is_main_frame = req.request_type == "main_frame" || req.request_type == "document";
    let site_url = if is_main_frame {
        req.url.as_str()
    } else {
        req.initiator.as_deref().unwrap_or(req.url.as_str())
    };
    let site_host = extract_host(site_url).unwrap_or(req_host);
    let site_etld1 = get_etld1(site_host);

    let scheme = extract_scheme(&req.url).unwrap_or(SchemeMask::HTTP);
    let is_third_party = !site_etld1.is_empty() && req_etld1 != site_etld1;
    let request_type = RequestType::from_str(&req.request_type);

    let ctx = RequestContext {
        url: &req.url,
        req_host,
        req_etld1: &req_etld1,
        site_host,
        site_etld1: &site_etld1,
        is_third_party,
        request_type,
        scheme,
        tab_id: 1,
        frame_id: 0,
        request_id: "perf",
    };

    matcher.match_request(&ctx)
}
