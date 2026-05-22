# WinCleaner SCP - Windows Cleanup MCP Server

A robust, developer-centric, and system-wide Windows diagnostic and cleanup utility running as a Model Context Protocol (MCP) server. Built in Rust, it allows LLM agents to safely scan, analyze, and optimize Windows systems by finding and cleaning developer caches, temp folders, user junk, AppData leftovers, and registry remnants.

> [!WARNING]
> **Experimental & Potentially Dangerous**: This project is highly experimental and primarily designed to test the Model Context Protocol (MCP). Since it grants file deletion and registry modification privileges to LLM agents, it is **potentially very dangerous**. Use with extreme caution and at your own risk!

---


## Features

The server implements **7 high-level tools** exposed to MCP-compatible LLM clients:

### 1. Storage & Space Analysis
* **`get_disks`**: Lists all mounted drives on the system with total and available storage space.
* **`scan_directory`**: Recursively scans any directory to find "hotspots" (subdirectories or files larger than a specified limit, default: 100MB).
* **`inspect_directory`**: Scans a folder's immediate contents, lists them by size, and auto-detects developer signatures (e.g., `node_modules`, `.git`, Cargo `target`, Gradle `.gradle`, or Bazel outputs).

### 2. User Junk & Downloads Cleaning
* **`scan_user_junk`**: 
  * Calculates Recycle Bin size and file count.
  * Scans the `Downloads` directory for obsolete installers and archives (`.exe`, `.msi`, `.zip`, `.tar.gz`, `.7z`, etc.) older than 30 days.

### 3. Orphaned AppData Detection
* **`scan_appdata_leftovers`**:
  * Scans `AppData\Local` and `AppData\Roaming`.
  * Cross-references folder names with actually installed apps (from the Registry) and directories under `Program Files` / `Program Files (x86)`.
  * Identifies orphaned directories (last modified >15 days ago) from previously uninstalled software.

### 4. Windows Registry Cleanup
* **`scan_registry_junk`**:
  * **Broken Startup Items**: Scans `Run` and `RunOnce` keys (HKCU & HKLM) for automatic startup entries pointing to non-existent executable files.
  * **Orphaned Uninstall Entries**: Finds software registry keys in the `Uninstall` branch whose installation directories or uninstall commands no longer exist.
* **`delete_registry_value`**: Safely deletes a specific registry value under HKCU or HKLM to clean up the identified junk.

### 5. File System Deletion
* **`delete_path`**: Safely and recursively deletes a file or directory.

---

## Safety & Heuristics

To prevent accidental data loss in active developer environments, the server implements strict safety boundaries:
1. **Protected Folders**: `scan_appdata_leftovers` explicitly ignores critical systems and development directories, including:
   `microsoft`, `packages`, `google`, `mozilla`, `github`, `git`, `npm`, `yarn`, `rustup`, `cargo`, `.gemini`, `antigravity-ide`, `adobe`, `intel`, `nvidia`, `dropbox`, `onedrive`, `apple`, `spotify`, etc.
2. **Robust Registry Path Validation**:
   * **Environment Variables**: Dynamically expands registry path variables (like `%windir%`, `%SystemRoot%`, `%ProgramFiles%`) to correctly check if a file exists on disk, avoiding false positives.
   * **Quote Wrapping**: Strips surrounding double quotes from paths in registry keys before validation.

---

## Prompting Guide (How to interact with it)

LLM agents can use this MCP server autonomously when prompted with instructions like:

* *"Analyze my computer's storage space and list my hard drives."*
  * **Agent Flow**: Calls `get_disks` -> reports status -> calls `scan_user_junk` & `scan_appdata_leftovers` to pinpoint potential savings.
* *"Scan for leftovers of uninstalled applications."*
  * **Agent Flow**: Calls `scan_appdata_leftovers` and `scan_registry_junk` -> compiles a list of orphans in `AppData` and the registry.
* *"Clean up old downloaded setup files and empty the recycle bin."*
  * **Agent Flow**: Calls `scan_user_junk` -> lists candidates -> asks for confirmation -> calls `delete_path` on approved downloads.
* *"Inspect the folder `C:\Projects\my-app` and tell me what technology it uses."*
  * **Agent Flow**: Calls `inspect_directory` -> detects `.git` and `node_modules` -> replies: *"It is a Node.js project under Git control."*

---

## Build & Installation

### Prerequisites
* **Rust**: Ensure you have the Rust toolchain installed (edition 2024).
* **Windows OS**: This MCP server uses Windows-specific APIs (`winreg`, environment variables) and is designed only for Windows.

### Compilation
Clone the repository and build the binary:
```bash
cargo build --release
```
The compiled executable will be located at `target/release/mcp.exe`.

### Configuration in MCP Hosts
Add the server configuration to your MCP settings file (e.g., `mcp_config.json` for Claude Desktop or other assistant hosts):

```json
{
  "mcpServers": {
    "DiskAnalyzer": {
      "command": "C:\\path\\to\\your\\wincleaner_scp\\target\\release\\mcp.exe",
      "args": [],
      "env": {}
    }
  }
}
```

---

## License
This project is licensed under the GNU General Public License v3.0 - see the [LICENSE](LICENSE) file for details.

