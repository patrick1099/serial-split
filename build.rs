fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    if std::env::var("TARGET").is_ok_and(|target| target.ends_with("windows-gnullvm")) {
        // gnullvm otherwise imports libunwind.dll from the compiler's bin directory.
        let path = std::env::var_os("PATH").expect("PATH is required");
        let library = std::env::split_paths(&path)
            .filter_map(|bin| bin.parent().map(|root| root.join("x86_64-w64-mingw32/lib")))
            .find(|dir| dir.join("libunwind.a").is_file())
            .expect("Add llvm-mingw/bin to PATH before building");
        println!("cargo:rustc-link-search=native={}", library.display());
        println!("cargo:rustc-link-lib=static=unwind");
    }
}
