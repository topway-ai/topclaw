//! Channel factory module.
//!
//! This module provides channel instantiation from configuration.
//! It creates all supported channel types based on the runtime config.

use super::traits::Channel;
use crate::config::Config;
use std::sync::Arc;

#[cfg(feature = "channel-email")]
pub use super::EmailChannel;
#[cfg(feature = "channel-irc")]
pub use super::IrcChannel;
#[cfg(feature = "channel-lark")]
pub use super::LarkChannel;
#[cfg(feature = "channel-matrix")]
pub use super::MatrixChannel;
#[cfg(feature = "channel-nostr")]
pub use super::NostrChannel;
#[cfg(feature = "whatsapp-web")]
pub use super::WhatsAppWebChannel;
pub use super::{
    BridgeChannel, ClawdTalkChannel, DingTalkChannel, DiscordChannel, IMessageChannel, LinqChannel,
    MattermostChannel, NextcloudTalkChannel, QQChannel, SignalChannel, SlackChannel,
    TelegramChannel, WatiChannel, WhatsAppChannel,
};

/// A configured channel with its display name.
pub struct ConfiguredChannel {
    pub display_name: &'static str,
    pub channel: Arc<dyn Channel>,
}

/// Collect all configured channels from the config.
pub fn collect_configured_channels(
    config: &Config,
    matrix_skip_context: &str,
) -> Vec<ConfiguredChannel> {
    let _ = matrix_skip_context;
    let mut channels = Vec::new();

    if let Some(ref bridge_cfg) = config.channels_config.bridge {
        channels.push(ConfiguredChannel {
            display_name: "Bridge",
            channel: Arc::new(BridgeChannel::new(bridge_cfg.clone())),
        });
    }

    if let Some(ref tg) = config.channels_config.telegram {
        let mut telegram = TelegramChannel::new(
            tg.bot_token.clone(),
            tg.allowed_users.clone(),
            tg.effective_group_reply_mode().requires_mention(),
        )
        .with_group_reply_allowed_senders(tg.group_reply_allowed_sender_ids())
        .with_streaming(tg.stream_mode, tg.draft_update_interval_ms)
        .with_transcription(config.transcription.clone())
        .with_workspace_dir(config.workspace_dir.clone());

        if let Some(ref base_url) = tg.base_url {
            telegram = telegram.with_api_base(base_url.clone());
        }

        channels.push(ConfiguredChannel {
            display_name: "Telegram",
            channel: Arc::new(telegram),
        });
    }

    if let Some(ref dc) = config.channels_config.discord {
        channels.push(ConfiguredChannel {
            display_name: "Discord",
            channel: Arc::new(
                DiscordChannel::new(
                    dc.bot_token.clone(),
                    dc.guild_id.clone(),
                    dc.allowed_users.clone(),
                    dc.listen_to_bots,
                    dc.effective_group_reply_mode().requires_mention(),
                )
                .with_group_reply_allowed_senders(dc.group_reply_allowed_sender_ids())
                .with_transcription(config.transcription.clone())
                .with_workspace_dir(config.workspace_dir.clone()),
            ),
        });
    }

    if let Some(ref sl) = config.channels_config.slack {
        channels.push(ConfiguredChannel {
            display_name: "Slack",
            channel: Arc::new(
                SlackChannel::new(
                    sl.bot_token.clone(),
                    sl.channel_id.clone(),
                    sl.allowed_users.clone(),
                )
                .with_group_reply_policy(
                    sl.effective_group_reply_mode().requires_mention(),
                    sl.group_reply_allowed_sender_ids(),
                ),
            ),
        });
    }

    if let Some(ref mm) = config.channels_config.mattermost {
        channels.push(ConfiguredChannel {
            display_name: "Mattermost",
            channel: Arc::new(
                MattermostChannel::new(
                    mm.url.clone(),
                    mm.bot_token.clone(),
                    mm.channel_id.clone(),
                    mm.allowed_users.clone(),
                    mm.thread_replies.unwrap_or(true),
                    mm.effective_group_reply_mode().requires_mention(),
                )
                .with_group_reply_allowed_senders(mm.group_reply_allowed_sender_ids()),
            ),
        });
    }

    if let Some(ref im) = config.channels_config.imessage {
        channels.push(ConfiguredChannel {
            display_name: "iMessage",
            channel: Arc::new(IMessageChannel::new(im.allowed_contacts.clone())),
        });
    }

    #[cfg(feature = "channel-matrix")]
    if let Some(ref mx) = config.channels_config.matrix {
        channels.push(ConfiguredChannel {
            display_name: "Matrix",
            channel: Arc::new(
                MatrixChannel::new_with_session_hint_and_topclaw_dir(
                    mx.homeserver.clone(),
                    mx.access_token.clone(),
                    mx.room_id.clone(),
                    mx.allowed_users.clone(),
                    mx.user_id.clone(),
                    mx.device_id.clone(),
                    config.config_path.parent().map(|path| path.to_path_buf()),
                )
                .with_mention_only(mx.mention_only)
                .with_transcription(config.transcription.clone()),
            ),
        });
    }

    #[cfg(not(feature = "channel-matrix"))]
    if config.channels_config.matrix.is_some() {
        tracing::warn!(
            "Matrix channel is configured but this build was compiled without `channel-matrix`; skipping Matrix {}.",
            matrix_skip_context
        );
    }

    if let Some(ref sig) = config.channels_config.signal {
        channels.push(ConfiguredChannel {
            display_name: "Signal",
            channel: Arc::new(SignalChannel::new(
                sig.http_url.clone(),
                sig.account.clone(),
                sig.group_id.clone(),
                sig.allowed_from.clone(),
                sig.ignore_attachments,
                sig.ignore_stories,
            )),
        });
    }

    if let Some(ref wa) = config.channels_config.whatsapp {
        if wa.is_ambiguous_config() {
            tracing::warn!(
                "WhatsApp config has both phone_number_id and session_path set; preferring Cloud API mode. Remove one selector to avoid ambiguity."
            );
        }

        match wa.backend_type() {
            "cloud" => {
                if wa.is_cloud_config() {
                    channels.push(ConfiguredChannel {
                        display_name: "WhatsApp",
                        channel: Arc::new(WhatsAppChannel::new(
                            wa.access_token.clone().unwrap_or_default(),
                            wa.phone_number_id.clone().unwrap_or_default(),
                            wa.verify_token.clone().unwrap_or_default(),
                            wa.allowed_numbers.clone(),
                        )),
                    });
                } else {
                    tracing::warn!(
                        "WhatsApp Cloud API configured but missing required fields (phone_number_id, access_token, verify_token)"
                    );
                }
            }
            "web" => {
                #[cfg(feature = "whatsapp-web")]
                if wa.is_web_config() {
                    channels.push(ConfiguredChannel {
                        display_name: "WhatsApp",
                        channel: Arc::new(WhatsAppWebChannel::new(
                            wa.session_path.clone().unwrap_or_default(),
                            wa.pair_phone.clone(),
                            wa.pair_code.clone(),
                            wa.allowed_numbers.clone(),
                        )),
                    });
                } else {
                    tracing::warn!("WhatsApp Web configured but session_path not set");
                }
                #[cfg(not(feature = "whatsapp-web"))]
                {
                    tracing::warn!(
                        "WhatsApp Web backend requires 'whatsapp-web' feature. Enable with: cargo build --features whatsapp-web"
                    );
                }
            }
            _ => {
                tracing::warn!(
                    "WhatsApp config invalid: neither phone_number_id (Cloud API) nor session_path (Web) is set"
                );
            }
        }
    }

    if let Some(ref lq) = config.channels_config.linq {
        channels.push(ConfiguredChannel {
            display_name: "Linq",
            channel: Arc::new(LinqChannel::new(
                lq.api_token.clone(),
                lq.from_phone.clone(),
                lq.allowed_senders.clone(),
            )),
        });
    }

    if let Some(ref wati_cfg) = config.channels_config.wati {
        channels.push(ConfiguredChannel {
            display_name: "WATI",
            channel: Arc::new(WatiChannel::new(
                wati_cfg.api_token.clone(),
                wati_cfg.api_url.clone(),
                wati_cfg.tenant_id.clone(),
                wati_cfg.allowed_numbers.clone(),
            )),
        });
    }

    if let Some(ref nc) = config.channels_config.nextcloud_talk {
        channels.push(ConfiguredChannel {
            display_name: "Nextcloud Talk",
            channel: Arc::new(NextcloudTalkChannel::new(
                nc.base_url.clone(),
                nc.app_token.clone(),
                nc.allowed_users.clone(),
            )),
        });
    }

    #[cfg(feature = "channel-email")]
    if let Some(ref email_cfg) = config.channels_config.email {
        channels.push(ConfiguredChannel {
            display_name: "Email",
            channel: Arc::new(EmailChannel::new(email_cfg.clone())),
        });
    }

    #[cfg(not(feature = "channel-email"))]
    if config.channels_config.email.is_some() {
        tracing::warn!(
            "Email channel is configured but this build was compiled without `channel-email`; skipping Email startup."
        );
    }

    #[cfg(feature = "channel-irc")]
    if let Some(ref irc) = config.channels_config.irc {
        channels.push(ConfiguredChannel {
            display_name: "IRC",
            channel: Arc::new(IrcChannel::new(super::irc::IrcChannelConfig {
                server: irc.server.clone(),
                port: irc.port,
                nickname: irc.nickname.clone(),
                username: irc.username.clone(),
                channels: irc.channels.clone(),
                allowed_users: irc.allowed_users.clone(),
                server_password: irc.server_password.clone(),
                nickserv_password: irc.nickserv_password.clone(),
                sasl_password: irc.sasl_password.clone(),
                verify_tls: irc.verify_tls.unwrap_or(true),
            })),
        });
    }

    #[cfg(feature = "channel-lark")]
    if let Some(ref lk) = config.channels_config.lark {
        channels.push(ConfiguredChannel {
            display_name: "Lark",
            channel: Arc::new(LarkChannel::from_lark_config(lk)),
        });
    }

    #[cfg(feature = "channel-lark")]
    if let Some(ref fs) = config.channels_config.feishu {
        channels.push(ConfiguredChannel {
            display_name: "Feishu",
            channel: Arc::new(LarkChannel::from_feishu_config(fs)),
        });
    }

    #[cfg(not(feature = "channel-lark"))]
    if config.channels_config.lark.is_some() || config.channels_config.feishu.is_some() {
        tracing::warn!(
            "Lark/Feishu channel is configured but this build was compiled without `channel-lark`; skipping Lark/Feishu health check."
        );
    }

    if let Some(ref dt) = config.channels_config.dingtalk {
        channels.push(ConfiguredChannel {
            display_name: "DingTalk",
            channel: Arc::new(DingTalkChannel::new(
                dt.client_id.clone(),
                dt.client_secret.clone(),
                dt.allowed_users.clone(),
            )),
        });
    }

    if let Some(ref qq) = config.channels_config.qq {
        if qq.receive_mode == crate::config::schema::QQReceiveMode::Webhook {
            tracing::info!(
                "QQ channel configured with receive_mode=webhook; websocket listener startup skipped."
            );
        } else {
            channels.push(ConfiguredChannel {
                display_name: "QQ",
                channel: Arc::new(QQChannel::new(
                    qq.app_id.clone(),
                    qq.app_secret.clone(),
                    qq.allowed_users.clone(),
                )),
            });
        }
    }

    if let Some(ref ct) = config.channels_config.clawdtalk {
        channels.push(ConfiguredChannel {
            display_name: "ClawdTalk",
            channel: Arc::new(ClawdTalkChannel::new(ct.clone())),
        });
    }

    channels
}

/// Append Nostr channel if configured (async initialization).
/// Returns None on success, or Some(error_message) on failure.
pub async fn append_nostr_channel_if_available(
    config: &Config,
    _channels: &mut Vec<ConfiguredChannel>,
    startup_context: &str,
) -> Option<String> {
    #[cfg(not(feature = "channel-nostr"))]
    {
        if config.channels_config.nostr.is_some() {
            let reason = format!(
                "Nostr channel is configured but this build was compiled without `channel-nostr`; skipping Nostr {startup_context}."
            );
            tracing::warn!("{reason}");
            return Some(reason);
        }
        None
    }

    #[cfg(feature = "channel-nostr")]
    let ns = config.channels_config.nostr.as_ref()?;

    #[cfg(feature = "channel-nostr")]
    match NostrChannel::new(&ns.private_key, ns.relays.clone(), &ns.allowed_pubkeys).await {
        Ok(channel) => {
            _channels.push(ConfiguredChannel {
                display_name: "Nostr",
                channel: Arc::new(channel),
            });
            None
        }
        Err(err) => {
            let reason = format!("Nostr init failed during {startup_context}: {err}");
            tracing::warn!("{reason}");
            Some(reason)
        }
    }
}
