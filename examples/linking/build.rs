fn main() {
    std::process::Command::new("gcc")
        .arg("-shared")
        .arg("-fPIC")
        .arg("src/math.c")
        .args(["-o", "libmath.so"])
        .output()
        .expect("Failed to compile libmath");

    std::process::Command::new("gcc")
        .arg("-shared")
        .arg("-fPIC")
        .arg("src/power.c")
        .args(["-o", "libpower.so"])
        .arg("-lmath")
        .arg("-L.")
        .output()
        .expect("Failed to compile libpower");

    std::process::Command::new("gcc")
        .arg("src/main.c")
        .args(["-o", "main"])
        .arg("-lpower")
        .arg("-lmath")
        .arg("-L.")
        .args(["-Wl,-rpath", "."])
        .output()
        .expect("Failed to compile main executable");
}
