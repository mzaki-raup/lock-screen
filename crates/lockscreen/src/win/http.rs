//! The small HTTPS client the update check needs: one GET, into memory, capped.
//!
//! WinHTTP rather than an HTTP crate, which is the house pattern and also the
//! right call here: TLS and certificate validation come from Schannel and the
//! system certificate store, so there is no second trust root to keep current,
//! and it adds no dependencies to a security tool.
//!
//! **HTTPS only.** `WINHTTP_FLAG_SECURE` is always set and a non-HTTPS URL is
//! refused outright. An update must not arrive over a connection anyone on the
//! path can rewrite -- and note what that does *not* buy: the checksum travels
//! the same channel as the binary, so this protects against a network attacker
//! and not against the forge itself. Only Authenticode covers that.

use windows::Win32::Networking::WinHttp::{
    URL_COMPONENTS, WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY, WINHTTP_FLAG_SECURE, WINHTTP_INTERNET_SCHEME_HTTPS,
    WINHTTP_QUERY_FLAG_NUMBER, WINHTTP_QUERY_STATUS_CODE, WinHttpCloseHandle, WinHttpConnect, WinHttpCrackUrl,
    WinHttpOpen, WinHttpOpenRequest, WinHttpQueryHeaders, WinHttpReadData, WinHttpReceiveResponse, WinHttpSendRequest,
    WinHttpSetTimeouts,
};
use windows::core::PCWSTR;

use super::ui::wide;

/// Closes a WinHTTP handle however the function returns.
struct Handle(*mut core::ffi::c_void);

impl Drop for Handle {
    fn drop(&mut self) {
        if !self.0.is_null() {
            // SAFETY: a handle from one of the WinHttp open calls, closed once.
            unsafe {
                let _ = WinHttpCloseHandle(self.0);
            }
        }
    }
}

/// Fetch `url`, giving up after `limit` bytes.
///
/// The cap is not politeness: without it a forge that answered with an endless
/// stream would be answered by filling memory.
pub fn get(url: &str, limit: usize) -> Result<Vec<u8>, String> {
    let (host, port, path) = split(url)?;
    let agent = wide(&format!("LockScreen/{}", lockscreen_core::VERSION));
    let host_w = wide(&host);
    let path_w = wide(&path);

    // SAFETY: every pointer below addresses a local that outlives its call, and
    // each handle is closed by `Handle::drop` on every path out.
    unsafe {
        let session = Handle(WinHttpOpen(
            PCWSTR(agent.as_ptr()),
            WINHTTP_ACCESS_TYPE_AUTOMATIC_PROXY,
            PCWSTR::null(),
            PCWSTR::null(),
            0,
        ));
        if session.0.is_null() {
            return Err("no connection could be opened".to_owned());
        }
        // Nothing here is worth waiting on for long: the check is on demand and
        // the download can be tried again.
        let _ = WinHttpSetTimeouts(session.0, 10_000, 15_000, 30_000, 60_000);

        let connect = Handle(WinHttpConnect(session.0, PCWSTR(host_w.as_ptr()), port, 0));
        if connect.0.is_null() {
            return Err(format!("{host} couldn't be reached"));
        }

        let verb = wide("GET");
        let request = Handle(WinHttpOpenRequest(
            connect.0,
            PCWSTR(verb.as_ptr()),
            PCWSTR(path_w.as_ptr()),
            PCWSTR::null(),
            PCWSTR::null(),
            std::ptr::null(),
            WINHTTP_FLAG_SECURE,
        ));
        if request.0.is_null() {
            return Err("the request couldn't be made".to_owned());
        }

        let accept = wide("Accept: application/json, application/octet-stream\r\n");
        WinHttpSendRequest(request.0, Some(&accept[..accept.len() - 1]), None, 0, 0, 0)
            .map_err(|e| format!("{host} didn't answer ({})", e.message()))?;
        WinHttpReceiveResponse(request.0, std::ptr::null_mut())
            .map_err(|e| format!("{host} didn't answer ({})", e.message()))?;

        let mut status: u32 = 0;
        let mut len = std::mem::size_of::<u32>() as u32;
        WinHttpQueryHeaders(
            request.0,
            WINHTTP_QUERY_STATUS_CODE | WINHTTP_QUERY_FLAG_NUMBER,
            PCWSTR::null(),
            Some(&mut status as *mut u32 as *mut _),
            &mut len,
            std::ptr::null_mut(),
        )
        .map_err(|_| "the answer had no status".to_owned())?;
        if status == 404 {
            return Err("there are no releases to see (the forge answered 404)".to_owned());
        }
        if !(200..300).contains(&status) {
            return Err(format!("the forge answered {status}"));
        }

        let mut body = Vec::new();
        let mut buf = [0u8; 64 * 1024];
        loop {
            let mut read: u32 = 0;
            WinHttpReadData(request.0, buf.as_mut_ptr() as *mut _, buf.len() as u32, &mut read)
                .map_err(|e| format!("the download stopped ({})", e.message()))?;
            if read == 0 {
                break;
            }
            body.extend_from_slice(&buf[..read as usize]);
            if body.len() > limit {
                return Err("the download was larger than expected".to_owned());
            }
        }
        Ok(body)
    }
}

/// Host, port and path of an HTTPS URL.
///
/// Refuses anything that is not HTTPS, so there is no code path that could be
/// talked into plain HTTP by a redirect in a release's asset URL.
fn split(url: &str) -> Result<(String, u16, String), String> {
    let wide_url = wide(url);
    let mut host = [0u16; 256];
    let mut path = [0u16; 2048];
    let mut parts = URL_COMPONENTS {
        dwStructSize: std::mem::size_of::<URL_COMPONENTS>() as u32,
        lpszHostName: windows::core::PWSTR(host.as_mut_ptr()),
        dwHostNameLength: host.len() as u32,
        lpszUrlPath: windows::core::PWSTR(path.as_mut_ptr()),
        dwUrlPathLength: path.len() as u32,
        ..Default::default()
    };

    // SAFETY: the structure declares the capacity of each buffer it points at,
    // and both outlive the call.
    unsafe {
        WinHttpCrackUrl(&wide_url[..wide_url.len() - 1], 0, &mut parts)
            .map_err(|_| format!("{url} isn't a usable URL"))?;
    }

    if parts.nScheme != WINHTTP_INTERNET_SCHEME_HTTPS {
        return Err(format!("{url} is not HTTPS, so it will not be fetched"));
    }

    let take = |buf: &[u16], len: u32| String::from_utf16_lossy(&buf[..len as usize]);
    let host = take(&host, parts.dwHostNameLength);
    let path = take(&path, parts.dwUrlPathLength);
    let port = if parts.nPort == 0 { 443 } else { parts.nPort };
    Ok((host, port, if path.is_empty() { "/".to_owned() } else { path }))
}

/// The SHA-256 of `data` as lower-case hex, using Windows' own CNG.
///
/// No hashing crate: the OS has this, and a security tool is a poor place to
/// add a dependency that could be avoided.
pub fn sha256(data: &[u8]) -> Option<String> {
    use windows::Win32::Security::Cryptography::{
        BCRYPT_ALG_HANDLE, BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS, BCRYPT_SHA256_ALGORITHM, BCryptCloseAlgorithmProvider,
        BCryptHash, BCryptOpenAlgorithmProvider,
    };

    let mut alg = BCRYPT_ALG_HANDLE::default();
    let mut out = [0u8; 32];
    // SAFETY: `alg` and `out` are live for the calls, and the provider is
    // closed before returning on both paths.
    unsafe {
        if BCryptOpenAlgorithmProvider(
            &mut alg,
            BCRYPT_SHA256_ALGORITHM,
            PCWSTR::null(),
            BCRYPT_OPEN_ALGORITHM_PROVIDER_FLAGS(0),
        )
        .0 != 0
        {
            return None;
        }
        let hashed = BCryptHash(alg, None, data, &mut out).0 == 0;
        let _ = BCryptCloseAlgorithmProvider(alg, 0);
        if !hashed {
            return None;
        }
    }
    Some(out.iter().fold(String::with_capacity(64), |mut s, b| {
        use std::fmt::Write as _;
        let _ = write!(s, "{b:02x}");
        s
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sha256_matches_the_published_test_vectors() {
        assert_eq!(sha256(b"").as_deref(), Some("e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855"));
        assert_eq!(sha256(b"abc").as_deref(), Some("ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"));
    }

    #[test]
    fn sha256_is_lower_case_hex_of_the_right_length() {
        let h = sha256(b"anything").expect("hashed");
        assert_eq!(h.len(), 64);
        assert!(h.bytes().all(|b| b.is_ascii_hexdigit() && !b.is_ascii_uppercase()), "{h}");
    }

    #[test]
    fn an_https_url_splits_into_host_port_and_path() {
        let (host, port, path) = split("https://git.zacx.net/api/v1/repos/owner/lock-screen/releases").expect("split");
        assert_eq!(host, "git.zacx.net");
        assert_eq!(port, 443);
        assert_eq!(path, "/api/v1/repos/owner/lock-screen/releases");
    }

    #[test]
    fn a_url_with_no_path_gets_a_root_one() {
        let (host, _, path) = split("https://git.zacx.net").expect("split");
        assert_eq!(host, "git.zacx.net");
        assert_eq!(path, "/");
    }

    #[test]
    fn plain_http_is_refused_rather_than_upgraded() {
        // The one check that matters here: an update must never be fetched over
        // a connection anyone on the path can rewrite.
        let err = split("http://git.zacx.net/releases").expect_err("http must be refused");
        assert!(err.contains("not HTTPS"), "{err}");
        for url in ["ftp://x/y", "file:///c:/x", "git.zacx.net/releases", ""] {
            assert!(split(url).is_err(), "{url:?} should not split");
        }
    }

    #[test]
    fn a_non_standard_https_port_is_kept() {
        let (_, port, _) = split("https://git.zacx.net:8443/releases").expect("split");
        assert_eq!(port, 8443);
    }
}
