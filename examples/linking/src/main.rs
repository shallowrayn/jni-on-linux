use std::{env, ffi::c_int};

use anyhow::Result;
use jni_loader::JNI;

extern "C" fn m_cube(x: c_int) -> c_int {
    x * x * x
}

fn main() -> Result<()> {
    env_logger::init();
    let current_dir = env::current_dir()?.join("examples").join("linking");
    let lib_math_path = current_dir.clone().join("libmath.so");
    let lib_power_path = current_dir.clone().join("libpower.so");

    let mut lib_math = JNI::new(lib_math_path)?;
    lib_math.add_dependency("libc.so.6", None);
    lib_math.load_dependencies()?;
    lib_math.initialize()?;

    let mut lib_power = JNI::new(lib_power_path)?;
    lib_power.add_dependency("libc.so.6", None);
    lib_power.add_dependency("libmath.so", Some(lib_math));
    lib_power.load_dependencies()?;
    lib_power.override_symbol("m_cube", Some(m_cube as *const ()));
    lib_power.initialize()?;

    let (test_libpower, _) = lib_power.get_symbol("test_libpower").unwrap();
    let test_libpower: extern "C" fn() -> std::ffi::c_int = unsafe { std::mem::transmute(test_libpower) };
    println!("test_libpower() - {}", test_libpower());

    Ok(())
}
