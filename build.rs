use std::path::PathBuf;
use std::process::Command;

fn main() {
    let config = slint_build::CompilerConfiguration::new().with_style("fluent-light".into());
    slint_build::compile_with_config("ui/app.slint", config).expect("Slint UI compilation failed");

    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }
    println!("cargo:rerun-if-changed=app.rc");
    println!("cargo:rerun-if-changed=assets/nobet_panel.ico");
    if std::env::var("CARGO_CFG_TARGET_ENV").as_deref() == Ok("gnu") {
        // embed-resource passes --include-dir, which GNU windres breaks on paths
        // containing spaces; call windres directly and link the object instead.
        let out = PathBuf::from(std::env::var("OUT_DIR").unwrap()).join("app_icon.o");
        match Command::new("windres").args(["--input", "app.rc", "--output-format=coff", "--output"]).arg(&out).status() {
            Ok(s) if s.success() => println!("cargo:rustc-link-arg-bins={}", out.display()),
            _ => println!("cargo:warning=windres not available; the exe will have no embedded icon"),
        }
    } else {
        let _ = embed_resource::compile("app.rc", embed_resource::NONE);
    }
}
