use anyhow::Result;
use rmcp::{
    ServerHandler, ServiceExt,
    handler::server::tool::ToolRouter,
    model::*,
    tool, tool_handler, tool_router,
    transport::stdio,
};
use rmcp::{handler::server::wrapper::{Parameters, Json}, schemars};
use tracing_subscriber::{self, EnvFilter};
use sysinfo::Disks;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE, KEY_WRITE};
use winreg::RegKey;
use std::collections::HashSet;

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct DiskInfo {
    pub name: String,
    pub mount_point: String,
    pub total_space_gb: f64,
    pub available_space_gb: f64,
    pub file_system: String,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct DisksResponse {
    pub disks: Vec<DiskInfo>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct ScanRequest {
    #[schemars(description = "The absolute path of the directory to scan")]
    pub path: String,
    #[schemars(description = "Minimum size in MB to report a directory (default: 100)")]
    pub min_size_mb: Option<u64>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct DirectoryHotspot {
    pub name: String,
    pub path: String,
    pub size_mb: f64,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct ScanResponse {
    pub hotspots: Vec<DirectoryHotspot>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct InspectRequest {
    #[schemars(description = "The absolute path of the directory to inspect")]
    pub path: String,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct FileDetail {
    pub name: String,
    pub size_mb: f64,
    pub is_dir: bool,
    pub last_modified: String,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct InspectResult {
    pub path: String,
    pub total_size_mb: f64,
    pub entries: Vec<FileDetail>,
    pub summary: String,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteRequest {
    #[schemars(description = "The absolute path of the file or directory to delete")]
    pub path: String,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct DownloadCandidate {
    pub name: String,
    pub path: String,
    pub size_mb: f64,
    pub last_modified: String,
    pub age_days: u64,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct UserJunkResponse {
    pub recycle_bin_size_mb: f64,
    pub recycle_bin_file_count: usize,
    pub obsolete_downloads: Vec<DownloadCandidate>,
}

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

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct RegistryStartupJunk {
    pub hive: String,
    pub key_path: String,
    pub value_name: String,
    pub command: String,
    pub expected_path: String,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct RegistryUninstallJunk {
    pub hive: String,
    pub key_path: String,
    pub display_name: String,
    pub invalid_path: String,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct RegistryJunkResponse {
    pub broken_startup_items: Vec<RegistryStartupJunk>,
    pub leftover_uninstall_keys: Vec<RegistryUninstallJunk>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct DeleteRegistryRequest {
    #[schemars(description = "The registry hive (HKEY_CURRENT_USER or HKEY_LOCAL_MACHINE)")]
    pub hive: String,
    #[schemars(description = "The subkey path under the hive")]
    pub sub_key: String,
    #[schemars(description = "The name of the registry value to delete")]
    pub value_name: String,
}

fn expand_and_clean_path(path: &str) -> String {
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

fn extract_exec_path(cmd: &str) -> String {
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

fn get_recycle_bin_stats() -> (u64, usize) {
    let mut size = 0u64;
    let mut count = 0;
    let recycle_path = std::path::Path::new("C:\\$Recycle.Bin");
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

fn get_installed_apps() -> HashSet<String> {
    let mut apps = HashSet::new();
    let hives_and_paths = [
        (HKEY_CURRENT_USER, "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
        (HKEY_LOCAL_MACHINE, "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
        (HKEY_LOCAL_MACHINE, "Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
    ];

    for (hive, path) in hives_and_paths {
        if let Ok(uninstall_key) = RegKey::predef(hive).open_subkey(path) {
            for name in uninstall_key.enum_keys().filter_map(|x| x.ok()) {
                if let Ok(sub_key) = uninstall_key.open_subkey(&name) {
                    if let Ok(display_name) = sub_key.get_value::<String, _>("DisplayName") {
                        apps.insert(display_name.to_lowercase());
                    }
                    if let Ok(publisher) = sub_key.get_value::<String, _>("Publisher") {
                        apps.insert(publisher.to_lowercase());
                    }
                }
                apps.insert(name.to_lowercase());
            }
        }
    }
    apps
}

fn get_program_dirs() -> HashSet<String> {
    let mut dirs = HashSet::new();
    for base in &["C:\\Program Files", "C:\\Program Files (x86)"] {
        let path = std::path::Path::new(base);
        if path.exists() {
            if let Ok(entries) = std::fs::read_dir(path) {
                for entry in entries.filter_map(|e| e.ok()) {
                    dirs.insert(entry.file_name().to_string_lossy().to_lowercase());
                }
            }
        }
    }
    dirs
}

fn get_appdata_leftovers() -> Vec<AppDataLeftover> {
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
            let appdata_path = std::path::Path::new(&path_str);
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

fn scan_registry_startup() -> Vec<RegistryStartupJunk> {
    let mut startup_junk = Vec::new();
    let hives_and_paths = [
        (HKEY_CURRENT_USER, "Software\\Microsoft\\Windows\\CurrentVersion\\Run", "HKEY_CURRENT_USER"),
        (HKEY_CURRENT_USER, "Software\\Microsoft\\Windows\\CurrentVersion\\RunOnce", "HKEY_CURRENT_USER"),
        (HKEY_LOCAL_MACHINE, "Software\\Microsoft\\Windows\\CurrentVersion\\Run", "HKEY_LOCAL_MACHINE"),
        (HKEY_LOCAL_MACHINE, "Software\\Microsoft\\Windows\\CurrentVersion\\RunOnce", "HKEY_LOCAL_MACHINE"),
    ];

    for (hive, path, hive_name) in hives_and_paths {
        if let Ok(run_key) = RegKey::predef(hive).open_subkey(path) {
            for val_name in run_key.enum_values().filter_map(|x| x.ok().map(|(n, _)| n)) {
                if let Ok(cmd) = run_key.get_value::<String, _>(&val_name) {
                    let exec_path = extract_exec_path(&cmd);
                    if !exec_path.is_empty() {
                        let clean_path = expand_and_clean_path(&exec_path);
                        let p = std::path::Path::new(&clean_path);
                        if !p.exists() {
                            startup_junk.push(RegistryStartupJunk {
                                hive: hive_name.to_string(),
                                key_path: path.to_string(),
                                value_name: val_name,
                                command: cmd,
                                expected_path: clean_path,
                            });
                        }
                    }
                }
            }
        }
    }
    startup_junk
}

fn scan_leftover_uninstall() -> Vec<RegistryUninstallJunk> {
    let mut uninstall_junk = Vec::new();
    let hives_and_paths = [
        (HKEY_CURRENT_USER, "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall", "HKEY_CURRENT_USER"),
        (HKEY_LOCAL_MACHINE, "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall", "HKEY_LOCAL_MACHINE"),
        (HKEY_LOCAL_MACHINE, "Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall", "HKEY_LOCAL_MACHINE"),
    ];

    for (hive, path, hive_name) in hives_and_paths {
        if let Ok(uninstall_key) = RegKey::predef(hive).open_subkey(path) {
            for name in uninstall_key.enum_keys().filter_map(|x| x.ok()) {
                if let Ok(sub_key) = uninstall_key.open_subkey(&name) {
                    let display_name = sub_key.get_value::<String, _>("DisplayName").unwrap_or_else(|_| name.clone());
                    
                    let mut is_invalid = false;
                    let mut invalid_path = String::new();
                    
                    if let Ok(install_loc) = sub_key.get_value::<String, _>("InstallLocation") {
                        let loc = install_loc.trim();
                        if !loc.is_empty() {
                            let clean_loc = expand_and_clean_path(loc);
                            let p = std::path::Path::new(&clean_loc);
                            if !p.exists() {
                                is_invalid = true;
                                invalid_path = clean_loc;
                            }
                        }
                    } else if let Ok(uninstall_str) = sub_key.get_value::<String, _>("UninstallString") {
                        let exec_path = extract_exec_path(&uninstall_str);
                        if !exec_path.is_empty() {
                            let clean_exec = expand_and_clean_path(&exec_path);
                            let p = std::path::Path::new(&clean_exec);
                            if !p.exists() {
                                is_invalid = true;
                                invalid_path = clean_exec;
                            }
                        }
                    }

                    if is_invalid {
                        uninstall_junk.push(RegistryUninstallJunk {
                            hive: hive_name.to_string(),
                            key_path: format!("{}\\{}", path, name),
                            display_name,
                            invalid_path,
                        });
                    }
                }
            }
        }
    }
    uninstall_junk
}

#[derive(Debug, Clone)]
pub struct DiskAnalyzer {
    tool_router: ToolRouter<Self>,
}

fn get_dir_size(path: &std::path::Path) -> u64 {
    walkdir::WalkDir::new(path)
        .into_iter()
        .filter_map(|entry| entry.ok())
        .filter_map(|entry| entry.metadata().ok())
        .filter(|metadata| metadata.is_file())
        .map(|metadata| metadata.len())
        .sum()
}

#[tool_router]
impl DiskAnalyzer {
    pub fn new() -> Self {
        Self {
            tool_router: Self::tool_router(),
        }
    }

    #[tool(description = "Get the available drives on the system with space details")]
    fn get_disks(&self) -> Json<DisksResponse> {
        let disks = Disks::new_with_refreshed_list();
        let mut list = Vec::new();
        for disk in &disks {
            list.push(DiskInfo {
                name: disk.name().to_string_lossy().into_owned(),
                mount_point: disk.mount_point().to_string_lossy().into_owned(),
                total_space_gb: disk.total_space() as f64 / 1_073_741_824.0,
                available_space_gb: disk.available_space() as f64 / 1_073_741_824.0,
                file_system: disk.file_system().to_string_lossy().into_owned(),
            });
        }
        Json(DisksResponse { disks: list })
    }

    #[tool(description = "Scan a directory to identify large subdirectories (hotspots)")]
    fn scan_directory(&self, Parameters(ScanRequest { path, min_size_mb }): Parameters<ScanRequest>) -> Result<Json<ScanResponse>, String> {
        let root_path = std::path::Path::new(&path);
        if !root_path.exists() {
            return Err(format!("Path '{}' does not exist", path));
        }
        if !root_path.is_dir() {
            return Err(format!("Path '{}' is not a directory", path));
        }

        let min_bytes = min_size_mb.unwrap_or(100) * 1024 * 1024;
        let mut hotspots = Vec::new();

        let entries = match std::fs::read_dir(root_path) {
            Ok(entries) => entries,
            Err(e) => return Err(format!("Failed to read directory: {}", e)),
        };

        for entry in entries {
            if let Ok(entry) = entry {
                let entry_path = entry.path();
                if entry_path.is_dir() {
                    let size = get_dir_size(&entry_path);
                    if size >= min_bytes {
                        hotspots.push(DirectoryHotspot {
                            name: entry.file_name().to_string_lossy().into_owned(),
                            path: entry_path.to_string_lossy().into_owned(),
                            size_mb: size as f64 / 1_048_576.0,
                        });
                    }
                } else if entry_path.is_file() {
                    if let Ok(meta) = entry.metadata() {
                        let size = meta.len();
                        if size >= min_bytes {
                            hotspots.push(DirectoryHotspot {
                                name: entry.file_name().to_string_lossy().into_owned(),
                                path: entry_path.to_string_lossy().into_owned(),
                                size_mb: size as f64 / 1_048_576.0,
                            });
                        }
                    }
                }
            }
        }

        hotspots.sort_by(|a, b| b.size_mb.partial_cmp(&a.size_mb).unwrap_or(std::cmp::Ordering::Equal));

        Ok(Json(ScanResponse { hotspots }))
    }

    #[tool(description = "Inspect a specific directory's contents to help qualify what it is")]
    fn inspect_directory(&self, Parameters(InspectRequest { path }): Parameters<InspectRequest>) -> Result<Json<InspectResult>, String> {
        let root_path = std::path::Path::new(&path);
        if !root_path.exists() {
            return Err(format!("Path '{}' does not exist", path));
        }

        let entries = match std::fs::read_dir(root_path) {
            Ok(entries) => entries,
            Err(e) => return Err(format!("Failed to read directory: {}", e)),
        };

        let mut file_details = Vec::new();
        let mut total_size = 0u64;
        let mut has_node_modules = false;
        let mut has_target = false;
        let mut has_git = false;
        let mut has_gradle = false;
        let mut has_bazel = false;

        for entry in entries {
            if let Ok(entry) = entry {
                let entry_path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                let is_dir = entry_path.is_dir();
                
                let size = if is_dir {
                    if name == "node_modules" { has_node_modules = true; }
                    if name == "target" { has_target = true; }
                    if name == ".git" { has_git = true; }
                    if name == ".gradle" { has_gradle = true; }
                    if name == "bazel-bin" || name == "bazel-out" { has_bazel = true; }
                    let sz = get_dir_size(&entry_path);
                    total_size += sz;
                    sz
                } else {
                    let sz = entry.metadata().map(|m| m.len()).unwrap_or(0);
                    total_size += sz;
                    sz
                };

                let last_modified = entry.metadata()
                    .and_then(|m| m.modified())
                    .map(|t| {
                        let datetime: chrono::DateTime<chrono::Local> = t.into();
                        datetime.format("%Y-%m-%d %H:%M:%S").to_string()
                    })
                    .unwrap_or_else(|_| "Unknown".to_string());

                file_details.push(FileDetail {
                    name,
                    size_mb: size as f64 / 1_048_576.0,
                    is_dir,
                    last_modified,
                });
            }
        }

        file_details.sort_by(|a, b| b.size_mb.partial_cmp(&a.size_mb).unwrap_or(std::cmp::Ordering::Equal));

        let mut detections = Vec::new();
        if has_node_modules { detections.push("Node.js (node_modules)"); }
        if has_target { detections.push("Rust Cargo (target)"); }
        if has_git { detections.push("Git Repository (.git)"); }
        if has_gradle { detections.push("Gradle (.gradle)"); }
        if has_bazel { detections.push("Bazel Build (bazel-*)"); }

        let summary = if detections.is_empty() {
            "Dossier standard ou inconnu".to_string()
        } else {
            format!("Dossier contenant des signatures de technologies de développement : {}", detections.join(", "))
        };

        Ok(Json(InspectResult {
            path,
            total_size_mb: total_size as f64 / 1_048_576.0,
            entries: file_details,
            summary,
        }))
    }

    #[tool(description = "Delete a file or directory recursively (irreversible)")]
    fn delete_path(&self, Parameters(DeleteRequest { path }): Parameters<DeleteRequest>) -> Result<String, String> {
        let p = std::path::Path::new(&path);
        if !p.exists() {
            return Err(format!("Path '{}' does not exist", path));
        }

        if p.is_dir() {
            match std::fs::remove_dir_all(p) {
                Ok(_) => Ok(format!("Directory '{}' and all its contents deleted successfully", path)),
                Err(e) => Err(format!("Failed to delete directory: {}", e)),
            }
        } else {
            match std::fs::remove_file(p) {
                Ok(_) => Ok(format!("File '{}' deleted successfully", path)),
                Err(e) => Err(format!("Failed to delete file: {}", e)),
            }
        }
    }

    #[tool(description = "Scan for user junk files: Recycle Bin size/count and obsolete Download files (>30 days old installers/archives)")]
    fn scan_user_junk(&self) -> Result<Json<UserJunkResponse>, String> {
        let (recycle_size, recycle_count) = get_recycle_bin_stats();
        let recycle_size_mb = recycle_size as f64 / 1_048_576.0;

        let mut obsolete_downloads = Vec::new();
        if let Ok(user_profile) = std::env::var("USERPROFILE") {
            let downloads_path = std::path::Path::new(&user_profile).join("Downloads");
            if downloads_path.exists() {
                let now = std::time::SystemTime::now();
                let obsolete_extensions = [
                    "exe", "msi", "zip", "rar", "7z", "tar", "gz", "cab", "iso", "dmg", "pkg"
                ];

                if let Ok(entries) = std::fs::read_dir(downloads_path) {
                    for entry in entries.filter_map(|e| e.ok()) {
                        let entry_path = entry.path();
                        if entry_path.is_file() {
                            let ext = entry_path.extension()
                                .and_then(|s| s.to_str())
                                .unwrap_or("")
                                .to_lowercase();

                            if obsolete_extensions.contains(&ext.as_str()) {
                                if let Ok(meta) = entry.metadata() {
                                    if let Ok(modified) = meta.modified() {
                                        if let Ok(age) = now.duration_since(modified) {
                                            let age_days = age.as_secs() / 86400;
                                            if age_days > 30 {
                                                let last_modified = {
                                                    let datetime: chrono::DateTime<chrono::Local> = modified.into();
                                                    datetime.format("%Y-%m-%d %H:%M:%S").to_string()
                                                };
                                                obsolete_downloads.push(DownloadCandidate {
                                                    name: entry.file_name().to_string_lossy().into_owned(),
                                                    path: entry_path.to_string_lossy().into_owned(),
                                                    size_mb: meta.len() as f64 / 1_048_576.0,
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
                }
            }
        }

        obsolete_downloads.sort_by(|a, b| b.size_mb.partial_cmp(&a.size_mb).unwrap_or(std::cmp::Ordering::Equal));

        Ok(Json(UserJunkResponse {
            recycle_bin_size_mb: recycle_size_mb,
            recycle_bin_file_count: recycle_count,
            obsolete_downloads,
        }))
    }

    #[tool(description = "Scan AppData (Local/Roaming) for leftover directories from uninstalled applications")]
    fn scan_appdata_leftovers(&self) -> Result<Json<AppDataLeftoversResponse>, String> {
        let leftovers = get_appdata_leftovers();
        Ok(Json(AppDataLeftoversResponse { leftovers }))
    }

    #[tool(description = "Scan the registry for startup entries pointing to non-existent files and leftover uninstall entries")]
    fn scan_registry_junk(&self) -> Result<Json<RegistryJunkResponse>, String> {
        let broken_startup_items = scan_registry_startup();
        let leftover_uninstall_keys = scan_leftover_uninstall();
        Ok(Json(RegistryJunkResponse {
            broken_startup_items,
            leftover_uninstall_keys,
        }))
    }

    #[tool(description = "Delete a registry value under HKEY_CURRENT_USER or HKEY_LOCAL_MACHINE")]
    fn delete_registry_value(&self, Parameters(DeleteRegistryRequest { hive, sub_key, value_name }): Parameters<DeleteRegistryRequest>) -> Result<String, String> {
        let hive_predef = match hive.as_str() {
            "HKEY_CURRENT_USER" => HKEY_CURRENT_USER,
            "HKEY_LOCAL_MACHINE" => HKEY_LOCAL_MACHINE,
            _ => return Err(format!("Unsupported or invalid registry hive: '{}'", hive)),
        };

        let key = RegKey::predef(hive_predef)
            .open_subkey_with_flags(&sub_key, KEY_WRITE)
            .map_err(|e| format!("Failed to open registry key: {}", e))?;

        key.delete_value(&value_name)
            .map_err(|e| format!("Failed to delete registry value '{}': {}", value_name, e))?;

        Ok(format!("Successfully deleted registry value '{}' under {}\\{}", value_name, hive, sub_key))
    }
}

#[tool_handler]
impl ServerHandler for DiskAnalyzer {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("A disk analyzer MCP server that helps identify large folders".into()),
            capabilities: ServerCapabilities::builder().enable_tools().build(),
            ..Default::default()
        }
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(EnvFilter::from_default_env().add_directive(tracing::Level::DEBUG.into()))
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .init();

    tracing::info!("Starting Disk Analyzer MCP server");

    let service = DiskAnalyzer::new().serve(stdio()).await.inspect_err(|e| {
        tracing::error!("serving error: {:?}", e);
    })?;

    service.waiting().await?;
    Ok(())
}
