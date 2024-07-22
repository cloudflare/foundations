#[cfg(feature = "settings")]
use crate::settings::settings;

/// Memory profiler settings.
#[cfg_attr(feature = "settings", settings(crate_path = "crate"))]
#[cfg_attr(not(feature = "settings"), derive(Clone, Debug, serde::Deserialize))]
pub struct MemoryProfilerSettings {
    /// Enables memory profiling
    pub enabled: bool,

    /// Value between `0` and `64` which specifies the number of bytes of
    /// allocation activity between samples as `number_of_bytes = 2 ^ sample_interval`.
    ///
    /// Increasing the `sample_interval` decreases profile fidelity, but also decreases the
    /// computational overhead.
    ///
    /// The default is `19` (2 ^ 19 = 512KiB).
    #[serde(default = "MemoryProfilerSettings::default_sample_interval")]
    pub sample_interval: u8,
}

#[cfg(not(feature = "settings"))]
impl Default for MemoryProfilerSettings {
    fn default() -> Self {
        Self {
            enabled: false,
            sample_interval: MemoryProfilerSettings::default_sample_interval(),
        }
    }
}

impl MemoryProfilerSettings {
    fn default_sample_interval() -> u8 {
        19
    }
}
