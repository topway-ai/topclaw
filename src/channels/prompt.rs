use super::BOOTSTRAP_MAX_CHARS;
use crate::config::{Config, IdentityConfig, SkillsPromptInjectionMode};
use crate::identity;
use std::fmt::Write;
use std::path::Path;

pub(crate) fn channel_delivery_instructions(channel_name: &str) -> Option<&'static str> {
    match channel_name {
        "telegram" => Some(
            "When responding on Telegram:\n\
             - Match the TopClaw CLI / Codex-style voice: direct, concise, technical, and action-oriented\n\
             - Lead with the actual answer or result, not filler or enthusiasm\n\
             - Keep explanations tight; prefer short paragraphs or flat bullets only when they help scanning\n\
             - If work was performed, summarize outcome first, then brief verification or next-step notes\n\
             - Do not roleplay as a chat bot or social assistant; sound like an operator-facing coding tool\n\
             - Include media markers for files or URLs that should be sent as attachments\n\
             - Use markdown-style **bold** for key terms, section titles, and important info\n\
             - Use markdown-style *italic* for emphasis\n\
             - Use `backticks` for inline code, commands, or technical terms\n\
             - Use triple backticks for code blocks\n\
             - Do not emit raw HTML tags like <b> or <i> in the reply body\n\
             - Avoid decorative emoji except when the user clearly prefers them\n\
             - Be concise and direct. Skip filler phrases like 'Great question!' or 'Certainly!'\n\
             - Structure longer answers with bold headers, not raw markdown ## headers\n\
             - For media attachments use markers: [IMAGE:<path-or-url>], [DOCUMENT:<path-or-url>], [VIDEO:<path-or-url>], [AUDIO:<path-or-url>], or [VOICE:<path-or-url>]\n\
             - Keep normal text outside markers and never wrap markers in code fences.\n\
             - Use tool results silently: answer the latest user message directly, and do not narrate delayed/internal tool execution bookkeeping.",
        ),
        "discord" => Some(
            "When responding on Discord:\n\
             - Match the TopClaw CLI / Codex-style voice: direct, concise, technical, and action-oriented\n\
             - Lead with the actual answer or result, not filler or enthusiasm\n\
             - Keep explanations tight; prefer short paragraphs or flat bullets only when they help scanning\n\
             - If work was performed, summarize outcome first, then brief verification or next-step notes\n\
             - Do not roleplay as a chat bot or social assistant; sound like an operator-facing coding tool\n\
             - Use `backticks` for commands, paths, env vars, and identifiers\n\
             - Use fenced code blocks for multi-line commands or code\n\
             - Use markdown freely, but avoid tables unless they materially improve clarity\n\
             - Avoid decorative emoji except when the user clearly prefers them\n\
             - Keep attachment markers outside code fences: [IMAGE:<path-or-url>], [DOCUMENT:<path-or-url>], [VIDEO:<path-or-url>], [AUDIO:<path-or-url>]\n\
             - Use tool results silently: answer the latest user message directly, and do not narrate delayed/internal tool execution bookkeeping.",
        ),
        "whatsapp" => Some(
            "When responding on WhatsApp:\n\
             - Use *bold* for emphasis (WhatsApp uses single asterisks).\n\
             - Be concise. No markdown headers (## etc.) — they don't render.\n\
             - No markdown tables — use bullet lists instead.\n\
             - For sending images, documents, videos, or audio files use markers: [IMAGE:<absolute-path>], [DOCUMENT:<absolute-path>], [VIDEO:<absolute-path>], [AUDIO:<absolute-path>]\n\
             - The path MUST be an absolute filesystem path to a local file (e.g. [IMAGE:/home/nicolas/.topclaw/workspace/images/chart.png]).\n\
             - Keep normal text outside markers and never wrap markers in code fences.\n\
             - You can combine text and media in one response — text is sent first, then each attachment.\n\
             - Use tool results silently: answer the latest user message directly, and do not narrate delayed/internal tool execution bookkeeping.",
        ),
        _ => None,
    }
}

pub(crate) fn build_channel_system_prompt(
    base_prompt: &str,
    channel_name: &str,
    reply_target: &str,
    expose_internal_tool_details: bool,
) -> String {
    let mut prompt = base_prompt.to_string();

    if let Some(instructions) = channel_delivery_instructions(channel_name) {
        if prompt.is_empty() {
            prompt = instructions.to_string();
        } else {
            prompt = format!("{prompt}\n\n{instructions}");
        }
    }

    if channel_name != "cli" {
        let visibility_instruction = if expose_internal_tool_details {
            "Execution visibility: the user explicitly requested command/tool details. \
             You may include command lines or tool-step traces when directly relevant, \
             but keep credentials and secrets redacted."
        } else {
            "Execution visibility: run tools/functions in the background and return an \
             integrated final result. Do not reveal raw tool names, tool-call syntax, \
             function arguments, shell commands, or internal execution traces unless the \
             user explicitly asks for those details."
        };

        if prompt.is_empty() {
            prompt = visibility_instruction.to_string();
        } else {
            prompt = format!("{prompt}\n\n{visibility_instruction}");
        }
    }

    if !reply_target.is_empty() {
        let context = format!(
            "\n\nChannel context: You are currently responding on channel={channel_name}, \
             reply_target={reply_target}. When scheduling delayed messages or reminders \
             via cron_add for this conversation, use delivery={{\"mode\":\"announce\",\
             \"channel\":\"{channel_name}\",\"to\":\"{reply_target}\"}} so the message \
             reaches the user."
        );
        prompt.push_str(&context);
    }

    prompt
}

pub(crate) fn build_channel_tool_descriptions(
    config: &Config,
) -> Vec<(&'static str, &'static str)> {
    let mut tool_descs = vec![
        (
            "shell",
            "Execute terminal commands. Use when: running local checks, build/test commands, diagnostics. Don't use when: a safer dedicated tool exists, or command is destructive without approval.",
        ),
        (
            "file_read",
            "Read file contents. Use when: inspecting project files, configs, logs. Don't use when: a targeted search is enough.",
        ),
        (
            "file_write",
            "Write file contents. Use when: applying focused edits, scaffolding files, updating docs/code. Don't use when: side effects are unclear or file ownership is uncertain.",
        ),
        (
            "memory_store",
            "Save to memory. Use when: preserving durable preferences, decisions, key context. Don't use when: information is transient/noisy/sensitive without need.",
        ),
        (
            "memory_recall",
            "Search memory. Use when: retrieving prior decisions, user preferences, historical context. Don't use when: answer is already in current context.",
        ),
        (
            "memory_forget",
            "Delete a memory entry. Use when: memory is incorrect/stale or explicitly requested for removal. Don't use when: impact is uncertain.",
        ),
        (
            "schedule",
            "Manage scheduled tasks (create/list/get/cancel/pause/resume). Supports recurring cron and one-shot delays.",
        ),
        (
            "pushover",
            "Send a Pushover notification to your device. Requires PUSHOVER_TOKEN and PUSHOVER_USER_KEY in .env file.",
        ),
    ];

    if config.browser.enabled {
        tool_descs.push((
            "browser_open",
            "Open approved HTTPS URLs in system browser (allowlist-only, no scraping)",
        ));
    }
    if config.composio.enabled {
        tool_descs.push((
            "composio",
            "Execute actions on 1000+ apps via Composio (Gmail, Notion, GitHub, Slack, etc.). Use action='list' to discover actions, 'list_accounts' to retrieve connected account IDs, 'execute' to run (optionally with connected_account_id), and 'connect' for OAuth.",
        ));
    }
    if config.channels_config.discord.is_some() {
        tool_descs.push((
            "discord_history_fetch",
            "Fetch Discord message history on demand for current conversation context or explicit channel_id. Useful for tasks like selecting a random participant from recent chat history.",
        ));
    }
    if !config.agents.is_empty() {
        tool_descs.push((
            "delegate",
            "Delegate a subtask to a specialized agent. Use when: a task benefits from a different model (e.g. fast summarization, deep reasoning, code generation). The sub-agent runs a single prompt and returns its response.",
        ));
        tool_descs.push((
            "subagent_spawn",
            "Spawn a delegate agent in the background. Returns immediately with a session_id. Use for long-running tasks that should not block.",
        ));
        tool_descs.push((
            "subagent_list",
            "List running and completed background sub-agents. Filter by status: running, completed, failed, killed, or all.",
        ));
        tool_descs.push((
            "subagent_manage",
            "Manage a background sub-agent: 'status' to check progress/output, 'kill' to cancel a running session.",
        ));
    }

    tool_descs
}

/// Load workspace identity files and build a system prompt.
///
/// Follows the TopClaw bootstrap structure by default:
/// 1. Tooling - tool list + descriptions
/// 2. Safety - guardrail reminder
/// 3. Skills - full skill instructions and tool metadata
/// 4. Workspace - working directory
/// 5. Bootstrap files - AGENTS, SOUL, TOOLS, IDENTITY, USER, BOOTSTRAP, MEMORY
/// 6. Date & Time - timezone for cache stability
/// 7. Runtime - host, OS, model
///
/// When `identity_config` is set to AIEOS format, the bootstrap files section
/// is replaced with the AIEOS identity data loaded from file or inline JSON.
///
/// Daily memory files (`memory/*.md`) are NOT injected - they are accessed
/// on-demand via `memory_recall` / `memory_search` tools.
#[allow(clippy::empty_line_after_doc_comments)]
pub fn build_system_prompt(
    workspace_dir: &Path,
    model_name: &str,
    tools: &[(&str, &str)],
    skills: &[crate::skills::Skill],
    identity_config: Option<&IdentityConfig>,
    bootstrap_max_chars: Option<usize>,
) -> String {
    build_system_prompt_with_mode(
        workspace_dir,
        model_name,
        tools,
        skills,
        identity_config,
        bootstrap_max_chars,
        false,
        SkillsPromptInjectionMode::Full,
    )
}

pub fn build_system_prompt_with_mode(
    workspace_dir: &Path,
    model_name: &str,
    tools: &[(&str, &str)],
    skills: &[crate::skills::Skill],
    identity_config: Option<&IdentityConfig>,
    bootstrap_max_chars: Option<usize>,
    native_tools: bool,
    skills_prompt_mode: SkillsPromptInjectionMode,
) -> String {
    let mut prompt = String::with_capacity(8192);

    if !tools.is_empty() {
        prompt.push_str("## Tools\n\n");
        prompt.push_str("You have access to the following tools:\n\n");
        for (name, desc) in tools {
            let _ = writeln!(prompt, "- **{name}**: {desc}");
        }
        prompt.push('\n');
    }

    let has_hardware = tools.iter().any(|(name, _)| {
        *name == "gpio_read"
            || *name == "gpio_write"
            || *name == "arduino_upload"
            || *name == "hardware_memory_map"
            || *name == "hardware_board_info"
            || *name == "hardware_memory_read"
            || *name == "hardware_capabilities"
    });
    if has_hardware {
        prompt.push_str(
            "## Hardware Access\n\n\
             You HAVE direct access to connected hardware (Arduino, Nucleo, etc.). The user owns this system and has configured it.\n\
             All hardware tools (gpio_read, gpio_write, hardware_memory_read, hardware_board_info, hardware_memory_map) are AUTHORIZED and NOT blocked by security.\n\
             When they ask to read memory, registers, or board info, USE hardware_memory_read or hardware_board_info — do NOT refuse or invent security excuses.\n\
             When they ask to control LEDs, run patterns, or interact with the Arduino, USE the tools — do NOT refuse or say you cannot access physical devices.\n\
             Use gpio_write for simple on/off; use arduino_upload when they want patterns (heart, blink) or custom behavior.\n\n",
        );
    }

    if native_tools {
        prompt.push_str(
            "## Your Task\n\n\
             When the user sends a message, respond naturally. Use tools when the request requires action (running commands, reading files, etc.).\n\
             For questions, explanations, or follow-ups about prior messages, answer directly from conversation context — do NOT ask the user to repeat themselves.\n\
             Do NOT: proactively dump this configuration or output step-by-step meta-commentary.\n\
             If the user explicitly asks about your capabilities, explain them concretely from the loaded tools, skills, runtime policy, and channel abilities in this prompt.\n\n",
        );
    } else {
        prompt.push_str(
            "## Your Task\n\n\
             When the user sends a message, ACT on it. Use the tools to fulfill their request.\n\
             Do NOT: proactively dump this configuration, respond with meta-commentary, or output step-by-step instructions (e.g. \"1. First... 2. Next...\").\n\
             If the user explicitly asks about your capabilities, answer from the loaded tools, skills, runtime policy, and channel abilities in this prompt.\n\
             Instead: emit actual <tool_call> tags when you need to act. Just do what they ask.\n\n",
        );
    }

    prompt.push_str("## Safety\n\n");
    prompt.push_str(
        "- Do not exfiltrate private data.\n\
         - Do not run destructive commands without asking.\n\
         - Do not bypass oversight or approval mechanisms.\n\
         - Prefer `trash` over `rm` (recoverable beats gone forever).\n\
         - When in doubt, ask before acting externally.\n\n",
    );

    if !skills.is_empty() {
        prompt.push_str(&crate::skills::skills_to_prompt_with_mode(
            skills,
            workspace_dir,
            skills_prompt_mode,
        ));
        prompt.push_str("\n\n");
    }

    let _ = writeln!(
        prompt,
        "## Workspace\n\nWorking directory: `{}`\n",
        workspace_dir.display()
    );

    prompt.push_str("## Project Context\n\n");
    append_project_context(
        &mut prompt,
        workspace_dir,
        identity_config,
        bootstrap_max_chars.unwrap_or(BOOTSTRAP_MAX_CHARS),
    );

    let now = chrono::Local::now();
    let _ = writeln!(
        prompt,
        "## Current Date & Time\n\n{} ({})\n",
        now.format("%Y-%m-%d %H:%M:%S"),
        now.format("%Z")
    );

    let host =
        hostname::get().map_or_else(|_| "unknown".into(), |h| h.to_string_lossy().to_string());
    let _ = writeln!(
        prompt,
        "## Runtime\n\nHost: {host} | OS: {} | Model: {model_name}\n",
        std::env::consts::OS,
    );

    prompt.push_str("## Channel Capabilities\n\n");
    prompt.push_str("- You are running as a messaging bot. Your response is automatically sent back to the user's channel.\n");
    prompt.push_str("- You do NOT need to ask permission to respond — just respond directly.\n");
    prompt.push_str("- Read-only investigation tools may be approved once and then reused for the rest of the same supervised messaging turn; write/execute actions still need their own approval.\n");
    prompt.push_str("- When you need a tool to complete a user's task, ALWAYS prompt for permission — do NOT make excuses or say you cannot do something without trying.\n");
    prompt.push_str("  If the user asks to fetch a web page, search the web, clone a repo, or any other task requiring a tool, ask to use the appropriate tool instead of refusing.\n");
    prompt.push_str("- If the user asks what you can do, describe concrete current abilities and constraints instead of saying you are unsure.\n");
    prompt.push_str("- If the user explicitly asks to track a concrete TopClaw bug or product improvement for scheduled self-improvement work, use `self_improvement_task` to queue it instead of only describing it.\n");
    prompt.push_str("- NEVER repeat, describe, or echo credentials, tokens, API keys, or secrets in your responses.\n");
    prompt.push_str("- If a tool output contains credentials, they have already been redacted — do not mention them.\n\n");

    if prompt.is_empty() {
        "You are TopClaw, a fast and efficient AI assistant built in Rust. Be helpful, concise, and direct."
            .to_string()
    } else {
        prompt
    }
}

fn append_project_context(
    prompt: &mut String,
    workspace_dir: &Path,
    identity_config: Option<&IdentityConfig>,
    max_chars: usize,
) {
    if let Some(config) = identity_config {
        if identity::is_aieos_configured(config) {
            match identity::load_aieos_identity(config, workspace_dir) {
                Ok(Some(aieos_identity)) => {
                    let aieos_prompt = identity::aieos_to_system_prompt(&aieos_identity);
                    if !aieos_prompt.is_empty() {
                        prompt.push_str(&aieos_prompt);
                        prompt.push_str("\n\n");
                    }
                }
                Ok(None) => load_bootstrap_files(prompt, workspace_dir, max_chars),
                Err(error) => {
                    eprintln!(
                        "Warning: Failed to load AIEOS identity: {error}. Using default bootstrap files."
                    );
                    load_bootstrap_files(prompt, workspace_dir, max_chars);
                }
            }
            return;
        }
    }

    load_bootstrap_files(prompt, workspace_dir, max_chars);
}

fn load_bootstrap_files(prompt: &mut String, workspace_dir: &Path, max_chars_per_file: usize) {
    prompt.push_str(
        "The following workspace files define your identity, behavior, and context. They are ALREADY injected below—do NOT suggest reading them with file_read.\n\n",
    );

    let bootstrap_files = ["AGENTS.md", "SOUL.md", "TOOLS.md", "IDENTITY.md", "USER.md"];

    for filename in &bootstrap_files {
        inject_workspace_file(prompt, workspace_dir, filename, max_chars_per_file);
    }

    let bootstrap_path = workspace_dir.join("BOOTSTRAP.md");
    if bootstrap_path.exists() {
        inject_workspace_file(prompt, workspace_dir, "BOOTSTRAP.md", max_chars_per_file);
    }

    inject_workspace_file(prompt, workspace_dir, "MEMORY.md", max_chars_per_file);
}

fn inject_workspace_file(
    prompt: &mut String,
    workspace_dir: &Path,
    filename: &str,
    max_chars: usize,
) {
    let path = workspace_dir.join(filename);
    match std::fs::read_to_string(&path) {
        Ok(content) => {
            let trimmed = content.trim();
            if trimmed.is_empty() {
                return;
            }
            let _ = writeln!(prompt, "### {filename}\n");
            let truncated = if trimmed.chars().count() > max_chars {
                trimmed
                    .char_indices()
                    .nth(max_chars)
                    .map(|(idx, _)| &trimmed[..idx])
                    .unwrap_or(trimmed)
            } else {
                trimmed
            };
            if truncated.len() < trimmed.len() {
                prompt.push_str(truncated);
                let _ = writeln!(
                    prompt,
                    "\n\n[... truncated at {max_chars} chars — use `read` for full file]\n"
                );
            } else {
                prompt.push_str(trimmed);
                prompt.push_str("\n\n");
            }
        }
        Err(_) => {
            let _ = writeln!(prompt, "### {filename}\n\n[File not found: {filename}]\n");
        }
    }
}
