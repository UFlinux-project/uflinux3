use gtk::glib;
use gtk::prelude::*;
use serde_json::{Value, json};
use std::{
    cell::{Cell, RefCell},
    io::{BufRead, BufReader, Write},
    process::{Command, Stdio},
    rc::Rc,
    sync::mpsc,
    time::Duration,
};

const BACKEND: &str = "/usr/libexec/uflinux-install-gui-backend";

fn label(text: &str) -> gtk::Label {
    gtk::Label::builder()
        .label(text)
        .xalign(0.0)
        .wrap(true)
        .build()
}
fn field(page: &gtk::Box, text: &str, secret: bool) -> gtk::Entry {
    let l = label(text);
    let e = gtk::Entry::builder()
        .visibility(!secret)
        .hexpand(true)
        .build();
    l.set_mnemonic_widget(Some(&e));
    page.append(&l);
    page.append(&e);
    e
}
fn page(title: &str, body: &str) -> gtk::Box {
    let page = gtk::Box::new(gtk::Orientation::Vertical, 14);
    page.set_margin_top(24);
    page.set_margin_bottom(24);
    page.set_margin_start(32);
    page.set_margin_end(32);
    let heading = label(title);
    heading.add_css_class("title-1");
    page.append(&heading);
    let description = label(body);
    description.add_css_class("dim-label");
    page.append(&description);
    page
}
fn dropdown(page: &gtk::Box, title: &str, values: &[&str]) -> gtk::DropDown {
    page.append(&label(title));
    let d = gtk::DropDown::from_strings(values);
    d.set_enable_search(true);
    page.append(&d);
    d
}
fn set_items(drop: &gtk::DropDown, items: &[String]) {
    let refs: Vec<_> = items.iter().map(String::as_str).collect();
    drop.set_model(Some(&gtk::StringList::new(&refs)));
    if !items.is_empty() {
        drop.set_selected(0);
    }
}
fn text(entry: &gtk::Entry) -> String {
    entry.text().to_string()
}
fn selected(drop: &gtk::DropDown, values: &[String]) -> String {
    values
        .get(drop.selected() as usize)
        .cloned()
        .unwrap_or_default()
}

const TRANSLATIONS: &[(&str, &str)] = &[
    ("Welcome to UFLinux", "Vítejte v UFLinuxu"),
    (
        "Install UFLinux on this computer. Your choices are reviewed before anything is changed.",
        "Nainstalujte UFLinux do tohoto počítače. Před provedením změn své volby zkontrolujete.",
    ),
    ("Keyboard", "Klávesnice"),
    (
        "Choose the keyboard layout for the installed system. The live keyboard will not change.",
        "Vyberte rozložení klávesnice pro nainstalovaný systém. Klávesnice živého prostředí se nezmění.",
    ),
    ("Keyboard layout", "Rozložení klávesnice"),
    ("Installation source", "Zdroj instalace"),
    (
        "The signed installation media is used by default. Online packages require an existing connection.",
        "Ve výchozím nastavení se používá podepsané instalační médium. Online balíčky vyžadují existující připojení.",
    ),
    (
        "Allow official Alpine mirrors for packages missing from the media",
        "Povolit oficiální zrcadla Alpine pro balíčky chybějící na médiu",
    ),
    ("Storage", "Úložiště"),
    (
        "Choose a disk. Erasing removes all its data. Prepared partitions must be unmounted and on the selected disk.",
        "Vyberte disk. Smazání odstraní všechna jeho data. Připravené oddíly musí být odpojené a na vybraném disku.",
    ),
    ("Installation method", "Způsob instalace"),
    ("Disk", "Disk"),
    ("Prepared boot partition", "Připravený zaváděcí oddíl"),
    (
        "Prepared XFS root partition",
        "Připravený kořenový oddíl XFS",
    ),
    ("Looking for disks…", "Vyhledávají se disky…"),
    (
        "No available disks found.",
        "Nebyly nalezeny žádné dostupné disky.",
    ),
    ("Create your account", "Vytvořte svůj účet"),
    (
        "Choose the account you will use to sign in. The password must contain at least eight characters.",
        "Zvolte účet pro přihlášení. Heslo musí obsahovat alespoň osm znaků.",
    ),
    ("Your name", "Vaše jméno"),
    ("Username", "Uživatelské jméno"),
    ("Computer name", "Název počítače"),
    ("Password", "Heslo"),
    ("Repeat password", "Zopakujte heslo"),
    ("Time zone", "Časové pásmo"),
    (
        "Choose the time zone for the installed system.",
        "Zvolte časové pásmo nainstalovaného systému.",
    ),
    ("Review your choices", "Zkontrolujte své volby"),
    (
        "No changes have been made. Installation begins when you select Install.",
        "Zatím nebyly provedeny žádné změny. Instalace začne po stisknutí tlačítka Instalovat.",
    ),
    (
        "I understand the listed disk changes",
        "Rozumím uvedeným změnám na disku",
    ),
    ("Installing UFLinux", "Instaluje se UFLinux"),
    (
        "Keep your computer powered on until installation finishes.",
        "Do dokončení instalace ponechte počítač zapnutý.",
    ),
    ("Preparing…", "Příprava…"),
    ("Installation details", "Podrobnosti instalace"),
    ("Installation complete", "Instalace dokončena"),
    (
        "Restart to use your new UFLinux installation, or continue using the live session.",
        "Restartujte počítač pro použití nového systému UFLinux nebo pokračujte v živém prostředí.",
    ),
    ("Restart now", "Restartovat"),
    ("Continue live session", "Pokračovat v živém prostředí"),
    ("Back", "Zpět"),
    ("Next", "Další"),
    ("Install", "Instalovat"),
    ("Select an available disk.", "Vyberte dostupný disk."),
    (
        "Select compatible boot and XFS root partitions.",
        "Vyberte kompatibilní zaváděcí a kořenový oddíl XFS.",
    ),
    (
        "Complete all fields and enter matching passwords of at least eight characters.",
        "Vyplňte všechna pole a zadejte shodná hesla o alespoň osmi znacích.",
    ),
    ("Select a time zone.", "Vyberte časové pásmo."),
    (
        "Check the name, username, computer name, and matching passwords (at least eight characters; no colon or line breaks).",
        "Zkontrolujte jméno, uživatelské jméno, název počítače a shodná hesla (alespoň osm znaků; bez dvojtečky a zalomení řádku).",
    ),
    (
        "Acknowledge the disk changes before installing.",
        "Před instalací potvrďte změny na disku.",
    ),
    (
        "Disk changed since review; review storage again",
        "Disk se od kontroly změnil; zkontrolujte úložiště znovu",
    ),
    (
        "Disk is mounted, in use, read-only, or is live media",
        "Disk je připojený, používá se, je pouze pro čtení nebo obsahuje živé médium",
    ),
    (
        "Signed installation media repository is unavailable",
        "Repozitář podepsaného instalačního média není dostupný",
    ),
    (
        "UEFI variables are unavailable",
        "Proměnné UEFI nejsou dostupné",
    ),
    (
        "Unsupported architecture or firmware mode",
        "Nepodporovaná architektura nebo režim firmwaru",
    ),
    (
        "Another UFLinux installation is already running",
        "Jiná instalace UFLinuxu již probíhá",
    ),
    (
        "Disk is too small for an 8 GiB XFS root",
        "Disk je příliš malý pro kořenový oddíl XFS o velikosti 8 GiB",
    ),
    (
        "Root filesystem must be at least 8 GiB",
        "Kořenový souborový systém musí mít alespoň 8 GiB",
    ),
    (
        "The EFI system partition must be FAT32",
        "Systémový oddíl EFI musí používat FAT32",
    ),
    (
        "PowerPC requires a PReP boot partition",
        "PowerPC vyžaduje zaváděcí oddíl PReP",
    ),
    (
        "Target root must be empty apart from boot and lost+found",
        "Kořenový oddíl musí být prázdný s výjimkou boot a lost+found",
    ),
    (
        "Target boot must be a normal directory",
        "Cílový adresář boot musí být běžný adresář",
    ),
    (
        "Installation failed. The target may be incomplete.",
        "Instalace selhala. Cílový systém může být neúplný.",
    ),
    ("Could not inspect disks.", "Disky nelze prozkoumat."),
    ("Preparing storage", "Připravuje se úložiště"),
    (
        "Installing system packages and bootloader",
        "Instalují se systémové balíčky a zavaděč",
    ),
    (
        "Applying personal settings",
        "Používají se osobní nastavení",
    ),
];
fn translate(text: &str, czech: bool) -> String {
    for (en, cs) in TRANSLATIONS {
        if text == *en || text == *cs {
            return if czech { *cs } else { *en }.to_string();
        }
    }
    text.to_string()
}
fn translate_widgets(widget: &gtk::Widget, czech: bool) {
    if let Some(label) = widget.downcast_ref::<gtk::Label>() {
        label.set_label(&translate(label.label().as_str(), czech));
    }
    if let Some(button) = widget.downcast_ref::<gtk::Button>() {
        if let Some(s) = button.label() {
            button.set_label(&translate(&s, czech));
        }
    }
    if let Some(button) = widget.downcast_ref::<gtk::CheckButton>() {
        if let Some(s) = button.label() {
            button.set_label(Some(&translate(&s, czech)));
        }
    }
    if let Some(expander) = widget.downcast_ref::<gtk::Expander>() {
        if let Some(s) = expander.label() {
            expander.set_label(Some(&translate(&s, czech)));
        }
    }
    let mut child = widget.first_child();
    while let Some(w) = child {
        child = w.next_sibling();
        translate_widgets(&w, czech);
    }
}

fn main() {
    let app = gtk::Application::builder()
        .application_id("org.uflinux.Installer")
        .build();
    app.connect_activate(|app| {
        if let Some(window) = app.windows().first() {
            window.present();
            return;
        }
        build(app);
    });
    app.run();
}
fn build(app: &gtk::Application) {
    let win = gtk::ApplicationWindow::builder()
        .application(app)
        .title("Install UFLinux")
        .default_width(860)
        .default_height(650)
        .build();
    let outer = gtk::Box::new(gtk::Orientation::Vertical, 0);
    let bar = gtk::HeaderBar::new();
    bar.set_title_widget(Some(&gtk::Label::new(Some("UFLinux"))));
    win.set_titlebar(Some(&bar));
    let stack = gtk::Stack::new();
    stack.set_hexpand(true);
    stack.set_vexpand(true);
    stack.set_transition_type(gtk::StackTransitionType::None);
    outer.append(&stack);
    let welcome = page(
        "Welcome to UFLinux",
        "Install UFLinux on this computer. Your choices are reviewed before anything is changed.",
    );
    let language = dropdown(&welcome, "Language / Jazyk", &["English", "Čeština"]);
    stack.add_named(&welcome, Some("welcome"));
    let keyboard_page = page(
        "Keyboard",
        "Choose the keyboard layout for the installed system. The live keyboard will not change.",
    );
    let rules = std::fs::read_to_string("/usr/share/X11/xkb/rules/base.lst").unwrap_or_default();
    let mut layout_section = false;
    let mut keyboard_choices: Vec<(String, String)> = Vec::new();
    for line in rules.lines() {
        if line.starts_with('!') {
            layout_section = line.trim() == "! layout";
            continue;
        }
        if layout_section {
            let mut words = line.split_whitespace();
            if let Some(code) = words.next() {
                let description = words.collect::<Vec<_>>().join(" ");
                if !description.is_empty() {
                    keyboard_choices
                        .push((code.to_string(), format!("{} ({})", description, code)));
                }
            }
        }
    }
    if keyboard_choices.is_empty() {
        keyboard_choices = vec![
            ("us".into(), "English (us)".into()),
            ("cz".into(), "Čeština (cz)".into()),
        ];
    }
    let keyboards: Vec<String> = keyboard_choices.iter().map(|x| x.0.clone()).collect();
    let keyboard = dropdown(
        &keyboard_page,
        "Keyboard layout",
        &keyboard_choices
            .iter()
            .map(|x| x.1.as_str())
            .collect::<Vec<_>>(),
    );
    if let Some(i) = keyboards.iter().position(|x| x == "us") {
        keyboard.set_selected(i as u32);
    }
    stack.add_named(&keyboard_page, Some("keyboard"));
    let source_page = page(
        "Installation source",
        "The signed installation media is used by default. Online packages require an existing connection.",
    );
    let online = gtk::CheckButton::with_label(
        "Allow official Alpine mirrors for packages missing from the media",
    );
    source_page.append(&online);
    stack.add_named(&source_page, Some("source"));
    let storage_page = page(
        "Storage",
        "Choose a disk. Erasing removes all its data. Prepared partitions must be unmounted and on the selected disk.",
    );
    let storage_mode = dropdown(
        &storage_page,
        "Installation method",
        &["Erase disk and install UFLinux", "Use prepared partitions"],
    );
    let disks = dropdown(&storage_page, "Disk", &[]);
    let boot = dropdown(&storage_page, "Prepared boot partition", &[]);
    let root = dropdown(&storage_page, "Prepared XFS root partition", &[]);
    let storage_note = label("Looking for disks…");
    storage_page.append(&storage_note);
    stack.add_named(&storage_page, Some("storage"));
    let account_page = page(
        "Create your account",
        "Choose the account you will use to sign in. The password must contain at least eight characters.",
    );
    let name = field(&account_page, "Your name", false);
    let user = field(&account_page, "Username", false);
    let hostname = field(&account_page, "Computer name", false);
    hostname.set_text("uflinux");
    let password = field(&account_page, "Password", true);
    let repeat = field(&account_page, "Repeat password", true);
    stack.add_named(&account_page, Some("account"));
    let timezone_page = page(
        "Time zone",
        "Choose the time zone for the installed system.",
    );
    let mut zones: Vec<String> = std::fs::read_to_string("/usr/share/zoneinfo/zone.tab")
        .unwrap_or_default()
        .lines()
        .filter(|l| !l.starts_with('#'))
        .filter_map(|l| l.split('\t').nth(2))
        .map(str::to_string)
        .collect();
    zones.insert(0, "Etc/UTC".into());
    let timezone = dropdown(
        &timezone_page,
        "Time zone",
        &zones.iter().map(String::as_str).collect::<Vec<_>>(),
    );
    let local_zone = std::fs::read_to_string("/etc/timezone").unwrap_or_default();
    if let Some(i) = zones.iter().position(|x| x == local_zone.trim()) {
        timezone.set_selected(i as u32);
    }
    stack.add_named(&timezone_page, Some("timezone"));
    let review_page = page(
        "Review your choices",
        "No changes have been made. Installation begins when you select Install.",
    );
    let review = label("");
    review.set_selectable(true);
    let review_scroll = gtk::ScrolledWindow::builder()
        .min_content_height(220)
        .vexpand(true)
        .child(&review)
        .build();
    review_page.append(&review_scroll);
    let acknowledgement = gtk::CheckButton::with_label("I understand the listed disk changes");
    review_page.append(&acknowledgement);
    stack.add_named(&review_page, Some("review"));
    let progress_page = page(
        "Installing UFLinux",
        "Keep your computer powered on until installation finishes.",
    );
    let phase = label("Preparing…");
    progress_page.append(&phase);
    let spinner = gtk::Spinner::new();
    spinner.start();
    progress_page.append(&spinner);
    let logs = gtk::TextView::new();
    logs.set_editable(false);
    logs.set_cursor_visible(false);
    logs.set_monospace(true);
    let scroll = gtk::ScrolledWindow::builder()
        .min_content_height(150)
        .child(&logs)
        .build();
    let expander = gtk::Expander::new(Some("Installation details"));
    expander.set_child(Some(&scroll));
    progress_page.append(&expander);
    stack.add_named(&progress_page, Some("progress"));
    let done_page = page(
        "Installation complete",
        "Restart to use your new UFLinux installation, or continue using the live session.",
    );
    let restart = gtk::Button::with_label("Restart now");
    restart.add_css_class("suggested-action");
    done_page.append(&restart);
    let continue_live = gtk::Button::with_label("Continue live session");
    done_page.append(&continue_live);
    stack.add_named(&done_page, Some("done"));
    let bottom = gtk::Box::new(gtk::Orientation::Horizontal, 12);
    bottom.set_margin_top(16);
    bottom.set_margin_bottom(16);
    bottom.set_margin_start(32);
    bottom.set_margin_end(32);
    let back = gtk::Button::with_label("Back");
    let next = gtk::Button::with_label("Next");
    next.add_css_class("suggested-action");
    let error = label("");
    error.add_css_class("error");
    error.set_hexpand(true);
    bottom.append(&back);
    bottom.append(&error);
    bottom.append(&next);
    outer.append(&bottom);
    win.set_child(Some(&outer));
    let step = Rc::new(Cell::new(0usize));
    let running = Rc::new(Cell::new(false));
    let discovery: Rc<RefCell<Value>> = Rc::new(RefCell::new(Value::Null));
    let boot_values: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
    let root_values: Rc<RefCell<Vec<String>>> = Rc::new(RefCell::new(vec![]));
    let refresh = {
        let (discovery, disks, boot, root, boot_values, root_values, storage_note, language) = (
            discovery.clone(),
            disks.clone(),
            boot.clone(),
            root.clone(),
            boot_values.clone(),
            root_values.clone(),
            storage_note.clone(),
            language.clone(),
        );
        Rc::new(move || {
            let data = discovery.borrow();
            let disk = &data["disks"][disks.selected() as usize]["device"];
            let parts = disk["children"].as_array().cloned().unwrap_or_default();
            let mut bv = vec![];
            let mut rv = vec![];
            for p in parts {
                if let Some(path) = p["path"].as_str() {
                    if p["fstype"] == "xfs" {
                        rv.push(path.to_string());
                    }
                    if p["fstype"]
                        == if data["mode"] == "uefi" {
                            "vfat"
                        } else {
                            "ext4"
                        }
                    {
                        bv.push(path.to_string());
                    }
                }
            }
            set_items(&boot, &bv);
            set_items(&root, &rv);
            *boot_values.borrow_mut() = bv;
            *root_values.borrow_mut() = rv;
            storage_note.set_label(&translate(
                if data["disks"].as_array().is_some_and(|a| !a.is_empty()) {
                    ""
                } else {
                    "No available disks found."
                },
                language.selected() == 1,
            ));
        })
    };
    {
        let refresh = refresh.clone();
        disks.connect_selected_notify(move |_| refresh());
    }
    let (tx, rx) = mpsc::channel::<Result<Value, String>>();
    std::thread::spawn(move || {
        let result = Command::new(BACKEND)
            .arg("--discover")
            .output()
            .map_err(|e| e.to_string())
            .and_then(|o| {
                if o.status.success() {
                    serde_json::from_slice(&o.stdout).map_err(|e| e.to_string())
                } else {
                    Err(serde_json::from_slice::<Value>(&o.stdout)
                        .ok()
                        .and_then(|v| v["message"].as_str().map(str::to_string))
                        .filter(|s| !s.is_empty())
                        .unwrap_or_else(|| "Could not inspect disks.".into()))
                }
            });
        let _ = tx.send(result);
    });
    {
        let discovery = discovery.clone();
        let disks = disks.clone();
        let refresh = refresh.clone();
        let storage_note = storage_note.clone();
        let language = language.clone();
        glib::timeout_add_local(Duration::from_millis(100), move || match rx.try_recv() {
            Ok(Ok(data)) => {
                let names: Vec<String> = data["disks"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|x| {
                        format!(
                            "{}  {}  {:.1} GiB",
                            x["device"]["path"].as_str().unwrap_or("?"),
                            x["device"]["model"].as_str().unwrap_or(""),
                            x["device"]["size"].as_u64().unwrap_or(0) as f64 / 1073741824.0
                        )
                    })
                    .collect();
                *discovery.borrow_mut() = data;
                set_items(&disks, &names);
                refresh();
                glib::ControlFlow::Break
            }
            Ok(Err(e)) => {
                storage_note.set_label(&translate(&e, language.selected() == 1));
                glib::ControlFlow::Break
            }
            Err(mpsc::TryRecvError::Disconnected) => glib::ControlFlow::Break,
            _ => glib::ControlFlow::Continue,
        });
    }
    {
        let (outer, storage_mode) = (outer.clone(), storage_mode.clone());
        language.connect_selected_notify(move |language| {
            let cs = language.selected() == 1;
            translate_widgets(outer.upcast_ref(), cs);
            let current = storage_mode.selected();
            let items: Vec<String> = if cs {
                vec![
                    "Smazat disk a nainstalovat UFLinux".into(),
                    "Použít připravené oddíly".into(),
                ]
            } else {
                vec![
                    "Erase disk and install UFLinux".into(),
                    "Use prepared partitions".into(),
                ]
            };
            set_items(&storage_mode, &items);
            storage_mode.set_selected(current);
        });
    }
    let pages = [
        "welcome", "keyboard", "source", "storage", "account", "timezone", "review", "progress",
        "done",
    ];
    {
        let (step, stack, back, next, error, language) = (
            step.clone(),
            stack.clone(),
            back.clone(),
            next.clone(),
            error.clone(),
            language.clone(),
        );
        back.clone().connect_clicked(move |_| {
            let i = step.get().saturating_sub(1);
            step.set(i);
            stack.set_visible_child_name(pages[i]);
            back.set_sensitive(i > 0);
            next.set_label(&translate("Next", language.selected() == 1));
            error.set_label("");
        });
    }
    {
        let (win, running) = (win.clone(), running.clone());
        win.clone().connect_close_request(move |_| {
            if running.get() {
                win.set_visible(false);
                glib::Propagation::Stop
            } else {
                glib::Propagation::Proceed
            }
        });
    }
    {
        let win = win.clone();
        continue_live.connect_clicked(move |_| win.close());
    }
    restart.connect_clicked(move |_| {
        let _ = Command::new("loginctl").arg("reboot").spawn();
    });
    {
        let (step, stack, back, next, error, discovery, boot_values, root_values, running, win) = (
            step.clone(),
            stack.clone(),
            back.clone(),
            next.clone(),
            error.clone(),
            discovery.clone(),
            boot_values.clone(),
            root_values.clone(),
            running.clone(),
            win.clone(),
        );
        next.clone().connect_clicked(move |_| {
        let i=step.get(); error.set_label("");
        let fail = |s: &str| { error.set_label(&translate(s, language.selected()==1)); };
        if i==3 && discovery.borrow()["disks"].get(disks.selected() as usize).is_none() { fail("Select an available disk."); return; }
        if i==3 && storage_mode.selected()==1 && (selected(&boot,&boot_values.borrow()).is_empty() || selected(&root,&root_values.borrow()).is_empty()) { fail("Select compatible boot and XFS root partitions."); return; }
        if i==4 {
            let u=text(&user); let h=text(&hostname); let n=text(&name); let p=text(&password);
            let valid_user = u.len()<=31 && !matches!(u.as_str(), "root"|"live"|"greetd") && u.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c=='_') && u.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c=='_' || c=='-');
            let valid_host = h.len()<=63 && h.chars().next().is_some_and(|c| c.is_ascii_lowercase() || c.is_ascii_digit()) && h.chars().all(|c| c.is_ascii_lowercase() || c.is_ascii_digit() || c=='-');
            if !valid_user || !valid_host || n.is_empty() || n.len()>128 || n.contains([':','\n','\r']) || p.chars().count()<8 || p.chars().count()>1024 || p.contains([':','\n','\r']) || p!=text(&repeat) { fail("Check the name, username, computer name, and matching passwords (at least eight characters; no colon or line breaks)."); return; }
        }
        if i==5 && selected(&timezone,&zones).is_empty() { fail("Select a time zone."); return; }
        if i==5 {
            let d=discovery.borrow(); let disk=&d["disks"][disks.selected() as usize]["device"];
            let cs=language.selected()==1;
            let parts: Vec<String> = disk["children"].as_array().into_iter().flatten().map(|p| format!("{} ({}, {:.1} GiB)", p["path"].as_str().unwrap_or("?"), p["fstype"].as_str().unwrap_or("unknown"), p["size"].as_u64().unwrap_or(0) as f64/1073741824.0)).collect();
            let action=if storage_mode.selected()==0 {
                if cs {"SMAZAT celý disk; vytvořit 1 GiB zaváděcí oddíl a kořen XFS".to_string()} else {"ERASE the entire disk; create a 1 GiB boot partition and XFS root".to_string()}
            } else {
                format!("{} {} / {} {}. {}", if cs {"Použít zaváděcí oddíl"} else {"Use existing boot"}, selected(&boot,&boot_values.borrow()), if cs {"kořen XFS"} else {"XFS root"}, selected(&root,&root_values.borrow()), if cs {"Zaváděcí soubory a zavaděč mohou být přepsány."} else {"Boot files and bootloader may be replaced."})
            };
            let disk_label=if cs {"Disk"}else{"Disk"}; let serial_label=if cs {"Sériové číslo"}else{"Serial"};
            let current_label=if cs {"Současné oddíly"}else{"Current partitions"}; let firmware_label=if cs {"Firmware a cíl zavaděče"}else{"Firmware and bootloader destination"};
            let account_label=if cs {"Účet"}else{"Account"}; let host_label=if cs {"Počítač"}else{"Computer"}; let lang_label=if cs {"Jazyk"}else{"Language"};
            let keyboard_label=if cs {"Klávesnice"}else{"Keyboard"}; let zone_label=if cs {"Časové pásmo"}else{"Time zone"}; let source_label=if cs {"Zdroj"}else{"Source"};
            let source=if online.is_active() {if cs {"Médium a oficiální zrcadla"}else{"Media and official mirrors"}} else if cs {"Podepsané médium"} else {"Signed media"};
            review.set_label(&format!("{disk_label}: {} — {} ({:.1} GiB)\n{serial_label}: {}\n{current_label}: {}\n\n{}\n{firmware_label}: {} / {}\n{account_label}: {} ({})\n{host_label}: {}\n{lang_label}: {}\n{keyboard_label}: {}\n{zone_label}: {}\n{source_label}: {}", disk["path"].as_str().unwrap_or("?"),disk["model"].as_str().unwrap_or(""),disk["size"].as_u64().unwrap_or(0) as f64/1073741824.0,disk["serial"].as_str().unwrap_or("—"),if parts.is_empty(){"—".to_string()}else{parts.join(", ")},action,d["mode"].as_str().unwrap_or("?"),disk["path"].as_str().unwrap_or("?"),text(&name),text(&user),text(&hostname),if cs {"Čeština"} else {"English"},selected(&keyboard,&keyboards),selected(&timezone,&zones),source));
            acknowledgement.set_active(false);
        }
        if i==6 {
          if !acknowledgement.is_active() { fail("Acknowledge the disk changes before installing."); return; }
          let d=discovery.borrow(); let disk=&d["disks"][disks.selected() as usize];
          let request=json!({"version":1,"storage":{"mode":if storage_mode.selected()==0 {"erase"} else {"prepared"},"disk":disk["device"]["path"],"fingerprint":disk["fingerprint"],"root":if storage_mode.selected()==0 {String::new()} else {selected(&root,&root_values.borrow())},"boot":if storage_mode.selected()==0 {String::new()} else {selected(&boot,&boot_values.borrow())}},"account":{"name":text(&name),"user":text(&user),"hostname":text(&hostname),"password":text(&password)},"locale":if language.selected()==1 {"cs_CZ.UTF-8"} else {"en_US.UTF-8"},"keyboard":selected(&keyboard,&keyboards),"timezone":selected(&timezone,&zones),"online":online.is_active()});
          password.set_text(""); repeat.set_text(""); running.set(true); back.set_sensitive(false); next.set_sensitive(false); step.set(7); stack.set_visible_child_name("progress");
          let (tx,rx)=mpsc::channel::<String>();
          std::thread::spawn(move || { let result=(|| -> Result<(),String> { let mut child=Command::new("doas").args([BACKEND,"--install"]).stdin(Stdio::piped()).stdout(Stdio::piped()).stderr(Stdio::null()).spawn().map_err(|e|e.to_string())?; child.stdin.take().unwrap().write_all(request.to_string().as_bytes()).map_err(|e|e.to_string())?; let mut complete=false; for line in BufReader::new(child.stdout.take().unwrap()).lines() { let line=line.map_err(|e|e.to_string())?; if serde_json::from_str::<Value>(&line).ok().is_some_and(|v| v["event"] == "complete") { complete=true; } tx.send(line).map_err(|e|e.to_string())?; } let status=child.wait().map_err(|e|e.to_string())?; if !status.success() || !complete {return Err("Installation failed. The target may be incomplete.".into())} Ok(()) })(); if let Err(e)=result { let _=tx.send(json!({"event":"error","message":e}).to_string()); } });
          let (stack,phase,logs,running,spinner,win,language)=(stack.clone(),phase.clone(),logs.clone(),running.clone(),spinner.clone(),win.clone(),language.clone());
          glib::timeout_add_local(Duration::from_millis(100), move || { while let Ok(line)=rx.try_recv() { if let Ok(v)=serde_json::from_str::<Value>(&line) { let msg=v["message"].as_str().unwrap_or(""); match v["event"].as_str().unwrap_or("") { "phase"=>phase.set_label(&translate(msg,language.selected()==1)), "log"=>{let b=logs.buffer(); let mut end=b.end_iter(); b.insert(&mut end,&format!("{msg}\n"));}, "complete"=>{running.set(false);spinner.stop();stack.set_visible_child_name("done");win.present();return glib::ControlFlow::Break}, "error"=>{running.set(false);spinner.stop();phase.set_label(&format!("{}\n{}", translate(msg,language.selected()==1), if language.selected()==1 {"Cílový systém může být neúplný. Zavřete instalátor a prohlédněte podrobnosti instalace."} else {"The target may be incomplete. Close the installer and inspect the installation details."}));win.present();return glib::ControlFlow::Break}, _=>{} } } } glib::ControlFlow::Continue }); return;
        }
        let n=i+1;step.set(n);stack.set_visible_child_name(pages[n]);back.set_sensitive(true);next.set_label(&translate(if n==6 {"Install"} else {"Next"}, language.selected()==1));
      });
    }
    back.set_sensitive(false);
    win.present();
}
