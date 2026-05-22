use std::collections::HashSet;
use std::path::Path;
use winreg::enums::{HKEY_CURRENT_USER, HKEY_LOCAL_MACHINE};
use winreg::RegKey;
use crate::utils::{expand_and_clean_path, extract_exec_path};

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

pub fn get_installed_apps() -> HashSet<String> {
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

pub fn get_program_dirs() -> HashSet<String> {
    let mut dirs = HashSet::new();
    for base in &["C:\\Program Files", "C:\\Program Files (x86)"] {
        let path = Path::new(base);
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

pub fn scan_registry_startup() -> Vec<RegistryStartupJunk> {
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
                        let p = Path::new(&clean_path);
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

pub fn scan_leftover_uninstall() -> Vec<RegistryUninstallJunk> {
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
                            let p = Path::new(&clean_loc);
                            if !p.exists() {
                                is_invalid = true;
                                invalid_path = clean_loc;
                            }
                        }
                    } else if let Ok(uninstall_str) = sub_key.get_value::<String, _>("UninstallString") {
                        let exec_path = extract_exec_path(&uninstall_str);
                        if !exec_path.is_empty() {
                            let clean_exec = expand_and_clean_path(&exec_path);
                            let p = Path::new(&clean_exec);
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
