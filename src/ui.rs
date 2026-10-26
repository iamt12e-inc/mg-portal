use crate::{
    config::Settings,
    credentials,
    worker::{self, AppStatus, StatusKind},
};
use anyhow::{Context, Result};
use gtk::gio;
use gtk::glib;
use gtk::prelude::*;
use std::cell::{Cell, RefCell};
use std::fs;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::{Arc, Mutex, mpsc};
use std::thread;
use std::time::Duration;

const APP_ID: &str = "io.github.tanaka_mambinge.MugonatPortal";
const HOTSPOT_STATUS_URL: &str = "http://login.net/status";

struct BackgroundResult {
    enabled: bool,
    failed: bool,
}

struct Widgets {
    window: gtk::ApplicationWindow,
    status_card: gtk::Box,
    status_icon: gtk::Image,
    status_title: gtk::Label,
    status_detail: gtk::Label,
    try_sign_in_button: gtk::Button,
    portal_section: gtk::Box,
    portal_address: gtk::Label,
    open_portal_button: gtk::Button,
    hotspot_status_button: gtk::Button,
    connection_feedback: gtk::Label,
    network_view: gtk::TextView,
    network_save_button: gtk::Button,
    network_feedback: gtk::Label,
    username_entry: gtk::Entry,
    password_entry: gtk::Entry,
    credentials_save_button: gtk::Button,
    credentials_feedback: gtk::Label,
    auto_start_switch: gtk::Switch,
    auto_start_guard: Rc<Cell<bool>>,
    background_feedback: gtk::Label,
}

pub fn install(app: &gtk::Application, settings: Arc<Mutex<Settings>>) {
    let (status_sender, status_receiver) = mpsc::channel();
    let (refresh_sender, refresh_receiver) = mpsc::channel::<worker::MonitorCommand>();
    let refresh_receiver = Rc::new(RefCell::new(Some(refresh_receiver)));
    let (background_sender, background_receiver) = mpsc::channel::<BackgroundResult>();
    let status_receiver = Rc::new(RefCell::new(status_receiver));
    let background_receiver = Rc::new(RefCell::new(background_receiver));
    let widgets: Rc<RefCell<Option<Widgets>>> = Rc::new(RefCell::new(None));
    let current_status = Rc::new(RefCell::new(AppStatus {
        kind: StatusKind::Waiting,
        title: "Starting up".to_owned(),
        detail: "Checking your Wi-Fi connection.".to_owned(),
        portal_url: None,
        can_try_sign_in: false,
    }));

    let settings_for_startup = settings.clone();
    let widgets_for_startup = widgets.clone();
    let status_for_startup = current_status.clone();
    let status_receiver_for_startup = status_receiver.clone();
    let background_receiver_for_startup = background_receiver.clone();
    let refresh_receiver_for_startup = refresh_receiver.clone();
    app.connect_startup(move |app| {
        install_styles();
        let sender = status_sender.clone();
        let settings = settings_for_startup.clone();
        if let Some(refresh_receiver) = refresh_receiver_for_startup.borrow_mut().take() {
            let _ = thread::Builder::new()
                .name("mugonat-network-monitor".to_owned())
                .spawn(move || worker::run(settings, sender, refresh_receiver));
        }

        let status_receiver = status_receiver_for_startup.clone();
        let background_receiver = background_receiver_for_startup.clone();
        let widgets = widgets_for_startup.clone();
        let current_status = status_for_startup.clone();
        let settings_for_timeout = settings_for_startup.clone();
        let app = app.clone();

        glib::timeout_add_local(Duration::from_millis(500), move || {
            loop {
                let received = status_receiver.borrow_mut().try_recv();
                let Ok(status) = received else { break };
                let changed = *current_status.borrow() != status;
                *current_status.borrow_mut() = status.clone();
                if let Some(widgets) = widgets.borrow().as_ref() {
                    apply_status(widgets, &status);
                }
                if changed {
                    handle_status_attention(&app, &status);
                }
            }

            loop {
                let received = background_receiver.borrow_mut().try_recv();
                let Ok(result) = received else { break };
                let auto_start = result.enabled && !result.failed;
                let mut settings_saved = true;
                if let Ok(mut settings) = settings_for_timeout.lock() {
                    settings.auto_start = auto_start;
                    if let Err(error) = settings.save() {
                        eprintln!("Could not save the startup preference: {error:#}");
                        settings_saved = false;
                    }
                } else {
                    settings_saved = false;
                }
                if let Some(widgets) = widgets.borrow().as_ref() {
                    widgets.auto_start_guard.set(true);
                    widgets.auto_start_switch.set_active(auto_start);
                    set_auto_start_style(&widgets.auto_start_switch, auto_start);
                    widgets.auto_start_guard.set(false);
                    let message = if result.failed {
                        "Couldn't change this setting. Try again."
                    } else if !settings_saved {
                        "Couldn't save this setting. Try again."
                    } else if auto_start {
                        "Mugonat Portal will start when you sign in."
                    } else {
                        "Mugonat Portal won't start automatically."
                    };
                    show_feedback(&widgets.background_feedback, message);
                }
            }
            glib::ControlFlow::Continue
        });
    });

    let settings_for_activate = settings.clone();
    let widgets_for_activate = widgets.clone();
    let status_for_activate = current_status.clone();
    let background_sender = background_sender.clone();
    let refresh_sender_for_activate = refresh_sender.clone();
    app.connect_activate(move |app| {
        if widgets_for_activate.borrow().is_none() {
            let built = build_window(
                app,
                settings_for_activate.clone(),
                status_for_activate.clone(),
                background_sender.clone(),
                refresh_sender_for_activate.clone(),
            );
            *widgets_for_activate.borrow_mut() = Some(built);
        }
        if let Some(widgets) = widgets_for_activate.borrow().as_ref() {
            apply_status(widgets, &status_for_activate.borrow());
            widgets.window.present();
        }
    });

    let background_hold: Rc<RefCell<Option<gio::ApplicationHoldGuard>>> =
        Rc::new(RefCell::new(None));
    let background_hold_for_command_line = background_hold.clone();
    app.connect_command_line(move |app, command_line| {
        let args = command_line.arguments();
        let background_launch = args
            .iter()
            .any(|arg| arg.to_string_lossy() == "--background");
        if background_launch {
            let has_background_hold = background_hold_for_command_line.borrow().is_some();
            if !has_background_hold {
                *background_hold_for_command_line.borrow_mut() = Some(app.hold());
            }
        } else {
            app.activate();
        }
        0
    });
}

fn install_styles() {
    let Some(screen) = gtk::gdk::Screen::default() else {
        return;
    };
    let provider = gtk::CssProvider::new();
    let _ = provider.load_from_data(include_str!("../data/style.css").as_bytes());
    gtk::StyleContext::add_provider_for_screen(
        &screen,
        &provider,
        gtk::STYLE_PROVIDER_PRIORITY_APPLICATION,
    );
}

fn section_heading(text: &str) -> gtk::Label {
    let label = gtk::Label::builder().label(text).xalign(0.0).build();
    label.style_context().add_class("heading");
    label
}

fn helper_label(text: &str) -> gtk::Label {
    let label = gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build();
    label.style_context().add_class("dim-label");
    label
}

trait BoxAppendExt {
    fn append(&self, child: &impl IsA<gtk::Widget>);
}

impl BoxAppendExt for gtk::Box {
    fn append(&self, child: &impl IsA<gtk::Widget>) {
        let child = child.as_ref();
        self.pack_start(child, child.hexpands() || child.vexpands(), true, 0);
    }
}

fn page_with_scrollbar(scroll: &gtk::ScrolledWindow) -> gtk::Box {
    let adjustment = scroll.vadjustment();
    let scrollbar = gtk::Scrollbar::new(gtk::Orientation::Vertical, Some(&adjustment));
    scrollbar.set_vexpand(true);
    scrollbar.set_margin_start(4);
    scrollbar.set_visible(false);

    let scrollbar_for_range = scrollbar.clone();
    adjustment.connect_changed(move |adjustment| {
        scrollbar_for_range
            .set_visible(adjustment.upper() > adjustment.lower() + adjustment.page_size() + 1.0);
    });
    let scrollbar_for_page = scrollbar.clone();
    adjustment.connect_page_size_notify(move |adjustment| {
        scrollbar_for_page
            .set_visible(adjustment.upper() > adjustment.lower() + adjustment.page_size() + 1.0);
    });

    let page = gtk::Box::new(gtk::Orientation::Horizontal, 0);
    page.append(scroll);
    page.append(&scrollbar);
    page
}

fn show_feedback(label: &gtk::Label, text: &str) {
    label.set_text(text);
    label.set_visible(true);
}

fn set_auto_start_style(switch: &gtk::Switch, enabled: bool) {
    if enabled {
        switch.style_context().add_class("autostart-enabled");
    } else {
        switch.style_context().remove_class("autostart-enabled");
    }
}

fn build_window(
    app: &gtk::Application,
    settings: Arc<Mutex<Settings>>,
    current_status: Rc<RefCell<AppStatus>>,
    background_sender: mpsc::Sender<BackgroundResult>,
    refresh_sender: mpsc::Sender<worker::MonitorCommand>,
) -> Widgets {
    let snapshot = settings
        .lock()
        .map(|settings| settings.clone())
        .unwrap_or_default();
    let saved_username = credentials::saved_username();
    let window = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Mugonat Portal")
        .default_width(520)
        .default_height(680)
        .build();

    let header = gtk::HeaderBar::new();
    header.set_show_close_button(true);
    let window_title = gtk::Label::new(Some("Mugonat Portal"));
    header.set_custom_title(Some(&window_title));

    let refresh_button =
        gtk::Button::from_icon_name(Some("view-refresh-symbolic"), gtk::IconSize::Button);
    refresh_button.set_tooltip_text(Some("Check your connection now"));
    header.pack_end(&refresh_button);
    window.set_titlebar(Some(&header));
    let refresh_sender_for_button = refresh_sender.clone();
    refresh_button.connect_clicked(move |_| {
        if refresh_sender_for_button
            .send(worker::MonitorCommand::CheckNow)
            .is_err()
        {
            eprintln!("The connection checker is no longer running.");
        }
    });

    let root = gtk::Box::new(gtk::Orientation::Vertical, 14);
    root.set_margin_top(12);
    root.set_margin_bottom(16);
    let stack = gtk::Stack::builder().vexpand(true).build();
    let switcher = gtk::StackSwitcher::new();
    switcher.set_stack(Some(&stack));
    switcher.set_halign(gtk::Align::Center);
    switcher.set_margin_start(22);
    switcher.set_margin_end(22);
    switcher.set_margin_top(8);
    switcher.set_margin_bottom(4);
    root.append(&switcher);
    root.append(&stack);
    window.add(&root);

    let status_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .build();
    let status_content = gtk::Box::new(gtk::Orientation::Vertical, 18);
    status_content.set_margin_top(20);
    status_content.set_margin_bottom(8);
    status_content.set_margin_start(18);
    status_content.set_margin_end(18);
    status_scroll.add(&status_content);

    let settings_scroll = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vscrollbar_policy(gtk::PolicyType::Never)
        .hexpand(true)
        .vexpand(true)
        .build();
    let settings_content = gtk::Box::new(gtk::Orientation::Vertical, 22);
    settings_content.set_margin_top(18);
    settings_content.set_margin_bottom(12);
    settings_content.set_margin_start(22);
    settings_content.set_margin_end(22);
    settings_scroll.add(&settings_content);

    let status_title = gtk::Label::builder()
        .label("Starting up")
        .xalign(0.0)
        .wrap(true)
        .build();
    status_title.style_context().add_class("title-2");
    let status_detail = gtk::Label::builder()
        .label("Checking your Wi-Fi connection.")
        .xalign(0.0)
        .wrap(true)
        .build();
    status_detail.style_context().add_class("status-detail");
    let status_copy = gtk::Box::new(gtk::Orientation::Vertical, 8);
    status_copy.set_valign(gtk::Align::Center);
    status_copy.append(&status_title);
    status_copy.append(&status_detail);
    let status_icon =
        gtk::Image::from_icon_name(Some("network-wireless-symbolic"), gtk::IconSize::Dialog);
    status_icon.set_pixel_size(40);
    status_icon.style_context().add_class("status-icon");
    status_icon.set_valign(gtk::Align::Center);
    let status_card = gtk::Box::new(gtk::Orientation::Horizontal, 18);
    status_card.style_context().add_class("status-card");
    status_card.append(&status_icon);
    status_card.append(&status_copy);
    status_content.append(&status_card);

    let try_sign_in_button = gtk::Button::with_label("Try sign-in again");
    try_sign_in_button
        .style_context()
        .add_class("suggested-action");
    try_sign_in_button.set_halign(gtk::Align::Start);
    try_sign_in_button.set_visible(false);

    let portal_address = gtk::Label::builder()
        .label("")
        .xalign(0.0)
        .selectable(true)
        .wrap(true)
        .build();
    portal_address.style_context().add_class("dim-label");
    let portal_heading = gtk::Label::builder()
        .label("Sign-in page")
        .xalign(0.0)
        .build();
    portal_heading.style_context().add_class("heading");
    let open_portal_button = gtk::Button::with_label("Open sign-in page");
    open_portal_button.set_sensitive(false);
    open_portal_button.style_context().add_class("flat");
    open_portal_button.set_halign(gtk::Align::Start);
    let portal_section = gtk::Box::new(gtk::Orientation::Vertical, 10);
    portal_section.append(&portal_heading);
    portal_section.append(&portal_address);
    portal_section.append(&open_portal_button);
    portal_section.set_visible(false);
    status_content.append(&portal_section);

    let hotspot_status_button = gtk::Button::with_label("View hotspot usage");
    hotspot_status_button.style_context().add_class("flat");
    hotspot_status_button.set_halign(gtk::Align::Start);
    let connection_actions = gtk::FlowBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .row_spacing(4)
        .column_spacing(8)
        .min_children_per_line(1)
        .max_children_per_line(2)
        .build();
    connection_actions.set_hexpand(true);
    connection_actions.insert(&try_sign_in_button, -1);
    connection_actions.insert(&hotspot_status_button, -1);
    status_content.append(&connection_actions);

    let settings_heading = gtk::Label::builder()
        .label("Settings")
        .xalign(0.0)
        .build();
    settings_heading.style_context().add_class("title-2");
    settings_content.append(&settings_heading);
    let settings_description =
        helper_label("Choose which Wi-Fi networks to watch and save your portal sign-in details.");
    settings_content.append(&settings_description);

    let network_heading = section_heading("Wi-Fi networks");
    let network_label = helper_label("Use the exact Wi-Fi names the app should watch.");
    let network_view = gtk::TextView::new();
    network_view.set_wrap_mode(gtk::WrapMode::WordChar);
    network_view.set_top_margin(8);
    network_view.set_bottom_margin(8);
    network_view.set_left_margin(8);
    network_view.set_right_margin(8);
    if let Some(buffer) = network_view.buffer() {
        buffer.set_text(&snapshot.target_networks.join("\n"));
    }
    let network_scroll = gtk::ScrolledWindow::builder()
        .min_content_height(96)
        .hscrollbar_policy(gtk::PolicyType::Never)
        .build();
    network_scroll.add(&network_view);
    let network_frame = gtk::Frame::new(None);
    network_frame.style_context().add_class("settings-input");
    network_frame.add(&network_scroll);
    let network_save_button = gtk::Button::with_label("Save Wi-Fi networks");
    network_save_button
        .style_context()
        .add_class("suggested-action");
    network_save_button.set_halign(gtk::Align::End);
    let network_feedback = helper_label("Changes apply after you save the network list.");
    let network_section = gtk::Box::new(gtk::Orientation::Vertical, 10);
    network_section.append(&network_heading);
    network_section.append(&network_label);
    network_section.append(&network_frame);
    network_section.append(&network_save_button);
    network_section.append(&network_feedback);
    settings_content.append(&network_section);

    settings_content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let username_entry = gtk::Entry::builder()
        .placeholder_text("Portal username")
        .hexpand(true)
        .build();
    if let Some(username) = saved_username.as_deref() {
        username_entry.set_text(&username);
    }
    let password_entry = gtk::Entry::builder()
        .placeholder_text("Portal password")
        .visibility(false)
        .input_purpose(gtk::InputPurpose::Password)
        .build();
    password_entry.set_icon_from_icon_name(
        gtk::EntryIconPosition::Secondary,
        Some("view-reveal-symbolic"),
    );
    password_entry.set_icon_activatable(gtk::EntryIconPosition::Secondary, true);
    password_entry.set_icon_tooltip_text(
        gtk::EntryIconPosition::Secondary,
        Some("Show password"),
    );
    password_entry.connect_icon_press(|entry, position, _event| {
        if position == gtk::EntryIconPosition::Secondary {
            let visible = !entry.property::<bool>("visibility");
            entry.set_visibility(visible);
            entry.set_icon_from_icon_name(
                gtk::EntryIconPosition::Secondary,
                Some(if visible {
                    "view-conceal-symbolic"
                } else {
                    "view-reveal-symbolic"
                }),
            );
            entry.set_icon_tooltip_text(
                gtk::EntryIconPosition::Secondary,
                Some(if visible { "Hide password" } else { "Show password" }),
            );
        }
    });
    let credentials_save_button = gtk::Button::with_label("Save sign-in details");
    credentials_save_button
        .style_context()
        .add_class("suggested-action");
    credentials_save_button.set_halign(gtk::Align::End);
    let credential_description = if saved_username.is_some() {
        "Your username is saved. Re-enter the password to save sign-in details."
    } else {
        "Your sign-in details are stored securely on this computer."
    };
    let credential_help = helper_label(credential_description);
    let credentials_feedback = helper_label("");
    credentials_feedback.set_visible(false);
    let credentials_box = gtk::Box::new(gtk::Orientation::Vertical, 8);
    credentials_box.append(&username_entry);
    credentials_box.append(&password_entry);
    credentials_box.append(&credentials_save_button);
    credentials_box.append(&credentials_feedback);
    let credentials_heading = section_heading("Portal sign-in");
    let credentials_section = gtk::Box::new(gtk::Orientation::Vertical, 10);
    credentials_section.append(&credentials_heading);
    credentials_section.append(&credential_help);
    credentials_section.append(&credentials_box);
    settings_content.append(&credentials_section);

    settings_content.append(&gtk::Separator::new(gtk::Orientation::Horizontal));

    let auto_start_switch = gtk::Switch::new();
    auto_start_switch.set_active(snapshot.auto_start);
    set_auto_start_style(&auto_start_switch, snapshot.auto_start);
    auto_start_switch.set_valign(gtk::Align::Center);
    let auto_start_guard = Rc::new(Cell::new(false));
    let auto_start_label = gtk::Label::builder()
        .label("Start when you sign in")
        .xalign(0.0)
        .hexpand(true)
        .build();
    let auto_start_description =
        helper_label("Start Mugonat Portal when you sign in to your Linux desktop.");
    let auto_start_copy = gtk::Box::new(gtk::Orientation::Vertical, 4);
    auto_start_copy.append(&auto_start_label);
    auto_start_copy.append(&auto_start_description);
    let auto_start_row = gtk::Box::new(gtk::Orientation::Horizontal, 16);
    auto_start_row.append(&auto_start_copy);
    auto_start_row.append(&auto_start_switch);
    let background_feedback = helper_label("");
    background_feedback.set_visible(false);
    let background_heading = section_heading("Background startup");
    let background_section = gtk::Box::new(gtk::Orientation::Vertical, 10);
    background_section.append(&background_heading);
    background_section.append(&auto_start_row);
    background_section.append(&background_feedback);
    settings_content.append(&background_section);

    let status_page = page_with_scrollbar(&status_scroll);
    let settings_page = page_with_scrollbar(&settings_scroll);
    stack.add_titled(&status_page, "connection", "Connection");
    stack.add_titled(&settings_page, "settings", "Settings");

    let widgets = Widgets {
        window: window.clone(),
        status_card,
        status_icon,
        status_title,
        status_detail,
        try_sign_in_button,
        portal_section,
        portal_address,
        open_portal_button,
        hotspot_status_button,
        connection_feedback: helper_label(""),
        network_view,
        network_save_button,
        network_feedback,
        username_entry,
        password_entry,
        credentials_save_button,
        credentials_feedback,
        auto_start_switch,
        auto_start_guard,
        background_feedback,
    };

    widgets.connection_feedback.set_visible(false);
    status_content.append(&widgets.connection_feedback);

    connect_network_save_button(&widgets, settings.clone());
    connect_credentials_save_button(&widgets, settings.clone());
    connect_try_sign_in_button(&widgets, refresh_sender.clone(), current_status.clone());
    connect_open_portal_button(&widgets, current_status);
    widgets.hotspot_status_button.connect_clicked(|_| {
        if let Err(error) =
            gio::AppInfo::launch_default_for_uri(HOTSPOT_STATUS_URL, None::<&gio::AppLaunchContext>)
        {
            eprintln!("Could not open hotspot usage: {error}");
        }
    });
    connect_auto_start(&widgets, background_sender);

    let window = widgets.window.clone();
    window.connect_delete_event(|window, _event| {
        window.hide();
        glib::Propagation::Stop
    });

    widgets
}

fn connect_try_sign_in_button(
    widgets: &Widgets,
    sender: mpsc::Sender<worker::MonitorCommand>,
    current_status: Rc<RefCell<AppStatus>>,
) {
    widgets.try_sign_in_button.connect_clicked(move |_| {
        let command = if current_status.borrow().kind == StatusKind::Online {
            worker::MonitorCommand::CheckNow
        } else {
            worker::MonitorCommand::ForceSignIn
        };
        if sender.send(command).is_err() {
            eprintln!("The connection checker is no longer running.");
        }
    });
}

fn connect_network_save_button(widgets: &Widgets, settings: Arc<Mutex<Settings>>) {
    let network_view = widgets.network_view.clone();
    let feedback = widgets.network_feedback.clone();
    widgets.network_save_button.connect_clicked(move |_| {
        let networks = text_view_lines(&network_view);
        if networks.is_empty() {
            show_feedback(&feedback, "Add at least one Wi-Fi name.");
            return;
        }

        match settings.lock() {
            Ok(mut settings) => {
                let previous = settings.clone();
                settings.target_networks = networks;
                settings.revision = settings.revision.saturating_add(1);
                match settings.save() {
                    Ok(()) => show_feedback(&feedback, "Wi-Fi networks saved."),
                    Err(error) => {
                        *settings = previous;
                        eprintln!("Could not save Wi-Fi networks: {error:#}");
                        show_feedback(&feedback, "Couldn't save the Wi-Fi names. Try again.");
                    }
                }
            }
            Err(_) => show_feedback(&feedback, "Couldn't save the Wi-Fi names. Try again."),
        }
    });
}

fn connect_credentials_save_button(widgets: &Widgets, settings: Arc<Mutex<Settings>>) {
    let username = widgets.username_entry.clone();
    let password = widgets.password_entry.clone();
    let feedback = widgets.credentials_feedback.clone();
    widgets.credentials_save_button.connect_clicked(move |_| {
        let username_text = username.text().to_string();
        let password_text = password.text().to_string();
        if username_text.trim().is_empty() || password_text.is_empty() {
            show_feedback(&feedback, "Enter both your username and password.");
            return;
        }
        if let Err(error) = credentials::store(&username_text, &password_text) {
            eprintln!("Could not save sign-in details: {error:#}");
            show_feedback(&feedback, "Couldn't save your sign-in details. Try again.");
            return;
        }
        password.set_text("");
        match settings.lock() {
            Ok(mut settings) => {
                settings.revision = settings.revision.saturating_add(1);
                match settings.save() {
                    Ok(()) => show_feedback(&feedback, "Sign-in details saved securely."),
                    Err(error) => {
                        eprintln!("Could not update app settings after saving credentials: {error:#}");
                        show_feedback(
                            &feedback,
                            "Sign-in details saved, but the app couldn't save this change. Try again.",
                        );
                    }
                }
            }
            Err(_) => show_feedback(&feedback, "Sign-in details were saved, but the app couldn't save this change."),
        }
    });
}

fn connect_open_portal_button(widgets: &Widgets, current_status: Rc<RefCell<AppStatus>>) {
    widgets.open_portal_button.connect_clicked(move |_| {
        if let Some(url) = current_status.borrow().portal_url.as_deref() {
            if let Err(error) =
                gio::AppInfo::launch_default_for_uri(url, None::<&gio::AppLaunchContext>)
            {
                eprintln!("Could not open the portal page: {error}");
            }
        }
    });
}

fn connect_auto_start(widgets: &Widgets, background_sender: mpsc::Sender<BackgroundResult>) {
    let guard = widgets.auto_start_guard.clone();
    let feedback = widgets.background_feedback.clone();
    widgets
        .auto_start_switch
        .connect_state_set(move |_, state| {
            if guard.get() {
                return glib::Propagation::Proceed;
            }
            show_feedback(&feedback, "Updating startup setting…");
            let sender = background_sender.clone();
            let requested = state;
            let _ = thread::Builder::new()
                .name("mugonat-autostart".to_owned())
                .spawn(move || {
                    let result = request_background(requested);
                    let background_result = match result {
                        Ok(()) => BackgroundResult {
                            enabled: requested,
                            failed: false,
                        },
                        Err(error) => {
                            eprintln!("Could not update login startup: {error:#}");
                            BackgroundResult {
                                enabled: false,
                                failed: true,
                            }
                        }
                    };
                    let _ = sender.send(background_result);
                });
            glib::Propagation::Stop
        });
}

fn request_background(enable: bool) -> Result<()> {
    let config_home = std::env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| std::env::var_os("HOME").map(|home| PathBuf::from(home).join(".config")))
        .context("Could not find your desktop settings folder")?;
    let autostart_dir = config_home.join("autostart");
    let entry_path = autostart_dir.join(format!("{APP_ID}.desktop"));

    if enable {
        fs::create_dir_all(&autostart_dir)
            .context("Could not create the desktop startup folder")?;
        let executable = std::env::current_exe().context("Could not locate Mugonat Portal")?;
        let exec_path = escape_desktop_entry_argument(&executable.to_string_lossy());
        let entry = format!(
            "[Desktop Entry]\nType=Application\nName=Mugonat Portal\nComment=Keep Wi-Fi sign-in available\nExec={exec_path} --background\nTerminal=false\nX-GNOME-Autostart-enabled=true\n"
        );
        let temporary_path = entry_path.with_extension("desktop.tmp");
        fs::write(&temporary_path, entry).context("Could not save the startup setting")?;
        fs::rename(&temporary_path, &entry_path)
            .context("Could not finish saving the startup setting")?;
    } else {
        match fs::remove_file(&entry_path) {
            Ok(()) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error).context("Could not remove the startup setting"),
        }
    }

    Ok(())
}

fn escape_desktop_entry_argument(argument: &str) -> String {
    let escaped = argument
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('$', "\\$")
        .replace('`', "\\`");
    format!("\"{escaped}\"")
}

fn apply_status(widgets: &Widgets, status: &AppStatus) {
    widgets.status_title.set_text(&status.title);
    widgets.status_detail.set_text(&status.detail);

    for class in ["status-online", "status-attention", "status-error"] {
        widgets.status_card.style_context().remove_class(class);
    }
    let (icon, state_class) = match &status.kind {
        StatusKind::Disconnected => ("network-wireless-symbolic", None),
        StatusKind::Waiting => ("network-wireless-symbolic", None),
        StatusKind::Online => ("network-wireless-symbolic", Some("status-online")),
        StatusKind::NeedsCredentials | StatusKind::Manual => {
            ("dialog-warning-symbolic", Some("status-attention"))
        }
        StatusKind::Error => ("dialog-error-symbolic", Some("status-error")),
    };
    widgets
        .status_icon
        .set_from_icon_name(Some(icon), gtk::IconSize::Dialog);
    if let Some(class) = state_class {
        widgets.status_card.style_context().add_class(class);
    }

    widgets
        .try_sign_in_button
        .set_visible(status.can_try_sign_in);
    widgets
        .try_sign_in_button
        .set_label(if status.kind == StatusKind::Online {
            "Check connection now"
        } else {
            "Try sign-in again"
        });

    widgets
        .portal_section
        .set_visible(status.portal_url.is_some());
    widgets
        .open_portal_button
        .set_sensitive(status.portal_url.is_some());
    widgets
        .portal_address
        .set_text(status.portal_url.as_deref().unwrap_or(""));
}

fn handle_status_attention(app: &gtk::Application, status: &AppStatus) {
    let needs_attention = matches!(
        status.kind,
        StatusKind::NeedsCredentials | StatusKind::Manual | StatusKind::Error
    );
    if !needs_attention {
        return;
    }

    let notification = gio::Notification::new(&status.title);
    notification.set_body(Some(&status.detail));
    app.send_notification(Some("portal-attention"), &notification);

    if matches!(
        status.kind,
        StatusKind::NeedsCredentials | StatusKind::Manual
    ) {
        if let Some(url) = status.portal_url.as_deref() {
            if let Err(error) =
                gio::AppInfo::launch_default_for_uri(url, None::<&gio::AppLaunchContext>)
            {
                eprintln!("Could not open the portal page: {error}");
            }
        }
    }
}

fn text_view_lines(view: &gtk::TextView) -> Vec<String> {
    let Some(buffer) = view.buffer() else {
        return Vec::new();
    };
    let Some(text) = buffer.text(&buffer.start_iter(), &buffer.end_iter(), false) else {
        return Vec::new();
    };
    let mut lines = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        if !line.is_empty() && !lines.iter().any(|known| known == line) {
            lines.push(line.to_owned());
        }
    }
    lines
}
