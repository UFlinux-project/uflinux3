use crate::app::App;
use gtk::prelude::*;
use std::{
    cell::RefCell,
    ffi::{CStr, CString, c_char, c_int, c_void},
    rc::{Rc, Weak},
};

unsafe extern "C" {
    fn uf_polkit_register(
        callback: unsafe extern "C" fn(c_int, *const c_char, *const c_char, c_int, *mut c_void),
        data: *mut c_void,
    ) -> *mut c_void;
    fn uf_polkit_response(agent: *mut c_void, response: *const c_char);
    fn uf_polkit_cancel(agent: *mut c_void);
    fn uf_polkit_unregister(agent: *mut c_void);
}
pub struct Agent {
    ptr: *mut c_void,
    data: *mut Weak<App>,
}
impl Agent {
    pub fn register(app: Weak<App>) -> Self {
        let data = Box::into_raw(Box::new(app));
        let ptr = unsafe { uf_polkit_register(event, data.cast()) };
        Self { ptr, data }
    }
    fn respond(&self, text: &str) {
        if let Ok(text) = CString::new(text) {
            unsafe {
                uf_polkit_response(self.ptr, text.as_ptr());
            }
        }
    }
    fn cancel(&self) {
        unsafe {
            uf_polkit_cancel(self.ptr);
        }
    }
}
impl Drop for Agent {
    fn drop(&mut self) {
        unsafe {
            uf_polkit_unregister(self.ptr);
            drop(Box::from_raw(self.data));
        }
    }
}
thread_local! {static DIALOG:RefCell<Option<gtk::Window>>=const{RefCell::new(None)};}
pub fn active() -> bool {
    DIALOG.with(|dialog| {
        dialog
            .borrow()
            .as_ref()
            .is_some_and(|window| window.is_visible())
    })
}
unsafe extern "C" fn event(
    kind: c_int,
    text: *const c_char,
    message: *const c_char,
    echo: c_int,
    data: *mut c_void,
) {
    let weak = unsafe { &*data.cast::<Weak<App>>() };
    let Some(app) = weak.upgrade() else {
        return;
    };
    let text = unsafe { CStr::from_ptr(text) }
        .to_string_lossy()
        .into_owned();
    let message = unsafe { CStr::from_ptr(message) }
        .to_string_lossy()
        .into_owned();
    if kind == 2 {
        DIALOG.with(|d| {
            if let Some(window) = d.borrow_mut().take() {
                window.destroy();
            }
        });
        return;
    }
    if kind == 1 {
        app.error(&text);
        return;
    }
    DIALOG.with(|d| {
        if let Some(window) = d.borrow_mut().take() {
            window.destroy();
        }
    });
    let window = gtk::ApplicationWindow::builder()
        .application(&app.application)
        .title("Administrator authorization")
        .default_width(380)
        .resizable(false)
        .modal(true)
        .build();
    if let Some(parent) = app.parent() {
        window.set_transient_for(Some(&parent));
    }
    let column = crate::widgets::column();
    column.set_margin_top(24);
    column.set_margin_bottom(24);
    column.set_margin_start(24);
    column.set_margin_end(24);
    let heading = crate::widgets::description(&message);
    column.append(&heading);
    let entry = gtk::Entry::builder()
        .visibility(echo != 0)
        .placeholder_text(&text)
        .activates_default(true)
        .build();
    entry.set_input_purpose(if echo != 0 {
        gtk::InputPurpose::FreeForm
    } else {
        gtk::InputPurpose::Password
    });
    column.append(&entry);
    let buttons = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    let cancel = gtk::Button::with_label("Cancel");
    let authenticate = gtk::Button::with_label("Authenticate");
    authenticate.add_css_class("suggested-action");
    buttons.append(&cancel);
    buttons.append(&authenticate);
    column.append(&buttons);
    let weak = Rc::downgrade(&app);
    let field = entry.clone();
    authenticate.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            if let Some(agent) = &*app.agent.borrow() {
                agent.respond(&field.text());
                field.set_text("");
            }
        }
    });
    let weak = Rc::downgrade(&app);
    cancel.connect_clicked(move |_| {
        if let Some(app) = weak.upgrade() {
            if let Some(agent) = &*app.agent.borrow() {
                agent.cancel();
            }
        }
    });
    let weak = Rc::downgrade(&app);
    window.connect_close_request(move |_| {
        if let Some(app) = weak.upgrade() {
            if let Some(agent) = &*app.agent.borrow() {
                agent.cancel();
            }
        }
        glib::Propagation::Proceed
    });
    window.set_child(Some(&column));
    window.set_default_widget(Some(&authenticate));
    window.present();
    entry.grab_focus();
    DIALOG.with(|d| d.replace(Some(window.upcast())));
}
