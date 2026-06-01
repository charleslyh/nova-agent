fn main() {
    let manifest_dir =
        std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").expect("CARGO_MANIFEST_DIR"));

    // 目录级 watch：新增/修改 resource 文件时让 build.rs 重跑，从而再次执行 tauri-build 的拷贝。
    println!("cargo:rerun-if-changed=resources");
    println!("cargo:rerun-if-changed=tauri.conf.json");
    println!("cargo:rerun-if-changed=tauri.pro.conf.json");

    // 将源码 skills 目录的绝对路径编译进 client（`bundle::COMPILED_SKILLS_SRC`）。
    //
    // 背景：`tauri-build` 会把 `resources/skills` → `target/<profile>/skills`、
    // `resources/tools.toml` → `tools.toml`（见 tauri.conf.json），但拷贝只在 build.rs
    // 执行时发生，且对新文件不总触发重跑。dev 时 target 副本可能滞后或为空。
    //
    // debug 构建在运行时优先用这里的源码路径，改 `resources/skills/` 后重新编译 client 即可生效。
    let skills_src = manifest_dir.join("resources/skills");
    if skills_src.is_dir() {
        println!(
            "cargo:rustc-env=MORAY_SKILLS_SRC_DIR={}",
            skills_src.display()
        );
    }

    let tools_catalog_src = manifest_dir.join("resources/tools.toml");
    if tools_catalog_src.is_file() {
        println!(
            "cargo:rustc-env=MORAY_TOOLS_CATALOG_SRC={}",
            tools_catalog_src.display()
        );
    }

    let settings_src = manifest_dir.join("resources/settings.toml");
    if settings_src.is_file() {
        println!(
            "cargo:rustc-env=MORAY_SETTINGS_SRC={}",
            settings_src.display()
        );
    }

    let sessions_catalog_src = manifest_dir.join("resources/sessions.toml");
    if sessions_catalog_src.is_file() {
        println!(
            "cargo:rustc-env=MORAY_SESSIONS_CATALOG_SRC={}",
            sessions_catalog_src.display()
        );
    }

    tauri_build::build()
}
