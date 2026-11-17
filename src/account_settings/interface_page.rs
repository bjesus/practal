use adw::{prelude::*, subclass::prelude::*};
use gtk::{glib, glib::clone};

use crate::session::Session;

mod imp {
    use std::cell::RefCell;

    use glib::subclass::InitializingObject;

    use super::*;

    #[derive(Debug, Default, gtk::CompositeTemplate, glib::Properties)]
    #[template(resource = "/org/gnome/Fractal/ui/account_settings/interface_page.ui")]
    #[properties(wrapper_type = super::InterfacePage)]
    pub struct InterfacePage {
        #[template_child]
        hide_bots_from_room_names_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        bot_filter_keyword_row: TemplateChild<adw::EntryRow>,
        #[template_child]
        pin_favorites_row: TemplateChild<adw::SwitchRow>,
        #[template_child]
        use_chat_bubbles_row: TemplateChild<adw::SwitchRow>,
        /// The current session.
        #[property(get, set = Self::set_session, nullable)]
        session: glib::WeakRef<Session>,
        bindings: RefCell<Vec<glib::Binding>>,
        keyword_apply_handler: RefCell<Option<glib::SignalHandlerId>>,
    }

    #[glib::object_subclass]
    impl ObjectSubclass for InterfacePage {
        const NAME: &'static str = "InterfacePage";
        type Type = super::InterfacePage;
        type ParentType = adw::PreferencesPage;

        fn class_init(klass: &mut Self::Class) {
            Self::bind_template(klass);
        }

        fn instance_init(obj: &InitializingObject<Self>) {
            obj.init_template();
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for InterfacePage {
        fn dispose(&self) {
            self.disconnect_signals();
        }
    }

    impl WidgetImpl for InterfacePage {}
    impl PreferencesPageImpl for InterfacePage {}

    impl InterfacePage {
        /// Set the current session.
        fn set_session(&self, session: Option<&Session>) {
            if self.session.upgrade().as_ref() == session {
                return;
            }

            self.disconnect_signals();

            if let Some(session) = session {
                let session_settings = session.settings();

                let hide_bots_binding = session_settings
                    .bind_property(
                        "hide-bots-from-room-names",
                        &*self.hide_bots_from_room_names_row,
                        "active",
                    )
                    .bidirectional()
                    .sync_create()
                    .build();

                let pin_favorites_binding = session_settings
                    .bind_property("pin-favorites", &*self.pin_favorites_row, "active")
                    .bidirectional()
                    .sync_create()
                    .build();

                let use_chat_bubbles_binding = session_settings
                    .bind_property("use-chat-bubbles", &*self.use_chat_bubbles_row, "active")
                    .bidirectional()
                    .sync_create()
                    .build();

                // Set the initial keyword text.
                self.bot_filter_keyword_row
                    .set_text(&session_settings.bot_filter_keyword());

                // When the user presses the apply button on the entry row,
                // save the keyword.
                let keyword_handler_id = self.bot_filter_keyword_row.connect_apply(clone!(
                    #[weak]
                    session_settings,
                    move |row| {
                        session_settings.set_bot_filter_keyword(row.text().to_string());
                    }
                ));

                self.keyword_apply_handler.replace(Some(keyword_handler_id));
                self.bindings.replace(vec![
                    hide_bots_binding,
                    pin_favorites_binding,
                    use_chat_bubbles_binding,
                ]);
            }

            self.session.set(session);
            self.obj().notify_session();
        }

        /// Disconnect signal handlers and unbind properties.
        fn disconnect_signals(&self) {
            if let Some(handler_id) = self.keyword_apply_handler.take() {
                self.bot_filter_keyword_row.disconnect(handler_id);
            }

            for binding in self.bindings.take() {
                binding.unbind();
            }
        }
    }
}

glib::wrapper! {
    /// Interface settings page.
    pub struct InterfacePage(ObjectSubclass<imp::InterfacePage>)
        @extends gtk::Widget, adw::PreferencesPage,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl InterfacePage {
    pub fn new(session: &Session) -> Self {
        glib::Object::builder().property("session", session).build()
    }
}
