use anyhow::Result;

#[cfg(windows)]
pub fn protect(data: &[u8], decrypt: bool) -> Result<Vec<u8>> {
    use windows_sys::Win32::Foundation::LocalFree;
    use windows_sys::Win32::Security::Cryptography::{
        CryptProtectData, CryptUnprotectData, CRYPTPROTECT_UI_FORBIDDEN, CRYPT_INTEGER_BLOB,
    };
    let input = CRYPT_INTEGER_BLOB {
        cbData: data.len().try_into()?,
        pbData: data.as_ptr() as *mut u8,
    };
    let mut output: CRYPT_INTEGER_BLOB = unsafe { std::mem::zeroed() };
    // DPAPI binds the encrypted store to the current Windows user. The returned
    // allocation belongs to Windows and must be released with LocalFree.
    let ok = unsafe {
        if decrypt {
            CryptUnprotectData(
                &input,
                std::ptr::null_mut(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        } else {
            CryptProtectData(
                &input,
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                std::ptr::null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut output,
            )
        }
    };
    if ok == 0 {
        return Err(std::io::Error::last_os_error().into());
    }
    let bytes =
        unsafe { std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec() };
    unsafe {
        LocalFree(output.pbData as *mut core::ffi::c_void);
    }
    Ok(bytes)
}

#[cfg(not(windows))]
pub fn protect(data: &[u8], _decrypt: bool) -> Result<Vec<u8>> {
    Ok(data.to_vec())
}

#[cfg(target_os = "macos")]
fn entry(email: &str) -> Result<keyring::Entry> {
    Ok(keyring::Entry::new("cc.jokerdeck.client", email)?)
}

#[cfg(target_os = "macos")]
pub fn save_password(email: &str, password: &str) -> Result<()> {
    entry(email)?.set_password(password)?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn load_password(email: &str) -> Option<String> {
    entry(email).ok()?.get_password().ok()
}

#[cfg(target_os = "macos")]
pub fn remove_password(email: &str) -> Result<()> {
    let credential = entry(email)?;
    match credential.delete_credential() {
        Ok(()) | Err(keyring::Error::NoEntry) => Ok(()),
        Err(err) => Err(err.into()),
    }
}

#[cfg(target_os = "macos")]
pub fn save_subscription(url: &str) -> Result<()> {
    entry("chatgpt-subscription")?.set_password(url)?;
    Ok(())
}

#[cfg(target_os = "macos")]
pub fn load_subscription() -> Option<String> {
    entry("chatgpt-subscription").ok()?.get_password().ok()
}

#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn dpapi_round_trip() {
        let plain = br#"{"saved_password":"not-plaintext"}"#;
        let encrypted = super::protect(plain, false).unwrap();
        assert_ne!(encrypted, plain);
        assert_eq!(super::protect(&encrypted, true).unwrap(), plain);
        assert!(super::protect(b"invalid", true).is_err());
    }
}
