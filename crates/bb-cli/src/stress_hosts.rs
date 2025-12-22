use std::collections::BTreeSet;
use std::fs;
use std::path::{Path, PathBuf};

pub struct StressHostsOptions {
    pub inputs: Vec<String>,
    pub output: String,
}

pub fn run_generate_hosts(opts: StressHostsOptions) -> Result<(), String> {
    let sources = if opts.inputs.is_empty() {
        vec![default_input_path()?]
    } else {
        opts.inputs.iter().map(PathBuf::from).collect()
    };

    let mut domains = BTreeSet::new();
    let mut total_lines = 0usize;

    for source in &sources {
        let content = fs::read_to_string(source)
            .map_err(|e| format!("Failed to read '{}': {}", source.display(), e))?;
        let lines = content.lines();
        total_lines += lines.clone().count();

        for line in lines {
            if let Some(domain) = extract_domain(line) {
                if domain.contains('.') && !domain.contains('*') {
                    domains.insert(domain.to_string());
                }
            }
        }
    }

    let output_path = PathBuf::from(&opts.output);
    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent)
            .map_err(|e| format!("Failed to create '{}': {}", parent.display(), e))?;
    }

    let out_vec: Vec<String> = domains.into_iter().collect();
    let json = serde_json::to_string_pretty(&out_vec)
        .map_err(|e| format!("Failed to serialize JSON: {}", e))?;
    fs::write(&output_path, json)
        .map_err(|e| format!("Failed to write '{}': {}", output_path.display(), e))?;

    println!("Generated {}", output_path.display());
    println!("Source files: {}", sources.len());
    println!("Source lines: {}", total_lines);
    println!("Unique domains: {}", out_vec.len());

    Ok(())
}

fn extract_domain(line: &str) -> Option<&str> {
    let trimmed = line.trim();
    if trimmed.is_empty() || trimmed.starts_with('!') || trimmed.starts_with('[') || trimmed.starts_with("@@") {
        return None;
    }

    if let Some(rest) = trimmed.strip_prefix("||") {
        let mut end = rest.len();
        for (idx, ch) in rest.char_indices() {
            if ch == '^' || ch == '/' {
                end = idx;
                break;
            }
        }
        let domain = &rest[..end];
        return if domain.is_empty() { None } else { Some(domain) };
    }

    if trimmed.starts_with("0.0.0.0 ") || trimmed.starts_with("127.0.0.1 ") {
        let mut parts = trimmed.split_whitespace();
        let _ = parts.next();
        return parts.next();
    }

    None
}

fn default_input_path() -> Result<PathBuf, String> {
    let cwd = std::env::current_dir()
        .map_err(|e| format!("Failed to resolve cwd: {}", e))?;
    Ok(Path::new(&cwd).join("ultimate.txt"))
}
