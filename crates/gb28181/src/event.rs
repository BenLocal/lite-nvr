//! The single inbound event stream both facades emit (spec §3 row 3).

use crate::client::InviteNegotiation;

/// Events surfaced by `GbServer` / `GbClient`. The receiver comes from `bind()`.
///
/// `InviteReceived` carries a live dialog handle (`InviteNegotiation`), so the
/// enum can't derive `Clone`/`PartialEq`/`Eq` — match on it instead.
#[derive(Debug)]
pub enum GbEvent {
    /// A device REGISTERed for the first time (server role).
    Registered { device_id: String },
    /// A device sent REGISTER with Expires: 0, or its registration expired (server role).
    Unregistered { device_id: String },
    /// Keepalive overdue — device still registered but marked offline (server role).
    Offline { device_id: String },
    /// A Keepalive notify arrived and refreshed the device (server role).
    KeepaliveReceived { device_id: String },
    /// An INVITE arrived (client role): answer or reject via the negotiation.
    InviteReceived(InviteNegotiation),
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
        assert!(
            matches!(e, GbEvent::Registered { device_id } if device_id == "34020000001320000001")
        );
    }
}
