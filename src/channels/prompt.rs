use super::BOOTSTRAP_MAX_CHARS;
use crate::config::{Config, IdentityConfig, SkillsPromptInjectionMode};
use std::fmt::Write;
use std::path::Path;

/// Build the "Self-Configuration" instructions appended to the channel system
/// prompt. Tells the model that the TopClaw runtime config file is editable
/// (with explicit user confirmation) so it stops claiming the file is
/// out-of-reach when a user asks for a runtime tweak.
pub(crate) fn build_self_config_instructions(config_path: &Path) -> String {
    let display_path = config_path.display();
    format!(
        "\n## Self-Configuration\n\n\
         Your runtime configuration lives at `{display_path}`. It is a regular \
         TOML file owned by the same user that started this process — when the \
         user asks you to enable a tool, widen an allowlist, switch provider/model, \
         or otherwise change runtime behavior, treat it as editable through \
         `file_read`/`file_edit` (or `shell` if those are gated).\n\
         - Do NOT tell the user the config is \"outside the workspace\" or that you \
         cannot read/write it; the path above is reachable by your normal file tools.\n\
         - Always READ the file first, then propose the exact diff (section, \
         before/after lines) and ask the user to confirm before writing.\n\
         - Never expand permissions silently. Confirmation must come from the user \
         in this same chat/channel for each distinct config change.\n\
         - After a confirmed edit, tell the user that some changes (channels, \
         provider keys, allowlists baked in at startup) only take effect after \
         restarting the TopClaw process, so they know whether to expect immediate \
         behavior or a restart.\n\
         - Never echo, paste, or describe the values of `api_key`, `bot_token`, or \
         any field whose value starts with `enc2:` — those are encrypted secrets \
         and must stay redacted in chat.\n"
    )
}

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
    // ── File operations ──
    let mut tool_descs = vec![
        (
            "file_read",
            "Read file contents with line numbers. Use when: inspecting code, configs, logs, PDFs. Supports partial reads (offset/limit) for large files. Don't use when: you need to search across many files — use content_search or glob_search instead.",
        ),
        (
            "file_write",
            "Create or overwrite a file. Use when: creating new files or replacing entire content. Don't use when: making targeted edits to existing files — use file_edit instead.",
        ),
        (
            "file_edit",
            "Edit a file by finding and replacing an exact string. Use when: making targeted changes to existing files (rename a variable, fix a bug, update a config value). Safer than file_write because it only changes what you specify. Don't use when: creating new files or rewriting entire content — use file_write.",
        ),
        // ── Search ──
        (
            "glob_search",
            "Find files by name/path pattern. Use when: locating files by extension, name pattern, or directory structure (e.g. '**/*.rs', 'src/**/test_*'). Don't use when: searching file contents — use content_search.",
        ),
        (
            "content_search",
            "Search file contents by regex (like grep/ripgrep). Use when: finding code patterns, TODOs, references, function definitions, string matches across files. Supports context lines, file filters, count mode. Don't use when: you already know the exact file — use file_read.",
        ),
        // ── Shell & processes ──
        (
            "shell",
            "Execute terminal commands. Use when: running builds, tests, diagnostics, or any task not covered by a dedicated tool. Has 60-second timeout. Don't use when: a safer dedicated tool exists (prefer file_read over cat, content_search over grep, git_operations over raw git).",
        ),
        (
            "process",
            "Manage background processes (spawn, status, kill, output). Use when: running long commands that exceed shell's 60-second timeout (servers, builds, watchers). Max 8 concurrent. Don't use when: command completes quickly — use shell instead.",
        ),
        // ── Git ──
        (
            "git_operations",
            "Structured git operations (status, diff, log, add, commit, branch, etc.) with parsed JSON output. Use when: performing git tasks — safer than raw git via shell because it sanitizes inputs and blocks dangerous flags. Don't use when: you need advanced git features not supported by the structured interface.",
        ),
        (
            "apply_patch",
            "Apply unified diffs to the repository. Use when: applying patches from git diff output. Always does dry-run first. Don't use when: making simple edits — use file_edit.",
        ),
        // ── Memory ──
        (
            "memory_store",
            "Save information to long-term memory. Use when: preserving user preferences, project decisions, recurring context that should survive across sessions. Don't use when: information is transient or already in workspace files.",
        ),
        (
            "memory_recall",
            "Search long-term memory by keywords. Use when: retrieving prior decisions, user preferences, stored facts. Don't use when: the answer is already in conversation context or workspace files.",
        ),
        (
            "memory_forget",
            "Delete a memory entry by key. Use when: memory is incorrect, outdated, or user requests removal. Don't use when: uncertain about impact.",
        ),
        // ── Conversation history ──
        (
            "lossless_describe",
            "List preserved conversation sessions with metadata. Use when: inspecting past conversation history across sessions.",
        ),
        (
            "lossless_search",
            "Search across preserved conversation messages and summaries. Use when: finding context from previous conversations by keyword.",
        ),
        // ── Planning ──
        (
            "task_plan",
            "Create and manage in-session task checklists. Use when: breaking complex multi-step work into tracked subtasks. Not persisted across sessions. Don't use when: task is simple enough to do in one step.",
        ),
        // ── Scheduling ──
        (
            "schedule",
            "Manage local scheduled tasks (cron, one-shot, interval). Use when: scheduling shell commands to run later or on a recurring basis. Output is logged locally. Don't use when: you need results delivered to a chat channel — use cron_add instead.",
        ),
        (
            "cron_add",
            "Create scheduled jobs (shell or agent) with optional channel delivery. Use when: scheduling tasks that should announce results in Discord/Telegram/Slack/etc. Supports cron expressions, at-syntax, and interval schedules.",
        ),
        (
            "cron_list",
            "List all scheduled cron jobs. Use when: checking what's scheduled.",
        ),
        (
            "cron_remove",
            "Remove a cron job by ID. Use when: canceling a scheduled task.",
        ),
        (
            "cron_run",
            "Manually trigger a cron job immediately. Use when: testing a scheduled job.",
        ),
        (
            "cron_runs",
            "List past cron job executions and their output. Use when: checking job history and results.",
        ),
        (
            "cron_update",
            "Update an existing cron job's schedule, command, or delivery. Use when: modifying a scheduled task without recreating it.",
        ),
        // ── Media ──
        (
            "screenshot",
            "Capture the current screen as an image. Use when: user asks to see what's on screen or needs visual context. Returns base64 PNG.",
        ),
        (
            "image_info",
            "Extract image metadata (dimensions, format, size) and optionally base64 data. Use when: analyzing image files or preparing images for multimodal queries.",
        ),
        // ── Runtime configuration ──
        (
            "proxy_config",
            "Read or write HTTP proxy settings at runtime. Use when: user needs to configure proxy for network access.",
        ),
        (
            "model_routing_config",
            "Read or write model routing and delegate agent configurations. Use when: dynamically switching models or configuring sub-agents.",
        ),
    ];

    // ── Conditional: desktop computer use (listed FIRST to take priority over web tools) ──
    // Desktop automation must appear before web_fetch/browser_open so the LLM
    // selects it for "open Chrome" / "open <app>" / "navigate to URL" requests
    // instead of falling through to URL-fetching tools that only download HTML.
    if config.browser.computer_use.enabled {
        tool_descs.push((
            "computer_use",
            "Desktop automation: launch applications, open URLs in a visible browser window, list/focus/close windows, take screenshots, click, drag, type, or press keys. \
             IMPORTANT: When the user says 'open Chrome', 'open <app>', 'open this link in Chrome', or 'navigate to <URL> on the computer', use action=app_launch with the app name and args=[\"URL\"]. \
             Do NOT use web_fetch for these — web_fetch only downloads HTML text, it does NOT open a visible window or interact with the desktop. \
             Do NOT use browser_open for launching apps — browser_open only opens URLs and cannot launch arbitrary applications. \
             Example: to open https://example.com in Chrome, call computer_use with action=app_launch, app=\"google-chrome\", args=[\"https://example.com\"]. \
             If a call fails because Linux desktop helpers are missing, call it once with action=bootstrap to install them, then retry. \
             Do NOT claim an app opened, a click happened, or the screen was captured unless this tool actually returned success.",
        ));
    }

    // ── Conditional: web tools ──
    if config.web_search.enabled {
        tool_descs.push((
            "web_search",
            "Search the internet for current information. Use when: user asks about current events, needs fresh data not in workspace, wants to look something up. Don't use when: answer is available locally in workspace files or memory.",
        ));
    }
    if config.web_fetch.enabled {
        tool_descs.push((
            "web_fetch",
            "Fetch a web page and convert to readable text/markdown. Use when: user provides a URL to read, or you need to retrieve specific web content. Don't use when: you just need search results — use web_search. Don't use when: user wants to open a URL in a browser — use computer_use.",
        ));
    }
    if config.http_request.enabled {
        tool_descs.push((
            "http_request",
            "Make HTTP API requests (GET/POST/PUT/DELETE/etc.). Use when: interacting with REST APIs, webhooks, or services. Domain-allowlist enforced. Don't use when: fetching web pages for reading — use web_fetch.",
        ));
    }

    // ── Conditional: browser ──
    if config.browser.enabled {
        tool_descs.push((
            "browser_open",
            "Open an approved HTTPS URL in the system browser. Allowlist-only, no scraping, no app launching. Don't use when: user wants to launch a specific app or interact with the desktop — use computer_use.",
        ));
        tool_descs.push((
            "browser",
            "Full browser automation (navigate, click, type, scroll). Use when: complex web interactions that require DOM manipulation or multi-step flows. Don't use when: simple URL fetch works — prefer web_fetch or web_search.",
        ));
    }

    // ── Conditional: discord history ──
    if config.channels_config.discord.is_some() {
        tool_descs.push((
            "discord_history_fetch",
            "Fetch Discord message history for conversation context. Use when: need to reference recent Discord messages.",
        ));
    }

    // ── Conditional: delegate agents ──
    if !config.agents.is_empty() {
        tool_descs.push((
            "delegate",
            "Delegate a subtask to a specialized agent (different model/provider). Use when: task benefits from a different model for speed, reasoning depth, or specialization.",
        ));
        tool_descs.push((
            "subagent_spawn",
            "Spawn a delegate agent in background, returns immediately with session_id. Use for long-running tasks that should not block the current conversation.",
        ));
        tool_descs.push((
            "subagent_list",
            "List background sub-agents. Filter by status: running, completed, failed, killed, all.",
        ));
        tool_descs.push((
            "subagent_manage",
            "Check sub-agent progress ('status') or cancel ('kill') a running session.",
        ));
        tool_descs.push((
            "delegate_coordination_status",
            "Inspect delegate coordination runtime state for debugging multi-agent workflows.",
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

    // Desktop automation routing: when computer_use is available, inject a
    // strong hint so the LLM picks it for "open Chrome", "open <app>",
    // "navigate to URL on the computer" instead of falling through to
    // web_fetch or browser_open (which only download HTML / open URLs).
    let has_computer_use = tools.iter().any(|(name, _)| *name == "computer_use");
    let headless_linux = cfg!(target_os = "linux") && std::env::var("DISPLAY").is_err();
    if has_computer_use {
        prompt.push_str(
            "## Desktop Automation\n\n\
             You HAVE the computer_use tool for desktop automation. USE it when:\n\
             - The user says 'open Chrome', 'open <app>', or 'launch <program>'\n\
             - The user says 'open this link in Chrome/the browser' or 'navigate to <URL> on the computer'\n\
             - The user wants to see what's on screen, click, type, or interact with the desktop\n\
             Do NOT use web_fetch for these — web_fetch only downloads HTML text and does NOT open a visible window.\n\
             Do NOT use browser_open to launch apps — browser_open only opens URLs and cannot launch arbitrary applications.\n\
             To open a URL in Chrome: computer_use with action=app_launch, app=\"google-chrome\", args=[\"<URL>\"].\n\
             To open a URL in Firefox: computer_use with action=app_launch, app=\"firefox\", args=[\"<URL>\"].\n\
             If the tool reports missing Linux helpers, call it once with action=bootstrap to install them, then retry.\n",
        );
        // When the tool is registered but the environment is headless,
        // add a caveat so the LLM warns the user instead of failing silently.
        if headless_linux {
            prompt.push_str(
                "WARNING: no display server detected ($DISPLAY not set) — \
                 GUI operations (app_launch, screen_capture, mouse/keyboard) will likely fail. \
                 If a computer_use call returns an error about missing helpers or no display, \
                 tell the user that desktop automation requires running TopClaw on a host \
                 with a display server (not inside a headless Docker container). \
                 For web content, fall back to web_fetch or web_search.\n\n",
            );
        } else {
            prompt.push('\n');
        }
    } else if headless_linux {
        // No display server detected (likely headless/Docker). Tell the LLM
        // so it doesn't attempt impossible desktop tasks and gives a clear
        // explanation to the user instead.
        prompt.push_str(
            "## Desktop Automation\n\n\
             Desktop automation (opening apps, clicking, taking screenshots) is NOT available in this environment —\n\
             there is no display server (X11/Wayland). This typically means TopClaw is running inside a headless container.\n\
             Do NOT attempt to use computer_use, browser_open, or any tool that requires a GUI.\n\
             If the user asks to 'open Chrome', 'open an app', or 'see the screen', explain that this requires\n\
             running TopClaw on a host with a display server (not in a headless Docker container).\n\
             For web content, use web_fetch or web_search instead.\n\n",
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
         - When in doubt, ask before acting externally.\n\
         - Do not claim an action was performed (app opened, file written, \
         message sent, click registered) unless a tool actually ran and \
         returned success in this turn. If no tool confirmed the effect, \
         say so plainly instead of narrating a fictional outcome.\n\n",
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
    prompt.push_str("- When a task clearly needs a tool, attempt the tool call instead of inventing a manual approval ritual in chat.\n");
    prompt.push_str("  The runtime will emit the real approval prompt if supervised access is required, then auto-resume the task after approval.\n");
    prompt.push_str("  Never tell the user to reply with plain text like `approve`, `allow`, or similar to unlock tools.\n");
    prompt.push_str("  If the user asks to fetch a web page, search the web, clone a repo, or any other task requiring a tool, call the appropriate tool flow instead of refusing or claiming execution is unavailable.\n");
    prompt.push_str("- If the user asks what you can do, describe concrete current abilities and constraints instead of saying you are unsure.\n");
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
    _identity_config: Option<&IdentityConfig>,
    max_chars: usize,
) {
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

#[cfg(test)]
mod self_config_tests {
    use super::build_self_config_instructions;
    use std::path::PathBuf;

    #[test]
    fn self_config_instructions_include_path_and_confirmation_rules() {
        let path = PathBuf::from("/home/topclaw_user/.topclaw/config.toml");
        let prompt = build_self_config_instructions(&path);
        assert!(prompt.contains("/home/topclaw_user/.topclaw/config.toml"));
        assert!(prompt.contains("Self-Configuration"));
        assert!(prompt.contains("ask the user to confirm before writing"));
        assert!(prompt.contains("Never expand permissions silently"));
        assert!(prompt.contains("enc2:"));
        // The whole point of this section: stop the model from refusing.
        assert!(prompt.contains("Do NOT tell the user the config is"));
    }
}

#[cfg(test)]
mod tool_description_tests {
    use super::build_channel_tool_descriptions;
    use crate::config::Config;

    #[test]
    fn computer_use_appears_before_web_fetch_when_enabled() {
        let mut config = Config::default();
        config.browser.computer_use.enabled = true;
        config.web_fetch.enabled = true;

        let descs = build_channel_tool_descriptions(&config);
        let names: Vec<&str> = descs.iter().map(|(n, _)| *n).collect();

        let cu_idx = names
            .iter()
            .position(|&n| n == "computer_use")
            .expect("computer_use should be present when enabled");
        let wf_idx = names
            .iter()
            .position(|&n| n == "web_fetch")
            .expect("web_fetch should be present when enabled");

        assert!(
            cu_idx < wf_idx,
            "computer_use (index {cu_idx}) must appear before web_fetch (index {wf_idx}) so the LLM selects it for 'open Chrome' requests"
        );
    }

    #[test]
    fn computer_use_appears_before_browser_open_when_enabled() {
        let mut config = Config::default();
        config.browser.computer_use.enabled = true;
        config.browser.enabled = true;

        let descs = build_channel_tool_descriptions(&config);
        let names: Vec<&str> = descs.iter().map(|(n, _)| *n).collect();

        let cu_idx = names
            .iter()
            .position(|&n| n == "computer_use")
            .expect("computer_use should be present when enabled");
        let bo_idx = names
            .iter()
            .position(|&n| n == "browser_open")
            .expect("browser_open should be present when browser.enabled");

        assert!(
            cu_idx < bo_idx,
            "computer_use (index {cu_idx}) must appear before browser_open (index {bo_idx})"
        );
    }

    #[test]
    fn computer_use_absent_when_disabled() {
        let mut config = Config::default();
        config.browser.computer_use.enabled = false;

        let descs = build_channel_tool_descriptions(&config);
        let names: Vec<&str> = descs.iter().map(|(n, _)| *n).collect();

        assert!(!names.contains(&"computer_use"));
    }
}

#[cfg(test)]
mod desktop_automation_prompt_tests {
    use super::build_system_prompt;
    use tempfile::TempDir;

    #[test]
    fn desktop_automation_section_present_when_computer_use_tool_available() {
        let tmp = TempDir::new().unwrap();
        let tools: Vec<(&str, &str)> = vec![
            ("computer_use", "Desktop automation tool"),
            ("web_fetch", "Fetch web pages"),
        ];
        let prompt = build_system_prompt(tmp.path(), "test-model", &tools, &[], None, None);

        assert!(
            prompt.contains("## Desktop Automation"),
            "System prompt must include Desktop Automation section when computer_use tool is available"
        );
        assert!(
            prompt.contains("You HAVE the computer_use tool"),
            "Desktop Automation section must tell the LLM it has the tool"
        );
        assert!(
            prompt.contains("Do NOT use web_fetch"),
            "Desktop Automation section must warn against web_fetch for desktop tasks"
        );
        assert!(
            prompt.contains("Do NOT use browser_open"),
            "Desktop Automation section must warn against browser_open for desktop tasks"
        );
    }

    #[test]
    fn desktop_automation_section_absent_when_no_computer_use_tool() {
        let tmp = TempDir::new().unwrap();
        let tools: Vec<(&str, &str)> = vec![
            ("web_fetch", "Fetch web pages"),
            ("shell", "Run commands"),
        ];
        let prompt = build_system_prompt(tmp.path(), "test-model", &tools, &[], None, None);

        // The positive routing hints must be absent
        assert!(
            !prompt.contains("You HAVE the computer_use tool"),
            "Desktop Automation 'you HAVE' section must NOT appear when computer_use is absent"
        );
        assert!(
            !prompt.contains("app_launch"),
            "app_launch routing hint must NOT appear when computer_use is absent"
        );
        assert!(
            !prompt.contains("google-chrome"),
            "Chrome routing hint must NOT appear when computer_use is absent"
        );
    }

    #[test]
    fn desktop_automation_routing_mentions_chrome_and_firefox() {
        let tmp = TempDir::new().unwrap();
        let tools: Vec<(&str, &str)> = vec![
            ("computer_use", "Desktop automation tool"),
        ];
        let prompt = build_system_prompt(tmp.path(), "test-model", &tools, &[], None, None);

        assert!(
            prompt.contains("google-chrome"),
            "Desktop Automation section must include Chrome example"
        );
        assert!(
            prompt.contains("firefox"),
            "Desktop Automation section must include Firefox example"
        );
        assert!(
            prompt.contains("app_launch"),
            "Desktop Automation section must mention app_launch action"
        );
    }
}
