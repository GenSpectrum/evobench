use std::io::stdout;
use std::io::Write;

use anyhow::Result;
use evobench_evaluator::log_message::Metadata;
use evobench_evaluator::{
    git::GitGraph,
    log_message::{ExecutionTimings, LogMessage, ThreadId},
};

include!("../include/evobench_version.rs");

fn deser() -> Result<()> {
    let msg = r#" { "r_ns": 12349089123, "u_ns": 12312321343, "s_ns": 18292137129837219812 } "#;

    let timings: ExecutionTimings = serde_json::from_str(msg)?;
    dbg!(timings);

    Ok(())
}

fn wr<T: serde::Serialize>(val: &T) -> Result<()> {
    let mut lock = stdout().lock();
    serde_json::ser::to_writer(&mut lock, val)?;
    write!(&mut lock, "\n")?;
    Ok(())
}

// fn ser() -> Result<()> {
//     wr(&LogMessage::Start {
//         evobench_version: EVOBENCH_VERSION.into(),
//         evobench_log_version: 1,
//     })?;
//     wr(&LogMessage::Metadata(Metadata {
//         hostname: "dev-1".into(),
//         os: r#"PRETTY_NAME="Debian GNU/Linux 12 (bookworm)"
// NAME="Debian GNU/Linux"
// VERSION_ID="12"
// VERSION="12 (bookworm)"
// VERSION_CODENAME=bookworm
// ID=debian
// HOME_URL="https://www.debian.org/"
// SUPPORT_URL="https://www.debian.org/support"
// BUG_REPORT_URL="https://bugs.debian.org/"
// "#
//         .into(),
//         compiler: "Debian clang version 16.0.6 (15~deb12u1)
// Target: x86_64-pc-linux-gnu
// Thread model: posix
// InstalledDir: /usr/bin
// "
//         .into(),
//     }))?;
//     wr(&LogMessage::T {
//         r: RealTime {
//             sec: 1743028469,
//             nsec: 123213123,
//         },
//         module: "some".into(),
//         action: "foo".into(),
//         tid: ThreadId(4323),
//         d: ExecutionTimings {
//             r_ns: 12787129712,
//             u_ns: 123829318723,
//             s_ns: 81293817239182,
//         },
//     })?;
//     wr(&LogMessage::T {
//         r: RealTime {
//             sec: 1743028469,
//             nsec: 423213123,
//         },
//         module: "some".into(),
//         action: "bar".into(),
//         tid: ThreadId(4323),
//         d: ExecutionTimings {
//             r_ns: 1278712971232342,
//             u_ns: 123829318723,
//             s_ns: 81293817,
//         },
//     })?;
//     wr(&LogMessage::TEnd)?;
//     Ok(())
// }

fn graph() -> Result<()> {
    let graph = GitGraph::new_dir_ref("/home/chrisrust", "HEAD")?;
    dbg!(graph.commits.len());
    dbg!(&graph.entry_githash);
    if let Some(h) = &graph.entry_githash {
        let c = graph.get(h);
        println!("{h} = {c:?}");
    }
    dbg!(graph);

    Ok(())
}

fn main() -> Result<()> {
    println!("evobench_evaluator tool, version {EVOBENCH_VERSION}");
    // ser()?;

    Ok(())
}
