//! Watch mode support

use crate::{
    cmd::{build::BuildArgs, test::TestArgs},
    utils::{self, FoundryPathExt},
};
use clap::Parser;
use regex::Regex;
use std::{convert::Infallible, path::PathBuf, str::FromStr, sync::Arc};
use watchexec::{
    action::{Action, Outcome, PreSpawn},
    command::Shell,
    config::{InitConfig, RuntimeConfig},
    event::{Event, ProcessEnd},
    handler::SyncFnHandler,
    paths::summarise_events_to_env,
    signal::source::MainSignal,
    Watchexec,
};

#[derive(Debug, Clone, Parser, Default)]
pub struct WatchArgs {
    /// File updates debounce delay
    ///
    /// During this time, incoming change events are accumulated and
    /// only once the delay has passed, is an action taken. Note that
    /// this does not mean a command will be started: if --no-restart is
    /// given and a command is already running, the outcome of the
    /// action will be to do nothing.
    ///
    /// Defaults to 50ms. Parses as decimal seconds by default, but
    /// using an integer with the `ms` suffix may be more convenient.
    /// When using --poll mode, you'll want a larger duration, or risk
    /// overloading disk I/O.
    #[clap(short = 'd', long = "delay", forbid_empty_values = true)]
    pub delay: Option<String>,

    /// Don’t restart command while it’s still running
    #[clap(long = "no-restart")]
    pub no_restart: bool,

    /// Explicitly run all tests on change
    #[clap(long = "run-all")]
    pub run_all: bool,

    /// Watch specific file(s) or folder(s)
    ///
    /// By default, the project's source dir is watched
    #[clap(
        short = 'w',
        long = "watch",
        value_name = "path",
        min_values = 0,
        multiple_values = true,
        multiple_occurrences = false
    )]
    pub watch: Option<Vec<PathBuf>>,
}

/// Executes a [`Watchexec`] that listens for changes in the project's src dir and reruns `forge
/// build`
pub async fn watch_build(args: BuildArgs) -> eyre::Result<()> {
    let (init, mut runtime) = args.watchexec_config()?;
    let cmd = cmd_args(args.watch.watch.as_ref().map(|paths| paths.len()).unwrap_or_default());
    runtime.command(cmd.clone());

    let wx = Watchexec::new(init, runtime.clone())?;
    on_action(args.watch, runtime, Arc::clone(&wx), cmd, (), |_| {});

    // start executing the command immediately
    wx.send_event(Event::default()).await?;
    wx.main().await??;

    Ok(())
}

/// Executes a [`Watchexec`] that listens for changes in the project's src dir and reruns `forge
/// test`
pub async fn watch_test(args: TestArgs) -> eyre::Result<()> {
    let (init, mut runtime) = args.build_args().watchexec_config()?;
    let cmd = cmd_args(
        args.build_args().watch.watch.as_ref().map(|paths| paths.len()).unwrap_or_default(),
    );
    runtime.command(cmd.clone());
    let wx = Watchexec::new(init, runtime.clone())?;

    // marker to check whether we can safely override the command
    let has_conflicting_pattern_args = args.filter().pattern.is_some() ||
        args.filter().test_pattern.is_some() ||
        args.filter().path_pattern.is_some() ||
        args.build_args().watch.run_all;

    on_action(
        args.build_args().watch.clone(),
        runtime,
        Arc::clone(&wx),
        cmd,
        WatchTestState { has_conflicting_pattern_args, last_test_files: Default::default() },
        on_test,
    );

    // start executing the command immediately
    wx.send_event(Event::default()).await?;
    wx.main().await??;

    Ok(())
}

#[derive(Debug, Clone)]
struct WatchTestState {
    /// marks whether the initial test args contains args that would conflict when adding a
    /// match-path arg
    has_conflicting_pattern_args: bool,
    /// Tracks the last changed test files, if any so that if a non-test file was modified we run
    /// this file instead *Note:* this is a vec, so we can also watch out for changes
    /// introduced by `forge fmt`
    last_test_files: Vec<String>,
}

/// The `on_action` hook for `forge test --watch`
fn on_test(action: OnActionState<WatchTestState>) {
    let OnActionState { args, runtime, action, wx, cmd, other } = action;
    let WatchTestState { has_conflicting_pattern_args, last_test_files } = other;
    if has_conflicting_pattern_args {
        // can't set conflicting arguments
        return
    }

    let mut cmd = cmd.clone();

    let mut changed_sol_test_files: Vec<_> = action
        .events
        .iter()
        .flat_map(|e| e.paths())
        .filter(|(path, _)| path.is_sol_test())
        .filter_map(|(path, _)| path.to_str())
        .map(str::to_string)
        .collect();

    if changed_sol_test_files.is_empty() {
        if last_test_files.is_empty() {
            return
        }
        // reuse the old test files if a non test file was changed
        changed_sol_test_files = last_test_files;
    }

    // replace `--match-path` | `-mp` argument
    if let Some(pos) = cmd.iter().position(|arg| arg == "--match-path" || arg == "-mp") {
        // --match-path requires 1 argument
        cmd.drain(pos..=(pos + 1));
    }

    // append `--match-path` regex
    let re_str = format!("({})", changed_sol_test_files.join("|"));
    if let Ok(re) = Regex::from_str(&re_str) {
        let mut new_cmd = cmd.clone();
        new_cmd.push("--match-path".to_string());
        new_cmd.push(re.to_string());
        // reconfigure the executor with a new runtime
        let mut config = runtime.clone();
        config.command(new_cmd);
        // re-register the action
        on_action(
            args.clone(),
            config,
            wx,
            cmd,
            WatchTestState {
                has_conflicting_pattern_args,
                last_test_files: changed_sol_test_files,
            },
            on_test,
        );
    } else {
        eprintln!("failed to parse new regex {}", re_str);
    }
}

/// Returns the env args without the `--watch` flag from the args for the Watchexec command
fn cmd_args(num: usize) -> Vec<String> {
    // all the forge arguments including path to forge bin
    let mut cmd_args: Vec<_> = std::env::args().collect();
    if let Some(pos) = cmd_args.iter().position(|arg| arg == "--watch" || arg == "-w") {
        cmd_args.drain(pos..=(pos + num));
    }
    cmd_args
}

/// Returns the Initialisation configuration for [`Watchexec`].
pub fn init() -> eyre::Result<InitConfig> {
    let mut config = InitConfig::default();
    config.on_error(SyncFnHandler::from(|data| -> std::result::Result<(), Infallible> {
        tracing::trace!("[[{:?}]]", data);
        Ok(())
    }));

    Ok(config)
}

/// Contains all necessary context to reconfigure a [`Watchexec`] on the fly
struct OnActionState<'a, T: Clone> {
    args: &'a WatchArgs,
    runtime: &'a RuntimeConfig,
    action: &'a Action,
    cmd: &'a Vec<String>,
    wx: Arc<Watchexec>,
    // additional context to inject
    other: T,
}

/// Registers the `on_action` hook on the `RuntimeConfig` currently in use in the `Watchexec`
///
/// **Note** this is a bit weird since we're installing the hook on the config that's already used
/// in `Watchexec` but necessary if we want to have access to it in order to
/// [`Watchexec::reconfigure`]
fn on_action<F, T>(
    args: WatchArgs,
    mut config: RuntimeConfig,
    wx: Arc<Watchexec>,
    cmd: Vec<String>,
    other: T,
    f: F,
) where
    F: for<'a> Fn(OnActionState<'a, T>) + Send + 'static,
    T: Clone + Send + 'static,
{
    let on_busy = if args.no_restart { "do-nothing" } else { "restart" };
    let runtime = config.clone();
    let w = Arc::clone(&wx);
    config.on_action(move |action: Action| {
        let fut = async { Ok::<(), Infallible>(()) };
        let signals: Vec<MainSignal> = action.events.iter().flat_map(|e| e.signals()).collect();
        let has_paths = action.events.iter().flat_map(|e| e.paths()).next().is_some();

        if signals.contains(&MainSignal::Terminate) || signals.contains(&MainSignal::Interrupt) {
            action.outcome(Outcome::both(Outcome::Stop, Outcome::Exit));
            return fut
        }

        if !has_paths {
            if !signals.is_empty() {
                let mut out = Outcome::DoNothing;
                for sig in signals {
                    out = Outcome::both(out, Outcome::Signal(sig.into()));
                }

                action.outcome(out);
                return fut
            }

            let completion = action.events.iter().flat_map(|e| e.completions()).next();
            if let Some(status) = completion {
                match status {
                    Some(ProcessEnd::ExitError(code)) => {
                        tracing::trace!("Command exited with {}", code)
                    }
                    Some(ProcessEnd::ExitSignal(sig)) => {
                        tracing::trace!("Command killed by {:?}", sig)
                    }
                    Some(ProcessEnd::ExitStop(sig)) => {
                        tracing::trace!("Command stopped by {:?}", sig)
                    }
                    Some(ProcessEnd::Continued) => tracing::trace!("Command continued"),
                    Some(ProcessEnd::Exception(ex)) => {
                        tracing::trace!("Command ended by exception {:#x}", ex)
                    }
                    Some(ProcessEnd::Success) => tracing::trace!("Command was successful"),
                    None => tracing::trace!("Command completed"),
                };

                action.outcome(Outcome::DoNothing);
                return fut
            }
        }

        f(OnActionState {
            args: &args,
            runtime: &runtime,
            action: &action,
            wx: w.clone(),
            cmd: &cmd,
            other: other.clone(),
        });

        // mattsse: could be made into flag to never clear the shell
        let clear = true;
        let when_running = match (clear, on_busy) {
            (_, "do-nothing") => Outcome::DoNothing,
            (true, "restart") => {
                Outcome::both(Outcome::Stop, Outcome::both(Outcome::Clear, Outcome::Start))
            }
            (false, "restart") => Outcome::both(Outcome::Stop, Outcome::Start),
            _ => Outcome::DoNothing,
        };

        let when_idle =
            if clear { Outcome::both(Outcome::Clear, Outcome::Start) } else { Outcome::Start };

        action.outcome(Outcome::if_running(when_running, when_idle));

        fut
    });

    let _ = wx.reconfigure(config);
}

/// Returns the Runtime configuration for [`Watchexec`].
pub fn runtime(args: &WatchArgs) -> eyre::Result<RuntimeConfig> {
    let mut config = RuntimeConfig::default();

    config.pathset(args.watch.clone().unwrap_or_default());

    if let Some(delay) = &args.delay {
        config.action_throttle(utils::parse_delay(delay)?);
    }

    config.command_shell(default_shell());

    config.on_pre_spawn(move |prespawn: PreSpawn| async move {
        let envs = summarise_events_to_env(prespawn.events.iter());
        if let Some(mut command) = prespawn.command().await {
            for (k, v) in envs {
                command.env(format!("CARGO_WATCH_{}_PATH", k), v);
            }
        }

        Ok::<(), Infallible>(())
    });

    Ok(config)
}

#[cfg(windows)]
fn default_shell() -> Shell {
    Shell::Powershell
}

#[cfg(not(windows))]
fn default_shell() -> Shell {
    Shell::default()
}
