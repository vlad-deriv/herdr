use std::io;

use tracing::{debug, warn};

use crate::protocol::NotifyKind;

use super::shell;

pub(super) fn handle_shell_notification_effects(
    effects: Vec<shell::ClientShellNotificationEffect>,
    sound_config: &crate::config::SoundConfig,
    #[cfg(windows)] event_tx: &tokio::sync::mpsc::Sender<super::events::ClientLoopEvent>,
) {
    for effect in effects {
        match effect {
            shell::ClientShellNotificationEffect::Sound { sound, agent } => {
                let agent = agent.as_deref().and_then(crate::detect::parse_agent_label);
                if sound_config.allows(agent) {
                    crate::sound::play(sound, sound_config);
                }
            }
            shell::ClientShellNotificationEffect::Terminal { title, body } => {
                if let Err(err) = crate::terminal_notify::show_notification(&title, body.as_deref())
                {
                    warn!(err = %err, "failed to emit terminal notification");
                }
            }
            shell::ClientShellNotificationEffect::System {
                title,
                body,
                #[cfg(windows)]
                target,
            } => {
                #[cfg(windows)]
                let result = show_system_notification(&title, body.as_deref(), target, event_tx);
                #[cfg(not(windows))]
                let result = crate::platform::show_desktop_notification(&title, body.as_deref());
                if let Err(err) = result {
                    warn!(err = %err, "failed to emit system notification");
                }
            }
        }
    }
}

#[cfg(windows)]
fn show_system_notification(
    title: &str,
    body: Option<&str>,
    target: Option<shell::ClientSystemNotificationTarget>,
    event_tx: &tokio::sync::mpsc::Sender<super::events::ClientLoopEvent>,
) -> io::Result<bool> {
    let Some(target) = target else {
        return crate::platform::show_desktop_notification(title, body);
    };
    let key = serde_json::to_string(&(
        &target.endpoint_id.storage_key(),
        &target.boot_id,
        &target.pane_id,
    ))
    .map_err(io::Error::other)?;
    let event_tx = event_tx.clone();
    let runtime = tokio::runtime::Handle::current();
    crate::platform::show_actionable_desktop_notification(
        title,
        body,
        key,
        std::sync::Arc::new(move || {
            let event_tx = event_tx.clone();
            let target = target.clone();
            // Never block a native callback: WinRT can dispatch it while Show is awaited.
            runtime.spawn(async move {
                let _ = event_tx
                    .send(super::events::ClientLoopEvent::NotificationActivated(
                        target,
                    ))
                    .await;
            });
        }),
    )
}

pub(super) fn handle_notify(
    kind: NotifyKind,
    message: &str,
    body: Option<&str>,
    sound_config: &crate::config::SoundConfig,
) {
    handle_notify_with_notifiers(
        kind,
        message,
        body,
        sound_config,
        crate::terminal_notify::show_notification,
        crate::platform::show_desktop_notification,
    );
}

pub(super) fn handle_notify_with_notifiers(
    kind: NotifyKind,
    message: &str,
    body: Option<&str>,
    sound_config: &crate::config::SoundConfig,
    mut show_terminal_notification: impl FnMut(&str, Option<&str>) -> io::Result<bool>,
    mut show_system_notification: impl FnMut(&str, Option<&str>) -> io::Result<bool>,
) {
    match kind {
        NotifyKind::Sound => {
            let Some(sound) = sound_from_notify_message(message) else {
                warn!(
                    message = message,
                    "received unknown sound notification from server"
                );
                return;
            };
            if sound_config.enabled {
                crate::sound::play(sound, sound_config);
            }
        }
        NotifyKind::Toast => {
            debug!(
                message = message,
                "received terminal toast notification from server"
            );
            if let Err(err) = show_terminal_notification(message, body) {
                warn!(err = %err, "failed to emit terminal notification");
            }
        }
        NotifyKind::SystemToast => {
            debug!(
                message = message,
                "received system toast notification from server"
            );
            if let Err(err) = show_system_notification(message, body) {
                warn!(err = %err, "failed to emit system notification");
            }
        }
    }
}

pub(super) fn sound_from_notify_message(message: &str) -> Option<crate::sound::Sound> {
    match message {
        "agent done" => Some(crate::sound::Sound::Done),
        "agent attention" => Some(crate::sound::Sound::Request),
        _ => None,
    }
}
