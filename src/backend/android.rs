//! Android backend: calls the Kotlin `com.dga.bevy.iap.DgaIap` bridge over JNI.
//!
//! The bridge class ships in this crate's `android/` folder and must be compiled into the app.
//! It is loaded through the activity's class loader, because JNI's `FindClass` on a native
//! thread only sees system classes.

use std::ffi::c_void;

use jni::objects::{GlobalRef, JClass, JObject, JString, JValue};
use jni::{JNIEnv, JavaVM, NativeMethod};

use super::Backend;
use crate::bridge::{self, Command};

const BRIDGE_CLASS: &str = "com.dga.bevy.iap.DgaIap";

pub(crate) struct AndroidBackend {
    vm: JavaVM,
    class: GlobalRef,
}

impl AndroidBackend {
    pub(crate) fn new() -> Result<Self, String> {
        let context = ndk_context::android_context();
        // SAFETY: ndk-context is initialised by android-activity with the app's JavaVM and Activity.
        let vm = unsafe { JavaVM::from_raw(context.vm().cast()) }.map_err(|e| e.to_string())?;
        let activity = unsafe { JObject::from_raw(context.context().cast()) };
        let mut env = vm
            .attach_current_thread_permanently()
            .map_err(|e| e.to_string())?;

        let class = load_bridge_class(&mut env, &activity).map_err(|e| {
            let _ = env.exception_clear();
            format!("Kotlin bridge {BRIDGE_CLASS} not found ({e}); add android/ to the app's sources")
        })?;
        let class = env.new_global_ref(class).map_err(|e| e.to_string())?;

        let jclass: &JClass = class.as_obj().into();
        env.register_native_methods(
            jclass,
            &[NativeMethod {
                name: "nativeOnMessage".into(),
                sig: "(Ljava/lang/String;)V".into(),
                fn_ptr: native_on_message as *mut c_void,
            }],
        )
        .map_err(|e| e.to_string())?;
        env.call_static_method(
            jclass,
            "init",
            "(Landroid/app/Activity;)V",
            &[JValue::Object(&activity)],
        )
        .map_err(|e| e.to_string())?;

        Ok(Self { vm, class })
    }
}

fn load_bridge_class<'local>(
    env: &mut JNIEnv<'local>,
    activity: &JObject,
) -> jni::errors::Result<JObject<'local>> {
    let loader = env
        .call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?
        .l()?;
    let name = env.new_string(BRIDGE_CLASS)?;
    env.call_method(
        &loader,
        "loadClass",
        "(Ljava/lang/String;)Ljava/lang/Class;",
        &[JValue::Object(&name)],
    )?
    .l()
}

impl Backend for AndroidBackend {
    fn send(&self, command: Command) {
        let json = match serde_json::to_string(&command) {
            Ok(json) => json,
            Err(err) => return tracing::error!("dga_bevy_iap: can't encode {command:?}: {err}"),
        };
        let result = self.vm.attach_current_thread_permanently().and_then(|mut env| {
            let json = env.new_string(json)?;
            let jclass: &JClass = self.class.as_obj().into();
            let result = env.call_static_method(
                jclass,
                "command",
                "(Ljava/lang/String;)V",
                &[JValue::Object(&json)],
            );
            if result.is_err() {
                let _ = env.exception_describe();
                let _ = env.exception_clear();
            }
            result.map(|_| ())
        });
        if let Err(err) = result {
            tracing::error!("dga_bevy_iap: Kotlin bridge call failed: {err}");
        }
    }
}

extern "system" fn native_on_message(mut env: JNIEnv, _class: JClass, json: JString) {
    match env.get_string(&json) {
        Ok(json) => bridge::post_native_json(&String::from(json)),
        Err(err) => tracing::error!("dga_bevy_iap: unreadable string from Kotlin bridge: {err}"),
    }
}
