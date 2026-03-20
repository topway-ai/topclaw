use super::{ChannelsConfig, GroupReplyConfig, GroupReplyMode, QQReceiveMode, StreamMode};
use crate::config::traits::ChannelConfig;

struct ConfigWrapper<T: ChannelConfig>(std::marker::PhantomData<T>);

impl<T: ChannelConfig> ConfigWrapper<T> {
    fn new(_: Option<&T>) -> Self {
        Self(std::marker::PhantomData)
    }
}

impl<T: ChannelConfig> crate::config::traits::ConfigHandle for ConfigWrapper<T> {
    fn name(&self) -> &'static str {
        T::name()
    }

    fn desc(&self) -> &'static str {
        T::desc()
    }
}

impl ChannelsConfig {
    /// get channels' metadata and `.is_some()`, except webhook
    #[rustfmt::skip]
    pub fn channels_except_webhook(&self) -> Vec<(Box<dyn crate::config::traits::ConfigHandle>, bool)> {
        vec![
            (
                Box::new(ConfigWrapper::new(self.bridge.as_ref())),
                self.bridge.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.telegram.as_ref())),
                self.telegram.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.discord.as_ref())),
                self.discord.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.slack.as_ref())),
                self.slack.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.mattermost.as_ref())),
                self.mattermost.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.imessage.as_ref())),
                self.imessage.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.matrix.as_ref())),
                self.matrix.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.signal.as_ref())),
                self.signal.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.whatsapp.as_ref())),
                self.whatsapp.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.linq.as_ref())),
                self.linq.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.wati.as_ref())),
                self.wati.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.nextcloud_talk.as_ref())),
                self.nextcloud_talk.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.email.as_ref())),
                self.email.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.irc.as_ref())),
                self.irc.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.lark.as_ref())),
                self.lark.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.feishu.as_ref())),
                self.feishu.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.dingtalk.as_ref())),
                self.dingtalk.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.qq.as_ref())),
                self.qq
                    .as_ref()
                    .is_some_and(|qq| qq.receive_mode == QQReceiveMode::Websocket),
            ),
            (
                Box::new(ConfigWrapper::new(self.nostr.as_ref())),
                self.nostr.is_some(),
            ),
            (
                Box::new(ConfigWrapper::new(self.clawdtalk.as_ref())),
                self.clawdtalk.is_some(),
            ),
        ]
    }

    pub fn channels(&self) -> Vec<(Box<dyn crate::config::traits::ConfigHandle>, bool)> {
        let mut ret = self.channels_except_webhook();
        ret.push((
            Box::new(ConfigWrapper::new(self.webhook.as_ref())),
            self.webhook.is_some(),
        ));
        ret
    }
}

impl Default for ChannelsConfig {
    fn default() -> Self {
        Self {
            cli: true,
            bridge: None,
            telegram: None,
            discord: None,
            slack: None,
            mattermost: None,
            webhook: None,
            imessage: None,
            matrix: None,
            signal: None,
            whatsapp: None,
            linq: None,
            wati: None,
            nextcloud_talk: None,
            email: None,
            irc: None,
            lark: None,
            feishu: None,
            dingtalk: None,
            qq: None,
            nostr: None,
            clawdtalk: None,
            message_timeout_secs: default_channel_message_timeout_secs(),
        }
    }
}

pub(crate) fn default_channel_message_timeout_secs() -> u64 {
    300
}

pub(crate) fn default_draft_update_interval_ms() -> u64 {
    500
}

pub(crate) fn default_telegram_stream_mode() -> StreamMode {
    StreamMode::Partial
}

pub(crate) fn resolve_group_reply_mode(
    group_reply: Option<&GroupReplyConfig>,
    default_mode: GroupReplyMode,
) -> GroupReplyMode {
    if let Some(mode) = group_reply.and_then(|cfg| cfg.mode) {
        return mode;
    }
    default_mode
}

pub(crate) fn clone_group_reply_allowed_sender_ids(
    group_reply: Option<&GroupReplyConfig>,
) -> Vec<String> {
    group_reply
        .map(|cfg| cfg.allowed_sender_ids.clone())
        .unwrap_or_default()
}

pub(crate) fn default_wati_api_url() -> String {
    "https://live-mt-server.wati.io".to_string()
}

pub(crate) fn default_irc_port() -> u16 {
    6697
}

pub fn default_lark_draft_update_interval_ms() -> u64 {
    3000
}

pub fn default_lark_max_draft_edits() -> u32 {
    20
}

pub fn default_nostr_relays() -> Vec<String> {
    vec![
        "wss://relay.damus.io".to_string(),
        "wss://nos.lol".to_string(),
        "wss://relay.primal.net".to_string(),
        "wss://relay.snort.social".to_string(),
    ]
}

pub fn default_imap_port() -> u16 {
    993
}

pub fn default_imap_folder() -> String {
    "INBOX".to_string()
}

pub fn default_smtp_port() -> u16 {
    465
}

pub fn default_idle_timeout() -> u64 {
    1740
}
