use super::ChannelsConfig;
use std::sync::LazyLock;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChannelMenuChoice {
    Telegram,
    Discord,
    OtherChannels,
    Slack,
    IMessage,
    Matrix,
    Signal,
    WhatsApp,
    Linq,
    Irc,
    Webhook,
    NextcloudTalk,
    DingTalk,
    QqOfficial,
    LarkFeishu,
    Nostr,
    Done,
    Back,
}

#[allow(clippy::vec_init_then_push)]
fn build_channel_menu_choices() -> Box<[ChannelMenuChoice]> {
    let mut choices = Vec::new();

    #[cfg(feature = "channel-telegram")]
    choices.push(ChannelMenuChoice::Telegram);
    #[cfg(feature = "channel-discord")]
    choices.push(ChannelMenuChoice::Discord);
    choices.push(ChannelMenuChoice::OtherChannels);
    choices.push(ChannelMenuChoice::Done);

    choices.into_boxed_slice()
}

#[allow(clippy::vec_init_then_push)]
fn build_other_channel_menu_choices() -> Box<[ChannelMenuChoice]> {
    let mut choices = Vec::new();

    #[cfg(feature = "channel-slack")]
    choices.push(ChannelMenuChoice::Slack);
    #[cfg(feature = "channel-imessage")]
    choices.push(ChannelMenuChoice::IMessage);
    #[cfg(feature = "channel-matrix")]
    choices.push(ChannelMenuChoice::Matrix);
    #[cfg(feature = "channel-signal")]
    choices.push(ChannelMenuChoice::Signal);
    #[cfg(feature = "channel-whatsapp")]
    choices.push(ChannelMenuChoice::WhatsApp);
    #[cfg(feature = "channel-linq")]
    choices.push(ChannelMenuChoice::Linq);
    #[cfg(feature = "channel-irc")]
    choices.push(ChannelMenuChoice::Irc);
    choices.push(ChannelMenuChoice::Webhook);
    #[cfg(feature = "channel-nextcloud-talk")]
    choices.push(ChannelMenuChoice::NextcloudTalk);
    #[cfg(feature = "channel-dingtalk")]
    choices.push(ChannelMenuChoice::DingTalk);
    #[cfg(feature = "channel-qq")]
    choices.push(ChannelMenuChoice::QqOfficial);
    #[cfg(feature = "channel-lark")]
    choices.push(ChannelMenuChoice::LarkFeishu);
    #[cfg(feature = "channel-nostr")]
    choices.push(ChannelMenuChoice::Nostr);
    choices.push(ChannelMenuChoice::Back);

    choices.into_boxed_slice()
}

static CHANNEL_MENU_CHOICES: LazyLock<Box<[ChannelMenuChoice]>> =
    LazyLock::new(build_channel_menu_choices);
static OTHER_CHANNEL_MENU_CHOICES: LazyLock<Box<[ChannelMenuChoice]>> =
    LazyLock::new(build_other_channel_menu_choices);

pub(super) fn channel_menu_choices() -> &'static [ChannelMenuChoice] {
    &CHANNEL_MENU_CHOICES
}

pub(super) fn other_channel_menu_choices() -> &'static [ChannelMenuChoice] {
    &OTHER_CHANNEL_MENU_CHOICES
}

fn channel_menu_option_label(config: &ChannelsConfig, choice: ChannelMenuChoice) -> String {
    match choice {
        ChannelMenuChoice::OtherChannels => {
            "Other channels... — Webhook, Lark/Feishu, and more".to_string()
        }
        ChannelMenuChoice::Done => "Done — finish setup".to_string(),
        ChannelMenuChoice::Back => "Back — recommended channels".to_string(),
        ChannelMenuChoice::Telegram => format!(
            "Telegram   {}",
            if config.telegram.is_some() {
                "✅ connected"
            } else {
                "— connect your bot"
            }
        ),
        ChannelMenuChoice::Discord => format!(
            "Discord    {}",
            if config.discord.is_some() {
                "✅ connected"
            } else {
                "— connect your bot"
            }
        ),
        ChannelMenuChoice::Slack => format!(
            "Slack      {}",
            if config.slack.is_some() {
                "✅ connected"
            } else {
                "— connect your bot"
            }
        ),
        ChannelMenuChoice::IMessage => format!(
            "iMessage   {}",
            if config.imessage.is_some() {
                "✅ configured"
            } else {
                "— macOS only"
            }
        ),
        ChannelMenuChoice::Matrix => format!(
            "Matrix     {}",
            if config.matrix.is_some() {
                "✅ connected"
            } else {
                "— self-hosted chat"
            }
        ),
        ChannelMenuChoice::Signal => format!(
            "Signal     {}",
            if config.signal.is_some() {
                "✅ connected"
            } else {
                "— signal-cli daemon bridge"
            }
        ),
        ChannelMenuChoice::WhatsApp => format!(
            "WhatsApp   {}",
            if config.whatsapp.is_some() {
                "✅ connected"
            } else {
                "— Business Cloud API"
            }
        ),
        ChannelMenuChoice::Linq => format!(
            "Linq       {}",
            if config.linq.is_some() {
                "✅ connected"
            } else {
                "— iMessage/RCS/SMS via Linq API"
            }
        ),
        ChannelMenuChoice::Irc => format!(
            "IRC        {}",
            if config.irc.is_some() {
                "✅ configured"
            } else {
                "— IRC over TLS"
            }
        ),
        ChannelMenuChoice::Webhook => format!(
            "Webhook    {}",
            if config.webhook.is_some() {
                "✅ configured"
            } else {
                "— HTTP endpoint"
            }
        ),
        ChannelMenuChoice::NextcloudTalk => format!(
            "Nextcloud  {}",
            if config.nextcloud_talk.is_some() {
                "✅ connected"
            } else {
                "— Talk webhook + OCS API"
            }
        ),
        ChannelMenuChoice::DingTalk => format!(
            "DingTalk   {}",
            if config.dingtalk.is_some() {
                "✅ connected"
            } else {
                "— DingTalk Stream Mode"
            }
        ),
        ChannelMenuChoice::QqOfficial => format!(
            "QQ Official {}",
            if config.qq.is_some() {
                "✅ connected"
            } else {
                "— Tencent QQ Bot"
            }
        ),
        ChannelMenuChoice::LarkFeishu => format!(
            "Lark/Feishu {}",
            if config.lark.is_some() {
                "✅ connected"
            } else {
                "— Lark/Feishu Bot"
            }
        ),
        ChannelMenuChoice::Nostr => format!(
            "Nostr {}",
            if config.nostr.is_some() {
                "✅ connected"
            } else {
                "     — Nostr DMs"
            }
        ),
    }
}

pub(super) fn channel_menu_option_labels(config: &ChannelsConfig) -> Vec<String> {
    channel_menu_choices()
        .iter()
        .map(|choice| channel_menu_option_label(config, *choice))
        .collect()
}

pub(super) fn other_channel_menu_option_labels(config: &ChannelsConfig) -> Vec<String> {
    other_channel_menu_choices()
        .iter()
        .map(|choice| channel_menu_option_label(config, *choice))
        .collect()
}

pub(super) fn default_channel_menu_index(config: &ChannelsConfig) -> usize {
    if config.channels().iter().any(|(_, configured)| *configured) {
        return channel_menu_choices()
            .iter()
            .position(|choice| matches!(choice, ChannelMenuChoice::Done))
            .unwrap_or(0);
    }

    channel_menu_choices()
        .iter()
        .position(|choice| matches!(choice, ChannelMenuChoice::Telegram))
        .or_else(|| {
            channel_menu_choices()
                .iter()
                .position(|choice| !matches!(choice, ChannelMenuChoice::Done))
        })
        .unwrap_or(0)
}

pub(super) fn default_other_channel_menu_index(config: &ChannelsConfig) -> usize {
    other_channel_menu_choices()
        .iter()
        .position(|choice| {
            !matches!(choice, ChannelMenuChoice::Back)
                && !channel_choice_is_configured(config, *choice)
        })
        .unwrap_or(0)
}

pub(super) fn channel_choice_is_configured(
    config: &ChannelsConfig,
    choice: ChannelMenuChoice,
) -> bool {
    match choice {
        #[cfg(feature = "channel-telegram")]
        ChannelMenuChoice::Telegram => config.telegram.is_some(),
        #[cfg(feature = "channel-discord")]
        ChannelMenuChoice::Discord => config.discord.is_some(),
        ChannelMenuChoice::OtherChannels => false,
        #[cfg(feature = "channel-slack")]
        ChannelMenuChoice::Slack => config.slack.is_some(),
        #[cfg(feature = "channel-imessage")]
        ChannelMenuChoice::IMessage => config.imessage.is_some(),
        #[cfg(feature = "channel-matrix")]
        ChannelMenuChoice::Matrix => config.matrix.is_some(),
        #[cfg(feature = "channel-signal")]
        ChannelMenuChoice::Signal => config.signal.is_some(),
        #[cfg(feature = "channel-whatsapp")]
        ChannelMenuChoice::WhatsApp => config.whatsapp.is_some(),
        #[cfg(feature = "channel-linq")]
        ChannelMenuChoice::Linq => config.linq.is_some(),
        #[cfg(feature = "channel-irc")]
        ChannelMenuChoice::Irc => config.irc.is_some(),
        ChannelMenuChoice::Webhook => config.webhook.is_some(),
        #[cfg(feature = "channel-nextcloud-talk")]
        ChannelMenuChoice::NextcloudTalk => config.nextcloud_talk.is_some(),
        #[cfg(feature = "channel-dingtalk")]
        ChannelMenuChoice::DingTalk => config.dingtalk.is_some(),
        #[cfg(feature = "channel-qq")]
        ChannelMenuChoice::QqOfficial => config.qq.is_some(),
        #[cfg(feature = "channel-lark")]
        ChannelMenuChoice::LarkFeishu => config.lark.is_some() || config.feishu.is_some(),
        #[cfg(feature = "channel-nostr")]
        ChannelMenuChoice::Nostr => config.nostr.is_some(),
        ChannelMenuChoice::Done => false,
        ChannelMenuChoice::Back => false,
        _ => false,
    }
}
