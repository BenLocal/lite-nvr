//! The single inbound event stream both facades emit (spec §3 row 3).

/// Events surfaced by `GbServer` / `GbClient`. The receiver comes from `bind()`.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GbEvent {
    /// A device REGISTERed for the first time (server role).
    Registered { device_id: String },
    /// A device sent REGISTER with Expires: 0, or its registration expired (server role).
    Unregistered { device_id: String },
    /// Keepalive overdue — device still registered but marked offline (server role).
    Offline { device_id: String },
    /// A Keepalive notify arrived and refreshed the device (server role).
    KeepaliveReceived { device_id: String },
    /// A media dialog ended (BYE from either side, or error). Both roles.
    SessionClosed { dialog_id: String },
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn events_carry_device_id() {
        let e = GbEvent::Registered {
            device_id: "34020000001320000001".into(),
        };
        assert_eq!(
            e,
            GbEvent::Registered {
                device_id: "34020000001320000001".into()
            }
        );
    }
}
