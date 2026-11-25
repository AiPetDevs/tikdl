//! Android glue over JNI: files picked in the system picker arrive as `content://` URIs, not paths;
//! sharing needs a real file, so the picked file is copied into the app's private `imports/` folder.
//! The foreground service keeps the process (and the iroh node) alive while something is shared.

use std::{fs::File, io, os::fd::FromRawFd, path::{Path, PathBuf}};

use anyhow::{anyhow, Context, Result};
use jni::{
    objects::{JObject, JValue},
    JNIEnv, JavaVM,
};

fn with_env<T>(f: impl FnOnce(&mut JNIEnv, &JObject) -> Result<T>) -> Result<T> {
    let ctx = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(ctx.vm().cast()) }?;
    let mut env = vm.attach_current_thread()?;
    let activity = unsafe { JObject::from_raw(ctx.context().cast()) };
    let r = f(&mut env, &activity);
    if env.exception_check().unwrap_or(false) {
        let _ = env.exception_describe();
        let _ = env.exception_clear();
        return Err(anyhow!("ошибка Android (подробности в журнале)"));
    }
    r
}

/// Display name and an open file for a `content://` URI.
fn open_content(uri: &str) -> Result<(String, File)> {
    with_env(|env, activity| {
        let resolver = env
            .call_method(activity, "getContentResolver", "()Landroid/content/ContentResolver;", &[])?
            .l()?;
        let juri_s = env.new_string(uri)?;
        let juri = env
            .call_static_method("android/net/Uri", "parse", "(Ljava/lang/String;)Landroid/net/Uri;", &[JValue::Object(&juri_s)])?
            .l()?;
        let null = JObject::null();
        let cursor = env
            .call_method(
                &resolver,
                "query",
                "(Landroid/net/Uri;[Ljava/lang/String;Ljava/lang/String;[Ljava/lang/String;Ljava/lang/String;)Landroid/database/Cursor;",
                &[JValue::Object(&juri), JValue::Object(&null), JValue::Object(&null), JValue::Object(&null), JValue::Object(&null)],
            )?
            .l()?;
        let mut name = String::new();
        if !cursor.is_null() {
            if env.call_method(&cursor, "moveToFirst", "()Z", &[])?.z()? {
                let col = env.new_string("_display_name")?;
                let idx = env.call_method(&cursor, "getColumnIndex", "(Ljava/lang/String;)I", &[JValue::Object(&col)])?.i()?;
                if idx >= 0 {
                    let s = env.call_method(&cursor, "getString", "(I)Ljava/lang/String;", &[JValue::Int(idx)])?.l()?;
                    if !s.is_null() {
                        name = env.get_string(&s.into())?.into();
                    }
                }
            }
            env.call_method(&cursor, "close", "()V", &[])?;
        }
        let mode = env.new_string("r")?;
        let pfd = env
            .call_method(
                &resolver,
                "openFileDescriptor",
                "(Landroid/net/Uri;Ljava/lang/String;)Landroid/os/ParcelFileDescriptor;",
                &[JValue::Object(&juri), JValue::Object(&mode)],
            )?
            .l()?;
        if pfd.is_null() {
            return Err(anyhow!("файл не открывается"));
        }
        let fd = env.call_method(&pfd, "detachFd", "()I", &[])?.i()?;
        Ok((name, unsafe { File::from_raw_fd(fd) }))
    })
}

/// Copy a picked `content://` file into `imports_dir/<unique>/<name>` and return the path to share.
pub fn import_content(uri: &str, imports_dir: &Path) -> Result<PathBuf> {
    let (name, mut src) = open_content(uri).with_context(|| format!("не удалось открыть {uri}"))?;
    let name = Path::new(&name)
        .file_name()
        .map(|n| n.to_string_lossy().to_string())
        .filter(|n| !n.is_empty() && !n.starts_with('.'))
        .unwrap_or_else(|| "file".into());
    let stamp = std::time::SystemTime::now().duration_since(std::time::UNIX_EPOCH).map(|d| d.as_nanos()).unwrap_or(0);
    let dir = imports_dir.join(format!("{stamp:x}"));
    std::fs::create_dir_all(&dir)?;
    let dest = dir.join(&name);
    let mut out = File::create(&dest)?;
    io::copy(&mut src, &mut out).context("копирование выбранного файла")?;
    Ok(dest)
}

/// Start or stop `ShareService` (a foreground service with a notification) from Kotlin side.
pub fn set_sharing(active: bool) -> Result<()> {
    with_env(|env, activity| {
        let class = env.call_method(activity, "getClassLoader", "()Ljava/lang/ClassLoader;", &[])?.l()?;
        let cname = env.new_string("app.tikdl.desktop.ShareService")?;
        let svc = env
            .call_method(&class, "loadClass", "(Ljava/lang/String;)Ljava/lang/Class;", &[JValue::Object(&cname)])?
            .l()?;
        let svc = jni::objects::JClass::from(svc);
        env.call_static_method(
            &svc,
            "setActive",
            "(Landroid/content/Context;Z)V",
            &[JValue::Object(activity), JValue::Bool(active as u8)],
        )?;
        Ok(())
    })
}
