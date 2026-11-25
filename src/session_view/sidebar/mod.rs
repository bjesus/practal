use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{
    gdk, gio, glib,
    glib::{clone, closure_local},
};
use tracing::error;

mod icon_item_row;
mod room_row;
mod row;
mod section_row;
mod verification_row;

use self::{
    icon_item_row::SidebarIconItemRow,
    room_row::SidebarRoomRow,
    row::{SidebarRow, build_room_action_group},
    section_row::SidebarSectionRow,
    verification_row::SidebarVerificationRow,
};
use crate::{
    account_settings::{AccountSettings, AccountSettingsSubpage},
    account_switcher::AccountSwitcherButton,
    components::{Avatar, OfflineBanner},
    session::{
        CryptoIdentityState, HighlightFlags, RecoveryState, Room, RoomCategory, Session,
        SessionVerificationState, SidebarListModel, SidebarSection, SidebarSectionName,
        TargetRoomCategory, User,
    },
    utils::FixedSelection,
};

mod imp {
    use std::{
        cell::{Cell, OnceCell, RefCell},
        sync::LazyLock,
    };

    use glib::subclass::{InitializingObject, Signal};

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/session_view/sidebar/mod.ui")]
    #[properties(wrapper_type = super::Sidebar)]
    pub struct Sidebar {
        #[template_child]
        pub(super) header_bar: TemplateChild<adw::HeaderBar>,
        #[template_child]
        account_switcher_button: TemplateChild<AccountSwitcherButton>,
        #[template_child]
        security_banner: TemplateChild<adw::Banner>,
        #[template_child]
        pinned_favorites_box: TemplateChild<gtk::FlowBox>,
        #[template_child]
        scrolled_window: TemplateChild<gtk::ScrolledWindow>,
        #[template_child]
        listview: TemplateChild<gtk::ListView>,
        #[template_child]
        room_search_entry: TemplateChild<gtk::SearchEntry>,
        #[template_child]
        room_stack: TemplateChild<gtk::Stack>,
        #[template_child]
        pub(super) room_tabs_stack: TemplateChild<adw::ViewStack>,
        #[template_child]
        normal_tab_page: TemplateChild<adw::ViewStackPage>,
        #[template_child]
        low_priority_tab_page: TemplateChild<adw::ViewStackPage>,
        #[template_child]
        room_tabs_switcher: TemplateChild<adw::InlineViewSwitcher>,
        #[template_child]
        pub(super) room_search: TemplateChild<gtk::SearchBar>,
        #[template_child]
        room_row_menu: TemplateChild<gio::MenuModel>,
        room_row_popover: OnceCell<gtk::PopoverMenu>,
        /// The logged-in user.
        #[property(get, set = Self::set_user, explicit_notify, nullable)]
        user: RefCell<Option<User>>,
        /// The category of the source that activated drop mode.
        pub(super) drop_source_category: Cell<Option<RoomCategory>>,
        /// The category of the drop target that is currently hovered.
        pub(super) drop_active_target_category: Cell<Option<TargetRoomCategory>>,
        /// The list model of this sidebar.
        #[property(get, set = Self::set_list_model, explicit_notify, nullable)]
        list_model: glib::WeakRef<SidebarListModel>,
        /// The currently selected room, if any.
        #[property(get, set = Self::set_selected_room, explicit_notify, nullable)]
        selected_room: glib::WeakRef<Room>,
        /// Whether to display `1..=9` quick-switch shortcut badges (while `Alt`
        /// is held).
        #[property(get, set = Self::set_show_shortcut_badges, explicit_notify)]
        show_shortcut_badges: Cell<bool>,
        session_handler: RefCell<Option<glib::SignalHandlerId>>,
        settings_handler: RefCell<Option<glib::SignalHandlerId>>,
        security_handlers: RefCell<Vec<glib::SignalHandlerId>>,
        list_model_handlers: RefCell<Vec<glib::SignalHandlerId>>,
        favorite_section: glib::WeakRef<SidebarSection>,
        favorite_empty_handler: RefCell<Option<glib::SignalHandlerId>>,
        tab_badge_handlers:
            RefCell<Vec<(glib::WeakRef<SidebarSection>, Vec<glib::SignalHandlerId>)>>,
        active_tab: Cell<SidebarSectionName>,
        normal_tab_scroll: Cell<f64>,
        low_priority_tab_scroll: Cell<f64>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for Sidebar {
        const NAME: &'static str = "Sidebar";
        type Type = super::Sidebar;
        type ParentType = adw::NavigationPage;

        fn class_init(klass: &mut Self::Class) {
            OfflineBanner::ensure_type();

            Self::bind_template(klass);
            Self::bind_template_callbacks(klass);

            klass.set_css_name("sidebar");
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for Sidebar {
        fn signals() -> &'static [Signal] {
            static SIGNALS: LazyLock<Vec<Signal>> = LazyLock::new(|| {
                vec![
                    Signal::builder("drop-source-category-changed").build(),
                    Signal::builder("drop-active-target-category-changed").build(),
                ]
            });
            SIGNALS.as_ref()
        }

        fn constructed(&self) {
            self.parent_constructed();
            let obj = self.obj();

            let factory = gtk::SignalListItemFactory::new();
            factory.connect_setup(clone!(
                #[weak]
                obj,
                move |_, item| {
                    let Some(item) = item.downcast_ref::<gtk::ListItem>() else {
                        error!("List item factory did not receive a list item: {item:?}");
                        return;
                    };
                    let row = SidebarRow::new(&obj);
                    item.set_child(Some(&row));
                    item.bind_property("item", &row, "item").build();
                }
            ));
            self.listview.set_factory(Some(&factory));

            self.listview.connect_activate(clone!(
                #[weak(rename_to = imp)]
                self,
                move |listview, pos| {
                    let Some(model) = listview.model().and_downcast::<FixedSelection>() else {
                        return;
                    };
                    let Some(item) = model.item(pos) else {
                        return;
                    };

                    if let Some(section) = item.downcast_ref::<SidebarSection>() {
                        section.set_is_expanded(!section.is_expanded());
                    } else {
                        model.set_selected(pos);
                        if imp.room_search.is_search_mode() {
                            imp.room_search_entry.set_text("");
                            imp.room_search.set_search_mode(false);
                        }
                    }
                }
            ));

            obj.property_expression("list-model")
                .chain_property::<SidebarListModel>("selection-model")
                .bind(&*self.listview, "model", None::<&glib::Object>);

            // FIXME: Remove this hack once https://gitlab.gnome.org/GNOME/gtk/-/issues/4938 is resolved
            self.scrolled_window
                .vscrollbar()
                .first_child()
                .unwrap()
                .set_overflow(gtk::Overflow::Hidden);

            // Use the built-in search-changed signal which is already
            // debounced by the search-delay property (default 150ms).
            self.room_search_entry.connect_search_changed(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_search_filter();
                }
            ));

            self.active_tab.set(SidebarSectionName::Normal);

            self.room_tabs_stack
                .connect_visible_child_name_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |stack| {
                        let tab = if stack.visible_child_name().as_deref() == Some("low-priority") {
                            SidebarSectionName::LowPriority
                        } else {
                            SidebarSectionName::Normal
                        };
                        imp.switch_room_tab(tab);
                    }
                ));
        }

        fn dispose(&self) {
            if let Some(user) = self.user.take() {
                let session = user.session();
                if let Some(handler) = self.session_handler.take() {
                    session.disconnect(handler);
                }

                let security = session.security();
                for handler in self.security_handlers.take() {
                    security.disconnect(handler);
                }
            }
        }
    }

    impl WidgetImpl for Sidebar {
        fn grab_focus(&self) -> bool {
            if self.listview.grab_focus() {
                true
            } else {
                self.account_switcher_button.grab_focus()
            }
        }
    }

    impl NavigationPageImpl for Sidebar {}

    #[gtk::template_callbacks]
    impl Sidebar {
        /// Set the logged-in user.
        fn set_user(&self, user: Option<User>) {
            let prev_user = self.user.borrow().clone();
            if prev_user == user {
                return;
            }

            if let Some(user) = prev_user {
                let session = user.session();
                if let Some(handler) = self.session_handler.take() {
                    session.disconnect(handler);
                }
                if let Some(handler) = self.settings_handler.take() {
                    session.settings().disconnect(handler);
                }

                let security = session.security();
                for handler in self.security_handlers.take() {
                    security.disconnect(handler);
                }
            }

            if let Some(user) = &user {
                let session = user.session();

                let offline_handler = session.connect_is_offline_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_security_banner();
                    }
                ));
                self.session_handler.replace(Some(offline_handler));

                let pin_favorites_handler =
                    session.settings().connect_pin_favorites_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.update_pinned_favorites();
                        }
                    ));
                self.settings_handler.replace(Some(pin_favorites_handler));

                let security = session.security();
                let crypto_identity_handler =
                    security.connect_crypto_identity_state_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.update_security_banner();
                        }
                    ));
                let verification_handler = security.connect_verification_state_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_security_banner();
                    }
                ));
                let recovery_handler = security.connect_recovery_state_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_security_banner();
                    }
                ));

                self.security_handlers.replace(vec![
                    crypto_identity_handler,
                    verification_handler,
                    recovery_handler,
                ]);
            }

            self.user.replace(user);

            self.update_security_banner();
            self.update_pinned_favorites();
            self.obj().notify_user();
        }

        /// Set whether to display `1..=9` quick-switch shortcut badges.
        fn set_show_shortcut_badges(&self, show: bool) {
            if self.show_shortcut_badges.get() == show {
                return;
            }
            self.show_shortcut_badges.set(show);
            self.obj().notify_show_shortcut_badges();
        }

        /// Return up to the first 9 visible rooms in sidebar order (pinned
        /// favorites first, then the room list).
        pub(super) fn quick_switch_rooms(&self) -> Vec<Room> {
            let mut rooms = Vec::with_capacity(9);

            if self.pinned_favorites_box.is_visible()
                && let Some(fav_section) = self.favorite_section.upgrade()
            {
                for item in fav_section.iter::<glib::Object>() {
                    let Ok(item) = item else {
                        break;
                    };
                    if let Ok(room) = item.downcast::<Room>() {
                        rooms.push(room);
                        if rooms.len() >= 9 {
                            return rooms;
                        }
                    }
                }
            }

            if let Some(model) = self.listview.model() {
                for item in model.iter::<glib::Object>() {
                    let Ok(item) = item else {
                        break;
                    };
                    if let Ok(room) = item.downcast::<Room>()
                        && !rooms.iter().any(|r| r.room_id() == room.room_id())
                    {
                        rooms.push(room);
                        if rooms.len() >= 9 {
                            break;
                        }
                    }
                }
            }

            rooms
        }

        /// Set the currently selected room.
        fn set_selected_room(&self, room: Option<&Room>) {
            if self.selected_room.upgrade().as_ref() == room {
                return;
            }

            self.selected_room.set(room);
            self.obj().notify_selected_room();
        }

        /// Set the list model of the sidebar.
        fn set_list_model(&self, list_model: Option<&SidebarListModel>) {
            let prev_list_model = self.list_model.upgrade();
            if prev_list_model.as_ref() == list_model {
                return;
            }
            let obj = self.obj();

            if let Some(prev) = prev_list_model {
                for handler in self.list_model_handlers.take() {
                    prev.disconnect(handler);
                }
            }
            if let Some(prev_section) = self.favorite_section.upgrade()
                && let Some(handler) = self.favorite_empty_handler.take()
            {
                prev_section.disconnect(handler);
            }
            self.favorite_section.set(None);
            for (section_ref, handlers) in self.tab_badge_handlers.take() {
                if let Some(section) = section_ref.upgrade() {
                    for handler in handlers {
                        section.disconnect(handler);
                    }
                }
            }

            self.list_model.set(list_model);

            if let Some(list_model) = list_model {
                self.update_search_filter();

                self.update_room_stack(&list_model.selection_model());

                let empty_handler = list_model.selection_model().connect_is_empty_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |list| {
                        imp.update_room_stack(list);
                    }
                ));
                let filtered_handler = list_model.connect_is_filtered_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_pinned_favorites();
                    }
                ));
                self.list_model_handlers
                    .replace(vec![empty_handler, filtered_handler]);

                let favorite_section = list_model
                    .item_list()
                    .section_from_room_category(RoomCategory::Favorite);
                self.pinned_favorites_box.bind_model(
                    favorite_section.as_ref(),
                    clone!(
                        #[weak(rename_to = imp)]
                        self,
                        #[upgrade_or_panic]
                        move |item| {
                            let room = item
                                .downcast_ref::<Room>()
                                .expect("Favorite section should only contain Room items");
                            imp.build_pinned_favorite_widget(room)
                        }
                    ),
                );
                if let Some(section) = &favorite_section {
                    let section_empty_handler = section.connect_is_empty_notify(clone!(
                        #[weak(rename_to = imp)]
                        self,
                        move |_| {
                            imp.update_pinned_favorites();
                        }
                    ));
                    self.favorite_empty_handler
                        .replace(Some(section_empty_handler));
                    self.favorite_section.set(Some(section));
                }

                let mut tab_handlers = Vec::new();
                if let Some(normal_section) = list_model
                    .item_list()
                    .section_from_room_category(RoomCategory::Normal)
                {
                    tab_handlers.push(bind_tab_page_badge(&normal_section, &self.normal_tab_page));
                }
                if let Some(low_priority_section) = list_model
                    .item_list()
                    .section_from_room_category(RoomCategory::LowPriority)
                {
                    tab_handlers.push(bind_tab_page_badge(
                        &low_priority_section,
                        &self.low_priority_tab_page,
                    ));
                }
                self.tab_badge_handlers.replace(tab_handlers);
            } else {
                self.pinned_favorites_box
                    .bind_model(None::<&gio::ListModel>, |_| unreachable!());
                self.normal_tab_page.set_badge_number(0);
                self.normal_tab_page.set_needs_attention(false);
                self.low_priority_tab_page.set_badge_number(0);
                self.low_priority_tab_page.set_needs_attention(false);
            }

            self.update_pinned_favorites();
            obj.notify_list_model();
        }

        /// Update the visibility of the pinned favorites bar, bottom room tabs,
        /// and the sidebar list sections.
        fn update_pinned_favorites(&self) {
            let pin_favorites = self
                .session()
                .is_some_and(|session| session.settings().pin_favorites());
            let list_model = self.list_model.upgrade();
            let is_filtered = list_model
                .as_ref()
                .is_some_and(SidebarListModel::is_filtered);
            let has_favorites = self
                .favorite_section
                .upgrade()
                .is_some_and(|section| !section.is_empty());

            let use_compact_mode = pin_favorites && !is_filtered;
            let active_tab =
                if self.room_tabs_stack.visible_child_name().as_deref() == Some("low-priority") {
                    SidebarSectionName::LowPriority
                } else {
                    SidebarSectionName::Normal
                };

            if let Some(list_model) = list_model {
                list_model
                    .item_list()
                    .set_tab_mode_section(use_compact_mode.then_some(active_tab));
            }

            self.pinned_favorites_box
                .set_visible(use_compact_mode && has_favorites);
            self.room_tabs_switcher.set_visible(use_compact_mode);
        }

        /// Switch the active bottom room tab while preserving each tab's
        /// scroll position.
        fn switch_room_tab(&self, new_tab: SidebarSectionName) {
            let old_tab = self.active_tab.get();
            if old_tab != new_tab {
                let current_scroll = self.scrolled_window.vadjustment().value();
                match old_tab {
                    SidebarSectionName::Normal => self.normal_tab_scroll.set(current_scroll),
                    SidebarSectionName::LowPriority => {
                        self.low_priority_tab_scroll.set(current_scroll)
                    }
                    _ => {}
                }
                self.active_tab.set(new_tab);
            }

            self.update_pinned_favorites();
            self.listview.scroll_to(0, gtk::ListScrollFlags::NONE, None);

            let target = match new_tab {
                SidebarSectionName::Normal => self.normal_tab_scroll.get(),
                SidebarSectionName::LowPriority => self.low_priority_tab_scroll.get(),
                _ => 0.0,
            };
            if target > 0.0 {
                let imp = self.downgrade();
                glib::idle_add_local_once(move || {
                    if let Some(imp) = imp.upgrade() {
                        imp.scrolled_window.vadjustment().set_value(target);
                    }
                });
            }
        }

        /// Build a compact icon tile for a pinned favorite room.
        fn build_pinned_favorite_widget(&self, room: &Room) -> gtk::Widget {
            let obj = self.obj();

            let button = gtk::Button::builder()
                .css_classes(["flat", "pinned-favorite-button"])
                .halign(gtk::Align::Fill)
                .valign(gtk::Align::Center)
                .build();

            room.bind_property("display-name", &button, "tooltip-text")
                .sync_create()
                .build();

            let update_selected = clone!(
                #[weak]
                obj,
                #[weak]
                room,
                #[weak]
                button,
                move || {
                    if obj.selected_room().as_ref() == Some(&room) {
                        button.add_css_class("selected");
                    } else {
                        button.remove_css_class("selected");
                    }
                }
            );
            update_selected();
            let selected_handler =
                std::cell::Cell::new(Some(obj.connect_selected_room_notify(move |_| {
                    update_selected();
                })));
            button.connect_destroy(clone!(
                #[weak]
                obj,
                move |_| {
                    if let Some(handler) = selected_handler.take() {
                        obj.disconnect(handler);
                    }
                }
            ));

            let avatar = Avatar::new();
            avatar.set_size(36);
            avatar.set_halign(gtk::Align::Center);
            avatar.set_valign(gtk::Align::Center);
            avatar.add_css_class("pinned-favorite-avatar");
            avatar.set_watched_room(Some(room));
            room.bind_property("avatar-data", &avatar, "data")
                .sync_create()
                .build();

            let badge = gtk::Label::builder()
                .halign(gtk::Align::End)
                .valign(gtk::Align::Start)
                .visible(false)
                .build();

            let update_badge = std::rc::Rc::new(clone!(
                #[weak]
                room,
                #[weak]
                avatar,
                #[weak]
                badge,
                move || {
                    let count = room.notification_count();
                    let is_read = room.is_read();
                    update_badge_label(&badge, count, room.highlight(), is_read);
                    if !is_read || count > 0 {
                        avatar.add_css_class("unread");
                    } else {
                        avatar.remove_css_class("unread");
                    }
                }
            ));
            update_badge();
            room.connect_notification_count_notify({
                let update_badge = update_badge.clone();
                move |_| update_badge()
            });
            room.connect_highlight_notify({
                let update_badge = update_badge.clone();
                move |_| update_badge()
            });
            room.connect_is_read_notify(move |_| update_badge());

            let shortcut_badge = gtk::Label::builder()
                .halign(gtk::Align::Start)
                .valign(gtk::Align::End)
                .css_classes(["shortcut-badge"])
                .visible(false)
                .build();

            let update_shortcut_badge = clone!(
                #[weak]
                obj,
                #[weak]
                room,
                #[weak]
                shortcut_badge,
                move || {
                    if let Some(num) = obj.shortcut_number_for_room(&room) {
                        shortcut_badge.set_label(&num.to_string());
                        shortcut_badge.set_visible(true);
                    } else {
                        shortcut_badge.set_visible(false);
                    }
                }
            );
            update_shortcut_badge();
            let shortcut_handler =
                std::cell::Cell::new(Some(obj.connect_show_shortcut_badges_notify(move |_| {
                    update_shortcut_badge();
                })));
            button.connect_destroy(clone!(
                #[weak]
                obj,
                move |_| {
                    if let Some(handler) = shortcut_handler.take() {
                        obj.disconnect(handler);
                    }
                }
            ));

            let overlay = gtk::Overlay::builder()
                .halign(gtk::Align::Center)
                .valign(gtk::Align::Center)
                .child(&avatar)
                .build();
            overlay.add_overlay(&badge);
            overlay.add_overlay(&shortcut_badge);
            button.set_child(Some(&overlay));

            button.connect_clicked(clone!(
                #[weak]
                room,
                move |btn| {
                    let _ = btn.activate_action(
                        "session.show-room",
                        Some(&room.room_id().as_str().to_variant()),
                    );
                }
            ));

            let show_context_menu = clone!(
                #[weak(rename_to = imp)]
                self,
                #[weak]
                room,
                #[weak]
                button,
                move |x: f64, y: f64| {
                    let action_group = build_room_action_group(button.upcast_ref(), &room);
                    button.insert_action_group("room-row", Some(&action_group));

                    let popover = gtk::PopoverMenu::from_model(Some(&*imp.room_row_menu));
                    popover.set_has_arrow(false);
                    popover.set_halign(gtk::Align::Start);
                    popover.set_parent(&button);
                    popover.set_pointing_to(Some(&gdk::Rectangle::new(x as i32, y as i32, 0, 0)));
                    popover.connect_closed(|popover| {
                        popover.unparent();
                    });
                    popover.popup();
                }
            );

            let right_click = gtk::GestureClick::builder()
                .button(gdk::BUTTON_SECONDARY)
                .build();
            right_click.connect_released(clone!(
                #[strong]
                show_context_menu,
                move |gesture, _, x, y| {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    show_context_menu(x, y);
                }
            ));
            button.add_controller(right_click);

            let long_press = gtk::GestureLongPress::new();
            long_press.connect_pressed(clone!(
                #[strong]
                show_context_menu,
                move |gesture, x, y| {
                    gesture.set_state(gtk::EventSequenceState::Claimed);
                    show_context_menu(x, y);
                }
            ));
            button.add_controller(long_press);

            button.upcast()
        }

        /// Update the search filter with the current search text.
        ///
        /// Called when the search term changes and when the list model is set.
        fn update_search_filter(&self) {
            let Some(list_model) = self.list_model.upgrade() else {
                return;
            };

            let text = self.room_search_entry.text();
            let normalized = secular::normalized_lower_lay_string(&text);
            list_model.string_filter().set_search(Some(&normalized));
        }

        /// The current session, if any.
        fn session(&self) -> Option<Session> {
            self.user.borrow().as_ref().map(User::session)
        }

        /// Switch between room list and placeholder status page
        /// depending on if the users search has any results or not
        fn update_room_stack(&self, list: &FixedSelection) {
            let show_no_results = list.is_empty()
                && self
                    .list_model
                    .upgrade()
                    .as_ref()
                    .is_some_and(SidebarListModel::is_filtered);
            self.room_stack.set_visible_child_name(if show_no_results {
                "no-results"
            } else {
                "room-list"
            });
        }

        /// Update the security banner.
        fn update_security_banner(&self) {
            let Some(session) = self.session() else {
                return;
            };

            if session.is_offline() {
                // Only show one banner at a time.
                // The user will not be able to solve security issues while offline anyway.
                self.security_banner.set_revealed(false);
                return;
            }

            let security = session.security();
            let crypto_identity_state = security.crypto_identity_state();
            let verification_state = security.verification_state();
            let recovery_state = security.recovery_state();

            if crypto_identity_state == CryptoIdentityState::Unknown
                || verification_state == SessionVerificationState::Unknown
                || recovery_state == RecoveryState::Unknown
            {
                // Do not show the banner prematurely, unknown states should solve themselves.
                self.security_banner.set_revealed(false);
                return;
            }

            if verification_state == SessionVerificationState::Verified
                && recovery_state == RecoveryState::Enabled
            {
                // No need for the banner.
                self.security_banner.set_revealed(false);
                return;
            }

            let (title, button) = if crypto_identity_state == CryptoIdentityState::Missing {
                (gettext("No crypto identity"), gettext("Enable"))
            } else if verification_state == SessionVerificationState::Unverified {
                (gettext("Crypto identity incomplete"), gettext("Verify"))
            } else {
                match recovery_state {
                    RecoveryState::Disabled => {
                        (gettext("Account recovery disabled"), gettext("Enable"))
                    }
                    RecoveryState::Incomplete => {
                        (gettext("Account recovery incomplete"), gettext("Recover"))
                    }
                    _ => unreachable!(),
                }
            };

            self.security_banner.set_title(&title);
            self.security_banner.set_button_label(Some(&button));
            self.security_banner.set_revealed(true);
        }

        /// Set the category of the source that activated drop mode.
        pub(super) fn set_drop_source_category(&self, source_category: Option<RoomCategory>) {
            if self.drop_source_category.get() == source_category {
                return;
            }

            self.drop_source_category.set(source_category);

            if source_category.is_some() {
                self.listview.add_css_class("drop-mode");
            } else {
                self.listview.remove_css_class("drop-mode");
            }

            let Some(item_list) = self.list_model.upgrade().map(|model| model.item_list()) else {
                return;
            };

            item_list.set_show_all_for_room_category(source_category);
            self.obj()
                .emit_by_name::<()>("drop-source-category-changed", &[]);
        }

        /// The shared popover for a room row in the sidebar.
        pub(super) fn room_row_popover(&self) -> &gtk::PopoverMenu {
            self.room_row_popover.get_or_init(|| {
                let popover = gtk::PopoverMenu::builder()
                    .menu_model(&*self.room_row_menu)
                    .has_arrow(false)
                    .halign(gtk::Align::Start)
                    .build();
                popover
                    .update_property(&[gtk::accessible::Property::Label(&gettext("Context Menu"))]);

                popover
            })
        }

        /// Scroll to the currently selected item of the sidebar.
        pub(super) fn scroll_to_selection(&self) {
            let Some(list_model) = self.list_model.upgrade() else {
                return;
            };

            let selected = list_model.selection_model().selected();

            if selected != gtk::INVALID_LIST_POSITION {
                self.listview
                    .scroll_to(selected, gtk::ListScrollFlags::FOCUS, None);
            }
        }

        /// Open the proper security flow to fix the current issue.
        #[template_callback]
        fn fix_security_issue(&self) {
            let Some(session) = self.session() else {
                return;
            };

            let dialog = AccountSettings::new(&session);

            // Show the encryption tab if the user uses the back button.
            dialog.show_encryption_tab();

            let security = session.security();
            let crypto_identity_state = security.crypto_identity_state();
            let verification_state = security.verification_state();

            let subpage = if crypto_identity_state == CryptoIdentityState::Missing
                || verification_state == SessionVerificationState::Unverified
            {
                AccountSettingsSubpage::CryptoIdentitySetup
            } else {
                AccountSettingsSubpage::RecoverySetup
            };
            dialog.show_subpage(subpage);

            dialog.present(Some(&*self.obj()));
        }

        /// Select the first room found in search result,
        /// or noop if no search term has been entered.
        #[template_callback]
        fn activate_first_search_result(&self) {
            if self.room_search_entry.text().is_empty() {
                return;
            }

            let _ = self
                .listview
                .activate_action("list.activate-item", Some(&0u32.into()));
        }
    }
}

glib::wrapper! {
    /// The sidebar of the session view, displaying the list of rooms
    /// available for the current session, among other things.
    pub struct Sidebar(ObjectSubclass<imp::Sidebar>)
        @extends gtk::Widget, adw::NavigationPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

#[gtk::template_callbacks]
impl Sidebar {
    pub fn new() -> Self {
        glib::Object::new()
    }

    /// The search bar allowing to filter rooms in the sidebar.
    pub(crate) fn room_search_bar(&self) -> gtk::SearchBar {
        self.imp().room_search.clone()
    }

    /// The category of the source that activated drop mode.
    fn drop_source_category(&self) -> Option<RoomCategory> {
        self.imp().drop_source_category.get()
    }

    /// Set the category of the source that activated drop mode.
    fn set_drop_source_category(&self, source_category: Option<RoomCategory>) {
        self.imp().set_drop_source_category(source_category);
    }

    /// The category of the drop target that is currently hovered.
    fn drop_active_target_category(&self) -> Option<TargetRoomCategory> {
        self.imp().drop_active_target_category.get()
    }

    /// Set the category of the drop target that is currently hovered.
    fn set_drop_active_target_category(&self, target_category: Option<TargetRoomCategory>) {
        if self.drop_active_target_category() == target_category {
            return;
        }

        self.imp().drop_active_target_category.set(target_category);
        self.emit_by_name::<()>("drop-active-target-category-changed", &[]);
    }

    /// The shared popover for a room row in the sidebar.
    fn room_row_popover(&self) -> &gtk::PopoverMenu {
        self.imp().room_row_popover()
    }

    /// The `AdwHeaderBar` of the sidebar.
    pub(crate) fn header_bar(&self) -> &adw::HeaderBar {
        &self.imp().header_bar
    }

    /// Scroll to the currently selected item of the sidebar.
    pub(crate) fn scroll_to_selection(&self) {
        self.imp().scroll_to_selection();
    }

    /// Switch the bottom room tab to match the given room category, if
    /// applicable.
    pub(crate) fn select_tab_for_category(&self, category: RoomCategory) {
        let imp = self.imp();
        match category {
            RoomCategory::Normal => imp.room_tabs_stack.set_visible_child_name("normal"),
            RoomCategory::LowPriority => imp.room_tabs_stack.set_visible_child_name("low-priority"),
            _ => {}
        }
    }

    /// Switch to the `index`-th (`0..9`) chat in the sidebar (including pinned
    /// favorites at the top).
    pub(crate) fn select_quick_switch_room(&self, index: usize) {
        if let Some(room) = self.imp().quick_switch_rooms().get(index) {
            let _ = self.activate_action(
                "session.show-room",
                Some(&room.room_id().as_str().to_variant()),
            );
        }
    }

    /// Return the 1-based shortcut number (`1..=9`) for `room` if shortcut
    /// badges are currently shown and `room` is among the first 9 sidebar
    /// chats.
    pub(crate) fn shortcut_number_for_room(&self, room: &Room) -> Option<usize> {
        if !self.show_shortcut_badges() {
            return None;
        }
        self.imp()
            .quick_switch_rooms()
            .iter()
            .position(|r| r.room_id() == room.room_id())
            .map(|idx| idx + 1)
    }

    /// Connect to the signal emitted when the drop source category changed.
    pub fn connect_drop_source_category_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "drop-source-category-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }

    /// Connect to the signal emitted when the drop active target category
    /// changed.
    pub fn connect_drop_active_target_category_changed<F: Fn(&Self) + 'static>(
        &self,
        f: F,
    ) -> glib::SignalHandlerId {
        self.connect_closure(
            "drop-active-target-category-changed",
            true,
            closure_local!(move |obj: Self| {
                f(&obj);
            }),
        )
    }
}

/// Update an unread/notification badge label from a count, highlight, and read
/// state.
fn update_badge_label(badge: &gtk::Label, count: u64, highlight: HighlightFlags, is_read: bool) {
    if count > 0 {
        badge.set_size_request(-1, -1);
        badge.set_text(&count.to_string());
        badge.remove_css_class("pinned-unread-dot");
        badge.add_css_class("pinned-badge");
        if highlight.contains(HighlightFlags::HIGHLIGHT) {
            badge.add_css_class("highlight");
        } else {
            badge.remove_css_class("highlight");
        }
        badge.set_visible(true);
    } else if !is_read {
        badge.set_text("");
        badge.set_size_request(8, 8);
        badge.remove_css_class("pinned-badge");
        badge.remove_css_class("highlight");
        badge.add_css_class("pinned-unread-dot");
        badge.set_visible(true);
    } else {
        badge.set_visible(false);
    }
}

/// Bind an [`adw::ViewStackPage`]'s badge and attention state to a
/// [`SidebarSection`].
fn bind_tab_page_badge(
    section: &SidebarSection,
    page: &adw::ViewStackPage,
) -> (glib::WeakRef<SidebarSection>, Vec<glib::SignalHandlerId>) {
    let update = std::rc::Rc::new(clone!(
        #[weak]
        section,
        #[weak]
        page,
        move || {
            let count = section.notification_count();
            page.set_badge_number(u32::try_from(count).unwrap_or(u32::MAX));
            page.set_needs_attention(count == 0 && !section.is_read());
        }
    ));
    update();

    let count_handler = section.connect_notification_count_notify({
        let update = update.clone();
        move |_| update()
    });
    let read_handler = section.connect_is_read_notify(move |_| update());

    (section.downgrade(), vec![count_handler, read_handler])
}
