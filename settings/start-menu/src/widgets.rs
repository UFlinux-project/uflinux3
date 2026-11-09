use gtk::prelude::*;

pub fn tr(text: &str) -> glib::GString {
    glib::dgettext(Some("uflinux-settings"), text)
}
pub fn column() -> gtk::Box {
    gtk::Box::new(gtk::Orientation::Vertical, 8)
}
pub fn description(text: &str) -> gtk::Label {
    let label = gtk::Label::new(Some(&tr(text)));
    label.set_wrap(true);
    label.set_xalign(0.0);
    label.add_css_class("description");
    label
}
