// src/labs/registry.rs
use std::fs;
use super::spec::LabSpec;

pub fn load_labs(dir: &str) -> Result<Vec<LabSpec>, String> {
    let entries = fs::read_dir(dir)
        .map_err(|e| format!("Cannot read labs directory '{}': {}", dir, e))?;

    let mut labs: Vec<LabSpec> = Vec::new();
    let mut errors: Vec<String> = Vec::new();

    for entry in entries.flatten() {
        let path = entry.path();
        if !path.is_file() { continue; }
        let ext = path.extension().and_then(|e| e.to_str()).unwrap_or("");
        if ext != "yaml" && ext != "yml" { continue; }

        match fs::read_to_string(&path) {
            Ok(content) => match serde_yaml::from_str::<LabSpec>(&content) {
                Ok(spec) => labs.push(spec),
                Err(e) => errors.push(format!("YAML error in {:?}: {}", path, e)),
            },
            Err(e) => errors.push(format!("Cannot read {:?}: {}", path, e)),
        }
    }

    for e in &errors { eprintln!("⚠️  {}", e); }

    labs.retain(|l| l.meta.enabled);
    labs.sort_by_key(|l| l.meta.order);
    Ok(labs)
}