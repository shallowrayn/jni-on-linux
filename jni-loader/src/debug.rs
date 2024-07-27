use std::{
    collections::HashMap,
    ffi::{c_char, CString, NulError},
    sync::{Arc, Mutex},
};

use lazy_static::lazy_static;

lazy_static! {
    static ref LIBRARIES: Arc<Mutex<HashMap<u64, CString>>> = Arc::new(Mutex::new(HashMap::new()));
}

pub(crate) fn add_library(base_address: u64, name: &str) -> Result<(), NulError> {
    let mut libraries = LIBRARIES.lock().unwrap();
    let name = CString::new(name.as_bytes())?;
    let name_ptr = name.as_ptr();
    libraries.insert(base_address, name);
    drop(libraries);
    jni_loader_lib_loaded(base_address, name_ptr);
    Ok(())
}

pub(crate) fn remove_library(base_address: u64) {
    let mut libraries = LIBRARIES.lock().unwrap();
    let _ = libraries.remove(&base_address);
}

#[repr(C)]
pub struct Library {
    base_address: u64,
    name: *const c_char,
}

pub type IterCallback = extern "C" fn(*const Library, usize);

#[no_mangle]
pub extern "C" fn jni_loader_iter_libs(callback: IterCallback) {
    let libs = LIBRARIES.lock().unwrap();
    let iter =
        libs.iter().map(|(&base_address, name)| Library { base_address, name: name.as_ptr() }).collect::<Vec<_>>();
    callback(iter.as_ptr(), iter.len());
}

#[no_mangle]
pub extern "C" fn jni_loader_lib_loaded(_base_address: u64, _name: *const c_char) {}
