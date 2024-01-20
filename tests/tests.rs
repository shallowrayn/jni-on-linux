use std::path::PathBuf;

use jni_loader::{locate, JNI};

#[test]
fn find_libc() {
    match locate::locate_library("libc.so", None) {
        Some(libc_path) => println!("Found libc: {:?}", libc_path),
        None => panic!("Failed to find libc.so"),
    };
}

#[test]
fn spotify() {
    let Ok(jni) = JNI::new(PathBuf::from("./liborbit-jni-spotify-8.8.96-x86_64.so")) else {
        println!("Failed to load Spotify JNI, download from https://drive.proton.me/urls/RJX1MDD6KG#VrZCZk72EmBL");
        return;
    };
}
