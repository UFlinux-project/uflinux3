use settings_core::{bus, *};
use std::io::{Read, Write};

unsafe extern "C" {
    fn uflinux_renderer_probe() -> libc::c_int;
}
fn main() {
    if let Err(e) = run() {
        eprintln!("{e}");
        std::process::exit(1);
    }
}
fn run() -> Result<()> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    if args.first().map(String::as_str) == Some("renderer") {
        let user_dir = files::config_dir().parent().unwrap().join("labwc");
        let mut sources = vec![user_dir.join("environment")];
        let mut extra = std::fs::read_dir(user_dir.join("environment.d"))
            .into_iter()
            .flatten()
            .flatten()
            .map(|e| e.path())
            .filter(|p| p.extension().is_some_and(|x| x == "env"))
            .collect::<Vec<_>>();
        extra.sort();
        sources.extend(extra);
        let mut explicit = None;
        for path in sources {
            if let Ok(text) = std::fs::read_to_string(path) {
                for line in text.lines() {
                    if let Some((key, value)) = line.trim().split_once('=') {
                        if key == "WLR_RENDERER" && ["vulkan", "gles2", "pixman"].contains(&value) {
                            explicit = Some(value.to_string());
                        }
                    }
                }
            }
        }
        if let Some(renderer) = explicit {
            println!("explicit:{renderer}");
        } else {
            println!(
                "{}",
                match unsafe { uflinux_renderer_probe() } {
                    2 => "vulkan",
                    1 => "gles2",
                    _ => "pixman",
                }
            );
        }
        return Ok(());
    }
    glib::MainContext::default().block_on(async {
        if args.first().map(String::as_str)==Some("system-request") {
            let mut raw=String::new();std::io::stdin().lock().take(MAX_REQUEST as u64+1).read_to_string(&mut raw).map_err(|e|e.to_string())?;
            if raw.len()>MAX_REQUEST{return Err("Request too large".into());}
            let operation: SystemAction=serde_json::from_str(&raw).map_err(|_|"Invalid typed system request")?;
            let request=serde_json::to_string(&operation).map_err(|_|"Cannot encode system operation")?;
            let connection=gio::bus_get_future(gio::BusType::System).await.map_err(|e|e.to_string())?;
            use glib::variant::ToVariant;
            let reply=bus::call(&connection,SYSTEM_NAME,SYSTEM_PATH,SYSTEM_NAME,"Execute",Some(&(request,).to_variant())).await?;
            println!("{}",reply.get::<(String,)>().ok_or("Invalid system reply")?.0);return Ok(());
        }
        let number=|| args.get(1).ok_or("Missing value")?.parse::<f64>().map_err(|_|"Invalid value".to_string());
        let enabled=||match args.get(1).map(String::as_str){Some("on")=>Ok(true),Some("off")=>Ok(false),_=>Err("Choose on or off".to_string())};
        let action=match args.first().map(String::as_str) {
            Some("status")=> { println!("{}",bus::session("GetState",None).await?); return Ok(()); }
            Some("brightness")=>Action::AdjustBrightness{delta:number()?},
            Some("volume")=>Action::AdjustVolume{delta:number()?},
            Some("mute")=>Action::ToggleMute,
            Some("network")=>Action::Networking{enabled:enabled()?},
            Some("wifi")=>Action::Wifi{enabled:enabled()?},
            Some("scan")=>Action::Scan,
            Some("power-profile")=>Action::SetPowerProfile{profile:PowerProfile::parse(args.get(1).ok_or("Choose a power profile")?)?},
            Some("request")=> {
                // Automation uses stdin, never a password-bearing command argument.
                let mut input=String::new();
                std::io::stdin().lock().take(MAX_REQUEST as u64+1).read_to_string(&mut input).map_err(|e|e.to_string())?;
                if input.len()>MAX_REQUEST {return Err("Request too large".into());}
                serde_json::from_str(&input).map_err(|_|"Invalid typed request".to_string())?
            }
            _=>return Err("Usage: uflinux-settingsctl status|renderer|volume DELTA|brightness DELTA|mute|power-profile power-saver/balanced/performance|network on/off|wifi on/off|scan|request|system-request < request.json".into()),
        };
        let result=bus::execute(&action).await?;
        writeln!(std::io::stdout(),"{result}").map_err(|e|e.to_string())
    })
}
