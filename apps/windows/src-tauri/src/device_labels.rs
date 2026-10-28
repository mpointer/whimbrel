//! Which device a timeline row came from, as a person reads it (plan Task 4).
//!
//! A row's `device_id` is its capturing device's base64 X25519 public key, the same
//! string the Worker lists as `DeviceRecord.public_key`. The names come from the
//! Worker's device list, which Task 5 fetches at launch and every 10 minutes, not only
//! while the Devices pane is open. That is the gap that leaves the Mac showing
//! "Device xxxxxx" until its pane has been opened (plan optional Task B).

use std::collections::HashMap;

/// The fallback id of Mac rows captured before sync identities existed.
pub const LEGACY_SPIKE_DEVICE: &str = "spike-device";

#[derive(Debug, Clone, Default)]
pub struct DeviceLabeler {
    pub own_device_id: Option<String>,
    /// `device_id` (public key) → the name set in the Worker.
    pub names: HashMap<String, String>,
}

impl DeviceLabeler {
    pub fn label(&self, device_id: &str) -> String {
        if self.is_this_device(device_id) {
            return "This PC".to_string();
        }
        if let Some(name) = self.names.get(device_id).map(|n| n.trim()).filter(|n| !n.is_empty()) {
            return name.to_string();
        }
        if device_id == LEGACY_SPIKE_DEVICE {
            return "Unknown device".to_string();
        }
        format!("Device {}", device_id.chars().take(6).collect::<String>())
    }

    pub fn is_this_device(&self, device_id: &str) -> bool {
        self.own_device_id.as_deref() == Some(device_id)
    }

    /// Replaces the names from a fresh Worker device list.
    pub fn names_from_records(records: &[api::sync_ops::enrollment::DeviceRecord]) -> HashMap<String, String> {
        records
            .iter()
            .filter_map(|r| r.name.as_ref().map(|n| (r.public_key.clone(), n.clone())))
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn labeler() -> DeviceLabeler {
        DeviceLabeler {
            own_device_id: Some("sFpWOJmD/UjH-this-pc".into()),
            names: HashMap::from([("MacKey123456".to_string(), "Mike's MacBook Pro".to_string()), ("Blank000000".to_string(), "  ".to_string())]),
        }
    }

    #[test]
    fn this_device_is_this_pc() {
        assert_eq!(labeler().label("sFpWOJmD/UjH-this-pc"), "This PC");
    }

    #[test]
    fn a_named_peer_shows_its_worker_name() {
        assert_eq!(labeler().label("MacKey123456"), "Mike's MacBook Pro");
    }

    #[test]
    fn legacy_and_unknown_devices_fall_back() {
        assert_eq!(labeler().label(LEGACY_SPIKE_DEVICE), "Unknown device");
        assert_eq!(labeler().label("QWERTYuiop=="), "Device QWERTY");
        assert_eq!(labeler().label("Blank000000"), "Device Blank0", "a blank Worker name is not a name");
    }

    #[test]
    fn names_come_from_named_worker_records_only() {
        let record = |pk: &str, name: Option<&str>| api::sync_ops::enrollment::DeviceRecord {
            id: format!("id-{pk}"),
            account_id: "acct".into(),
            public_key: pk.into(),
            name: name.map(String::from),
            platform: None,
            status: "approved".into(),
            enrolled_at_unix_ms: 0,
            revoked_at_unix_ms: None,
        };
        let names = DeviceLabeler::names_from_records(&[record("A", Some("Mac")), record("B", None)]);
        assert_eq!(names, HashMap::from([("A".to_string(), "Mac".to_string())]));
    }
}
