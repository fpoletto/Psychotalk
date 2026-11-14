// jni 0.22.4's generated wrappers use an atomic API renamed in Rust 1.100.
#![allow(deprecated)]
use crate::session::{Session, Shared};
use jni::{
    Env, native_method,
    objects::{JByteArray, JObject, JString},
    sys::jboolean,
};
use std::{
    path::Path,
    sync::{Arc, Mutex, OnceLock},
};
use zeroize::{Zeroize, Zeroizing};
static SESSION: OnceLock<Mutex<Option<Session>>> = OnceLock::new();
fn session() -> &'static Mutex<Option<Session>> {
    SESSION.get_or_init(|| Mutex::new(None))
}
fn shared() -> Option<Arc<Shared>> {
    session().lock().ok()?.as_ref().map(|s| s.shared.clone())
}

const _: jni::NativeMethod = native_method! {java_type="app.psychotalk.Native",extern fn native_initialize(directory:JString,key:[jbyte])->jboolean,};
fn native_initialize<'a>(
    env: &mut Env<'a>,
    _: JObject<'a>,
    directory: JString<'a>,
    key: JByteArray<'a>,
) -> jni::errors::Result<jboolean> {
    let key = Zeroizing::new(env.convert_byte_array(&key)?);
    let path = std::path::PathBuf::from(directory.to_string());
    Ok(std::fs::create_dir_all(&path).is_ok()
        && crate::vault::Vault::open(&path.join("vault.db"), &key, true).is_ok())
}

const _: jni::NativeMethod = native_method! {java_type="app.psychotalk.Native",extern fn native_derive(password:[jbyte],salt:[jbyte])->[jbyte],};
fn native_derive<'a>(
    env: &mut Env<'a>,
    _: JObject<'a>,
    password: JByteArray<'a>,
    salt: JByteArray<'a>,
) -> jni::errors::Result<JByteArray<'a>> {
    let p = Zeroizing::new(env.convert_byte_array(&password)?);
    let salt = env.convert_byte_array(&salt)?;
    match crate::vault::derive(&p, &salt) {
        Ok(key) => env.byte_array_from_slice(key.as_ref()),
        Err(_) => env.byte_array_from_slice(&[]),
    }
}
const _: jni::NativeMethod = native_method! {java_type="app.psychotalk.Native",extern fn native_open(directory:JString,binary:JString,key:[jbyte],create:jboolean)->jboolean,};
fn native_open<'a>(
    env: &mut Env<'a>,
    _: JObject<'a>,
    directory: JString<'a>,
    binary: JString<'a>,
    key: JByteArray<'a>,
    create: jboolean,
) -> jni::errors::Result<jboolean> {
    let key = Zeroizing::new(env.convert_byte_array(&key)?);
    let mut slot = session().lock().unwrap();
    if slot.is_some() {
        return Ok(false);
    }
    match Session::open(
        Path::new(&directory.to_string()),
        Path::new(&binary.to_string()),
        &key,
        create,
    ) {
        Ok(s) => {
            *slot = Some(s);
            Ok(true)
        }
        Err(_) => Ok(false),
    }
}
const _: jni::NativeMethod = native_method! {java_type="app.psychotalk.Native",extern fn native_command(input:JString)->JString,};
fn native_command<'a>(
    env: &mut Env<'a>,
    _: JObject<'a>,
    input: JString<'a>,
) -> jni::errors::Result<JString<'a>> {
    let request = Zeroizing::new(input.to_string());
    let result = shared().and_then(|s| s.command(&request).ok());
    let output = Zeroizing::new(
        result
            .map(|mut v| {
                let output = v.to_string();
                if let Some(messages) = v["view"]["active"]["messages"].as_array_mut() {
                    for message in messages {
                        if let Some(serde_json::Value::String(text)) = message.get_mut("text") {
                            text.zeroize();
                        }
                    }
                }
                output
            })
            .unwrap_or_else(|| "{\"error\":true}".into()),
    );
    env.new_string(output.as_str())
}
const _: jni::NativeMethod =
    native_method! {java_type="app.psychotalk.Native",extern fn native_heartbeat(),};
fn native_heartbeat<'a>(_: &mut Env<'a>, _: JObject<'a>) -> jni::errors::Result<()> {
    if let Some(s) = shared() {
        s.heartbeat();
    }
    Ok(())
}
const _: jni::NativeMethod =
    native_method! {java_type="app.psychotalk.Native",extern fn native_revoke(),};
fn native_revoke<'a>(_: &mut Env<'a>, _: JObject<'a>) -> jni::errors::Result<()> {
    if let Some(s) = shared() {
        s.revoke();
    }
    Ok(())
}
const _: jni::NativeMethod =
    native_method! {java_type="app.psychotalk.Native",extern fn native_close(),};
fn native_close<'a>(_: &mut Env<'a>, _: JObject<'a>) -> jni::errors::Result<()> {
    let old = session().lock().unwrap().take();
    if let Some(s) = old {
        s.close();
    }
    Ok(())
}
