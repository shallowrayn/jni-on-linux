use std::arch::asm;

#[no_mangle]
#[naked]
pub unsafe extern "C" fn jni_dlfcn_trampoline() {
    asm!(
        ".quad 0x0102030405060708", // JNI*
        ".quad 0x0807060504030201", // Function pointer
        "mov r8, [rip-23]",
        "mov r9, [rip-22]",
        "mov rdx, rsi", // Move args[1] to args[2]
        "mov rsi, rdi", // Move args[0] to args[1]
        "mov rdi, r8",  // Move JNI* into args[0]
        "jmp r9",
        options(noreturn)
    )
}

pub const TRAMPOLINE_SIZE: usize = 64;
pub const JNI_OFFSET: usize = 0;
pub const FN_OFFSET: usize = 8;
pub const CODE_OFFSET: usize = 16;
