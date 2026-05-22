mod utils;
mod registry;
mod appdata;

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
use winreg::enums::KEY_WRITE;
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
        let (recycle_size, recycle_count) = utils::get_recycle_bin_stats();
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
    fn scan_appdata_leftovers(&self) -> Result<Json<appdata::AppDataLeftoversResponse>, String> {
        let leftovers = appdata::get_appdata_leftovers();
        Ok(Json(appdata::AppDataLeftoversResponse { leftovers }))
    }

    #[tool(description = "Scan the registry for startup entries pointing to non-existent files and leftover uninstall entries")]
    fn scan_registry_junk(&self) -> Result<Json<registry::RegistryJunkResponse>, String> {
        let broken_startup_items = registry::scan_registry_startup();
        let leftover_uninstall_keys = registry::scan_leftover_uninstall();
        Ok(Json(registry::RegistryJunkResponse {
            broken_startup_items,
            leftover_uninstall_keys,
        }))
    }

    #[tool(description = "Delete a registry value under HKEY_CURRENT_USER or HKEY_LOCAL_MACHINE")]
    fn delete_registry_value(&self, Parameters(registry::DeleteRegistryRequest { hive, sub_key, value_name }): Parameters<registry::DeleteRegistryRequest>) -> Result<String, String> {
        let hive_predef = match hive.as_str() {
            "HKEY_CURRENT_USER" => winreg::enums::HKEY_CURRENT_USER,
            "HKEY_LOCAL_MACHINE" => winreg::enums::HKEY_LOCAL_MACHINE,
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
impl ServerHandler for WinCleaner {
    fn get_info(&self) -> ServerInfo {
        ServerInfo {
            instructions: Some("A developer-centric and system-wide Windows cleanup MCP server".into()),
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

    tracing::info!("Starting WinCleaner MCP server");

    let service = WinCleaner::new().serve(stdio()).await.inspect_err(|e| {
        tracing::error!("serving error: {:?}", e);
    })?;

    service.waiting().await?;
    Ok(())
}
