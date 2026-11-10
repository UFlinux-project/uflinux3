//! Event-driven foreign-toplevel tracking on a dedicated Wayland thread.
use crate::taskbar::{Window, WindowId};
use std::{
    collections::HashMap,
    io::{Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::{Arc, Mutex, mpsc},
    thread,
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle,
    protocol::{wl_registry, wl_seat},
};
use wayland_protocols_wlr::foreign_toplevel::v1::client::{
    zwlr_foreign_toplevel_handle_v1::{self as handle, ZwlrForeignToplevelHandleV1},
    zwlr_foreign_toplevel_manager_v1::{self as manager, ZwlrForeignToplevelManagerV1},
};

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct Snapshot {
    pub windows: Vec<Window>,
    pub error: Option<String>,
}
#[derive(Clone, Copy, Debug)]
pub enum Command {
    Activate(WindowId),
    Minimize(WindowId),
    Close(WindowId),
    Stop,
}

/// Notifications are coalesced: read the latest snapshot when one arrives.
pub struct Client {
    commands: mpsc::SyncSender<Command>,
    wake: UnixStream,
    snapshot: Arc<Mutex<Snapshot>>,
}
impl Client {
    pub fn start() -> std::io::Result<(Self, async_channel::Receiver<()>)> {
        let (commands, receiver) = mpsc::sync_channel(128);
        let (notify, notifications) = async_channel::bounded(1);
        let (wake, reader) = UnixStream::pair()?;
        wake.set_nonblocking(true)?;
        reader.set_nonblocking(true)?;
        let snapshot = Arc::new(Mutex::new(Snapshot::default()));
        let shared = snapshot.clone();
        thread::Builder::new()
            .name("desktop-toplevel".into())
            .spawn(move || {
                if let Err(error) = worker(reader, receiver, notify.clone(), shared.clone()) {
                    if let Ok(mut view) = shared.lock() {
                        view.windows.clear();
                        view.error = Some(error);
                    }
                    let _ = notify.try_send(());
                }
            })?;
        Ok((
            Self {
                commands,
                wake,
                snapshot,
            },
            notifications,
        ))
    }
    pub fn snapshot(&self) -> Snapshot {
        self.snapshot
            .lock()
            .map(|v| v.clone())
            .unwrap_or_else(|_| Snapshot {
                windows: vec![],
                error: Some("Window tracker stopped".into()),
            })
    }
    pub fn send(&mut self, command: Command) -> Result<(), String> {
        self.commands
            .try_send(command)
            .map_err(|e| format!("Window tracker unavailable: {e}"))?;
        match self.wake.write(&[1]) {
            Ok(_) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::WouldBlock => Ok(()),
            Err(e) => Err(e.to_string()),
        }
    }
}
impl Drop for Client {
    fn drop(&mut self) {
        let _ = self.send(Command::Stop);
    }
}
struct Item {
    proxy: ZwlrForeignToplevelHandleV1,
    window: Window,
    committed: Option<Window>,
}
struct State {
    manager: Option<ZwlrForeignToplevelManagerV1>,
    seat: Option<(u32, wl_seat::WlSeat)>,
    items: HashMap<u32, Item>,
    next: WindowId,
    finished: bool,
    shared: Arc<Mutex<Snapshot>>,
    notify: async_channel::Sender<()>,
}
impl State {
    fn publish(&self) {
        let mut windows: Vec<_> = self
            .items
            .values()
            .filter_map(|i| i.committed.clone())
            .collect();
        windows.sort_by_key(|w| w.id);
        if let Ok(mut view) = self.shared.lock() {
            view.windows = windows;
            view.error = None;
        }
        let _ = self.notify.try_send(());
    }
}
fn worker(
    mut wake: UnixStream,
    commands: mpsc::Receiver<Command>,
    notify: async_channel::Sender<()>,
    shared: Arc<Mutex<Snapshot>>,
) -> Result<(), String> {
    let connection = Connection::connect_to_env().map_err(|e| e.to_string())?;
    let mut queue = connection.new_event_queue::<State>();
    let qh = queue.handle();
    let _registry = connection.display().get_registry(&qh, ());
    let mut state = State {
        manager: None,
        seat: None,
        items: HashMap::new(),
        next: 1,
        finished: false,
        shared,
        notify,
    };
    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    if state.manager.is_none() {
        return Err("Foreign-toplevel management is unavailable".into());
    }
    state.publish();
    loop {
        queue
            .dispatch_pending(&mut state)
            .map_err(|e| e.to_string())?;
        if state.finished {
            return Err("Foreign-toplevel management stopped".into());
        }
        loop {
            match commands.try_recv() {
                Ok(Command::Stop) | Err(mpsc::TryRecvError::Disconnected) => return Ok(()),
                Ok(command) => {
                    let id = match command {
                        Command::Activate(id) | Command::Minimize(id) | Command::Close(id) => id,
                        Command::Stop => unreachable!(),
                    };
                    if let Some(item) = state.items.values().find(|item| item.window.id == id) {
                        match command {
                            Command::Activate(_) => {
                                if let Some((_, seat)) = &state.seat {
                                    if item.window.minimized {
                                        item.proxy.unset_minimized();
                                    }
                                    item.proxy.activate(seat);
                                }
                            }
                            Command::Minimize(_) => item.proxy.set_minimized(),
                            Command::Close(_) => item.proxy.close(),
                            Command::Stop => unreachable!(),
                        }
                    }
                }
                Err(mpsc::TryRecvError::Empty) => break,
            }
        }
        connection.flush().map_err(|e| e.to_string())?;
        let Some(guard) = queue.prepare_read() else {
            continue;
        };
        let mut fds = [
            libc::pollfd {
                fd: connection.backend().poll_fd().as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
            libc::pollfd {
                fd: wake.as_raw_fd(),
                events: libc::POLLIN,
                revents: 0,
            },
        ];
        let ready = unsafe { libc::poll(fds.as_mut_ptr(), fds.len() as libc::nfds_t, -1) };
        if ready < 0 {
            drop(guard);
            let e = std::io::Error::last_os_error();
            if e.kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err(e.to_string());
        }
        if fds[1].revents != 0 {
            let mut bytes = [0; 128];
            while wake.read(&mut bytes).is_ok() {}
        }
        if fds[0].revents & libc::POLLIN != 0 {
            guard.read().map_err(|e| e.to_string())?;
        } else {
            drop(guard);
        }
        if fds[0].revents & (libc::POLLERR | libc::POLLHUP | libc::POLLNVAL) != 0 {
            return Err("Wayland disconnected".into());
        }
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        s: &mut Self,
        registry: &wl_registry::WlRegistry,
        e: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        match e {
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "zwlr_foreign_toplevel_manager_v1" && s.manager.is_none() => {
                s.manager = Some(registry.bind(name, version.min(3), qh, ()))
            }
            wl_registry::Event::Global {
                name,
                interface,
                version,
            } if interface == "wl_seat" && s.seat.is_none() => {
                s.seat = Some((name, registry.bind(name, version.min(7), qh, ())))
            }
            wl_registry::Event::GlobalRemove { name }
                if s.seat.as_ref().is_some_and(|(id, _)| *id == name) =>
            {
                s.seat = None
            }
            _ => {}
        }
    }
}
impl Dispatch<wl_seat::WlSeat, ()> for State {
    fn event(
        _: &mut Self,
        _: &wl_seat::WlSeat,
        _: wl_seat::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
impl Dispatch<ZwlrForeignToplevelManagerV1, ()> for State {
    fn event(
        s: &mut Self,
        _: &ZwlrForeignToplevelManagerV1,
        e: manager::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match e {
            manager::Event::Toplevel { toplevel } => {
                let id = s.next;
                s.next += 1;
                s.items.insert(
                    toplevel.id().protocol_id(),
                    Item {
                        proxy: toplevel,
                        window: Window {
                            id,
                            app_id: String::new(),
                            title: String::new(),
                            focused: false,
                            minimized: false,
                        },
                        committed: None,
                    },
                );
            }
            manager::Event::Finished => s.finished = true,
            _ => {}
        }
    }
    wayland_client::event_created_child!(State, ZwlrForeignToplevelManagerV1, [manager::EVT_TOPLEVEL_OPCODE => (ZwlrForeignToplevelHandleV1, ())]);
}
impl Dispatch<ZwlrForeignToplevelHandleV1, ()> for State {
    fn event(
        s: &mut Self,
        proxy: &ZwlrForeignToplevelHandleV1,
        e: handle::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = proxy.id().protocol_id();
        if matches!(e, handle::Event::Closed) {
            s.items.remove(&id);
            proxy.destroy();
            s.publish();
            return;
        }
        let Some(item) = s.items.get_mut(&id) else {
            return;
        };
        match e {
            handle::Event::Title { title } => item.window.title = title,
            handle::Event::AppId { app_id } => item.window.app_id = app_id,
            handle::Event::State { state } => {
                let values: Vec<_> = state
                    .chunks_exact(4)
                    .map(|c| u32::from_ne_bytes(c.try_into().unwrap()))
                    .collect();
                item.window.minimized = values.contains(&(handle::State::Minimized as u32));
                item.window.focused = values.contains(&(handle::State::Activated as u32));
            }
            handle::Event::Done => {
                item.committed = Some(item.window.clone());
                s.publish();
            }
            _ => {}
        }
    }
}
