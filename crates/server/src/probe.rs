use orkestr_protocol::*;
use serde::Serialize;
use std::{
    io::{Read, Write},
    process::Stdio,
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, Instant},
};
use zeroize::Zeroizing;

pub fn run(
    profile: &ConnectionProfile,
    credential: &CredentialProfile,
    secret: Zeroizing<Vec<u8>>,
) -> &'static str {
    match query(
        include_str!("probe.ps1"),
        profile,
        credential,
        secret,
        &(),
        QueryLimits {
            deadline: Duration::from_secs(30),
            max_output: 64,
            stopping: None,
        },
    ) {
        Ok(response) if response == b"READ_ONLY_OK" => "READ_ONLY_OK",
        Ok(_) => "PROBE_OUTPUT_FAILED",
        Err(code) => code,
    }
}

pub(crate) struct QueryLimits<'a> {
    pub deadline: Duration,
    pub max_output: usize,
    pub stopping: Option<&'a AtomicBool>,
}

pub(crate) fn query(
    script: &str,
    profile: &ConnectionProfile,
    credential: &CredentialProfile,
    secret: Zeroizing<Vec<u8>>,
    options: &impl Serialize,
    limits: QueryLimits<'_>,
) -> Result<Vec<u8>, &'static str> {
    if credential.kind != profile.kind || credential.target != profile.host {
        return Err("CREDENTIAL_SCOPE_MISMATCH");
    }
    #[derive(Serialize)]
    struct Input<'a, T> {
        host: &'a str,
        port: u16,
        kind: ProfileKind,
        winrm_authentication: WinRmAuthentication,
        username: &'a str,
        password: &'a str,
        use_server_identity: bool,
        options: &'a T,
    }
    let Ok(password) = std::str::from_utf8(&secret) else {
        return Err("CREDENTIAL_UNAVAILABLE");
    };
    let Ok(input) = serde_json::to_vec(&Input {
        host: &profile.host,
        port: profile.port,
        kind: profile.kind,
        winrm_authentication: profile.winrm_authentication.unwrap_or_default(),
        username: &credential.username,
        password,
        use_server_identity: credential.username == "SERVER_IDENTITY"
            && secret.as_slice() == b"OrkestrEMP integrated identity v1"
            && profile.kind == ProfileKind::Execution
            && profile.winrm_authentication == Some(WinRmAuthentication::Kerberos),
        options,
    }) else {
        return Err("CREDENTIAL_UNAVAILABLE");
    };
    drop(secret);
    invoke(script, Zeroizing::new(input), limits)
}

pub(crate) fn invoke(
    script: &str,
    input: Zeroizing<Vec<u8>>,
    limits: QueryLimits<'_>,
) -> Result<Vec<u8>, &'static str> {
    let Ok(mut child) = crate::platform::powershell(script)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::null())
        .spawn()
    else {
        return Err("PROBE_START_FAILED");
    };
    let pipe = child.stdin.take();
    let stdout = child.stdout.take();
    let max_output = limits.max_output;
    let (output_sender, output) = std::sync::mpsc::sync_channel(1);
    std::thread::spawn(move || {
        let result = stdout.ok_or("PROBE_OUTPUT_FAILED").and_then(|pipe| {
            let mut bytes = Vec::new();
            pipe.take(max_output as u64 + 1)
                .read_to_end(&mut bytes)
                .map_err(|_| "PROBE_OUTPUT_FAILED")?;
            if bytes.len() > max_output {
                return Err("PROBE_OUTPUT_TOO_LARGE");
            }
            Ok(bytes)
        });
        let _ = output_sender.send(result);
    });
    let (written, result) = std::sync::mpsc::sync_channel(1);
    // Even stdin writes are bounded by the parent process deadline.
    std::thread::spawn(move || {
        let ok = pipe.is_some_and(|mut pipe| pipe.write_all(&input).is_ok());
        let _ = written.send(ok);
    });
    let start = Instant::now();
    let mut completed_output = None;
    loop {
        if matches!(result.try_recv(), Ok(false)) {
            let _ = child.kill();
            let _ = child.wait();
            return Err("PROBE_INPUT_FAILED");
        }
        if let Ok(value) = output.try_recv() {
            match value {
                Ok(value) => completed_output = Some(value),
                Err(code) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(code);
                }
            }
        }
        match child.try_wait() {
            Ok(Some(status)) => {
                if !status.success() {
                    return Err("CONNECTION_TEST_FAILED");
                }
                return completed_output.map_or_else(
                    || {
                        output
                            .recv_timeout(Duration::from_secs(1))
                            .map_err(|_| "PROBE_OUTPUT_FAILED")?
                    },
                    Ok,
                );
            }
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Err("PROBE_WAIT_FAILED");
            }
            _ => {}
        }
        let stopping = limits
            .stopping
            .is_some_and(|flag| flag.load(Ordering::Relaxed));
        if stopping || start.elapsed() > limits.deadline {
            let _ = child.kill();
            let _ = child.wait();
            return Err(if stopping {
                "SERVER_STOPPING"
            } else {
                "CONNECTION_TEST_TIMEOUT"
            });
        }
        std::thread::sleep(Duration::from_millis(50));
    }
}
