//! Digital IDs in the Windows certificate store (Personal, for the signed-in user), so a
//! certificate installed there, or on a smart card Windows knows, can sign without a
//! certificate file. The key never leaves Windows: Panna sends it the digest to sign, and
//! Windows asks for a PIN when the key needs one. Nothing is looked up online.

use std::sync::Arc;

use panna_core::HeldKey;

/// A certificate with a private key, as the signing card lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct StoreId {
    /// Windows' name for it: 40 hex digits.
    thumbprint: String,
    label: String,
    /// The certificate and those that issued it, in DER.
    chain: Arc<Vec<Vec<u8>>>,
}

impl std::fmt::Display for StoreId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(&self.label)
    }
}

impl StoreId {
    /// The key, for signing with.
    pub fn held(&self) -> HeldKey {
        let thumbprint = self.thumbprint.clone();
        HeldKey {
            chain: self.chain.to_vec(),
            sign: Arc::new(move |digest: &[u8]| sign(&thumbprint, digest)),
        }
    }
}

/// The store's certificates are listed here only.
pub const AVAILABLE: bool = cfg!(windows);

/// Lists the certificates that are in date and have a private key, each with the
/// certificates that issued it as far as they're in the local stores.
const LIST: &str = "$ErrorActionPreference = 'Stop'; \
    [Console]::OutputEncoding = [Text.Encoding]::UTF8; \
    $now = Get-Date; \
    $others = @(Get-ChildItem Cert:\\CurrentUser\\CA, Cert:\\LocalMachine\\CA, Cert:\\CurrentUser\\Root, Cert:\\LocalMachine\\Root -ErrorAction SilentlyContinue); \
    foreach ($c in Get-ChildItem Cert:\\CurrentUser\\My) { \
      if (-not $c.HasPrivateKey -or $c.NotAfter -lt $now -or $c.NotBefore -gt $now) { continue }; \
      $chain = @($c); $at = $c; \
      for ($i = 0; $i -lt 5 -and $at.Issuer -ne $at.Subject; $i++) { \
        $up = $others | Where-Object { $_.Subject -eq $at.Issuer } | Select-Object -First 1; \
        if ($up -eq $null) { break }; $chain += $up; $at = $up }; \
      $hex = ($chain | ForEach-Object { [BitConverter]::ToString($_.RawData).Replace('-', '') }) -join ','; \
      $name = $c.GetNameInfo('SimpleName', $false) -replace '[\\t\\r\\n]', ' '; \
      $issuer = $c.GetNameInfo('SimpleName', $true) -replace '[\\t\\r\\n]', ' '; \
      \"$($c.Thumbprint)`t$name`t$issuer`t$($c.NotAfter.ToString('yyyy-MM-dd'))`t$hex\" }";

/// Signs the SHA-256 digest on the second line of standard input with the key of the
/// certificate named on the first, writing the signature in hex.
const SIGN: &str = "$ErrorActionPreference = 'Stop'; \
    $t = [Console]::In.ReadLine(); $d = [Console]::In.ReadLine(); \
    $c = Get-Item -LiteralPath ('Cert:\\CurrentUser\\My\\' + $t); \
    $b = New-Object byte[] ($d.Length / 2); \
    for ($i = 0; $i -lt $b.Length; $i++) { $b[$i] = [Convert]::ToByte($d.Substring($i * 2, 2), 16) }; \
    $rsa = [Security.Cryptography.X509Certificates.RSACertificateExtensions]::GetRSAPrivateKey($c); \
    if ($rsa -ne $null) { \
      $s = $rsa.SignHash($b, [Security.Cryptography.HashAlgorithmName]::SHA256, [Security.Cryptography.RSASignaturePadding]::Pkcs1) \
    } else { \
      $s = [Security.Cryptography.X509Certificates.ECDsaCertificateExtensions]::GetECDsaPrivateKey($c).SignHash($b) }; \
    [BitConverter]::ToString($s).Replace('-', '')";

/// Output read from PowerShell at most.
#[cfg(windows)]
const MAX_OUTPUT: u64 = 16 * 1024 * 1024;

fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02X}")).collect()
}

fn unhex(s: &str) -> Option<Vec<u8>> {
    let s = s.trim().as_bytes();
    if !s.len().is_multiple_of(2) {
        return None;
    }
    s.chunks(2)
        .map(|pair| u8::from_str_radix(std::str::from_utf8(pair).ok()?, 16).ok())
        .collect()
}

/// Reads one line of the list.
fn parse(line: &str) -> Option<StoreId> {
    let mut parts = line.trim_end_matches('\r').split('\t');
    let thumbprint = parts.next()?.to_ascii_uppercase();
    if thumbprint.len() != 40 || !thumbprint.bytes().all(|b| b.is_ascii_hexdigit()) {
        return None;
    }
    let (name, issuer, until) = (parts.next()?, parts.next()?, parts.next()?);
    let chain: Vec<Vec<u8>> = parts.next()?.split(',').map(unhex).collect::<Option<_>>()?;
    if chain.is_empty() {
        return None;
    }
    let label = if issuer.is_empty() || issuer == name {
        format!("{name}, until {until}")
    } else {
        format!("{name}, from {issuer}, until {until}")
    };
    Some(StoreId {
        thumbprint,
        label: label.chars().filter(|c| !c.is_control()).take(120).collect(),
        chain: Arc::new(chain),
    })
}

/// The certificates to sign with; none away from Windows.
pub async fn find() -> Result<Vec<StoreId>, String> {
    if !AVAILABLE {
        return Ok(Vec::new());
    }
    let (tx, rx) = futures_channel::oneshot::channel();
    std::thread::spawn(move || {
        let _ = tx.send(powershell(LIST, "").map(|out| out.lines().filter_map(parse).collect()));
    });
    rx.await
        .unwrap_or_else(|_| Err("The certificate store couldn't be read.".into()))
}

fn sign(thumbprint: &str, digest: &[u8]) -> Result<Vec<u8>, String> {
    // Checked again, as it becomes part of a path in the store.
    if thumbprint.len() != 40 || !thumbprint.bytes().all(|b| b.is_ascii_hexdigit()) {
        return Err("That certificate can't be used.".into());
    }
    let out = powershell(SIGN, &format!("{thumbprint}\n{}\n", hex(digest)))?;
    unhex(out.lines().rev().find(|l| !l.trim().is_empty()).unwrap_or(""))
        .filter(|s| !s.is_empty())
        .ok_or_else(|| "Windows didn't sign.".into())
}

/// Runs a constant script with Windows PowerShell, `input` on standard input, and gives
/// what it writes.
#[cfg(windows)]
fn powershell(script: &str, input: &str) -> Result<String, String> {
    use std::io::{Read as _, Write as _};
    use std::os::windows::process::CommandExt;
    use std::process::{Command, Stdio};
    const CREATE_NO_WINDOW: u32 = 0x0800_0000;
    // The full path, so no program of the same name elsewhere is run instead.
    let root = std::env::var_os("SystemRoot").unwrap_or_else(|| "C:\\Windows".into());
    let mut child =
        Command::new(std::path::Path::new(&root).join("System32\\WindowsPowerShell\\v1.0\\powershell.exe"))
            .args(["-NoProfile", "-NonInteractive", "-Command", script])
            .creation_flags(CREATE_NO_WINDOW)
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|e| e.to_string())?;
    if let Some(mut stdin) = child.stdin.take() {
        let _ = stdin.write_all(input.as_bytes());
    }
    let stderr = child.stderr.take();
    let errors = std::thread::spawn(move || {
        let mut err = String::new();
        if let Some(stderr) = stderr {
            let _ = stderr.take(64 * 1024).read_to_string(&mut err);
        }
        err
    });
    let mut out = String::new();
    if let Some(stdout) = child.stdout.take() {
        let _ = stdout.take(MAX_OUTPUT).read_to_string(&mut out);
    }
    let status = child.wait().map_err(|e| e.to_string())?;
    let err = errors.join().unwrap_or_default();
    if status.success() {
        return Ok(out);
    }
    let first = err.lines().find(|l| !l.trim().is_empty()).unwrap_or("").trim();
    Err(if err.contains("canceled") || err.contains("cancelled") {
        "Signing was cancelled.".to_owned()
    } else if first.is_empty() {
        "Windows couldn't sign with that certificate.".to_owned()
    } else {
        first.to_owned()
    })
}

#[cfg(not(windows))]
fn powershell(_script: &str, _input: &str) -> Result<String, String> {
    Err("The certificate store is read on Windows only.".into())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn list_lines_are_read_and_odd_ones_dropped() {
        let line = format!(
            "{}\tPat Example\tPanna Test CA\t2030-01-01\t3082,AB01",
            "a".repeat(40)
        );
        let id = parse(&line).unwrap_or_else(|| panic!("not read"));
        assert_eq!(id.thumbprint, "A".repeat(40));
        assert_eq!(id.label, "Pat Example, from Panna Test CA, until 2030-01-01");
        assert_eq!(*id.chain, vec![vec![0x30, 0x82], vec![0xAB, 0x01]]);
        for bad in [
            format!("{}\tPat\tPat\t2030-01-01\t30", "a".repeat(39)),
            format!("{}\\x\tPat\tPat\t2030-01-01\t30", "a".repeat(38)),
            format!("{}\tPat\tPat\t2030-01-01\t3G", "a".repeat(40)),
            format!("{}\tPat\tPat", "a".repeat(40)),
        ] {
            assert!(parse(&bad).is_none(), "{bad}");
        }
        assert_eq!(unhex(&hex(&[0, 15, 255])), Some(vec![0, 15, 255]));
    }
}
