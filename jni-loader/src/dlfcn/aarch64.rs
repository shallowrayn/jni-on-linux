use std::arch::asm;

#[no_mangle]
#[naked]
pub unsafe extern "C" fn jni_dlfcn_trampoline() {
    asm!(
        ".quad 0x0102030405060708", // JNI*
        ".quad 0x0807060504030201", // Function pointer
        "ldr x7, -16",              // Get JNI*
        "ldr x8, -12",              // Get function pointer
        "mov x2, x1",               // Move args[1] to args[2]
        "mov x1, x0",               // Move args[0] to args[1]
        "mov x0, x7",               // Move JNI* into args[0]
        "br x8",
        options(noreturn)
    )
}

pub const TRAMPOLINE_SIZE: usize = 64;
pub const JNI_OFFSET: usize = 0;
pub const FN_OFFSET: usize = 8;
pub const CODE_OFFSET: usize = 16;
