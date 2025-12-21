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
    let rules_before = all_rules.len();

    let opt_start = Instant::now();
    optimize_rules(&mut all_rules);
    let opt_time = opt_start.elapsed();
    let rules_after = all_rules.len();

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
    println!("  Rules:    {} -> {} (dedupe removed {})", rules_before, rules_after, rules_before - rules_after);
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
