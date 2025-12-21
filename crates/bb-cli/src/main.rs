//! BetterBlocker CLI
//!
//! CLI tool for compiling filter lists and managing snapshots.

use std::fs;
use std::io::Write;
use std::path::Path;
use std::time::Instant;

use clap::{Parser, Subcommand};

use bb_compiler::{build_snapshot, optimize_rules, parse_filter_list};
use bb_core::snapshot::Snapshot;

#[derive(Parser)]
#[command(name = "bb-cli")]
#[command(about = "BetterBlocker filter list compiler and tools")]
struct Cli {
    #[command(subcommand)]
    command: Commands,
}

#[derive(Subcommand)]
enum Commands {
    /// Compile filter lists into a UBX snapshot
    Compile {
        /// Input filter list files
        #[arg(short, long, required = true)]
        input: Vec<String>,

        /// Output snapshot file
        #[arg(short, long, default_value = "snapshot.ubx")]
        output: String,

        /// Verbose output
        #[arg(short, long)]
        verbose: bool,
    },

    /// Validate a UBX snapshot
    Validate {
        /// Snapshot file to validate
        #[arg(short, long)]
        input: String,
    },

    /// Dump snapshot info
    Info {
        /// Snapshot file to inspect
        #[arg(short, long)]
        input: String,
    },

    /// Check bundled lists compile without errors (CI gate)
    Check {
        /// Input filter list files
        #[arg(short, long, required = true)]
        input: Vec<String>,

        /// Fail if parse ratio drops below threshold (0.0-1.0)
        #[arg(long, default_value = "0.95")]
        min_parse_ratio: f64,
    },
}

fn main() {
    let cli = Cli::parse();

    let result = match cli.command {
        Commands::Compile {
            input,
            output,
            verbose,
        } => cmd_compile(&input, &output, verbose),
        Commands::Validate { input } => cmd_validate(&input),
        Commands::Info { input } => cmd_info(&input),
        Commands::Check { input, min_parse_ratio } => cmd_check(&input, min_parse_ratio),
    };

    if let Err(e) = result {
        eprintln!("Error: {e}");
        std::process::exit(1);
    }
}

fn cmd_compile(inputs: &[String], output: &str, verbose: bool) -> Result<(), String> {
    if inputs.is_empty() {
        return Err("No input files specified".to_string());
    }

    let start = Instant::now();
    let mut all_rules = Vec::new();
    let mut total_lines = 0usize;

    for (list_id, path) in inputs.iter().enumerate() {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read '{}': {}", path, e))?;

        let line_count = content.lines().count();
        total_lines += line_count;

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

    let parse_time = start.elapsed();

    let opt_start = Instant::now();
    let optimize_stats = optimize_rules(&mut all_rules);
    let opt_time = opt_start.elapsed();
    let rules_before = optimize_stats.before;
    let rules_after = optimize_stats.after;

    let build_start = Instant::now();
    let snapshot_bytes = build_snapshot(&all_rules);
    let build_time = build_start.elapsed();

    Snapshot::load(&snapshot_bytes)
        .map_err(|e| format!("Generated snapshot failed validation: {}", e))?;

    let mut file = fs::File::create(output)
        .map_err(|e| format!("Failed to create '{}': {}", output, e))?;
    file.write_all(&snapshot_bytes)
        .map_err(|e| format!("Failed to write '{}': {}", output, e))?;

    let total_time = start.elapsed();

    println!("Compiled {} filter lists to '{}'", inputs.len(), output);
    println!("  Lines:    {}", total_lines);
    println!(
        "  Rules:    {} -> {} (dedupe removed {}, badfilter removed {} incl {} directives)",
        rules_before,
        rules_after,
        optimize_stats.deduped,
        optimize_stats.badfiltered_rules + optimize_stats.badfilter_rules,
        optimize_stats.badfilter_rules
    );
    println!("  Size:     {} bytes ({:.1} KB)", snapshot_bytes.len(), snapshot_bytes.len() as f64 / 1024.0);
    println!("  Time:     {:.1}ms (parse: {:.1}ms, opt: {:.1}ms, build: {:.1}ms)",
        total_time.as_secs_f64() * 1000.0,
        parse_time.as_secs_f64() * 1000.0,
        opt_time.as_secs_f64() * 1000.0,
        build_time.as_secs_f64() * 1000.0,
    );

    Ok(())
}

fn cmd_validate(input: &str) -> Result<(), String> {
    let bytes = fs::read(input)
        .map_err(|e| format!("Failed to read '{}': {}", input, e))?;

    let snapshot = Snapshot::load(&bytes)
        .map_err(|e| format!("Invalid snapshot: {}", e))?;

    println!("Snapshot '{}' is valid", input);
    println!("  Version:     {}", snapshot.version);
    println!("  Sections:    {}", snapshot.section_count());
    println!("  Size:        {} bytes", bytes.len());

    Ok(())
}

fn cmd_info(input: &str) -> Result<(), String> {
    let bytes = fs::read(input)
        .map_err(|e| format!("Failed to read '{}': {}", input, e))?;

    let snapshot = Snapshot::load(&bytes)
        .map_err(|e| format!("Invalid snapshot: {}", e))?;

    println!("Snapshot: {}", input);
    println!("  Magic:       UBX1");
    println!("  Version:     {}", snapshot.version);
    println!("  Sections:    {}", snapshot.section_count());
    println!("  Total size:  {} bytes ({:.1} KB)", bytes.len(), bytes.len() as f64 / 1024.0);
    println!();

    let block_set = snapshot.domain_block_set();
    let allow_set = snapshot.domain_allow_set();
    println!("Domain Sets:");
    println!("  Block set:   {} entries (capacity {})", block_set.entry_count(), block_set.capacity());
    println!("  Allow set:   {} entries (capacity {})", allow_set.entry_count(), allow_set.capacity());
    println!();

    let rules = snapshot.rules();
    println!("Rules:");
    println!("  Count:       {}", rules.count);

    Ok(())
}

fn cmd_check(inputs: &[String], min_parse_ratio: f64) -> Result<(), String> {
    if inputs.is_empty() {
        return Err("No input files specified".to_string());
    }

    let start = Instant::now();
    let mut all_rules = Vec::new();
    let mut total_lines = 0usize;
    let mut total_content_lines = 0usize;

    println!("Checking {} filter list(s)...\n", inputs.len());

    for (list_id, path) in inputs.iter().enumerate() {
        let content = fs::read_to_string(path)
            .map_err(|e| format!("Failed to read '{}': {}", path, e))?;

        let line_count = content.lines().count();
        let content_lines = content
            .lines()
            .filter(|l| {
                let t = l.trim();
                !t.is_empty() && !t.starts_with('!') && !t.starts_with('[')
            })
            .count();

        total_lines += line_count;
        total_content_lines += content_lines;

        let mut rules = parse_filter_list(&content);
        let rule_count = rules.len();

        for rule in &mut rules {
            rule.list_id = list_id as u16;
        }

        let parse_ratio = if content_lines > 0 {
            rule_count as f64 / content_lines as f64
        } else {
            1.0
        };

        let status = if parse_ratio >= min_parse_ratio { "OK" } else { "WARN" };

        println!(
            "[{}] {} - {} content lines -> {} rules ({:.1}%)",
            status,
            Path::new(path).file_name().unwrap_or_default().to_string_lossy(),
            content_lines,
            rule_count,
            parse_ratio * 100.0
        );

        all_rules.extend(rules);
    }

    let parse_time = start.elapsed();

    let opt_start = Instant::now();
    let optimize_stats = optimize_rules(&mut all_rules);
    let opt_time = opt_start.elapsed();

    let build_start = Instant::now();
    let snapshot_bytes = build_snapshot(&all_rules);
    let build_time = build_start.elapsed();

    Snapshot::load(&snapshot_bytes)
        .map_err(|e| format!("Generated snapshot failed validation: {}", e))?;

    let total_time = start.elapsed();
    let overall_ratio = if total_content_lines > 0 {
        optimize_stats.before as f64 / total_content_lines as f64
    } else {
        1.0
    };

    println!("\n--- Summary ---");
    println!("Total lines:     {}", total_lines);
    println!("Content lines:   {}", total_content_lines);
    println!("Rules parsed:    {}", optimize_stats.before);
    println!("Rules after opt: {}", optimize_stats.after);
    println!("Parse ratio:     {:.2}%", overall_ratio * 100.0);
    println!("Snapshot size:   {} bytes ({:.1} KB)", snapshot_bytes.len(), snapshot_bytes.len() as f64 / 1024.0);
    println!("Time:            {:.1}ms (parse: {:.1}ms, opt: {:.1}ms, build: {:.1}ms)",
        total_time.as_secs_f64() * 1000.0,
        parse_time.as_secs_f64() * 1000.0,
        opt_time.as_secs_f64() * 1000.0,
        build_time.as_secs_f64() * 1000.0,
    );

    if overall_ratio < min_parse_ratio {
        return Err(format!(
            "Parse ratio {:.2}% is below threshold {:.2}%",
            overall_ratio * 100.0,
            min_parse_ratio * 100.0
        ));
    }

    println!("\n✓ All checks passed");
    Ok(())
}
