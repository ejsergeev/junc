// SPDX-License-Identifier: GPL-3.0-or-later
fn main() {
    // Debug info lets the testing backend find elements by their labels.
    let config = slint_build::CompilerConfiguration::new()
        .with_bundled_translations("translations")
        .with_debug_info(true);
    slint_build::compile_with_config("ui/app.slint", config).expect("slint");
}
