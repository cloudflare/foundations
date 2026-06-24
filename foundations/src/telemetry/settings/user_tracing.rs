use std::num::NonZeroUsize;

#[cfg(feature = "settings")]
use crate::settings::settings;

/// Distributed user tracing settings.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, serde::Deserialize))]
pub struct UserTracingSettings {
    /// Enables user tracing.
    #[serde(default = "UserTracingSettings::default_enabled")]
    pub enabled: bool,

    /// Maximum number of spans to buffer for output. Any spans above
    /// this limit will be dropped until the queue regains capacity.
    ///
    /// The default is to buffer up to 1 million spans in memory. This protects
    /// services from out-of-memory errors when the output gets heavily backed up.
    /// To disable the limit entirely, set this setting to `None`.
    #[serde(default = "UserTracingSettings::default_max_queue_size")]
    pub max_queue_size: Option<NonZeroUsize>,

    /// The output for the collected user traces.
    pub output: UserTracesOutput,
}

#[cfg(not(feature = "settings"))]
impl Default for UserTracingSettings {
    fn default() -> Self {
        Self {
            enabled: UserTracingSettings::default_enabled(),
            max_queue_size: UserTracingSettings::default_max_queue_size(),
            output: Default::default(),
        }
    }
}

impl UserTracingSettings {
    fn default_enabled() -> bool {
        true
    }

    const fn default_max_queue_size() -> Option<NonZeroUsize> {
        Some(const { NonZeroUsize::new(1_000_000).expect("1_000_000 is not zero") })
    }
}

/// The output for the collected user traces.
#[cfg_attr(
    feature = "settings",
    settings(crate_path = "crate", impl_default = false)
)]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, serde::Deserialize))]
pub enum UserTracesOutput {
    /// Send user tracing spans as OTLP over a Unix domain socket to an OTLP endpoint.
    OtlpUds(OtlpUdsOutputSettings),
}

impl Default for UserTracesOutput {
    fn default() -> Self {
        Self::OtlpUds(Default::default())
    }
}

/// [OTLP over UDS] output settings for user tracing.
///
/// Sends trace data as protobuf-encoded OTLP over HTTP to a Unix domain socket.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, serde::Deserialize))]
pub struct OtlpUdsOutputSettings {
    /// Path to the Unix domain socket for the OTLP endpoint.
    pub socket_path: String,

    /// Number of concurrent worker tasks for user trace export.
    ///
    /// # Default
    ///
    /// Default value is `2`.
    #[serde(default = "OtlpUdsOutputSettings::default_num_tasks")]
    pub num_tasks: usize,

    /// Maximum number of spans to drain per batch.
    ///
    /// # Default
    ///
    /// Default value is `512`.
    #[serde(default = "OtlpUdsOutputSettings::default_max_batch_size")]
    pub max_batch_size: usize,
}

#[cfg(not(feature = "settings"))]
impl Default for OtlpUdsOutputSettings {
    fn default() -> Self {
        Self {
            socket_path: String::new(),
            num_tasks: OtlpUdsOutputSettings::default_num_tasks(),
            max_batch_size: OtlpUdsOutputSettings::default_max_batch_size(),
        }
    }
}

impl OtlpUdsOutputSettings {
    const fn default_num_tasks() -> usize {
        2
    }

    const fn default_max_batch_size() -> usize {
        512
    }
}
