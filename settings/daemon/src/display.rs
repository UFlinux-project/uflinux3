//! Wayland runs on one sleeping worker, woken by its fd or an explicit command.
use settings_core::*;
use std::{
    collections::HashMap,
    io::{Read, Write},
    os::{fd::AsRawFd, unix::net::UnixStream},
    sync::{
        Arc, Mutex,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
use wayland_client::{
    Connection, Dispatch, Proxy, QueueHandle, WEnum,
    protocol::{wl_output, wl_registry},
};
use wayland_protocols_wlr::output_management::v1::client::{
    zwlr_output_configuration_head_v1::{self, ZwlrOutputConfigurationHeadV1},
    zwlr_output_configuration_v1::{self, ZwlrOutputConfigurationV1},
    zwlr_output_head_v1::{self, ZwlrOutputHeadV1},
    zwlr_output_manager_v1::{self, ZwlrOutputManagerV1},
    zwlr_output_mode_v1::{self, ZwlrOutputModeV1},
};

pub enum Event {
    Outputs(Vec<Output>),
    Unavailable(String),
}
enum Command {
    Apply(
        Vec<OutputConfig>,
        async_channel::Sender<Result<()>>,
        Arc<AtomicBool>,
    ),
}
#[derive(Clone)]
pub struct Client {
    tx: mpsc::Sender<Command>,
    wake: Arc<Mutex<UnixStream>>,
}
struct Head {
    proxy: ZwlrOutputHeadV1,
    view: Output,
    modes: HashMap<u32, ZwlrOutputModeV1>,
    current: Option<u32>,
}
struct Pending {
    configs: Vec<OutputConfig>,
    previous: Vec<OutputConfig>,
    canceled: Arc<AtomicBool>,
    reply: async_channel::Sender<Result<()>>,
    tested: bool,
}
struct State {
    manager: Option<ZwlrOutputManagerV1>,
    serial: u32,
    heads: HashMap<u32, Head>,
    modes: HashMap<u32, Mode>,
    pending: HashMap<u32, Pending>,
    events: async_channel::Sender<Event>,
}
impl Client {
    pub fn start(events: async_channel::Sender<Event>) -> Result<Self> {
        let (tx, rx) = mpsc::channel();
        let (wake, reader) = UnixStream::pair().map_err(|e| e.to_string())?;
        reader.set_nonblocking(true).map_err(|e| e.to_string())?;
        std::thread::Builder::new()
            .name("settings-wayland".into())
            .spawn(move || {
                if let Err(error) = worker(reader, rx, events.clone()) {
                    let _ = events.send_blocking(Event::Unavailable(error));
                }
            })
            .map_err(|e| e.to_string())?;
        Ok(Self {
            tx,
            wake: Arc::new(Mutex::new(wake)),
        })
    }
    pub async fn apply(&self, configs: Vec<OutputConfig>) -> Result<()> {
        let (send, recv) = async_channel::bounded(1);
        let canceled = Arc::new(AtomicBool::new(false));
        let cancellation = canceled.clone();
        self.tx
            .send(Command::Apply(configs, send, canceled))
            .map_err(|_| "Display management is unavailable")?;
        self.wake
            .lock()
            .map_err(|_| "Display worker stopped")?
            .write_all(&[1])
            .map_err(|e| e.to_string())?;
        // The worker always replies on protocol success, rejection, cancellation or disconnect.
        let result = gio::spawn_blocking(move || {
            // Avoid blocking the GLib main thread while waiting for compositor events.
            let began = std::time::Instant::now();
            loop {
                match recv.try_recv() {
                    Ok(v) => return v,
                    Err(async_channel::TryRecvError::Closed) => {
                        return Err("Display operation canceled".into());
                    }
                    _ => {}
                }
                if began.elapsed() > std::time::Duration::from_secs(8) {
                    cancellation.store(true, Ordering::Release);
                    return Err("The compositor did not respond to the display change".into());
                }
                std::thread::sleep(std::time::Duration::from_millis(10));
            }
        })
        .await
        .map_err(|_| "Display operation failed".to_string())?;
        result
    }
}
fn worker(
    mut wake: UnixStream,
    rx: mpsc::Receiver<Command>,
    events: async_channel::Sender<Event>,
) -> Result<()> {
    let connection = Connection::connect_to_env().map_err(|_| "No Wayland session is available")?;
    let mut queue = connection.new_event_queue::<State>();
    let qh = queue.handle();
    let _registry = connection.display().get_registry(&qh, ());
    let mut state = State {
        manager: None,
        serial: 0,
        heads: HashMap::new(),
        modes: HashMap::new(),
        pending: HashMap::new(),
        events,
    };
    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    queue.roundtrip(&mut state).map_err(|e| e.to_string())?;
    if state.manager.is_none() {
        return Err("This compositor does not expose output management".into());
    }
    loop {
        queue
            .dispatch_pending(&mut state)
            .map_err(|e| e.to_string())?;
        while let Ok(command) = rx.try_recv() {
            match command {
                Command::Apply(configs, reply, canceled) => {
                    let result = validate::displays(&configs, &state.outputs());
                    if let Err(e) = result {
                        let _ = reply.send_blocking(Err(e));
                        continue;
                    }
                    if !state.pending.is_empty() {
                        let _ =
                            reply.send_blocking(Err("Another display change is pending".into()));
                        continue;
                    }
                    state.configure(
                        Pending {
                            configs,
                            previous: state.outputs().iter().map(Output::config).collect(),
                            canceled,
                            reply,
                            tested: false,
                        },
                        &qh,
                    )?;
                }
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
        let result = unsafe { libc::poll(fds.as_mut_ptr(), 2, -1) };
        if result < 0 {
            drop(guard);
            if std::io::Error::last_os_error().kind() == std::io::ErrorKind::Interrupted {
                continue;
            }
            return Err("Display event polling failed".into());
        }
        if fds[1].revents != 0 {
            let mut bytes = [0; 64];
            while wake.read(&mut bytes).is_ok() {}
        }
        if fds[0].revents & libc::POLLIN != 0 {
            guard.read().map_err(|e| e.to_string())?;
        } else {
            drop(guard);
        }
        if fds[0].revents & (libc::POLLHUP | libc::POLLERR) != 0 {
            return Err("Wayland disconnected".into());
        }
    }
}
impl State {
    fn outputs(&self) -> Vec<Output> {
        let mut outputs = self
            .heads
            .values()
            .map(|h| {
                let mut v = h.view.clone();
                v.current = h.current.and_then(|id| self.modes.get(&id).cloned());
                v.modes = h
                    .modes
                    .keys()
                    .filter_map(|id| self.modes.get(id).cloned())
                    .filter(|m| m.width > 0 && m.height > 0)
                    .collect();
                v.modes.sort_by_key(|m| (-m.width, -m.height, -m.refresh));
                v
            })
            .collect::<Vec<_>>();
        outputs.sort_by(|a, b| a.name.cmp(&b.name));
        outputs
    }
    fn publish(&self) {
        let _ = self.events.send_blocking(Event::Outputs(self.outputs()));
    }
    fn configure(&mut self, pending: Pending, qh: &QueueHandle<Self>) -> Result<()> {
        let manager = self.manager.as_ref().ok_or("Output manager disappeared")?;
        let config = manager.create_configuration(self.serial, qh, ());
        for c in &pending.configs {
            let h = self
                .heads
                .values()
                .find(|h| h.view.name == c.name)
                .ok_or("Display disappeared")?;
            if !c.enabled {
                config.disable_head(&h.proxy);
                continue;
            }
            let head = config.enable_head(&h.proxy, qh, ());
            let mode = c.mode.as_ref().ok_or("No display mode selected")?;
            let mode_id = h
                .modes
                .keys()
                .find(|id| {
                    self.modes.get(id).is_some_and(|m| {
                        m.width == mode.width
                            && m.height == mode.height
                            && m.refresh == mode.refresh
                    })
                })
                .ok_or("Display mode disappeared")?;
            head.set_mode(&h.modes[mode_id]);
            head.set_position(c.x, c.y);
            head.set_scale(c.scale);
            let transform = wl_output::Transform::try_from(c.transform)
                .map_err(|_| "Invalid display orientation")?;
            head.set_transform(transform);
            if head.version() >= 4 {
                head.set_adaptive_sync(if c.adaptive_sync {
                    zwlr_output_head_v1::AdaptiveSyncState::Enabled
                } else {
                    zwlr_output_head_v1::AdaptiveSyncState::Disabled
                });
            }
        }
        if pending.tested {
            config.apply();
        } else {
            config.test();
        }
        self.pending.insert(config.id().protocol_id(), pending);
        Ok(())
    }
}
impl Dispatch<wl_registry::WlRegistry, ()> for State {
    fn event(
        state: &mut Self,
        registry: &wl_registry::WlRegistry,
        event: wl_registry::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        if let wl_registry::Event::Global {
            name,
            interface,
            version,
        } = event
        {
            if interface == "zwlr_output_manager_v1" {
                state.manager = Some(registry.bind(name, version.min(4), qh, ()));
            }
        }
    }
}
impl Dispatch<ZwlrOutputManagerV1, ()> for State {
    fn event(
        state: &mut Self,
        _: &ZwlrOutputManagerV1,
        event: zwlr_output_manager_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        match event {
            zwlr_output_manager_v1::Event::Head { head } => {
                let control = head.version() >= 4;
                state.heads.insert(
                    head.id().protocol_id(),
                    Head {
                        proxy: head,
                        view: Output {
                            scale: 1.,
                            adaptive_sync_control: control,
                            ..Output::default()
                        },
                        modes: HashMap::new(),
                        current: None,
                    },
                );
            }
            zwlr_output_manager_v1::Event::Done { serial } => {
                state.serial = serial;
                state.publish();
            }
            zwlr_output_manager_v1::Event::Finished => {
                state.manager = None;
                let _ = state
                    .events
                    .send_blocking(Event::Unavailable("Output management ended".into()));
            }
            _ => {}
        }
    }
    wayland_client::event_created_child!(State,ZwlrOutputManagerV1,[zwlr_output_manager_v1::EVT_HEAD_OPCODE=>(ZwlrOutputHeadV1,())]);
}
impl Dispatch<ZwlrOutputHeadV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputHeadV1,
        event: zwlr_output_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = proxy.id().protocol_id();
        if matches!(event, zwlr_output_head_v1::Event::Finished) {
            state.heads.remove(&id);
            return;
        }
        let Some(head) = state.heads.get_mut(&id) else {
            return;
        };
        match event {
            zwlr_output_head_v1::Event::Name { name } => head.view.name = name,
            zwlr_output_head_v1::Event::Make { make } => head.view.make = make,
            zwlr_output_head_v1::Event::Model { model } => head.view.model = model,
            zwlr_output_head_v1::Event::SerialNumber { serial_number } => {
                head.view.serial = serial_number
            }
            zwlr_output_head_v1::Event::Enabled { enabled } => head.view.enabled = enabled != 0,
            zwlr_output_head_v1::Event::Position { x, y } => {
                head.view.x = x;
                head.view.y = y;
            }
            zwlr_output_head_v1::Event::Scale { scale } => head.view.scale = scale,
            zwlr_output_head_v1::Event::Transform {
                transform: WEnum::Value(value),
            } => head.view.transform = value as u32,
            zwlr_output_head_v1::Event::AdaptiveSync {
                state: WEnum::Value(value),
            } => head.view.adaptive_sync = value == zwlr_output_head_v1::AdaptiveSyncState::Enabled,
            zwlr_output_head_v1::Event::CurrentMode { mode } => {
                head.current = Some(mode.id().protocol_id())
            }
            zwlr_output_head_v1::Event::Mode { mode } => {
                let id = mode.id().protocol_id();
                head.modes.insert(id, mode);
                state.modes.insert(id, Mode::default());
            }
            _ => {}
        }
    }
    wayland_client::event_created_child!(State,ZwlrOutputHeadV1,[zwlr_output_head_v1::EVT_MODE_OPCODE=>(ZwlrOutputModeV1,())]);
}
impl Dispatch<ZwlrOutputModeV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputModeV1,
        event: zwlr_output_mode_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
        let id = proxy.id().protocol_id();
        if matches!(event, zwlr_output_mode_v1::Event::Finished) {
            state.modes.remove(&id);
            for h in state.heads.values_mut() {
                h.modes.remove(&id);
            }
            return;
        }
        let Some(mode) = state.modes.get_mut(&id) else {
            return;
        };
        match event {
            zwlr_output_mode_v1::Event::Size { width, height } => {
                mode.width = width;
                mode.height = height;
            }
            zwlr_output_mode_v1::Event::Refresh { refresh } => mode.refresh = refresh,
            zwlr_output_mode_v1::Event::Preferred => mode.preferred = true,
            _ => {}
        }
    }
}
impl Dispatch<ZwlrOutputConfigurationV1, ()> for State {
    fn event(
        state: &mut Self,
        proxy: &ZwlrOutputConfigurationV1,
        event: zwlr_output_configuration_v1::Event,
        _: &(),
        _: &Connection,
        qh: &QueueHandle<Self>,
    ) {
        let Some(mut pending) = state.pending.remove(&proxy.id().protocol_id()) else {
            return;
        };
        proxy.destroy();
        if pending.canceled.load(Ordering::Acquire) {
            if pending.tested && matches!(event, zwlr_output_configuration_v1::Event::Succeeded) {
                // A slow compositor must not apply an abandoned change later.
                pending.configs = std::mem::take(&mut pending.previous);
                pending.tested = false;
                pending.canceled = Arc::new(AtomicBool::new(false));
                if let Err(error) = state.configure(pending, qh) {
                    let _ = state.events.send_blocking(Event::Unavailable(error));
                }
            }
            return;
        }
        match event {
            zwlr_output_configuration_v1::Event::Succeeded if !pending.tested => {
                pending.tested = true;
                if let Err(error) = state.configure(pending, qh) {
                    let _ = state.events.send_blocking(Event::Unavailable(error));
                }
            }
            zwlr_output_configuration_v1::Event::Succeeded => {
                let _ = pending.reply.send_blocking(Ok(()));
            }
            zwlr_output_configuration_v1::Event::Failed => {
                let _ = pending.reply.send_blocking(Err(
                    "The display driver rejected this mode, scale or VRR setting".into(),
                ));
            }
            _ => {
                let _ = pending.reply.send_blocking(Err(
                    "Displays changed while applying; refresh and try again".into(),
                ));
            }
        }
    }
}
impl Dispatch<ZwlrOutputConfigurationHeadV1, ()> for State {
    fn event(
        _: &mut Self,
        _: &ZwlrOutputConfigurationHeadV1,
        _: zwlr_output_configuration_head_v1::Event,
        _: &(),
        _: &Connection,
        _: &QueueHandle<Self>,
    ) {
    }
}
