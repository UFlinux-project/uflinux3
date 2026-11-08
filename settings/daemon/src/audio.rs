//! Event-driven PulseAudio protocol client, connected to pipewire-pulse.
use libpulse_binding as pulse;
use pulse::{
    callbacks::ListResult,
    channelmap::Map,
    context::{Context, FlagSet, State, subscribe::InterestMaskSet},
    volume::{ChannelVolumes, Volume},
};
use settings_core::*;
use std::{
    cell::{Cell, RefCell},
    collections::HashMap,
    rc::Rc,
};

pub struct Client {
    context: RefCell<Context>,
    _mainloop: libpulse_glib_binding::Mainloop,
    pub view: RefCell<Audio>,
    volumes: RefCell<HashMap<(bool, String), (ChannelVolumes, Map)>>,
    stream_volumes: RefCell<HashMap<(bool, u32), ChannelVolumes>>,
    scheduled: Cell<bool>,
    emit: Rc<dyn Fn(Audio)>,
}
fn percent(v: &ChannelVolumes) -> f64 {
    v.max().0 as f64 / Volume::NORMAL.0 as f64 * 100.0
}
fn value(percent: f64) -> Volume {
    Volume((percent.clamp(0.0, 100.0) / 100.0 * Volume::NORMAL.0 as f64).round() as u32)
}
fn label(s: &Option<std::borrow::Cow<'_, str>>) -> String {
    s.as_deref().unwrap_or_default().to_string()
}
impl Client {
    pub fn new(emit: Rc<dyn Fn(Audio)>) -> Result<Rc<Self>> {
        let mainloop = libpulse_glib_binding::Mainloop::new(None)
            .ok_or("Cannot initialize audio event loop")?;
        let context =
            Context::new(&mainloop, "UFLinux Settings").ok_or("Cannot initialize audio client")?;
        let this = Rc::new(Self {
            _mainloop: mainloop,
            context: RefCell::new(context),
            view: RefCell::new(Audio::default()),
            volumes: RefCell::new(HashMap::new()),
            stream_volumes: RefCell::new(HashMap::new()),
            scheduled: Cell::new(false),
            emit,
        });
        let weak = Rc::downgrade(&this);
        this.context
            .borrow_mut()
            .set_state_callback(Some(Box::new(move || {
                if let Some(this) = weak.upgrade() {
                    let task = this.clone();
                    glib::idle_add_local_once(move || {
                        let state = task.context.borrow().get_state();
                        if state == State::Ready {
                            let weak = Rc::downgrade(&task);
                            task.context
                                .borrow_mut()
                                .set_subscribe_callback(Some(Box::new(move |_, _, _| {
                                    if let Some(t) = weak.upgrade() {
                                        t.schedule();
                                    }
                                })));
                            task.context.borrow_mut().subscribe(
                                InterestMaskSet::SINK
                                    | InterestMaskSet::SOURCE
                                    | InterestMaskSet::SINK_INPUT
                                    | InterestMaskSet::SOURCE_OUTPUT
                                    | InterestMaskSet::SERVER
                                    | InterestMaskSet::CARD,
                                |_| {},
                            );
                            task.view.borrow_mut().available = true;
                            task.refresh();
                        } else if matches!(state, State::Failed | State::Terminated) {
                            *task.view.borrow_mut() = Audio::default();
                            task.publish();
                        }
                    });
                }
            })));
        this.context
            .borrow_mut()
            .connect(None, FlagSet::NOAUTOSPAWN, None)
            .map_err(|e| {
                e.to_string()
                    .unwrap_or_else(|| "Audio connection failed".into())
            })?;
        Ok(this)
    }
    pub fn ready(&self) -> bool {
        self.context.borrow().get_state() == State::Ready
    }
    fn publish(&self) {
        (self.emit)(self.view.borrow().clone());
    }
    fn schedule(self: &Rc<Self>) {
        if self.scheduled.replace(true) {
            return;
        }
        let this = self.clone();
        glib::timeout_add_local_once(std::time::Duration::from_millis(70), move || {
            this.scheduled.set(false);
            this.refresh();
        });
    }
    pub fn refresh(self: &Rc<Self>) {
        if !self.ready() {
            return;
        }
        let introspect = self.context.borrow().introspect();
        let weak = Rc::downgrade(self);
        introspect.get_server_info(move |info| {
            if let Some(t) = weak.upgrade() {
                let mut v = t.view.borrow_mut();
                v.default_output = label(&info.default_sink_name);
                v.default_input = label(&info.default_source_name);
                drop(v);
                t.publish();
            }
        });
        let weak = Rc::downgrade(self);
        let items = Rc::new(RefCell::new(Vec::new()));
        introspect.get_sink_info_list(move |result| {
            if let Some(t) = weak.upgrade() {
                match result {
                    ListResult::Item(info) => {
                        let name = label(&info.name);
                        t.volumes
                            .borrow_mut()
                            .insert((false, name.clone()), (info.volume, info.channel_map));
                        items.borrow_mut().push(AudioDevice {
                            index: info.index,
                            name,
                            description: label(&info.description),
                            volume: percent(&info.volume),
                            muted: info.mute,
                            balance: info.volume.get_balance(&info.channel_map) as f64,
                            port: info
                                .active_port
                                .as_ref()
                                .map(|p| label(&p.name))
                                .unwrap_or_default(),
                            ports: info
                                .ports
                                .iter()
                                .map(|p| (label(&p.name), label(&p.description)))
                                .collect(),
                        });
                    }
                    ListResult::End => {
                        t.view.borrow_mut().outputs = std::mem::take(&mut *items.borrow_mut());
                        t.volumes.borrow_mut().retain(|(input, name), _| {
                            *input || t.view.borrow().outputs.iter().any(|d| &d.name == name)
                        });
                        t.publish();
                    }
                    _ => {}
                }
            }
        });
        let weak = Rc::downgrade(self);
        let items = Rc::new(RefCell::new(Vec::new()));
        introspect.get_source_info_list(move |result| {
            if let Some(t) = weak.upgrade() {
                match result {
                    ListResult::Item(info) => {
                        let name = label(&info.name);
                        t.volumes
                            .borrow_mut()
                            .insert((true, name.clone()), (info.volume, info.channel_map));
                        items.borrow_mut().push(AudioDevice {
                            index: info.index,
                            name,
                            description: label(&info.description),
                            volume: percent(&info.volume),
                            muted: info.mute,
                            balance: info.volume.get_balance(&info.channel_map) as f64,
                            port: info
                                .active_port
                                .as_ref()
                                .map(|p| label(&p.name))
                                .unwrap_or_default(),
                            ports: info
                                .ports
                                .iter()
                                .map(|p| (label(&p.name), label(&p.description)))
                                .collect(),
                        });
                    }
                    ListResult::End => {
                        t.view.borrow_mut().inputs = std::mem::take(&mut *items.borrow_mut());
                        t.volumes.borrow_mut().retain(|(input, name), _| {
                            !*input || t.view.borrow().inputs.iter().any(|d| &d.name == name)
                        });
                        t.publish();
                    }
                    _ => {}
                }
            }
        });
        let weak = Rc::downgrade(self);
        let items = Rc::new(RefCell::new(Vec::new()));
        introspect.get_sink_input_info_list(move |result| {
            if let Some(t) = weak.upgrade() {
                match result {
                    ListResult::Item(info) => {
                        t.stream_volumes
                            .borrow_mut()
                            .insert((false, info.index), info.volume);
                        items.borrow_mut().push(AudioStream {
                            index: info.index,
                            name: info
                                .proplist
                                .get_str("application.name")
                                .unwrap_or_else(|| label(&info.name)),
                            input: false,
                            volume: percent(&info.volume),
                            muted: info.mute,
                        });
                    }
                    ListResult::End => {
                        let mut v = t.view.borrow_mut();
                        v.streams.retain(|s| s.input);
                        v.streams.extend(std::mem::take(&mut *items.borrow_mut()));
                        drop(v);
                        t.publish();
                    }
                    _ => {}
                }
            }
        });
        let weak = Rc::downgrade(self);
        let items = Rc::new(RefCell::new(Vec::new()));
        introspect.get_source_output_info_list(move |result| {
            if let Some(t) = weak.upgrade() {
                match result {
                    ListResult::Item(info) => {
                        t.stream_volumes
                            .borrow_mut()
                            .insert((true, info.index), info.volume);
                        items.borrow_mut().push(AudioStream {
                            index: info.index,
                            name: info
                                .proplist
                                .get_str("application.name")
                                .unwrap_or_else(|| label(&info.name)),
                            input: true,
                            volume: percent(&info.volume),
                            muted: info.mute,
                        });
                    }
                    ListResult::End => {
                        let mut v = t.view.borrow_mut();
                        v.streams.retain(|s| !s.input);
                        v.streams.extend(std::mem::take(&mut *items.borrow_mut()));
                        drop(v);
                        t.publish();
                    }
                    _ => {}
                }
            }
        });
        let weak = Rc::downgrade(self);
        let items = Rc::new(RefCell::new(Vec::new()));
        introspect.get_card_info_list(move |result| {
            if let Some(t) = weak.upgrade() {
                match result {
                    ListResult::Item(info) => items.borrow_mut().push(AudioCard {
                        index: info.index,
                        description: info
                            .proplist
                            .get_str("device.description")
                            .unwrap_or_else(|| label(&info.name)),
                        profile: info
                            .active_profile
                            .as_ref()
                            .map(|p| label(&p.name))
                            .unwrap_or_default(),
                        profiles: info
                            .profiles
                            .iter()
                            .filter(|p| p.available)
                            .map(|p| (label(&p.name), label(&p.description)))
                            .collect(),
                    }),
                    ListResult::End => {
                        t.view.borrow_mut().cards = std::mem::take(&mut *items.borrow_mut());
                        t.publish();
                    }
                    _ => {}
                }
            }
        });
    }
    pub async fn execute(self: &Rc<Self>, action: &Action) -> Result<()> {
        if !self.ready() {
            return Err("PipeWire audio is unavailable".into());
        }
        let (send, receive) = async_channel::bounded(1);
        let callback = move |success| {
            let _ = send.try_send(success);
        };
        let mut introspect = self.context.borrow().introspect();
        match action {
            Action::SetVolume {
                input,
                device,
                percent: p,
            }
            | Action::SetBalance {
                input,
                device,
                balance: p,
            } => {
                let name = self.device(*input, device)?;
                let (mut volume, map) = self
                    .volumes
                    .borrow()
                    .get(&(*input, name.clone()))
                    .copied()
                    .ok_or("Audio device disappeared")?;
                if matches!(action, Action::SetBalance { .. }) {
                    if !p.is_finite() || !(-1.0..=1.0).contains(p) {
                        return Err("Invalid channel balance".into());
                    }
                    if !map.can_balance() {
                        return Err("This device does not support balance".into());
                    }
                    volume.set_balance(&map, *p as f32);
                } else {
                    validate::percent(*p)?;
                    volume.scale(value(*p));
                }
                if *input {
                    introspect.set_source_volume_by_name(&name, &volume, Some(Box::new(callback)));
                } else {
                    introspect.set_sink_volume_by_name(&name, &volume, Some(Box::new(callback)));
                }
            }
            Action::SetMute {
                input,
                device,
                muted,
            } => {
                let name = self.device(*input, device)?;
                if *input {
                    introspect.set_source_mute_by_name(&name, *muted, Some(Box::new(callback)));
                } else {
                    introspect.set_sink_mute_by_name(&name, *muted, Some(Box::new(callback)));
                }
            }
            Action::SetDefault { input, device } => {
                let name = self.device(*input, device)?;
                if *input {
                    self.context
                        .borrow_mut()
                        .set_default_source(&name, callback);
                } else {
                    self.context.borrow_mut().set_default_sink(&name, callback);
                }
            }
            Action::SetPort {
                input,
                device,
                port,
            } => {
                let name = self.device(*input, device)?;
                let v = self.view.borrow();
                let list = if *input { &v.inputs } else { &v.outputs };
                if !list
                    .iter()
                    .find(|d| d.name == name)
                    .is_some_and(|d| d.ports.iter().any(|p| &p.0 == port))
                {
                    return Err("Unknown audio port".into());
                }
                if *input {
                    introspect.set_source_port_by_name(&name, port, Some(Box::new(callback)));
                } else {
                    introspect.set_sink_port_by_name(&name, port, Some(Box::new(callback)));
                }
            }
            Action::SetProfile { card, profile } => {
                if !self
                    .view
                    .borrow()
                    .cards
                    .iter()
                    .find(|c| c.index == *card)
                    .is_some_and(|c| c.profiles.iter().any(|p| &p.0 == profile))
                {
                    return Err("Unknown audio profile".into());
                }
                introspect.set_card_profile_by_index(*card, profile, Some(Box::new(callback)));
            }
            Action::SetStream {
                input,
                index,
                percent: p,
                muted,
            } => {
                validate::percent(*p)?;
                let mut volume = self
                    .stream_volumes
                    .borrow()
                    .get(&(*input, *index))
                    .copied()
                    .ok_or("Audio stream ended")?;
                volume.scale(value(*p));
                if *input {
                    introspect.set_source_output_volume(*index, &volume, None);
                    introspect.set_source_output_mute(*index, *muted, Some(Box::new(callback)));
                } else {
                    introspect.set_sink_input_volume(*index, &volume, None);
                    introspect.set_sink_input_mute(*index, *muted, Some(Box::new(callback)));
                }
            }
            _ => return Err("Not an audio operation".into()),
        }
        if !receive
            .recv()
            .await
            .map_err(|_| "Audio operation canceled")?
        {
            return Err("The audio server rejected this change".into());
        }
        self.schedule();
        Ok(())
    }
    pub fn device(&self, input: bool, requested: &str) -> Result<String> {
        let view = self.view.borrow();
        let name = if requested.is_empty() {
            if input {
                &view.default_input
            } else {
                &view.default_output
            }
        } else {
            requested
        };
        let devices = if input { &view.inputs } else { &view.outputs };
        if devices.iter().any(|d| d.name == name) {
            Ok(name.to_string())
        } else {
            Err("No matching audio device".into())
        }
    }
}
