//! Relay reconfiguration belongs to exchange, with durable settings and rollback.
use taypeer_runtime_client::{Backend, Ticket};
use taypeer_settings_ui::local_settings::LocalSettings;
/// Change the running endpoint and persist settings; restore the old endpoint on failure.
pub fn configure_relay(backend: &Backend, settings: LocalSettings) -> Ticket<LocalSettings> {
    let profile = backend.profile.clone();
    backend.with_network(move |host| {
        let old = LocalSettings::load(&profile)?;
        let next = settings.relay.setting()?;
        let running = host
            .as_ref()
            .is_some_and(|host| host.network_address().is_ok());
        let changed = (|| {
            if running && let Some(host) = &host {
                host.stop_network();
                host.start_network(next)?;
            }
            settings.save(&profile)?;
            Ok(settings)
        })();
        if changed.is_err()
            && running
            && let Some(host) = host
        {
            host.stop_network();
            // A failure to restore connectivity is visible in the subsequent stopped snapshot.
            let _ = host.start_network(old.relay.setting()?);
        }
        changed
    })
}
