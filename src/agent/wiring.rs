use crate::config::{Config, EmbeddingRouteConfig};
use crate::memory::{self, Memory};
use crate::observability::{self, Observer};
use crate::runtime::{self, RuntimeAdapter};
use crate::security::SecurityPolicy;
use crate::tools::{self, Tool};
use anyhow::Result;
use std::sync::Arc;

pub(crate) struct ExecutionSupport {
    #[allow(dead_code)]
    pub runtime: Arc<dyn RuntimeAdapter>,
    #[allow(dead_code)]
    pub security: Arc<SecurityPolicy>,
    pub memory: Arc<dyn Memory>,
    pub tools: Vec<Box<dyn Tool>>,
}

pub(crate) fn build_observer(config: &Config) -> Arc<dyn Observer> {
    Arc::from(observability::create_observer(&config.observability))
}

pub(crate) fn build_execution_support(
    config: &Config,
    embedding_routes: &[EmbeddingRouteConfig],
) -> Result<ExecutionSupport> {
    build_execution_support_with_tool_profile(config, embedding_routes, false)
}

pub(crate) fn build_channel_execution_support(
    config: &Config,
    embedding_routes: &[EmbeddingRouteConfig],
) -> Result<ExecutionSupport> {
    build_execution_support_with_tool_profile(config, embedding_routes, true)
}

fn build_execution_support_with_tool_profile(
    config: &Config,
    embedding_routes: &[EmbeddingRouteConfig],
    channel_focused_tools: bool,
) -> Result<ExecutionSupport> {
    let runtime: Arc<dyn RuntimeAdapter> = Arc::from(runtime::create_runtime(&config.runtime)?);
    let security = Arc::new(SecurityPolicy::from_runtime_config(config)?);

    memory::prepare_memory_workspace(
        &config.memory,
        Some(&config.storage.provider.config),
        &config.workspace_dir,
    )?;
    let memory: Arc<dyn Memory> = Arc::from(memory::create_memory_backend_with_storage_and_routes(
        &config.memory,
        embedding_routes,
        Some(&config.storage.provider.config),
        &config.workspace_dir,
        config.api_key.as_deref(),
    )?);

    let (composio_key, composio_entity_id) = composio_context(config);
    let tools = if channel_focused_tools {
        tools::channel_tools_with_runtime(
            Arc::new(config.clone()),
            &security,
            runtime.clone(),
            memory.clone(),
            composio_key,
            composio_entity_id,
            &config.browser,
            &config.http_request,
            &config.web_fetch,
            &config.workspace_dir,
            &config.agents,
            config.api_key.as_deref(),
            config,
        )
    } else {
        tools::all_tools_with_runtime(
            Arc::new(config.clone()),
            &security,
            runtime.clone(),
            memory.clone(),
            composio_key,
            composio_entity_id,
            &config.browser,
            &config.http_request,
            &config.web_fetch,
            &config.workspace_dir,
            &config.agents,
            config.api_key.as_deref(),
            config,
        )
    };

    Ok(ExecutionSupport {
        runtime,
        security,
        memory,
        tools,
    })
}

pub(crate) fn load_skills(config: &Config) -> Vec<crate::skills::Skill> {
    crate::skills::load_skills_with_config(&config.workspace_dir, config)
}

fn composio_context(config: &Config) -> (Option<&str>, Option<&str>) {
    if config.composio.enabled {
        (
            config.composio.api_key.as_deref(),
            Some(config.composio.entity_id.as_str()),
        )
    } else {
        (None, None)
    }
}
