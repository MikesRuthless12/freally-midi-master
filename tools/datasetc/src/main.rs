//! `datasetc` — the style dataset compiler/checker.
//!
//! CI runs this on every push so a malformed community model fails the build
//! rather than a user's session. It is also the fastest way for a contributor
//! to find out whether the model they just wrote is sane.
//!
//! ```text
//! datasetc validate data/     schema + lints + inheritance; the CI gate
//! datasetc lint     data/     semantic lints only, no schema
//! datasetc stats    data/     counts by type, tier and genre
//! datasetc coverage data/     which part blocks each model actually defines
//! ```

use std::collections::BTreeMap;
use std::fs;
use std::path::{Path, PathBuf};
use std::process::ExitCode;

use engine::dataset::{files, registry_from, validate, Registry};
use serde_json::Value;

const USAGE: &str = "\
datasetc — validate and report on the Freally MIDI Master style dataset

USAGE:
    datasetc <COMMAND> [DIR]

COMMANDS:
    validate    JSON Schema + semantic lints + inheritance resolution (the CI gate)
    lint        semantic lints only
    stats       counts by type, tier and genre
    coverage    which part blocks each model defines, and what it inherits

ARGS:
    DIR         dataset directory (default: data)

Exit code is 1 if anything fails, so this can gate CI directly.
";

fn main() -> ExitCode {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let command = match args.first().map(String::as_str) {
        None | Some("-h") | Some("--help") | Some("help") => {
            print!("{USAGE}");
            return ExitCode::SUCCESS;
        }
        Some(c) => c,
    };

    let dir = PathBuf::from(args.get(1).map(String::as_str).unwrap_or("data"));

    let result = match command {
        "validate" => run_validate(&dir),
        "lint" => run_lint(&dir),
        "stats" => run_stats(&dir),
        "coverage" => run_coverage(&dir),
        other => {
            eprintln!("unknown command `{other}`\n\n{USAGE}");
            return ExitCode::FAILURE;
        }
    };

    match result {
        Ok(true) => ExitCode::SUCCESS,
        Ok(false) => ExitCode::FAILURE,
        Err(e) => {
            eprintln!("error: {e}");
            ExitCode::FAILURE
        }
    }
}

/// Every `*.json` model under `dir`.
///
/// The scan itself lives in the engine, next to the loader, so this CLI and the
/// app cannot disagree about which files are models — a green run here would
/// otherwise say nothing about the set the app loads at startup. What differs is
/// the policy: an unreadable file is a problem the app skips and a failure here.
fn load(dir: &Path) -> Result<(Registry, Vec<(PathBuf, String)>), String> {
    let scan = files::scan(dir).map_err(|e| e.to_string())?;

    if let Some(problem) = scan.problems.first() {
        return Err(format!("{}: {}", problem.source, problem.message));
    }
    if scan.files.is_empty() {
        return Err(format!("no model files found under {}", dir.display()));
    }
    Ok((registry_from(scan.files.clone()), scan.files))
}

/// Report parse-time rejections. Returns false if there were any.
fn report_rejections(registry: &Registry) -> bool {
    if registry.rejected().is_empty() {
        return true;
    }
    for rejected in registry.rejected() {
        eprintln!("FAIL {}", rejected.path.display());
        eprintln!("     {}", rejected.error);
    }
    false
}

fn run_validate(dir: &Path) -> Result<bool, String> {
    let (registry, files) = load(dir)?;
    let mut ok = report_rejections(&registry);

    // Findings accumulate per file so each one is reported once, under a single
    // heading, however many checks contributed to it.
    let mut findings: BTreeMap<PathBuf, Vec<String>> = BTreeMap::new();
    // Which file declared which model id, so an inheritance failure can be
    // attributed to a file rather than a bare id.
    let mut file_of: BTreeMap<String, PathBuf> = BTreeMap::new();

    // 1. Shape, against the JSON Schema.
    let schema_path = dir.join("schema").join("artist-style.schema.json");
    let validator = match compile_schema(&schema_path) {
        Ok(v) => Some(v),
        Err(e) => {
            eprintln!("FAIL {}", schema_path.display());
            eprintln!("     {e}");
            ok = false;
            None
        }
    };

    for (path, text) in &files {
        let Ok(instance) = serde_json::from_str::<Value>(text) else {
            continue; // already reported as a rejection
        };

        if let Some(id) = instance.get("id").and_then(Value::as_str) {
            file_of.insert(id.to_owned(), path.clone());
        }

        let entry = findings.entry(path.clone()).or_default();

        if let Some(validator) = &validator {
            for e in validator.iter_errors(&instance) {
                // The instance pointer is what tells an author where to look.
                entry.push(format!("{}: {e}", pointer_or_root(e.instance_path())));
            }
        }

        // 2. Semantics, checked on the raw model so a finding is attributed to
        //    the file that actually contains it rather than to an heir.
        for f in validate::lint(&instance) {
            entry.push(f.to_string());
        }
    }

    // 3. Inheritance: every model must resolve. Lint failures are already
    //    reported against their own file above, so only surface one here if the
    //    model's own file was clean — meaning the problem emerged from the
    //    merge itself.
    let (resolved, errors) = registry.resolve_all();
    for (id, error) in &errors {
        let path = file_of.get(id).cloned();
        let already_reported = path
            .as_ref()
            .and_then(|p| findings.get(p))
            .is_some_and(|f| !f.is_empty());

        if matches!(error, engine::dataset::DatasetError::Lint(_)) && already_reported {
            continue;
        }

        match path {
            Some(p) => findings.entry(p).or_default().push(format!("{error}")),
            None => {
                eprintln!("FAIL model `{id}`");
                eprintln!("     {error}");
                ok = false;
            }
        }
    }

    for (path, mut items) in findings {
        if items.is_empty() {
            continue;
        }
        ok = false;
        items.sort();
        items.dedup();
        eprintln!("FAIL {}", path.display());
        for item in items {
            eprintln!("     {item}");
        }
    }

    if ok {
        println!(
            "ok: {} model{} validated in {}",
            resolved.len(),
            if resolved.len() == 1 { "" } else { "s" },
            dir.display()
        );
    }
    Ok(ok)
}

fn pointer_or_root(pointer: &jsonschema::paths::Location) -> String {
    let s = pointer.to_string();
    if s.is_empty() {
        "/".into()
    } else {
        s
    }
}

fn compile_schema(path: &Path) -> Result<jsonschema::Validator, String> {
    let text = fs::read_to_string(path).map_err(|e| format!("{}: {e}", path.display()))?;
    let value: Value = serde_json::from_str(&text).map_err(|e| format!("invalid JSON: {e}"))?;
    jsonschema::validator_for(&value).map_err(|e| format!("invalid schema: {e}"))
}

fn lint_files(files: &[(PathBuf, String)]) -> bool {
    let mut ok = true;
    for (path, text) in files {
        let Ok(value) = serde_json::from_str::<Value>(text) else {
            continue;
        };
        let findings = validate::lint(&value);
        if !findings.is_empty() {
            ok = false;
            eprintln!("FAIL {}", path.display());
            for f in findings {
                eprintln!("     {f}");
            }
        }
    }
    ok
}

fn run_lint(dir: &Path) -> Result<bool, String> {
    let (registry, files) = load(dir)?;
    let mut ok = report_rejections(&registry);
    if !lint_files(&files) {
        ok = false;
    }
    if ok {
        println!("ok: {} model files linted clean", files.len());
    }
    Ok(ok)
}

fn run_stats(dir: &Path) -> Result<bool, String> {
    let (registry, _) = load(dir)?;
    if !report_rejections(&registry) {
        return Ok(false);
    }

    let mut by_type: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_tier: BTreeMap<String, usize> = BTreeMap::new();
    let mut by_genre: BTreeMap<String, usize> = BTreeMap::new();
    let mut unsourced = Vec::new();

    for id in registry.ids() {
        let model = registry.raw(id).expect("id came from the registry");

        let type_name = model.get("type").and_then(Value::as_str).unwrap_or("?");
        *by_type.entry(type_name.to_owned()).or_default() += 1;

        let tier = model
            .get("tier")
            .and_then(Value::as_str)
            .unwrap_or("(none)");
        *by_tier.entry(tier.to_owned()).or_default() += 1;

        if let Some(genres) = model.get("genres").and_then(Value::as_array) {
            for g in genres.iter().filter_map(Value::as_str) {
                *by_genre.entry(g.to_owned()).or_default() += 1;
            }
        }

        let sourced = model
            .get("sources")
            .and_then(Value::as_array)
            .is_some_and(|s| !s.is_empty());
        if !sourced {
            unsourced.push(id.clone());
        }
    }

    println!("{} models in {}\n", registry.len(), dir.display());
    print_table("By type", &by_type);
    print_table("By tier", &by_tier);
    print_table("By genre", &by_genre);

    if unsourced.is_empty() {
        println!("Every model cites its sources.");
    } else {
        println!("Models with no sources ({}):", unsourced.len());
        for id in &unsourced {
            println!("  {id}");
        }
    }

    Ok(true)
}

fn print_table(title: &str, counts: &BTreeMap<String, usize>) {
    println!("{title}");
    if counts.is_empty() {
        println!("  (none)\n");
        return;
    }
    let width = counts.keys().map(String::len).max().unwrap_or(0);
    for (key, count) in counts {
        println!("  {key:<width$}  {count}");
    }
    println!();
}

/// The part blocks a complete model is expected to carry.
const PART_BLOCKS: &[&str] = &[
    "session",
    "drums",
    "chords",
    "melody",
    "countermelody",
    "bassline",
    "arrangement",
];

fn run_coverage(dir: &Path) -> Result<bool, String> {
    let (registry, _) = load(dir)?;
    if !report_rejections(&registry) {
        return Ok(false);
    }

    // `own` = declared in the file; `inherited` = arrives via extends. The
    // difference is the interesting part: it shows what a model actually says
    // versus what it merely accepts from its parents.
    println!("{:<16} {}", "MODEL", PART_BLOCKS.join("  "));
    println!("{}", "-".repeat(16 + PART_BLOCKS.join("  ").len()));

    let mut ids: Vec<&String> = registry.ids().collect();
    ids.sort();

    for id in ids {
        let own = registry.raw(id).expect("id came from the registry");
        let resolved = registry.resolve(id).ok();

        let mut cells = Vec::new();
        for block in PART_BLOCKS {
            let declared = own.get(*block).is_some();
            let present = resolved.as_ref().is_some_and(|m| match *block {
                "session" => m.session.is_some(),
                other => m.blocks.contains_key(other),
            });
            // ● declared here · ○ inherited · · absent
            let mark = if declared {
                "●"
            } else if present {
                "○"
            } else {
                "·"
            };
            cells.push(format!("{mark:^width$}", width = block.len()));
        }
        println!("{id:<16} {}", cells.join("  "));
    }

    println!("\n● declared in this model   ○ inherited   · absent");
    Ok(true)
}
