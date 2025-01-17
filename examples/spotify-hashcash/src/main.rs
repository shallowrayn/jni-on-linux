use std::path::PathBuf;

use anyhow::Result;
use jni_loader::JNI;
#[cfg(feature = "qbdi")]
use qbdi_sys2::{
    qbdi_addCodeCB, qbdi_addInstrumentedRange, qbdi_alignedFree, qbdi_allocateVirtualStack, qbdi_call,
    qbdi_getGPRState, qbdi_getInstAnalysis, qbdi_initVM, qbdi_terminateVM, AnalysisType_QBDI_ANALYSIS_DISASSEMBLY,
    AnalysisType_QBDI_ANALYSIS_INSTRUCTION, FPRState, GPRState, InstPosition_QBDI_PREINST, VMAction,
    VMAction_QBDI_CONTINUE, VMInstanceRef,
};

#[repr(C)]
struct Array {
    start: *mut u8,
    end: *mut u8,
}
impl Array {
    pub fn new(input: Vec<u8>) -> Self {
        let mut data = Vec::with_capacity(input.len());
        data.extend(input);
        let length = data.len();
        let start = data.as_mut_ptr();
        let end = unsafe { start.add(length) };
        std::mem::forget(data);
        Self { start, end }
    }
}
impl Drop for Array {
    fn drop(&mut self) {
        let length = (self.end as usize) - (self.start as usize);
        let _ = unsafe { Vec::from_raw_parts(self.start, length, length) };
    }
}

type PowSolveHashcash = extern "C" fn(
    client_hello: *const Array,
    ap_response: *const Array,
    prefix: *const u8,
    prefix_len: u32,
    length: u8,
    target: u32,
    suffix: *mut u8,
) -> u32;

#[cfg(feature = "qbdi")]
unsafe extern "C" fn qbdi_instruction_callback(
    vm: VMInstanceRef, _gpr_state: *mut GPRState, _fpr_state: *mut FPRState, _data: *mut std::os::raw::c_void,
) -> VMAction {
    let instruction =
        qbdi_getInstAnalysis(vm, AnalysisType_QBDI_ANALYSIS_INSTRUCTION | AnalysisType_QBDI_ANALYSIS_DISASSEMBLY);
    let address = (*instruction).address;
    let dissassembly = std::ffi::CStr::from_ptr((*instruction).disassembly);
    println!("[{address:#018x}] {}", dissassembly.to_string_lossy());
    VMAction_QBDI_CONTINUE
}

fn solve_hashcash_wrapper(
    lib: &JNI, client_hello: &Array, ap_response: &Array, prefix: &[u8], length: u8, target: u32,
) -> Vec<u8> {
    #[cfg(target_arch = "x86_64")]
    let pow_solve_hashcash: PowSolveHashcash = unsafe { std::mem::transmute(lib.get_offset(0x000000000122609c)) };
    #[cfg(target_arch = "aarch64")]
    let pow_solve_hashcash: PowSolveHashcash = unsafe { std::mem::transmute(lib.get_offset(0x00000000010D5EAC)) };

    let mut suffix = vec![0; 16];
    let suffix_len = {
        #[cfg(not(feature = "qbdi"))]
        {
            pow_solve_hashcash(
                client_hello,
                ap_response,
                prefix.as_ptr(),
                prefix.len() as u32,
                length,
                target,
                suffix.as_mut_ptr(),
            ) as usize
        }
        #[cfg(feature = "qbdi")]
        #[allow(clippy::fn_to_numeric_cast)]
        {
            unsafe {
                let mut vm: VMInstanceRef = std::ptr::null_mut();
                qbdi_initVM(&mut vm, std::ptr::null(), std::ptr::null_mut(), 0);
                let vm_state = qbdi_getGPRState(vm);
                let mut fakestack = std::ptr::null_mut();
                qbdi_allocateVirtualStack(vm_state, 0x100000, &mut fakestack);
                qbdi_addCodeCB(vm, InstPosition_QBDI_PREINST, Some(qbdi_instruction_callback), std::ptr::null_mut(), 0);
                let lib_start = lib.mapping.base as u64;
                let lib_end = lib_start + lib.mapping.size as u64;
                qbdi_addInstrumentedRange(vm, lib_start, lib_end);
                let mut suffix_len = 0;
                qbdi_call(
                    vm,
                    &mut suffix_len,
                    pow_solve_hashcash as u64,
                    7,
                    client_hello,
                    ap_response,
                    prefix.as_ptr(),
                    prefix.len() as u32,
                    length as u32,
                    target,
                    suffix.as_mut_ptr(),
                );
                qbdi_alignedFree(fakestack as *mut std::os::raw::c_void);
                qbdi_terminateVM(vm);
                suffix_len as usize
            }
        }
    };
    suffix.resize(suffix_len, 0);
    suffix
}

fn main() -> Result<()> {
    env_logger::init();
    #[cfg(target_arch = "x86_64")]
    // Download from https://drive.proton.me/urls/RJX1MDD6KG#VrZCZk72EmBL
    let file_path = PathBuf::from("./liborbit-jni-spotify-8.8.96-x86_64.so");
    #[cfg(target_arch = "aarch64")]
    // Download from https://drive.proton.me/urls/04RQ1T7VZ8#MSXjs92nMK78
    let file_path = PathBuf::from("./liborbit-jni-spotify-8.8.96-arm64.so");
    let mut lib = JNI::new(file_path).expect("Failed to load Spotify JNI");
    lib.add_dependency("liblog.so", None);
    lib.add_dependency("libOpenSLES.so", None);
    lib.add_dependency("libmediandk.so", None);
    lib.add_dependency("libm.so", None);
    lib.add_dependency("libandroid.so", None);
    lib.add_dependency("libdl.so", None);
    lib.add_dependency("libc.so", None);
    lib.load_dependencies()?;
    lib.override_symbol("memset", Some(libc::memset as *const ()));
    lib.override_symbol("memcpy", Some(libc::memcpy as *const ()));
    lib.initialize()?;

    let client_hello = Array::new(hex::decode("000400000167520f5002a00100f00106c002ecf298a803a00100f00100c0020092036752655260e09175a90dc3588406eca23640be7a7f8903b044828b4a819b42305fb66fc00ce02316f6da94b02da4745b942252dac629f6953f215b88b232f4e97e133e091ca41300bfc877a192e5140e382b74804771291b565297babe399121481dce0419a00101e20310a58cdb3e823431a92a306e2e6927b7a9b204bb01d7a03e5e7b604ba1158265fe6a5e3a3e948b1e3574ad9927ad2846afa55f8f833ff04574de802dc5d9853eba7684eda2a92e4799252147d849d01cc95b41e0817c3a896a9f643bf2a211a150df43be862b9221be3e43113c87c0b2cba0550883e6ed110e71147bc53696e307a6ad9820049b95fc30e50a3385fd6e0f74492979121be7851347e1fd6bf04ef5f573be6f660e766c3506c5155976dc0fb30a0bc144848803719626170d930865c273f2ff970fcbb2f16516cd577ffb82050808011a002a020800")?);
    let ap_response = Array::new(hex::decode("000002ae52a70552ec0252e9025260502cd2f60babde23f07f48f511ed2e4950fb4bc156c74ff09e0c2f5c79c9e3d2636c71bdc085a781c0c780b79f17442c30e4db543ef1f641f205a4c15de21d9566d16098bce24de0349deeab12f4096191217189923f06132185e15b5f3f1878a00100f201800280ee2632129ef3d2e1a365551fa0456db298a0f8451720ab4cc8a58837ed367f1ef59bf55286a3afe97c5b9075aa1b3fe34f693b4f41ad5b7d939b5e78a2f0c1ba43180e34bf2c05a037f22b422055040241ed8aed27f9b48c4430c00d31ae3af3edc2f859805624ccda4bb2c392216b05d74ebfc86141a47bcebea8c6525be23c0f023f7e4cb271144255718af417837e06a73cfcd8aa3a34bbb68d2a78e3e2f4d7b1aa3f2e54a1d5fc702f55b2b07755cfbf6abb33574b6cdbc80755ee8a8f7a44daf6756e9389f5f2d9b665b7855e5d0611e987990d71fc82ec4154b6ad26b813409c3e2a2a27947870d0c94de688b3cb2c7966224aecf80c9f4237c024a6a20114521252105af68faf96c89be641b4e49927687cc2f2011b5219521028d5c0534cd34ed3adf5fc9cfb6c3f92a0010ef001bb22c2020252009203104e775d56ea62a054cad5b5f726ee29b9e203e701cd61f26d997da3a60c6e46609470d013c6c9aeaf44254e1f3a4fb7ed2abc53e59d8da028b796ce7525966c32573c9d22b7b32fd21d186af156d301e5da802f358ee53fea872802b4777ad63adc85893b63c8f6d1f9366bc0c0e8ff09da6ee51bd1c5ee00ffac6647f71a53b9e64b81cec2a8a8bc9c7804623c5de9c4ac4b88b32f9648eb24e759737e2fd8f22e7ccca712122bf3a8cc0f1d8a6864e9c022d096a0620abaa9c15f0a1f8346a5fa03e75bd2350a7e8d27667302661ad7d3ba67ecc58e1becd2535cc71f0e0b8f66462aa9f82258aa768f8f237cf9ae0d6e1a95bba3e7ce1cdbcae1")?);
    let prefix = hex::decode("28d5c0534cd34ed3adf5fc9cfb6c3f92")?;
    let length = 14;
    let target = 4411;
    let target_suffix = hex::decode("0555aff2840bee9600000000000049d5")?;

    let suffix = solve_hashcash_wrapper(&lib, &client_hello, &ap_response, &prefix, length, target);

    println!("Suffix len: {}", suffix.len());
    println!("Suffix: {}", hex::encode(&suffix));
    println!("Target: {}", hex::encode(&target_suffix));

    assert_eq!(suffix.len(), 16);
    assert_eq!(suffix, target_suffix);
    Ok(())
}
