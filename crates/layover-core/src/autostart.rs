//! Starting the Tower when the computer starts.
//!
//! A lights-out factory that stops at every reboot is not lights-out. This module generates the
//! platform's own autostart artefact — a Scheduled Task on Windows, a launchd agent on macOS, a
//! systemd user unit on Linux — rather than inventing a daemon of its own.
//!
//! Generating rather than installing is deliberate for the part that can be: the text is
//! inspectable, diffable and testable on any platform, so what gets written is decided by code
//! that runs everywhere and only the final file write is platform-specific.
//!
//! # Why a *user* service, never a system one
//!
//! The Tower spawns agent CLIs that use *your* provider credentials, *your* git identity and
//! *your* workspace. A machine-wide service would run as another user and have none of them, or
//! worse, run as root with all of them. Every artefact here installs into the logged-in user's
//! own session.

use std::fmt;
use std::path::{Path, PathBuf};

/// Which autostart mechanism to generate for.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Platform {
    /// A Windows Scheduled Task, triggered at logon.
    Windows,
    /// A macOS launchd user agent.
    MacOs,
    /// A systemd user unit.
    Linux,
}

impl Platform {
    /// Returns the platform this binary was built for.
    #[must_use]
    pub fn current() -> Option<Self> {
        match std::env::consts::OS {
            "windows" => Some(Self::Windows),
            "macos" => Some(Self::MacOs),
            "linux" => Some(Self::Linux),
            _ => None,
        }
    }

    /// The file name the generated artefact is written as.
    #[must_use]
    pub fn artefact_name(&self) -> &'static str {
        match self {
            Self::Windows => "layover-autostart.xml",
            Self::MacOs => "dev.layover.tower.plist",
            Self::Linux => "layover.service",
        }
    }

    /// What to run to register the generated artefact.
    #[must_use]
    pub fn install_hint(&self, artefact: &Path) -> String {
        let path = artefact.display();
        match self {
            Self::Windows => format!(
                "schtasks /Create /TN Layover /XML \"{path}\" /F\n\
                 Remove it later with: schtasks /Delete /TN Layover /F"
            ),
            Self::MacOs => format!(
                "cp \"{path}\" ~/Library/LaunchAgents/\n\
                 launchctl load -w ~/Library/LaunchAgents/dev.layover.tower.plist\n\
                 Remove it later with: launchctl unload -w ~/Library/LaunchAgents/dev.layover.tower.plist"
            ),
            Self::Linux => format!(
                "cp \"{path}\" ~/.config/systemd/user/\n\
                 systemctl --user enable --now layover.service\n\
                 Remove it later with: systemctl --user disable --now layover.service"
            ),
        }
    }
}

impl fmt::Display for Platform {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Windows => "Windows Scheduled Task",
            Self::MacOs => "launchd user agent",
            Self::Linux => "systemd user unit",
        })
    }
}

/// What the generated artefact needs to know.
#[derive(Debug, Clone)]
pub struct Autostart {
    /// Absolute path to the `layover` binary.
    pub binary: PathBuf,
    /// Absolute path to the factory definition.
    pub config: PathBuf,
}

impl Autostart {
    /// Describes an autostart entry.
    #[must_use]
    pub fn new(binary: impl Into<PathBuf>, config: impl Into<PathBuf>) -> Self {
        Self {
            binary: binary.into(),
            config: config.into(),
        }
    }

    /// Renders the artefact for `platform`.
    #[must_use]
    pub fn render(&self, platform: Platform) -> String {
        match platform {
            Platform::Windows => self.scheduled_task(),
            Platform::MacOs => self.launch_agent(),
            Platform::Linux => self.systemd_unit(),
        }
    }

    fn binary(&self) -> String {
        self.binary.display().to_string()
    }

    fn config(&self) -> String {
        self.config.display().to_string()
    }

    /// A Scheduled Task registered at logon.
    ///
    /// `RunLevel` is `LeastPrivilege` on purpose: the Tower must run as the logged-in user, with
    /// that user's credentials and git identity, and nothing it does wants administrator rights.
    fn scheduled_task(&self) -> String {
        let binary = xml_escape(&self.binary());
        let config = xml_escape(&self.config());
        let work_dir = xml_escape(
            &self
                .config
                .parent()
                .unwrap_or(Path::new("."))
                .display()
                .to_string(),
        );

        format!(
            r#"<?xml version="1.0" encoding="UTF-16"?>
<Task version="1.4" xmlns="http://schemas.microsoft.com/windows/2004/02/mit/task">
  <RegistrationInfo>
    <Description>Layover — runs the agent factory defined in {config}.</Description>
    <URI>\Layover</URI>
  </RegistrationInfo>
  <Triggers>
    <LogonTrigger>
      <Enabled>true</Enabled>
    </LogonTrigger>
  </Triggers>
  <Principals>
    <Principal id="Author">
      <LogonType>InteractiveToken</LogonType>
      <RunLevel>LeastPrivilege</RunLevel>
    </Principal>
  </Principals>
  <Settings>
    <MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>
    <DisallowStartIfOnBatteries>false</DisallowStartIfOnBatteries>
    <StopIfGoingOnBatteries>false</StopIfGoingOnBatteries>
    <AllowHardTerminate>true</AllowHardTerminate>
    <StartWhenAvailable>true</StartWhenAvailable>
    <ExecutionTimeLimit>PT0S</ExecutionTimeLimit>
    <RestartOnFailure>
      <Interval>PT1M</Interval>
      <Count>3</Count>
    </RestartOnFailure>
  </Settings>
  <Actions Context="Author">
    <Exec>
      <Command>{binary}</Command>
      <Arguments>run --config "{config}"</Arguments>
      <WorkingDirectory>{work_dir}</WorkingDirectory>
    </Exec>
  </Actions>
</Task>
"#
        )
    }

    /// A launchd agent, loaded at login.
    fn launch_agent(&self) -> String {
        let binary = xml_escape(&self.binary());
        let config = xml_escape(&self.config());

        format!(
            r#"<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
  <key>Label</key>
  <string>dev.layover.tower</string>
  <key>ProgramArguments</key>
  <array>
    <string>{binary}</string>
    <string>run</string>
    <string>--config</string>
    <string>{config}</string>
  </array>
  <key>RunAtLoad</key>
  <true/>
  <key>KeepAlive</key>
  <dict>
    <key>SuccessfulExit</key>
    <false/>
  </dict>
  <key>ProcessType</key>
  <string>Background</string>
</dict>
</plist>
"#
        )
    }

    /// A systemd user unit.
    ///
    /// `default.target` rather than `multi-user.target`, because this is a user service: it starts
    /// with the session that owns the credentials, not with the machine.
    fn systemd_unit(&self) -> String {
        format!(
            "[Unit]\n\
             Description=Layover — runs the agent factory defined in {config}\n\
             After=network-online.target\n\
             Wants=network-online.target\n\
             \n\
             [Service]\n\
             Type=simple\n\
             ExecStart={binary} run --config {config}\n\
             Restart=on-failure\n\
             RestartSec=60\n\
             \n\
             [Install]\n\
             WantedBy=default.target\n",
            binary = self.binary(),
            config = self.config()
        )
    }
}

/// Escapes text for an XML text node or attribute.
fn xml_escape(raw: &str) -> String {
    let mut out = String::with_capacity(raw.len());
    for ch in raw.chars() {
        match ch {
            '&' => out.push_str("&amp;"),
            '<' => out.push_str("&lt;"),
            '>' => out.push_str("&gt;"),
            '"' => out.push_str("&quot;"),
            '\'' => out.push_str("&apos;"),
            other => out.push(other),
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn entry() -> Autostart {
        Autostart::new(
            r"C:\Users\ada\.cargo\bin\layover.exe",
            r"C:\Users\ada\factory\layover.toml",
        )
    }

    #[test]
    fn the_scheduled_task_runs_at_logon_without_elevation() {
        let xml = entry().render(Platform::Windows);

        assert!(xml.contains("<LogonTrigger>"));
        assert!(
            xml.contains("<RunLevel>LeastPrivilege</RunLevel>"),
            "the Tower runs as the user whose credentials it uses, never elevated"
        );
        assert!(
            xml.contains("<ExecutionTimeLimit>PT0S</ExecutionTimeLimit>"),
            "a factory is long-running; a time limit would kill it"
        );
        assert!(
            xml.contains("<MultipleInstancesPolicy>IgnoreNew</MultipleInstancesPolicy>"),
            "a second Tower would fight the first over the same state directory"
        );
    }

    #[test]
    fn the_launch_agent_restarts_only_on_failure() {
        let plist = entry().render(Platform::MacOs);

        assert!(plist.contains("<string>dev.layover.tower</string>"));
        assert!(plist.contains("<key>RunAtLoad</key>"));
        assert!(
            plist.contains("<key>SuccessfulExit</key>"),
            "a clean exit is a Ground Stop or a deliberate shutdown, not something to undo"
        );
    }

    #[test]
    fn the_systemd_unit_is_a_user_service() {
        let unit = Autostart::new("/usr/local/bin/layover", "/home/ada/factory/layover.toml")
            .render(Platform::Linux);

        assert!(unit.contains("ExecStart=/usr/local/bin/layover run --config"));
        assert!(
            unit.contains("WantedBy=default.target"),
            "a user unit starts with the session that owns the credentials"
        );
        assert!(unit.contains("Restart=on-failure"));
    }

    #[test]
    fn paths_with_xml_significant_characters_are_escaped() {
        // A directory called `A & B` would otherwise produce a Scheduled Task XML that
        // `schtasks` refuses to import, with an error naming neither the file nor the reason.
        let xml = Autostart::new(r"C:\bin\layover.exe", r"C:\A & B\<odd>\layover.toml")
            .render(Platform::Windows);

        assert!(xml.contains("A &amp; B"), "{xml}");
        assert!(xml.contains("&lt;odd&gt;"), "{xml}");
        assert!(!xml.contains("A & B"));
    }

    #[test]
    fn every_platform_names_its_own_artefact_and_command() {
        for platform in [Platform::Windows, Platform::MacOs, Platform::Linux] {
            let name = platform.artefact_name();
            let hint = platform.install_hint(Path::new("/tmp").join(name).as_path());

            assert!(!name.is_empty());
            assert!(hint.contains(name), "{platform}: {hint}");
            assert!(
                hint.contains("Remove it later"),
                "{platform} must say how to undo it"
            );
        }
    }

    #[test]
    fn the_config_path_reaches_the_command_line_on_every_platform() {
        for platform in [Platform::Windows, Platform::MacOs, Platform::Linux] {
            let rendered = entry().render(platform);
            assert!(
                rendered.contains("layover.toml"),
                "{platform} lost the config path"
            );
        }
    }
}
