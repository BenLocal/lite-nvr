//! In-memory registrar: device id -> registration, with expiry + keepalive.
//! The clock is injected (a `now: i64` unix-seconds argument) so it's fully testable.

use std::collections::HashMap;

use crate::types::{RegisteredDevice, Transport};

#[derive(Default)]
pub struct Registrar {
    devices: HashMap<String, RegisteredDevice>,
    keepalive_grace: i64, // seconds of missed keepalive before offline
}

/// What changed after an operation, so the facade can emit the right event.
#[derive(Debug, PartialEq, Eq)]
pub enum RegistrarChange {
    Registered,
    Refreshed,
    Unregistered,
    NoChange,
}

impl Registrar {
    pub fn new(keepalive_grace: i64) -> Self {
        Self {
            devices: HashMap::new(),
            keepalive_grace,
        }
    }

    /// REGISTER with `expires` seconds (0 = unregister). Returns what changed.
    pub fn register(
        &mut self,
        device_id: &str,
        contact: &str,
        transport: Transport,
        expires: i64,
        now: i64,
    ) -> RegistrarChange {
        if expires == 0 {
            return if self.devices.remove(device_id).is_some() {
                RegistrarChange::Unregistered
            } else {
                RegistrarChange::NoChange
            };
        }
        let existed = self.devices.contains_key(device_id);
        self.devices.insert(
            device_id.to_string(),
            RegisteredDevice {
                device_id: device_id.to_string(),
                contact: contact.to_string(),
                transport,
                expires_at: now + expires,
                last_keepalive: now,
                online: true,
            },
        );
        if existed {
            RegistrarChange::Refreshed
        } else {
            RegistrarChange::Registered
        }
    }

    /// Record a keepalive. Returns true if the device is known.
    pub fn keepalive(&mut self, device_id: &str, now: i64) -> bool {
        if let Some(d) = self.devices.get_mut(device_id) {
            d.last_keepalive = now;
            d.online = true;
            true
        } else {
            false
        }
    }

    /// Mark devices offline if keepalive is stale, and drop expired registrations.
    /// Returns the device ids that just went offline or were removed.
    pub fn sweep(&mut self, now: i64) -> Vec<String> {
        let mut changed = Vec::new();
        self.devices.retain(|id, d| {
            if now >= d.expires_at {
                changed.push(id.clone());
                return false; // drop expired
            }
            if d.online && now - d.last_keepalive > self.keepalive_grace {
                d.online = false;
                changed.push(id.clone());
            }
            true
        });
        changed
    }

    pub fn get(&self, device_id: &str) -> Option<&RegisteredDevice> {
        self.devices.get(device_id)
    }

    pub fn list(&self) -> Vec<RegisteredDevice> {
        self.devices.values().cloned().collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn register_then_refresh() {
        let mut r = Registrar::new(90);
        assert_eq!(
            r.register("d", "sip:d@1.2.3.4", Transport::Udp, 3600, 1000),
            RegistrarChange::Registered
        );
        assert_eq!(
            r.register("d", "sip:d@1.2.3.4", Transport::Udp, 3600, 1050),
            RegistrarChange::Refreshed
        );
        assert_eq!(r.get("d").unwrap().expires_at, 1050 + 3600);
    }

    #[test]
    fn expires_zero_unregisters() {
        let mut r = Registrar::new(90);
        r.register("d", "c", Transport::Udp, 3600, 0);
        assert_eq!(
            r.register("d", "c", Transport::Udp, 0, 10),
            RegistrarChange::Unregistered
        );
        assert!(r.get("d").is_none());
    }

    #[test]
    fn sweep_marks_offline_after_missed_keepalives() {
        let mut r = Registrar::new(90); // grace 90s
        r.register("d", "c", Transport::Udp, 3600, 0);
        // no keepalive; at t=100 (>90) it should go offline but stay registered
        let changed = r.sweep(100);
        assert_eq!(changed, vec!["d".to_string()]);
        assert!(!r.get("d").unwrap().online);
    }

    #[test]
    fn sweep_drops_expired() {
        let mut r = Registrar::new(90);
        r.register("d", "c", Transport::Udp, 60, 0); // expires at 60
        let changed = r.sweep(61);
        assert_eq!(changed, vec!["d".to_string()]);
        assert!(r.get("d").is_none());
    }
}
