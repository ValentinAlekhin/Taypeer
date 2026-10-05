//! logind and the optional desktop ScreenSaver service are independent lock sources.
use super::Shared;
use std::sync::{Arc, Mutex};
use taypeer_runtime::session::LockReason;
use zbus::{
    MatchRule,
    blocking::{Connection, MessageIterator, Proxy},
    message::Type,
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
    fn publish(&self, shared: &Shared) {
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
    pub(super) fn start(shared: Arc<Shared>) -> zbus::Result<Self> {
        let system = Connection::system()?;
        let mut monitor = Self {
            connections: vec![system.clone()],
        };
        let state = Arc::new(Mutex::new(State::default()));
        let rule = MatchRule::builder().msg_type(Type::Signal).build();
        let signals = MessageIterator::for_match_rule(rule.clone(), &system, Some(256))?;
        let manager = Proxy::new(
            &system,
            "org.freedesktop.login1",
            "/org/freedesktop/login1",
            "org.freedesktop.login1.Manager",
        )?;
        let path: zbus::zvariant::OwnedObjectPath =
            match manager.call("GetSessionByPID", &(std::process::id(),)) {
                Ok(path) => path,
                Err(zbus::Error::MethodError(name, _, _))
                    if name.as_str() == "org.freedesktop.login1.NoSessionForPID" =>
                {
                    // Desktop launchers often use user systemd scopes outside the login PID tree.
                    // logind's auto session is the calling user's display session, not an env-provided ID.
                    manager.call("GetSession", &("auto",))?
                }
                Err(error) => return Err(error),
            };
        let session = Proxy::new(
            &system,
            "org.freedesktop.login1",
            path.clone(),
            "org.freedesktop.login1.Session",
        )?;
        let owner = name_owner(&system, "org.freedesktop.login1")?;
        {
            let mut current = state
                .lock()
                .map_err(|_| zbus::Error::Failure("lifecycle state unavailable".into()))?;
            current.sleeping = manager.get_property("PreparingForSleep")?;
            current.locked_hint = session.get_property("LockedHint")?;
            current.session_active = session.get_property("Active")?;
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
                    if let Ok((name, _, _)) =
                        message.body().deserialize::<(String, String, String)>()
                        && name == "org.freedesktop.login1"
                    {
                        current.lost = true;
                        current.publish(&system_shared);
                    }
                    continue;
                }
                if sender != owner {
                    continue;
                }
                match member {
                    "PrepareForSleep" => {
                        let Ok((sleeping,)) = message.body().deserialize::<(bool,)>() else {
                            break;
                        };
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
                            .is_some_and(|value| value.as_str() == path.as_str()) =>
                    {
                        current.explicit_lock = true
                    }
                    "Unlock"
                        if header
                            .path()
                            .is_some_and(|value| value.as_str() == path.as_str()) =>
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
                        let Ok((interface, changed, invalidated)) = message.body().deserialize::<(
                            String,
                            std::collections::HashMap<String, zbus::zvariant::OwnedValue>,
                            Vec<String>,
                        )>(
                        ) else {
                            break;
                        };
                        if interface != "org.freedesktop.login1.Session" {
                            continue;
                        }
                        let locked_changed = changed.contains_key("LockedHint")
                            || invalidated.iter().any(|name| name == "LockedHint");
                        let active_changed = changed.contains_key("Active")
                            || invalidated.iter().any(|name| name == "Active");
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
        let screensaver_owner = match name_owner(&session_bus, "org.freedesktop.ScreenSaver") {
            Ok(owner) => Some(owner),
            Err(zbus::Error::MethodError(name, _, _))
                if name.as_str() == "org.freedesktop.DBus.Error.NameHasNoOwner" =>
            {
                None
            }
            Err(error) => return Err(error),
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
                        if sender == owner && member == "ActiveChanged" {
                            let Ok((active,)) = message.body().deserialize::<(bool,)>() else {
                                break;
                            };
                            current.screensaver = active;
                            current.publish(&saver_shared);
                        } else if sender == "org.freedesktop.DBus"
                            && member == "NameOwnerChanged"
                            && let Ok((name, _, _)) =
                                message.body().deserialize::<(String, String, String)>()
                            && name == "org.freedesktop.ScreenSaver"
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
        let proxy = Proxy::new(
            connection,
            "org.freedesktop.ScreenSaver",
            path,
            "org.freedesktop.ScreenSaver",
        )?;
        match proxy.call("GetActive", &()) {
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
fn name_owner(connection: &Connection, name: &str) -> zbus::Result<String> {
    Proxy::new(
        connection,
        "org.freedesktop.DBus",
        "/org/freedesktop/DBus",
        "org.freedesktop.DBus",
    )?
    .call("GetNameOwner", &(name,))
}
fn confirmed_session(
    connection: &Connection,
    path: &zbus::zvariant::OwnedObjectPath,
) -> zbus::Result<(bool, bool)> {
    let proxy = Proxy::new(
        connection,
        "org.freedesktop.login1",
        path.clone(),
        "org.freedesktop.login1.Session",
    )?;
    let locked: bool = proxy.get_property("LockedHint")?;
    let active: bool = proxy.get_property("Active")?;
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
    use std::sync::atomic::{AtomicBool, Ordering};
    #[test]
    fn confirmed_vt_return_permits_authentication_but_retains_explicit_lock() {
        let (send, _) = std::sync::mpsc::channel();
        let shared = Shared {
            available: AtomicBool::new(true),
            failed: AtomicBool::new(false),
            transition: Mutex::new(()),
            sessions: Mutex::new(None),
            send,
        };
        let mut state = State {
            session_active: true,
            ..State::default()
        };
        state.session_active = false;
        state.publish(&shared);
        assert!(!shared.available.load(Ordering::Acquire));
        state.session_active = true;
        state.publish(&shared);
        assert!(shared.available.load(Ordering::Acquire));
        state.explicit_lock = true;
        state.session_active = false;
        state.publish(&shared);
        state.session_active = true;
        state.publish(&shared);
        assert!(!shared.available.load(Ordering::Acquire));
        state.explicit_lock = false;
        state.publish(&shared);
        assert!(shared.available.load(Ordering::Acquire));
        shared.suspend(LockReason::HostExited);
        state.publish(&shared);
        assert!(!shared.available.load(Ordering::Acquire));
    }
    #[test]
    fn resume_does_not_override_an_independent_lock_or_lost_source() {
        let (send, events) = std::sync::mpsc::channel();
        let shared = Shared {
            available: AtomicBool::new(false),
            failed: AtomicBool::new(false),
            transition: Mutex::new(()),
            sessions: Mutex::new(None),
            send,
        };
        let mut state = State {
            sleeping: true,
            explicit_lock: true,
            session_active: true,
            ..State::default()
        };
        state.publish(&shared);
        state.sleeping = false;
        state.publish(&shared);
        assert!(!shared.available.load(Ordering::Acquire));
        state.explicit_lock = false;
        state.screensaver = true;
        state.publish(&shared);
        assert!(!shared.available.load(Ordering::Acquire));
        state.screensaver = false;
        state.publish(&shared);
        assert!(shared.available.load(Ordering::Acquire));
        state.lost = true;
        state.publish(&shared);
        assert!(!shared.available.load(Ordering::Acquire));
        assert_eq!(events.try_iter().count(), 5);
    }
}
