//! Typed identifiers. All Kindly entities use UUIDv7 (time-ordered) so that
//! ledger and audit exports sort chronologically without extra columns.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::str::FromStr;
use uuid::Uuid;

macro_rules! define_id {
    ($name:ident, $what:literal) => {
        #[derive(
            Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize,
        )]
        #[serde(transparent)]
        pub struct $name(pub Uuid);

        impl $name {
            /// Create a new time-ordered identifier.
            pub fn new() -> Self {
                Self(Uuid::now_v7())
            }

            pub const fn from_uuid(id: Uuid) -> Self {
                Self(id)
            }

            pub const fn as_uuid(&self) -> Uuid {
                self.0
            }
        }

        impl Default for $name {
            fn default() -> Self {
                Self::new()
            }
        }

        impl fmt::Display for $name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "{}", self.0)
            }
        }

        impl FromStr for $name {
            type Err = uuid::Error;

            fn from_str(s: &str) -> Result<Self, Self::Err> {
                Ok(Self(Uuid::parse_str(s)?))
            }
        }

        impl AsRef<Uuid> for $name {
            fn as_ref(&self) -> &Uuid {
                &self.0
            }
        }

        impl $name {
            pub const ENTITY: &'static str = $what;
        }
    };
}

define_id!(ClientId, "client");
define_id!(InvoiceId, "invoice");
define_id!(InvoiceLineId, "invoice_line");
define_id!(MandateId, "mandate");
define_id!(ActionId, "action");
define_id!(ThreadId, "thread");
define_id!(MessageId, "message");
define_id!(PaymentId, "payment");
define_id!(EventId, "paypal_event");
define_id!(AttributionId, "attribution");
define_id!(PlanId, "payment_plan");
define_id!(SessionId, "a2a_session");
define_id!(LedgerEventId, "ledger_event");

/// Which actor caused an event. Used across the ledger, messages and actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum Actor {
    /// The freelancer / business owner (the Kindly user).
    Owner,
    /// Kindly's collections agent acting under a mandate.
    KindlyAgent,
    /// The client being invoiced.
    Client,
    /// The client's own AP agent (agent-to-agent mode).
    ClientAgent,
    /// PayPal as an external system (webhooks, API responses).
    PayPal,
    /// Kindly platform automation (scheduler, reconciliation).
    System,
}

impl Actor {
    pub fn as_str(self) -> &'static str {
        match self {
            Actor::Owner => "owner",
            Actor::KindlyAgent => "kindly_agent",
            Actor::Client => "client",
            Actor::ClientAgent => "client_agent",
            Actor::PayPal => "paypal",
            Actor::System => "system",
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Actor::Owner => "You",
            Actor::KindlyAgent => "Kindly",
            Actor::Client => "Client",
            Actor::ClientAgent => "Their agent",
            Actor::PayPal => "PayPal",
            Actor::System => "Kindly system",
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ids_are_unique_and_parseable() {
        let a = InvoiceId::new();
        let b = InvoiceId::new();
        assert_ne!(a, b);
        let parsed: InvoiceId = a.to_string().parse().unwrap();
        assert_eq!(a, parsed);
    }

    #[test]
    fn ids_round_trip_through_json_as_strings() {
        let id = ClientId::new();
        let json = serde_json::to_string(&id).unwrap();
        assert!(json.starts_with('"') && json.ends_with('"'));
        let back: ClientId = serde_json::from_str(&json).unwrap();
        assert_eq!(id, back);
    }
}
