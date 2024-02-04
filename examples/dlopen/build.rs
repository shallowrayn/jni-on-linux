fn main() {
    std::process::Command::new("gcc")
        .arg("-shared")
        .arg("-fPIC")
        .arg("src/store.c")
        .args(["-o", "libstore.so"])
        .output()
        .expect("Failed to compile libstore");

    std::process::Command::new("gcc")
        .arg("-shared")
        .arg("-fPIC")
        .arg("src/wrapper.c")
        .args(["-o", "libwrapper.so"])
        .arg("-ldl")
        .output()
        .expect("Failed to compile libstore");
}
