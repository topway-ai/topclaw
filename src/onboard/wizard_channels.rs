use super::ChannelsConfig;
use std::sync::LazyLock;

#[allow(dead_code)]
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum ChannelMenuChoice {
    Telegram,
    Discord,
    OtherChannels,
    Webhook,
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

    choices.push(ChannelMenuChoice::Webhook);
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
        ChannelMenuChoice::OtherChannels => "Advanced/gateway channels... — Webhook".to_string(),
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
        ChannelMenuChoice::Webhook => format!(
            "Webhook    {}",
            if config.webhook.is_some() {
                "✅ configured"
            } else {
                "— HTTP endpoint"
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
    if config
        .launchable_channels()
        .iter()
        .any(|(_, configured)| *configured)
    {
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
    let choices = other_channel_menu_choices();
    choices
        .iter()
        .position(|choice| {
            !matches!(choice, ChannelMenuChoice::Back)
                && !channel_choice_is_configured(config, *choice)
        })
        .unwrap_or_else(|| {
            // All non-Back options are configured — default to "Back"
            choices
                .iter()
                .position(|choice| matches!(choice, ChannelMenuChoice::Back))
                .unwrap_or(0)
        })
}

pub(super) fn channel_choice_is_configured(
    config: &ChannelsConfig,
    choice: ChannelMenuChoice,
) -> bool {
    match choice {
        ChannelMenuChoice::Telegram => config.telegram.is_some(),
        ChannelMenuChoice::Discord => config.discord.is_some(),
        ChannelMenuChoice::Webhook => config.webhook.is_some(),
        ChannelMenuChoice::OtherChannels | ChannelMenuChoice::Done | ChannelMenuChoice::Back => {
            false
        }
    }
}
