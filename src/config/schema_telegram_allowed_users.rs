use super::ChannelsConfig;
use crate::config::autonomy::is_valid_env_var_name;
use anyhow::{Context, Result};

fn parse_telegram_allowed_users_env_value(
    raw_value: &str,
    env_name: &str,
    field_name: &str,
) -> Result<Vec<String>> {
    let trimmed = raw_value.trim();
    if trimmed.is_empty() {
        anyhow::bail!("{field_name} env reference ${{env:{env_name}}} resolved to an empty value");
    }

    let mut resolved: Vec<String> = Vec::new();
    if trimmed.starts_with('[') {
        let parsed: serde_json::Value = serde_json::from_str(trimmed).with_context(|| {
            format!(
                "{field_name} env reference ${{env:{env_name}}} must be valid JSON array or comma-separated list"
            )
        })?;
        let items = parsed.as_array().with_context(|| {
            format!("{field_name} env reference ${{env:{env_name}}} must be a JSON array")
        })?;
        for (idx, item) in items.iter().enumerate() {
            let candidate = match item {
                serde_json::Value::String(value) => value.trim().to_string(),
                serde_json::Value::Number(value) => value.to_string(),
                _ => {
                    anyhow::bail!(
                        "{field_name} env reference ${{env:{env_name}}}[{idx}] must be string or number"
                    );
                }
            };
            if !candidate.is_empty() {
                resolved.push(candidate);
            }
        }
    } else {
        resolved.extend(
            trimmed
                .split(',')
                .map(str::trim)
                .filter(|value| !value.is_empty())
                .map(ToString::to_string),
        );
    }

    if resolved.is_empty() {
        anyhow::bail!("{field_name} env reference ${{env:{env_name}}} produced no user IDs");
    }

    Ok(resolved)
}

pub(super) fn resolve_telegram_allowed_users_env_refs(channels: &mut ChannelsConfig) -> Result<()> {
    let Some(telegram) = channels.telegram.as_mut() else {
        return Ok(());
    };

    let field_name = "config.channels_config.telegram.allowed_users";
    let mut expanded_allowed_users: Vec<String> = Vec::new();
    for (idx, raw_entry) in telegram.allowed_users.drain(..).enumerate() {
        let entry = raw_entry.trim();
        if entry.is_empty() {
            continue;
        }

        if let Some(env_expr) = entry
            .strip_prefix("${env:")
            .and_then(|value| value.strip_suffix('}'))
        {
            let env_name = env_expr.trim();
            if !is_valid_env_var_name(env_name) {
                anyhow::bail!(
                    "{field_name}[{idx}] has invalid env var name ({env_name}); expected [A-Za-z_][A-Za-z0-9_]*"
                );
            }
            let env_value = std::env::var(env_name).with_context(|| {
                format!("{field_name}[{idx}] references unset environment variable {env_name}")
            })?;
            let mut parsed =
                parse_telegram_allowed_users_env_value(&env_value, env_name, field_name)?;
            expanded_allowed_users.append(&mut parsed);
        } else {
            expanded_allowed_users.push(entry.to_string());
        }
    }

    telegram.allowed_users = expanded_allowed_users;
    Ok(())
}
