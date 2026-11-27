use std::{
    io::Write,
    path::Path,
    process::{Command, Stdio},
};

/// Fixed script; neither paths nor credentials are interpolated into PowerShell code.
pub(crate) fn powershell(script: &str) -> Command {
    let mut command =
        Command::new(system_root().join(r"System32\WindowsPowerShell\v1.0\powershell.exe"));
    // PowerShell 7 module paths can shadow the built-in Windows PowerShell modules.
    command
        .env_remove("PSModulePath")
        .args(["-NoProfile", "-NonInteractive", "-Command", script]);
    command
}
fn system_root() -> std::path::PathBuf {
    std::env::var_os("SystemRoot")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| std::path::PathBuf::from(r"C:\Windows"))
}

/// Local NTFS only; no reparse ancestors. ACL applies before writing any secrets.
pub fn protected_directory(path: &Path, initialize: bool) -> Result<(), String> {
    if !cfg!(windows) {
        return Err("WINDOWS_REQUIRED".into());
    }
    if !path.is_absolute() || path.to_string_lossy().starts_with(r"\\") {
        return Err("LOCAL_NTFS_PATH_REQUIRED".into());
    }
    if initialize {
        std::fs::create_dir(path).map_err(|_| "NEW_DIRECTORY_REQUIRED")?;
        let sid = orkestr_credentials::identity().map_err(|_| "IDENTITY_UNAVAILABLE")?;
        let status = Command::new(system_root().join("System32\\icacls.exe"))
            .arg(path)
            .args([
                "/inheritance:r",
                "/grant:r",
                &format!("*{sid}:(OI)(CI)F"),
                "*S-1-5-18:(OI)(CI)F",
                "*S-1-5-32-544:(OI)(CI)F",
            ])
            .stdout(Stdio::null())
            .stderr(Stdio::null())
            .status()
            .map_err(|_| "ACL_FAILED")?;
        if !status.success() {
            return Err("ACL_FAILED".into());
        }
    }
    let script = r#"
$ErrorActionPreference='Stop'
[Console]::InputEncoding=[Text.UTF8Encoding]::new($false)
try {
  $p=[Console]::In.ReadToEnd()
  $item=Get-Item -LiteralPath $p -Force
  $drive=New-Object System.IO.DriveInfo($item.PSDrive.Root)
  if ($drive.DriveType -ne 'Fixed' -or $drive.DriveFormat -ne 'NTFS') { throw 'path' }
  $n=$item
  while ($null -ne $n) {
    if (($n.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'reparse' }
    $n=$n.Parent
  }
  $sid=[Security.Principal.WindowsIdentity]::GetCurrent().User.Value
  $allowed=@($sid,'S-1-5-18','S-1-5-32-544')
  foreach ($f in @($item) + @(Get-ChildItem -LiteralPath $p -Force -Recurse)) {
    if (($f.Attributes -band [IO.FileAttributes]::ReparsePoint) -ne 0) { throw 'reparse' }
    $acl=Get-Acl -LiteralPath $f.FullName
    if ($acl.GetOwner([Security.Principal.SecurityIdentifier]).Value -notin $allowed) { throw 'owner' }
    if ($f.FullName -eq $item.FullName -and -not $acl.AreAccessRulesProtected) { throw 'inheritance' }
    foreach ($r in $acl.Access) {
      $who=$r.IdentityReference.Translate([Security.Principal.SecurityIdentifier]).Value
      if ($r.AccessControlType -ne 'Allow' -or $who -notin $allowed) { throw 'acl' }
    }
  }
  [Console]::Out.Write('OK')
} catch { exit 1 }
"#;
    let mut child = powershell(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
        .map_err(|_| "ACL_CHECK_FAILED")?;
    child
        .stdin
        .take()
        .ok_or("ACL_CHECK_FAILED")?
        .write_all(path.to_string_lossy().as_bytes())
        .map_err(|_| "ACL_CHECK_FAILED")?;
    let output = child.wait_with_output().map_err(|_| "ACL_CHECK_FAILED")?;
    if !output.status.success() || output.stdout != b"OK" {
        return Err("PATH_OR_ACL_UNSAFE".into());
    }
    Ok(())
}
