#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // On Linux the same binary doubles as the detached proxy helper daemon when
    // launched with `--proxy-helper` (so there's no separate binary to package
    // or locate). It owns the loopback listener and outlives this GUI process;
    // see `gate_connect_core::proxy::helper`. Dispatch before Tauri starts.
    #[cfg(target_os = "linux")]
    if std::env::args().skip(1).any(|a| a == "--proxy-helper") {
        if let Err(e) = gate_connect_core::proxy::helper::run_daemon() {
            eprintln!("gate proxy helper exited: {e}");
            std::process::exit(1);
        }
        return;
    }

    gate_connect_desktop_lib::run()
}
