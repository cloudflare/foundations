//! Command line interface-related functionality.

use super::settings::Settings;
use super::{BootstrapResult, ServiceInfo};
use anyhow::anyhow;
use clap::error::ErrorKind;
use clap::Command;
use std::ffi::OsString;

pub use clap::{Arg, ArgAction, ArgMatches};

const GENERATE_CONFIG_OPT_ID: &str = "generate";
const USE_CONFIG_OPT_ID: &str = "config";

/// A command line interface (CLI) helper that takes care of the command line arguments parsing
/// basics.
///
/// Foundations-based services are expected to primarily use [`Settings`] for its configuration. This
/// helper takes care of setting up CLI with the provided [`ServiceInfo`] and service configuration
/// parsing.
///
/// By default the following command line options are added:
///
/// - `-c`, `--config` - specifies an existing configuration file for the service.
/// - `-g`, `--generate` - generates a new default configuration file for the service.
/// - `-h`, `--help` - prints CLI help information and exits.
/// - `-v`, `--version` - prints the service version and exits.
///
/// Additional arguments can be added via `custom_args` argument of the [`Cli::new`] function.
///
/// [`Settings`]: crate::settings::Settings
pub struct Cli<S: Settings> {
    /// Parsed service settings.
    pub settings: S,

    /// Parsed service arguments.
    pub arg_matches: ArgMatches,
}

impl<S: Settings> Cli<S> {
    /// Bootstraps a new command line interface (CLI) for the service.
    ///
    /// `custom_args` argument can be used to add extra service-specific arguments to the CLI.
    ///
    /// The function will implicitly print relevant information and exit the process if
    /// `--help` or `--version` command line options are specified.
    ///
    /// Any command line parsing errors are intentionally propagated as a [`BootstrapResult`],
    /// so they can be reported to a panic handler (e.g. [Sentry]) if the service uses one.
    ///
    /// [Sentry]: https://sentry.io/
    pub fn new(service_info: &ServiceInfo, custom_args: Vec<Arg>) -> BootstrapResult<Self> {
        Self::new_from_os_args(service_info, custom_args, std::env::args_os())
    }

    /// Bootstraps a new command line interface (CLI) for the service with the provided `os_args`.
    ///
    /// This method is the same as [`Cli::new`], but accepts source OS arguments instead of taking
    /// them fron [`std::env::args_os`].
    ///
    /// Useful for testing purposes.
    pub fn new_from_os_args(
        service_info: &ServiceInfo,
        custom_args: Vec<Arg>,
        os_args: impl IntoIterator<Item = impl Into<OsString> + Clone>,
    ) -> BootstrapResult<Self> {
        let mut cmd = Command::new(service_info.name)
            .version(service_info.version)
            .author(service_info.author)
            .about(service_info.description)
            .arg(
                Arg::new("config")
                    .required_unless_present(GENERATE_CONFIG_OPT_ID)
                    .action(ArgAction::Set)
                    .long("config")
                    .short('c')
                    .help("Specifies the config to run the service with"),
            )
            .arg(
                Arg::new(GENERATE_CONFIG_OPT_ID)
                    .action(ArgAction::Set)
                    .long("generate")
                    .short('g')
                    .help("Generates a new default config for the service"),
            );

        for arg in custom_args {
            cmd = cmd.arg(arg);
        }

        let arg_matches = get_arg_matches(cmd, os_args)?;
        let settings = get_settings(&arg_matches)?;

        Ok(Self {
            settings,
            arg_matches,
        })
    }
}

fn get_arg_matches(
    cmd: Command,
    os_args: impl IntoIterator<Item = impl Into<OsString> + Clone>,
) -> BootstrapResult<ArgMatches> {
    cmd.try_get_matches_from(os_args).map_err(|e| {
        let kind = e.kind();

        // NOTE: print info and terminate the process
        if kind == ErrorKind::DisplayHelp || kind == ErrorKind::DisplayVersion {
            e.exit();
        }

        // NOTE: otherwise propagate as an error, so it can be captured by a panic handler
        // if necesary (e.g. Sentry).
        e.into()
    })
}

fn get_settings<S: Settings>(arg_matches: &ArgMatches) -> BootstrapResult<S> {
    if let Some(path) = arg_matches.get_one::<String>(GENERATE_CONFIG_OPT_ID) {
        let settings = S::default();

        crate::settings::to_yaml_file(&settings, path)?;

        return Ok(settings);
    }

    if let Some(path) = arg_matches.get_one::<String>(USE_CONFIG_OPT_ID) {
        return crate::settings::from_file(path).map_err(|e| anyhow!(e));
    }

    unreachable!("clap should require config options to be present")
}
