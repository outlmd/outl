// Prevents additional console window on Windows in release, DO NOT REMOVE!!
#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

fn main() {
    // The zero every boot measurement counts from, first statement so
    // it brackets the whole launch. `outl_core::boot_timing` owns the
    // rest; see `docs/boot-performance.md`.
    outl_core::boot_timing::note_process_start();
    outl_desktop_lib::run()
}
