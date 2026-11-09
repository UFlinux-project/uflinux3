use super::*;
use std::cell::Cell;
pub fn page(app: &Rc<App>, parent: &gtk::Box) {
    parent.append(&description("Choose a power mode for supported processor and device settings. The selected mode is kept after restarting and when connecting or removing the charger."));
    let status = description("");
    parent.append(&status);
    let target = status.downgrade();
    app.bind_for(Surface::Full, &status, move |s| {
        if let Some(label) = target.upgrade() {
            label.set_text(&format!(
                "{} · {}",
                if s.power.on_battery {
                    "On battery"
                } else {
                    "External power / no discharging battery"
                },
                s.power
                    .profile
                    .map(PowerProfile::label)
                    .unwrap_or(if s.power.available {
                        "Power management has not reported an active mode"
                    } else {
                        "Power management is unavailable"
                    })
            ));
        }
    });
    profile(app, parent, Surface::Full);
    parent.append(&description("Power saver favors lower energy use. Balanced is the default. Performance favors responsiveness. The effect depends on the drivers and hardware available."));
    section(parent, "Power button");
    parent.append(&description("Choose what a short press of your device’s power button does. Holding the button to force power off is controlled by the hardware."));
    let choices = [
        ("poweroff", "Shut down"),
        ("suspend", "Suspend"),
        ("nothing", "Do nothing"),
    ];
    let key = |action| match action {
        PowerButtonAction::Poweroff => "poweroff",
        PowerButtonAction::Suspend => "suspend",
        PowerButtonAction::Nothing => "nothing",
    };
    let ac = choose(
        parent,
        "When plugged in",
        &choices,
        key(app.state.borrow().power.policy.on_ac),
    );
    let battery = choose(
        parent,
        "When on battery",
        &choices,
        key(app.state.borrow().power.policy.on_battery),
    );
    let target = ac.downgrade();
    let second = battery.downgrade();
    let button = gtk::Button::with_label("Apply power button settings");
    let weak = Rc::downgrade(app);
    button.connect_clicked(move |button| {
        if let (Some(app), Some(ac), Some(battery)) =
            (weak.upgrade(), target.upgrade(), second.upgrade())
        {
            let action = |n| match n {
                1 => PowerButtonAction::Suspend,
                2 => PowerButtonAction::Nothing,
                _ => PowerButtonAction::Poweroff,
            };
            app.execute(
                Action::System {
                    operation: SystemAction::SetPowerButtons {
                        on_ac: action(ac.selected()),
                        on_battery: action(battery.selected()),
                    },
                },
                Some(button.clone().upcast()),
            );
        }
    });
    parent.append(&button);
    let note = description("");
    parent.append(&note);
    let target = note.downgrade();
    app.bind_for(Surface::Full,&note,move|s|if let Some(note)=target.upgrade(){
        note.set_text(if !s.power.button_handler_running{"Power-button handling is not running or this device has no supported input event."}
            else if !s.power.can_suspend{"Suspend is unavailable on this hardware. Shut down and Do nothing remain available."}
            else{"System sleep and shutdown inhibitors are respected. Changes require administrator authorization."});
    });
}

pub fn profile(app: &Rc<App>, parent: &gtk::Box, surface: Surface) {
    section(parent, "Power mode");
    let status = description("");
    parent.append(&status);
    let target = status.downgrade();
    app.bind_for(surface, &status, move |state| {
        if let Some(status) = target.upgrade() {
            status.set_text(if state.power.available {
                ""
            } else {
                "Power profiles are unavailable."
            });
        }
    });
    let profiles = [
        PowerProfile::PowerSaver,
        PowerProfile::Balanced,
        PowerProfile::Performance,
    ];
    let labels = profiles.iter().map(|p| tr(p.label())).collect::<Vec<_>>();
    let drop = gtk::DropDown::from_strings(&labels.iter().map(|s| s.as_str()).collect::<Vec<_>>());
    parent.append(&row("Power mode", &drop));
    let guard = Rc::new(Cell::new(false));
    let flag = guard.clone();
    let weak = Rc::downgrade(app);
    drop.connect_selected_notify(move |drop| {
        if !flag.get() {
            if let (Some(app), Some(profile)) =
                (weak.upgrade(), profiles.get(drop.selected() as usize))
            {
                app.execute(
                    Action::SetPowerProfile { profile: *profile },
                    Some(drop.clone().upcast()),
                );
            }
        }
    });
    let target = drop.downgrade();
    app.bind_for(surface, &drop, move |s| {
        if let Some(drop) = target.upgrade() {
            guard.set(true);
            drop.set_sensitive(s.power.available);
            drop.set_selected(
                s.power
                    .profile
                    .and_then(|p| profiles.iter().position(|v| *v == p))
                    .map(|n| n as u32)
                    .unwrap_or(gtk::INVALID_LIST_POSITION),
            );
            guard.set(false);
        }
    });
}
