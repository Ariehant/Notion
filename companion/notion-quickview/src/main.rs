//! Component C — the GTK4/libadwaita calendar quick-view.
//!
//! A lightweight native window (~8 MB, vs. booting the WebView editor) for
//! glancing at the week, quick-adding events, and the local-AI "Ask" mode. All
//! data + AI logic is delegated to the tested `notion-companion` crate (see
//! [`data`]); this file is GTK widget glue only.
//!
//! Launched by the GNOME Shell extension: bare `notion-quickview` opens the
//! agenda; `notion-quickview --ask` opens straight into the AI add dialog.
//!
//! NOT built in headless CI — needs libgtk-4-dev + libadwaita-1-dev. Build with:
//!   sudo apt-get install libgtk-4-dev libadwaita-1-dev libdbus-1-dev
//!   cargo build --release

mod data;

use std::rc::Rc;

use adw::prelude::*;
use gtk::{gio, glib};

use notion_companion::ai::Interpretation;
use notion_companion::dbaccess::AccessError;
use notion_companion::event::CompanionEvent;
use notion_companion::time as ctime;

const APP_ID: &str = "co.merai.notion.quickview";

fn main() -> glib::ExitCode {
    // Capture our own flags before GApplication sees them (we pass no args on).
    let ask_on_start = std::env::args().any(|a| a == "--ask");

    let app = adw::Application::builder().application_id(APP_ID).build();
    app.connect_activate(move |app| build_ui(app, ask_on_start));
    app.run_with_args::<&str>(&[])
}

fn build_ui(app: &adw::Application, ask_on_start: bool) {
    let window = adw::ApplicationWindow::builder()
        .application(app)
        .title("Notion Calendar")
        .default_width(420)
        .default_height(620)
        .build();

    let header = adw::HeaderBar::new();

    let add_button = gtk::Button::from_icon_name("list-add-symbolic");
    add_button.set_tooltip_text(Some("Quick add"));
    header.pack_start(&add_button);

    let ask_button = gtk::Button::builder()
        .label("Ask AI ✨")
        .css_classes(["suggested-action"])
        .build();
    header.pack_end(&ask_button);

    // The agenda list lives inside a scroller that fills the window body.
    let list = gtk::ListBox::builder()
        .selection_mode(gtk::SelectionMode::None)
        .css_classes(["boxed-list"])
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();

    let scroller = gtk::ScrolledWindow::builder()
        .hscrollbar_policy(gtk::PolicyType::Never)
        .vexpand(true)
        .child(&list)
        .build();

    let toolbar = adw::ToolbarView::new();
    toolbar.add_top_bar(&header);
    toolbar.set_content(Some(&scroller));
    window.set_content(Some(&toolbar));

    // A single shared refresh closure so every mutation re-renders the list.
    let refresh: Rc<dyn Fn()> = {
        let list = list.clone();
        Rc::new(move || populate(&list))
    };

    add_button.connect_clicked({
        let window = window.clone();
        let refresh = refresh.clone();
        move |_| open_quick_add(&window, refresh.clone())
    });
    ask_button.connect_clicked({
        let window = window.clone();
        let refresh = refresh.clone();
        move |_| open_ask_ai(&window, refresh.clone())
    });

    refresh();
    window.present();

    if ask_on_start {
        open_ask_ai(&window, refresh.clone());
    }
}

/// Clear and repopulate the agenda list from the shared DB.
fn populate(list: &gtk::ListBox) {
    while let Some(child) = list.first_child() {
        list.remove(&child);
    }

    let events = match data::load_week() {
        Ok(events) => events,
        Err(AccessError::Locked) => {
            list.append(&info_row(
                "Vault locked",
                "Unlock the Notion app to view your calendar.",
            ));
            return;
        }
        Err(e) => {
            list.append(&info_row("Couldn’t load events", &e.to_string()));
            return;
        }
    };

    if events.is_empty() {
        list.append(&info_row(
            "No events this week",
            "Add one with + or Ask AI ✨.",
        ));
        return;
    }

    let offset = ctime::local_offset_secs();
    let mut last_day = i64::MIN;
    for ev in &events {
        let day = (ev.start_time + offset).div_euclid(ctime::SECS_PER_DAY);
        if day != last_day {
            list.append(&day_header(ev.start_time));
            last_day = day;
        }
        list.append(&event_row(ev));
    }
}

/// A bold, non-selectable day separator ("Monday, 17 Nov").
fn day_header(start_time: i64) -> gtk::ListBoxRow {
    let label = gtk::Label::builder()
        .label(format_unix(start_time, "%A, %-d %b"))
        .xalign(0.0)
        .css_classes(["heading"])
        .margin_top(6)
        .margin_bottom(2)
        .build();
    let row = gtk::ListBoxRow::new();
    row.set_selectable(false);
    row.set_activatable(false);
    row.set_child(Some(&label));
    row
}

/// One event row: time · title (expanding) · delete button.
fn event_row(ev: &CompanionEvent) -> gtk::ListBoxRow {
    let hbox = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    hbox.set_margin_top(8);
    hbox.set_margin_bottom(8);
    hbox.set_margin_start(6);
    hbox.set_margin_end(6);

    let when = if ev.all_day {
        "all day".to_string()
    } else {
        format_unix(ev.start_time, "%H:%M")
    };
    let time_label = gtk::Label::builder()
        .label(when)
        .width_chars(6)
        .xalign(0.0)
        .css_classes(["dim-label", "numeric"])
        .build();

    let title_box = gtk::Box::new(gtk::Orientation::Vertical, 0);
    title_box.set_hexpand(true);
    let title = gtk::Label::builder()
        .label(&ev.title)
        .xalign(0.0)
        .ellipsize(gtk::pango::EllipsizeMode::End)
        .build();
    title_box.append(&title);
    if let Some(loc) = ev.location.as_deref().filter(|s| !s.is_empty()) {
        let loc_label = gtk::Label::builder()
            .label(loc)
            .xalign(0.0)
            .css_classes(["dim-label", "caption"])
            .ellipsize(gtk::pango::EllipsizeMode::End)
            .build();
        title_box.append(&loc_label);
    }

    let delete = gtk::Button::from_icon_name("user-trash-symbolic");
    delete.add_css_class("flat");
    delete.set_valign(gtk::Align::Center);
    let id = ev.id.clone();
    delete.connect_clicked(move |btn| {
        if let Err(e) = data::delete_event(&id) {
            eprintln!("notion-quickview: delete failed: {e}");
        }
        // Re-render by asking the enclosing ListBox to rebuild.
        if let Some(list) = btn
            .ancestor(gtk::ListBox::static_type())
            .and_downcast::<gtk::ListBox>()
        {
            populate(&list);
        }
    });

    hbox.append(&time_label);
    hbox.append(&title_box);
    hbox.append(&delete);

    let row = gtk::ListBoxRow::new();
    row.set_activatable(false);
    row.set_child(Some(&hbox));
    row
}

/// A centered informational row (locked / empty / error states).
fn info_row(title: &str, subtitle: &str) -> gtk::ListBoxRow {
    let vbox = gtk::Box::new(gtk::Orientation::Vertical, 4);
    vbox.set_margin_top(24);
    vbox.set_margin_bottom(24);
    vbox.append(
        &gtk::Label::builder()
            .label(title)
            .css_classes(["title-4"])
            .build(),
    );
    vbox.append(
        &gtk::Label::builder()
            .label(subtitle)
            .css_classes(["dim-label"])
            .wrap(true)
            .justify(gtk::Justification::Center)
            .build(),
    );
    let row = gtk::ListBoxRow::new();
    row.set_selectable(false);
    row.set_activatable(false);
    row.set_child(Some(&vbox));
    row
}

/// The quick-add form (title / date / time / duration), as a MessageDialog.
fn open_quick_add(parent: &adw::ApplicationWindow, refresh: Rc<dyn Fn()>) {
    let dialog = adw::MessageDialog::new(Some(parent), Some("New event"), None);
    dialog.add_responses(&[("cancel", "Cancel"), ("save", "Save")]);
    dialog.set_response_appearance("save", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("save"));

    let form = gtk::Box::new(gtk::Orientation::Vertical, 8);
    let title = gtk::Entry::builder().placeholder_text("Title").build();
    let date = gtk::Entry::builder()
        .text(format_unix(data::now_secs(), "%Y-%m-%d"))
        .build();
    let time = gtk::Entry::builder().text("09:00").build();
    let duration = gtk::SpinButton::with_range(15.0, 600.0, 15.0);
    duration.set_value(60.0);
    let location = gtk::Entry::builder()
        .placeholder_text("Location (optional)")
        .build();

    for (caption, widget) in [
        ("Title", title.clone().upcast::<gtk::Widget>()),
        ("Date (YYYY-MM-DD)", date.clone().upcast()),
        ("Time (HH:MM)", time.clone().upcast()),
        ("Duration (min)", duration.clone().upcast()),
        ("Location", location.clone().upcast()),
    ] {
        form.append(
            &gtk::Label::builder()
                .label(caption)
                .xalign(0.0)
                .css_classes(["caption", "dim-label"])
                .build(),
        );
        form.append(&widget);
    }
    dialog.set_extra_child(Some(&form));

    let parent = parent.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "save" {
            return;
        }
        let built = data::build_quick_event(
            title.text().as_str(),
            date.text().as_str(),
            time.text().as_str(),
            duration.value() as i64,
            Some(location.text().to_string()),
        );
        match built {
            Ok(ev) => match data::save_event(&ev) {
                Ok(()) => refresh(),
                Err(e) => show_error(&parent, "Could not save", &e.to_string()),
            },
            Err(msg) => show_error(&parent, "Invalid event", &msg),
        }
    });
    dialog.present();
}

/// The local-AI add dialog: free text → Ollama → confirm/conflicts → save.
fn open_ask_ai(parent: &adw::ApplicationWindow, refresh: Rc<dyn Fn()>) {
    let dialog = adw::MessageDialog::new(
        Some(parent),
        Some("Ask AI ✨"),
        Some("Describe the event in plain language."),
    );
    dialog.add_responses(&[("cancel", "Cancel"), ("add", "Add")]);
    dialog.set_response_appearance("add", adw::ResponseAppearance::Suggested);
    dialog.set_default_response(Some("add"));

    let entry = gtk::Entry::builder()
        .placeholder_text("e.g. Dentist next Tuesday at 10am for 30 min")
        .activates_default(true)
        .build();
    dialog.set_extra_child(Some(&entry));

    let parent = parent.clone();
    dialog.connect_response(None, move |_, response| {
        if response != "add" {
            return;
        }
        let text = entry.text().to_string();
        if text.trim().is_empty() {
            return;
        }
        let parent = parent.clone();
        let refresh = refresh.clone();
        // Run the blocking DB + HTTP work off the GTK main thread, then resume.
        glib::spawn_future_local(async move {
            let outcome = gio::spawn_blocking(move || data::ask_ai(&text)).await;
            match outcome {
                Ok(Ok(interp)) => confirm_and_save(&parent, interp, refresh),
                Ok(Err(msg)) => show_error(&parent, "AI couldn’t help", &msg),
                Err(_) => show_error(&parent, "AI error", "The background task failed."),
            }
        });
    });
    dialog.present();
}

/// Save immediately when clear, or ask the user to confirm over conflicts.
fn confirm_and_save(
    parent: &adw::ApplicationWindow,
    interp: Interpretation,
    refresh: Rc<dyn Fn()>,
) {
    if interp.conflicts.is_empty() {
        finish_save(parent, &interp.event, &refresh);
        return;
    }

    let lines: Vec<String> = interp
        .conflicts
        .iter()
        .map(|c| format!("• {} at {}", c.title, format_unix(c.start_time, "%a %H:%M")))
        .collect();
    let body = format!(
        "“{}” overlaps {} existing event(s):\n{}",
        interp.event.title,
        interp.conflicts.len(),
        lines.join("\n")
    );
    let dialog = adw::MessageDialog::new(Some(parent), Some("Schedule anyway?"), Some(&body));
    dialog.add_responses(&[("cancel", "Cancel"), ("save", "Add anyway")]);
    dialog.set_response_appearance("save", adw::ResponseAppearance::Destructive);

    let parent = parent.clone();
    let event = interp.event;
    dialog.connect_response(None, move |_, response| {
        if response == "save" {
            finish_save(&parent, &event, &refresh);
        }
    });
    dialog.present();
}

fn finish_save(parent: &adw::ApplicationWindow, event: &CompanionEvent, refresh: &Rc<dyn Fn()>) {
    match data::save_event(event) {
        Ok(()) => refresh(),
        Err(e) => show_error(parent, "Could not save", &e.to_string()),
    }
}

/// A simple OK error dialog.
fn show_error(parent: &adw::ApplicationWindow, heading: &str, body: &str) {
    let dialog = adw::MessageDialog::new(Some(parent), Some(heading), Some(body));
    dialog.add_response("ok", "OK");
    dialog.set_default_response(Some("ok"));
    dialog.present();
}

/// Format a Unix-second instant in local time with a strftime pattern.
fn format_unix(ts: i64, fmt: &str) -> String {
    glib::DateTime::from_unix_local(ts)
        .and_then(|dt| dt.format(fmt))
        .map(|s| s.to_string())
        .unwrap_or_default()
}
