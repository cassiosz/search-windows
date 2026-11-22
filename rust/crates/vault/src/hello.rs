//! Proving the person at the keyboard is the PC's user before a password is
//! shown, copied, exported or every one removed (Vault.swift:252 `prove`,
//! Touch ID upstream).
//!
//! Windows Hello through WinRT's `UserConsentVerifier` — face, fingerprint or
//! the Hello PIN, whatever this PC takes — asked for a window through
//! `IUserConsentVerifierInterop::RequestVerificationForWindowAsync`, so the
//! prompt belongs to the browser's window as a Win32 app's must. Only the
//! WinRT bindings for that one class are compiled in.
//!
//! Upstream answers yes when the Mac can't ask at all (no password set). Here
//! that case is an error, [`Error::NoVerifier`], and the app decides: Hello
//! not set up is not "no password", and saying yes would show passwords to
//! whoever has the unlocked PC.

use crate::{Error, Result};
use windows::core::{factory, HSTRING};
use windows::Security::Credentials::UI::{
    UserConsentVerificationResult, UserConsentVerifier, UserConsentVerifierAvailability,
};
use windows::Win32::Foundation::HWND;
use windows::Win32::System::WinRT::IUserConsentVerifierInterop;

/// Asks Windows Hello, over the window `owner` (an HWND; 0 for none), saying
/// `reason` ("show the password for example.com"). `Ok(true)`: it was the
/// user. `Ok(false)`: cancelled, refused, too many tries, or the sensor
/// busy. `Err(NoVerifier)`: Hello can't ask on this PC.
///
/// Blocks until answered: call it off the UI thread and hand the answer
/// back, or the window stops pumping messages while the prompt is up.
pub fn verify_user(owner: isize, reason: &str) -> Result<bool> {
    let os = |e: windows::core::Error| Error::Os(e.code().0);
    let available = UserConsentVerifier::CheckAvailabilityAsync().and_then(|op| op.join()).map_err(os)?;
    if available != UserConsentVerifierAvailability::Available {
        // DeviceBusy is worth trying again later, the rest is how this PC is.
        if available == UserConsentVerifierAvailability::DeviceBusy {
            return Ok(false);
        }
        return Err(Error::NoVerifier(available.0));
    }
    let message = HSTRING::from(reason);
    // Both arms are IAsyncOperation<UserConsentVerificationResult>.
    let asked = if owner != 0 {
        let interop = factory::<UserConsentVerifier, IUserConsentVerifierInterop>().map_err(os)?;
        // SAFETY: the caller's own window handle; the interface checks it.
        unsafe { interop.RequestVerificationForWindowAsync(HWND(owner as *mut _), &message) }.map_err(os)?
    } else {
        UserConsentVerifier::RequestVerificationAsync(&message).map_err(os)?
    };
    let result = asked.join().map_err(os)?;
    Ok(result == UserConsentVerificationResult::Verified)
}
