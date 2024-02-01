use std::env;

use anyhow::Result;
use jni_loader::{locate, JNI};
use test_log::test;

#[test]
fn find_libc() {
    match locate::locate_library("libc.so.6", None) {
        Some(libc_path) => println!("Found libc: {:?}", libc_path),
        None => panic!("Failed to find libc.so"),
    };
}

extern "C" fn m_cube(x: std::ffi::c_int) -> std::ffi::c_int {
    x * x * x
}

#[test]
fn native_library() -> Result<()> {
    let linking_dir = env::current_dir()?.join("tests").join("linking");
    let lib_math_path = linking_dir.clone().join("libmath.so");
    let lib_power_path = linking_dir.clone().join("libpower.so");

    let mut lib_math = JNI::new(lib_math_path)?;
    lib_math.add_dependency("libc.so.6", None);
    lib_math.load_dependencies()?;
    lib_math.initialize();

    let mut lib_power = JNI::new(lib_power_path)?;
    lib_power.add_dependency("libc.so.6", None);
    lib_power.add_dependency("libmath.so", Some(lib_math));
    lib_power.load_dependencies()?;
    lib_power.override_symbol("m_cube", Some(m_cube as *const ()));
    lib_power.initialize();

    let (test_libpower, _) = lib_power.get_symbol("test_libpower").unwrap();
    let test_libpower: extern "C" fn() -> std::ffi::c_int = unsafe { std::mem::transmute(test_libpower) };
    assert_eq!(0, test_libpower());

    Ok(())
}
