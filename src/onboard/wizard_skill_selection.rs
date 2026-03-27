use crate::config::Config;
use crate::security::ShellRedirectPolicy;
use crate::skills::{CuratedSkillCatalogEntry, CuratedSkillRisk};
use anyhow::{Context, Result};
use console::{style, Key, Term};

#[derive(Debug, Clone, Default)]
pub(super) struct SkillOnboardingSelection {
    pub(super) selected_curated_slugs: Vec<String>,
}

pub(super) fn default_selected_onboarding_skill(entry: &CuratedSkillCatalogEntry) -> bool {
    entry.risk == CuratedSkillRisk::Lower
}

fn onboarding_skill_short_description(entry: &CuratedSkillCatalogEntry) -> &'static str {
    match entry.slug {
        "find-skills" => "find more skills",
        "skill-creator" => "create skills",
        "local-file-analyzer" => "read local files",
        "workspace-search" => "search workspace",
        "code-explainer" => "explain code",
        "change-summary" => "summarize changes",
        "safe-web-search" => "search the web",
        "self-improving-agent" => "save learnings",
        "multi-search-engine" => "advanced web search",
        "agent-browser-extension" => "browser automation",
        "desktop-computer-use" => "control desktop apps",
        _ => entry.description,
    }
}

pub(super) fn format_onboarding_skill_label(entry: &CuratedSkillCatalogEntry) -> String {
    format!(
        "{} — {}",
        entry.slug,
        onboarding_skill_short_description(entry)
    )
}

fn print_onboarding_skill_controls() {
    println!(
        "  {}",
        style("Up/Down navigate, Space toggles, Return/Enter continues, 'a' toggles all, 'c' clears all.")
            .dim()
    );
}

fn format_onboarding_skill_selection_report(
    entries: &[&CuratedSkillCatalogEntry],
    selected: &[usize],
) -> String {
    let chosen: Vec<&str> = selected
        .iter()
        .filter_map(|index| entries.get(*index).map(|entry| entry.slug))
        .collect();

    if chosen.is_empty() {
        "none".to_string()
    } else {
        chosen.join(", ")
    }
}

fn prompt_skill_selection_report(
    title: &str,
    entries: &[&CuratedSkillCatalogEntry],
    selected: &[usize],
) {
    println!(
        "  {} {}",
        style(title).cyan().bold(),
        style(format!(
            "[{}]",
            format_onboarding_skill_selection_report(entries, selected)
        ))
        .dim()
    );
}

fn prompt_skill_selection_instruction(help_text: &str) {
    println!("  {}", style(help_text).dim());
    print_onboarding_skill_controls();
}

/// Declarative mapping from skill slugs to the policy defaults they require.
/// Used during onboarding to automatically un-exclude and auto-approve tools,
/// and to unlock capability-first shell defaults when the user selects the
/// corresponding skill.
struct SkillPolicyMapping {
    slug: &'static str,
    tools_to_unexclude: &'static [&'static str],
    tools_to_auto_approve: &'static [&'static str],
    unlocks_capability_first_shell: bool,
}

const SKILL_POLICY_MAPPINGS: &[SkillPolicyMapping] = &[
    SkillPolicyMapping {
        slug: "workspace-search",
        tools_to_unexclude: &["shell"],
        tools_to_auto_approve: &["shell"],
        unlocks_capability_first_shell: true,
    },
    SkillPolicyMapping {
        slug: "code-explainer",
        tools_to_unexclude: &["shell"],
        tools_to_auto_approve: &["shell"],
        unlocks_capability_first_shell: true,
    },
    SkillPolicyMapping {
        slug: "change-summary",
        tools_to_unexclude: &["shell", "git_operations"],
        tools_to_auto_approve: &["shell", "git_operations"],
        unlocks_capability_first_shell: true,
    },
    SkillPolicyMapping {
        slug: "self-improving-agent",
        tools_to_unexclude: &["file_write", "file_edit", "memory_store"],
        tools_to_auto_approve: &["file_write", "file_edit", "memory_store"],
        unlocks_capability_first_shell: false,
    },
    SkillPolicyMapping {
        slug: "skill-creator",
        tools_to_unexclude: &["shell", "file_write", "file_edit"],
        tools_to_auto_approve: &["shell", "file_write", "file_edit"],
        unlocks_capability_first_shell: true,
    },
    SkillPolicyMapping {
        slug: "multi-search-engine",
        tools_to_unexclude: &["http_request"],
        tools_to_auto_approve: &["http_request"],
        unlocks_capability_first_shell: false,
    },
    SkillPolicyMapping {
        slug: "agent-browser-extension",
        tools_to_unexclude: &["browser", "browser_open"],
        tools_to_auto_approve: &["browser", "browser_open"],
        unlocks_capability_first_shell: false,
    },
    SkillPolicyMapping {
        slug: "desktop-computer-use",
        tools_to_unexclude: &["browser", "browser_open", "screenshot"],
        tools_to_auto_approve: &["browser", "browser_open", "screenshot"],
        unlocks_capability_first_shell: false,
    },
];

fn append_unique_strings(target: &mut Vec<String>, entries: &[&str]) {
    for entry in entries {
        if !target.iter().any(|existing| existing == entry) {
            target.push((*entry).to_string());
        }
    }
}

pub(super) fn apply_onboarding_skill_tool_defaults(
    config: &mut Config,
    selection: &SkillOnboardingSelection,
) {
    let has_skill = |slug: &str| {
        selection
            .selected_curated_slugs
            .iter()
            .any(|selected| selected == slug)
    };

    // Enable config feature flags that skills depend on.
    if has_skill("find-skills") || has_skill("safe-web-search") {
        config.web_search.enabled = true;
    }
    if has_skill("multi-search-engine") {
        config.web_fetch.enabled = true;
    }

    let wants_browser_extension = has_skill("agent-browser-extension");
    let wants_desktop_computer_use = has_skill("desktop-computer-use");
    if wants_browser_extension || wants_desktop_computer_use {
        config.browser.enabled = true;
        config.browser.backend = match (wants_browser_extension, wants_desktop_computer_use) {
            (true, true) => "auto",
            (true, false) => "agent_browser",
            (false, true) => "computer_use",
            (false, false) => unreachable!("checked above"),
        }
        .to_string();
    }

    // Un-exclude and auto-approve tools required by selected skills so they
    // work on non-CLI channels (Telegram, Discord, etc.) without manual policy
    // edits or approval prompts.
    let mut tools_to_unexclude = std::collections::HashSet::new();
    let mut tools_to_auto_approve = std::collections::HashSet::new();
    let mut unlocks_capability_first_shell = false;
    for mapping in SKILL_POLICY_MAPPINGS {
        if has_skill(mapping.slug) {
            tools_to_unexclude.extend(mapping.tools_to_unexclude.iter().copied());
            tools_to_auto_approve.extend(mapping.tools_to_auto_approve.iter().copied());
            unlocks_capability_first_shell |= mapping.unlocks_capability_first_shell;
        }
    }

    if !tools_to_unexclude.is_empty() {
        config
            .autonomy
            .non_cli_excluded_tools
            .retain(|tool| !tools_to_unexclude.contains(tool.as_str()));
    }

    if !tools_to_auto_approve.is_empty() {
        let mut tools: Vec<&str> = tools_to_auto_approve.iter().copied().collect();
        tools.sort_unstable();
        append_unique_strings(&mut config.autonomy.auto_approve, &tools);
        config
            .autonomy
            .always_ask
            .retain(|tool| !tools_to_auto_approve.contains(tool.as_str()));
    }

    if unlocks_capability_first_shell {
        append_unique_strings(&mut config.autonomy.allowed_commands, &["*"]);
        config.autonomy.shell_redirect_policy = ShellRedirectPolicy::Allow;
        config.autonomy.require_approval_for_medium_risk = false;
    }
}

fn prompt_skill_selection(
    title: &str,
    help_text: &str,
    entries: &[&CuratedSkillCatalogEntry],
) -> Result<Vec<String>> {
    if entries.is_empty() {
        return Ok(Vec::new());
    }

    prompt_skill_selection_instruction(help_text);
    let labels: Vec<String> = entries
        .iter()
        .map(|entry| format_onboarding_skill_label(entry))
        .collect();
    let defaults: Vec<bool> = entries
        .iter()
        .map(|entry| default_selected_onboarding_skill(entry))
        .collect();
    let selected = interact_onboarding_skill_selection(title, &labels, &defaults)
        .context("failed to read skill selection")?;
    prompt_skill_selection_report(title, entries, &selected);

    Ok(selected
        .into_iter()
        .map(|index| entries[index].slug.to_string())
        .collect())
}

fn render_onboarding_skill_selection(
    term: &Term,
    title: &str,
    labels: &[String],
    checked: &[bool],
    active: usize,
) -> Result<usize> {
    term.write_line(&format!("{title}:"))?;
    for (index, label) in labels.iter().enumerate() {
        let prefix = match (checked[index], active == index) {
            (true, true) => "> [x]",
            (true, false) => "  [x]",
            (false, true) => "> [ ]",
            (false, false) => "  [ ]",
        };
        term.write_line(&format!("{prefix} {label}"))?;
    }
    Ok(labels.len() + 1)
}

pub(super) fn apply_onboarding_skill_selection_key(
    key: Key,
    checked: &mut [bool],
    active: &mut usize,
) -> bool {
    match key {
        Key::ArrowDown | Key::Tab | Key::Char('j') => {
            *active = (*active + 1) % checked.len();
        }
        Key::ArrowUp | Key::BackTab | Key::Char('k') => {
            *active = (*active + checked.len() - 1) % checked.len();
        }
        Key::Char(' ') => {
            checked[*active] = !checked[*active];
        }
        Key::Char('a') => {
            if checked.iter().all(|item_checked| *item_checked) {
                checked.fill(false);
            } else {
                checked.fill(true);
            }
        }
        Key::Char('c') => {
            checked.fill(false);
        }
        Key::Enter => return true,
        _ => {}
    }

    false
}

fn interact_onboarding_skill_selection(
    title: &str,
    labels: &[String],
    defaults: &[bool],
) -> Result<Vec<usize>> {
    let term = Term::stderr();
    if !term.is_term() {
        anyhow::bail!("not a terminal");
    }
    if labels.is_empty() {
        return Ok(Vec::new());
    }

    let mut checked = defaults.to_vec();
    let mut active = 0usize;
    term.hide_cursor()?;

    let result = (|| -> Result<Vec<usize>> {
        let mut rendered_lines =
            render_onboarding_skill_selection(&term, title, labels, &checked, active)?;
        loop {
            let submit =
                apply_onboarding_skill_selection_key(term.read_key()?, &mut checked, &mut active);
            if submit {
                term.clear_last_lines(rendered_lines)?;
                return Ok(checked
                    .iter()
                    .enumerate()
                    .filter_map(|(index, selected)| selected.then_some(index))
                    .collect());
            }

            term.clear_last_lines(rendered_lines)?;
            rendered_lines =
                render_onboarding_skill_selection(&term, title, labels, &checked, active)?;
        }
    })();

    term.show_cursor()?;
    result
}

pub(super) fn setup_skills() -> Result<SkillOnboardingSelection> {
    println!(
        "  {}",
        style("Optional starter skills. Recommended ones are preselected.").dim()
    );

    let catalog = crate::skills::curated_skill_catalog();
    let ordered_entries: Vec<&CuratedSkillCatalogEntry> = catalog.iter().collect();
    let selected = prompt_skill_selection(
        "Starter skills",
        "Choose the skills you want to install.",
        &ordered_entries,
    )?;

    Ok(SkillOnboardingSelection {
        selected_curated_slugs: selected,
    })
}
