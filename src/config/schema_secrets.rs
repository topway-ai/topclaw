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
        &mut config.composio.api_key,
        "config.composio.api_key",
    )?;
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
        &mut config.composio.api_key,
        "config.composio.api_key",
    )?;
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
    if let Some(ref mut slack) = channels.slack {
        decrypt_secret(
            store,
            &mut slack.bot_token,
            "config.channels_config.slack.bot_token",
        )?;
        decrypt_optional_secret(
            store,
            &mut slack.app_token,
            "config.channels_config.slack.app_token",
        )?;
    }
    if let Some(ref mut mattermost) = channels.mattermost {
        decrypt_secret(
            store,
            &mut mattermost.bot_token,
            "config.channels_config.mattermost.bot_token",
        )?;
    }
    if let Some(ref mut webhook) = channels.webhook {
        decrypt_optional_secret(
            store,
            &mut webhook.secret,
            "config.channels_config.webhook.secret",
        )?;
    }
    if let Some(ref mut bridge) = channels.bridge {
        if !bridge.auth_token.trim().is_empty() {
            decrypt_secret(
                store,
                &mut bridge.auth_token,
                "config.channels_config.bridge.auth_token",
            )?;
        }
    }
    if let Some(ref mut matrix) = channels.matrix {
        decrypt_secret(
            store,
            &mut matrix.access_token,
            "config.channels_config.matrix.access_token",
        )?;
    }
    if let Some(ref mut whatsapp) = channels.whatsapp {
        decrypt_optional_secret(
            store,
            &mut whatsapp.access_token,
            "config.channels_config.whatsapp.access_token",
        )?;
        decrypt_optional_secret(
            store,
            &mut whatsapp.app_secret,
            "config.channels_config.whatsapp.app_secret",
        )?;
        decrypt_optional_secret(
            store,
            &mut whatsapp.verify_token,
            "config.channels_config.whatsapp.verify_token",
        )?;
    }
    if let Some(ref mut linq) = channels.linq {
        decrypt_secret(
            store,
            &mut linq.api_token,
            "config.channels_config.linq.api_token",
        )?;
        decrypt_optional_secret(
            store,
            &mut linq.signing_secret,
            "config.channels_config.linq.signing_secret",
        )?;
    }
    if let Some(ref mut nextcloud) = channels.nextcloud_talk {
        decrypt_secret(
            store,
            &mut nextcloud.app_token,
            "config.channels_config.nextcloud_talk.app_token",
        )?;
        decrypt_optional_secret(
            store,
            &mut nextcloud.webhook_secret,
            "config.channels_config.nextcloud_talk.webhook_secret",
        )?;
    }
    if let Some(ref mut irc) = channels.irc {
        decrypt_optional_secret(
            store,
            &mut irc.server_password,
            "config.channels_config.irc.server_password",
        )?;
        decrypt_optional_secret(
            store,
            &mut irc.nickserv_password,
            "config.channels_config.irc.nickserv_password",
        )?;
        decrypt_optional_secret(
            store,
            &mut irc.sasl_password,
            "config.channels_config.irc.sasl_password",
        )?;
    }
    if let Some(ref mut lark) = channels.lark {
        decrypt_secret(
            store,
            &mut lark.app_secret,
            "config.channels_config.lark.app_secret",
        )?;
        decrypt_optional_secret(
            store,
            &mut lark.encrypt_key,
            "config.channels_config.lark.encrypt_key",
        )?;
        decrypt_optional_secret(
            store,
            &mut lark.verification_token,
            "config.channels_config.lark.verification_token",
        )?;
    }
    if let Some(ref mut dingtalk) = channels.dingtalk {
        decrypt_secret(
            store,
            &mut dingtalk.client_secret,
            "config.channels_config.dingtalk.client_secret",
        )?;
    }
    if let Some(ref mut qq) = channels.qq {
        decrypt_secret(
            store,
            &mut qq.app_secret,
            "config.channels_config.qq.app_secret",
        )?;
    }
    if let Some(ref mut nostr) = channels.nostr {
        decrypt_secret(
            store,
            &mut nostr.private_key,
            "config.channels_config.nostr.private_key",
        )?;
    }
    if let Some(ref mut clawdtalk) = channels.clawdtalk {
        decrypt_secret(
            store,
            &mut clawdtalk.api_key,
            "config.channels_config.clawdtalk.api_key",
        )?;
        decrypt_optional_secret(
            store,
            &mut clawdtalk.webhook_secret,
            "config.channels_config.clawdtalk.webhook_secret",
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
    if let Some(ref mut slack) = channels.slack {
        encrypt_secret(
            store,
            &mut slack.bot_token,
            "config.channels_config.slack.bot_token",
        )?;
        encrypt_optional_secret(
            store,
            &mut slack.app_token,
            "config.channels_config.slack.app_token",
        )?;
    }
    if let Some(ref mut mattermost) = channels.mattermost {
        encrypt_secret(
            store,
            &mut mattermost.bot_token,
            "config.channels_config.mattermost.bot_token",
        )?;
    }
    if let Some(ref mut webhook) = channels.webhook {
        encrypt_optional_secret(
            store,
            &mut webhook.secret,
            "config.channels_config.webhook.secret",
        )?;
    }
    if let Some(ref mut bridge) = channels.bridge {
        if !bridge.auth_token.trim().is_empty() {
            encrypt_secret(
                store,
                &mut bridge.auth_token,
                "config.channels_config.bridge.auth_token",
            )?;
        }
    }
    if let Some(ref mut matrix) = channels.matrix {
        encrypt_secret(
            store,
            &mut matrix.access_token,
            "config.channels_config.matrix.access_token",
        )?;
    }
    if let Some(ref mut whatsapp) = channels.whatsapp {
        encrypt_optional_secret(
            store,
            &mut whatsapp.access_token,
            "config.channels_config.whatsapp.access_token",
        )?;
        encrypt_optional_secret(
            store,
            &mut whatsapp.app_secret,
            "config.channels_config.whatsapp.app_secret",
        )?;
        encrypt_optional_secret(
            store,
            &mut whatsapp.verify_token,
            "config.channels_config.whatsapp.verify_token",
        )?;
    }
    if let Some(ref mut linq) = channels.linq {
        encrypt_secret(
            store,
            &mut linq.api_token,
            "config.channels_config.linq.api_token",
        )?;
        encrypt_optional_secret(
            store,
            &mut linq.signing_secret,
            "config.channels_config.linq.signing_secret",
        )?;
    }
    if let Some(ref mut nextcloud) = channels.nextcloud_talk {
        encrypt_secret(
            store,
            &mut nextcloud.app_token,
            "config.channels_config.nextcloud_talk.app_token",
        )?;
        encrypt_optional_secret(
            store,
            &mut nextcloud.webhook_secret,
            "config.channels_config.nextcloud_talk.webhook_secret",
        )?;
    }
    if let Some(ref mut irc) = channels.irc {
        encrypt_optional_secret(
            store,
            &mut irc.server_password,
            "config.channels_config.irc.server_password",
        )?;
        encrypt_optional_secret(
            store,
            &mut irc.nickserv_password,
            "config.channels_config.irc.nickserv_password",
        )?;
        encrypt_optional_secret(
            store,
            &mut irc.sasl_password,
            "config.channels_config.irc.sasl_password",
        )?;
    }
    if let Some(ref mut lark) = channels.lark {
        encrypt_secret(
            store,
            &mut lark.app_secret,
            "config.channels_config.lark.app_secret",
        )?;
        encrypt_optional_secret(
            store,
            &mut lark.encrypt_key,
            "config.channels_config.lark.encrypt_key",
        )?;
        encrypt_optional_secret(
            store,
            &mut lark.verification_token,
            "config.channels_config.lark.verification_token",
        )?;
    }
    if let Some(ref mut dingtalk) = channels.dingtalk {
        encrypt_secret(
            store,
            &mut dingtalk.client_secret,
            "config.channels_config.dingtalk.client_secret",
        )?;
    }
    if let Some(ref mut qq) = channels.qq {
        encrypt_secret(
            store,
            &mut qq.app_secret,
            "config.channels_config.qq.app_secret",
        )?;
    }
    if let Some(ref mut nostr) = channels.nostr {
        encrypt_secret(
            store,
            &mut nostr.private_key,
            "config.channels_config.nostr.private_key",
        )?;
    }
    if let Some(ref mut clawdtalk) = channels.clawdtalk {
        encrypt_secret(
            store,
            &mut clawdtalk.api_key,
            "config.channels_config.clawdtalk.api_key",
        )?;
        encrypt_optional_secret(
            store,
            &mut clawdtalk.webhook_secret,
            "config.channels_config.clawdtalk.webhook_secret",
        )?;
    }
    Ok(())
}
