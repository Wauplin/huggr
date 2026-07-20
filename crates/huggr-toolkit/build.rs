use std::path::Path;

fn main() {
    println!("cargo:rerun-if-changed=.cargo_vcs_info.json");
    if !Path::new(".cargo_vcs_info.json").exists() {
        println!(
            "cargo:rustc-env=HUGGR_TOOLKIT_DEV_PATH={}",
            std::env::var("CARGO_MANIFEST_DIR").expect("Cargo sets CARGO_MANIFEST_DIR")
        );
    }
}
