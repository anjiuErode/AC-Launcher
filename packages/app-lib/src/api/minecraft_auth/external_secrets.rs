#[cfg(windows)]
pub(super) fn protect(bytes: &[u8]) -> crate::Result<String> {
	use base64::{Engine, engine::general_purpose::STANDARD};
	Ok(STANDARD.encode(dpapi(bytes, true)?))
}

#[cfg(windows)]
pub(super) fn unprotect(value: &str) -> crate::Result<Vec<u8>> {
	use base64::{Engine, engine::general_purpose::STANDARD};
	dpapi(&STANDARD.decode(value)?, false)
}

#[cfg(windows)]
fn dpapi(bytes: &[u8], protect: bool) -> crate::Result<Vec<u8>> {
	use windows::Win32::Foundation::{HLOCAL, LocalFree};
	use windows::Win32::Security::Cryptography::{
		CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData,
		CryptUnprotectData,
	};
	use windows::core::PCWSTR;

	let input = CRYPT_INTEGER_BLOB {
		cbData: bytes.len().try_into()?,
		pbData: bytes.as_ptr().cast_mut(),
	};
	let mut output = CRYPT_INTEGER_BLOB::default();
	// DPAPI borrows input for this call and allocates output with LocalAlloc.
	unsafe {
		if protect {
			CryptProtectData(
				&input, PCWSTR::null(), None, None, None,
				CRYPTPROTECT_UI_FORBIDDEN, &mut output,
			)?;
		} else {
			CryptUnprotectData(
				&input, None, None, None, None, CRYPTPROTECT_UI_FORBIDDEN,
				&mut output,
			)?;
		}
		let result = if output.cbData == 0 {
			Vec::new()
		} else {
			std::slice::from_raw_parts(output.pbData, output.cbData as usize).to_vec()
		};
		let _ = LocalFree(Some(HLOCAL(output.pbData.cast())));
		Ok(result)
	}
}

#[cfg(all(windows, test))]
mod tests {
	use super::{protect, unprotect};

	#[test]
	fn dpapi_round_trip_protects_synthetic_tokens() -> crate::Result<()> {
		let token = "synthetic-external-account-access-token";
		let ciphertext = protect(token.as_bytes())?;
		assert!(!ciphertext.contains(token));
		assert_eq!(unprotect(&ciphertext)?, token.as_bytes());
		assert!(unprotect("YmFkLXRva2Vu").is_err());
		Ok(())
	}
}
