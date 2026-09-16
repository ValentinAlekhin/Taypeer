//! Device command boundary consumed by the settings screen.
use crate::local_settings::{LocalSettings, RelayPreference};
use gpui_kit::*;
use taypeer_ui::forms::Done;
/// Commands supplied by the composing owner; callbacks complete after durable persistence.
pub trait SettingsHost: Sized + 'static {
    /// Current device preferences, with no secrets.
    fn local<'a>(&self, cx: &'a App) -> &'a LocalSettings;
    /// Current inactivity interval in seconds.
    fn idle_seconds(&self, cx: &App) -> u32;
    /// Whether another settings operation is pending.
    fn settings_busy(&self, cx: &App) -> bool;
    /// Whether relay reconfiguration would overlap another exchange.
    fn exchange_busy(&self, cx: &App) -> bool;
    /// Persist a new inactivity interval.
    fn set_idle(&mut self, seconds: u32, cx: &mut Context<Self>);
    /// Persist nonsensitive device settings.
    fn save_local(&mut self, settings: LocalSettings, cx: &mut Context<Self>);
    /// Complete a device-settings form after persistence.
    fn save_local_form(
        &mut self,
        settings: LocalSettings,
        done: Done,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    /// Complete an inactivity form after persistence.
    fn set_idle_form(
        &mut self,
        seconds: u32,
        done: Done,
        window: &mut Window,
        cx: &mut Context<Self>,
    );
    /// Reconfigure relay with rollback on failure.
    fn set_relay(&mut self, relay: RelayPreference, done: Option<Done>, cx: &mut Context<Self>);
    /// Request return from settings to the previous destination.
    fn settings(&mut self, cx: &mut Context<Self>);
}
