// Discovery for libmaned_c — MANEDRS_PLAN.md §4.5.
//
// Resolution order, all target-aware (§5.7 — never key on the host):
//   1. MANED_LIB_DIR / MANED_INCLUDE_DIR — explicit, wins over everything.
//   2. pkg-config for maned_c (§3.3), honouring PKG_CONFIG_PATH and
//      PKG_CONFIG_SYSROOT_DIR. Cross builds need PKG_CONFIG_ALLOW_CROSS=1,
//      which is the pkg-config crate refusing to hand a host library to a
//      cross target — exactly hazard 7's mitigation, so it is left on.
//   3. The conventional prefixes: /usr/local/lib, /opt/homebrew/lib,
//      $HOME/.local/lib.
//   4. Otherwise fail the build with a message naming every path tried, the
//      capi-tarball hint and the MANED_LIB_DIR override. The refusal message
//      is the API contract (§4.1, house style).
//
// A `static` link request is refused (§3.1): LICENSE §3 grants the *shared*
// library only, and a static archive would undo the symbol gate. The
// `docsrs-stub` feature short-circuits to no link directives at all, because
// the docs.rs builder is sandboxed with no network and no library (§4.5).

use std::path::{Path, PathBuf};

fn main() {
    // An upgraded capi tarball must trigger a relink (§4.5).
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-env-changed=MANED_LIB_DIR");
    println!("cargo:rerun-if-env-changed=MANED_INCLUDE_DIR");
    println!("cargo:rerun-if-env-changed=MANED_LINK_STATIC");
    println!("cargo:rerun-if-env-changed=MANED_SKIP_ARCH_CHECK");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_PATH");
    println!("cargo:rerun-if-env-changed=PKG_CONFIG_SYSROOT_DIR");

    if feature("DOCSRS_STUB") {
        // §4.5: documentation builds, nothing links, the API renders.
        println!(
            "cargo:warning=maned-sys: docsrs-stub is on; no link directives \
             emitted. The extern \"C\" declarations are never called."
        );
        println!("cargo:abi_version=1");
        return;
    }

    refuse_static();

    let target_os = env_or_fail("CARGO_CFG_TARGET_OS");
    let target_arch = env_or_fail("CARGO_CFG_TARGET_ARCH");
    let target = std::env::var("TARGET").unwrap_or_else(|_| "<unknown>".into());

    if target_os == "windows" {
        fail(&format!(
            "maned-sys does not support Windows (target {target}).\n\
             \n\
             This is a decision, not an oversight: the engine's serve and net \
             layers are POSIX throughout (MANEDRS_PLAN.md §5.12). Linux and \
             macOS are the supported targets."
        ));
    }

    let mut tried: Vec<String> = Vec::new();
    let found = discover(&target_os, &mut tried)
        .unwrap_or_else(|| fail(&not_found_message(&target, &target_os, &tried)));

    check_arch(&found.lib_file, &target_arch, &target);

    // The link directives proper. `native=` and `dylib=` propagate to every
    // dependent's final link; the rpath link-arg does not — which is why the
    // `maned` crate has a build.rs of its own that re-emits it from
    // DEP_MANED_C_LIB_DIR (see maned/build.rs).
    println!("cargo:rustc-link-search=native={}", found.lib_dir.display());
    println!("cargo:rustc-link-lib=dylib=maned_c");
    println!("cargo:rerun-if-changed={}", found.lib_file.display());

    for arg in rpath_args(&found.lib_dir, &target_os) {
        println!("cargo:rustc-link-arg={arg}");
    }

    // DEP_MANED_C_* metadata for direct dependents — the whole point of
    // `links = "maned_c"` (§2, cost 2).
    println!("cargo:lib_dir={}", found.lib_dir.display());
    println!("cargo:lib={}", found.lib_file.display());
    println!("cargo:abi_version=1");
    if let Some(inc) = &found.include_dir {
        println!("cargo:include_dir={}", inc.display());
    }
    println!("cargo:source={}", found.source);
}

struct Found {
    lib_dir: PathBuf,
    lib_file: PathBuf,
    include_dir: Option<PathBuf>,
    source: &'static str,
}

// --------------------------------------------------------------------------
// Resolution
// --------------------------------------------------------------------------

fn discover(target_os: &str, tried: &mut Vec<String>) -> Option<Found> {
    // 1. Explicit.
    if let Some(dir) = env_dir("MANED_LIB_DIR") {
        let file = lib_path(&dir, target_os);
        tried.push(format!("{} (MANED_LIB_DIR)", file.display()));
        if file.is_file() {
            return Some(Found {
                include_dir: env_dir("MANED_INCLUDE_DIR").or_else(|| sibling_include(&dir)),
                lib_dir: dir,
                lib_file: file,
                source: "MANED_LIB_DIR",
            });
        }
        // An explicit, wrong MANED_LIB_DIR is a mistake worth naming loudly
        // rather than silently falling through to a system copy.
        fail(&format!(
            "MANED_LIB_DIR is set to {} but {} is not there.\n\
             \n\
             MANED_LIB_DIR must name the directory that *contains* the \
             library, i.e. the `lib/` of an unpacked maned-capi tarball.",
            dir.display(),
            lib_file_name(target_os)
        ));
    }

    // 2. pkg-config (§3.3).
    tried.push("pkg-config --libs maned_c".to_string());
    if let Some(found) = try_pkg_config(target_os) {
        return Some(found);
    }

    // 3. Conventional prefixes.
    for dir in conventional_prefixes() {
        let file = lib_path(&dir, target_os);
        tried.push(file.display().to_string());
        if file.is_file() {
            return Some(Found {
                lib_dir: dir.clone(),
                lib_file: file,
                include_dir: sibling_include(&dir),
                source: "conventional prefix",
            });
        }
    }

    None
}

fn try_pkg_config(target_os: &str) -> Option<Found> {
    let lib = pkg_config::Config::new()
        // We emit our own directives, including the rpath pkg-config knows
        // nothing about.
        .cargo_metadata(false)
        .print_system_libs(false)
        .probe("maned_c")
        .ok()?;

    for dir in &lib.link_paths {
        let file = lib_path(dir, target_os);
        if file.is_file() {
            return Some(Found {
                lib_dir: dir.clone(),
                lib_file: file,
                include_dir: lib.include_paths.first().cloned(),
                source: "pkg-config",
            });
        }
    }
    None
}

fn conventional_prefixes() -> Vec<PathBuf> {
    let mut v = vec![
        PathBuf::from("/usr/local/lib"),
        PathBuf::from("/opt/homebrew/lib"),
    ];
    if let Some(home) = std::env::var_os("HOME") {
        v.push(Path::new(&home).join(".local/lib"));
    }
    v
}

fn lib_file_name(target_os: &str) -> &'static str {
    if target_os == "macos" || target_os == "ios" {
        "libmaned_c.dylib"
    } else {
        "libmaned_c.so"
    }
}

fn lib_path(dir: &Path, target_os: &str) -> PathBuf {
    dir.join(lib_file_name(target_os))
}

fn sibling_include(lib_dir: &Path) -> Option<PathBuf> {
    let candidate = lib_dir.parent()?.join("include");
    candidate.join("maned_c.h").is_file().then_some(candidate)
}

fn rpath_args(lib_dir: &Path, target_os: &str) -> Vec<String> {
    let dir = lib_dir.display();
    // The step forgotten in roughly every -sys crate: without an rpath the
    // binary links fine and fails at startup (§4.5). Both the absolute path
    // (works here and now) and the relocatable one (works after the whole
    // tree is moved) are recorded.
    if target_os == "macos" || target_os == "ios" {
        vec![
            format!("-Wl,-rpath,{dir}"),
            "-Wl,-rpath,@loader_path/../lib".to_string(),
        ]
    } else {
        vec![
            format!("-Wl,-rpath,{dir}"),
            "-Wl,-rpath,$ORIGIN/../lib".to_string(),
        ]
    }
}

// --------------------------------------------------------------------------
// Refusals
// --------------------------------------------------------------------------

fn refuse_static() {
    let asked = feature("STATIC")
        || matches!(
            std::env::var("MANED_LINK_STATIC").as_deref(),
            Ok("1") | Ok("true") | Ok("yes")
        );
    if !asked {
        return;
    }
    fail(
        "static linking of libmaned_c is refused, by design \
         (MANEDRS_PLAN.md §3.1).\n\
         \n\
         LICENSE §3 grants redistribution of `libmaned_c.so`, \
         `libmaned_c.dylib` and any equivalent SHARED library, in unmodified \
         form. A static archive is outside that grant on three separate \
         clauses — it copies object code into another binary, and \
         --gc-sections and LTO alter the exported symbol set by design.\n\
         \n\
         It would also undo the symbol gate: -fvisibility=hidden, \
         --version-script and strip are all shared-object mechanisms and none \
         of them apply to a `.a`.\n\
         \n\
         Link dynamically. If you need a single-binary deployment, that is a \
         conversation about a partially linked, symbol-filtered object \
         published under its own grant — not about this flag.",
    );
}

/// Assert the discovered library is built for the *target* (§5.7). A
/// universal macOS dylib in the path is the nastiest version of this bug: the
/// build succeeds and the binary does not run.
fn check_arch(lib: &Path, target_arch: &str, target: &str) {
    if std::env::var_os("MANED_SKIP_ARCH_CHECK").is_some() {
        return;
    }
    let bytes = match std::fs::read(lib) {
        Ok(b) => b,
        Err(_) => return, // unreadable is the linker's problem, not ours
    };
    match arch_matches(&bytes, target_arch) {
        Some(true) | None => {}
        Some(false) => fail(&format!(
            "{} exists but is not built for {target} (target arch {target_arch}).\n\
             \n\
             build.rs runs on the HOST, so MANED_LIB_DIR and the conventional \
             prefixes all point at host artifacts (MANEDRS_PLAN.md §5.7). \
             Point MANED_LIB_DIR at the maned-capi tarball for {target}, or \
             set MANED_SKIP_ARCH_CHECK=1 if you know better than this check.",
            lib.display()
        )),
    }
}

/// `None` = unrecognised container, do not judge.
fn arch_matches(bytes: &[u8], target_arch: &str) -> Option<bool> {
    fn be32(b: &[u8], at: usize) -> Option<u32> {
        let s = b.get(at..at + 4)?;
        Some(u32::from_be_bytes([s[0], s[1], s[2], s[3]]))
    }
    fn le32(b: &[u8], at: usize) -> Option<u32> {
        let s = b.get(at..at + 4)?;
        Some(u32::from_le_bytes([s[0], s[1], s[2], s[3]]))
    }

    // Mach-O CPU types.
    const CPU_X86_64: u32 = 0x0100_0007;
    const CPU_ARM64: u32 = 0x0100_000C;
    let want_macho = match target_arch {
        "x86_64" => CPU_X86_64,
        "aarch64" => CPU_ARM64,
        _ => 0,
    };
    // ELF e_machine.
    let want_elf: u16 = match target_arch {
        "x86_64" => 0x3E,
        "aarch64" => 0xB7,
        "x86" => 0x03,
        "arm" => 0x28,
        "riscv64" => 0xF3,
        _ => 0,
    };

    let magic = be32(bytes, 0)?;
    match magic {
        // Fat / universal Mach-O: any slice matching is a match.
        0xCAFE_BABE | 0xCAFE_BABF => {
            if want_macho == 0 {
                return None;
            }
            let wide = magic == 0xCAFE_BABF;
            let n = be32(bytes, 4)? as usize;
            let (stride, base) = if wide { (32, 8) } else { (20, 8) };
            for i in 0..n.min(64) {
                if be32(bytes, base + i * stride)? == want_macho {
                    return Some(true);
                }
            }
            Some(false)
        }
        // Thin Mach-O, 64-bit, little-endian host order.
        _ if le32(bytes, 0) == Some(0xFEED_FACF) || le32(bytes, 0) == Some(0xFEED_FACE) => {
            if want_macho == 0 {
                return None;
            }
            Some(le32(bytes, 4)? == want_macho)
        }
        // ELF.
        _ if bytes.starts_with(b"\x7fELF") => {
            if want_elf == 0 {
                return None;
            }
            let little = bytes.get(5) == Some(&1);
            let s = bytes.get(18..20)?;
            let machine = if little {
                u16::from_le_bytes([s[0], s[1]])
            } else {
                u16::from_be_bytes([s[0], s[1]])
            };
            Some(machine == want_elf)
        }
        _ => None,
    }
}

fn not_found_message(target: &str, target_os: &str, tried: &[String]) -> String {
    let mut m = String::new();
    m.push_str("libmaned_c was not found, so maned-sys cannot link.\n\nTried, in order:\n");
    for t in tried {
        m.push_str("  - ");
        m.push_str(t);
        m.push('\n');
    }
    m.push_str(&format!(
        "\nUnlike a wheel or a jar, a .crate cannot ship a per-platform binary \
         (MANEDRS_PLAN.md §4.5): the library has to be installed first.\n\
         \n\
         Install the capi tarball for {target}:\n\
         \n    curl -LO https://github.com/maned-lang/maned/releases/latest/download/maned-capi-$VERSION-{target}.tar.gz\n\
         \n    tar xzf maned-capi-$VERSION-{target}.tar.gz -C $HOME/.local --strip-components=1\n\
         \nor point the build at an unpacked tarball directly:\n\
         \n    MANED_LIB_DIR=/path/to/maned-capi/lib cargo build\n\
         \n(MANED_LIB_DIR names the directory containing {}; MANED_INCLUDE_DIR \
         is optional and only used for metadata.)",
        lib_file_name(target_os)
    ));
    m
}

// --------------------------------------------------------------------------
// Small helpers
// --------------------------------------------------------------------------

fn feature(name: &str) -> bool {
    std::env::var_os(format!("CARGO_FEATURE_{name}")).is_some()
}

fn env_dir(name: &str) -> Option<PathBuf> {
    let v = std::env::var_os(name)?;
    if v.is_empty() {
        return None;
    }
    Some(PathBuf::from(v))
}

fn env_or_fail(name: &str) -> String {
    std::env::var(name).unwrap_or_else(|_| fail(&format!("cargo did not set {name}")))
}

fn fail(message: &str) -> ! {
    // One `cargo:warning` per line keeps the message readable; the panic is
    // what actually stops the build.
    for line in message.lines() {
        println!("cargo:warning={line}");
    }
    panic!("\n\n{message}\n\n");
}
