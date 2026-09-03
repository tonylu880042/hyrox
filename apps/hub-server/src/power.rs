//! Switching the machine off, on the appliance (M6; ADR 0009).
//!
//! The platform-specific half of `api::Power`. Everything about *when* an action is allowed
//! lives above this (a class on the floor wins); everything about *how* lives here, and
//! here is the only file in the workspace that knows this machine runs systemd
//! (CLAUDE.md 2).
//!
//! The hub runs as an unprivileged user under a hardened unit. It calls `systemctl`
//! **without sudo**: unprivileged `systemctl poweroff` is a D-Bus call to logind, not an
//! escalation, so it works under `NoNewPrivileges=yes` -- which sudo does not, by design.
//! What logind allows is a polkit rule the appliance ships
//! (`packaging/etc/polkit-1/rules.d/50-hyrox-power.rules`).
//!
//! Found the hard way on the first real machine: with sudo this returned "permission
//! denied" while the same command worked from a shell, because the hardening flag that
//! makes the service safe is exactly the flag that stops setuid.

use api::{Power, PowerAction};
use std::process::Command;
use std::time::Duration;

/// How long the hub waits before carrying the action out.
///
/// Long enough for the response to reach the tablet that asked. Powering the machine off
/// inside the request handler leaves the operator looking at a failed request with no way
/// to tell whether it worked.
const GRACE: Duration = Duration::from_secs(2);

pub struct SystemdPower;

impl Power for SystemdPower {
    fn request(&self, action: PowerAction) -> Result<(), String> {
        let args: &[&str] = match action {
            PowerAction::Poweroff => &["poweroff"],
            PowerAction::Reboot => &["reboot"],
            PowerAction::RestartService => &["restart", "hyrox-hub.service"],
        };

        // Asked before the delay, so a missing polkit rule reaches the screen rather than
        // being discovered two seconds later by nobody. `--dry-run` goes through the same
        // authorisation as the real call but changes nothing.
        let mut check = Command::new("systemctl");
        check.arg("--dry-run").args(args);
        let allowed = check
            .output()
            .map_err(|e| format!("cannot run systemctl: {e}"))?;
        if !allowed.status.success() {
            return Err(format!(
                "systemd refused: {}",
                String::from_utf8_lossy(&allowed.stderr).trim()
            ));
        }

        let owned: Vec<String> = args.iter().map(|a| a.to_string()).collect();
        std::thread::spawn(move || {
            std::thread::sleep(GRACE);
            let mut command = Command::new("systemctl");
            command.args(&owned);
            match command.status() {
                Ok(status) if status.success() => {}
                Ok(status) => eprintln!("power: systemctl {owned:?} exited with {status}"),
                Err(e) => eprintln!("power: cannot run systemctl {owned:?}: {e}"),
            }
        });
        Ok(())
    }
}
