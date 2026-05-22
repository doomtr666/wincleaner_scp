mod utils;

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
use winreg::enums::{KEY_READ, KEY_WRITE};
use winreg::RegKey;

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

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct RegistryRequest {
    #[schemars(description = "The registry hive (HKEY_CURRENT_USER or HKEY_LOCAL_MACHINE)")]
    pub hive: String,
    #[schemars(description = "The subkey path under the hive")]
    pub sub_key: String,
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

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct RegistryKeysResponse {
    pub keys: Vec<String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct RegistryValuesResponse {
    pub values: std::collections::HashMap<String, String>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct RegistryTreeResponse {
    pub keys: std::collections::HashMap<String, std::collections::HashMap<String, String>>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema, Clone)]
pub struct InstalledApp {
    pub name: String,
    pub publisher: String,
    pub version: String,
    pub size_mb: f64,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct InstalledAppsResponse {
    pub apps: Vec<InstalledApp>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct AppdataOrphan {
    pub folder_name: String,
    pub path: String,
    pub size_mb: f64,
    pub location_type: String,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct AppdataOrphansResponse {
    pub orphans: Vec<AppdataOrphan>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct FindFilesRequest {
    #[schemars(description = "The directory path to search in")]
    pub path: String,
    #[schemars(description = "Optional list of extensions to match (without dot, e.g. ['zip', 'exe'])")]
    pub extensions: Option<Vec<String>>,
    #[schemars(description = "Minimum age in days since last modification")]
    pub min_age_days: Option<u64>,
    #[schemars(description = "Minimum size in MB")]
    pub min_size_mb: Option<f64>,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct FoundFile {
    pub name: String,
    pub path: String,
    pub size_mb: f64,
    pub last_modified: String,
    pub age_days: u64,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct FindFilesResponse {
    pub files: Vec<FoundFile>,
}

#[derive(Debug, serde::Deserialize, schemars::JsonSchema)]
pub struct GetDirSizeRequest {
    #[schemars(description = "The absolute path of the directory")]
    pub path: String,
}

#[derive(Debug, serde::Serialize, schemars::JsonSchema)]
pub struct GetDirSizeResponse {
    pub size_mb: f64,
}

fn parse_hive(hive_str: &str) -> Result<winreg::HKEY, String> {
    match hive_str.to_uppercase().as_str() {
        "HKEY_CURRENT_USER" | "HKCU" => Ok(winreg::enums::HKEY_CURRENT_USER),
        "HKEY_LOCAL_MACHINE" | "HKLM" => Ok(winreg::enums::HKEY_LOCAL_MACHINE),
        _ => Err(format!("Unsupported or invalid registry hive: '{}'", hive_str)),
    }
}

fn fetch_installed_apps() -> Vec<InstalledApp> {
    let mut apps = Vec::new();
    let paths = vec![
        (winreg::enums::HKEY_LOCAL_MACHINE, "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
        (winreg::enums::HKEY_LOCAL_MACHINE, "Software\\Wow6432Node\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
        (winreg::enums::HKEY_CURRENT_USER, "Software\\Microsoft\\Windows\\CurrentVersion\\Uninstall"),
    ];

    for (hive, path) in paths {
        if let Ok(key) = RegKey::predef(hive).open_subkey_with_flags(path, KEY_READ) {
            for subkey_name in key.enum_keys().filter_map(|x| x.ok()) {
                if let Ok(child_key) = key.open_subkey_with_flags(&subkey_name, KEY_READ) {
                    let display_name = child_key.get_value::<String, _>("DisplayName").unwrap_or_default();
                    if display_name.is_empty() {
                        continue;
                    }
                    
                    let publisher = child_key.get_value::<String, _>("Publisher").unwrap_or_default();
                    let version = child_key.get_value::<String, _>("DisplayVersion").unwrap_or_default();
                    
                    let size_kb = match child_key.get_value::<u32, _>("EstimatedSize") {
                        Ok(s) => s as f64,
                        Err(_) => match child_key.get_value::<u64, _>("EstimatedSize") {
                            Ok(s) => s as f64,
                            Err(_) => 0.0
                        }
                    };
                    
                    apps.push(InstalledApp {
                        name: display_name,
                        publisher,
                        version,
                        size_mb: size_kb / 1024.0,
                    });
                }
            }
        }
    }
    apps.sort_by(|a, b| b.size_mb.partial_cmp(&a.size_mb).unwrap_or(std::cmp::Ordering::Equal));
    apps
}

#[derive(Debug, Clone)]
pub struct WinCleaner {
    tool_router: ToolRouter<Self>,
}

#[tool_router]
impl WinCleaner {
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
                    let size = utils::get_dir_size(&entry_path);
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

        for entry in entries {
            if let Ok(entry) = entry {
                let entry_path = entry.path();
                let name = entry.file_name().to_string_lossy().into_owned();
                let is_dir = entry_path.is_dir();
                
                let size = if is_dir {
                    let sz = utils::get_dir_size(&entry_path);
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
        Ok(Json(InspectResult {
            path,
            total_size_mb: total_size as f64 / 1_048_576.0,
            entries: file_details,
            summary: "Generic directory inspection".to_string(),
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

    #[tool(description = "Empty the Windows global recycle bin silently")]
    fn empty_recycle_bin(&self) -> Result<String, String> {
        match std::process::Command::new("powershell")
            .arg("-NoProfile")
            .arg("-Command")
            .arg("Clear-RecycleBin -Force -ErrorAction SilentlyContinue")
            .status()
        {
            Ok(status) if status.success() => Ok("Recycle bin emptied successfully".to_string()),
            Ok(status) => Err(format!("Failed to empty recycle bin: exit code {}", status)),
            Err(e) => Err(format!("Failed to execute powershell: {}", e)),
        }
    }

    #[tool(description = "List all subkeys for a given registry hive and path")]
    fn list_registry_keys(&self, Parameters(RegistryRequest { hive, sub_key }): Parameters<RegistryRequest>) -> Result<Json<RegistryKeysResponse>, String> {
        let hive_predef = parse_hive(&hive)?;
        let key = RegKey::predef(hive_predef).open_subkey_with_flags(&sub_key, KEY_READ).map_err(|e| format!("Failed to open registry key: {}", e))?;
        
        let mut keys = Vec::new();
        for name in key.enum_keys().filter_map(|x| x.ok()) {
            keys.push(name);
        }
        Ok(Json(RegistryKeysResponse { keys }))
    }

    #[tool(description = "Get all values for a given registry hive and path as strings")]
    fn get_registry_values(&self, Parameters(RegistryRequest { hive, sub_key }): Parameters<RegistryRequest>) -> Result<Json<RegistryValuesResponse>, String> {
        let hive_predef = parse_hive(&hive)?;
        let key = RegKey::predef(hive_predef).open_subkey_with_flags(&sub_key, KEY_READ).map_err(|e| format!("Failed to open registry key: {}", e))?;
        
        let mut values = std::collections::HashMap::new();
        for (name, val) in key.enum_values().filter_map(|x| x.ok()) {
            let _val = val; // ignore unused var warning
            let val_str = match key.get_value::<String, _>(&name) {
                Ok(s) => s,
                Err(_) => match key.get_value::<u32, _>(&name) {
                    Ok(i) => i.to_string(),
                    Err(_) => match key.get_value::<u64, _>(&name) {
                        Ok(i) => i.to_string(),
                        Err(_) => "[Unsupported Type]".to_string()
                    }
                }
            };
            values.insert(if name.is_empty() { "(Default)".to_string() } else { name }, val_str);
        }
        Ok(Json(RegistryValuesResponse { values }))
    }

    #[tool(description = "Get all subkeys and their values for a given registry hive and path in one bulk operation")]
    fn get_registry_tree(&self, Parameters(RegistryRequest { hive, sub_key }): Parameters<RegistryRequest>) -> Result<Json<RegistryTreeResponse>, String> {
        let hive_predef = parse_hive(&hive)?;
        let parent_key = RegKey::predef(hive_predef)
            .open_subkey_with_flags(&sub_key, KEY_READ)
            .map_err(|e| format!("Failed to open registry key: {}", e))?;
        
        let mut keys = std::collections::HashMap::new();
        
        for subkey_name in parent_key.enum_keys().filter_map(|x| x.ok()) {
            if let Ok(child_key) = parent_key.open_subkey_with_flags(&subkey_name, KEY_READ) {
                let mut values = std::collections::HashMap::new();
                for (name, val) in child_key.enum_values().filter_map(|x| x.ok()) {
                    let _val = val; // ignore unused var warning
                    let val_str = match child_key.get_value::<String, _>(&name) {
                        Ok(s) => s,
                        Err(_) => match child_key.get_value::<u32, _>(&name) {
                            Ok(i) => i.to_string(),
                            Err(_) => match child_key.get_value::<u64, _>(&name) {
                                Ok(i) => i.to_string(),
                                Err(_) => "[Unsupported Type]".to_string()
                            }
                        }
                    };
                    values.insert(if name.is_empty() { "(Default)".to_string() } else { name }, val_str);
                }
                keys.insert(subkey_name, values);
            }
        }
        
        Ok(Json(RegistryTreeResponse { keys }))
    }

    #[tool(description = "Natively list all installed applications on the system by querying the registry internally")]
    fn list_installed_apps(&self) -> Result<Json<InstalledAppsResponse>, String> {
        let apps = fetch_installed_apps();
        Ok(Json(InstalledAppsResponse { apps }))
    }

    #[tool(description = "Scan AppData (Local and Roaming) for folders belonging to uninstalled applications")]
    fn scan_appdata_leftovers(&self) -> Result<Json<AppdataOrphansResponse>, String> {
        let apps = fetch_installed_apps();
        let mut known_keywords = std::collections::HashSet::new();
        
        let system_folders = vec!["microsoft", "windows", "temp", "programs", "packages", "intel", "amd", "nvidia", "google", "crashdumps", "d3dscache", "diagnostics", "connecteddevicesplatform"];
        for sys in system_folders {
            known_keywords.insert(sys.to_string());
        }

        for app in &apps {
            if !app.name.is_empty() {
                for part in app.name.split(|c: char| !c.is_alphanumeric()) {
                    if part.len() > 3 {
                        known_keywords.insert(part.to_lowercase());
                    }
                }
            }
            if !app.publisher.is_empty() {
                for part in app.publisher.split(|c: char| !c.is_alphanumeric()) {
                    if part.len() > 3 {
                        known_keywords.insert(part.to_lowercase());
                    }
                }
            }
        }
        
        let local = std::env::var("LOCALAPPDATA").unwrap_or_else(|_| "C:\\Users\\Default\\AppData\\Local".to_string());
        let roaming = std::env::var("APPDATA").unwrap_or_else(|_| "C:\\Users\\Default\\AppData\\Roaming".to_string());
        
        let mut orphans = Vec::new();
        
        for (path, loc_type) in [(local, "Local"), (roaming, "Roaming")] {
            if let Ok(entries) = std::fs::read_dir(&path) {
                for entry in entries.filter_map(|e| e.ok()) {
                    if entry.path().is_dir() {
                        let folder_name = entry.file_name().to_string_lossy().into_owned();
                        let lower_name = folder_name.to_lowercase();
                        
                        let mut is_known = false;
                        for kw in &known_keywords {
                            if lower_name.contains(kw) || kw.contains(&lower_name) {
                                is_known = true;
                                break;
                            }
                        }
                        
                        if !is_known {
                            let size_mb = utils::get_dir_size(&entry.path()) as f64 / 1_048_576.0;
                            if size_mb > 1.0 { // only report orphans larger than 1MB to avoid clutter
                                orphans.push(AppdataOrphan {
                                    folder_name,
                                    path: entry.path().to_string_lossy().into_owned(),
                                    size_mb,
                                    location_type: loc_type.to_string(),
                                });
                            }
                        }
                    }
                }
            }
        }
        
        orphans.sort_by(|a, b| b.size_mb.partial_cmp(&a.size_mb).unwrap_or(std::cmp::Ordering::Equal));
        Ok(Json(AppdataOrphansResponse { orphans }))
    }

    #[tool(description = "Delete a registry value under HKEY_CURRENT_USER or HKEY_LOCAL_MACHINE")]
    fn delete_registry_value(&self, Parameters(DeleteRegistryRequest { hive, sub_key, value_name }): Parameters<DeleteRegistryRequest>) -> Result<String, String> {
        let hive_predef = parse_hive(&hive)?;
        let key = RegKey::predef(hive_predef)
            .open_subkey_with_flags(&sub_key, KEY_WRITE)
            .map_err(|e| format!("Failed to open registry key: {}", e))?;

        key.delete_value(&value_name)
            .map_err(|e| format!("Failed to delete registry value '{}': {}", value_name, e))?;

        Ok(format!("Successfully deleted registry value '{}' under {}\\{}", value_name, hive, sub_key))
    }

    #[tool(description = "Get the total size of a directory recursively in MB")]
    fn get_directory_size(&self, Parameters(GetDirSizeRequest { path }): Parameters<GetDirSizeRequest>) -> Result<Json<GetDirSizeResponse>, String> {
        let p = std::path::Path::new(&path);
        if !p.exists() {
            return Err(format!("Path '{}' does not exist", path));
        }
        if !p.is_dir() {
            return Err(format!("Path '{}' is not a directory", path));
        }
        let size_mb = utils::get_dir_size(p) as f64 / 1_048_576.0;
        Ok(Json(GetDirSizeResponse { size_mb }))
    }

    #[tool(description = "Search for files in a directory matching specific criteria (extensions, minimum age, minimum size)")]
    fn find_files(&self, Parameters(FindFilesRequest { path, extensions, min_age_days, min_size_mb }): Parameters<FindFilesRequest>) -> Result<Json<FindFilesResponse>, String> {
        let p = std::path::Path::new(&path);
        if !p.exists() || !p.is_dir() {
            return Err(format!("Invalid path: '{}'", path));
        }

        let mut files = Vec::new();
        let now = std::time::SystemTime::now();
        let exts = extensions.unwrap_or_default().into_iter().map(|e| e.to_lowercase()).collect::<Vec<_>>();
        let min_mb = min_size_mb.unwrap_or(0.0);

        if let Ok(entries) = std::fs::read_dir(p) {
            for entry in entries.filter_map(|e| e.ok()) {
                let entry_path = entry.path();
                if entry_path.is_file() {
                    let ext = entry_path.extension().and_then(|s| s.to_str()).unwrap_or("").to_lowercase();
                    if !exts.is_empty() && !exts.contains(&ext) {
                        continue;
                    }

                    if let Ok(meta) = entry.metadata() {
                        let size_mb = meta.len() as f64 / 1_048_576.0;
                        if size_mb < min_mb {
                            continue;
                        }

                        let mut age_days = 0;
                        let mut last_modified_str = "Unknown".to_string();

                        if let Ok(modified) = meta.modified() {
                            let datetime: chrono::DateTime<chrono::Local> = modified.into();
                            last_modified_str = datetime.format("%Y-%m-%d %H:%M:%S").to_string();
                            if let Ok(age) = now.duration_since(modified) {
                                age_days = age.as_secs() / 86400;
                            }
                        }

                        if let Some(min_age) = min_age_days {
                            if age_days < min_age {
                                continue;
                            }
                        }

                        files.push(FoundFile {
                            name: entry.file_name().to_string_lossy().into_owned(),
                            path: entry_path.to_string_lossy().into_owned(),
                            size_mb,
                            last_modified: last_modified_str,
                            age_days,
                        });
                    }
                }
            }
        }

        files.sort_by(|a, b| b.size_mb.partial_cmp(&a.size_mb).unwrap_or(std::cmp::Ordering::Equal));
        Ok(Json(FindFilesResponse { files }))
    }
}

#[tool_handler]
impl ServerHandler for WinCleaner {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("A developer-centric and system-wide Windows cleanup MCP server providing basic diagnostic tools".into()),
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

    tracing::info!("Starting WinCleaner MCP server (Generic Mode)");

    let service = WinCleaner::new().serve(stdio()).await.inspect_err(|e| {
        tracing::error!("serving error: {:?}", e);
    })?;

    service.waiting().await?;
    Ok(())
}
