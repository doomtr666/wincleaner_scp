# WinCleaner MCP - The Agentic Diagnostic Swiss Army Knife 🛠️

A robust, system-wide Windows diagnostic utility running as a Model Context Protocol (MCP) server. Built in Rust, it provides LLM agents with powerful, low-level primitives to safely scan, analyze, and optimize Windows systems.

Unlike traditional monolithic cleaners, **WinCleaner MCP** follows an agentic architecture: it provides fast, native C-like primitives (Registry reading, size calculation) and a few highly optimized analytical tools. The LLM Agent orchestrates these tools to perform complex, dynamic, and context-aware cleanups based on the user's intent.

> [!WARNING]
> **Experimental & Potentially Dangerous**: This project grants file deletion and registry modification privileges to LLM agents. Use with extreme caution and at your own risk!

---

## 🚀 The Tools (9 Available)

The server implements a hybrid set of low-level generic primitives and high-level analytical native tools:

### 📊 Native Analytical Tools (Fast & Compiled)
To avoid having the AI execute arbitrary slow Python or PowerShell scripts on the host, the MCP natively handles the most resource-intensive cross-referencing:
* **`list_installed_apps`**: Natively iterates through `HKLM` and `HKCU` to instantly return a clean JSON array of all installed applications, their publishers, and sizes (sorted).
* **`scan_appdata_leftovers`**: The Holy Grail of cleanup. It fetches the installed apps internally, scans `AppData\Local` and `AppData\Roaming`, and smartly cross-references the data to output a list of **orphaned directories** left behind by uninstalled software.

### 💾 File System Primitives
* **`get_directory_size`**: Instantly calculates the total recursive size of any directory.
* **`find_files`**: A lightning-fast recursive file search using Regex.
* **`delete_path`**: Safely and recursively deletes any file or directory.

### ⚙️ Windows Registry Primitives
* **`list_registry_keys`**: Lists the immediate child subkeys of a given path.
* **`get_registry_values`**: Reads and stringifies all values within a specific registry key.
* **`get_registry_tree`**: The "Bulk" primitive. Recursively fetches an entire registry tree (e.g., the whole `Uninstall` branch) in a single fast request to avoid RPC bottlenecks.
* **`delete_registry_value`**: Deletes a specific registry value.

---

## 🤖 Prompting Guide (How to interact with it)

LLM agents can use this MCP server autonomously to perform context-aware operations:

* *"Can you check what's taking up so much space in my AppData?"*
  * **Agent Flow**: Calls `scan_appdata_leftovers` -> identifies 8GB of dead dev caches (NuGet, npm) and old game configs -> asks user for confirmation -> calls `delete_path` concurrently.
* *"I want to uninstall Dungeondraft and Wonderdraft silently."*
  * **Agent Flow**: Calls `get_registry_tree` or `get_registry_values` -> extracts the `QuietUninstallString` -> executes it natively via the host's shell.
* *"Find all `.log` files in `C:\Temp` and delete the ones larger than 10MB."*
  * **Agent Flow**: Calls `find_files` -> iterates with `get_directory_size` -> deletes matching targets.

---

## 🛠️ Build & Installation

### Prerequisites
* **Rust**: Ensure you have the Rust toolchain installed.
* **Windows OS**: This MCP server uses Windows-specific APIs (`winreg`) and is designed only for Windows.

### Compilation
Clone the repository and build the binary:
```bash
cargo build --release
```
The compiled executable will be located at `target/release/wincleaner_mcp.exe`.

### Configuration in MCP Hosts
Add the server configuration to your MCP settings file (e.g., `mcp_config.json` for Claude Desktop or Gemini Antigravity IDE):

```json
{
  "mcpServers": {
    "wincleaner": {
      "command": "C:\\path\\to\\your\\wincleaner_mcp\\target\\release\\wincleaner_mcp.exe",
      "args": [],
      "env": {}
    }
  }
}
```

---

## License
This project is licensed under the GNU General Public License v3.0 - see the [LICENSE](LICENSE) file for details.

