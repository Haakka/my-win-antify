use std::{env, fs, path::PathBuf};

fn main() {
    let vendor = PathBuf::from(env::var_os("CARGO_MANIFEST_DIR").unwrap())
        .join("../../vendor/windivert/x64");
    let out = PathBuf::from(env::var_os("OUT_DIR").unwrap());
    // Cargo versions use different numbers of intermediate directories below build/.
    let profile = out
        .ancestors()
        .find(|path| path.file_name().is_some_and(|name| name == "build"))
        .and_then(|path| path.parent())
        .expect("Cargo profile directory");
    for file in ["WinDivert.dll", "WinDivert64.sys"] {
        let source = vendor.join(file);
        let destination = profile.join(file);
        println!("cargo:rerun-if-changed={}", source.display());
        println!("cargo:rerun-if-changed={}", destination.display());
        let expected =
            fs::read(&source).unwrap_or_else(|e| panic!("read {}: {e}", source.display()));
        if fs::read(&destination).as_deref().ok() != Some(expected.as_slice()) {
            fs::write(&destination, expected)
                .unwrap_or_else(|e| panic!("copy to {}: {e}", destination.display()));
        }
    }
    println!(
        "cargo:rustc-env=MY_WIN_ANTIFY_DRIVER_TEST_DIR={}",
        profile.display()
    );
}
