use super::{ChannelsConfig, Config};
use crate::security::SecretStore;
use anyhow::{Context, Result};
use std::collections::HashMap;
use std::path::Path;

pub(super) fn decrypt_config_secrets(topclaw_dir: &Path, config: &mut Config) -> Result<()> {
    let store = SecretStore::new(topclaw_dir, config.secrets.encrypt);

    decrypt_optional_secret(&store, &mut config.api_key, "config.api_key")?;
    decrypt_optional_secret(
        &store,
        &mut config.proxy.http_proxy,
        "config.proxy.http_proxy",
    )?;
    decrypt_optional_secret(
        &store,
        &mut config.proxy.https_proxy,
        "config.proxy.https_proxy",
    )?;
    decrypt_optional_secret(
        &store,
        &mut config.proxy.all_proxy,
        "config.proxy.all_proxy",
    )?;
    decrypt_optional_secret(
        &store,
        &mut config.browser.computer_use.api_key,
        "config.browser.computer_use.api_key",
    )?;
    decrypt_optional_secret(
        &store,
        &mut config.web_search.brave_api_key,
        "config.web_search.brave_api_key",
    )?;
    decrypt_optional_secret(
        &store,
        &mut config.storage.provider.config.db_url,
        "config.storage.provider.config.db_url",
    )?;
    decrypt_vec_secrets(
        &store,
        &mut config.reliability.api_keys,
        "config.reliability.api_keys",
    )?;
    decrypt_map_secrets(
        &store,
        &mut config.reliability.fallback_api_keys,
        "config.reliability.fallback_api_keys",
    )?;
    decrypt_vec_secrets(
        &store,
        &mut config.gateway.paired_tokens,
        "config.gateway.paired_tokens",
    )?;

    for agent in config.agents.values_mut() {
        decrypt_optional_secret(&store, &mut agent.api_key, "config.agents.*.api_key")?;
    }

    decrypt_channel_secrets(&store, &mut config.channels_config)?;
    Ok(())
}

pub(super) fn encrypt_config_secrets(topclaw_dir: &Path, config: &mut Config) -> Result<()> {
    let store = SecretStore::new(topclaw_dir, config.secrets.encrypt);

    encrypt_optional_secret(&store, &mut config.api_key, "config.api_key")?;
    encrypt_optional_secret(
        &store,
        &mut config.proxy.http_proxy,
        "config.proxy.http_proxy",
    )?;
    encrypt_optional_secret(
        &store,
        &mut config.proxy.https_proxy,
        "config.proxy.https_proxy",
    )?;
    encrypt_optional_secret(
        &store,
        &mut config.proxy.all_proxy,
        "config.proxy.all_proxy",
    )?;
    encrypt_optional_secret(
        &store,
        &mut config.browser.computer_use.api_key,
        "config.browser.computer_use.api_key",
    )?;
    encrypt_optional_secret(
        &store,
        &mut config.web_search.brave_api_key,
        "config.web_search.brave_api_key",
    )?;
    encrypt_optional_secret(
        &store,
        &mut config.storage.provider.config.db_url,
        "config.storage.provider.config.db_url",
    )?;
    encrypt_vec_secrets(
        &store,
        &mut config.reliability.api_keys,
        "config.reliability.api_keys",
    )?;
    encrypt_map_secrets(
        &store,
        &mut config.reliability.fallback_api_keys,
        "config.reliability.fallback_api_keys",
    )?;
    encrypt_vec_secrets(
        &store,
        &mut config.gateway.paired_tokens,
        "config.gateway.paired_tokens",
    )?;

    for agent in config.agents.values_mut() {
        encrypt_optional_secret(&store, &mut agent.api_key, "config.agents.*.api_key")?;
    }

    encrypt_channel_secrets(&store, &mut config.channels_config)?;
    Ok(())
}

fn decrypt_optional_secret(
    store: &SecretStore,
    value: &mut Option<String>,
    field_name: &str,
) -> Result<()> {
    if let Some(raw) = value.clone() {
        if SecretStore::is_encrypted(&raw) {
            *value = Some(
                store
                    .decrypt(&raw)
                    .with_context(|| format!("Failed to decrypt {field_name}"))?,
            );
        }
    }
    Ok(())
}

fn decrypt_secret(store: &SecretStore, value: &mut String, field_name: &str) -> Result<()> {
    if SecretStore::is_encrypted(value) {
        *value = store
            .decrypt(value)
            .with_context(|| format!("Failed to decrypt {field_name}"))?;
    }
    Ok(())
}

fn decrypt_vec_secrets(store: &SecretStore, values: &mut [String], field_name: &str) -> Result<()> {
    for (idx, value) in values.iter_mut().enumerate() {
        if SecretStore::is_encrypted(value) {
            *value = store
                .decrypt(value)
                .with_context(|| format!("Failed to decrypt {field_name}[{idx}]"))?;
        }
    }
    Ok(())
}

fn decrypt_map_secrets(
    store: &SecretStore,
    values: &mut HashMap<String, String>,
    field_name: &str,
) -> Result<()> {
    for (key, value) in values.iter_mut() {
        if SecretStore::is_encrypted(value) {
            *value = store
                .decrypt(value)
                .with_context(|| format!("Failed to decrypt {field_name}.{key}"))?;
        }
    }
    Ok(())
}

fn encrypt_optional_secret(
    store: &SecretStore,
    value: &mut Option<String>,
    field_name: &str,
) -> Result<()> {
    if let Some(raw) = value.clone() {
        if !SecretStore::is_encrypted(&raw) {
            *value = Some(
                store
                    .encrypt(&raw)
                    .with_context(|| format!("Failed to encrypt {field_name}"))?,
            );
        }
    }
    Ok(())
}

fn encrypt_secret(store: &SecretStore, value: &mut String, field_name: &str) -> Result<()> {
    if !SecretStore::is_encrypted(value) {
        *value = store
            .encrypt(value)
            .with_context(|| format!("Failed to encrypt {field_name}"))?;
    }
    Ok(())
}

fn encrypt_vec_secrets(store: &SecretStore, values: &mut [String], field_name: &str) -> Result<()> {
    for (idx, value) in values.iter_mut().enumerate() {
        if !SecretStore::is_encrypted(value) {
            *value = store
                .encrypt(value)
                .with_context(|| format!("Failed to encrypt {field_name}[{idx}]"))?;
        }
    }
    Ok(())
}

fn encrypt_map_secrets(
    store: &SecretStore,
    values: &mut HashMap<String, String>,
    field_name: &str,
) -> Result<()> {
    for (key, value) in values.iter_mut() {
        if !SecretStore::is_encrypted(value) {
            *value = store
                .encrypt(value)
                .with_context(|| format!("Failed to encrypt {field_name}.{key}"))?;
        }
    }
    Ok(())
}

fn decrypt_channel_secrets(store: &SecretStore, channels: &mut ChannelsConfig) -> Result<()> {
    if let Some(ref mut telegram) = channels.telegram {
        decrypt_secret(
            store,
            &mut telegram.bot_token,
            "config.channels_config.telegram.bot_token",
        )?;
    }
    if let Some(ref mut discord) = channels.discord {
        decrypt_secret(
            store,
            &mut discord.bot_token,
            "config.channels_config.discord.bot_token",
        )?;
    }
    if let Some(ref mut webhook) = channels.webhook {
        decrypt_optional_secret(
            store,
            &mut webhook.secret,
            "config.channels_config.webhook.secret",
        )?;
    }
    Ok(())
}

fn encrypt_channel_secrets(store: &SecretStore, channels: &mut ChannelsConfig) -> Result<()> {
    if let Some(ref mut telegram) = channels.telegram {
        encrypt_secret(
            store,
            &mut telegram.bot_token,
            "config.channels_config.telegram.bot_token",
        )?;
    }
    if let Some(ref mut discord) = channels.discord {
        encrypt_secret(
            store,
            &mut discord.bot_token,
            "config.channels_config.discord.bot_token",
        )?;
    }
    if let Some(ref mut webhook) = channels.webhook {
        encrypt_optional_secret(
            store,
            &mut webhook.secret,
            "config.channels_config.webhook.secret",
        )?;
    }
    Ok(())
}
