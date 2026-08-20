use std::process::Command;

fn main() {
    println!("cargo:rerun-if-changed=native/pg_native.cpp");
    let cflags = pkg_config("libavformat");
    let mut compile = Command::new("g++");
    compile.args(["-std=c++17", "-O3", "-fPIC", "-c", "native/pg_native.cpp", "-o"]);
    let object = std::path::Path::new(&std::env::var("OUT_DIR").unwrap()).join("pg_native.o");
    compile.arg(object);
    for flag in cflags.iter().filter(|flag| flag.starts_with("-I")) {
        compile.arg(flag);
    }
    if !compile.status().expect("start g++").success() {
        panic!("native FFmpeg bridge compilation failed");
    }
    let out = std::env::var("OUT_DIR").unwrap();
    println!("cargo:rustc-link-search=native={out}");
    println!("cargo:rustc-link-arg={out}/pg_native.o");
    for lib in ["avformat", "avcodec", "avutil", "swresample"] {
        println!("cargo:rustc-link-lib=dylib={lib}");
    }
    println!("cargo:rustc-link-lib=dylib=stdc++");
}

fn pkg_config(package: &str) -> Vec<String> {
    Command::new("pkg-config")
        .args(["--cflags", package])
        .output()
        .ok()
        .filter(|output| output.status.success())
        .map(|output| String::from_utf8_lossy(&output.stdout).split_whitespace().map(str::to_string).collect())
        .unwrap_or_default()
}
