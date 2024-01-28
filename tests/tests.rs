use std::{env, io};

use jni_loader::{locate, JNI};
use test_log::test;

#[test]
fn find_libc() {
    match locate::locate_library("libc.so", None) {
        Some(libc_path) => println!("Found libc: {:?}", libc_path),
        None => panic!("Failed to find libc.so"),
    };
}

extern "C" fn add2(a: std::ffi::c_int, b: std::ffi::c_int) -> std::ffi::c_int {
    a + b
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
    lib.add_dependency("libc.so.6", None);
    lib.load_dependencies().expect("Failed to load dependencies");
    lib.override_symbol("_ITM_deregisterTMCloneTable", None);
    lib.override_symbol("__gmon_start__", None);
    lib.override_symbol("_ITM_registerTMCloneTable", None);
    lib.override_symbol("__cxa_finalize", None);
    lib.override_symbol("add2", Some(add2 as *const ()));
    lib.initialize();

    let (test_add2, _) = lib.get_symbol("test_add2").unwrap();
    let test_add2: extern "C" fn() -> std::ffi::c_int = unsafe { std::mem::transmute(test_add2) };
    assert_eq!(3 + 3, test_add2());

    Ok(())
}
