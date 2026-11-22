//! TMouseOpen — Linux configurator for the Titorion YDX-GM009 (2.4G receiver 04f3:026f).
//!
//! Usage:
//!   tmouseopen            open the UI
//!   tmouseopen --demo     UI with a simulated mouse (no hardware needed)
//!   tmouseopen --list     list the receiver's hidraw nodes

mod device;
mod privs;
mod protocol;
mod state;
mod ui;
mod worker;

fn main() -> eframe::Result {
    let args: Vec<String> = std::env::args().skip(1).collect();

    let demo = args.iter().any(|a| a == "--demo") || std::env::var_os("TMOUSEOPEN_DEMO").is_some();

    // Under sudo: open the receiver, then drop root for good before anything else happens.
    let prepared = match privs::prepare(demo) {
        Ok(p) => p,
        Err(e) => {
            eprintln!("{e}");
            std::process::exit(1);
        }
    };
    if let privs::Mode::Dropped { home, .. } = &prepared.mode {
        state::set_home_override(home.clone());
    }

    if args.iter().any(|a| a == "--list") {
        let c = device::candidates();
        if c.is_empty() {
            println!("No hidraw node found for receiver 04f3:026f.");
        }
        for c in c {
            let tag = if c.interface == protocol::CONTROL_INTERFACE { "  <- control" } else { "" };
            println!("{}  interface={}  descriptor={:?} bytes{tag}", c.node.display(), c.interface, c.rdesc_len);
        }
        return Ok(());
    }

    let sudo_mode = matches!(prepared.mode, privs::Mode::Dropped { .. });
    let initial = prepared.device;
    let note = prepared.note;
    let options = eframe::NativeOptions {
        viewport: eframe::egui::ViewportBuilder::default()
            .with_title("TMouseOpen")
            .with_app_id("tmouseopen")
            .with_inner_size([960.0, 700.0])
            .with_min_inner_size([820.0, 600.0]),
        ..Default::default()
    };
    eframe::run_native("TMouseOpen", options, Box::new(move |cc| Ok(Box::new(ui::App::new(cc, demo, initial, sudo_mode, note)))))
}
