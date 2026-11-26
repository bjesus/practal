use adw::{prelude::*, subclass::prelude::*};
use gettextrs::gettext;
use gtk::{gdk, gio, glib, glib::clone};

use super::{
    ContentFormat, EventTimestamp, MessageReactionList, MessageSenderName, ReadReceiptsList,
    text::MessageText, visual_media::MessageVisualMedia,
};
use crate::{
    components::{Avatar, ContextMenuBin, UserProfileDialog},
    prelude::*,
    session::{Event, EventHeaderState, Member, SessionSettings},
    session_view::room_history::{EventActionsGroup, RoomHistory},
    utils::{
        BoundObject, GroupingListGroup,
        matrix::{MediaMessage, MessageCacheKey, VisualMediaMessage},
    },
};

mod tile_imp {
    use std::{cell::RefCell, rc::Rc};

    use super::*;

    #[derive(Debug)]
    pub struct MediaAlbumTile {
        pub(super) room_history: glib::WeakRef<RoomHistory>,
        pub(super) event: RefCell<Option<Event>>,
        pub(super) visual_media: MessageVisualMedia,
        pub(super) reactions: MessageReactionList,
        pub(super) action_group: RefCell<Option<gio::SimpleActionGroup>>,
    }

    impl Default for MediaAlbumTile {
        fn default() -> Self {
            Self {
                room_history: glib::WeakRef::new(),
                event: RefCell::default(),
                visual_media: MessageVisualMedia::default(),
                reactions: glib::Object::new(),
                action_group: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MediaAlbumTile {
        const NAME: &'static str = "ContentMediaAlbumTile";
        type Type = super::MediaAlbumTile;
        type ParentType = ContextMenuBin;
    }

    impl ObjectImpl for MediaAlbumTile {
        fn constructed(&self) {
            self.parent_constructed();

            let overlay = gtk::Overlay::new();
            overlay.set_child(Some(&self.visual_media));

            self.reactions.set_halign(gtk::Align::Start);
            self.reactions.set_valign(gtk::Align::End);
            self.reactions.set_margin_start(4);
            self.reactions.set_margin_end(4);
            self.reactions.set_margin_bottom(4);
            self.reactions.add_css_class("album-tile-reactions");
            overlay.add_overlay(&self.reactions);

            self.obj().set_child(Some(&overlay));

            self.visual_media.connect_texture_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |media| {
                    if let Some(copy_image_action) = imp
                        .action_group
                        .borrow()
                        .as_ref()
                        .and_then(|group| group.lookup_action("copy-image"))
                        .and_downcast::<gio::SimpleAction>()
                    {
                        copy_image_action.set_enabled(media.texture().is_some());
                    }
                }
            ));
        }
    }

    impl WidgetImpl for MediaAlbumTile {}

    impl ContextMenuBinImpl for MediaAlbumTile {
        fn menu_opened(&self) {
            let Some(room_history) = self.room_history.upgrade() else {
                return;
            };

            let obj = self.obj();
            let Some(event) = self.event.borrow().clone() else {
                obj.set_popover(None);
                return;
            };
            if self.action_group.borrow().is_none() {
                obj.set_popover(None);
                return;
            }

            let menu = room_history.event_context_menu();

            let closed_handler_cell: Rc<RefCell<Option<glib::SignalHandlerId>>> = Rc::default();
            let closed_handler = menu.popover.connect_closed(clone!(
                #[weak]
                room_history,
                #[strong]
                closed_handler_cell,
                move |popover| {
                    room_history.enable_sticky_mode(true);

                    if let Some(handler) = closed_handler_cell.take() {
                        popover.disconnect(handler);
                    }
                }
            ));
            closed_handler_cell.replace(Some(closed_handler));

            if event.can_be_reacted_to() {
                menu.add_quick_reaction_chooser(event.reactions());
            } else {
                menu.remove_quick_reaction_chooser();
            }

            room_history.enable_sticky_mode(false);
            obj.set_popover(Some(menu.popover.clone()));
        }
    }

    impl EventActionsGroup for MediaAlbumTile {
        fn event(&self) -> Option<Event> {
            self.event.borrow().clone()
        }

        fn texture(&self) -> Option<gdk::Texture> {
            self.visual_media.texture()
        }

        fn popover(&self) -> Option<gtk::PopoverMenu> {
            self.obj().popover()
        }
    }

    impl MediaAlbumTile {
        pub(super) fn update_actions(&self) {
            let obj = self.obj();
            let action_group = self.event_actions_group();
            let has_context_menu = action_group.is_some();

            if let Some(copy_image_action) = action_group
                .as_ref()
                .and_then(|group| group.lookup_action("copy-image"))
                .and_downcast::<gio::SimpleAction>()
            {
                copy_image_action.set_enabled(self.texture().is_some());
            }

            obj.insert_action_group("event", action_group.as_ref());
            self.action_group.replace(action_group);
            obj.set_has_context_menu(has_context_menu);
        }
    }
}

glib::wrapper! {
    /// A single media tile inside a `MediaGroupRow` with its own event context
    /// menu (reactions, reply, copy/save, properties) and reaction overlay.
    pub struct MediaAlbumTile(ObjectSubclass<tile_imp::MediaAlbumTile>)
        @extends gtk::Widget, ContextMenuBin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl MediaAlbumTile {
    fn new(room_history: Option<&RoomHistory>, event: &Event) -> Self {
        let obj: Self = glib::Object::new();
        let imp = obj.imp();
        imp.room_history.set(room_history);
        imp.event.replace(Some(event.clone()));
        imp.reactions
            .set_reaction_list(&event.room().get_or_create_members(), &event.reactions());
        imp.update_actions();
        obj
    }

    fn visual_media(&self) -> &MessageVisualMedia {
        &self.imp().visual_media
    }
}

mod imp {
    use std::cell::RefCell;

    use super::*;

    #[derive(Debug, glib::Properties)]
    #[properties(wrapper_type = super::MediaGroupRow)]
    pub struct MediaGroupRow {
        /// The ancestor room history of this row.
        #[property(get, set, construct_only, nullable)]
        room_history: glib::WeakRef<RoomHistory>,
        /// The group of media events presented by this row.
        #[property(get, set = Self::set_group, explicit_notify, nullable)]
        group: BoundObject<GroupingListGroup>,
        grid: gtk::Grid,
        avatar_button: gtk::Button,
        avatar: Avatar,
        header: gtk::Box,
        display_name: MessageSenderName,
        timestamp: EventTimestamp,
        bubble_box: gtk::Box,
        album_grid: gtk::Grid,
        caption_widget: MessageText,
        read_receipts: ReadReceiptsList,
        first_event: BoundObject<Event>,
        settings_handler: RefCell<Option<(glib::WeakRef<SessionSettings>, glib::SignalHandlerId)>>,
        sender: RefCell<Option<Member>>,
    }

    impl Default for MediaGroupRow {
        fn default() -> Self {
            Self {
                room_history: glib::WeakRef::new(),
                group: BoundObject::default(),
                grid: gtk::Grid::default(),
                avatar_button: gtk::Button::default(),
                avatar: glib::Object::new(),
                header: gtk::Box::default(),
                display_name: MessageSenderName::default(),
                timestamp: EventTimestamp::default(),
                bubble_box: gtk::Box::default(),
                album_grid: gtk::Grid::default(),
                caption_widget: MessageText::default(),
                read_receipts: glib::Object::new(),
                first_event: BoundObject::default(),
                settings_handler: RefCell::default(),
                sender: RefCell::default(),
            }
        }
    }

    #[glib::object_subclass]
    impl ObjectSubclass for MediaGroupRow {
        const NAME: &'static str = "ContentMediaGroupRow";
        type Type = super::MediaGroupRow;
        type ParentType = adw::Bin;

        fn class_init(klass: &mut Self::Class) {
            klass.set_css_name("media-group-row");
            klass.set_accessible_role(gtk::AccessibleRole::ListItem);
        }
    }

    #[glib::derived_properties]
    impl ObjectImpl for MediaGroupRow {
        fn constructed(&self) {
            self.parent_constructed();

            let obj = self.obj();
            obj.add_css_class("room-history-row");

            self.grid.set_column_spacing(8);
            self.grid.add_css_class("event-content");

            // Avatar button
            self.avatar.set_size(36);
            self.avatar
                .set_accessible_role(gtk::AccessibleRole::Presentation);
            self.avatar_button.set_child(Some(&self.avatar));
            self.avatar_button
                .set_tooltip_text(Some(&gettext("Show Sender Profile")));
            self.avatar_button.set_valign(gtk::Align::Start);
            self.avatar_button.add_css_class("circular");
            self.avatar_button.add_css_class("flat");
            self.avatar_button.add_css_class("sender-avatar");
            self.avatar_button.connect_clicked(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.view_sender_profile();
                }
            ));
            self.grid.attach(&self.avatar_button, 0, 0, 1, 3);

            // Header (sender name + timestamp)
            self.header.set_spacing(5);
            self.header.set_margin_bottom(3);
            self.display_name.set_hexpand(true);
            self.display_name.set_halign(gtk::Align::Start);
            self.timestamp.set_hexpand(true);
            self.header.append(&self.display_name);
            self.header.append(&self.timestamp);
            self.grid.attach(&self.header, 1, 0, 1, 1);

            // Album container (grid + optional caption)
            self.bubble_box.set_orientation(gtk::Orientation::Vertical);
            self.bubble_box.set_spacing(6);
            self.bubble_box.set_halign(gtk::Align::Start);

            self.album_grid.set_row_spacing(3);
            self.album_grid.set_column_spacing(3);
            self.album_grid.set_overflow(gtk::Overflow::Hidden);
            self.album_grid.add_css_class("media-album-grid");
            self.bubble_box.append(&self.album_grid);

            self.caption_widget.set_visible(false);
            self.caption_widget.add_css_class("media-album-caption");
            self.bubble_box.append(&self.caption_widget);

            self.grid.attach(&self.bubble_box, 1, 1, 1, 1);

            // Read receipts
            self.grid.attach(&self.read_receipts, 1, 2, 1, 1);

            obj.set_child(Some(&self.grid));
        }

        fn dispose(&self) {
            self.disconnect_signals();
        }
    }

    impl WidgetImpl for MediaGroupRow {}
    impl BinImpl for MediaGroupRow {}

    impl MediaGroupRow {
        /// Disconnect signal handlers for the current group.
        fn disconnect_signals(&self) {
            self.group.disconnect_signals();
            self.first_event.disconnect_signals();
            if let Some((settings_ref, handler)) = self.settings_handler.take()
                && let Some(settings) = settings_ref.upgrade()
            {
                settings.disconnect(handler);
            }
        }

        /// Set the group of media events presented by this row.
        fn set_group(&self, group: Option<GroupingListGroup>) {
            if self.group.obj() == group {
                return;
            }

            self.disconnect_signals();

            if let Some(group) = group {
                let items_changed_handler = group.connect_items_changed(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_, _, _, _| {
                        imp.rebuild();
                    }
                ));

                self.group.set(group, vec![items_changed_handler]);
                self.rebuild();
            } else {
                self.clear_grid();
                self.sender.replace(None);
            }

            self.obj().notify_group();
        }

        /// Remove all tiles from the album grid.
        fn clear_grid(&self) {
            while let Some(child) = self.album_grid.first_child() {
                self.album_grid.remove(&child);
            }
        }

        /// Rebuild the album grid and metadata for the current group.
        fn rebuild(&self) {
            self.clear_grid();
            self.first_event.disconnect_signals();
            if let Some((settings_ref, handler)) = self.settings_handler.take()
                && let Some(settings) = settings_ref.upgrade()
            {
                settings.disconnect(handler);
            }

            let Some(group) = self.group.obj() else {
                return;
            };

            let events: Vec<Event> = group
                .iter::<glib::Object>()
                .filter_map(|item| item.ok().and_downcast::<Event>())
                .collect();

            let (Some(first), Some(last)) = (events.first(), events.last()) else {
                return;
            };

            let room = first.room();
            let sender = first.sender();
            self.avatar.set_data(Some(sender.avatar_data().clone()));
            self.display_name.set_sender(Some(sender.clone()));
            self.sender.replace(Some(sender));

            // Use the last event's timestamp and read receipts.
            self.timestamp.set_datetime(Some(last.timestamp()));
            self.read_receipts
                .set_members(Some(room.get_or_create_members()));
            self.read_receipts.set_source(last.read_receipts());
            self.read_receipts.set_visible(last.has_read_receipts());

            let header_handler = first.connect_header_state_notify(clone!(
                #[weak(rename_to = imp)]
                self,
                move |_| {
                    imp.update_header();
                }
            ));
            self.first_event.set(first.clone(), vec![header_handler]);

            if let Some(session) = room.session() {
                let settings = session.settings();
                let settings_handler = settings.connect_use_chat_bubbles_notify(clone!(
                    #[weak(rename_to = imp)]
                    self,
                    move |_| {
                        imp.update_bubble_mode();
                    }
                ));
                self.settings_handler
                    .replace(Some((settings.downgrade(), settings_handler)));
            }

            let visual_items: Vec<(Event, VisualMediaMessage)> = events
                .iter()
                .filter_map(|event| {
                    let visual_media_msg: VisualMediaMessage = match event.media_message()? {
                        MediaMessage::Image(image) => image.into(),
                        MediaMessage::Video(video) => video.into(),
                        _ => return None,
                    };
                    Some((event.clone(), visual_media_msg))
                })
                .collect();

            let album_context: Vec<(Option<ruma::OwnedEventId>, VisualMediaMessage)> = visual_items
                .iter()
                .map(|(event, msg)| (event.event_id(), msg.clone()))
                .collect();

            let room_history = self.room_history.upgrade();
            let count = visual_items.len();
            for (index, (event, visual_media_msg)) in visual_items.into_iter().enumerate() {
                let (col, row, col_span, row_span, dims) = tile_layout(index, count);

                let album_tile = MediaAlbumTile::new(room_history.as_ref(), &event);
                let tile = album_tile.visual_media();
                tile.set_tile_dimensions(Some(dims));
                tile.set_album_context(album_context.clone(), index);
                tile.set_media_message(
                    visual_media_msg,
                    &room,
                    ContentFormat::Natural,
                    MessageCacheKey {
                        transaction_id: event.transaction_id(),
                        event_id: event.event_id(),
                        is_edited: event.is_edited(),
                    },
                );

                self.album_grid
                    .attach(&album_tile, col, row, col_span, row_span);
            }

            // Show caption below the grid if the last item has one.
            if let Some(media_message) = last.media_message()
                && let Some((caption, formatted)) = media_message.caption()
            {
                let detect_at_room =
                    last.can_contain_at_room() && last.sender().can_notify_room();
                self.caption_widget.with_markup(
                    formatted,
                    caption,
                    &room,
                    ContentFormat::Natural,
                    detect_at_room,
                );
                self.caption_widget.set_visible(true);
            } else {
                self.caption_widget.set_visible(false);
            }

            self.update_bubble_mode();
        }

        /// Update the header visibility.
        fn update_header(&self) {
            let Some(first) = self.first_event.obj() else {
                return;
            };

            let obj = self.obj();
            let header_state = first.header_state();
            let use_bubbles = first
                .room()
                .session()
                .is_some_and(|session| session.settings().use_chat_bubbles());
            let is_own = first.sender_id() == *first.room().own_member().user_id();
            let avatar_name_visible =
                header_state == EventHeaderState::Full && !(use_bubbles && is_own);
            let header_visible = header_state != EventHeaderState::Hidden;

            self.avatar_button.set_visible(avatar_name_visible);
            self.display_name.set_visible(avatar_name_visible);
            self.header.set_visible(header_visible);

            if header_state == EventHeaderState::Full {
                obj.add_css_class("has-avatar");
            } else {
                obj.remove_css_class("has-avatar");
            }
        }

        /// Update layout and CSS classes for chat bubble mode.
        fn update_bubble_mode(&self) {
            let Some(first) = self.first_event.obj() else {
                return;
            };

            let obj = self.obj();
            let use_bubbles = first
                .room()
                .session()
                .is_some_and(|session| session.settings().use_chat_bubbles());

            if use_bubbles {
                let is_own = first.sender_id() == *first.room().own_member().user_id();
                let align = if is_own {
                    gtk::Align::End
                } else {
                    gtk::Align::Start
                };

                obj.add_css_class("chat-bubble");
                if is_own {
                    obj.add_css_class("own");
                } else {
                    obj.remove_css_class("own");
                }
                self.bubble_box.add_css_class("bubble");
                self.bubble_box.add_css_class("media-album-bubble");

                self.grid.set_halign(align);
                self.bubble_box.set_halign(align);
                self.read_receipts.set_halign(align);
                self.timestamp.set_halign(align);
            } else {
                obj.remove_css_class("chat-bubble");
                obj.remove_css_class("own");
                self.bubble_box.remove_css_class("bubble");
                self.bubble_box.remove_css_class("media-album-bubble");

                self.grid.set_halign(gtk::Align::Fill);
                self.bubble_box.set_halign(gtk::Align::Start);
                self.read_receipts.set_halign(gtk::Align::Fill);
                self.timestamp.set_halign(gtk::Align::Fill);
            }

            self.update_header();
        }

        /// View the sender's profile dialog.
        fn view_sender_profile(&self) {
            let Some(member) = self.sender.borrow().clone() else {
                return;
            };

            let dialog = UserProfileDialog::new();
            dialog.set_room_member(member);
            dialog.present(Some(&*self.obj()));
        }
    }
}

/// Compute `(col, row, col_span, row_span, (width, height))` for tile `index`
/// in an album of `count` items.
fn tile_layout(index: usize, count: usize) -> (i32, i32, i32, i32, (i32, i32)) {
    let idx = i32::try_from(index).unwrap_or(0);
    match count {
        2 => (idx, 0, 1, 1, (190, 190)),
        3 => {
            if index == 0 {
                // Hero image spanning both columns on top.
                (0, 0, 2, 1, (383, 210))
            } else {
                (idx - 1, 1, 1, 1, (190, 175))
            }
        }
        4 => (idx % 2, idx / 2, 1, 1, (190, 190)),
        _ => (idx % 3, idx / 3, 1, 1, (132, 132)),
    }
}

glib::wrapper! {
    /// A row presenting a grouped album of consecutive visual media events.
    pub struct MediaGroupRow(ObjectSubclass<imp::MediaGroupRow>)
        @extends gtk::Widget, adw::Bin,
        @implements gtk::Accessible, gtk::Buildable, gtk::ConstraintTarget;
}

impl MediaGroupRow {
    /// Construct a new empty `MediaGroupRow` for the given `RoomHistory`.
    pub fn new(room_history: &RoomHistory) -> Self {
        glib::Object::builder()
            .property("room-history", room_history)
            .build()
    }
}
