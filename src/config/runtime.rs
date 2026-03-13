use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

const fn default_true() -> bool {
    true
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct RuntimeConfig {
    #[serde(default = "default_runtime_kind")]
    pub kind: String,
    #[serde(default)]
    pub docker: DockerRuntimeConfig,
    #[serde(default)]
    pub wasm: WasmRuntimeConfig,
    #[serde(default)]
    pub reasoning_enabled: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct DockerRuntimeConfig {
    #[serde(default = "default_docker_image")]
    pub image: String,
    #[serde(default = "default_docker_network")]
    pub network: String,
    #[serde(default = "default_docker_memory_limit_mb")]
    pub memory_limit_mb: Option<u64>,
    #[serde(default = "default_docker_cpu_limit")]
    pub cpu_limit: Option<f64>,
    #[serde(default = "default_true")]
    pub read_only_rootfs: bool,
    #[serde(default = "default_true")]
    pub mount_workspace: bool,
    #[serde(default)]
    pub allowed_workspace_roots: Vec<String>,
}

#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WasmRuntimeConfig {
    #[serde(default = "default_wasm_tools_dir")]
    pub tools_dir: String,
    #[serde(default = "default_wasm_fuel_limit")]
    pub fuel_limit: u64,
    #[serde(default = "default_wasm_memory_limit_mb")]
    pub memory_limit_mb: u64,
    #[serde(default = "default_wasm_max_module_size_mb")]
    pub max_module_size_mb: u64,
    #[serde(default)]
    pub allow_workspace_read: bool,
    #[serde(default)]
    pub allow_workspace_write: bool,
    #[serde(default)]
    pub allowed_hosts: Vec<String>,
    #[serde(default)]
    pub security: WasmSecurityConfig,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WasmCapabilityEscalationMode {
    #[default]
    Deny,
    Clamp,
}

#[derive(Debug, Clone, Copy, Default, Serialize, Deserialize, JsonSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum WasmModuleHashPolicy {
    Disabled,
    #[default]
    Warn,
    Enforce,
}

#[allow(clippy::struct_excessive_bools)]
#[derive(Debug, Clone, Serialize, Deserialize, JsonSchema)]
pub struct WasmSecurityConfig {
    #[serde(default = "default_true")]
    pub require_workspace_relative_tools_dir: bool,
    #[serde(default = "default_true")]
    pub reject_symlink_modules: bool,
    #[serde(default = "default_true")]
    pub reject_symlink_tools_dir: bool,
    #[serde(default = "default_true")]
    pub strict_host_validation: bool,
    #[serde(default)]
    pub capability_escalation_mode: WasmCapabilityEscalationMode,
    #[serde(default)]
    pub module_hash_policy: WasmModuleHashPolicy,
    #[serde(default)]
    pub module_sha256: BTreeMap<String, String>,
}

fn default_runtime_kind() -> String {
    "native".into()
}

fn default_docker_image() -> String {
    "alpine:3.20".into()
}

fn default_docker_network() -> String {
    "none".into()
}

fn default_docker_memory_limit_mb() -> Option<u64> {
    Some(512)
}

fn default_docker_cpu_limit() -> Option<f64> {
    Some(1.0)
}

fn default_wasm_tools_dir() -> String {
    "tools/wasm".into()
}

fn default_wasm_fuel_limit() -> u64 {
    1_000_000
}

fn default_wasm_memory_limit_mb() -> u64 {
    64
}

fn default_wasm_max_module_size_mb() -> u64 {
    50
}

impl Default for DockerRuntimeConfig {
    fn default() -> Self {
        Self {
            image: default_docker_image(),
            network: default_docker_network(),
            memory_limit_mb: default_docker_memory_limit_mb(),
            cpu_limit: default_docker_cpu_limit(),
            read_only_rootfs: true,
            mount_workspace: true,
            allowed_workspace_roots: Vec::new(),
        }
    }
}

impl Default for WasmRuntimeConfig {
    fn default() -> Self {
        Self {
            tools_dir: default_wasm_tools_dir(),
            fuel_limit: default_wasm_fuel_limit(),
            memory_limit_mb: default_wasm_memory_limit_mb(),
            max_module_size_mb: default_wasm_max_module_size_mb(),
            allow_workspace_read: false,
            allow_workspace_write: false,
            allowed_hosts: Vec::new(),
            security: WasmSecurityConfig::default(),
        }
    }
}

impl Default for WasmSecurityConfig {
    fn default() -> Self {
        Self {
            require_workspace_relative_tools_dir: true,
            reject_symlink_modules: true,
            reject_symlink_tools_dir: true,
            strict_host_validation: true,
            capability_escalation_mode: WasmCapabilityEscalationMode::Deny,
            module_hash_policy: WasmModuleHashPolicy::Warn,
            module_sha256: BTreeMap::new(),
        }
    }
}

impl Default for RuntimeConfig {
    fn default() -> Self {
        Self {
            kind: default_runtime_kind(),
            docker: DockerRuntimeConfig::default(),
            wasm: WasmRuntimeConfig::default(),
            reasoning_enabled: None,
        }
    }
}
