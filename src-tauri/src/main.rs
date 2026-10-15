// No console window in release builds.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    let args: Vec<String> = std::env::args().collect();
    // Elevated helper mode: clean admin-only rules and exit without opening a window.
    if let Some(pos) = args.iter().position(|a| a == "--elevated-clean") {
        let code = reclaim_app::helper::run(args.get(pos + 1).map(String::as_str));
        std::process::exit(code);
    }
    // After "Restart as administrator" the old instance may still be closing; wait for it so
    // the single-instance check does not mistake it for a second copy.
    if let Some(pid) = args
        .iter()
        .position(|a| a == "--wait-parent")
        .and_then(|pos| args.get(pos + 1))
        .and_then(|p| p.parse::<u32>().ok())
    {
        reclaim_app::helper::wait_for_process(pid);
    }
    reclaim_app::run();
}
