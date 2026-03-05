// Keep the Windows build fully windowed and rely on file logging instead of a console.
#![cfg_attr(target_os = "windows", windows_subsystem = "windows")]

fn main() {
    equirust_lib::run();
}
