use std::collections::BTreeSet;

use gtk::{glib, prelude::*, subclass::prelude::*};
use indexmap::IndexSet;
use ruma::{OwnedServerName, events::media_preview_config::MediaPreviews};
use serde::{Deserialize, Serialize};
use tracing::info;

use super::SidebarSectionName;
use crate::{Application, session_list::SessionListSettings};

/// The current version of the stored session settings.
const CURRENT_VERSION: u8 = 2;

/// The default keyword used to identify bot members.
fn default_bot_filter_keyword() -> String {
    "Bot".to_owned()
}

/// Whether the given string is the default bot filter keyword.
fn is_default_bot_filter_keyword(s: &str) -> bool {
    s == "Bot"
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[allow(clippy::struct_excessive_bools)]
pub(crate) struct StoredSessionSettings {
    /// The version of the stored settings.
    #[serde(default)]
    pub(super) version: u8,

    /// Custom servers to explore.
    #[serde(default, skip_serializing_if = "IndexSet::is_empty")]
    explore_custom_servers: IndexSet<OwnedServerName>,

    /// Whether notifications are enabled for this session.
    #[serde(
        default = "ruma::serde::default_true",
        skip_serializing_if = "ruma::serde::is_true"
    )]
    notifications_enabled: bool,

    /// Whether public read receipts are enabled for this session.
    #[serde(
        default = "ruma::serde::default_true",
        skip_serializing_if = "ruma::serde::is_true"
    )]
    public_read_receipts_enabled: bool,

    /// Whether typing notifications are enabled for this session.
    #[serde(
        default = "ruma::serde::default_true",
        skip_serializing_if = "ruma::serde::is_true"
    )]
    typing_enabled: bool,

    /// The sections that are expanded.
    #[serde(default)]
    sections_expanded: SectionsExpanded,

    /// Whether to filter bots from computed room display names.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    hide_bots_from_room_names: bool,

    /// The keyword used to identify bot members (matched as a whole word,
    /// case-insensitive).
    #[serde(
        default = "default_bot_filter_keyword",
        skip_serializing_if = "is_default_bot_filter_keyword"
    )]
    bot_filter_keyword: String,

    /// Whether to display favorite rooms as pinned icons in the sidebar.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    pin_favorites: bool,

    /// Whether to display messages as chat bubbles.
    #[serde(default, skip_serializing_if = "std::ops::Not::not")]
    use_chat_bubbles: bool,

    /// Which rooms display media previews for this session.
    ///
    /// Legacy setting from version 0 of the stored settings.
    #[serde(skip_serializing)]
    pub(super) media_previews_enabled: Option<MediaPreviewsSetting>,

    /// Whether to display avatars in invites.
    ///
    /// Legacy setting from version 0 of the stored settings.
    #[serde(skip_serializing)]
    pub(super) invite_avatars_enabled: Option<bool>,
}

impl Default for StoredSessionSettings {
    fn default() -> Self {
        Self {
            version: CURRENT_VERSION,
            explore_custom_servers: Default::default(),
            notifications_enabled: true,
            public_read_receipts_enabled: true,
            typing_enabled: true,
            sections_expanded: Default::default(),
            hide_bots_from_room_names: false,
            bot_filter_keyword: default_bot_filter_keyword(),
            pin_favorites: false,
            use_chat_bubbles: false,
            media_previews_enabled: Default::default(),
            invite_avatars_enabled: Default::default(),
        }
    }
}

mod imp {
    use std::{
        cell::{OnceCell, RefCell},
        marker::PhantomData,
    };

    use super::*;

    #[derive(Debug, Default, glib::Properties)]
    #[properties(wrapper_type = super::SessionSettings)]
    pub struct SessionSettings {
        /// The ID of the session these settings are for.
        #[property(get, construct_only)]
        session_id: OnceCell<String>,
        /// The stored settings.
        pub(super) stored_settings: RefCell<StoredSessionSettings>,
        /// Whether notifications are enabled for this session.
        #[property(get = Self::notifications_enabled, set = Self::set_notifications_enabled, explicit_notify, default = true)]
        notifications_enabled: PhantomData<bool>,
        /// Whether public read receipts are enabled for this session.
        #[property(get = Self::public_read_receipts_enabled, set = Self::set_public_read_receipts_enabled, explicit_notify, default = true)]
        public_read_receipts_enabled: PhantomData<bool>,
        /// Whether typing notifications are enabled for this session.
        #[property(get = Self::typing_enabled, set = Self::set_typing_enabled, explicit_notify, default = true)]
        typing_enabled: PhantomData<bool>,
        /// Whether to filter bots from computed room display names.
        #[property(get = Self::hide_bots_from_room_names, set = Self::set_hide_bots_from_room_names, explicit_notify)]
        hide_bots_from_room_names: PhantomData<bool>,
        /// The keyword used to identify bot members.
        #[property(get = Self::bot_filter_keyword, set = Self::set_bot_filter_keyword, explicit_notify)]
        bot_filter_keyword: PhantomData<String>,
        /// Whether to display favorite rooms as pinned icons in the sidebar.
        #[property(get = Self::pin_favorites, set = Self::set_pin_favorites, explicit_notify)]
        pin_favorites: PhantomData<bool>,
        /// Whether to display messages as chat bubbles.
        #[property(get = Self::use_chat_bubbles, set = Self::set_use_chat_bubbles, explicit_notify)]
        use_chat_bubbles: PhantomData<bool>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for SessionSettings {
        const NAME: &'static str = "SessionSettings";
        type Type = super::SessionSettings;
    }

    #[glib::derived_properties]
    impl ObjectImpl for SessionSettings {}

    impl SessionSettings {
        /// Whether notifications are enabled for this session.
        fn notifications_enabled(&self) -> bool {
            self.stored_settings.borrow().notifications_enabled
        }

        /// Set whether notifications are enabled for this session.
        fn set_notifications_enabled(&self, enabled: bool) {
            if self.notifications_enabled() == enabled {
                return;
            }

            self.stored_settings.borrow_mut().notifications_enabled = enabled;
            session_list_settings().save();
            self.obj().notify_notifications_enabled();
        }

        /// Whether public read receipts are enabled for this session.
        fn public_read_receipts_enabled(&self) -> bool {
            self.stored_settings.borrow().public_read_receipts_enabled
        }

        /// Set whether public read receipts are enabled for this session.
        fn set_public_read_receipts_enabled(&self, enabled: bool) {
            if self.public_read_receipts_enabled() == enabled {
                return;
            }

            self.stored_settings
                .borrow_mut()
                .public_read_receipts_enabled = enabled;
            session_list_settings().save();
            self.obj().notify_public_read_receipts_enabled();
        }

        /// Whether typing notifications are enabled for this session.
        fn typing_enabled(&self) -> bool {
            self.stored_settings.borrow().typing_enabled
        }

        /// Set whether typing notifications are enabled for this session.
        fn set_typing_enabled(&self, enabled: bool) {
            if self.typing_enabled() == enabled {
                return;
            }

            self.stored_settings.borrow_mut().typing_enabled = enabled;
            session_list_settings().save();
            self.obj().notify_typing_enabled();
        }

        /// Whether to filter bots from computed room display names.
        fn hide_bots_from_room_names(&self) -> bool {
            self.stored_settings.borrow().hide_bots_from_room_names
        }

        /// Set whether to filter bots from computed room display names.
        fn set_hide_bots_from_room_names(&self, enabled: bool) {
            if self.hide_bots_from_room_names() == enabled {
                return;
            }

            self.stored_settings.borrow_mut().hide_bots_from_room_names = enabled;
            session_list_settings().save();
            self.obj().notify_hide_bots_from_room_names();
        }

        /// Whether to display favorite rooms as pinned icons in the sidebar.
        fn pin_favorites(&self) -> bool {
            self.stored_settings.borrow().pin_favorites
        }

        /// Set whether to display favorite rooms as pinned icons in the
        /// sidebar.
        fn set_pin_favorites(&self, enabled: bool) {
            if self.pin_favorites() == enabled {
                return;
            }

            self.stored_settings.borrow_mut().pin_favorites = enabled;
            session_list_settings().save();
            self.obj().notify_pin_favorites();
        }

        /// Whether to display messages as chat bubbles.
        fn use_chat_bubbles(&self) -> bool {
            self.stored_settings.borrow().use_chat_bubbles
        }

        /// Set whether to display messages as chat bubbles.
        fn set_use_chat_bubbles(&self, enabled: bool) {
            if self.use_chat_bubbles() == enabled {
                return;
            }

            self.stored_settings.borrow_mut().use_chat_bubbles = enabled;
            session_list_settings().save();
            self.obj().notify_use_chat_bubbles();
        }

        /// The keyword used to identify bot members.
        fn bot_filter_keyword(&self) -> String {
            self.stored_settings.borrow().bot_filter_keyword.clone()
        }

        /// Set the keyword used to identify bot members.
        fn set_bot_filter_keyword(&self, keyword: String) {
            if self.bot_filter_keyword() == keyword {
                return;
            }

            self.stored_settings.borrow_mut().bot_filter_keyword = keyword;
            session_list_settings().save();
            self.obj().notify_bot_filter_keyword();
        }

        /// Apply the migration of the stored settings from version 0 to version
        /// 1.
        pub(crate) fn apply_version_1_migration(&self) {
            {
                let mut stored_settings = self.stored_settings.borrow_mut();

                if stored_settings.version > 0 {
                    return;
                }

                info!(
                    session = self.obj().session_id(),
                    "Migrating store session to version 1"
                );

                stored_settings.media_previews_enabled.take();
                stored_settings.invite_avatars_enabled.take();
                stored_settings.version = 1;
            }

            session_list_settings().save();
        }

        /// Apply the migration of the stored settings from version 1 to version
        /// 2.
        ///
        /// This is a no-op migration: the new interface settings fields have
        /// serde defaults so they are deserialized automatically. We just bump
        /// the version number.
        pub(crate) fn apply_version_2_migration(&self) {
            {
                let mut stored_settings = self.stored_settings.borrow_mut();

                if stored_settings.version > 1 {
                    return;
                }

                info!(
                    session = self.obj().session_id(),
                    "Migrating stored session settings to version 2"
                );

                stored_settings.version = 2;
            }

            session_list_settings().save();
        }
    }
}

glib::wrapper! {
    /// The settings of a [`Session`](super::Session).
    pub struct SessionSettings(ObjectSubclass<imp::SessionSettings>);
}

impl SessionSettings {
    /// Create a new `SessionSettings` for the given session ID.
    pub(crate) fn new(session_id: &str) -> Self {
        glib::Object::builder()
            .property("session-id", session_id)
            .build()
    }

    /// Restore existing `SessionSettings` with the given session ID and stored
    /// settings.
    pub(crate) fn restore(session_id: &str, stored_settings: StoredSessionSettings) -> Self {
        let obj = Self::new(session_id);
        *obj.imp().stored_settings.borrow_mut() = stored_settings;
        obj
    }

    /// The stored settings.
    pub(crate) fn stored_settings(&self) -> StoredSessionSettings {
        self.imp().stored_settings.borrow().clone()
    }

    /// Apply the migration of the stored settings from version 0 to version 1.
    pub(crate) fn apply_version_1_migration(&self) {
        self.imp().apply_version_1_migration();
    }

    /// Apply the migration of the stored settings from version 1 to version 2.
    pub(crate) fn apply_version_2_migration(&self) {
        self.imp().apply_version_2_migration();
    }

    /// Delete the settings from the application settings.
    pub(crate) fn delete(&self) {
        session_list_settings().remove(&self.session_id());
    }

    /// Custom servers to explore.
    pub(crate) fn explore_custom_servers(&self) -> IndexSet<OwnedServerName> {
        self.imp()
            .stored_settings
            .borrow()
            .explore_custom_servers
            .clone()
    }

    /// Set the custom servers to explore.
    pub(crate) fn set_explore_custom_servers(&self, servers: IndexSet<OwnedServerName>) {
        if self.explore_custom_servers() == servers {
            return;
        }

        self.imp()
            .stored_settings
            .borrow_mut()
            .explore_custom_servers = servers;
        session_list_settings().save();
    }

    /// Whether the section with the given name is expanded.
    pub(crate) fn is_section_expanded(&self, section_name: SidebarSectionName) -> bool {
        self.imp()
            .stored_settings
            .borrow()
            .sections_expanded
            .is_section_expanded(section_name)
    }

    /// Set whether the section with the given name is expanded.
    pub(crate) fn set_section_expanded(&self, section_name: SidebarSectionName, expanded: bool) {
        self.imp()
            .stored_settings
            .borrow_mut()
            .sections_expanded
            .set_section_expanded(section_name, expanded);
        session_list_settings().save();
    }
}

/// The sections that are expanded.
#[derive(Debug, Clone, PartialEq, Eq, Deserialize, Serialize)]
pub(crate) struct SectionsExpanded(BTreeSet<SidebarSectionName>);

impl SectionsExpanded {
    /// Whether the section with the given name is expanded.
    pub(crate) fn is_section_expanded(&self, section_name: SidebarSectionName) -> bool {
        self.0.contains(&section_name)
    }

    /// Set whether the section with the given name is expanded.
    pub(crate) fn set_section_expanded(
        &mut self,
        section_name: SidebarSectionName,
        expanded: bool,
    ) {
        if expanded {
            self.0.insert(section_name);
        } else {
            self.0.remove(&section_name);
        }
    }
}

impl Default for SectionsExpanded {
    fn default() -> Self {
        Self(BTreeSet::from([
            SidebarSectionName::VerificationRequest,
            SidebarSectionName::InviteRequest,
            SidebarSectionName::Invited,
            SidebarSectionName::Favorite,
            SidebarSectionName::Normal,
            SidebarSectionName::LowPriority,
        ]))
    }
}

/// Setting about which rooms display media previews.
///
/// Legacy setting from version 0 of the stored settings.
#[derive(Debug, Clone, Default, Deserialize)]
pub(super) struct MediaPreviewsSetting {
    /// The default setting for all rooms.
    #[serde(default)]
    pub(super) global: MediaPreviewsGlobalSetting,
}

/// Possible values of the global setting about which rooms display media
/// previews.
///
/// Legacy setting from version 0 of the stored settings.
#[derive(Debug, Clone, Default, Deserialize)]
#[serde(rename_all = "lowercase")]
pub(super) enum MediaPreviewsGlobalSetting {
    /// All rooms show media previews.
    All,
    /// Only private rooms show media previews.
    #[default]
    Private,
    /// No rooms show media previews.
    None,
}

impl From<MediaPreviewsGlobalSetting> for MediaPreviews {
    fn from(value: MediaPreviewsGlobalSetting) -> Self {
        match value {
            MediaPreviewsGlobalSetting::All => Self::On,
            MediaPreviewsGlobalSetting::Private => Self::Private,
            MediaPreviewsGlobalSetting::None => Self::Off,
        }
    }
}

/// The session list settings of the application.
fn session_list_settings() -> SessionListSettings {
    Application::default().session_list().settings()
}
