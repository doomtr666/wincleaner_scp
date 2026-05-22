use std::path::Path;
use crate::utils::get_dir_size;
use crate::registry::{get_installed_apps, get_program_dirs};

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct AppDataLeftover {
    pub name: String,
    pub path: String,
    pub size_mb: f64,
    pub last_modified: String,
    pub age_days: u64,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct AppDataLeftoversResponse {
    pub leftovers: Vec<AppDataLeftover>,
}

pub fn get_appdata_leftovers() -> Vec<AppDataLeftover> {
    let mut leftovers = Vec::new();
    let installed_apps = get_installed_apps();
    let program_dirs = get_program_dirs();

    let paths = [
        std::env::var("LOCALAPPDATA"),
        std::env::var("APPDATA"),
    ];

    let exclude_dirs = [
        "microsoft", "packages", "temp", "google", "mozilla", "github", "git", 
        "npm", "yarn", "rustup", "cargo", ".gemini", "antigravity-ide", "adobe", 
        "intel", "nvidia", "dropbox", "onedrive", "apple", "apple computer", "spotify",
        "assembly", "com_microsoft_visualstudio_telemetry", "d3dscache", "identities",
        "virtualstore"
    ];

    let now = std::time::SystemTime::now();

    for path_res in paths {
        if let Ok(path_str) = path_res {
            let appdata_path = Path::new(&path_str);
            if appdata_path.exists() {
                if let Ok(entries) = std::fs::read_dir(appdata_path) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let name = entry.file_name().to_string_lossy().into_owned();
                        let name_lower = name.to_lowercase();
                        
                        if exclude_dirs.contains(&name_lower.as_str()) {
                            continue;
                        }

                        let entry_path = entry.path();
                        if !entry_path.is_dir() {
                            continue;
                        }

                        let is_installed = program_dirs.contains(&name_lower) 
                            || installed_apps.contains(&name_lower)
                            || program_dirs.iter().any(|d| d.contains(&name_lower) || name_lower.contains(d))
                            || installed_apps.iter().any(|a| a.contains(&name_lower) || name_lower.contains(a));

                        if !is_installed {
                            let mut last_modified = "Unknown".to_string();
                            let mut age_days = 0;
                            if let Ok(meta) = entry.metadata() {
                                if let Ok(modified) = meta.modified() {
                                    let datetime: chrono::DateTime<chrono::Local> = modified.into();
                                    last_modified = datetime.format("%Y-%m-%d %H:%M:%S").to_string();
                                    if let Ok(age) = now.duration_since(modified) {
                                        age_days = age.as_secs() / 86400;
                                    }
                                }
                            }

                            if age_days > 15 {
                                let size_mb = get_dir_size(&entry_path) as f64 / 1_048_576.0;
                                leftovers.push(AppDataLeftover {
                                    name,
                                    path: entry_path.to_string_lossy().into_owned(),
                                    size_mb,
                                    last_modified,
                                    age_days,
                                });
                            }
                        }
                    }
                }
            }
        }
    }
    leftovers.sort_by(|a, b| b.size_mb.partial_cmp(&a.size_mb).unwrap_or(std::cmp::Ordering::Equal));
    leftovers
}
