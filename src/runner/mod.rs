mod graph;
mod tasks;
mod test;
mod unstable_features;
mod worker;

use crate::config::Config;
use crate::crates::Crate;
use crate::experiments::{Experiment, Mode};
use crate::prelude::*;
use crate::results::{TestResult, WriteResults};
use crate::runner::graph::build_graph;
use crate::runner::worker::{DiskSpaceWatcher, Worker};
use crate::utils;
use crossbeam_utils::thread::{scope, ScopedJoinHandle};
use rustwide::logging::LogStorage;
use rustwide::Workspace;
use std::collections::HashMap;
use std::path::Path;
use std::sync::Mutex;
use std::thread;
use std::time::Duration;

const DISK_SPACE_WATCHER_INTERVAL: Duration = Duration::from_secs(600);
const DISK_SPACE_WATCHER_THRESHOLD: f32 = 0.9;

#[derive(Debug, Fail)]
#[fail(display = "overridden task result to {}", _0)]
pub struct OverrideResult(TestResult);

struct RunnerStateInner {
    prepare_logs: HashMap<Crate, LogStorage>,
}

struct RunnerState {
    inner: Mutex<RunnerStateInner>,
}

impl RunnerState {
    fn new() -> Self {
        RunnerState {
            inner: Mutex::new(RunnerStateInner {
                prepare_logs: HashMap::new(),
            }),
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<RunnerStateInner> {
        self.inner.lock().unwrap()
    }
}

pub fn run_ex<DB: WriteResults + Sync>(
    ex: &Experiment,
    workspace: &Workspace,
    crates: &[Crate],
    db: &DB,
    threads_count: usize,
    config: &Config,
) -> Fallible<()> {
    if !rustwide::cmd::docker_running(workspace) {
        return Err(err_msg("docker is not running"));
    }

    let res = run_ex_inner(ex, workspace, crates, db, threads_count, config);


    res
}

fn run_ex_inner<DB: WriteResults + Sync>(
    ex: &Experiment,
    workspace: &Workspace,
    crates: &[Crate],
    db: &DB,
    threads_count: usize,
    config: &Config,
) -> Fallible<()> {
    info!("computing the tasks graph...");
    let graph = Mutex::new(build_graph(ex, crates, config));

    info!("preparing the execution...");
    for tc in &ex.toolchains {
        tc.install(workspace)?;
        if ex.mode == Mode::Clippy {
            tc.add_component(workspace, "clippy")?;
        }
    }

    info!("running tasks in {} threads...", threads_count);

    // An HashMap is used instead of an HashSet because Thread is not Eq+Hash
    let parked_threads: Mutex<HashMap<thread::ThreadId, thread::Thread>> =
        Mutex::new(HashMap::new());
    let state = RunnerState::new();

    let workers = (0..threads_count)
        .map(|i| {
            Worker::new(
                format!("worker-{}", i),
                workspace,
                ex,
                config,
                &graph,
                &state,
                db,
                &parked_threads,
            )
        })
        .collect::<Vec<_>>();

    let disk_watcher = DiskSpaceWatcher::new(
        DISK_SPACE_WATCHER_INTERVAL,
        DISK_SPACE_WATCHER_THRESHOLD,
        &workers,
    );

    scope(|scope| -> Fallible<()> {
        let mut threads = Vec::new();

        for worker in &workers {
            let join = scope
                .builder()
                .name(worker.name().into())
                .spawn(move || worker.run())?;
            threads.push(join);
        }
        let disk_watcher_thread = scope
            .builder()
            .name("disk-space-watcher".into())
            .spawn(|| disk_watcher.run())?;

        let clean_exit = join_threads(threads.drain(..));
        disk_watcher.stop();
        let disk_watcher_clean_exit = join_threads(std::iter::once(disk_watcher_thread));

        if clean_exit && disk_watcher_clean_exit {
            Ok(())
        } else {
            bail!("some threads returned an error");
        }
    })?;

    // Only the root node must be present
    let mut g = graph.lock().unwrap();
    assert!(g.next_task(ex, db).is_finished());
    assert_eq!(g.pending_crates_count(), 0);

    Ok(())
}

fn join_threads<'a, I>(iter: I) -> bool
where
    I: Iterator<Item = ScopedJoinHandle<'a, Fallible<()>>>,
{
    let mut clean_exit = true;
    for thread in iter {
        match thread.join() {
            Ok(Ok(())) => {}
            Ok(Err(err)) => {
                crate::utils::report_failure(&err);
                clean_exit = false;
            }
            Err(panic) => {
                crate::utils::report_panic(&panic);
                clean_exit = false;
            }
        }
    }
    clean_exit
}

pub fn dump_dot(ex: &Experiment, crates: &[Crate], config: &Config, dest: &Path) -> Fallible<()> {
    info!("computing the tasks graph...");
    let graph = build_graph(&ex, crates, config);

    info!("dumping the tasks graph...");
    ::std::fs::write(dest, format!("{:?}", graph.generate_dot()).as_bytes())?;

    info!("tasks graph available in {}", dest.to_string_lossy());

    Ok(())
}
