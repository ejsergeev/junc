// SPDX-License-Identifier: GPL-3.0-or-later
fn main() {
    // Debug info lets tests find elements by their accessible labels
    // (spike S6); release builds leave it out.
    let debug = std::env::var("PROFILE").is_ok_and(|p| p != "release");
    let config = slint_build::CompilerConfiguration::new().with_debug_info(debug);
    if let Err(e) = slint_build::compile_with_config("ui/app.slint", config) {
        panic!("ui/app.slint: {e}");
    }
}
