use std::path::Path;

pub fn expand_and_clean_path(path: &str) -> String {
    let mut cleaned = path.trim().to_string();
    if cleaned.starts_with('"') && cleaned.ends_with('"') && cleaned.len() >= 2 {
        cleaned = cleaned[1..cleaned.len() - 1].to_string();
    }
    
    let mut result = String::new();
    let mut last_pos = 0;
    while let Some(start) = cleaned[last_pos..].find('%') {
        let abs_start = last_pos + start;
        if let Some(end) = cleaned[abs_start + 1..].find('%') {
            let abs_end = abs_start + 1 + end;
            let var_name = &cleaned[abs_start + 1..abs_end];
            let value = std::env::var(var_name).unwrap_or_else(|_| format!("%{}%", var_name));
            result.push_str(&cleaned[last_pos..abs_start]);
            result.push_str(&value);
            last_pos = abs_end + 1;
        } else {
            break;
        }
    }
    result.push_str(&cleaned[last_pos..]);
    result
}

pub fn extract_exec_path(cmd: &str) -> String {
    let cmd = cmd.trim();
    if cmd.starts_with('"') {
        if let Some(end_idx) = cmd[1..].find('"') {
            return cmd[1..end_idx + 1].to_string();
        }
    }
    for ext in &[".exe", ".bat", ".cmd", ".msi", ".vbs"] {
        if let Some(idx) = cmd.to_lowercase().find(ext) {
            return cmd[..idx + ext.len()].to_string();
        }
    }
    cmd.split_whitespace().next().unwrap_or("").to_string()
}

pub fn get_recycle_bin_stats() -> (u64, usize) {
    let mut size = 0u64;
    let mut count = 0;
    let recycle_path = Path::new("C:\\$Recycle.Bin");
    if recycle_path.exists() {
        for entry in walkdir::WalkDir::new(recycle_path)
            .into_iter()
            .filter_map(|e| e.ok())
        {
            if entry.file_type().is_file() {
                size += entry.metadata().map(|m| m.len()).unwrap_or(0);
                count += 1;
            }
        }
    }
    (size, count)
}

pub fn get_dir_size(path: &Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}
