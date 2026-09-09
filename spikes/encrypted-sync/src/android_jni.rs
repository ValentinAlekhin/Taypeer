//! JVM bootstrap for synthetic Iroh processes, not an Android application service.
use jni::{
    JNIEnv,
    objects::{JClass, JObject, JObjectArray, JString},
    sys::jint,
};

#[unsafe(no_mangle)]
pub extern "system" fn Java_TaypeerNative_run(
    mut env: JNIEnv,
    _class: JClass,
    context: JObject,
    arguments: JObjectArray,
) -> jint {
    // This entry is invoked exactly once by the standalone Java process. Hold a
    // GLOBAL reference (never a JNI local reference) until process termination.
    let result = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let vm = env.get_java_vm()?;
        let context = env.new_global_ref(context)?;
        // SAFETY: the JVM and leaked global reference outlive every Iroh task.
        unsafe {
            iroh::dns::install_android_jni_context(
                vm.get_java_vm_pointer().cast(),
                context.as_obj().as_raw().cast(),
            );
        }
        std::mem::forget(context);
        let mut args = vec!["sync-node".to_string()];
        for index in 0..env.get_array_length(&arguments)? {
            let value = JString::from(env.get_object_array_element(&arguments, index)?);
            args.push(env.get_string(&value)?.into());
        }
        Ok::<_, jni::errors::Error>(crate::network::main_entry(&args))
    }));
    match result {
        Ok(Ok(code)) => code,
        _ => 1,
    }
}
