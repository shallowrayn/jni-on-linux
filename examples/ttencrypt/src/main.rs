#![feature(vec_into_raw_parts)]
use std::env;

use anyhow::Result;
use jni::{objects::JByteArray, InitArgsBuilder, JavaVM};
use jni_loader::JNI;
use nix::libc;

// Class:     com_bytedance_frameworks_encryptor_EncryptorUtil
// Method:    ttEncrypt
// Signature: ([BI)[B
type TTEncrypt = extern "C" fn(
    env: *mut jni::sys::JNIEnv,
    class: jni::sys::jclass,
    input: jni::sys::jbyteArray,
    length: jni::sys::jint,
) -> jni::sys::jbyteArray;

fn vec_i8_to_vec_u8(input: Vec<i8>) -> Vec<u8> {
    let mut input = std::mem::ManuallyDrop::new(input);
    let ptr = input.as_mut_ptr();
    let len = input.len();
    let capacity = input.capacity();
    unsafe { Vec::from_raw_parts(ptr as *mut u8, len, capacity) }
}

fn main() -> Result<()> {
    env_logger::init();

    let jvm_args = InitArgsBuilder::new().build()?;
    let jvm = JavaVM::new(jvm_args)?;
    let env = jvm.attach_current_thread()?;
    let data = hex::decode("000102030405060708090a0b0c0d0e").unwrap();
    let data_arr = env.byte_array_from_slice(&data)?;

    // https://drive.proton.me/urls/YM24QKY1B4#szA57GF5lUZd
    let lib_path = env::current_dir().unwrap().join("libEncryptor.so");
    let mut lib = JNI::new(lib_path)?;
    lib.add_dependency("liblog.so", None);
    lib.add_dependency("libc.so", None);
    lib.add_dependency("libm.so", None);
    lib.add_dependency("libstdc++.so", None);
    lib.add_dependency("libdl.so", None);
    lib.load_dependencies().unwrap();
    lib.override_symbol("srand", Some(libc::srand as *const ()));
    lib.override_symbol("time", Some(libc::time as *const ()));
    lib.override_symbol("rand", Some(libc::rand as *const ()));
    lib.override_symbol("malloc", Some(libc::malloc as *const ()));
    lib.override_symbol("memcpy", Some(libc::memcpy as *const ()));
    lib.override_symbol("memset", Some(libc::memset as *const ()));
    lib.override_symbol("free", Some(libc::free as *const ()));
    lib.initialize();

    let tt_encrypt: TTEncrypt = unsafe { std::mem::transmute(lib.get_offset(0x7d8c)) };
    let encrypted = tt_encrypt(env.get_raw(), std::ptr::null_mut(), data_arr.as_raw(), data.len() as i32);
    let encrypted = unsafe { JByteArray::from_raw(encrypted) };
    let encrypted_len = env.get_array_length(&encrypted)?;
    let mut encrypted_bytes = vec![0; encrypted_len as usize];
    env.get_byte_array_region(&encrypted, 0, &mut encrypted_bytes)?;
    let encrypted_bytes = vec_i8_to_vec_u8(encrypted_bytes);
    println!("Plaintext:  {}", hex::encode(data));
    println!("Ciphertext: {}", hex::encode(encrypted_bytes));

    Ok(())
}
