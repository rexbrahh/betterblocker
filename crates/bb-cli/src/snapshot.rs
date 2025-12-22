use std::fs;
use std::path::Path;
use std::time::Instant;

use bb_compiler::{build_snapshot, optimize_rules, parse_filter_list};
use bb_core::snapshot::Snapshot;

#[derive(Debug, Clone)]
pub struct CompileStats {
    pub rules_before: usize,
    pub rules_after: usize,
    pub rules_deduped: usize,
    pub badfilter_rules: usize,
    pub badfiltered_rules: usize,
    pub total_ms: f64,
}

pub fn compile_snapshot_bytes(inputs: &[String], verbose: bool) -> Result<(Vec<u8>, CompileStats), String> {
    if inputs.is_empty() {
        return Err("No input files specified".to_string());
    }

    let start = Instant::now();
    let mut all_rules = Vec::new();

    for (list_id, path) in inputs.iter().enumerate() {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read '{}': {}", path, e))?;

        let line_count = content.lines().count();

        let mut rules = parse_filter_list(&content);

        for rule in &mut rules {
            rule.list_id = list_id as u16;
        }

        if verbose {
            println!(
                "  [{}] {} - {} lines, {} rules",
                list_id,
                Path::new(path).file_name().unwrap_or_default().to_string_lossy(),
                line_count,
                rules.len()
            );
        }

        all_rules.extend(rules);
    }

    let optimize_stats = optimize_rules(&mut all_rules);
    let snapshot_bytes = build_snapshot(&all_rules);

    Snapshot::load(&snapshot_bytes)
        .map_err(|e| format!("Generated snapshot failed validation: {}", e))?;

    let total_time = start.elapsed();

    let stats = CompileStats {
        rules_before: optimize_stats.before,
        rules_after: optimize_stats.after,
        rules_deduped: optimize_stats.deduped,
        badfilter_rules: optimize_stats.badfilter_rules,
        badfiltered_rules: optimize_stats.badfiltered_rules,
        total_ms: total_time.as_secs_f64() * 1000.0,
    };

    Ok((snapshot_bytes, stats))
}

pub fn write_snapshot(path: &Path, bytes: &[u8]) -> Result<(), String> {
    if let Some(parent) = path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create '{}': {}", parent.display(), e))?;
    }
    fs::write(path, bytes)
        .map_err(|e| format!("Failed to write '{}': {}", path.display(), e))?;
    Ok(())
}

pub fn read_snapshot(path: &Path) -> Result<Vec<u8>, String> {
    fs::read(path)
        .map_err(|e| format!("Failed to read '{}': {}", path.display(), e))
}
