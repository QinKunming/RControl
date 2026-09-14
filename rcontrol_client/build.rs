fn main() {
    println!("cargo:rerun-if-changed=manifest.xml");
    println!("cargo:rerun-if-changed=resource.rc");
    let target = std::env::var("TARGET").unwrap_or_default();

    if target.contains("msvc") {
        embed_resource::compile("resource.rc", &[] as &[&str]);
        return;
    }

    // GNU 工具链: 优先 windres; 没有则用 Windows SDK 的 rc.exe + cvtres.exe
    if std::process::Command::new("windres").arg("--version").output().is_ok() {
        embed_resource::compile("resource.rc", &[] as &[&str]);
        return;
    }
    match compile_with_sdk_rc(&target) {
        Ok(obj) => println!("cargo:rustc-link-arg={}", obj.display()),
        Err(e) => println!("cargo:warning=资源编译跳过 (仅影响按钮主题样式): {}", e),
    }
}

/// rc.exe (SDK) 编译 .rc -> .res, cvtres.exe (MSVC) 转 COFF .obj, 交给 GNU ld 链接
fn compile_with_sdk_rc(target: &str) -> Result<std::path::PathBuf, String> {
    let rc = find_file(
        &[r"C:\Program Files (x86)\Windows Kits", r"C:\Program Files\Windows Kits"],
        3, // bin/<sdkver>/<arch>/rc.exe
        "rc.exe",
        target,
    )
    .ok_or("找不到 rc.exe")?;
    let cvtres = find_cvtres(target).ok_or("找不到 cvtres.exe")?;

    let out_dir = std::path::PathBuf::from(std::env::var("OUT_DIR").unwrap());
    let res = out_dir.join("resource.res");
    let obj = out_dir.join("resource.obj");
    let crate_dir = std::path::PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());

    let st = std::process::Command::new(&rc)
        .arg("/nologo")
        .arg(format!("/fo{}", res.display()))
        .arg("resource.rc")
        .current_dir(&crate_dir)
        .status()
        .map_err(|e| format!("rc.exe: {}", e))?;
    if !st.success() {
        return Err("rc.exe 编译失败".into());
    }
    let machine = if target.contains("x86_64") || target.contains("aarch64") {
        "x64"
    } else {
        "x86"
    };
    let st = std::process::Command::new(&cvtres)
        .arg("/nologo")
        .arg(format!("/machine:{}", machine))
        .arg(format!("/out:{}", obj.display()))
        .arg(&res)
        .status()
        .map_err(|e| format!("cvtres.exe: {}", e))?;
    if !st.success() {
        return Err("cvtres.exe 转换失败".into());
    }
    Ok(obj)
}

/// 在 Windows Kits 树下深搜 rc.exe (取最高 SDK 版本)
fn find_file(roots: &[&str], depth: u32, name: &str, target: &str) -> Option<std::path::PathBuf> {
    if depth == 0 {
        return None;
    }
    let arch = if target.contains("aarch64") {
        "arm64"
    } else if target.contains("x86_64") {
        "x64"
    } else {
        "x86"
    };
    for root in roots {
        let rd = match std::fs::read_dir(root) {
            Ok(r) => r,
            Err(_) => continue,
        };
        let mut subs: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
        subs.sort(); // 版本号字典序, 高版本在后
        for p in subs.iter().rev() {
            // SDK 布局: bin/<ver>/x64/rc.exe
            let cand = p.join(arch).join(name);
            if cand.is_file() {
                return Some(cand);
            }
            if let Some(hit) = find_file_sub(p, depth - 1, name, arch) {
                return Some(hit);
            }
        }
    }
    None
}

fn find_file_sub(dir: &std::path::Path, depth: u32, name: &str, arch: &str) -> Option<std::path::PathBuf> {
    if depth == 0 {
        return None;
    }
    let rd = std::fs::read_dir(dir).ok()?;
    let mut subs: Vec<_> = rd.filter_map(|e| e.ok()).map(|e| e.path()).collect();
    subs.sort();
    for p in subs.iter().rev() {
        let cand = p.join(arch).join(name);
        if cand.is_file() {
            return Some(cand);
        }
        if let Some(hit) = find_file_sub(p, depth - 1, name, arch) {
            return Some(hit);
        }
    }
    None
}

/// VS 布局: <VS>/<edition>/VC/Tools/MSVC/<ver>/bin/Hostx64/<arch>/cvtres.exe
fn find_cvtres(target: &str) -> Option<std::path::PathBuf> {
    let arch = if target.contains("x86_64") { "x64" } else { "x86" };
    for vs_root in [
        r"C:\Program Files\Microsoft Visual Studio",
        r"C:\Program Files (x86)\Microsoft Visual Studio",
    ] {
        // 第一个根目录可能不存在, 跳过继续找下一个 (不能用 `?`, 会整函数早退)
        let Ok(rd) = std::fs::read_dir(vs_root) else { continue };
        for year in rd.filter_map(|e| e.ok()) {
            // VS 布局: <root>\<年份>\<产品(BuildTools/Community/...)>\VC\Tools\MSVC\<ver>\...
            let Ok(er) = std::fs::read_dir(year.path()) else { continue };
            for edition in er.filter_map(|e| e.ok()) {
                let ms = edition.path().join("VC").join("Tools").join("MSVC");
                let mut vers: Vec<_> = std::fs::read_dir(&ms).ok()
                    .map(|rd| rd.filter_map(|e| e.ok()).map(|e| e.path()).collect())
                    .unwrap_or_default();
                vers.sort();
                for v in vers.iter().rev() {
                    for host in ["Hostx64", "Hostx86"] {
                        let cand = v.join("bin").join(host).join(arch).join("cvtres.exe");
                        if cand.is_file() {
                            return Some(cand);
                        }
                    }
                }
            }
        }
    }
    None
}
