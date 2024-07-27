const jni_loader_iter_libs_exp = Process.mainModule.enumerateExports().find(exp => exp.name === "jni_loader_iter_libs");
if (jni_loader_iter_libs_exp !== undefined) {
    const jni_loader_iter_libs = new NativeFunction(jni_loader_iter_libs_exp.address, "void", ["pointer"]);
    globalThis.getMappedLibs = function () {
        let modules = [];
        const callback = new NativeCallback((libsPtr, libsLen) => {
            // 0: base_address (u64)
            // 8: name (const char*)
            for (let i = 0; i < libsLen; i++) {
                const base = libsPtr.add(i * 16).readU64();
                const name = libsPtr.add(i * 16 + 8).readPointer().readCString() || "";
                modules.push({ base, name });
            }
        }, "void", ["pointer", "size_t"]);
        jni_loader_iter_libs(callback);
        return modules;
    }
} else {
    console.error("Failed to find jni_loader_iter_libs");
}

const jni_loader_lib_loaded_exp = Process.mainModule.enumerateExports().find(exp => exp.name === "jni_loader_lib_loaded");
if (jni_loader_lib_loaded_exp !== undefined) {
    Interceptor.attach(jni_loader_lib_loaded_exp.address, {
        onEnter: function (args) {
            const base = args[0];
            const name = args[1].readCString() || "";
            if (globalThis.onMappedLibLoad) {
                globalThis.onMappedLibLoad(base, name);
            }
        }
    });
}
