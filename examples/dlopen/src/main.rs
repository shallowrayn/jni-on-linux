use anyhow::Result;
use jni_loader::JNI;

fn load_wrapper() -> Result<Box<JNI>> {
    let current_path = std::env::current_dir()?;

    let store_path = current_path.clone().join("libstore.so");
    let mut store = JNI::new(store_path)?;
    store.add_dependency("libc.so.6", None);
    store.load_dependencies()?;
    store.initialize();

    let wrapper_path = current_path.clone().join("libwrapper.so");
    let mut wrapper = JNI::new(wrapper_path)?;
    wrapper.add_dependency("libstore.so", Some(store));
    wrapper.add_dependency("libc.so.6", None);
    wrapper.load_dependencies()?;
    wrapper.override_symbol("printf", Some(libc::printf as *const ()));
    wrapper.initialize();
    Ok(wrapper)
}

fn main() -> Result<()> {
    env_logger::init();

    let mut wrapper_one = load_wrapper()?;
    let (test_store_one, _) = wrapper_one.get_symbol("test_store").unwrap();
    let test_store_one: extern "C" fn() = unsafe { std::mem::transmute(test_store_one) };
    test_store_one();

    let mut wrapper_two = load_wrapper()?;
    let (test_store_two, _) = wrapper_two.get_symbol("test_store").unwrap();
    let test_store_two: extern "C" fn() = unsafe { std::mem::transmute(test_store_two) };
    test_store_two();

    Ok(())
}
