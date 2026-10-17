//! Permissions in plain words (docs/gui/APPS.md "Permissions"), as the App
//! Store, the permission dialog and Settings show them.

/// A permission as a player reads it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct PermText {
    /// Short: "Use the network".
    pub title: String,
    /// One sentence: what it lets the app do.
    pub detail: String,
}

fn t(title: &str, detail: &str) -> PermText {
    PermText { title: title.into(), detail: detail.into() }
}

/// What `permission` lets an app do.
pub fn describe(permission: &str) -> PermText {
    match permission {
        "network" => t("Use the network", "Connect to other computers and download things."),
        "radio" => t("Use radios", "Send and receive raw radio signals: Wi-Fi frames, SDRs, transmitters."),
        "cellular" => t("Use the cellular modem", "Talk to the modem directly (calls on its own data plan)."),
        "controller" => t("Use wireless controllers", "Read the paired wireless controllers."),
        "microphone" => t("Use the microphone", "Hear and record sound through the microphones (/dev/mic)."),
        "mixer" => t("Change the Volume Mixer", "Change the Volume Mixer: other apps' volumes and speakers."),
        "location" => t("Know where you are", "Read the phone's position and heading."),
        "notifications" => t("Show notifications", "Post notifications on the desktop or phone."),
        "contacts" => t("Read and change your contacts", "Contacts' address book (/home/.contacts)."),
        "phone" => t("Make and answer calls", "Calls through the phone's network; each call an app places is confirmed."),
        "sms" => t("Send and read texts", "Texts and picture messages; each message an app sends is confirmed."),
        "files:documents" => t("Read and change your documents", "Every file in /home."),
        "files:all" => t("Read and change all files", "The whole disk, except other apps' data."),
        "packages" => t("Install and manage apps", "Install, update, remove and open apps, and change their permissions."),
        "background:refresh" => t("Refresh in the background", "Wake up now and then for a moment to check for news."),
        p => {
            if let Some(ty) = p.strip_prefix("peripherals:") {
                let name = ty.replace('_', " ");
                t(&format!("Use {name} peripherals"), &format!("Control attached {name} peripherals."))
            } else if let Some(mode) = p.strip_prefix("background:") {
                t(&format!("Keep running for {mode}"), &format!("Keep running in the background while it does {mode}."))
            } else {
                t(p, "A permission this version doesn't know.")
            }
        }
    }
}

/// "Use the network, Show notifications" (for a one-line summary).
pub fn summary(permissions: &[String]) -> String {
    if permissions.is_empty() {
        return "No special permissions".into();
    }
    permissions.iter().map(|p| describe(p).title).collect::<Vec<_>>().join(", ")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_known_permission_has_words() {
        for p in ecm_pkg::manifest::PERMISSIONS {
            let d = describe(p);
            assert_ne!(d.title, p, "{p} needs a description");
            assert!(d.detail.ends_with('.'));
        }
        assert_eq!(describe("peripherals:redstone_link").title, "Use redstone link peripherals");
        assert_eq!(describe("background:audio").title, "Keep running for audio");
        assert_eq!(describe("warp").detail, "A permission this version doesn't know.");
        assert_eq!(summary(&["network".into(), "notifications".into()]), "Use the network, Show notifications");
        assert_eq!(summary(&[]), "No special permissions");
    }
}
