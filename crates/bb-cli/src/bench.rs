use std::cmp::Ordering;
use std::path::Path;
use std::time::Instant;

use bb_core::matcher::Matcher;
use bb_core::psl::get_etld1;
use bb_core::snapshot::Snapshot;
use bb_core::types::{MatchDecision, RequestContext, RequestType, SchemeMask};
use bb_core::url::{extract_host, extract_scheme};
use clap::ValueEnum;

use crate::snapshot;

#[derive(Debug, Clone, Copy, ValueEnum, PartialEq, Eq)]
pub enum BenchMode {
    ShouldBlock,
    MatchRequest,
    Both,
}

pub struct SimpleBenchOptions {
    pub input_paths: Vec<String>,
    pub snapshot_path: String,
    pub compile: bool,
}

pub struct RealisticBenchOptions {
    pub input_paths: Vec<String>,
    pub snapshot_path: String,
    pub compile: bool,
    pub mode: BenchMode,
    pub iterations: usize,
    pub warmup_ops: usize,
    pub sample_batch_ops: usize,
    pub trace_path: Option<String>,
    pub trace_limit: usize,
    pub synthetic_pages: usize,
    pub synthetic_reqs_per_page: usize,
    pub seed: u32,
}

struct SimpleRequest {
    url: String,
    request_type: String,
    initiator: Option<String>,
}

#[derive(Clone)]
struct BenchRequest {
    url: String,
    request_type: String,
    initiator: Option<String>,
    tab_id: i32,
    frame_id: i32,
    request_id: String,
}

fn ensure_snapshot(inputs: &[String], snapshot_path: &Path, compile: bool) -> Result<Vec<u8>, String> {
    if compile {
        let (bytes, stats) = snapshot::compile_snapshot_bytes(inputs, true)?;
        snapshot::write_snapshot(snapshot_path, &bytes)?;
        println!(
            "Compiled {} list(s): {} -> {} rules (dedupe {}, badfilter {} incl {})",
            inputs.len(),
            stats.rules_before,
            stats.rules_after,
            stats.rules_deduped,
            stats.badfiltered_rules + stats.badfilter_rules,
            stats.badfilter_rules
        );
        println!(
            "Snapshot size: {} bytes, total time {:.1}ms",
            bytes.len(),
            stats.total_ms
        );
    }

    snapshot::read_snapshot(snapshot_path)
}

fn should_block(matcher: &Matcher, req: &BenchRequest) -> bool {
    match_request(matcher, req).decision == MatchDecision::Block
}

fn match_request(matcher: &Matcher, req: &BenchRequest) -> bb_core::types::MatchResult {
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
        tab_id: req.tab_id,
        frame_id: req.frame_id,
        request_id: &req.request_id,
    };

    matcher.match_request(&ctx)
}

pub fn run_simple(opts: SimpleBenchOptions) -> Result<(), String> {
    println!("============================================================");
    println!("BetterBlocker Benchmark (Simple)");
    println!("============================================================");

    let snapshot_path = Path::new(&opts.snapshot_path);
    let snapshot_bytes = ensure_snapshot(&opts.input_paths, snapshot_path, opts.compile)?;
    let snapshot = Snapshot::load(&snapshot_bytes)
        .map_err(|e| format!("Invalid snapshot: {}", e))?;
    let matcher = Matcher::new(&snapshot);

    let realistic_mix = generate_realistic_mix();
    let random_requests = generate_test_requests(1000, DEFAULT_SEED);

    println!("Warmup...");
    warmup_simple(&matcher, &realistic_mix);

    println!("------------------------------------------------------------");
    println!("Benchmark: Realistic Mix (10 requests, 10000 iterations)");
    println!("------------------------------------------------------------");
    let realistic = run_benchmark_simple(&matcher, &realistic_mix, 10_000);
    println!("{}", format_simple_result("Realistic Mix", &realistic));

    println!("------------------------------------------------------------");
    println!("Benchmark: Random Requests (1000 requests, 100 iterations)");
    println!("------------------------------------------------------------");
    let random = run_benchmark_simple(&matcher, &random_requests, 100);
    println!("{}", format_simple_result("Random Requests", &random));

    println!("------------------------------------------------------------");
    println!("Benchmark: Single Hot Path (1 request, 100000 iterations)");
    println!("------------------------------------------------------------");
    let hot_path = run_benchmark_simple(&matcher, &realistic_mix[..1], 100_000);
    println!("{}", format_simple_result("Hot Path", &hot_path));

    println!("============================================================");
    println!("Summary");
    println!("============================================================");
    println!("Target: <5ms per request (5000μs)");
    println!("Achieved: {:.2}μs P99", realistic.p99_us);
    println!("Status: {}", if realistic.p99_us < 5000.0 { "✓ PASS" } else { "✗ FAIL" });

    Ok(())
}

pub fn run_realistic(opts: RealisticBenchOptions) -> Result<(), String> {
    println!("========================================================================");
    println!("BetterBlocker Realistic Benchmark");
    println!("========================================================================");
    println!("Input: {}", if opts.input_paths.is_empty() { "(default)" } else { "(custom)" });
    println!("Snapshot: {}", opts.snapshot_path);
    println!("Compile: {}", if opts.compile { "yes" } else { "no" });
    println!("Mode: {:?}", opts.mode);
    println!("Iterations: {}", opts.iterations);
    println!("Warmup ops: {}", opts.warmup_ops);
    println!("Sample batch ops: {}", opts.sample_batch_ops);
    println!();

    let snapshot_path = Path::new(&opts.snapshot_path);
    let snapshot_bytes = ensure_snapshot(&opts.input_paths, snapshot_path, opts.compile)?;
    let snapshot = Snapshot::load(&snapshot_bytes)
        .map_err(|e| format!("Invalid snapshot: {}", e))?;
    let matcher = Matcher::new(&snapshot);

    let requests = if let Some(path) = &opts.trace_path {
        println!("Loading trace: {} (limit {})", path, opts.trace_limit);
        load_trace_jsonl(path, opts.trace_limit)?
    } else {
        println!(
            "Generating synthetic workload: pages={}, reqs/page={}, seed={}",
            opts.synthetic_pages,
            opts.synthetic_reqs_per_page,
            opts.seed
        );
        generate_synthetic_workload(opts.synthetic_pages, opts.synthetic_reqs_per_page, opts.seed)
    };

    println!("Dataset size: {} requests", requests.len());
    println!();

    println!("Warming up...");
    if opts.mode == BenchMode::ShouldBlock || opts.mode == BenchMode::Both {
        warmup_realistic(&matcher, &requests, opts.warmup_ops, false);
    }
    if opts.mode == BenchMode::MatchRequest || opts.mode == BenchMode::Both {
        warmup_realistic(&matcher, &requests, opts.warmup_ops, true);
    }
    println!("Warmup done.");
    println!();

    let baseline = run_bench_batched(
        "Baseline (loop only)",
        &requests,
        opts.iterations.max(1) / 4,
        opts.sample_batch_ops,
        |_| 0,
    );
    println!("{}", format_realistic_result(&baseline));
    println!();

    if opts.mode == BenchMode::ShouldBlock || opts.mode == BenchMode::Both {
        let result = run_bench_batched(
            "should_block (core matcher)",
            &requests,
            opts.iterations,
            opts.sample_batch_ops,
            |req| if should_block(&matcher, req) { 1 } else { 0 },
        );
        println!("{}", format_realistic_result(&result));
        println!();
    }

    if opts.mode == BenchMode::MatchRequest || opts.mode == BenchMode::Both {
        let result = run_bench_batched(
            "match_request (extension-facing API)",
            &requests,
            opts.iterations,
            opts.sample_batch_ops,
            |req| if match_request(&matcher, req).decision != MatchDecision::Allow { 1 } else { 0 },
        );
        println!("{}", format_realistic_result(&result));
        println!();
    }

    println!("Notes:");
    println!("- p50/p95/p99 computed from per-batch wall-time samples divided by batch size.");
    println!("- For the most realistic numbers, feed a real trace via --trace (jsonl).");

    Ok(())
}

struct SimpleBenchResult {
    iterations: usize,
    total_ms: f64,
    avg_us: f64,
    p50_us: f64,
    p95_us: f64,
    p99_us: f64,
    ops_per_sec: u64,
}

fn run_benchmark_simple(matcher: &Matcher, requests: &[SimpleRequest], iterations: usize) -> SimpleBenchResult {
    let mut latencies = Vec::new();
    let mut total_ops = 0usize;

    for _ in 0..iterations {
        for req in requests {
            let start = Instant::now();
            let bench_req = BenchRequest {
                url: req.url.clone(),
                request_type: req.request_type.clone(),
                initiator: req.initiator.clone(),
                tab_id: 1,
                frame_id: 0,
                request_id: "bench".to_string(),
            };
            let _ = should_block(matcher, &bench_req);
            let elapsed = start.elapsed().as_secs_f64() * 1_000_000.0;
            latencies.push(elapsed);
            total_ops += 1;
        }
    }

    latencies.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));
    let total_ms = latencies.iter().sum::<f64>() / 1000.0;
    let avg_us = if latencies.is_empty() { 0.0 } else { latencies.iter().sum::<f64>() / latencies.len() as f64 };

    SimpleBenchResult {
        iterations: total_ops,
        total_ms,
        avg_us,
        p50_us: percentile(&latencies, 0.50),
        p95_us: percentile(&latencies, 0.95),
        p99_us: percentile(&latencies, 0.99),
        ops_per_sec: if total_ms > 0.0 { (total_ops as f64 / (total_ms / 1000.0)) as u64 } else { 0 },
    }
}

fn format_simple_result(name: &str, result: &SimpleBenchResult) -> String {
    format!(
        "{}:\n  Iterations: {}\n  Total time: {:.2}ms\n  Avg latency: {:.2}μs\n  P50 latency: {:.2}μs\n  P95 latency: {:.2}μs\n  P99 latency: {:.2}μs\n  Throughput:  {} ops/sec",
        name,
        result.iterations,
        result.total_ms,
        result.avg_us,
        result.p50_us,
        result.p95_us,
        result.p99_us,
        result.ops_per_sec,
    )
}

struct BenchResult {
    name: String,
    op_count: usize,
    total_ms: f64,
    avg_us: f64,
    p50_us: f64,
    p95_us: f64,
    p99_us: f64,
    ops_per_sec: u64,
    blocked_pct: f64,
}

fn run_bench_batched(
    name: &str,
    requests: &[BenchRequest],
    iterations: usize,
    sample_batch_ops: usize,
    mut f: impl FnMut(&BenchRequest) -> i32,
) -> BenchResult {
    let mut samples_us = Vec::new();
    let mut blocked = 0usize;
    let total_ops = requests.len() * iterations.max(1);

    let mut batch_ops = 0usize;
    let mut batch_start = Instant::now();
    let start = Instant::now();

    let mut sink = 0i32;

    for _ in 0..iterations.max(1) {
        for req in requests {
            let v = f(req);
            sink ^= v;
            if v != 0 {
                blocked += 1;
            }
            batch_ops += 1;
            if batch_ops == sample_batch_ops {
                let dt = batch_start.elapsed();
                let us_per_op = dt.as_secs_f64() * 1_000_000.0 / sample_batch_ops as f64;
                samples_us.push(us_per_op);
                batch_ops = 0;
                batch_start = Instant::now();
            }
        }
    }

    if sink == i32::MAX {
        println!("sink {}", sink);
    }

    let total_ms = start.elapsed().as_secs_f64() * 1000.0;
    samples_us.sort_by(|a, b| a.partial_cmp(b).unwrap_or(Ordering::Equal));

    let avg_us = if total_ops == 0 { 0.0 } else { total_ms * 1000.0 / total_ops as f64 };

    BenchResult {
        name: name.to_string(),
        op_count: total_ops,
        total_ms,
        avg_us,
        p50_us: percentile(&samples_us, 0.50),
        p95_us: percentile(&samples_us, 0.95),
        p99_us: percentile(&samples_us, 0.99),
        ops_per_sec: if total_ms > 0.0 { (total_ops as f64 / (total_ms / 1000.0)) as u64 } else { 0 },
        blocked_pct: if total_ops > 0 { (blocked as f64 / total_ops as f64) * 100.0 } else { 0.0 },
    }
}

fn format_realistic_result(result: &BenchResult) -> String {
    format!(
        "{}:\n  Ops: {}\n  Total: {:.2} ms\n  Avg: {:.2} us\n  P50: {:.2} us\n  P95: {:.2} us\n  P99: {:.2} us\n  Throughput: {} ops/sec\n  Blocked: {:.1}%",
        result.name,
        result.op_count,
        result.total_ms,
        result.avg_us,
        result.p50_us,
        result.p95_us,
        result.p99_us,
        result.ops_per_sec,
        result.blocked_pct,
    )
}

fn percentile(values: &[f64], p: f64) -> f64 {
    if values.is_empty() {
        return 0.0;
    }
    let idx = ((values.len() as f64) * p).ceil() as usize;
    let idx = idx.saturating_sub(1).min(values.len() - 1);
    values[idx]
}

fn warmup_simple(matcher: &Matcher, requests: &[SimpleRequest]) {
    for _ in 0..100 {
        for req in requests {
            let bench_req = BenchRequest {
                url: req.url.clone(),
                request_type: req.request_type.clone(),
                initiator: req.initiator.clone(),
                tab_id: 1,
                frame_id: 0,
                request_id: "warmup".to_string(),
            };
            let _ = should_block(matcher, &bench_req);
        }
    }
}

fn warmup_realistic(matcher: &Matcher, requests: &[BenchRequest], warmup_ops: usize, use_match_request: bool) {
    let loops = if requests.is_empty() { 0 } else { warmup_ops / requests.len() + 1 };
    for _ in 0..loops {
        for req in requests {
            if use_match_request {
                let _ = match_request(matcher, req);
            } else {
                let _ = should_block(matcher, req);
            }
        }
    }
}

fn load_trace_jsonl(path: &str, limit: usize) -> Result<Vec<BenchRequest>, String> {
    let text = std::fs::read_to_string(path)
        .map_err(|e| format!("Failed to read trace '{}': {}", path, e))?;
    let mut out = Vec::new();
    for (idx, line) in text.lines().enumerate() {
        if out.len() >= limit {
            break;
        }
        let trimmed = line.trim();
        if trimmed.is_empty() {
            continue;
        }
        let value: serde_json::Value = match serde_json::from_str(trimmed) {
            Ok(val) => val,
            Err(_) => continue,
        };
        let url = value.get("url").and_then(|v| v.as_str()).unwrap_or("");
        if url.is_empty() {
            continue;
        }
        let request_type = value.get("type").and_then(|v| v.as_str()).unwrap_or("other");
        let initiator = value.get("initiator").and_then(|v| v.as_str()).map(|s| s.to_string());
        let tab_id = value.get("tabId").and_then(|v| v.as_i64()).unwrap_or(1) as i32;
        let frame_id = value.get("frameId").and_then(|v| v.as_i64()).unwrap_or(0) as i32;
        let request_id = value
            .get("requestId")
            .and_then(|v| v.as_str())
            .unwrap_or_else(|| "0");

        out.push(BenchRequest {
            url: url.to_string(),
            request_type: request_type.to_string(),
            initiator,
            tab_id,
            frame_id,
            request_id: request_id.to_string(),
        });

        if idx % 10000 == 0 {
            // keep loop predictable
        }
    }

    if out.is_empty() {
        return Err(format!("Trace loaded 0 requests from {}", path));
    }
    Ok(out)
}

const DEFAULT_SEED: u32 = 0xc0ffee;

fn create_rng(seed: u32) -> impl FnMut() -> f64 {
    let mut state = seed;
    move || {
        state = state.wrapping_mul(1664525).wrapping_add(1013904223);
        (state as f64) / (u32::MAX as f64)
    }
}

fn pick<T: Clone>(items: &[T], rand: &mut impl FnMut() -> f64) -> T {
    let idx = (rand() * items.len() as f64).floor() as usize;
    items[idx.min(items.len() - 1)].clone()
}

fn generate_test_requests(count: usize, seed: u32) -> Vec<SimpleRequest> {
    const AD_DOMAINS: &[&str] = &[
        "ads.example.com",
        "tracking.example.com",
        "analytics.test.com",
        "doubleclick.net",
        "googlesyndication.com",
        "googleadservices.com",
        "google-analytics.com",
        "adservice.google.com",
        "pagead2.googlesyndication.com",
        "cdn.ads.com",
        "metrics.example.com",
    ];
    const CLEAN_DOMAINS: &[&str] = &[
        "example.com",
        "google.com",
        "github.com",
        "stackoverflow.com",
        "reddit.com",
        "twitter.com",
        "facebook.com",
        "amazon.com",
        "wikipedia.org",
        "mozilla.org",
    ];
    const PATHS: &[&str] = &[
        "/",
        "/index.html",
        "/assets/main.js",
        "/api/v1/data",
        "/images/logo.png",
        "/styles/app.css",
        "/ads/banner.gif",
        "/tracking/pixel.gif",
        "/analytics.js",
        "/beacon.js",
    ];
    const REQUEST_TYPES: &[&str] = &[
        "main_frame",
        "sub_frame",
        "script",
        "stylesheet",
        "image",
        "xmlhttprequest",
        "font",
        "ping",
    ];

    let mut rng = create_rng(seed);
    let mut requests = Vec::with_capacity(count);

    for _ in 0..count {
        let is_ad_request = rng() < 0.3;
        let domain = if is_ad_request {
            pick(AD_DOMAINS, &mut rng)
        } else {
            pick(CLEAN_DOMAINS, &mut rng)
        };
        let path = pick(PATHS, &mut rng);
        let request_type = pick(REQUEST_TYPES, &mut rng);

        let initiator_domain = pick(CLEAN_DOMAINS, &mut rng);
        let is_third_party = rng() < 0.6;
        let initiator = if is_third_party {
            format!("https://{}/", initiator_domain)
        } else {
            format!("https://{}/", domain)
        };

        requests.push(SimpleRequest {
            url: format!("https://{}{}", domain, path),
            request_type: request_type.to_string(),
            initiator: if request_type == "main_frame" { None } else { Some(initiator) },
        });
    }

    requests
}

fn generate_realistic_mix() -> Vec<SimpleRequest> {
    vec![
        SimpleRequest { url: "https://example.com/".to_string(), request_type: "main_frame".to_string(), initiator: None },
        SimpleRequest { url: "https://example.com/app.js".to_string(), request_type: "script".to_string(), initiator: Some("https://example.com/".to_string()) },
        SimpleRequest { url: "https://example.com/style.css".to_string(), request_type: "stylesheet".to_string(), initiator: Some("https://example.com/".to_string()) },
        SimpleRequest { url: "https://ads.example.com/banner.js".to_string(), request_type: "script".to_string(), initiator: Some("https://example.com/".to_string()) },
        SimpleRequest { url: "https://doubleclick.net/ads/show".to_string(), request_type: "xmlhttprequest".to_string(), initiator: Some("https://example.com/".to_string()) },
        SimpleRequest { url: "https://google-analytics.com/collect".to_string(), request_type: "ping".to_string(), initiator: Some("https://example.com/".to_string()) },
        SimpleRequest { url: "https://cdn.example.com/lib.js".to_string(), request_type: "script".to_string(), initiator: Some("https://example.com/".to_string()) },
        SimpleRequest { url: "https://fonts.googleapis.com/css".to_string(), request_type: "stylesheet".to_string(), initiator: Some("https://example.com/".to_string()) },
        SimpleRequest { url: "https://tracking.example.com/pixel.gif".to_string(), request_type: "image".to_string(), initiator: Some("https://example.com/".to_string()) },
        SimpleRequest { url: "https://pagead2.googlesyndication.com/pagead/js/adsbygoogle.js".to_string(), request_type: "script".to_string(), initiator: Some("https://example.com/".to_string()) },
    ]
}

fn generate_synthetic_workload(pages: usize, reqs_per_page: usize, seed: u32) -> Vec<BenchRequest> {
    let mut rng = create_rng(seed);

    const TOP_SITES: &[&str] = &[
        "google.com",
        "youtube.com",
        "github.com",
        "reddit.com",
        "amazon.com",
        "wikipedia.org",
        "mozilla.org",
        "nytimes.com",
        "cnn.com",
        "twitter.com",
        "x.com",
        "facebook.com",
        "instagram.com",
        "linkedin.com",
        "stackoverflow.com",
        "cloudflare.com",
    ];

    const CDN_DOMAINS: &[&str] = &[
        "cdnjs.cloudflare.com",
        "cdn.jsdelivr.net",
        "unpkg.com",
        "fonts.googleapis.com",
        "fonts.gstatic.com",
        "ajax.googleapis.com",
        "static.cloudflareinsights.com",
        "www.googletagmanager.com",
    ];

    const TRACKER_DOMAINS: &[&str] = &[
        "doubleclick.net",
        "googlesyndication.com",
        "googleadservices.com",
        "google-analytics.com",
        "www.google-analytics.com",
        "stats.g.doubleclick.net",
        "connect.facebook.net",
        "analytics.twitter.com",
        "bat.bing.com",
        "snap.licdn.com",
        "px.ads.linkedin.com",
    ];

    const FIRST_PARTY_ASSET_PATHS: &[&str] = &[
        "/assets/app.js",
        "/assets/vendor.js",
        "/assets/app.css",
        "/assets/logo.png",
        "/assets/hero.jpg",
        "/api/v1/graphql",
        "/api/v1/data",
        "/api/v2/search",
        "/static/chunk.js",
    ];

    const THIRD_PARTY_PATHS: &[&str] = &[
        "/gtm.js?id=GTM-XXXX",
        "/analytics.js",
        "/collect",
        "/g/collect",
        "/pixel.gif",
        "/beacon",
        "/tag/js/gpt.js",
        "/pagead/js/adsbygoogle.js",
        "/api/v1/pixel",
    ];

    let request_type_dist: &[(&str, u32)] = &[
        ("script", 22),
        ("image", 28),
        ("stylesheet", 10),
        ("xmlhttprequest", 16),
        ("font", 6),
        ("ping", 8),
        ("media", 6),
        ("websocket", 2),
        ("other", 2),
    ];

    let mut requests = Vec::new();
    let mut req_counter = 0usize;
    let mut tab_id = 1i32;

    for _ in 0..pages {
        let site = pick(TOP_SITES, &mut rng);
        let site_origin = format!("https://www.{}/", site);

        requests.push(BenchRequest {
            url: site_origin.clone(),
            request_type: "main_frame".to_string(),
            initiator: None,
            tab_id,
            frame_id: 0,
            request_id: format!("t{}:r{}", tab_id, req_counter),
        });
        req_counter += 1;

        let has_subframe = rng() < 0.25;
        let mut subframe_id = 0i32;
        if has_subframe {
            subframe_id = ((rng() * 3.0) as i32).clamp(1, 3);
            let frame_host = if rng() < 0.6 { pick(TRACKER_DOMAINS, &mut rng) } else { pick(CDN_DOMAINS, &mut rng) };
            requests.push(BenchRequest {
                url: format!("https://{}/frame.html{}", frame_host, make_query_params(&mut rng, "tracker")),
                request_type: "sub_frame".to_string(),
                initiator: Some(site_origin.clone()),
                tab_id,
                frame_id: subframe_id,
                request_id: format!("t{}:r{}", tab_id, req_counter),
            });
            req_counter += 1;
        }

        for _ in 0..reqs_per_page {
            let req_type = weighted_pick(request_type_dist, &mut rng);
            let third_party_chance = match req_type {
                "ping" => 0.85,
                "font" => 0.75,
                "script" => 0.45,
                "xmlhttprequest" => 0.35,
                "image" => 0.55,
                _ => 0.3,
            };
            let is_third_party = rng() < third_party_chance;

            let host = if is_third_party {
                if rng() < 0.6 {
                    pick(TRACKER_DOMAINS, &mut rng).to_string()
                } else {
                    pick(CDN_DOMAINS, &mut rng).to_string()
                }
            } else if rng() < 0.3 {
                format!("static.{}", site)
            } else {
                format!("www.{}", site)
            };

            let base_path = if is_third_party {
                pick(THIRD_PARTY_PATHS, &mut rng)
            } else {
                pick(FIRST_PARTY_ASSET_PATHS, &mut rng)
            };

            let worst = rng() < 0.03;
            let path = if worst { make_long_path(&mut rng) } else { base_path.to_string() };
            let qp = if worst {
                make_query_params(&mut rng, "worst")
            } else if is_third_party {
                make_query_params(&mut rng, "tracker")
            } else if rng() < 0.6 {
                make_query_params(&mut rng, "asset")
            } else {
                String::new()
            };

            let initiator = if req_type == "main_frame" { None } else { Some(site_origin.clone()) };
            let frame_id = if has_subframe && rng() < 0.2 { subframe_id } else { 0 };

            requests.push(BenchRequest {
                url: format!("https://{}{}{}", host, path, qp),
                request_type: req_type.to_string(),
                initiator,
                tab_id,
                frame_id,
                request_id: format!("t{}:r{}", tab_id, req_counter),
            });
            req_counter += 1;
        }

        tab_id += 1;
        if tab_id > 8 {
            tab_id = 1;
        }
    }

    requests
}

fn weighted_pick<'a>(items: &'a [(&'a str, u32)], rand: &mut impl FnMut() -> f64) -> &'a str {
    let total: u32 = items.iter().map(|(_, w)| *w).sum();
    let mut x = (rand() * (total as f64)).ceil() as i64;
    for (item, weight) in items {
        x -= *weight as i64;
        if x <= 0 {
            return item;
        }
    }
    items.last().map(|(item, _)| *item).unwrap_or("other")
}

fn rand_int(rand: &mut impl FnMut() -> f64, min: usize, max: usize) -> usize {
    let span = max - min + 1;
    min + ((rand() * span as f64) as usize).min(span - 1)
}

fn rand_hex(rand: &mut impl FnMut() -> f64, len: usize) -> String {
    const CHARS: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        let idx = (rand() * CHARS.len() as f64).floor() as usize;
        out.push(CHARS[idx.min(CHARS.len() - 1)] as char);
    }
    out
}

fn rand_alnum(rand: &mut impl FnMut() -> f64, len: usize) -> String {
    const CHARS: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut out = String::with_capacity(len);
    for _ in 0..len {
        let idx = (rand() * CHARS.len() as f64).floor() as usize;
        out.push(CHARS[idx.min(CHARS.len() - 1)] as char);
    }
    out
}

fn make_query_params(rand: &mut impl FnMut() -> f64, kind: &str) -> String {
    if kind == "asset" {
        let v = rand_hex(rand, 8);
        return format!("?v={}", v);
    }

    if kind == "tracker" {
        let cid = rand_hex(rand, 16);
        let tid = format!("UA-{}-1", rand_int(rand, 100000, 999999));
        let gclid = rand_alnum(rand, 24);
        let campaign = rand_alnum(rand, 10);
        let source = pick(["google", "twitter", "newsletter"].as_slice(), rand);
        return format!("?cid={}&tid={}&gclid={}&utm_source={}&utm_campaign={}", cid, tid, gclid, source, campaign);
    }

    let mut parts = Vec::new();
    for _ in 0..60 {
        let key = rand_alnum(rand, 6);
        let val_len = rand_int(rand, 8, 32);
        let val = rand_alnum(rand, val_len);
        parts.push(format!("{}={}", key, val));
    }
    parts.push(format!("gclid={}", rand_alnum(rand, 32)));
    parts.push(format!("fbclid={}", rand_alnum(rand, 32)));
    parts.push(format!("msclkid={}", rand_alnum(rand, 32)));
    format!("?{}", parts.join("&"))
}

fn make_long_path(rand: &mut impl FnMut() -> f64) -> String {
    let segs = rand_int(rand, 8, 18);
    let mut parts = Vec::with_capacity(segs);
    for _ in 0..segs {
        let len = rand_int(rand, 6, 24);
        parts.push(rand_alnum(rand, len));
    }
    format!("/{}", parts.join("/"))
}
