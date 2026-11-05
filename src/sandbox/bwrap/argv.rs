//! The pure half of the backend: the guest layout and the command line that builds it.

use std::{
    ffi::{OsStr, OsString},
    os::fd::RawFd,
    path::{Path, PathBuf},
};

use crate::{
    sandbox::{HOME, Network, OomKill, Spec},
    secret::{EnvValue, Secret},
    toolchains::{Closure, Toolchain},
};

/// The launcher writes one byte here just before it execs the command, and closes it for the
/// command. Silence means the sandbox never got that far: bwrap reports a failed mount with the
/// same exit code a program could use.
pub(super) const READY_FD: RawFd = 3;
/// bwrap reads the arguments that set secret variables from here, off every command line.
pub(super) const ARGS_FD: RawFd = 4;

const USER: &str = "agent";
const UID: u32 = 1000;
const HOSTNAME: &str = "sandbox";
const PROXY_SOCKET: &str = "/run/egress.sock";
const PROXY_PORT: u16 = 3128;
const NO_PROXY: &str = "localhost,127.0.0.1,::1";
const CA_BUNDLE: &str = "etc/ssl/certs/ca-bundle.crt";
/// How many 10 ms polls the egress bridge gets to start listening.
const BRIDGE_POLLS: u32 = 500;

/// Files of the base toolchain that every sandbox relies on, relative to its root.
pub(super) const BASE_FILES: [&str; 6] = [
    "bin/sh",
    "bin/env",
    "bin/socat",
    "bin/grep",
    "bin/sleep",
    CA_BUNDLE,
];

/// The contents of the guest's `/etc`.
pub(super) fn etc_files() -> [(&'static str, String); 3] {
    [
        (
            "passwd",
            format!("{USER}:x:{UID}:{UID}:{USER}:{HOME}:/bin/sh\n"),
        ),
        ("group", format!("{USER}:x:{UID}:\n")),
        (
            "hosts",
            format!("127.0.0.1 localhost {HOSTNAME}\n::1 localhost\n"),
        ),
    ]
}

/// What the backend contributes to every sandbox besides the `Spec`.
pub(super) struct Layout<'a> {
    /// The systemd scope, without its `.scope` suffix.
    pub unit: &'a str,
    pub base: &'a Toolchain,
    /// Host directory holding the files of `etc_files`.
    pub etc: &'a Path,
}

pub(super) struct Invocation {
    pub argv: Vec<OsString>,
    /// NUL-separated bwrap arguments for `ARGS_FD`.
    pub secret_args: Option<Secret<Vec<u8>>>,
}

#[derive(Default)]
struct Argv(Vec<OsString>);

impl Argv {
    fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.0.push(arg.as_ref().to_owned());
        self
    }
}

pub(super) fn invocation(layout: &Layout<'_>, spec: &Spec, command: &[OsString]) -> Invocation {
    let base = layout.base.root();
    let limits = &spec.limits;
    let mut argv = Argv::default();
    argv.arg("systemd-run")
        .arg("--user")
        .arg("--scope")
        .arg("--quiet")
        // With --scope, systemd-run itself would expand `$VAR` in the command it execs.
        .arg("--expand-environment=no")
        .arg(format!("--unit={}", layout.unit))
        .arg(format!("--property=MemoryMax={}", limits.memory))
        .arg("--property=MemorySwapMax=0")
        .arg(format!("--property=TasksMax={}", limits.tasks))
        .arg(format!(
            "--property=OOMPolicy={}",
            match limits.oom {
                OomKill::Sandbox => "stop",
                OomKill::Process => "continue",
            }
        ))
        .arg("--");
    if let Some(cpu) = limits.cpu {
        argv.arg("taskset").arg("--cpu-list").arg(cpu.to_string());
    }
    argv.arg("bwrap")
        .arg("--unshare-all")
        .arg("--die-with-parent")
        .arg("--new-session")
        .arg("--clearenv")
        .arg("--uid")
        .arg(UID.to_string())
        .arg("--gid")
        .arg(UID.to_string())
        .arg("--hostname")
        .arg(HOSTNAME)
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev")
        .arg("--tmpfs")
        .arg("/tmp")
        .arg("--tmpfs")
        .arg(HOME)
        .arg("--ro-bind")
        .arg(layout.etc)
        .arg("/etc")
        .arg("--symlink")
        .arg(base.join("bin/sh"))
        .arg("/bin/sh")
        .arg("--symlink")
        .arg(base.join("bin/env"))
        .arg("/usr/bin/env");
    for path in Closure::union([&spec.closure, layout.base.closure()]).paths() {
        argv.arg("--ro-bind").arg(path).arg(path);
    }
    if let Network::Egress(grant) = &spec.network {
        argv.arg("--bind").arg(grant.socket()).arg(PROXY_SOCKET);
    }
    for mount in &spec.mounts {
        let bind = if mount.writable {
            "--bind"
        } else {
            "--ro-bind"
        };
        argv.arg(bind).arg(&mount.host).arg(&mount.guest);
    }
    argv.arg("--chdir").arg(&spec.cwd);
    for (name, value) in environment(base, spec) {
        argv.arg("--setenv").arg(name).arg(value);
    }
    let secret_args = secret_args(spec);
    if secret_args.is_some() {
        argv.arg("--args").arg(ARGS_FD.to_string());
    }
    argv.arg("--")
        .arg(base.join("bin/sh"))
        .arg("-c")
        .arg(launcher(base, &spec.network))
        .arg("sh");
    for arg in command {
        argv.arg(arg);
    }
    Invocation {
        argv: argv.0,
        secret_args,
    }
}

/// The plain variables, in the order bwrap sets them: the guest's own, then the spec's, which
/// may override them.
fn environment(base: &Path, spec: &Spec) -> Vec<(OsString, OsString)> {
    let mut env: Vec<(OsString, OsString)> = vec![
        ("PATH".into(), search_path(&spec.path)),
        ("HOME".into(), HOME.into()),
        ("USER".into(), USER.into()),
        ("LOGNAME".into(), USER.into()),
        ("TMPDIR".into(), "/tmp".into()),
        ("SSL_CERT_FILE".into(), base.join(CA_BUNDLE).into()),
    ];
    if let Network::Egress(_) = spec.network {
        let proxy = format!("http://127.0.0.1:{PROXY_PORT}");
        for name in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"] {
            env.push((name.into(), proxy.clone().into()));
        }
        for name in ["NO_PROXY", "no_proxy"] {
            env.push((name.into(), NO_PROXY.into()));
        }
    }
    for (name, value) in &spec.env {
        if let EnvValue::Plain(value) = value {
            env.push((name.into(), value.into()));
        }
    }
    env
}

fn secret_args(spec: &Spec) -> Option<Secret<Vec<u8>>> {
    let mut payload = Vec::new();
    for (name, value) in &spec.env {
        if let EnvValue::Secret(secret) = value {
            for field in ["--setenv", name, secret.expose()] {
                payload.extend_from_slice(field.as_bytes());
                payload.push(0);
            }
        }
    }
    (!payload.is_empty()).then(|| Secret::new(payload))
}

fn search_path(dirs: &[PathBuf]) -> OsString {
    let mut path = OsString::new();
    for (index, dir) in dirs.iter().enumerate() {
        if index > 0 {
            path.push(":");
        }
        path.push(dir);
    }
    path
}

/// The shell script that starts every command. With egress it first bridges
/// `127.0.0.1:PROXY_PORT` to the bound proxy socket with socat and waits until the bridge
/// listens. Store paths hold no quote characters.
fn launcher(base: &Path, network: &Network) -> String {
    let tool = |name: &str| format!("'{}'", base.join("bin").join(name).display());
    let bridge = match network {
        Network::Offline => String::new(),
        Network::Egress(_) => {
            let (socat, grep, sleep) = (tool("socat"), tool("grep"), tool("sleep"));
            format!(
                "{socat} TCP-LISTEN:{PROXY_PORT},bind=127.0.0.1,fork,reuseaddr UNIX-CONNECT:{PROXY_SOCKET} </dev/null >/dev/null {READY_FD}>&- &
polls=0
until {grep} -q ' 0100007F:{PROXY_PORT:04X} 00000000:0000 0A ' /proc/net/tcp; do
  polls=$((polls + 1))
  if [ \"$polls\" -gt {BRIDGE_POLLS} ]; then echo 'sandbox: the egress bridge did not start' >&2; exit 125; fi
  {sleep} 0.01
done
"
            )
        }
    };
    // bash's exec re-exports SHLVL even when unset; env drops it, so the command gets exactly
    // the environment bwrap set.
    let env = tool("env");
    format!("{bridge}printf . >&{READY_FD}\nexec {env} -u SHLVL \"$@\" {READY_FD}>&-\n")
}

#[cfg(test)]
mod tests {
    use std::{collections::BTreeSet, time::Duration};

    use super::*;
    use crate::{
        egress::Grant,
        sandbox::{Limits, Mount, WORK},
    };

    const BASE: &str = "/nix/store/aaaa-gungnir-base";
    const LANGUAGE: &str = "/nix/store/cccc-gungnir-language-zig";
    const LIBC: &str = "/nix/store/bbbb-glibc-2.42";

    fn base() -> Toolchain {
        let closure = Closure::parse(&format!("{BASE}\n{LIBC}\n")).expect("base closure");
        Toolchain::new(BASE.into(), closure).expect("base toolchain")
    }

    fn spec(network: Network) -> Spec {
        Spec {
            network,
            closure: Closure::parse(&format!("{LANGUAGE}\n{LIBC}\n")).expect("closure"),
            mounts: vec![
                Mount::writable("/host/workspace", WORK),
                Mount::read_only("/host/examples", "/examples"),
            ],
            path: vec![
                PathBuf::from(LANGUAGE).join("bin"),
                PathBuf::from(BASE).join("bin"),
            ],
            env: vec![
                ("CARGO_HOME".into(), EnvValue::Plain("/tmp/cargo".into())),
                (
                    "TOKEN".into(),
                    EnvValue::Secret(Secret::new("sk-live-123".into())),
                ),
            ],
            cwd: WORK.into(),
            limits: Limits {
                wall: Duration::from_secs(60),
                memory: 1 << 30,
                tasks: 256,
                cpu: None,
                oom: OomKill::Sandbox,
            },
        }
    }

    fn build(spec: &Spec) -> (Vec<String>, Option<Secret<Vec<u8>>>) {
        let base = base();
        let layout = Layout {
            unit: "gungnir-test-1",
            base: &base,
            etc: Path::new("/tmp/gungnir-etc"),
        };
        let invocation = invocation(&layout, spec, &["zig".into(), "build".into()]);
        let argv = invocation
            .argv
            .iter()
            .map(|arg| arg.to_str().expect("UTF-8 argument").to_owned())
            .collect();
        (argv, invocation.secret_args)
    }

    /// The operands of every occurrence of `flag`.
    fn operands<'a>(argv: &'a [String], flag: &str, count: usize) -> Vec<Vec<&'a str>> {
        argv.iter()
            .enumerate()
            .filter(|(_, arg)| *arg == flag)
            .map(|(at, _)| {
                argv[at + 1..=at + count]
                    .iter()
                    .map(String::as_str)
                    .collect()
            })
            .collect()
    }

    fn setenv<'a>(argv: &'a [String], name: &str) -> Option<&'a str> {
        operands(argv, "--setenv", 2)
            .into_iter()
            .rfind(|pair| pair[0] == name)
            .map(|pair| pair[1])
    }

    /// The command bwrap execs: everything after its `--`.
    fn guest_command(argv: &[String]) -> &[String] {
        let at = argv.iter().rposition(|arg| arg == "--").expect("command");
        &argv[at + 1..]
    }

    #[test]
    fn runs_bwrap_in_a_limited_user_scope() {
        let (argv, _) = build(&spec(Network::Offline));
        let bwrap = argv.iter().position(|arg| arg == "bwrap").expect("bwrap");
        assert_eq!(argv[..3], ["systemd-run", "--user", "--scope"]);
        let scope = &argv[..bwrap];
        for expected in [
            "--expand-environment=no",
            "--unit=gungnir-test-1",
            "--property=MemoryMax=1073741824",
            "--property=MemorySwapMax=0",
            "--property=TasksMax=256",
            "--property=OOMPolicy=stop",
        ] {
            assert!(scope.iter().any(|arg| arg == expected), "{expected}");
        }
        assert!(!scope.iter().any(|arg| arg == "taskset"));
        assert_eq!(
            argv[bwrap + 1..bwrap + 5],
            [
                "--unshare-all",
                "--die-with-parent",
                "--new-session",
                "--clearenv"
            ]
        );
    }

    #[test]
    fn a_cpu_pin_runs_bwrap_under_taskset() {
        let mut pinned = spec(Network::Offline);
        pinned.limits.cpu = Some(3);
        let (argv, _) = build(&pinned);
        let taskset = argv
            .iter()
            .position(|arg| arg == "taskset")
            .expect("taskset");
        assert_eq!(
            argv[taskset..taskset + 4],
            ["taskset", "--cpu-list", "3", "bwrap"]
        );
    }

    #[test]
    fn binds_exactly_the_closures_read_only() {
        let (argv, _) = build(&spec(Network::Offline));
        let store: BTreeSet<_> = operands(&argv, "--ro-bind", 2)
            .into_iter()
            .chain(operands(&argv, "--bind", 2))
            .filter(|bind| bind[0].starts_with("/nix"))
            .inspect(|bind| assert_eq!(bind[0], bind[1], "store paths keep their path"))
            .map(|bind| bind[0])
            .collect();
        assert_eq!(store, BTreeSet::from([BASE, LIBC, LANGUAGE]));
    }

    #[test]
    fn provides_a_private_tmp_home_and_minimal_etc() {
        let (argv, _) = build(&spec(Network::Offline));
        let tmpfs: Vec<_> = operands(&argv, "--tmpfs", 1).concat();
        assert_eq!(tmpfs, ["/tmp", HOME]);
        assert!(operands(&argv, "--ro-bind", 2).contains(&vec!["/tmp/gungnir-etc", "/etc"]));
        assert!(
            operands(&argv, "--symlink", 2)
                .contains(&vec!["/nix/store/aaaa-gungnir-base/bin/sh", "/bin/sh"])
        );
        assert_eq!(setenv(&argv, "HOME"), Some(HOME));
        assert_eq!(
            setenv(&argv, "SSL_CERT_FILE"),
            Some("/nix/store/aaaa-gungnir-base/etc/ssl/certs/ca-bundle.crt")
        );
    }

    #[test]
    fn mounts_keep_their_access_and_come_after_the_store() {
        let (argv, _) = build(&spec(Network::Offline));
        let workspace = argv
            .iter()
            .position(|arg| arg == "/host/workspace")
            .expect("workspace");
        assert_eq!(
            argv[workspace - 1..=workspace + 1],
            ["--bind", "/host/workspace", WORK]
        );
        let examples = argv
            .iter()
            .position(|arg| arg == "/host/examples")
            .expect("examples");
        assert_eq!(argv[examples - 1], "--ro-bind");
        let last_store = argv.iter().rposition(|arg| arg == LANGUAGE).expect("store");
        assert!(workspace > last_store);
        assert_eq!(operands(&argv, "--chdir", 1), [[WORK]]);
    }

    #[test]
    fn plain_variables_are_set_and_secrets_stay_off_the_command_line() {
        let (argv, secret_args) = build(&spec(Network::Offline));
        assert_eq!(
            setenv(&argv, "PATH"),
            Some("/nix/store/cccc-gungnir-language-zig/bin:/nix/store/aaaa-gungnir-base/bin")
        );
        assert_eq!(setenv(&argv, "CARGO_HOME"), Some("/tmp/cargo"));
        assert!(!argv.iter().any(|arg| arg.contains("sk-live-123")));
        assert_eq!(operands(&argv, "--args", 1), [[ARGS_FD.to_string()]]);
        assert_eq!(
            secret_args.expect("secret args").expose().as_slice(),
            b"--setenv\0TOKEN\0sk-live-123\0"
        );
    }

    #[test]
    fn without_secrets_no_argument_fd_is_read() {
        let mut plain = spec(Network::Offline);
        plain
            .env
            .retain(|(_, value)| matches!(value, EnvValue::Plain(_)));
        let (argv, secret_args) = build(&plain);
        assert!(secret_args.is_none());
        assert!(operands(&argv, "--args", 1).is_empty());
    }

    #[test]
    fn offline_launches_the_command_without_a_proxy() {
        let (argv, _) = build(&spec(Network::Offline));
        let command = guest_command(&argv);
        assert_eq!(command[..2], ["/nix/store/aaaa-gungnir-base/bin/sh", "-c"]);
        assert_eq!(
            command[2],
            "printf . >&3\nexec '/nix/store/aaaa-gungnir-base/bin/env' -u SHLVL \"$@\" 3>&-\n"
        );
        assert_eq!(command[3..], ["sh", "zig", "build"]);
        assert_eq!(setenv(&argv, "HTTPS_PROXY"), None);
        assert!(!argv.iter().any(|arg| arg == PROXY_SOCKET));
    }

    #[test]
    fn egress_bridges_the_proxy_socket_before_the_command() {
        let (argv, _) = build(&spec(Network::Egress(Grant::at("/run/user/1/egress.sock"))));
        assert!(
            operands(&argv, "--bind", 2).contains(&vec!["/run/user/1/egress.sock", PROXY_SOCKET])
        );
        for name in ["HTTPS_PROXY", "https_proxy", "HTTP_PROXY", "http_proxy"] {
            assert_eq!(setenv(&argv, name), Some("http://127.0.0.1:3128"), "{name}");
        }
        assert_eq!(setenv(&argv, "NO_PROXY"), Some(NO_PROXY));

        let command = guest_command(&argv);
        assert_eq!(command[0], "/nix/store/aaaa-gungnir-base/bin/sh");
        assert_eq!(command[1], "-c");
        assert!(command[2].contains("TCP-LISTEN:3128,bind=127.0.0.1"));
        assert!(command[2].contains("UNIX-CONNECT:/run/egress.sock"));
        assert!(command[2].ends_with("-u SHLVL \"$@\" 3>&-\n"));
        assert!(command[2].find("UNIX-CONNECT") < command[2].find("printf . >&3"));
        assert_eq!(command[3..], ["sh", "zig", "build"]);
    }
}
