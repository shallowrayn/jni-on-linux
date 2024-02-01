#![cfg_attr(feature = "inline-asm", feature(naked_functions))]
#![cfg_attr(feature = "inline-asm", feature(asm_const))]

mod jni;
pub use jni::{Error, JNI};
pub mod locate;
mod mmap;
mod plt;
