//! Session polkit listener. Its blocking registration and PAM I/O live on a
//! dedicated GLib context; only owned prompt strings cross to the GTK thread.
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
struct Event {
    kind: i32,
    text: String,
    message: String,
    echo: i32,
}
enum Command {
    Respond(Secret),
    Cancel,
    Stop,
}
struct Secret(String);
impl Drop for Secret {
    fn drop(&mut self) {
        // Volatile writes keep the clearing operation observable to the compiler.
        for byte in unsafe { self.0.as_bytes_mut() } {
            unsafe { std::ptr::write_volatile(byte, 0) };
        }
    }
}
pub struct Agent {
    commands: async_channel::Sender<Command>,
}
impl Agent {
    pub fn register(app: Weak<App>) -> Self {
        let (commands, receiver) = async_channel::bounded::<Command>(8);
        let (events, incoming) = async_channel::bounded::<Event>(8);
        let result = std::thread::Builder::new()
            .name("desktop-polkit".into())
            .spawn(move || {
                let context = glib::MainContext::new();
                let _ = context.with_thread_default(|| {
                    let mainloop = glib::MainLoop::new(Some(&context), false);
                    let data = Box::into_raw(Box::new(events));
                    let ending = mainloop.clone();
                    context.spawn_local(async move {
                        let mut ptr: *mut c_void = std::ptr::null_mut();
                        loop {
                            if ptr.is_null() {
                                ptr = unsafe { uf_polkit_register(event, data.cast()) };
                            }
                            let command = if ptr.is_null() {
                                use futures_util::future::{Either, select};
                                match select(
                                    Box::pin(receiver.recv()),
                                    glib::timeout_future_seconds(5),
                                )
                                .await
                                {
                                    Either::Left((Ok(command), _)) => command,
                                    Either::Left((Err(_), _)) => break,
                                    Either::Right(_) => continue,
                                }
                            } else {
                                match receiver.recv().await {
                                    Ok(command) => command,
                                    Err(_) => break,
                                }
                            };
                            match command {
                                Command::Respond(text) => {
                                    if let Ok(response) = CString::new(text.0.as_str()) {
                                        unsafe {
                                            uf_polkit_response(ptr, response.as_ptr());
                                        }
                                        let mut bytes = response.into_bytes_with_nul();
                                        for byte in &mut bytes {
                                            unsafe { std::ptr::write_volatile(byte, 0) };
                                        }
                                    }
                                }
                                Command::Cancel => unsafe { uf_polkit_cancel(ptr) },
                                Command::Stop => break,
                            }
                        }
                        unsafe {
                            uf_polkit_unregister(ptr);
                        }
                        ending.quit();
                    });
                    mainloop.run();
                    unsafe {
                        drop(Box::from_raw(data));
                    }
                });
            });
        if let Err(error) = result {
            eprintln!("Could not start authorization worker: {error}");
        }
        glib::MainContext::default().spawn_local(async move {
            while let Ok(event) = incoming.recv().await {
                handle_event(&app, event);
            }
        });
        Self { commands }
    }
    fn respond(&self, text: &str) {
        let _ = self
            .commands
            .try_send(Command::Respond(Secret(text.to_string())));
    }
    fn cancel(&self) {
        let _ = self.commands.try_send(Command::Cancel);
    }
}
impl Drop for Agent {
    fn drop(&mut self) {
        let _ = self.commands.try_send(Command::Stop);
    }
}
unsafe extern "C" fn event(
    kind: c_int,
    text: *const c_char,
    message: *const c_char,
    echo: c_int,
    data: *mut c_void,
) {
    let sender = unsafe { &*data.cast::<async_channel::Sender<Event>>() };
    let copy = |value: *const c_char| {
        if value.is_null() {
            String::new()
        } else {
            unsafe { CStr::from_ptr(value) }
                .to_string_lossy()
                .into_owned()
        }
    };
    let _ = sender.try_send(Event {
        kind,
        text: copy(text),
        message: copy(message),
        echo,
    });
}
thread_local! {static DIALOG:RefCell<Option<gtk::Window>>=const{RefCell::new(None)};}
fn handle_event(weak: &Weak<App>, event: Event) {
    let Some(app) = weak.upgrade() else { return };
    let Event {
        kind,
        text,
        message,
        echo,
    } = event;
    if kind == 2 {
        app.authorization_active.set(false);
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
    app.authorization_active.set(true);
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
    let field = entry.clone();
    cancel.connect_clicked(move |_| {
        field.set_text("");
        if let Some(app) = weak.upgrade() {
            if let Some(agent) = &*app.agent.borrow() {
                agent.cancel();
            }
        }
    });
    let weak = Rc::downgrade(&app);
    let field = entry.clone();
    window.connect_close_request(move |_| {
        field.set_text("");
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
