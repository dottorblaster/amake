use serde::Deserialize;

#[derive(Debug, Clone, Default, Deserialize, PartialEq)]
#[serde(default)]
pub struct SandboxConfig {
    pub agent_policy: Option<String>,
    pub agent_allow: Vec<String>,
    pub pod_policy: Option<String>,
    pub memory: Option<String>,
    pub cpus: Option<String>,
    pub protect: Vec<String>,
    pub mask: Vec<String>,
    pub unmask: Vec<String>,
    pub gitconfig: bool,
    pub gh: bool,
    pub ssh: bool,
    pub tripwire: bool,
    pub extra_args: Vec<String>,
}

impl SandboxConfig {
    pub fn to_args(&self) -> Vec<String> {
        let mut args = Vec::new();

        if let Some(ref policy) = self.agent_policy {
            args.extend(["--agent-policy".into(), policy.clone()]);
        }
        for domain in &self.agent_allow {
            args.extend(["--agent-allow".into(), domain.clone()]);
        }
        if let Some(ref policy) = self.pod_policy {
            args.extend(["--pod-policy".into(), policy.clone()]);
        }
        if let Some(ref mem) = self.memory {
            args.extend(["--memory".into(), mem.clone()]);
        }
        if let Some(ref cpus) = self.cpus {
            args.extend(["--cpus".into(), cpus.clone()]);
        }
        for path in &self.protect {
            args.extend(["--protect".into(), path.clone()]);
        }
        for path in &self.mask {
            args.extend(["--mask".into(), path.clone()]);
        }
        for path in &self.unmask {
            args.extend(["--unmask".into(), path.clone()]);
        }
        if self.gitconfig {
            args.push("--gitconfig".into());
        }
        if self.gh {
            args.push("--gh".into());
        }
        if self.ssh {
            args.push("--ssh".into());
        }
        if self.tripwire {
            args.push("--tripwire".into());
        }
        args.extend(self.extra_args.iter().cloned());

        args
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn default_produces_no_args() {
        assert!(SandboxConfig::default().to_args().is_empty());
    }

    #[test]
    fn full_config_produces_correct_args() {
        let cfg = SandboxConfig {
            agent_policy: Some("deny".into()),
            agent_allow: vec!["api.github.com".into(), "example.com".into()],
            pod_policy: Some("allow".into()),
            memory: Some("4g".into()),
            cpus: Some("4".into()),
            protect: vec![".env".into()],
            mask: vec!["/secrets".into()],
            unmask: vec!["/tmp".into()],
            gitconfig: true,
            gh: true,
            ssh: false,
            tripwire: true,
            extra_args: vec!["--verbose".into()],
        };

        let args = cfg.to_args();
        assert_eq!(
            args,
            vec![
                "--agent-policy", "deny",
                "--agent-allow", "api.github.com",
                "--agent-allow", "example.com",
                "--pod-policy", "allow",
                "--memory", "4g",
                "--cpus", "4",
                "--protect", ".env",
                "--mask", "/secrets",
                "--unmask", "/tmp",
                "--gitconfig",
                "--gh",
                "--tripwire",
                "--verbose",
            ]
        );
    }
}
