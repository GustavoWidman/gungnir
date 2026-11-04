//! The bubblewrap backend against real bubblewrap, systemd and toolchains. Run them in the dev
//! shell with `cargo test -- --ignored`.

use std::{collections::BTreeSet, ffi::OsString, fs, path::PathBuf, time::Duration};

use eyre::Result;
use gungnir::{
    egress::{Allowlist, Proxy, Record, Verdict},
    sandbox::{
        Backend, Bubblewrap, Exit, Finished, HOME, Input, Io, Kill, Limits, Mount, Network, Output,
        Spec, WORK,
    },
    secret::{EnvValue, Secret},
    stop::Stop,
    toolchains::{Closure, Toolchains},
};
use tokio::{sync::mpsc, time::timeout};

const MIB: u64 = 1 << 20;

struct Rig {
    toolchains: Toolchains,
    backend: Bubblewrap,
    stop: Stop,
}

impl Rig {
    fn new() -> Result<Self> {
        let toolchains = Toolchains::from_env()?;
        let backend = Bubblewrap::new(toolchains.base().clone())?;
        Ok(Self {
            toolchains,
            backend,
            stop: Stop::default(),
        })
    }

    fn spec(&self) -> Spec {
        let base = self.toolchains.base();
        Spec {
            network: Network::Offline,
            closure: base.closure().clone(),
            mounts: Vec::new(),
            path: vec![base.bin()],
            env: Vec::new(),
            cwd: HOME.into(),
            limits: Limits {
                wall: Duration::from_secs(30),
                memory: 512 * MIB,
                tasks: 128,
                cpu: None,
            },
        }
    }

    async fn exec(&self, spec: &Spec, argv: &[&str], io: Io) -> Result<Finished> {
        let argv: Vec<OsString> = argv.iter().map(OsString::from).collect();
        Ok(self.backend.exec(spec, &argv, io, &self.stop).await?)
    }

    async fn bash(&self, spec: &Spec, script: &str) -> Result<Finished> {
        self.exec(spec, &["bash", "-c", script], captured()).await
    }
}

fn captured() -> Io {
    Io {
        stdin: Input::Null,
        stdout: Output::Capture { cap: MIB },
        stderr: Output::Capture { cap: MIB },
        kill: Kill::default(),
    }
}

fn text(bytes: &[u8]) -> String {
    String::from_utf8_lossy(bytes).into_owned()
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn offline_sandbox_has_no_network() -> Result<()> {
    let rig = Rig::new()?;
    let finished = rig
        .bash(
            &rig.spec(),
            "tail -n +3 /proc/net/dev | cut -d: -f1 | tr -d ' '; exec 3<>/dev/tcp/1.1.1.1/443",
        )
        .await?;
    assert_eq!(text(&finished.stdout.bytes), "lo\n");
    assert_eq!(finished.exit, Exit::Code(1));
    assert!(
        text(&finished.stderr.bytes).contains("Network is unreachable"),
        "{}",
        text(&finished.stderr.bytes)
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn only_closure_paths_are_visible_in_the_store() -> Result<()> {
    let rig = Rig::new()?;
    let root = PathBuf::from(std::env::var_os("GUNGNIR_TOOLCHAINS").expect("toolchains"));
    let mut closures = vec![rig.toolchains.base().closure()];
    for kind in ["language", "harness"] {
        for entry in fs::read_dir(root.join(kind))? {
            let name = entry?.file_name();
            let name = name.to_str().expect("UTF-8 name");
            let toolchain = match kind {
                "language" => rig.toolchains.language(name)?,
                _ => rig.toolchains.harness(name)?,
            };
            closures.push(toolchain.closure());
        }
    }
    let mut spec = rig.spec();
    spec.closure = Closure::union(closures);

    let finished = rig.bash(&spec, "ls /nix; ls -a /nix/store").await?;
    assert_eq!(
        finished.exit,
        Exit::Code(0),
        "{}",
        text(&finished.stderr.bytes)
    );
    let listing = text(&finished.stdout.bytes);
    let (nix, store) = listing.split_once('\n').expect("two listings");
    assert_eq!(nix, "store");
    let visible: BTreeSet<&str> = store
        .lines()
        .filter(|name| !matches!(*name, "." | ".."))
        .collect();
    let expected: BTreeSet<&str> = spec
        .closure
        .paths()
        .map(|path| {
            path.file_name()
                .and_then(|name| name.to_str())
                .expect("name")
        })
        .collect();
    assert_eq!(visible, expected);
    assert!(!visible.iter().any(|name| name.ends_with("-source")));
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn writable_mount_keeps_writes_and_read_only_mount_refuses_them() -> Result<()> {
    let rig = Rig::new()?;
    let workspace = tempfile::tempdir()?;
    let examples = tempfile::tempdir()?;
    fs::write(examples.path().join("case.in"), "3 4\n")?;
    let mut spec = rig.spec();
    spec.mounts = vec![
        Mount::writable(workspace.path(), WORK),
        Mount::read_only(examples.path(), "/examples"),
    ];
    spec.cwd = WORK.into();

    let finished = rig
        .bash(
            &spec,
            "pwd; cat /examples/case.in; echo built > out; ! echo x 2>/dev/null > /examples/case.in",
        )
        .await?;
    assert_eq!(
        finished.exit,
        Exit::Code(0),
        "{}",
        text(&finished.stderr.bytes)
    );
    assert_eq!(text(&finished.stdout.bytes), "/work\n3 4\n");
    assert_eq!(fs::read_to_string(workspace.path().join("out"))?, "built\n");
    assert_eq!(
        fs::read_to_string(examples.path().join("case.in"))?,
        "3 4\n"
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn environment_holds_only_what_the_spec_sets() -> Result<()> {
    let rig = Rig::new()?;
    let mut spec = rig.spec();
    spec.env = vec![
        ("CARGO_HOME".into(), EnvValue::Plain("/tmp/cargo".into())),
        (
            "TOKEN".into(),
            EnvValue::Secret(Secret::new("sk-live-123".into())),
        ),
    ];

    let finished = rig.exec(&spec, &["env"], captured()).await?;
    assert_eq!(
        finished.exit,
        Exit::Code(0),
        "{}",
        text(&finished.stderr.bytes)
    );
    let listing = text(&finished.stdout.bytes);
    let env: BTreeSet<(&str, &str)> = listing
        .lines()
        .filter_map(|line| line.split_once('='))
        .filter(|(name, _)| *name != "PWD")
        .collect();
    let names: BTreeSet<&str> = env.iter().map(|(name, _)| *name).collect();
    assert_eq!(
        names,
        BTreeSet::from([
            "CARGO_HOME",
            "HOME",
            "LOGNAME",
            "PATH",
            "SSL_CERT_FILE",
            "TMPDIR",
            "TOKEN",
            "USER"
        ])
    );
    assert!(env.contains(&("TOKEN", "sk-live-123")));
    assert!(env.contains(&("HOME", HOME)));
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn the_guest_identity_is_constant() -> Result<()> {
    let rig = Rig::new()?;
    let finished = rig
        .bash(
            &rig.spec(),
            "id -u; id -un; uname -n; echo $HOME; test -f \"$SSL_CERT_FILE\" && echo certs",
        )
        .await?;
    assert_eq!(
        text(&finished.stdout.bytes),
        format!("1000\nagent\nsandbox\n{HOME}\ncerts\n")
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn the_wall_time_limit_kills_the_program() -> Result<()> {
    let rig = Rig::new()?;
    let mut spec = rig.spec();
    spec.limits.wall = Duration::from_secs(1);

    let finished = rig.bash(&spec, "echo started; sleep 30").await?;
    assert_eq!(finished.exit, Exit::TimedOut);
    assert_eq!(text(&finished.stdout.bytes), "started\n");
    assert!(
        finished.wall >= Duration::from_secs(1) && finished.wall < Duration::from_secs(5),
        "{:?}",
        finished.wall
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn the_memory_limit_ends_in_out_of_memory() -> Result<()> {
    let rig = Rig::new()?;
    let mut spec = rig.spec();
    spec.limits.memory = 64 * MIB;

    for _ in 0..3 {
        let finished = rig.bash(&spec, "head -c 1G /dev/zero | tail").await?;
        assert_eq!(finished.exit, Exit::OutOfMemory);
    }
    let within = rig
        .bash(&spec, "head -c 16M /dev/zero | tail | wc -c")
        .await?;
    assert_eq!(within.exit, Exit::Code(0));
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn a_cpu_pin_confines_the_program() -> Result<()> {
    let rig = Rig::new()?;
    let mut spec = rig.spec();
    spec.limits.cpu = Some(1);

    let finished = rig
        .bash(&spec, "grep Cpus_allowed_list /proc/self/status")
        .await?;
    assert_eq!(
        text(&finished.stdout.bytes).split_whitespace().last(),
        Some("1")
    );
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn stdin_and_file_output_reach_their_ends() -> Result<()> {
    let rig = Rig::new()?;
    let dir = tempfile::tempdir()?;
    let input = dir.path().join("case.in");
    fs::write(&input, "from a host file\n")?;
    let log = dir.path().join("stdout.log");

    let piped = rig
        .exec(
            &rig.spec(),
            &["cat"],
            Io {
                stdin: Input::Bytes(b"from bytes\n".to_vec()),
                ..captured()
            },
        )
        .await?;
    assert_eq!(text(&piped.stdout.bytes), "from bytes\n");

    let filed = rig
        .exec(
            &rig.spec(),
            &["cat"],
            Io {
                stdin: Input::File(input),
                stdout: Output::File {
                    path: log.clone(),
                    cap: 9,
                },
                ..captured()
            },
        )
        .await?;
    assert_eq!(fs::read_to_string(&log)?, "from a ho");
    assert_eq!(filed.stdout.total, 17);
    assert!(filed.stdout.truncated);
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn lines_stream_live_and_the_kill_handle_ends_the_program() -> Result<()> {
    let rig = Rig::new()?;
    let spec = rig.spec();
    let (sink, mut lines) = mpsc::channel(16);
    let kill = Kill::default();
    let io = Io {
        stdout: Output::Lines { sink, cap: MIB },
        kill: kill.clone(),
        ..captured()
    };
    let argv = ["bash", "-c", "echo one; echo two; exec sleep 30"].map(OsString::from);
    let running = rig.backend.exec(&spec, &argv, io, &rig.stop);
    let reader = async {
        let mut seen = Vec::new();
        while let Some(line) = lines.recv().await {
            seen.push(text(&line));
            if seen.len() == 2 {
                kill.kill();
            }
        }
        seen
    };
    let (finished, seen) = timeout(Duration::from_secs(10), async {
        tokio::join!(running, reader)
    })
    .await?;
    let finished = finished?;
    assert_eq!(seen, ["one", "two"]);
    assert_eq!(finished.exit, Exit::Killed);
    assert!(finished.wall < Duration::from_secs(10));
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn the_second_ctrl_c_kills_running_sandboxes() -> Result<()> {
    let rig = Rig::new()?;
    let spec = rig.spec();
    let running = rig.bash(&spec, "sleep 30");
    let interrupt = async {
        tokio::time::sleep(Duration::from_millis(500)).await;
        rig.stop.escalate();
        rig.stop.escalate();
    };
    let (finished, ()) = tokio::join!(running, interrupt);
    let finished = finished?;
    assert_eq!(finished.exit, Exit::Killed);
    assert!(finished.wall < Duration::from_secs(10));
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn a_sandbox_that_cannot_start_is_a_fault() -> Result<()> {
    let rig = Rig::new()?;
    let mut spec = rig.spec();
    spec.mounts = vec![Mount::read_only("/nonexistent/gungnir", WORK)];

    let fault = rig.bash(&spec, "true").await.unwrap_err();
    assert!(format!("{fault:#}").contains("did not start"), "{fault:#}");
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn egress_reaches_allowlisted_hosts_and_refuses_others() -> Result<()> {
    let rig = Rig::new()?;
    let dir = tempfile::tempdir()?;
    let log = dir.path().join("egress.jsonl");
    let proxy = Proxy::start(
        &dir.path().join("egress.sock"),
        Allowlist::new(["api.z.ai"]),
        &log,
    )
    .await?;
    let mut spec = rig.spec();
    spec.network = Network::Egress(proxy.grant());

    let finished = rig
        .bash(
            &spec,
            r#"
            socat TCP-LISTEN:4443,bind=127.0.0.1,reuseaddr PROXY:127.0.0.1:api.z.ai:443,proxyport=3128 &
            until grep -q ' 0100007F:115B ' /proc/net/tcp; do sleep 0.01; done
            printf 'HEAD / HTTP/1.0\r\nHost: api.z.ai\r\n\r\n' \
              | socat - OPENSSL:127.0.0.1:4443,cafile="$SSL_CERT_FILE",commonname=api.z.ai,snihost=api.z.ai \
              | head -n 1
            socat -u /dev/null PROXY:127.0.0.1:example.com:443,proxyport=3128 2>/dev/null || echo refused
            "#,
        )
        .await?;
    proxy.close().await?;

    let stdout = text(&finished.stdout.bytes);
    let mut lines = stdout.lines();
    assert!(
        lines.next().is_some_and(|line| line.starts_with("HTTP/1.")),
        "{stdout}\n{}",
        text(&finished.stderr.bytes)
    );
    assert_eq!(lines.next(), Some("refused"));

    let records: Vec<Record> = fs::read_to_string(&log)?
        .lines()
        .map(serde_json::from_str)
        .collect::<Result<_, _>>()?;
    let summary: Vec<_> = records
        .iter()
        .map(|record| (record.host.as_deref(), record.port, record.verdict))
        .collect();
    assert_eq!(
        summary,
        [
            (Some("api.z.ai"), Some(443), Verdict::Allowed),
            (Some("example.com"), Some(443), Verdict::Denied),
        ]
    );
    assert!(records[0].bytes_up > 0 && records[0].bytes_down > 0);
    Ok(())
}

#[test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
fn toolchains_from_the_dev_shell_load() -> Result<()> {
    let toolchains = Toolchains::from_env()?;
    assert!(toolchains.base().bin().join("socat").exists());
    assert!(toolchains.language("zig")?.bin().join("zig").exists());
    Ok(())
}

#[tokio::test]
#[ignore = "needs bwrap and GUNGNIR_TOOLCHAINS"]
async fn harness_clis_start_in_an_offline_sandbox() -> Result<()> {
    let rig = Rig::new()?;
    for (harness, binary) in [("claude-code", "claude"), ("codex", "codex")] {
        let toolchain = rig.toolchains.harness(harness)?;
        let mut spec = rig.spec();
        spec.closure = Closure::union([&spec.closure, toolchain.closure()]);
        spec.path.insert(0, toolchain.bin());

        let finished = rig.exec(&spec, &[binary, "--version"], captured()).await?;
        assert_eq!(
            finished.exit,
            Exit::Code(0),
            "{binary}: {}",
            text(&finished.stderr.bytes)
        );
        assert!(
            !finished.stdout.bytes.is_empty(),
            "{binary} printed no version"
        );
    }
    Ok(())
}
