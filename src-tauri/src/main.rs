// Desktop entry: CLI commands first, otherwise the window app. Mobile enters via `tikdl_lib::run`.
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    if !tikdl_lib::cli_main() {
        tikdl_lib::run();
    }
}
