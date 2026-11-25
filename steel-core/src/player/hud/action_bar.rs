//! Keeps an owned action bar visible through the vanilla overlay timeout.

use std::time::{Duration, Instant};

use text_components::TextComponent;

/// Resend before the vanilla action bar starts fading.
const REFRESH_INTERVAL: Duration = Duration::from_secs(1);
/// Vanilla keeps an overlay message for 60 client ticks.
const TEMPORARY_OVERLAY_DURATION: Duration = Duration::from_secs(3);

#[derive(Debug, Default)]
pub(super) struct ActionBar {
    pub(super) desired: Option<TextComponent>,
    last_sent: Option<(TextComponent, Instant)>,
    temporary_until: Option<Instant>,
}

impl ActionBar {
    pub(super) fn defer(&mut self, now: Instant) {
        self.temporary_until = Some(now + TEMPORARY_OVERLAY_DURATION);
        self.last_sent = None;
    }

    pub(super) fn update(&mut self, now: Instant) -> Option<TextComponent> {
        if self.temporary_until.is_some_and(|until| now < until) {
            return None;
        }
        self.temporary_until = None;
        let Some(desired) = &self.desired else {
            return self.last_sent.take().map(|_| TextComponent::plain(""));
        };
        if self.last_sent.as_ref().is_some_and(|(text, sent_at)| {
            text == desired && now.duration_since(*sent_at) < REFRESH_INTERVAL
        }) {
            return None;
        }
        self.last_sent = Some((desired.clone(), now));
        Some(desired.clone())
    }
}
