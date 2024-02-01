use std::arch::asm;

const SAVED_RAX_IDX: usize = 0 * 8;
const SAVED_RCX_IDX: usize = 1 * 8;
const SAVED_RDX_IDX: usize = 2 * 8;
const SAVED_RSI_IDX: usize = 3 * 8;
const SAVED_RDI_IDX: usize = 4 * 8;
const SAVED_R8_IDX: usize = 5 * 8;
const SAVED_R9_IDX: usize = 6 * 8;
const SAVED_REGISTERS_SPACE: usize = 8 * 8;

#[naked]
pub unsafe extern "C" fn plt_callback_trampoline() {
    asm!(
        "push rbp",
        "mov rbp, rsp",
        "mov r10, [rsp + 0x08]", // PltData*
        "mov r11, [rsp + 0x10]", // Symbol index
        // Save registers
        "sub rsp, {SAVED_REGISTERS_SPACE}",
        "mov [rsp + {SAVED_RAX_IDX}], rax",
        "mov [rsp + {SAVED_RCX_IDX}], rcx",
        "mov [rsp + {SAVED_RDX_IDX}], rdx",
        "mov [rsp + {SAVED_RSI_IDX}], rsi",
        "mov [rsp + {SAVED_RDI_IDX}], rdi",
        "mov [rsp + {SAVED_R8_IDX}], r8",
        "mov [rsp + {SAVED_R9_IDX}], r9",
        // Call plt callback
        "mov rdi, [r10 + 0x0]", // PltData->jni
        "mov rsi, r11",         // Symbol index
        "call [r10 + 0x8]",     // Call plt_callback
        "mov r10, rax",         // Save the resolved address
        // Restore registers
        "mov rax, [rsp + {SAVED_RAX_IDX}]",
        "mov rcx, [rsp + {SAVED_RCX_IDX}]",
        "mov rdx, [rsp + {SAVED_RDX_IDX}]",
        "mov rsi, [rsp + {SAVED_RSI_IDX}]",
        "mov rdi, [rsp + {SAVED_RDI_IDX}]",
        "mov r8, [rsp + {SAVED_R8_IDX}]",
        "mov r9, [rsp + {SAVED_R9_IDX}]",
        // Restore stack
        "mov rsp, rbp",
        "pop rbp",
        "add rsp, 0x10", // Remove the symbol index and PltData*
        // Jump to the resolved address
        "jmp r10",

        SAVED_REGISTERS_SPACE = const SAVED_REGISTERS_SPACE,
        SAVED_RAX_IDX = const SAVED_RAX_IDX,
        SAVED_RCX_IDX = const SAVED_RCX_IDX,
        SAVED_RDX_IDX = const SAVED_RDX_IDX,
        SAVED_RSI_IDX = const SAVED_RSI_IDX,
        SAVED_RDI_IDX = const SAVED_RDI_IDX,
        SAVED_R8_IDX = const SAVED_R8_IDX,
        SAVED_R9_IDX = const SAVED_R9_IDX,
        options(noreturn)
    )
}
