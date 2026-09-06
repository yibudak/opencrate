fn main() {
    println!("cargo:rerun-if-changed=opencrate.rc");
    println!("cargo:rerun-if-changed=../../assets/branding/opencrate-icon.ico");
    let version = std::env::var("CARGO_PKG_VERSION").expect("package version");
    let major = std::env::var("CARGO_PKG_VERSION_MAJOR").unwrap();
    let minor = std::env::var("CARGO_PKG_VERSION_MINOR").unwrap();
    let patch = std::env::var("CARGO_PKG_VERSION_PATCH").unwrap();
    // Render values into the resource to avoid compiler-specific macro quoting.
    let icon = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../assets/branding/opencrate-icon.ico")
        .canonicalize()
        .expect("OpenCrate icon path");
    let source = include_str!("opencrate.rc")
        .replace(
            "OPENCRATE_VERSION_NUMBERS",
            &format!("{major},{minor},{patch},0"),
        )
        .replace("OPENCRATE_VERSION", &format!("\"{version}\""))
        .replace(
            "../../assets/branding/opencrate-icon.ico",
            icon.to_string_lossy()
                .replace('\\', "/")
                .trim_start_matches("//?/"),
        );
    let resource =
        std::path::PathBuf::from(std::env::var_os("OUT_DIR").unwrap()).join("opencrate.rc");
    std::fs::write(&resource, source).expect("write OpenCrate version resource");
    embed_resource::compile_for(resource, ["opencrate-ui"], embed_resource::NONE)
        .manifest_optional()
        .expect("compile OpenCrate Windows icon and version resources");
}
