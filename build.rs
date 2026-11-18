use std::{env, fs, path::PathBuf, process::Command};

fn main() {
    println!("cargo:rerun-if-changed=assets/logo.ico");
    println!("cargo:rerun-if-changed=assets/logo.png");
    println!("cargo:rerun-if-changed=assets/logo.svg");
    println!("cargo:rerun-if-changed=config.toml");

    if env::var("CARGO_CFG_TARGET_OS").as_deref() != Ok("windows") {
        return;
    }

    // When building via `dx` (Dioxus CLI), `dx` already compiles and links the
    // Windows icon resource from `Dioxus.toml` (`.winres/resource.lib` with ICON 1).
    // Skipping our custom resource compilation prevents `CVT1100: duplicate resource`
    // and `LNK1123` linker errors on Windows MSVC.
    if env::var_os("DX_RUSTC").is_some() || env::var_os("DIOXUS_CLI_ENABLED").is_some() {
        return;
    }

    let manifest_dir = match env::var("CARGO_MANIFEST_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => return,
    };
    let out_dir = match env::var("OUT_DIR") {
        Ok(dir) => PathBuf::from(dir),
        Err(_) => return,
    };

    let ico_path = manifest_dir.join("assets").join("logo.ico");
    if !ico_path.exists() {
        return;
    }

    let rc_path = out_dir.join("postier_icon.rc");
    let ico_escaped = ico_path.display().to_string().replace('\\', "\\\\");
    if fs::write(&rc_path, format!("1 ICON \"{ico_escaped}\"\n")).is_err() {
        return;
    }

    let target_env = env::var("CARGO_CFG_TARGET_ENV").unwrap_or_default();
    if target_env == "gnu" {
        let res_obj = out_dir.join("postier_icon.o");
        for tool in ["x86_64-w64-mingw32-windres", "windres"] {
            if Command::new(tool)
                .arg(&rc_path)
                .arg("-O")
                .arg("coff")
                .arg("-o")
                .arg(&res_obj)
                .status()
                .is_ok_and(|s| s.success())
            {
                println!("cargo:rustc-link-arg-bins={}", res_obj.display());
                break;
            }
        }
    } else {
        let res_file = out_dir.join("postier_icon.res");
        for tool in ["rc.exe", "llvm-rc"] {
            if Command::new(tool)
                .arg("/fo")
                .arg(&res_file)
                .arg(&rc_path)
                .status()
                .is_ok_and(|s| s.success())
            {
                println!("cargo:rustc-link-arg-bins={}", res_file.display());
                break;
            }
        }
    }
}
