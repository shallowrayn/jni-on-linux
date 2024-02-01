use std::arch::asm;

const SAVED_REGISTERS_SPACE: usize = 10 * 8;
const RELOC_OFFSET: usize = SAVED_REGISTERS_SPACE + 0; // [sp + 0]
const SAVED_X0_X1_OFFSET: usize = 0 * 16;
const SAVED_X2_X3_OFFSET: usize = 1 * 16;
const SAVED_X4_X5_OFFSET: usize = 2 * 16;
const SAVED_X6_X7_OFFSET: usize = 3 * 16;
const SAVED_X8_X9_OFFSET: usize = 4 * 16;

#[naked]
pub unsafe extern "C" fn plt_callback_trampoline() {
    // x16 contains &.got.plt[2]
    // x17 contains &plt_callback_trampoline
    // [sp + 0] contains Address of square reloc
    // [sp + 8] contains link register for original call

    asm!(
        // Save registers
        "sub sp, sp, {SAVED_REGISTERS_SPACE}",
        "stp x0, x1, [sp, {SAVED_X0_X1_OFFSET}]",
        "stp x2, x3, [sp, {SAVED_X2_X3_OFFSET}]",
        "stp x4, x5, [sp, {SAVED_X4_X5_OFFSET}]",
        "stp x6, x7, [sp, {SAVED_X6_X7_OFFSET}]",
        "stp x8, x9, [sp, {SAVED_X8_X9_OFFSET}]",
        // Call plt callback
        "ldr x3, [sp, {RELOC_OFFSET}]",
        "sub x1, x3, x16",    // .got.plt[n] - .got.plt[2]
        "sub x1, x1, 8",      // Should be relative to .got.plt[3] (first reloc) not .got.plt[2]
        "asr x1, x1, 3",      // Divide by 8 to get the index
        "ldr x2, [x16, -8]",  // Get value of .got.plt[1] (PltData*)
        "ldr x0, [x2, 0]",    // PltData->jni
        "ldr x17, [x2, 8]",   // PltData->plt_callback
        "blr x17",            // Call plt_callback
        "mov x17, x0",        // Save the resolved address
        // Return registers
        "ldp x0, x1, [sp, {SAVED_X0_X1_OFFSET}]",
        "ldp x2, x3, [sp, {SAVED_X2_X3_OFFSET}]",
        "ldp x4, x5, [sp, {SAVED_X4_X5_OFFSET}]",
        "ldp x6, x7, [sp, {SAVED_X6_X7_OFFSET}]",
        "ldp x8, x9, [sp, {SAVED_X8_X9_OFFSET}]",
        "add sp, sp, {SAVED_REGISTERS_SPACE}",
        // Restore link register
        "ldr x30, [sp, 8]",
        // Remove relocation address and link register, added by PLT handler
        "add sp, sp, 16",
        // Jump to the resolved address
        "br x17",

        SAVED_REGISTERS_SPACE = const SAVED_REGISTERS_SPACE,
        RELOC_OFFSET = const RELOC_OFFSET,
        SAVED_X0_X1_OFFSET = const SAVED_X0_X1_OFFSET,
        SAVED_X2_X3_OFFSET = const SAVED_X2_X3_OFFSET,
        SAVED_X4_X5_OFFSET = const SAVED_X4_X5_OFFSET,
        SAVED_X6_X7_OFFSET = const SAVED_X6_X7_OFFSET,
        SAVED_X8_X9_OFFSET = const SAVED_X8_X9_OFFSET,
        options(noreturn)
    )
}
