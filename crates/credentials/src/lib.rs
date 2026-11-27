//! DPAPI CurrentUser only. No portable or LocalMachine fallback, no secret Debug/Clone.
use zeroize::Zeroizing;

#[derive(Debug, thiserror::Error)]
pub enum Error {
    #[error("CREDENTIAL_UNAVAILABLE: DPAPI/profile unavailable under this Windows identity")]
    Unavailable,
    #[error("CREDENTIAL_IDENTITY_MISMATCH: re-enter secrets under the original service identity")]
    Identity,
    #[error("Windows CurrentUser DPAPI is required")]
    Unsupported,
}
pub type Result<T> = std::result::Result<T, Error>;

#[cfg(windows)]
mod native {
    use super::*;
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::{CloseHandle, LocalFree},
        Security::Cryptography::*,
        Security::{Authorization::*, *},
        System::Threading::{GetCurrentProcess, OpenProcessToken},
    };

    pub fn identity() -> Result<String> {
        unsafe {
            let mut token = null_mut();
            if OpenProcessToken(GetCurrentProcess(), TOKEN_QUERY, &mut token) == 0 {
                return Err(Error::Unavailable);
            }
            let mut size = 0;
            GetTokenInformation(token, TokenUser, null_mut(), 0, &mut size);
            // TOKEN_USER contains pointers: use an aligned backing allocation.
            let mut buffer = vec![0usize; (size as usize).div_ceil(std::mem::size_of::<usize>())];
            let ok = GetTokenInformation(
                token,
                TokenUser,
                buffer.as_mut_ptr().cast(),
                size,
                &mut size,
            );
            CloseHandle(token);
            if ok == 0 {
                return Err(Error::Unavailable);
            }
            let user = &*(buffer.as_ptr() as *const TOKEN_USER);
            let mut text = null_mut();
            if ConvertSidToStringSidW(user.User.Sid, &mut text) == 0 {
                return Err(Error::Unavailable);
            }
            let mut len = 0;
            while *text.add(len) != 0 {
                len += 1;
            }
            let sid = String::from_utf16_lossy(std::slice::from_raw_parts(text, len));
            LocalFree(text.cast());
            Ok(sid)
        }
    }
    pub fn crypt(input: &[u8], decrypt: bool) -> Result<Zeroizing<Vec<u8>>> {
        if input.len() > 1024 * 1024 {
            return Err(Error::Unavailable);
        }
        unsafe {
            let source = CRYPT_INTEGER_BLOB {
                cbData: input.len() as u32,
                pbData: input.as_ptr().cast_mut(),
            };
            let mut output = CRYPT_INTEGER_BLOB {
                cbData: 0,
                pbData: null_mut(),
            };
            let ok = if decrypt {
                CryptUnprotectData(
                    &source,
                    null_mut(),
                    null(),
                    null_mut(),
                    null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            } else {
                CryptProtectData(
                    &source,
                    null(),
                    null(),
                    null_mut(),
                    null(),
                    CRYPTPROTECT_UI_FORBIDDEN,
                    &mut output,
                )
            };
            if ok == 0 {
                return Err(Error::Unavailable);
            }
            let data = std::slice::from_raw_parts_mut(output.pbData, output.cbData as usize);
            let result = Zeroizing::new(data.to_vec());
            use zeroize::Zeroize;
            data.zeroize();
            LocalFree(output.pbData.cast());
            Ok(result)
        }
    }
}

pub fn identity() -> Result<String> {
    #[cfg(windows)]
    {
        native::identity()
    }
    #[cfg(not(windows))]
    {
        Err(Error::Unsupported)
    }
}
pub fn protect(plaintext: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    #[cfg(windows)]
    {
        native::crypt(plaintext, false)
    }
    #[cfg(not(windows))]
    {
        let _ = plaintext;
        Err(Error::Unsupported)
    }
}
pub fn unprotect(owner: &str, ciphertext: &[u8]) -> Result<Zeroizing<Vec<u8>>> {
    if identity()? != owner {
        return Err(Error::Identity);
    }
    #[cfg(windows)]
    {
        native::crypt(ciphertext, true)
    }
    #[cfg(not(windows))]
    {
        let _ = ciphertext;
        Err(Error::Unsupported)
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    #[test]
    fn current_user_roundtrip_and_fail_closed() {
        let sid = identity().unwrap();
        assert!(sid.starts_with("S-1-"));
        let blob = protect(b"synthetic-secret-only").unwrap();
        assert_ne!(blob.as_slice(), b"synthetic-secret-only");
        assert_eq!(
            unprotect(&sid, &blob).unwrap().as_slice(),
            b"synthetic-secret-only"
        );
        assert!(matches!(
            unprotect("S-1-wrong", &blob),
            Err(Error::Identity)
        ));
        assert!(unprotect(&sid, b"broken").is_err());
    }
}
