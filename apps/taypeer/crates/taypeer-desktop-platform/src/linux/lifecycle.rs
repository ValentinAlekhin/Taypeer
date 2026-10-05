//! logind and the optional desktop ScreenSaver service are independent lock sources.
use super::proxies::{
    ActiveChanged, Lock, LoginManagerProxy, LoginSessionProxy, PrepareForSleep, ScreenSaverProxy,
    Unlock,
};
use crate::state::EventSource;
use std::sync::{Arc, Mutex};
use taypeer_runtime::session::LockReason;
use zbus::{
    MatchRule,
    blocking::{Connection, MessageIterator, fdo::DBusProxy},
    fdo::{NameOwnerChanged, PropertiesChanged},
    message::Type,
    proxy::CacheProperties,
};

pub(super) struct Monitor {
    connections: Vec<Connection>,
}
#[derive(Default)]
struct State {
    sleeping: bool,
    locked_hint: bool,
    explicit_lock: bool,
    session_active: bool,
    screensaver: bool,
    lost: bool,
}
impl State {
    fn publish(&self, shared: &EventSource) {
        if self.lost {
            shared.suspend(LockReason::HostExited);
        } else if self.sleeping {
            shared.suspend(LockReason::Sleep);
        } else if self.locked_hint || self.explicit_lock || !self.session_active || self.screensaver
        {
            shared.suspend(LockReason::SystemLocked);
        } else {
            shared.active();
        }
    }
}
impl Monitor {
    pub(super) fn start(shared: Arc<EventSource>) -> zbus::Result<Self> {
        let system = Connection::system()?;
        let mut monitor = Self {
            connections: vec![system.clone()],
        };
        let state = Arc::new(Mutex::new(State::default()));
        let rule = MatchRule::builder().msg_type(Type::Signal).build();
        let signals = MessageIterator::for_match_rule(rule.clone(), &system, Some(256))?;
        let manager = LoginManagerProxy::builder(&system)
            .cache_properties(CacheProperties::No)
            .build()?;
        let path: zbus::zvariant::OwnedObjectPath =
            match manager.get_session_by_pid(std::process::id()) {
                Ok(path) => path,
                Err(zbus::Error::MethodError(name, _, _))
                    if name.as_str() == "org.freedesktop.login1.NoSessionForPID" =>
                {
                    // Desktop launchers often use user systemd scopes outside the login PID tree.
                    // logind's auto session is the calling user's display session, not an env-provided ID.
                    manager.get_session("auto")?
                }
                Err(error) => return Err(error),
            };
        let session = session_proxy(&system, &path)?;
        let owner =
            DBusProxy::new(&system)?.get_name_owner("org.freedesktop.login1".try_into()?)?;
        {
            let mut current = state
                .lock()
                .map_err(|_| zbus::Error::Failure("lifecycle state unavailable".into()))?;
            current.sleeping = manager.preparing_for_sleep()?;
            current.locked_hint = session.locked_hint()?;
            current.session_active = session.active()?;
        }
        let system_connection = system.clone();
        let system_state = Arc::clone(&state);
        let system_shared = Arc::clone(&shared);
        std::thread::spawn(move || {
            for message in signals {
                let Ok(message) = message else { break };
                let header = message.header();
                let member = header.member().map(|value| value.as_str()).unwrap_or("");
                let sender = header.sender().map(|value| value.as_str()).unwrap_or("");
                let mut current = match system_state.lock() {
                    Ok(state) => state,
                    Err(_) => break,
                };
                if member == "NameOwnerChanged" && sender == "org.freedesktop.DBus" {
                    if let Some(signal) = NameOwnerChanged::from_message(message.clone())
                        && let Ok(args) = signal.args()
                        && args.name().as_str() == "org.freedesktop.login1"
                    {
                        current.lost = true;
                        current.publish(&system_shared);
                    }
                    continue;
                }
                if sender != owner.as_str() {
                    continue;
                }
                match member {
                    "PrepareForSleep" => {
                        let Some(signal) = PrepareForSleep::from_message(message.clone()) else {
                            continue;
                        };
                        let Ok(args) = signal.args() else {
                            break;
                        };
                        let sleeping = args.sleeping;
                        if sleeping {
                            current.sleeping = true;
                        } else {
                            // Native calls cannot hold the shared state: an independent lock source
                            // must be able to revoke access even while logind is unresponsive.
                            drop(current);
                            let confirmed = confirmed_session(&system_connection, &path);
                            current = match system_state.lock() {
                                Ok(state) => state,
                                Err(_) => break,
                            };
                            current.sleeping = false;
                            match confirmed {
                                Ok((locked, active)) => {
                                    current.locked_hint = locked;
                                    current.session_active = active;
                                }
                                Err(_) => current.lost = true,
                            }
                        }
                    }
                    "Lock"
                        if header
                            .path()
                            .is_some_and(|value| value.as_str() == path.as_str())
                            && Lock::from_message(message.clone()).is_some() =>
                    {
                        current.explicit_lock = true
                    }
                    "Unlock"
                        if header
                            .path()
                            .is_some_and(|value| value.as_str() == path.as_str())
                            && Unlock::from_message(message.clone()).is_some() =>
                    {
                        drop(current);
                        system_shared.suspend(LockReason::SystemLocked);
                        let confirmed = confirmed_session(&system_connection, &path);
                        current = match system_state.lock() {
                            Ok(state) => state,
                            Err(_) => break,
                        };
                        match confirmed {
                            Ok((locked, active)) => {
                                current.locked_hint = locked;
                                current.session_active = active;
                                current.explicit_lock = false;
                            }
                            Err(_) => current.lost = true,
                        }
                    }
                    "PropertiesChanged"
                        if header
                            .path()
                            .is_some_and(|value| value.as_str() == path.as_str()) =>
                    {
                        let Some(signal) = PropertiesChanged::from_message(message.clone()) else {
                            continue;
                        };
                        let Ok(args) = signal.args() else {
                            break;
                        };
                        if args.interface_name().as_str() != "org.freedesktop.login1.Session" {
                            continue;
                        }
                        let locked_changed = args.changed_properties().contains_key("LockedHint")
                            || args.invalidated_properties().contains(&"LockedHint");
                        let active_changed = args.changed_properties().contains_key("Active")
                            || args.invalidated_properties().contains(&"Active");
                        if !locked_changed && !active_changed {
                            continue;
                        }
                        drop(current);
                        system_shared.suspend(LockReason::SystemLocked);
                        let confirmed = confirmed_session(&system_connection, &path);
                        current = match system_state.lock() {
                            Ok(state) => state,
                            Err(_) => break,
                        };
                        match confirmed {
                            Ok((locked, active)) => {
                                current.locked_hint = locked;
                                current.session_active = active;
                                if locked_changed {
                                    current.explicit_lock = false;
                                }
                            }
                            Err(_) => current.lost = true,
                        }
                    }
                    _ => continue,
                }
                current.publish(&system_shared);
            }
            system_shared.suspend(LockReason::HostExited);
        });

        // A missing ScreenSaver service is normal for compositors relying on logind.
        // Once subscribed, loss of that source is fail-closed, like loss of logind.
        let session_bus = Connection::session()?;
        let screensaver_owner = match DBusProxy::new(&session_bus)?
            .get_name_owner("org.freedesktop.ScreenSaver".try_into()?)
        {
            Ok(owner) => Some(owner),
            Err(zbus::fdo::Error::NameHasNoOwner(_)) => None,
            Err(error) => return Err(error.into()),
        };
        if let Some(owner) = screensaver_owner {
            monitor.connections.push(session_bus.clone());
            let signals = MessageIterator::for_match_rule(rule, &session_bus, Some(128))?;
            if let Some(active) = screensaver_active(&session_bus)? {
                state
                    .lock()
                    .map_err(|_| zbus::Error::Failure("lifecycle state unavailable".into()))?
                    .screensaver = active;
                let saver_state = Arc::clone(&state);
                let saver_shared = Arc::clone(&shared);
                std::thread::spawn(move || {
                    for message in signals {
                        let Ok(message) = message else { break };
                        let header = message.header();
                        let member = header.member().map(|value| value.as_str()).unwrap_or("");
                        let sender = header.sender().map(|value| value.as_str()).unwrap_or("");
                        let mut current = match saver_state.lock() {
                            Ok(state) => state,
                            Err(_) => break,
                        };
                        if sender == owner.as_str() && member == "ActiveChanged" {
                            let Some(signal) = ActiveChanged::from_message(message.clone()) else {
                                continue;
                            };
                            let Ok(args) = signal.args() else {
                                break;
                            };
                            current.screensaver = args.active;
                            current.publish(&saver_shared);
                        } else if sender == "org.freedesktop.DBus"
                            && member == "NameOwnerChanged"
                            && let Some(signal) = NameOwnerChanged::from_message(message.clone())
                            && let Ok(args) = signal.args()
                            && args.name().as_str() == "org.freedesktop.ScreenSaver"
                        {
                            current.lost = true;
                            current.publish(&saver_shared);
                        }
                    }
                    saver_shared.suspend(LockReason::HostExited);
                });
            }
        }
        state
            .lock()
            .map_err(|_| zbus::Error::Failure("lifecycle state unavailable".into()))?
            .publish(&shared);
        Ok(monitor)
    }
}
fn screensaver_active(connection: &Connection) -> zbus::Result<Option<bool>> {
    for path in ["/org/freedesktop/ScreenSaver", "/ScreenSaver"] {
        let proxy = ScreenSaverProxy::builder(connection).path(path)?.build()?;
        match proxy.get_active() {
            Ok(active) => return Ok(Some(active)),
            Err(zbus::Error::MethodError(name, _, _))
                if matches!(
                    name.as_str(),
                    "org.freedesktop.DBus.Error.UnknownMethod"
                        | "org.freedesktop.DBus.Error.UnknownObject"
                ) =>
            {
                continue;
            }
            Err(error) => return Err(error),
        }
    }
    // Some desktops own this name only for Inhibit/UnInhibit. They are not lifecycle sources.
    Ok(None)
}
fn session_proxy<'a>(
    connection: &'a Connection,
    path: &zbus::zvariant::OwnedObjectPath,
) -> zbus::Result<LoginSessionProxy<'a>> {
    // Every resume confirmation must read logind again, rather than a cached
    // property from before an independent source suspended interaction.
    LoginSessionProxy::builder(connection)
        .path(path.clone())?
        .cache_properties(CacheProperties::No)
        .build()
}
fn confirmed_session(
    connection: &Connection,
    path: &zbus::zvariant::OwnedObjectPath,
) -> zbus::Result<(bool, bool)> {
    let proxy = session_proxy(connection, path)?;
    let locked = proxy.locked_hint()?;
    let active = proxy.active()?;
    Ok((locked, active))
}
impl Drop for Monitor {
    fn drop(&mut self) {
        for connection in self.connections.drain(..) {
            // Closing wakes the blocked iterators; their EOF path also revokes access.
            let _ = connection.close();
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    use crate::state::PlatformState;
    #[test]
    fn confirmed_vt_return_permits_authentication_but_retains_explicit_lock() {
        let platform = PlatformState::new(true);
        let shared = platform.source();
        let mut state = State {
            session_active: true,
            ..State::default()
        };
        state.session_active = false;
        state.publish(&shared);
        assert!(!shared.available());
        state.session_active = true;
        state.publish(&shared);
        assert!(shared.available());
        state.explicit_lock = true;
        state.session_active = false;
        state.publish(&shared);
        state.session_active = true;
        state.publish(&shared);
        assert!(!shared.available());
        state.explicit_lock = false;
        state.publish(&shared);
        assert!(shared.available());
        shared.suspend(LockReason::HostExited);
        state.publish(&shared);
        assert!(!shared.available());
    }
    #[test]
    fn resume_does_not_override_an_independent_lock_or_lost_source() {
        let platform = PlatformState::new(false);
        let shared = platform.source();
        let mut state = State {
            sleeping: true,
            explicit_lock: true,
            session_active: true,
            ..State::default()
        };
        state.publish(&shared);
        state.sleeping = false;
        state.publish(&shared);
        assert!(!shared.available());
        state.explicit_lock = false;
        state.screensaver = true;
        state.publish(&shared);
        assert!(!shared.available());
        state.screensaver = false;
        state.publish(&shared);
        assert!(shared.available());
        state.lost = true;
        state.publish(&shared);
        assert!(!shared.available());
        assert_eq!(platform.drain().len(), 5);
    }
}
