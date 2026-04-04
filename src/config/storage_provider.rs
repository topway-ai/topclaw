use schemars::JsonSchema;
use serde::{Deserialize, Serialize};

/// Wrapper for the storage provider configuration section.
#[derive(Debug, Clone, Serialize, Deserialize, Default, JsonSchema)]
pub struct StorageProviderSection {
    /// Storage provider backend settings.
    #[serde(default)]
    pub config: StorageProviderConfig,
}

/// Storage provider backend configuration (e.g. connection details for remote storage).
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct StorageProviderConfig {
    /// Storage engine key (e.g. "postgres", "mariadb", "sqlite").
    #[serde(default)]
    pub provider: String,

    /// Connection URL for remote providers.
    #[serde(default)]
    pub db_url: Option<String>,

    /// Database schema for SQL backends.
    #[serde(default = "default_storage_schema")]
    pub schema: String,

    /// Table name for memory entries.
    #[serde(default = "default_storage_table")]
    pub table: String,

    /// Optional connection timeout in seconds for remote providers.
    #[serde(default)]
    pub connect_timeout_secs: Option<u64>,

    /// Enable TLS for SQL remote connections.
    ///
    /// `true` — request TLS from the backend (and for PostgreSQL skips certificate
    /// verification; suitable for self-signed certs and many managed databases).
    /// `false` (default) — plain TCP, backward-compatible.
    #[serde(default)]
    pub tls: bool,
}

fn default_storage_schema() -> String {
    "public".into()
}

fn default_storage_table() -> String {
    "memories".into()
}

impl Default for StorageProviderConfig {
    fn default() -> Self {
        Self {
            provider: String::new(),
            db_url: None,
            schema: default_storage_schema(),
            table: default_storage_table(),
            connect_timeout_secs: None,
            tls: false,
        }
    }
}
