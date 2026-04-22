use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Computer-use sidecar configuration for browser automation.
///
/// Uses a manual [`Debug`] impl that redacts `api_key` to prevent
/// credential leakage in log output.
#[derive(Clone, Serialize, Deserialize, JsonSchema)]
pub struct BrowserComputerUseConfig {
    /// Enable the general-purpose `computer_use` tool (launch/focus apps,
    /// screenshot, click, type). Provider-agnostic — works with any LLM.
    #[serde(default)]
    pub enabled: bool,
    /// Auto-start the sidecar on first tool use when `/health` is unreachable.
    #[serde(default = "default_true_bool")]
    pub auto_start: bool,
    /// Allowlist of binary names or paths the tool may launch via `app_launch`.
    /// Empty = permit-all at the TopClaw layer (sidecar still enforces its own
    /// policy).
    #[serde(default)]
    pub app_allowlist: Vec<String>,
    /// Sidecar endpoint URL
    #[serde(default = "default_computer_use_endpoint")]
    pub endpoint: String,
    /// Optional API key for the sidecar
    #[serde(default)]
    pub api_key: Option<String>,
    /// Request timeout in milliseconds
    #[serde(default = "default_computer_use_timeout_ms")]
    pub timeout_ms: u64,
    /// Allow non-localhost sidecar endpoints
    #[serde(default)]
    pub allow_remote_endpoint: bool,
    /// Window title allowlist for computer-use actions
    #[serde(default)]
    pub window_allowlist: Vec<String>,
    /// Maximum X coordinate for computer-use actions
    #[serde(default)]
    pub max_coordinate_x: Option<i64>,
    /// Maximum Y coordinate for computer-use actions
    #[serde(default)]
    pub max_coordinate_y: Option<i64>,
}

const fn default_true_bool() -> bool {
    true
}

fn default_computer_use_endpoint() -> String {
    "http://127.0.0.1:8787/v1/actions".into()
}

const fn default_computer_use_timeout_ms() -> u64 {
    15_000
}

impl std::fmt::Debug for BrowserComputerUseConfig {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("BrowserComputerUseConfig")
            .field("enabled", &self.enabled)
            .field("auto_start", &self.auto_start)
            .field("app_allowlist", &self.app_allowlist)
            .field("endpoint", &self.endpoint)
            // api_key deliberately omitted to prevent credential leakage
            .field("timeout_ms", &self.timeout_ms)
            .field("allow_remote_endpoint", &self.allow_remote_endpoint)
            .field("window_allowlist", &self.window_allowlist)
            .field("max_coordinate_x", &self.max_coordinate_x)
            .field("max_coordinate_y", &self.max_coordinate_y)
            .finish_non_exhaustive()
    }
}

impl BrowserComputerUseConfig {
    /// Check whether the configured sidecar endpoint points to localhost.
    pub fn endpoint_is_local(&self) -> bool {
        match reqwest::Url::parse(&self.endpoint) {
            Ok(u) => matches!(u.host_str(), Some("127.0.0.1" | "localhost" | "::1")),
            Err(_) => false,
        }
    }
}

impl Default for BrowserComputerUseConfig {
    fn default() -> Self {
        Self {
            // Enabled by default so a fresh TopClaw install can handle
            // "open $app" / "click" / "take a screenshot" requests without
            // asking the user to edit config.toml. Actions still flow
            // through the normal approval + autonomy gates.
            enabled: true,
            auto_start: true,
            app_allowlist: Vec::new(),
            endpoint: default_computer_use_endpoint(),
            api_key: None,
            timeout_ms: default_computer_use_timeout_ms(),
            allow_remote_endpoint: false,
            window_allowlist: Vec::new(),
            max_coordinate_x: None,
            max_coordinate_y: None,
        }
    }
}

/// Browser automation configuration (`[browser]` section).
///
/// Controls the `browser_open` tool and browser automation backends.
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct BrowserConfig {
    /// Enable `browser_open` tool (opens URLs in the system browser without scraping)
    #[serde(default)]
    pub enabled: bool,
    /// Allowed domains for `browser_open` (exact or subdomain match)
    #[serde(default)]
    pub allowed_domains: Vec<String>,
    /// Browser for `browser_open` tool: "disable" | "brave" | "chrome" | "firefox" | "default"
    #[serde(default = "default_browser_open")]
    pub browser_open: String,
    /// Browser session name (for agent-browser automation)
    #[serde(default)]
    pub session_name: Option<String>,
    /// Browser automation backend: "agent_browser" | "rust_native" | "computer_use" | "auto"
    #[serde(default = "default_browser_backend")]
    pub backend: String,
    /// Headless mode for rust-native backend
    #[serde(default = "default_true")]
    pub native_headless: bool,
    /// WebDriver endpoint URL for rust-native backend (e.g. http://127.0.0.1:9515)
    #[serde(default = "default_browser_webdriver_url")]
    pub native_webdriver_url: String,
    /// Optional Chrome/Chromium executable path for rust-native backend
    #[serde(default)]
    pub native_chrome_path: Option<String>,
    /// Computer-use sidecar configuration
    #[serde(default)]
    pub computer_use: BrowserComputerUseConfig,
}

const fn default_true() -> bool {
    true
}

fn default_browser_backend() -> String {
    "agent_browser".into()
}

fn default_browser_open() -> String {
    "default".into()
}

fn default_browser_webdriver_url() -> String {
    "http://127.0.0.1:9515".into()
}

impl Default for BrowserConfig {
    fn default() -> Self {
        Self {
            enabled: false,
            allowed_domains: Vec::new(),
            browser_open: default_browser_open(),
            session_name: None,
            backend: default_browser_backend(),
            native_headless: default_true(),
            native_webdriver_url: default_browser_webdriver_url(),
            native_chrome_path: None,
            computer_use: BrowserComputerUseConfig::default(),
        }
    }
}
