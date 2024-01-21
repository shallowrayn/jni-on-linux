use std::{env, io};

use jni_loader::{locate, JNI};

#[test]
fn find_libc() {
    match locate::locate_library("libc.so", None) {
        Some(libc_path) => println!("Found libc: {:?}", libc_path),
        None => panic!("Failed to find libc.so"),
    };
}

#[test]
fn native_library() -> io::Result<()> {
    let path = env::current_dir()?.join("tests").join("native-lib.so");
    let mut lib = match JNI::new(path.clone()) {
        Ok(lib) => lib,
        Err(err) => match err {
            jni_loader::Error::FileNotFound => {
                panic!("Expected to find shared library at {:?}, please run build-native.sh", path);
            },
            _ => panic!("Error when loading library: {:?}", err),
        },
    };
    lib.load_dependencies();

    Ok(())
}
